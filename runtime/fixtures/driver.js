// Runtimes the tests drive by hand.

import assert from "node:assert/strict";

import { Runtime } from "../src/fiber.js";

/**
 * A runtime whose slices wait in `slices` until the test runs them, and
 * whose defects are collected in `reports`. `flush` runs slices until none
 * is left.
 */
export function manual(options = {}) {
  const slices = [];
  const reports = [];
  const runtime = new Runtime({
    schedule: (fn) => slices.push(fn),
    reportDefect: (defect) => reports.push(defect),
    ...options,
  });
  const flush = () => {
    while (slices.length > 0) slices.shift()();
  };
  return { runtime, slices, reports, flush };
}

/**
 * A manual runtime with slices of a random size, for programs that leave
 * their async operations' completions in `pending`. `settle` runs slices and
 * completions in a random order until neither is left.
 */
export function shuffled(r, options = {}) {
  const pending = [];
  const { runtime, slices, reports } = manual({ yieldEvery: r.int(1, 6), budget: r.int(1, 40), ...options });
  const settle = () => {
    for (let guard = 0; slices.length > 0 || pending.length > 0; guard++) {
      assert.ok(guard < 1e6, "the program did not finish");
      if (slices.length > 0 && (pending.length === 0 || r.bool(0.7))) slices.shift()();
      else pending.splice(r.int(0, pending.length - 1), 1)[0]();
    }
  };
  return { runtime, reports, pending, settle };
}
