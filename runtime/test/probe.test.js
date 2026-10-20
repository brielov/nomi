import assert from "node:assert/strict";
import { test } from "node:test";

import { MINIMUM, probe } from "../src/probe.js";

/** A process that records its exits. */
function fakeProcess(release) {
  const exits = [];
  return { ...release, exits, exit: (code) => exits.push(code) };
}

/** The lines `run` writes to standard error. */
function errors(run) {
  const lines = [];
  const original = console.error;
  console.error = (...args) => lines.push(args.join(" "));
  try {
    run();
  } finally {
    console.error = original;
  }
  return lines;
}

test("Bun at the minimum or later passes", () => {
  for (const version of [`${MINIMUM}.0`, "1.4.3", "1.10.0", "2.0.0"]) {
    const proc = fakeProcess({});
    assert.deepEqual(errors(() => probe({ scope: { Bun: { version } }, proc })), [], version);
    assert.deepEqual(proc.exits, []);
  }
});

test("an older Bun is refused, naming its version, and the probe exits 1", () => {
  const proc = fakeProcess({});
  const lines = errors(() => probe({ scope: { Bun: { version: "1.3.9" } }, proc }));
  assert.deepEqual(lines, [`nomi: this program runs on Bun ${MINIMUM} or later (this is Bun 1.3.9).`]);
  assert.deepEqual(proc.exits, [1]);
});

test("Node is refused, naming its version", () => {
  const proc = fakeProcess({ version: "v24.21.0", release: { name: "node" } });
  const lines = errors(() => probe({ scope: {}, proc }));
  assert.deepEqual(lines, [`nomi: this program runs on Bun ${MINIMUM} or later (this is Node v24.21.0).`]);
  assert.deepEqual(proc.exits, [1]);
});

test("another host is refused", () => {
  const proc = fakeProcess({});
  const lines = errors(() => probe({ scope: {}, proc }));
  assert.deepEqual(lines, [`nomi: this program runs on Bun ${MINIMUM} or later (this is not Bun).`]);
  assert.deepEqual(proc.exits, [1]);
});
