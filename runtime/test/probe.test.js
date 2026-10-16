import assert from "node:assert/strict";
import { test } from "node:test";

import { probe, SINCE } from "../src/probe.js";

/** A process that records its exits, at version `version`. */
function fakeProcess(version) {
  const exits = [];
  return { version, exits, exit: (code) => exits.push(code) };
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

test("a scope with every feature passes", () => {
  const proc = fakeProcess("v22.3.0");
  assert.deepEqual(
    errors(() => probe(Object.keys(SINCE), { proc })),
    [],
  );
  assert.deepEqual(proc.exits, []);
});

test("the first missing feature is named with the Node that has it, and the probe exits 1", () => {
  const proc = fakeProcess("v22.1.0");
  const scope = { fetch() {}, process: { getBuiltinModule: undefined } };
  const lines = errors(() =>
    probe(["fetch", "process.getBuiltinModule", "AbortController"], { scope, proc }),
  );
  assert.deepEqual(lines, [
    "nomi: this program needs process.getBuiltinModule, which Node 22.3 and later have (this is Node v22.1.0).",
  ]);
  assert.deepEqual(proc.exits, [1]);
});

test("a feature under a missing object is missing", () => {
  const proc = fakeProcess("v14.0.0");
  const lines = errors(() => probe(["process.getBuiltinModule"], { scope: {}, proc }));
  assert.equal(lines.length, 1);
  assert.deepEqual(proc.exits, [1]);
});
