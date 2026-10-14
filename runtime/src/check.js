// Seeded, model-based property checks.
//
// A property draws a case from a seeded generator and runs it, usually by
// applying a random sequence of operations to the real implementation and
// to a small reference model and comparing them after each step. Seeds are
// 0..count-1 by default, so a run is reproducible; a failing case is shrunk
// and reported with the command that replays it.
//
//   NOMI_SEED=n        run only seed n
//   NOMI_SEEDS=count   seeds per property (default 100)
//   NOMI_SEED_BASE=b   start at seed b, to explore beyond the usual seeds

/** A small, fast PRNG (mulberry32). Same seed, same sequence, everywhere. */
export function rng(seed) {
  let state = seed >>> 0;
  const next = () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return (t ^ (t >>> 14)) >>> 0;
  };
  const r = {
    next,
    /** A float in [0, 1). */
    float: () => next() / 0x100000000,
    /** An integer in [lo, hi], inclusive. */
    int: (lo, hi) => lo + Math.floor(r.float() * (hi - lo + 1)),
    bool: (p) => r.float() < p,
    pick: (items) => items[r.int(0, items.length - 1)],
    /** Picks a value by weight from `[[weight, value], ...]`. */
    weighted: (choices) => {
      const total = choices.reduce((sum, [w]) => sum + w, 0);
      let x = r.float() * total;
      for (const [w, value] of choices) {
        x -= w;
        if (x < 0) return value;
      }
      return choices[choices.length - 1][1];
    },
    /** `length` values from `gen(r)`, with `length` drawn from [min, max]. */
    array: (gen, min, max) => Array.from({ length: r.int(min, max) }, () => gen(r)),
  };
  return r;
}

/** The seeds a property runs, from the environment. */
export function seeds(env) {
  if (env.NOMI_SEED !== undefined) return [parseSeed(env.NOMI_SEED, "NOMI_SEED")];
  const count = env.NOMI_SEEDS === undefined ? 100 : parseSeed(env.NOMI_SEEDS, "NOMI_SEEDS");
  const base = env.NOMI_SEED_BASE === undefined ? 0 : parseSeed(env.NOMI_SEED_BASE, "NOMI_SEED_BASE");
  return Array.from({ length: count }, (_, i) => base + i);
}

function parseSeed(text, name) {
  const n = Number(text);
  if (!Number.isSafeInteger(n) || n < 0) throw new Error(`${name}=${text} is not a non-negative integer`);
  return n;
}

/**
 * Checks `property(case)` for the case `generate(rng(seed))` draws at each
 * seed. `property` fails by throwing or rejecting. When it fails on an
 * array case, the case is shrunk by dropping elements while it still fails.
 * The thrown error names the seed, the shrunk case and how to replay it.
 */
export async function check(generate, property, env = process.env) {
  for (const seed of seeds(env)) {
    const original = generate(rng(seed));
    const failure = await failureOf(property, original);
    if (failure === undefined) continue;
    const { value: smallest, failure: cause } = Array.isArray(original)
      ? await shrink(property, original, failure)
      : { value: original, failure };
    throw new PropertyFailure(seed, original, smallest, cause);
  }
}

export class PropertyFailure extends Error {
  constructor(seed, original, smallest, cause) {
    const sizes = Array.isArray(original) ? ` (shrunk from ${original.length} to ${smallest.length} steps)` : "";
    super(
      `property failed at seed ${seed}${sizes}\n` +
        `  case: ${describe(smallest)}\n` +
        `  cause: ${cause?.message ?? cause}\n` +
        // Every property runs that seed alone, so the one that failed fails again.
        `  replay: NOMI_SEED=${seed} node --test`,
      { cause },
    );
    this.name = "PropertyFailure";
    this.seed = seed;
    this.case = smallest;
  }
}

async function failureOf(property, value) {
  try {
    await property(value);
    return undefined;
  } catch (error) {
    return error ?? new Error("property threw a nullish value");
  }
}

/** Drops chunks of halving size, then single elements, while the property still fails. */
async function shrink(property, value, failure) {
  let current = value;
  let cause = failure;
  for (let chunk = Math.floor(current.length / 2); chunk >= 1; chunk = Math.floor(chunk / 2)) {
    let start = 0;
    while (start < current.length) {
      const candidate = current.slice(0, start).concat(current.slice(start + chunk));
      const result = await failureOf(property, candidate);
      if (result === undefined) {
        start += chunk;
      } else {
        current = candidate;
        cause = result;
      }
    }
  }
  return { value: current, failure: cause };
}

function describe(value) {
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}
