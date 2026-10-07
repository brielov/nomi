//! Deterministic source mutations, ported from the first attempt's parser
//! mutation suite (`brielov/nomi@8e2b540:crates/syntax/tests/parser_mutation.rs`).
//! The same seed always yields the same mutations, so the oracle's verdicts
//! can be stored and the sources regenerated wherever they are needed.

/// What a mutation may insert: separators, brackets, layout and comments.
pub const INSERTIONS: &[&str] = &[
    " ", "\n", "\r", "\t", ",", ".", "=", "->", "(", ")", "[", "]", "{", "}", "--x\n", "{-x-}",
];

/// Boundaries [`suite`] samples in each seed.
pub const SUITE_BOUNDARIES: usize = 256;

/// Random mutations [`suite`] makes of each seed.
pub const SUITE_RANDOM: usize = 512;

/// One mutated source, named after what was done to the seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mutation {
    pub name: String,
    pub source: String,
}

/// The mutation suite's cases for one seed, as the first attempt's
/// accept/reject suite made them for its module seeds.
pub fn suite(seed_name: &str, source: &str) -> Vec<Mutation> {
    let mut mutations = systematic(source, SUITE_BOUNDARIES);
    mutations.extend(random(seed_name, source, SUITE_RANDOM));
    mutations
}

/// The SHA-256 of every mutation's name and source, so stored verdicts
/// can tell when the generator or a seed changed under them.
pub fn digest(mutations: &[Mutation]) -> String {
    let mut bytes = Vec::new();
    for m in mutations {
        bytes.extend(m.name.as_bytes());
        bytes.push(0);
        bytes.extend(m.source.as_bytes());
        bytes.push(0);
    }
    crate::ratchet::sha256_hex(&bytes)
}

/// The seed itself, then every insertion at up to `max_boundaries` evenly
/// sampled character boundaries, then the deletion and duplication of the
/// character at each of them. Insertions rotate with the boundary, so
/// neighbouring boundaries do not try them in the same order.
pub fn systematic(source: &str, max_boundaries: usize) -> Vec<Mutation> {
    let boundaries = sampled_boundaries(source, max_boundaries);
    let mut mutations = Vec::with_capacity(boundaries.len() * (INSERTIONS.len() + 2) + 1);
    mutations.push(Mutation {
        name: "original".to_owned(),
        source: source.to_owned(),
    });
    for (i, &boundary) in boundaries.iter().enumerate() {
        for offset in 0..INSERTIONS.len() {
            let insertion = INSERTIONS[(i + offset) % INSERTIONS.len()];
            let mut mutated = source.to_owned();
            mutated.insert_str(boundary, insertion);
            mutations.push(Mutation {
                name: format!("insert {insertion:?} at {boundary}"),
                source: mutated,
            });
        }
    }
    for &start in &boundaries {
        let Some(c) = source[start..].chars().next() else {
            continue;
        };
        let mut deleted = source.to_owned();
        deleted.replace_range(start..start + c.len_utf8(), "");
        mutations.push(Mutation {
            name: format!("delete {c:?} at {start}"),
            source: deleted,
        });
        let mut duplicated = source.to_owned();
        duplicated.insert(start, c);
        mutations.push(Mutation {
            name: format!("duplicate {c:?} at {start}"),
            source: duplicated,
        });
    }
    mutations
}

/// `count` mutations of two to four random insertions, deletions or
/// duplications each, from a generator seeded by `seed_name`.
pub fn random(seed_name: &str, source: &str, count: usize) -> Vec<Mutation> {
    let mut random = Random::new(fnv1a(seed_name.as_bytes()));
    (0..count)
        .map(|n| {
            let mut mutated = source.to_owned();
            for _ in 0..2 + random.index(3) {
                let boundaries = boundaries(&mutated);
                let at = boundaries[random.index(boundaries.len())];
                match random.index(INSERTIONS.len() + 2) {
                    op if op < INSERTIONS.len() => mutated.insert_str(at, INSERTIONS[op]),
                    op if op == INSERTIONS.len() => {
                        if let Some(c) = mutated[at..].chars().next() {
                            mutated.replace_range(at..at + c.len_utf8(), "");
                        }
                    }
                    _ => {
                        if let Some(c) = mutated[at..].chars().next() {
                            mutated.insert(at, c);
                        }
                    }
                }
            }
            Mutation {
                name: format!("random {n}"),
                source: mutated,
            }
        })
        .collect()
}

/// Every character boundary, the end included.
fn boundaries(source: &str) -> Vec<usize> {
    source
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(source.len()))
        .collect()
}

/// At most `maximum` boundaries, evenly spread and keeping both ends.
fn sampled_boundaries(source: &str, maximum: usize) -> Vec<usize> {
    let all = boundaries(source);
    if all.len() <= maximum {
        return all;
    }
    (0..maximum)
        .map(|sample| all[sample * (all.len() - 1) / (maximum - 1)])
        .collect()
}

/// xorshift64.
struct Random(u64);

impl Random {
    fn new(seed: u64) -> Random {
        Random(seed.max(1))
    }

    fn index(&mut self, upper: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % upper as u64) as usize
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systematic_covers_every_boundary_of_a_short_seed() {
        let mutations = systematic("ab", 256);
        // The original, 16 insertions at 3 boundaries, and a deletion and
        // a duplication at the 2 that start a character.
        assert_eq!(mutations.len(), 1 + 3 * INSERTIONS.len() + 2 * 2);
        assert_eq!(mutations[0].source, "ab");
        assert_eq!(mutations[1].name, "insert \" \" at 0");
        assert_eq!(mutations[1 + INSERTIONS.len()].name, "insert \"\\n\" at 1");
        assert_eq!(mutations[1 + INSERTIONS.len()].source, "a\nb");
        let last = &mutations[mutations.len() - 1];
        assert_eq!(
            (last.name.as_str(), last.source.as_str()),
            ("duplicate 'b' at 1", "abb")
        );
    }

    #[test]
    fn sampling_keeps_both_ends() {
        let source = "x".repeat(1000);
        let sampled = sampled_boundaries(&source, 256);
        assert_eq!(sampled.len(), 256);
        assert_eq!((sampled[0], sampled[255]), (0, 1000));
    }

    #[test]
    fn boundaries_respect_multibyte_characters() {
        let mutations = systematic("é", 256);
        assert!(mutations.iter().any(|m| m.source == "éé"));
        assert!(mutations.iter().any(|m| m.source.is_empty()));
    }

    #[test]
    fn random_mutations_are_deterministic() {
        let a = random("seed", "module A exposing (..)\n", 8);
        assert_eq!(a, random("seed", "module A exposing (..)\n", 8));
        assert_ne!(a, random("other", "module A exposing (..)\n", 8));
        // Pinned, so a change to the generator cannot pass unnoticed: the
        // stored oracle verdicts are keyed by these names and sources.
        assert_eq!(fnv1a(b"seed"), 0x2cfa_d118_d3ec_c02c);
    }
}
