// What a Node entry checks before it runs: that Node has each feature the
// program needs beyond the language, rather than which version it is, so
// a missing one is named before anything uses it.

/** Each feature a program may need, by its path from the global object, with the first Node release that has it. */
export const SINCE = {
  AbortController: "15.0",
  TextDecoder: "11.0",
  TextEncoder: "11.0",
  URLSearchParams: "10.0",
  fetch: "18.0",
  "process.getBuiltinModule": "22.3",
  setImmediate: "0.10",
};

/**
 * Exits 1 with one line naming the first of `features`, keys of `SINCE`,
 * that `scope` lacks. `scope` defaults to the global object and `proc` to
 * `process`.
 */
export function probe(features, { scope = globalThis, proc = process } = {}) {
  for (const feature of features) {
    const found = feature.split(".").reduce((object, key) => object?.[key], scope);
    if (typeof found !== "function") {
      console.error(
        `nomi: this program needs ${feature}, which Node ${SINCE[feature]} and later have (this is Node ${proc.version}).`,
      );
      proc.exit(1);
      return;
    }
  }
}
