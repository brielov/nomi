// The fiber runtime (architecture §8, "Runtime").
//
// A task is an immutable description; running one makes a fiber. The
// interpreter is shaped like elm/core's Scheduler.js, a loop over a stack of
// continuation frames, so the Elm kernel can sit on top of it unchanged:
//
//   exit      the task has a result; frames unwind until one handles it
//   sync      call a JS function; an exception becomes a defect
//   async     suspend until the host calls `resume` with the next task
//   wait      suspend until another fiber calls `resume` with the next task
//   andThen   continue with `f(value)` on success
//   onError   continue with `f(error)` on a typed failure
//   fork      start a child fiber, succeed with its handle
//   await     wait for a fiber, succeed with its exit
//   yield     go to the back of the run queue
//   sleep     suspend until the runtime's clock passes a delay
//   now       succeed with the clock's time
//   mask      run a task uninterruptibly
//   scoped    run a task in a new scope, closed when it ends
//   interrupt ask a fiber to stop and wait for its exit, or not
//   receive   continue with `f(message)` once the fiber's mailbox has one
//
// Structured concurrency: every fiber belongs to a scope. A fiber has its
// own scope for its whole life, and `scoped` opens narrower ones. A scope
// holds the fibers forked in it and the finalizers added to it; closing it
// interrupts those fibers, waits for them, then runs the finalizers in LIFO
// order, all uninterruptibly. `forkDaemon` attaches to the runtime's root
// scope instead, which is upstream's `Process.spawn`: it outlives its parent.
// `all` and `race` fork their tasks into a scope of their own, so those
// still running once the outcome is known are interrupted before it returns.
// A server's requests live in a `draining` scope: closing it first starts
// nothing more and gives its fibers until a deadline on the runtime's clock
// to end on their own, and only then interrupts the rest.
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
//
// Every fiber has a mailbox, elm/core's: `Runtime.send` puts a message in
// it and `receive` takes the oldest, suspending while there is none.

import { RealClock } from "./clock.js";

const EXIT = "exit";
const SYNC = "sync";
const ASYNC = "async";
const WAIT = "wait";
const AND_THEN = "andThen";
const ON_ERROR = "onError";
const FORK = "fork";
const AWAIT = "await";
const YIELD = "yield";
const MASK = "mask";
const SCOPED = "scoped";
// Not "interrupt", the exit kind: no exit may pass for a task.
const INTERRUPT = "interruptFiber";
const SLEEP = "sleep";
const NOW = "now";
const RECEIVE = "receive";
// Internal: only this module builds these.
const WITH_FIBER = "withFiber";
const REAP = "reap";
const GATHER = "gather";

const TAGS = new Set([EXIT, SYNC, ASYNC, WAIT, AND_THEN, ON_ERROR, FORK, AWAIT, YIELD, MASK, SCOPED, INTERRUPT, SLEEP, NOW, RECEIVE]);
const INTERNAL = new Set([WITH_FIBER, REAP, GATHER]);

// Frames that only the unwinding loop sees.
const ON_EXIT = "onExit";
const RESTORE = "restore";
const SCOPE = "scope";
const CLOSED = "closed";

// ---------------------------------------------------------------------------
// Exits

const SUCCESS = "success";
const FAIL = "fail";
const DEFECT = "defect";
const INTERRUPTED = "interrupt";

/** The four ways a fiber ends, as the `$` of its exit. */
export const ExitKind = Object.freeze({ SUCCESS, FAIL, DEFECT, INTERRUPTED });

/** Builds exits: a value, a typed failure, a defect, or interruption. */
export const Exit = Object.freeze({
  success: (value) => ({ $: SUCCESS, value }),
  fail: (error) => ({ $: FAIL, error }),
  defect: (defect) => ({ $: DEFECT, defect }),
  interrupt: () => ({ $: INTERRUPTED }),
});

// ---------------------------------------------------------------------------
// Tasks

const done = (exit) => ({ $: EXIT, exit });

/** Succeeds with `value`. */
export const succeed = (value) => done(Exit.success(value));

/** Fails with the typed error `error`, which `onError` can handle. */
export const fail = (error) => done(Exit.fail(error));

/** Dies with `defect`, a broken contract that `onError` does not handle. */
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

/**
 * Suspends the fiber as `async` does, for another fiber rather than the
 * host, so the clock does not wait for it. The task given to `resume` runs
 * before any interruption is delivered, so what a waker hands over, such
 * as a queue's item, is never dropped; `register`'s cancel function, which
 * interruption calls while the fiber waits, takes the waiter back.
 */
export const wait = (register) => ({ $: WAIT, register });

/** The reason an interrupted host call's signal is aborted with. */
export class Interrupted extends Error {
  name = "Interrupted";
  constructor() {
    super("the fiber was interrupted");
  }
}

/** Continues with `f(value)` when `task` succeeds; failures, defects and interruption pass through. */
export const andThen = (task, f) => ({ $: AND_THEN, task, f });

/** Continues with `f(error)` when `task` fails; success, defects and interruption pass through. */
export const onError = (task, f) => ({ $: ON_ERROR, task, f });

/** Succeeds with `f(value)` when `task` succeeds; anything else passes through. */
export const map = (task, f) => andThen(task, (value) => succeed(f(value)));

/** Starts `task` on a child fiber in the current scope and succeeds with the fiber. */
export const fork = (task) => ({ $: FORK, task, daemon: false, scope: null });

/** Starts `task` on a fiber in the root scope, so it outlives its parent. */
export const forkDaemon = (task) => ({ $: FORK, task, daemon: true, scope: null });

/** Waits for `fiber` and succeeds with its exit, whatever it is. */
export const awaitFiber = (fiber) => ({ $: AWAIT, fiber });

/** Waits for `fiber` and ends the same way it did. */
export const join = (fiber) => andThen(awaitFiber(fiber), done);

/** Goes to the back of the run queue, so the other fibers run first. */
export const yieldNow = Object.freeze({ $: YIELD });

/** Suspends for `ms` milliseconds of the runtime's clock. */
export const sleep = (ms) => ({ $: SLEEP, ms });

/** Succeeds with the runtime's clock time, in milliseconds. */
export const now = Object.freeze({ $: NOW });

/** Runs `task` with interruption deferred until it ends. */
export const uninterruptible = (task) => ({ $: MASK, task });

/** Runs `task` in a new scope, closed when `task` ends, however it ends. */
export const scoped = (task) => ({ $: SCOPED, task });

/**
 * Runs `use(spawn)` in a new scope that drains when it closes, however
 * `use`'s task ends: from then on `spawn` starts nothing, the scope's fibers
 * get `ms` milliseconds of the runtime's clock to end on their own, and the
 * rest are interrupted; then its finalizers run. `spawn(task)` forks `task`
 * into this scope from anywhere inside, a nested scope included; once the
 * scope is closing, the fiber it gives is interrupted before it runs.
 */
export const draining = (ms, use) =>
  withFiber((fiber) => {
    const scope = openScope(fiber, new DrainingScope(ms));
    return call(use, (task) => ({ $: FORK, task, daemon: false, scope }));
  });

/** Asks `fiber` to stop, waits for it, and succeeds with its exit. */
export const interrupt = (fiber) => ({ $: INTERRUPT, fiber, wait: true });

/** Asks `fiber` to stop and succeeds at once, without waiting for it. */
export const interruptFork = (fiber) => ({ $: INTERRUPT, fiber, wait: false });

/** Continues with `f(message)`, the oldest message in the fiber's mailbox, waiting for one if there is none. */
export const receive = (f) => ({ $: RECEIVE, f });

// Internal: runs `f(fiber)` and continues with the task it returns.
const withFiber = (f) => ({ $: WITH_FIBER, f });
// Suspends until every fiber in `fibers` is done, or until `ms` pass if it
// is given, without counting as waiting for them, so their defects are
// still reported.
const reap = (fibers, ms) => ({ $: REAP, fibers, ms });
// Forks `tasks` in the current scope and hands each one's exit, as it
// ends, to `step(exit, index)`, until `step` returns the task to continue
// with rather than `undefined`. Waiting through it counts as waiting.
const gather = (tasks, step) => ({ $: GATHER, tasks, step });
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
  withFiber((fiber) => (openScope(fiber).finalizers.push(() => finalizer), task));

/**
 * Runs `tasks` concurrently, each on a child fiber, and succeeds with their
 * values in order. The first to end otherwise ends it the same way, and the
 * rest are interrupted.
 */
export const all = (tasks) =>
  tasks.length === 0
    ? succeed([])
    : scoped(
        withFiber(() => {
          const values = new Array(tasks.length);
          let left = tasks.length;
          return gather(tasks, (exit, i) => {
            if (exit.$ !== SUCCESS) return done(exit);
            values[i] = exit.value;
            return --left === 0 ? succeed(values) : undefined;
          });
        }),
      );

/**
 * Runs `tasks` concurrently, each on a child fiber, and ends as the first
 * of them to end, however it ends; the rest are interrupted. With no tasks
 * it never ends.
 */
export const race = (tasks) => scoped(gather(tasks, done));

function isTask(value) {
  return typeof value === "object" && value !== null && (TAGS.has(value.$) || INTERNAL.has(value.$));
}

/** `value` if it is a task, else a defect saying it is not. */
function asTask(value) {
  return isTask(value) ? value : die(notATask(value));
}

// ---------------------------------------------------------------------------
// Fibers

const READY = "ready";
const RUNNING = "running";
const SUSPENDED = "suspended";
const DONE = "done";

let nextFiberId = 1;

/** Fibers and finalizers that end together. */
class Scope {
  fibers = new Set();
  finalizers = [];
}

/** A scope that lets its fibers end on their own, for `ms` milliseconds, before it interrupts them. */
class DrainingScope extends Scope {
  closing = false;

  constructor(ms) {
    super();
    this.ms = ms;
  }
}

/** Opens `scope` inside `fiber`'s innermost one, closed when the frame it pushes unwinds. */
function openScope(fiber, scope = new Scope()) {
  fiber.stack.push({ $: SCOPE, scope, previous: fiber.scope });
  fiber.scope = scope;
  return scope;
}

class Fiber {
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
  // Messages sent and not yet received, once there is one, and the
  // `receive` the fiber is suspended on, if it is.
  mailbox = null;
  receiving = null;
  // Whether the task a `wait` was resumed with runs before a pending
  // interruption.
  handedOff = false;

  constructor(task, parent) {
    this.parent = parent;
    // The fiber's own scope encloses everything it runs.
    openScope(this);
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
 * - `synchronous` makes work that arrives outside a drain run at once, in
 *   the caller's stack, as elm/core's scheduler does, so a port answers in
 *   the same tick; past the budget, the rest still goes through `schedule`.
 */
export class Runtime {
  /** Where top-level fibers and daemons live; it is never closed. */
  root = new Scope();
  /** Fibers started and finished so far; the difference is the live ones. */
  stats = { started: 0, finished: 0 };

  #queue = [];
  #head = 0;
  // Whether a drain is scheduled, and whether one is running.
  #scheduled = false;
  #draining = false;
  #synchronous;
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
    synchronous = false,
  } = {}) {
    this.#schedule = schedule;
    this.#synchronous = synchronous;
    this.#clock = clock;
    this.#yieldEvery = yieldEvery;
    this.#budget = budget;
    this.#reportDefect = reportDefect;
  }

  /**
   * Starts `task` on a new fiber in the root scope. `observe`, if given,
   * receives its exit and counts as waiting for it from before it starts,
   * which matters when a synchronous runtime runs it to its end at once.
   */
  fork(task, observe) {
    return this.#spawn(task, this.root, observe);
  }

  /** Asks `fiber` to stop and resolves with its exit. */
  interrupt(fiber) {
    this.#interrupt(fiber);
    return this.await(fiber);
  }

  /** Puts `message` in `fiber`'s mailbox; a fiber that is done drops it. */
  send(fiber, message) {
    if (fiber.exit !== null) return;
    (fiber.mailbox ??= []).push(message);
    const receiving = fiber.receiving;
    if (receiving !== null) {
      fiber.receiving = null;
      this.#resume(fiber, fiber.epoch, receiving);
    }
  }

  /** Resolves with `fiber`'s exit; the caller counts as waiting for it. */
  await(fiber) {
    if (fiber.exit !== null) return Promise.resolve(fiber.exit);
    return new Promise((resolve) => fiber.observers.add(resolve));
  }

  /** Runs `task` on a new fiber and resolves with its exit. */
  run(task) {
    return new Promise((resolve) => this.fork(task, resolve));
  }

  /**
   * Interrupts every fiber in the root scope and resolves once the scope is
   * empty: fibers forked into it meanwhile, as a finalizer's daemon, are
   * interrupted in turn, each once.
   */
  shutdown() {
    return new Promise((resolve) => {
      const sweep = () => {
        const fibers = [...this.root.fibers];
        if (fibers.length === 0) return resolve();
        let left = fibers.length;
        const reaper = () => --left === 0 && sweep();
        for (const fiber of fibers) {
          fiber.reapers.add(reaper);
          this.#interrupt(fiber);
        }
      };
      sweep();
    });
  }

  #spawn(task, scope, observe) {
    const fiber = new Fiber(asTask(task), scope);
    if (observe !== undefined) fiber.observers.add(observe);
    scope.fibers.add(fiber);
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
    if (this.#draining) return;
    if (this.#synchronous) this.#drain();
    else this.#later();
  }

  /** Schedules a drain, unless one already is. */
  #later() {
    if (this.#scheduled) return;
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
    if (this.#head === this.#queue.length) {
      // Emptied once read to its end, so it keeps no finished fiber's exit alive.
      this.#queue.length = 0;
      this.#head = 0;
    } else if (this.#head > 1024 && this.#head * 2 > this.#queue.length) {
      // Drop the consumed prefix once it dominates, keeping pushes O(1).
      this.#queue = this.#queue.slice(this.#head);
      this.#head = 0;
    }
    if (this.#head < this.#queue.length) {
      this.#later();
    } else if (this.#io === 0) {
      // What the clock wakes runs in a drain of its own, so a test clock
      // firing timer after timer doesn't nest drains.
      this.#clock.idle();
      if (this.#head < this.#queue.length) this.#later();
    }
    this.#draining = false;
  }

  /** Runs `fiber` until it suspends, finishes or uses its slice; returns the instructions run. */
  #step(fiber) {
    const stack = fiber.stack;
    let task = !fiber.handedOff && this.#takeInterrupt(fiber) ? done(Exit.interrupt()) : fiber.task;
    fiber.handedOff = false;
    fiber.status = RUNNING;
    for (let n = 1; ; n++) {
      if (n > this.#yieldEvery) {
        fiber.task = task;
        this.#enqueue(fiber);
        return n;
      }
      switch (task.$) {
        case EXIT: {
          // Unwind until a frame continues with a task; with none left, the
          // fiber is done.
          let exit = task.exit;
          task = null;
          while (task === null && stack.length > 0) {
            const frame = stack.pop();
            switch (frame.$) {
              case AND_THEN:
                if (exit.$ === SUCCESS) task = call(frame.f, exit.value);
                break;
              case ON_ERROR:
                if (exit.$ === FAIL) task = call(frame.f, exit.error);
                break;
              case ON_EXIT:
                task = call(frame.f, exit);
                break;
              case RESTORE:
                fiber.mask--;
                exit = this.#unmask(fiber, exit);
                break;
              case SCOPE:
                task = this.#close(fiber, frame, exit);
                break;
              case CLOSED:
                // The closing task succeeds with the exit to carry on with.
                fiber.mask--;
                exit = this.#unmask(fiber, exit.$ === SUCCESS ? exit.value : exit);
                break;
            }
          }
          if (task !== null) break;
          this.#finish(fiber, exit);
          return n;
        }
        case SYNC:
          task = callValue(task.f);
          break;
        case AND_THEN:
        case ON_ERROR:
          stack.push(task);
          task = inner(task);
          break;
        case FORK:
          task = succeed(this.#fork(fiber, task));
          break;
        // The next three return the task to continue with, or null once the
        // fiber has suspended.
        case AWAIT:
        case INTERRUPT:
          task = this.#await(fiber, task);
          if (task === null) return n;
          break;
        case REAP:
          task = this.#reap(fiber, task);
          if (task === null) return n;
          break;
        case GATHER:
          task = this.#gather(fiber, task);
          if (task === null) return n;
          break;
        case ASYNC:
        case WAIT:
          task = this.#async(fiber, task);
          if (task === null) return n;
          break;
        case SLEEP: {
          if (this.#takeInterrupt(fiber)) {
            task = done(Exit.interrupt());
            break;
          }
          const epoch = this.#suspend(fiber);
          fiber.cancel = this.#clock.setTimer(task.ms, () => this.#resume(fiber, epoch, succeed(undefined)));
          return n;
        }
        case NOW:
          task = callValue(() => this.#clock.now());
          break;
        case RECEIVE:
          task = this.#receive(fiber, task);
          if (task === null) return n;
          break;
        case YIELD:
          fiber.task = succeed(undefined);
          this.#enqueue(fiber);
          return n;
        case MASK:
          fiber.mask++;
          stack.push({ $: RESTORE });
          task = inner(task);
          break;
        case SCOPED:
          openScope(fiber);
          task = inner(task);
          break;
        case WITH_FIBER:
          task = call(task.f, fiber);
          break;
        default:
          task = die(notATask(task));
      }
    }
  }

  /** Starts a fiber for a `fork`: in the scope it names, or else the root for a daemon, or else `fiber`'s. */
  #fork(fiber, { task, daemon, scope }) {
    if (scope === null) return this.#spawn(task, daemon ? this.root : (fiber.scope ?? this.root));
    const child = this.#spawn(task, scope);
    if (scope.closing) this.#interrupt(child);
    return child;
  }

  /** Waits for the fiber `task` names, asking it to stop first when `task` is an `interrupt`. */
  #await(fiber, task) {
    const target = task.fiber;
    if (!(target instanceof Fiber)) {
      const name = task.$ === INTERRUPT ? "interrupt" : "awaitFiber";
      return die(new TypeError(`${name} expects a fiber, got ${describe(target)}`));
    }
    if (task.$ === INTERRUPT) {
      if (target === fiber) {
        // Interrupting yourself takes effect as soon as it may.
        fiber.interruptPending = true;
        if (this.#takeInterrupt(fiber)) return done(Exit.interrupt());
        return succeed(task.wait ? Exit.interrupt() : undefined);
      }
      this.#interrupt(target);
      if (!task.wait) return succeed(undefined);
    }
    if (target.exit !== null) return succeed(target.exit);
    if (this.#takeInterrupt(fiber)) return done(Exit.interrupt());
    const epoch = this.#suspend(fiber);
    const observe = (exit) => this.#resume(fiber, epoch, succeed(exit));
    target.observers.add(observe);
    fiber.cancel = () => target.observers.delete(observe);
    return null;
  }

  /** Takes the oldest message for `task.f`, or suspends `fiber` until `send` brings one. */
  #receive(fiber, task) {
    const mailbox = fiber.mailbox;
    if (mailbox !== null && mailbox.length > 0) return call(task.f, mailbox.shift());
    if (this.#takeInterrupt(fiber)) return done(Exit.interrupt());
    this.#suspend(fiber);
    fiber.receiving = task;
    fiber.cancel = () => (fiber.receiving = null);
    return null;
  }

  /** Waits until every fiber in `fibers` is done, or `ms` have passed if given, without counting as waiting for them. */
  #reap(fiber, { fibers, ms }) {
    const live = fibers.filter((f) => f.exit === null);
    if (live.length === 0) return succeed(undefined);
    const epoch = this.#suspend(fiber);
    let left = live.length;
    let cancelTimer = null;
    const stop = () => {
      for (const f of live) f.reapers.delete(reaper);
      if (cancelTimer !== null) cancelTimer();
    };
    // Whichever comes first takes the other back, so no timer outlives the wait.
    const wake = () => (stop(), this.#resume(fiber, epoch, succeed(undefined)));
    const reaper = () => --left === 0 && wake();
    for (const f of live) f.reapers.add(reaper);
    if (ms !== undefined) cancelTimer = this.#clock.setTimer(ms, wake);
    fiber.cancel = stop;
    return null;
  }

  /** Forks `task.tasks` in `fiber`'s scope and suspends it until `task.step` returns a task for an exit. */
  #gather(fiber, { tasks, step }) {
    if (this.#takeInterrupt(fiber)) return done(Exit.interrupt());
    const scope = fiber.scope ?? this.root;
    const fibers = tasks.map((t) => this.#spawn(t, scope));
    const epoch = this.#suspend(fiber);
    const observers = fibers.map((f, i) => {
      const observe = (exit) => {
        let next;
        try {
          next = step(exit, i);
        } catch (error) {
          next = die(error);
        }
        if (next === undefined) return;
        stop();
        this.#resume(fiber, epoch, asTask(next));
      };
      f.observers.add(observe);
      return observe;
    });
    const stop = () => fibers.forEach((f, i) => f.observers.delete(observers[i]));
    fiber.cancel = stop;
    return null;
  }

  /** Suspends `fiber` on the host for an `async`, or on another fiber for a `wait`, handing `register` its `resume`. */
  #async(fiber, { $, register }) {
    if (this.#takeInterrupt(fiber)) return done(Exit.interrupt());
    const epoch = this.#suspend(fiber);
    const io = $ === ASYNC;
    if (io) {
      fiber.io = true;
      this.#io++;
    }
    try {
      const cancel = register((next) => {
        if (fiber.epoch !== epoch) return;
        fiber.handedOff = !io;
        this.#resume(fiber, epoch, asTask(next));
      });
      if (fiber.epoch === epoch && typeof cancel === "function") fiber.cancel = cancel;
    } catch (error) {
      if (fiber.epoch === epoch) {
        // Nothing resumed it yet: the exception is how it ends.
        this.#endSuspension(fiber);
        fiber.status = RUNNING;
        return die(error);
      }
      // It already resumed, so the exception has no fiber to land in.
      this.#report(error, fiber);
    }
    return null;
  }

  /** Suspends `fiber`; returns the epoch its `#resume` must name. */
  #suspend(fiber) {
    fiber.status = SUSPENDED;
    fiber.task = null;
    return fiber.epoch;
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
    if (fiber.epoch !== epoch) return;
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
    return exit.$ === DEFECT ? exit : Exit.interrupt();
  }

  /**
   * Starts closing `frame.scope` with `exit`, uninterruptibly: interrupt the
   * scope's fibers, wait for them, then run its finalizers last to first; a
   * draining scope first waits for its fibers until its deadline. Returns
   * the task that does it; it succeeds with the exit to continue with, which
   * is `exit` unless a finalizer failed or died.
   */
  #close(fiber, frame, exit) {
    const { scope } = frame;
    fiber.scope = frame.previous;
    fiber.mask++;
    fiber.stack.push({ $: CLOSED });
    const finalizers = scope.finalizers.splice(0).reverse();
    let result = exit;
    const settle = (finalized) => {
      if (finalized.$ !== FAIL && finalized.$ !== DEFECT) return;
      const defect = finalized.$ === FAIL ? finalized.error : finalized.defect;
      // The first defect is the exit; later ones would be lost, so report them.
      if (result.$ === DEFECT) this.#report(defect, fiber);
      else result = Exit.defect(defect);
    };
    const run = (i) =>
      i === finalizers.length
        ? succeed(result)
        : andThen(attempt(andThen(succeed(exit), finalizers[i])), (finalized) => (settle(finalized), run(i + 1)));
    const end = () => {
      const children = [...scope.fibers];
      for (const child of children) this.#interrupt(child);
      return andThen(reap(children), () => run(0));
    };
    if (!(scope instanceof DrainingScope)) return end();
    scope.closing = true;
    return andThen(reap([...scope.fibers], scope.ms), end);
  }

  #finish(fiber, exit) {
    fiber.exit = exit;
    fiber.status = DONE;
    fiber.task = null;
    fiber.stack.length = 0;
    fiber.scope = null;
    fiber.mailbox = null;
    fiber.parent.fibers.delete(fiber);
    this.stats.finished++;
    const observers = [...fiber.observers];
    const reapers = [...fiber.reapers];
    fiber.observers.clear();
    fiber.reapers.clear();
    if (exit.$ === DEFECT && observers.length === 0) this.#report(exit.defect, fiber);
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
}

/**
 * The task an `andThen`, `onError`, mask or scope runs; a missing one is a
 * defect. Any other non-task needs no check here: the interpreter's
 * `default` makes it one.
 */
function inner(task) {
  return task.task ?? die(notATask(task.task));
}

/** The task `f(value)` returns; an exception, or a value that is not a task, is a defect. */
function call(f, value) {
  try {
    return asTask(f(value));
  } catch (error) {
    return die(error);
  }
}

/** Succeeds with `f()`; an exception is a defect. */
function callValue(f) {
  try {
    return succeed(f());
  } catch (error) {
    return die(error);
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
