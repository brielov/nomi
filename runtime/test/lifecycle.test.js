import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { andThen, async, forkDaemon, Runtime, succeed, sync } from "../src/fiber.js";
import { main, requestExit } from "../src/main.js";

const program = fileURLToPath(new URL("../fixtures/program.js", import.meta.url));
const worker = fileURLToPath(new URL("../fixtures/worker.js", import.meta.url));

/**
 * Runs the scenario of `fixture`, `program` by default. The first of `signals` is sent once it prints
 * "ready", and each later one once it prints another line, in reaction to
 * the one before: two identical signals sent together can arrive as one.
 */
function run(scenario, { fixture = program, signals = [], deadline } = {}) {
  const args = [fixture, scenario, ...(deadline === undefined ? [] : [String(deadline)])];
  const child = spawn(process.execPath, args, { stdio: ["ignore", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  child.stderr.on("data", (chunk) => (stderr += chunk));
  let pending = [...signals];
  let lines = Infinity;
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
    const now = stdout.split("\n").length;
    if (lines === Infinity && stdout.includes("ready\n")) lines = now - 1;
    if (now > lines && pending.length > 0) {
      lines = now;
      child.kill(pending.shift());
    }
  });
  const started = Date.now();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error(`${scenario} did not exit\nstdout: ${stdout}\nstderr: ${stderr}`));
    }, 10_000);
    child.on("exit", (code, signal) => {
      clearTimeout(timer);
      resolve({ code, signal, stdout: stdout.trim().split("\n"), stderr, ms: Date.now() - started });
    });
  });
}

/** Windows can't send a child a POSIX signal: `kill` ends it outright. */
const posixSignals = { skip: process.platform === "win32" && "Windows has no POSIX signals" };

test("SIGINT interrupts the program, runs finalizers last to first and exits 130", posixSignals, async () => {
  const { code, stdout } = await run("wait", { signals: ["SIGINT"] });
  assert.equal(code, 130);
  assert.deepEqual(stdout, ["a acquired", "b acquired", "ready", "b released", "a released"]);
});

test("SIGTERM exits 143 after the finalizers", posixSignals, async () => {
  const { code, stdout } = await run("wait", { signals: ["SIGTERM"] });
  assert.equal(code, 143);
  assert.deepEqual(stdout.slice(-2), ["b released", "a released"]);
});

test("a shutdown that outlasts the deadline is cut short", posixSignals, async () => {
  const { code, stdout, stderr } = await run("hang", { signals: ["SIGTERM"], deadline: 200 });
  assert.equal(code, 143);
  assert.deepEqual(stdout.slice(-1), ["releasing"]);
  assert.match(stderr, /shutdown took longer than 200 ms/);
});

test("a program can lengthen its shutdown deadline", posixSignals, async () => {
  const { code, stdout, stderr } = await run("allowed", { signals: ["SIGTERM"], deadline: 200 });
  assert.equal(code, 143);
  assert.deepEqual(stdout.slice(-1), ["slow released"]);
  assert.equal(stderr, "");
});

test("a second signal exits at once", posixSignals, async () => {
  const { code, stderr, ms } = await run("hang", { signals: ["SIGINT", "SIGINT"], deadline: 5_000 });
  assert.equal(code, 130);
  assert.match(stderr, /SIGINT again/);
  assert.ok(ms < 4_000, `${ms} ms`);
});

test("when the program ends, the fibers it spawned are interrupted", async () => {
  const { code, stdout } = await run("daemon");
  assert.equal(code, 0);
  assert.deepEqual(stdout, ["daemon acquired", "main done", "daemon released"]);
});

test("the exit code follows the program's exit", async () => {
  const ok = await run("ok");
  assert.equal(ok.code, 0);
  assert.deepEqual(ok.stdout, ["hello"]);
  const failed = await run("fail");
  assert.equal(failed.code, 1);
  assert.match(failed.stderr, /the program failed: bad input/);
  const died = await run("defect");
  assert.equal(died.code, 1);
  assert.match(died.stderr, /defect: Error: broken/);
});

test("a program that asks to exit runs its finalizers and exits with its code", async () => {
  const { code, stdout, stderr } = await run("exit");
  assert.equal(code, 3);
  assert.deepEqual(stdout, ["a acquired", "a released"]);
  assert.equal(stderr, "");
});

test("a worker's timer keeps it alive until a signal shuts its runtime down", posixSignals, async () => {
  for (const [signal, code] of [
    ["SIGTERM", 143],
    ["SIGINT", 130],
  ]) {
    const { code: exited, stdout } = await run("ticking", { fixture: worker, signals: [signal] });
    assert.equal(exited, code, signal);
    assert.deepEqual(stdout, ["ticker acquired", "ready", "ticker released"], signal);
  }
});

test("a worker with nothing live exits 0 on its own", async () => {
  const { code, stdout } = await run("idle", { fixture: worker });
  assert.equal(code, 0);
  assert.deepEqual(stdout, ["ready"]);
});

test("a program that asks to exit as a synchronous runtime starts it exits with its code", async () => {
  const proc = Object.assign(new EventEmitter(), { exitCode: undefined, exit: () => {} });
  const runtime = new Runtime({ synchronous: true });
  const forever = async(() => () => {});
  const code = await main(andThen(sync(() => requestExit(runtime, 3)), () => forever), { runtime, proc });
  assert.equal(code, 3);
  assert.equal(proc.exitCode, 3);
});

test("an exit main does not know is a runtime bug, not a clean exit", async () => {
  const proc = Object.assign(new EventEmitter(), { exitCode: undefined, exit: () => {} });
  const runtime = Object.assign(new Runtime(), { fork: (task, observe) => observe({ $: "lost" }) });
  await assert.rejects(main(succeed(), { runtime, proc }), /not an exit: lost/);
  assert.equal(proc.exitCode, undefined);
});

test("the runtime's metrics are complete when main resolves, before the process exits", async () => {
  // `exit` is only the fallback for a leaked handle; this test leaks none.
  const proc = Object.assign(new EventEmitter(), { exitCode: undefined, exit: () => {} });
  const runtime = new Runtime();
  const daemon = async(() => () => {});
  const code = await main(andThen(forkDaemon(daemon), () => succeed()), { runtime, proc });
  assert.equal(code, 0);
  assert.equal(proc.exitCode, 0);
  assert.deepEqual(runtime.stats, { started: 2, finished: 2 });
  // Signal handlers are gone, so the host's defaults apply again.
  assert.equal(proc.listenerCount("SIGINT") + proc.listenerCount("SIGTERM"), 0);
});
