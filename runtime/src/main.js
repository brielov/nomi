// Running a program as a process (architecture §8, "Lifecycle").
//
// The program's task runs on the root fiber. SIGINT or SIGTERM interrupts
// it, so its finalizers run, and the process exits with 128 plus the signal
// number: 130 or 143. Once the root fiber ends, the fibers it left in the
// root scope (`Process.spawn`) are interrupted too. Shutting down must
// finish within a deadline; past it, or on a second signal, the process
// exits at once. A `Platform.worker` has no root fiber: a signal shuts its
// runtime down directly, and until one comes it lives as long as Node's
// event loop does.

import { ExitKind, Runtime } from "./fiber.js";

const SIGNALS = { SIGINT: 130, SIGTERM: 143 };

/**
 * Runs `task` as the process's program and resolves with the exit code it
 * exits with. `proc` defaults to `process`.
 */
export function main(task, { runtime = new Runtime(), proc = process, deadline = 10_000 } = {}) {
  const root = runtime.fork(task);
  const life = lifecycle(runtime, proc, deadline, () => runtime.interrupt(root));
  return runtime.await(root).then((exit) => {
    const { code, message } = outcome(exit, life.signalled());
    if (message !== null) console.error(...message);
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
 * Handles SIGINT and SIGTERM for a program on `runtime`: the first calls
 * `stop` with the code to exit with, and arms the deadline; a second, or
 * the deadline passing, exits at once. `exit(code)` shuts `runtime` down,
 * then exits.
 */
function lifecycle(runtime, proc, deadline, stop) {
  let signalled = null;
  let deadlineTimer = null;

  const forceExit = (code, why) => {
    console.error(`nomi: ${why}; exiting now`);
    proc.exit(code);
  };
  const armDeadline = (code) => {
    if (deadlineTimer !== null) return;
    deadlineTimer = setTimeout(() => forceExit(code, `shutdown took longer than ${deadline} ms`), deadline);
  };
  const handlers = Object.entries(SIGNALS).map(([signal, code]) => {
    const handler = () => {
      if (signalled !== null) return forceExit(code, `${signal} again`);
      signalled = code;
      armDeadline(code);
      stop(code);
    };
    proc.on(signal, handler);
    return [signal, handler];
  });

  return {
    signalled: () => signalled,
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
}

/**
 * The code the process exits with once the program ended with `exit`, and
 * what the user should see, if anything, as `console.error`'s arguments. A
 * signal decides the code alone.
 */
function outcome(exit, signalled) {
  if (signalled !== null) return { code: signalled, message: null };
  switch (exit.$) {
    case ExitKind.SUCCESS:
      return { code: 0, message: null };
    case ExitKind.FAIL:
      return { code: 1, message: ["nomi: the program failed:", exit.error] };
    case ExitKind.DEFECT:
      return { code: 1, message: ["nomi: the program died with a defect:", exit.defect] };
    case ExitKind.INTERRUPTED:
      return { code: 1, message: ["nomi: the program was interrupted"] };
    default:
      // A fiber ends in one of the four; anything else is a runtime bug.
      throw new TypeError(`not an exit: ${exit.$}`);
  }
}
