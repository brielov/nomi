// Running a program as a process (architecture §8, "Lifecycle").
//
// The program's task runs on the root fiber. SIGINT or SIGTERM interrupts
// it, so its finalizers run, and the process exits with 128 plus the signal
// number: 130 or 143. Once the root fiber ends, the fibers it left in the
// root scope (`Process.spawn`) are interrupted too. Shutting down must
// finish within a deadline; past it, or on a second signal, the process
// exits at once.

import { Runtime } from "./fiber.js";

const SIGNALS = { SIGINT: 130, SIGTERM: 143 };

/**
 * Runs `task` as the process's program and resolves with the exit code it
 * exits with. `proc` defaults to `process`; `log` receives what the user
 * should see when the program ends badly.
 */
export function main(task, { runtime = new Runtime(), proc = process, deadline = 10_000, log = console.error } = {}) {
  let signalled = null;
  let deadlineTimer = null;
  const root = runtime.fork(task);

  const forceExit = (code, why) => {
    log(`nomi: ${why}; exiting now`);
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
      runtime.interrupt(root);
    };
    proc.on(signal, handler);
    return [signal, handler];
  });

  return runtime.await(root).then(async (exit) => {
    const code = signalled ?? codeFor(exit, log);
    armDeadline(code);
    await runtime.shutdown();
    clearTimeout(deadlineTimer);
    for (const [signal, handler] of handlers) proc.off(signal, handler);
    // Let pending output flush and the event loop empty on its own; if a
    // leaked handle keeps it alive, exit anyway.
    proc.exitCode = code;
    setTimeout(() => proc.exit(code), 1000).unref();
    return code;
  });
}

function codeFor(exit, log) {
  switch (exit.$) {
    case "success":
      return 0;
    case "fail":
      log("nomi: the program failed:", exit.error);
      return 1;
    case "defect":
      log("nomi: the program died with a defect:", exit.defect);
      return 1;
    case "interrupt":
      log("nomi: the program was interrupted");
      return 1;
  }
}
