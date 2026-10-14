import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { stripVTControlCharacters } from "node:util";

import { check, PropertyFailure, rng, seeds } from "../src/check.js";

test("the PRNG is deterministic and in range", () => {
  const a = rng(42);
  const b = rng(42);
  for (let i = 0; i < 1000; i++) {
    const x = a.int(-3, 7);
    assert.equal(x, b.int(-3, 7));
    assert.ok(x >= -3 && x <= 7 && Number.isInteger(x));
  }
  assert.notEqual(rng(1).next(), rng(2).next());
});

test("every value in a range is reachable", () => {
  const r = rng(7);
  const seen = new Set();
  for (let i = 0; i < 1000; i++) seen.add(r.int(0, 9));
  assert.equal(seen.size, 10);
});

test("seeds come from the environment", () => {
  assert.equal(seeds({}).length, 100);
  assert.deepEqual(seeds({ NOMI_SEED: "17" }), [17]);
  assert.deepEqual(seeds({ NOMI_SEEDS: "3", NOMI_SEED_BASE: "10" }), [10, 11, 12]);
  assert.throws(() => seeds({ NOMI_SEEDS: "many" }), /not a non-negative integer/);
});

// A stack with a planted bug: popping when it holds exactly three items
// returns the bottom one. A model-based run must find it, shrink the case
// to the few steps that matter, and replay it from the seed alone.
class BuggyStack {
  items = [];
  push(x) {
    this.items.push(x);
  }
  pop() {
    return this.items.length === 3 ? this.items.shift() : this.items.pop();
  }
}

const stackOps = (r) => r.array((r) => (r.bool(0.6) ? { push: r.int(0, 9) } : { pop: true }), 0, 40);

function agreesWithModel(ops) {
  const real = new BuggyStack();
  const model = [];
  for (const op of ops) {
    if ("push" in op) {
      real.push(op.push);
      model.push(op.push);
    } else {
      assert.equal(real.pop(), model.pop());
    }
  }
}

test("a model-based run finds, shrinks and replays a planted bug", async () => {
  const failure = await check(stackOps, agreesWithModel, {}).then(
    () => assert.fail("the planted bug went unnoticed"),
    (error) => error,
  );
  assert.ok(failure instanceof PropertyFailure);
  // Three pushes with distinct values and one pop is the smallest failing case.
  assert.equal(failure.case.length, 4);
  assert.match(failure.message, new RegExp(`replay: NOMI_SEED=${failure.seed} `));
  const replayed = await check(stackOps, agreesWithModel, { NOMI_SEED: String(failure.seed) }).then(
    () => undefined,
    (error) => error,
  );
  assert.equal(replayed?.seed, failure.seed);
  assert.deepEqual(replayed.case, failure.case);
});

test("a correct implementation passes every seed", async () => {
  await check(
    (r) => r.array((r) => r.int(0, 100), 0, 20),
    (xs) => assert.deepEqual([...xs].reverse().reverse(), xs),
    {},
  );
});

/** cmd.exe has no `NAME=value command` form. */
const posixShell = { skip: process.platform === "win32" && "the replay line is for a POSIX shell" };

test("the replay line a failure prints runs the failing property", posixShell, async (t) => {
  // A suite of its own, so running it doesn't run this test again.
  const dir = await mkdtemp(join(tmpdir(), "nomi-replay-"));
  t.after(() => rm(dir, { recursive: true }));
  const checker = JSON.stringify(new URL("../src/check.js", import.meta.url).href);
  await writeFile(
    join(dir, "threes.test.mjs"),
    `import { test } from "node:test";
import { check } from ${checker};
test("no threes", () => check((r) => r.int(0, 9), (n) => { if (n === 3) throw new Error("a three"); }));
`,
  );
  // Without this run's seeds, and outside its test runner, inside which
  // `node --test` runs nothing.
  const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !/^(NOMI_SEED|NODE_TEST_)/.test(name)));
  const run = (command) => {
    const { status, stdout } = spawnSync(command, { cwd: dir, env, shell: true, encoding: "utf8" });
    return { status, stdout: stripVTControlCharacters(stdout) };
  };
  const first = run("node --test");
  const [, seed] = first.stdout.match(/failed at seed (\d+)/);
  const [, replay] = first.stdout.match(/replay: (.*)/);
  const again = run(replay);
  assert.notEqual(again.status, 0, again.stdout);
  assert.match(again.stdout, new RegExp(`failed at seed ${seed}\\b`));
});
