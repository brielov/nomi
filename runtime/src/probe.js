// What a server entry checks before it runs: that its host is Bun, at
// MINIMUM or later, whose built-ins the kernels are written on. Any other
// host, Node included, is refused with one line naming what it is.

/** The oldest Bun nomi's programs run on, as references.toml's `bun-minimum`. */
export const MINIMUM = "1.4";

/**
 * Exits 1 with one line unless `scope` is Bun at `MINIMUM` or later.
 * `scope` defaults to the global object and `proc` to `process`.
 */
export function probe({ scope = globalThis, proc = process } = {}) {
  const version = scope.Bun?.version;
  if (typeof version === "string" && !older(version, MINIMUM)) return;
  const host = typeof version === "string" ? `Bun ${version}` : hostOf(proc);
  console.error(`nomi: this program runs on Bun ${MINIMUM} or later (this is ${host}).`);
  proc.exit(1);
}

/** Whether the version `version` is before `minimum`, part by numeric part. */
function older(version, minimum) {
  const have = version.split(".").map((part) => Number.parseInt(part, 10));
  const need = minimum.split(".").map((part) => Number.parseInt(part, 10));
  for (let i = 0; i < need.length; i++) {
    const part = have[i] || 0;
    if (part !== need[i]) return part < need[i];
  }
  return false;
}

/** The host that isn't Bun, as it names itself. */
function hostOf(proc) {
  if (proc?.release?.name === "node") return `Node ${proc.version}`;
  return "not Bun";
}
