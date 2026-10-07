// The fiber runtime (architecture §8, "Runtime").
//
// A task is an immutable description; running one makes a fiber. The
// interpreter is shaped like elm/core's Scheduler.js, a loop over a stack of
// continuation frames, so the Elm kernel can sit on top of it unchanged:
//
//   exit      the task has a result; frames unwind until one handles it
//   sync      call a JS function; an exception becomes a defect
//   async     suspend until the host calls `resume` with the next task
//   andThen   continue with `f(value)` on success
//   onError   continue with `f(error)` on a typed failure
//   fork      start a child fiber, succeed with its handle
//   await     wait for a fiber, succeed with its exit
//   yield     go to the back of the run queue
//   sleep     suspend until the runtime's clock passes a delay
//   now       succeed with the clock's time
//   mask      run a task uninterruptibly
//   scoped    run a task in a new scope, closed when it ends
//   interrupt ask a fiber to stop and wait for its exit
//
// Structured concurrency: every fiber belongs to a scope. A fiber has its
// own scope for its whole life, and `scoped` opens narrower ones. A scope
// holds the fibers forked in it and the finalizers added to it; closing it
// interrupts those fibers, waits for them, then runs the finalizers in LIFO
// order, all uninterruptibly. `forkDaemon` attaches to the runtime's root
// scope instead, which is upstream's `Process.spawn`: it outlives its parent.
//
// Interruption is a request, delivered only at safe points: when the fiber
// passes through the run queue (every yield, including the forced ones),
// when it is about to suspend, and while it is suspended, which cancels the
// operation it waits on (an `async` cancel function, or the AbortSignal of
// a host call). Inside an uninterruptible region it waits until the region
// ends. Delivery replaces the fiber's next step with an `interrupt` exit,
// which only scope closing observes.
//
// Every fiber ends in exactly one exit: success, a typed failure, a defect
// (a JS exception or a broken contract), or interruption. A defect nobody is
// waiting for is reported through `reportDefect`; it never escapes into the
// host and never crashes the process.

import { RealClock } from "./clock.js";

const EXIT = "exit";
const SYNC = "sync";
const ASYNC = "async";
const AND_THEN = "andThen";
const ON_ERROR = "onError";
const FORK = "fork";
const AWAIT = "await";
const YIELD = "yield";
const MASK = "mask";
const SCOPED = "scoped";
const INTERRUPT = "interrupt";
const SLEEP = "sleep";
const NOW = "now";
// Internal: only this module builds these.
const WITH_FIBER = "withFiber";
const REAP = "reap";

const TAGS = new Set([EXIT, SYNC, ASYNC, AND_THEN, ON_ERROR, FORK, AWAIT, YIELD, MASK, SCOPED, INTERRUPT, SLEEP, NOW]);
const INTERNAL = new Set([WITH_FIBER, REAP]);

// Frames that only the unwinding loop sees.
const ON_EXIT = "onExit";
const RESTORE = "restore";
const SCOPE = "scope";
const CLOSED = "closed";

// ---------------------------------------------------------------------------
// Exits

export const Exit = Object.freeze({
  success: (value) => ({ $: "success", value }),
  fail: (error) => ({ $: "fail", error }),
  defect: (defect) => ({ $: "defect", defect }),
  interrupt: () => ({ $: "interrupt" }),
});

// ---------------------------------------------------------------------------
// Tasks

export const done = (exit) => ({ $: EXIT, exit });
export const succeed = (value) => done(Exit.success(value));
export const fail = (error) => done(Exit.fail(error));
export const die = (defect) => done(Exit.defect(defect));

/** Calls `f()` when run; its result is the value, its exception a defect. */
export const sync = (f) => ({ $: SYNC, f });

/**
 * Suspends the fiber and calls `register(resume)`. The host calls
 * `resume(task)` once to continue the fiber with `task`; later calls are
 * ignored. `register` may return a function that cancels the operation.
 */
export const async = (register) => ({ $: ASYNC, register });

/**
 * Calls the host function `f(signal)` and waits for the promise it returns:
 * fulfilled is success, rejected a typed failure, and a synchronous throw a
 * defect. Interrupting the fiber aborts `signal`, and whatever the promise
 * does afterwards is ignored.
 */
export const fromPromise = (f) =>
  async((resume) => {
    const controller = new AbortController();
    Promise.resolve(f(controller.signal)).then(
      (value) => resume(succeed(value)),
      (error) => resume(fail(error)),
    );
    return () => controller.abort(new Interrupted());
  });

/** The reason an interrupted host call's signal is aborted with. */
export class Interrupted extends Error {
  name = "Interrupted";
  constructor() {
    super("the fiber was interrupted");
  }
}

export const andThen = (task, f) => ({ $: AND_THEN, task, f });
export const onError = (task, f) => ({ $: ON_ERROR, task, f });
export const map = (task, f) => andThen(task, (value) => succeed(f(value)));

/** Starts `task` on a child fiber in the current scope and succeeds with the fiber. */
export const fork = (task) => ({ $: FORK, task, daemon: false });

/** Starts `task` on a fiber in the root scope, so it outlives its parent. */
export const forkDaemon = (task) => ({ $: FORK, task, daemon: true });

/** Waits for `fiber` and succeeds with its exit, whatever it is. */
export const awaitFiber = (fiber) => ({ $: AWAIT, fiber });

/** Waits for `fiber` and ends the same way it did. */
export const join = (fiber) => andThen(awaitFiber(fiber), done);

export const yieldNow = Object.freeze({ $: YIELD });

/** Suspends for `ms` milliseconds of the runtime's clock. */
export const sleep = (ms) => ({ $: SLEEP, ms });

/** Succeeds with the runtime's clock time, in milliseconds. */
export const now = Object.freeze({ $: NOW });

/** Runs `task` with interruption deferred until it ends. */
export const uninterruptible = (task) => ({ $: MASK, task });

/** Runs `task` in a new scope, closed when `task` ends, however it ends. */
export const scoped = (task) => ({ $: SCOPED, task });

/** Asks `fiber` to stop, waits for it, and succeeds with its exit. */
export const interrupt = (fiber) => ({ $: INTERRUPT, fiber });

// Internal: runs `f(fiber)` and continues with the task it returns.
const withFiber = (f) => ({ $: WITH_FIBER, f });
// Suspends until every fiber in `fibers` is done, without counting as
// waiting for them, so their defects are still reported.
const reap = (fibers) => ({ $: REAP, fibers });
// Succeeds with the exit of `task`, whatever it is.
const attempt = (task) => withFiber((fiber) => (fiber.stack.push({ $: ON_EXIT, f: succeed }), task));

/**
 * Adds `finalizer(exit)` to the current scope. It runs once, when the scope
 * closes, with the exit the scope closed with; it must not fail.
 */
export const addFinalizer = (finalizer) =>
  withFiber((fiber) => {
    if (fiber.scope === null) return die(new Error("addFinalizer outside any scope"));
    fiber.scope.finalizers.push(finalizer);
    return succeed(undefined);
  });

/**
 * Acquires a resource uninterruptibly and registers `release(resource, exit)`
 * in the current scope, so it runs exactly once, however the scope ends.
 */
export const acquireRelease = (acquire, release) =>
  uninterruptible(andThen(acquire, (resource) => map(addFinalizer((exit) => release(resource, exit)), () => resource)));

/** Runs `finalizer` after `task`, however `task` ends, once `task` has started. */
export const ensuring = (task, finalizer) =>
  // Opening the scope and registering the finalizer are one step, so no
  // interruption can land between them.
  withFiber((fiber) => {
    const scope = new Scope(fiber);
    scope.finalizers.push(() => finalizer);
    fiber.stack.push({ $: SCOPE, scope, previous: fiber.scope });
    fiber.scope = scope;
    return task;
  });

export function isTask(value) {
  return typeof value === "object" && value !== null && (TAGS.has(value.$) || INTERNAL.has(value.$));
}

// ---------------------------------------------------------------------------
// Fibers

const READY = "ready";
const RUNNING = "running";
const SUSPENDED = "suspended";
const DONE = "done";

let nextFiberId = 1;

/** Fibers and finalizers that end together. */
export class Scope {
  fibers = new Set();
  finalizers = [];
  /** The fiber the scope belongs to; null for the runtime's root scope. */
  owner;

  constructor(owner) {
    this.owner = owner;
  }
}

export class Fiber {
  /** Unique within the process, for reports and debugging. */
  id = nextFiberId++;
  status = READY;
  /** The exit, once the fiber is done. */
  exit = null;
  // The task to run next and the continuation frames below it.
  task;
  stack = [];
  // Callbacks that receive the exit; each counts as someone waiting.
  observers = new Set();
  // Callbacks run at the end that do not count as waiting.
  reapers = new Set();
  // Bumped whenever a suspension ends, so a stale `resume` does nothing.
  epoch = 0;
  // Undoes the current suspension, if it can be undone.
  cancel = null;
  // Whether the current suspension waits on the host (I/O), which keeps a
  // test clock still.
  io = false;
  /** The scope the fiber was forked into. */
  parent;
  /** The innermost open scope; forks and finalizers go here. */
  scope = null;
  // Depth of uninterruptible regions, and whether an interruption waits
  // for it to reach zero.
  mask = 0;
  interruptPending = false;

  constructor(task, parent) {
    this.parent = parent;
    // The fiber's own scope encloses everything it runs.
    const scope = new Scope(this);
    this.stack.push({ $: SCOPE, scope, previous: null });
    this.scope = scope;
    this.task = task;
  }
}

// ---------------------------------------------------------------------------
// The runtime

function defaultReport(defect, fiber) {
  console.error(`nomi: fiber ${fiber.id} died with a defect:`, defect);
}

/**
 * Owns the run queue and the live fibers.
 *
 * - `schedule(fn)` hands control back to the host and calls `fn` later; the
 *   default is `setImmediate`, so I/O callbacks run between slices.
 * - `yieldEvery` is how many instructions a fiber runs before it goes to the
 *   back of the queue, so one busy fiber cannot starve the others.
 * - `budget` is how many instructions run before control goes back to the
 *   host, so a busy runtime cannot starve the event loop.
 * - `clock` serves `sleep` and `now`; see clock.js.
 */
export class Runtime {
  /** Live fibers by id; a finished fiber leaves at once. */
  fibers = new Map();
  /** Where top-level fibers and daemons live; it is never closed. */
  root = new Scope(null);
  stats = { started: 0, finished: 0 };

  #queue = [];
  #head = 0;
  #draining = false;
  #scheduled = false;
  #schedule;
  #yieldEvery;
  #budget;
  #reportDefect;
  #clock;
  // Fibers suspended on the host.
  #io = 0;

  constructor({
    schedule = setImmediate,
    yieldEvery = 2048,
    budget = 65536,
    reportDefect = defaultReport,
    clock = new RealClock(),
  } = {}) {
    this.#schedule = schedule;
    this.#clock = clock;
    this.#yieldEvery = yieldEvery;
    this.#budget = budget;
    this.#reportDefect = reportDefect;
  }

  /** Starts `task` on a new fiber in the root scope. */
  fork(task) {
    return this.#spawn(task, this.root);
  }

  /** Asks `fiber` to stop and resolves with its exit. */
  interrupt(fiber) {
    this.#interrupt(fiber);
    return this.await(fiber);
  }

  /** Resolves with `fiber`'s exit; the caller counts as waiting for it. */
  await(fiber) {
    if (fiber.exit !== null) return Promise.resolve(fiber.exit);
    return new Promise((resolve) => fiber.observers.add(resolve));
  }

  /** Runs `task` on a new fiber and resolves with its exit. */
  run(task) {
    return this.await(this.fork(task));
  }

  /** Interrupts every fiber in the root scope and resolves once all are done. */
  shutdown() {
    const fibers = [...this.root.fibers];
    for (const fiber of fibers) this.#interrupt(fiber);
    return new Promise((resolve) => {
      let left = fibers.length;
      if (left === 0) return resolve();
      const reaper = () => --left === 0 && resolve();
      for (const fiber of fibers) {
        if (fiber.exit !== null) reaper();
        else fiber.reapers.add(reaper);
      }
    });
  }

  #spawn(task, scope) {
    const fiber = new Fiber(isTask(task) ? task : die(notATask(task)), scope);
    scope.fibers.add(fiber);
    this.fibers.set(fiber.id, fiber);
    this.stats.started++;
    this.#enqueue(fiber);
    return fiber;
  }

  #enqueue(fiber) {
    fiber.status = READY;
    this.#queue.push(fiber);
    this.#wake();
  }

  #wake() {
    if (this.#draining || this.#scheduled) return;
    this.#scheduled = true;
    this.#schedule(() => {
      this.#scheduled = false;
      this.#drain();
    });
  }

  #drain() {
    this.#draining = true;
    let steps = 0;
    while (this.#head < this.#queue.length && steps < this.#budget) {
      const fiber = this.#queue[this.#head++];
      steps += this.#step(fiber);
    }
    // Drop the consumed prefix once it dominates, keeping pushes O(1).
    if (this.#head > 1024 && this.#head * 2 > this.#queue.length) {
      this.#queue = this.#queue.slice(this.#head);
      this.#head = 0;
    }
    this.#draining = false;
    if (this.#head < this.#queue.length) this.#wake();
    else if (this.#io === 0) this.#clock.idle();
  }

  /** Runs `fiber` until it suspends, finishes or uses its slice; returns the instructions run. */
  #step(fiber) {
    const stack = fiber.stack;
    let task = this.#takeInterrupt(fiber) ? done(Exit.interrupt()) : fiber.task;
    fiber.status = RUNNING;
    for (let n = 1; ; n++) {
      if (n > this.#yieldEvery) {
        fiber.task = task;
        this.#enqueue(fiber);
        return n;
      }
      switch (task.$) {
        case EXIT: {
          let exit = task.exit;
          let next = null;
          while (next === null && stack.length > 0) {
            const frame = stack.pop();
            switch (frame.$) {
              case AND_THEN:
                if (exit.$ === "success") next = this.#call(frame.f, exit.value);
                break;
              case ON_ERROR:
                if (exit.$ === "fail") next = this.#call(frame.f, exit.error);
                break;
              case ON_EXIT:
                next = this.#call(frame.f, exit);
                break;
              case RESTORE:
                fiber.mask--;
                exit = this.#unmask(fiber, exit);
                break;
              case SCOPE:
                next = this.#close(fiber, frame, exit);
                break;
              case CLOSED:
                // The closing task succeeds with the exit to carry on with.
                fiber.mask--;
                exit = this.#unmask(fiber, exit.$ === "success" ? exit.value : exit);
                break;
            }
          }
          if (next === null) {
            this.#finish(fiber, exit);
            return n;
          }
          task = next;
          break;
        }
        case SYNC:
          task = this.#callValue(task.f);
          break;
        case AND_THEN:
        case ON_ERROR:
          stack.push(task);
          task = task.task;
          break;
        case FORK:
          task = succeed(this.#spawn(task.task, task.daemon ? this.root : (fiber.scope ?? this.root)));
          break;
        case AWAIT:
        case INTERRUPT: {
          const target = task.fiber;
          if (!(target instanceof Fiber)) {
            task = die(new TypeError(`${task.$} expects a fiber, got ${describe(target)}`));
            break;
          }
          if (task.$ === INTERRUPT) {
            if (target === fiber) {
              // Interrupting yourself takes effect as soon as it may.
              fiber.interruptPending = true;
              task = this.#takeInterrupt(fiber) ? done(Exit.interrupt()) : succeed(Exit.interrupt());
              break;
            }
            this.#interrupt(target);
          }
          if (target.exit !== null) {
            task = succeed(target.exit);
            break;
          }
          if (this.#takeInterrupt(fiber)) {
            task = done(Exit.interrupt());
            break;
          }
          this.#suspend(fiber);
          const epoch = fiber.epoch;
          const observe = (exit) => this.#resume(fiber, epoch, succeed(exit));
          target.observers.add(observe);
          fiber.cancel = () => target.observers.delete(observe);
          return n;
        }
        case REAP: {
          const live = task.fibers.filter((f) => f.exit === null);
          if (live.length === 0) {
            task = succeed(undefined);
            break;
          }
          this.#suspend(fiber);
          const epoch = fiber.epoch;
          let left = live.length;
          const reaper = () => {
            if (--left === 0) this.#resume(fiber, epoch, succeed(undefined));
          };
          for (const f of live) f.reapers.add(reaper);
          fiber.cancel = () => live.forEach((f) => f.reapers.delete(reaper));
          return n;
        }
        case ASYNC: {
          if (this.#takeInterrupt(fiber)) {
            task = done(Exit.interrupt());
            break;
          }
          this.#suspend(fiber, true);
          const epoch = fiber.epoch;
          const resume = (next) => this.#resume(fiber, epoch, isTask(next) ? next : die(notATask(next)));
          try {
            const cancel = task.register(resume);
            if (fiber.epoch === epoch && typeof cancel === "function") fiber.cancel = cancel;
          } catch (error) {
            if (fiber.epoch === epoch && fiber.status === SUSPENDED) {
              // Nothing resumed it yet: the exception is how it ends.
              this.#endSuspension(fiber);
              fiber.status = RUNNING;
              task = die(error);
              break;
            }
            // It already resumed, so the exception has no fiber to land in.
            this.#report(error, fiber);
          }
          return n;
        }
        case SLEEP: {
          if (this.#takeInterrupt(fiber)) {
            task = done(Exit.interrupt());
            break;
          }
          this.#suspend(fiber);
          const epoch = fiber.epoch;
          fiber.cancel = this.#clock.setTimer(task.ms, () => this.#resume(fiber, epoch, succeed(undefined)));
          return n;
        }
        case NOW:
          task = this.#callValue(() => this.#clock.now());
          break;
        case YIELD:
          fiber.task = succeed(undefined);
          this.#enqueue(fiber);
          return n;
        case MASK:
          fiber.mask++;
          stack.push({ $: RESTORE });
          task = task.task;
          break;
        case SCOPED: {
          const scope = new Scope(fiber);
          stack.push({ $: SCOPE, scope, previous: fiber.scope });
          fiber.scope = scope;
          task = task.task;
          break;
        }
        case WITH_FIBER:
          task = this.#call(task.f, fiber);
          break;
        default:
          task = die(notATask(task));
      }
    }
  }

  #suspend(fiber, io = false) {
    fiber.status = SUSPENDED;
    fiber.task = null;
    fiber.io = io;
    if (io) this.#io++;
  }

  /** Ends the current suspension, so its `resume` and `cancel` go stale. */
  #endSuspension(fiber) {
    fiber.epoch++;
    fiber.cancel = null;
    if (fiber.io) {
      fiber.io = false;
      this.#io--;
    }
  }

  #resume(fiber, epoch, task) {
    if (fiber.epoch !== epoch || fiber.status !== SUSPENDED) return;
    this.#endSuspension(fiber);
    fiber.task = task;
    this.#enqueue(fiber);
  }

  /** Requests interruption; a suspended, interruptible fiber stops waiting at once. */
  #interrupt(fiber) {
    if (fiber.exit !== null) return;
    fiber.interruptPending = true;
    if (fiber.status !== SUSPENDED || !this.#takeInterrupt(fiber)) return;
    const cancel = fiber.cancel;
    this.#endSuspension(fiber);
    if (cancel !== null) {
      try {
        cancel();
      } catch (error) {
        this.#report(error, fiber);
      }
    }
    fiber.task = done(Exit.interrupt());
    this.#enqueue(fiber);
  }

  /** Whether a pending interruption may be delivered now; delivering clears it. */
  #takeInterrupt(fiber) {
    if (!fiber.interruptPending || fiber.mask > 0) return false;
    fiber.interruptPending = false;
    return true;
  }

  /** At the end of an uninterruptible region, a pending interruption replaces the exit, unless it is a defect. */
  #unmask(fiber, exit) {
    if (!this.#takeInterrupt(fiber)) return exit;
    return exit.$ === "defect" ? exit : Exit.interrupt();
  }

  /**
   * Starts closing `frame.scope` with `exit`, uninterruptibly: interrupt the
   * scope's fibers, wait for them, then run its finalizers last to first.
   * Returns the task that does it; it succeeds with the exit to continue
   * with, which is `exit` unless a finalizer failed or died.
   */
  #close(fiber, frame, exit) {
    const { scope } = frame;
    fiber.scope = frame.previous;
    fiber.mask++;
    fiber.stack.push({ $: CLOSED });
    const children = [...scope.fibers];
    for (const child of children) this.#interrupt(child);
    const finalizers = scope.finalizers.splice(0).reverse();
    let result = exit;
    const settle = (finalized) => {
      if (finalized.$ !== "fail" && finalized.$ !== "defect") return;
      const defect = finalized.$ === "fail" ? finalized.error : finalized.defect;
      // The first defect is the exit; later ones would be lost, so report them.
      if (result.$ === "defect") this.#report(defect, fiber);
      else result = Exit.defect(defect);
    };
    const run = (i) =>
      i === finalizers.length
        ? succeed(result)
        : andThen(attempt(andThen(succeed(exit), finalizers[i])), (finalized) => (settle(finalized), run(i + 1)));
    return andThen(reap(children), () => run(0));
  }

  #finish(fiber, exit) {
    fiber.exit = exit;
    fiber.status = DONE;
    fiber.task = null;
    fiber.stack.length = 0;
    fiber.scope = null;
    fiber.parent.fibers.delete(fiber);
    this.fibers.delete(fiber.id);
    this.stats.finished++;
    const observers = [...fiber.observers];
    const reapers = [...fiber.reapers];
    fiber.observers.clear();
    fiber.reapers.clear();
    if (exit.$ === "defect" && observers.length === 0) this.#report(exit.defect, fiber);
    for (const observe of observers) observe(exit);
    for (const reaper of reapers) reaper();
  }

  #report(defect, fiber) {
    try {
      this.#reportDefect(defect, fiber);
    } catch {
      // A broken reporter must not take the runtime down with it.
    }
  }

  #call(f, value) {
    try {
      const next = f(value);
      return isTask(next) ? next : die(notATask(next));
    } catch (error) {
      return die(error);
    }
  }

  #callValue(f) {
    try {
      return succeed(f());
    } catch (error) {
      return die(error);
    }
  }
}

function notATask(value) {
  return new TypeError(`expected a task, got ${describe(value)}`);
}

function describe(value) {
  if (value === null || value === undefined) return String(value);
  if (typeof value === "object") return value.constructor?.name ?? "an object";
  return typeof value;
}
