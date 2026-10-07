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
//
// Every fiber ends in exactly one exit: success, a typed failure, a defect
// (a JS exception or a broken contract), or interruption. A defect nobody is
// waiting for is reported through `reportDefect`; it never escapes into the
// host and never crashes the process.

const EXIT = "exit";
const SYNC = "sync";
const ASYNC = "async";
const AND_THEN = "andThen";
const ON_ERROR = "onError";
const FORK = "fork";
const AWAIT = "await";
const YIELD = "yield";

const TAGS = new Set([EXIT, SYNC, ASYNC, AND_THEN, ON_ERROR, FORK, AWAIT, YIELD]);

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

export const andThen = (task, f) => ({ $: AND_THEN, task, f });
export const onError = (task, f) => ({ $: ON_ERROR, task, f });
export const map = (task, f) => andThen(task, (value) => succeed(f(value)));

/** Starts `task` on a new fiber and succeeds with the fiber. */
export const fork = (task) => ({ $: FORK, task });

/** Waits for `fiber` and succeeds with its exit, whatever it is. */
export const awaitFiber = (fiber) => ({ $: AWAIT, fiber });

/** Waits for `fiber` and ends the same way it did. */
export const join = (fiber) => andThen(awaitFiber(fiber), done);

export const yieldNow = Object.freeze({ $: YIELD });

export function isTask(value) {
  return typeof value === "object" && value !== null && TAGS.has(value.$);
}

// ---------------------------------------------------------------------------
// Fibers

const READY = "ready";
const RUNNING = "running";
const SUSPENDED = "suspended";
const DONE = "done";

let nextFiberId = 1;

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
  // Bumped whenever a suspension ends, so a stale `resume` does nothing.
  epoch = 0;

  constructor(task) {
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
 */
export class Runtime {
  /** Live fibers by id; a finished fiber leaves at once. */
  fibers = new Map();
  stats = { started: 0, finished: 0 };

  #queue = [];
  #head = 0;
  #draining = false;
  #scheduled = false;
  #schedule;
  #yieldEvery;
  #budget;
  #reportDefect;

  constructor({ schedule = setImmediate, yieldEvery = 2048, budget = 65536, reportDefect = defaultReport } = {}) {
    this.#schedule = schedule;
    this.#yieldEvery = yieldEvery;
    this.#budget = budget;
    this.#reportDefect = reportDefect;
  }

  /** Starts `task` on a new top-level fiber. */
  fork(task) {
    return this.#spawn(task);
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

  #spawn(task) {
    const fiber = new Fiber(isTask(task) ? task : die(notATask(task)));
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
  }

  /** Runs `fiber` until it suspends, finishes or uses its slice; returns the instructions run. */
  #step(fiber) {
    const stack = fiber.stack;
    let task = fiber.task;
    fiber.status = RUNNING;
    for (let n = 1; ; n++) {
      if (n > this.#yieldEvery) {
        fiber.task = task;
        this.#enqueue(fiber);
        return n;
      }
      switch (task.$) {
        case EXIT: {
          const exit = task.exit;
          let next = null;
          while (stack.length > 0) {
            const frame = stack.pop();
            if (exit.$ === "success" && frame.$ === AND_THEN) {
              next = this.#call(frame.f, exit.value);
              break;
            }
            if (exit.$ === "fail" && frame.$ === ON_ERROR) {
              next = this.#call(frame.f, exit.error);
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
          task = succeed(this.#spawn(task.task));
          break;
        case AWAIT: {
          const target = task.fiber;
          if (!(target instanceof Fiber)) {
            task = die(new TypeError(`await expects a fiber, got ${describe(target)}`));
            break;
          }
          if (target.exit !== null) {
            task = succeed(target.exit);
            break;
          }
          this.#suspend(fiber);
          const epoch = fiber.epoch;
          target.observers.add((exit) => this.#resume(fiber, epoch, succeed(exit)));
          return n;
        }
        case ASYNC: {
          this.#suspend(fiber);
          const epoch = fiber.epoch;
          const resume = (next) => this.#resume(fiber, epoch, isTask(next) ? next : die(notATask(next)));
          try {
            task.register(resume);
          } catch (error) {
            if (fiber.epoch === epoch && fiber.status === SUSPENDED) {
              // Nothing resumed it yet: the exception is how it ends.
              fiber.epoch++;
              fiber.status = RUNNING;
              task = die(error);
              break;
            }
            // It already resumed, so the exception has no fiber to land in.
            this.#report(error, fiber);
          }
          return n;
        }
        case YIELD:
          fiber.task = succeed(undefined);
          this.#enqueue(fiber);
          return n;
        default:
          task = die(notATask(task));
      }
    }
  }

  #suspend(fiber) {
    fiber.status = SUSPENDED;
    fiber.task = null;
  }

  #resume(fiber, epoch, task) {
    if (fiber.epoch !== epoch || fiber.status !== SUSPENDED) return;
    fiber.epoch++;
    fiber.task = task;
    this.#enqueue(fiber);
  }

  #finish(fiber, exit) {
    fiber.exit = exit;
    fiber.status = DONE;
    fiber.task = null;
    fiber.stack.length = 0;
    this.fibers.delete(fiber.id);
    this.stats.finished++;
    const observers = [...fiber.observers];
    fiber.observers.clear();
    if (exit.$ === "defect" && observers.length === 0) this.#report(exit.defect, fiber);
    for (const observe of observers) observe(exit);
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
