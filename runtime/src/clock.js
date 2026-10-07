// Clocks for the runtime's timers (architecture §8, "Test clock").
//
// A clock has `now()`, `setTimer(ms, fire)`, which returns a function that
// cancels the timer, and `idle()`, which the runtime calls whenever no fiber
// can run and no I/O is pending.

/** Wall-clock time and host timers. */
export class RealClock {
  now() {
    return Date.now();
  }

  setTimer(ms, fire) {
    const handle = setTimeout(fire, ms);
    return () => clearTimeout(handle);
  }

  idle() {}
}

/**
 * Virtual time for tests. It moves only when the runtime is idle: every
 * live fiber is waiting on a timer or on another fiber, and no I/O is in
 * flight. Then it jumps to the earliest timer and fires every timer due at
 * that instant, in the order they were set.
 */
export class TestClock {
  #now;
  #seq = 0;
  // Sorted by due time, then by the order they were set.
  #timers = [];

  constructor(start = 0) {
    this.#now = start;
  }

  now() {
    return this.#now;
  }

  /** Timers set and not yet fired or cancelled. */
  get pending() {
    return this.#timers.length;
  }

  setTimer(ms, fire) {
    const timer = { due: this.#now + Math.max(0, ms), seq: this.#seq++, fire };
    this.#timers.splice(this.#insertionPoint(timer), 0, timer);
    return () => {
      const i = this.#timers.indexOf(timer);
      if (i !== -1) this.#timers.splice(i, 1);
    };
  }

  idle() {
    if (this.#timers.length === 0) return;
    this.#now = this.#timers[0].due;
    let due = 0;
    while (due < this.#timers.length && this.#timers[due].due === this.#now) due++;
    for (const timer of this.#timers.splice(0, due)) timer.fire();
  }

  #insertionPoint(timer) {
    let lo = 0;
    let hi = this.#timers.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      const other = this.#timers[mid];
      if (other.due < timer.due || (other.due === timer.due && other.seq < timer.seq)) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }
}
