import { readFile } from "node:fs/promises";

export async function read(path) {
  return readFile(path, "utf8");
}

export const twice = async (n) => n * 2, other = 1;
export class Clock { now() { return Date.now(); } }
export { read as load, twice as "double" };
export default async function () {}
export * as fs from "node:fs";
export * from "./more.mjs";
const s = `export function hidden() {} ${"}"}`;
/* export function commented() {} */
// export function alsoCommented() {}
const re = /export function inRegex\(\) {}/g;
