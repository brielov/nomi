// Running a program as a process (architecture §8, "Lifecycle").
//
// The program's task runs on the root fiber. SIGINT or SIGTERM interrupts
// it, so its finalizers run, and the process exits with 128 plus the signal
// number: 130 or 143. Once the root fiber ends, the fibers it left in the
// root scope (`Process.spawn`) are interrupted too. Shutting down must
// finish within a deadline; past it, or on a second signal, the process
// exits at once. That deadline is real time and bounds the whole shutdown,
// a server's drain included (a `draining` scope's, on the runtime's
// clock), so a server lengthens it with `allowShutdown` to cover its
// drain. A program may also ask to exit with a code of its own
// (`requestExit`), which stops it as a signal does. A `Platform.worker` has
// no root fiber: a signal shuts its runtime down directly, and until one
// comes it lives as long as Node's event loop does.

import { describeDefect, ExitKind, Runtime } from "./fiber.js";

const SIGNALS = { SIGINT: 130, SIGTERM: 143 };

// The longest delay a host timer waits; past it, `setTimeout` fires at once.
const LONGEST_DELAY = 2 ** 31 - 1;

// Each running lifecycle, by the runtime it runs.
const lifecycles = new WeakMap();

/**
 * Makes the shutdown deadline of the program running on `runtime` at least
 * `ms`; a runtime no lifecycle runs has none to change.
 */
export function allowShutdown(runtime, ms) {
  lifecycles.get(runtime)?.allow(ms);
}

/**
 * Stops the program running on `runtime` as a signal would, its finalizers
 * running first, and exits with `code`; returns whether a lifecycle runs
 * `runtime`. A program already stopping keeps the code it stops with.
 */
export function requestExit(runtime, code) {
  const life = lifecycles.get(runtime);
  life?.request(code);
  return life !== undefined;
}

/**
 * Runs `task` as the process's program and resolves with the exit code it
 * exits with. `proc` defaults to `process`.
 */
export function main(task, { runtime = new Runtime(), proc = process, deadline = 10_000 } = {}) {
  // The lifecycle comes first: a synchronous runtime runs the task as it is
  // forked, and the task may ask to exit before `fork` returns its fiber.
  let root;
  let stopped = false;
  const life = lifecycle(runtime, proc, deadline, () => {
    if (root === undefined) stopped = true;
    else runtime.interrupt(root);
  });
  const ended = new Promise((resolve) => (root = runtime.fork(task, resolve)));
  if (stopped) runtime.interrupt(root);
  return ended.then((exit) => {
    const { code, message } = outcome(exit, life.stopping());
    if (message !== null) console.error(message);
    return life.exit(code);
  });
}

/**
 * Runs the process of a `Platform.worker` whose `init` started it on
 * `runtime`. SIGINT or SIGTERM shuts `runtime` down and exits 130 or 143;
 * until then a live timer or subscription keeps the process alive, and
 * with none it exits 0, as Node does. `proc` defaults to `process`.
 */
export function worker(runtime, { proc = process, deadline = 10_000 } = {}) {
  const life = lifecycle(runtime, proc, deadline, (code) => life.exit(code));
}

/**
 * Handles SIGINT and SIGTERM for a program on `runtime`: the first, or an
 * exit the program requests, calls `stop` with the code to exit with and
 * arms the deadline; a signal after that, or the deadline passing, exits
 * at once. `exit(code)` shuts `runtime` down, then exits.
 */
function lifecycle(runtime, proc, deadline, stop) {
  // The code the program is stopping with, once something stopped it.
  let stopping = null;
  let deadlineTimer = null;

  const forceExit = (code, why) => {
    console.error(`nomi: ${why}; exiting now`);
    proc.exit(code);
  };
  const armDeadline = (code) => {
    if (deadlineTimer !== null) return;
    deadlineTimer = setTimeout(
      () => forceExit(code, `shutdown took longer than ${deadline} ms`),
      Math.min(deadline, LONGEST_DELAY),
    );
  };
  const handlers = Object.entries(SIGNALS).map(([signal, code]) => {
    const handler = () => {
      if (stopping !== null) return forceExit(code, `${signal} again`);
      life.request(code);
    };
    proc.on(signal, handler);
    return [signal, handler];
  });

  const life = {
    stopping: () => stopping,
    request(code) {
      if (stopping !== null) return;
      stopping = code;
      armDeadline(code);
      stop(code);
    },
    allow(ms) {
      deadline = Math.max(deadline, ms);
    },
    async exit(code) {
      armDeadline(code);
      await runtime.shutdown();
      clearTimeout(deadlineTimer);
      for (const [signal, handler] of handlers) proc.off(signal, handler);
      // Let pending output flush and the event loop empty on its own; if a
      // leaked handle keeps it alive, exit anyway.
      proc.exitCode = code;
      setTimeout(() => proc.exit(code), 1000).unref();
      return code;
    },
  };
  lifecycles.set(runtime, life);
  return life;
}

/**
 * The code the process exits with once the program ended with `exit`, and
 * the line the user should see, if any. A
 * signal, or an exit the program requested, decides the code alone; a
 * defect, as a finalizer's while stopping, is still shown.
 */
function outcome(exit, stopping) {
  if (stopping !== null) {
    const message = exit.$ === ExitKind.DEFECT ? defect(exit) : null;
    return { code: stopping, message };
  }
  switch (exit.$) {
    case ExitKind.SUCCESS:
      return { code: 0, message: null };
    case ExitKind.FAIL:
      return { code: 1, message: `nomi: the program failed: ${describeDefect(exit.error)}` };
    case ExitKind.DEFECT:
      return { code: 1, message: defect(exit) };
    case ExitKind.INTERRUPTED:
      return { code: 1, message: "nomi: the program was interrupted" };
    default:
      // A fiber ends in one of the four; anything else is a runtime bug.
      throw new TypeError(`not an exit: ${exit.$}`);
  }
}

function defect(exit) {
  return `nomi: the program died with a defect: ${describeDefect(exit.defect)}`;
}
