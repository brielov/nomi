// Prints whether a finished fiber's result is collected while its runtime
// lives on, at rest. Run with --expose-gc.

import { setImmediate } from "node:timers/promises";

import { Runtime, sync } from "../src/fiber.js";

const runtime = new Runtime();
let result = {};
const ref = new WeakRef(result);
await runtime.run(sync(() => result));
result = null;
for (let i = 0; i < 5; i++) {
  await setImmediate();
  gc();
}
console.log(ref.deref() === undefined ? "collected" : "kept");
