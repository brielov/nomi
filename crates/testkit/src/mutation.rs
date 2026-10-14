//! Deterministic source mutations: insertions, deletions and duplications
//! at sampled character boundaries, and seeded random combinations of
//! them. The same seed always yields the same mutations, so the oracle's
//! verdicts can be stored and the sources regenerated wherever they are
//! needed.

use std::fs;
use std::path::Path;
use std::str::FromStr;

use sha2::{Digest as _, Sha256};

/// What a mutation may insert: separators, brackets, layout and comments.
const INSERTIONS: &[&str] = &[
    " ", "\n", "\r", "\t", ",", ".", "=", "->", "(", ")", "[", "]", "{", "}", "--x\n", "{-x-}",
];

/// Boundaries [`suite`] samples in each seed.
const SUITE_BOUNDARIES: usize = 256;

/// Random mutations [`suite`] makes of each seed.
const SUITE_RANDOM: usize = 512;

/// One mutated source, named after what was done to the seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mutation {
    /// What was done, such as `insert "(" at 12` or `random 3`.
    pub name: String,
    /// The seed with the mutation made.
    pub source: String,
}

/// A seed of a mutation suite, as its `seeds.txt` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed<K> {
    /// What names the seed's mutations and their stored verdicts.
    pub name: String,
    /// The kind of project the seed's module belongs to.
    pub kind: K,
    pub text: String,
}

/// The seeds `seeds.txt` in `dir` lists, a `<name> <kind> <file>` line
/// each, with each file's text from `dir/seeds/`. Blank lines and `#`
/// comments are skipped.
pub fn seeds<K: FromStr>(dir: &Path) -> Result<Vec<Seed<K>>, String> {
    let list =
        fs::read_to_string(dir.join("seeds.txt")).map_err(|e| format!("reading seeds.txt: {e}"))?;
    (list.lines())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            let [name, kind, file] = words[..] else {
                return Err(format!("seeds.txt: malformed line `{line}`"));
            };
            let kind = (kind.parse()).map_err(|_| format!("seeds.txt: unknown kind `{kind}`"))?;
            let path = dir.join("seeds").join(file);
            let text = fs::read_to_string(&path)
                .map_err(|e| format!("reading {}: {e}", path.display()))?;
            Ok(Seed {
                name: name.to_owned(),
                kind,
                text,
            })
        })
        .collect()
}

/// The mutation suite's cases for one seed: the systematic mutations at up
/// to 256 boundaries, then 512 random ones.
pub fn suite(seed_name: &str, source: &str) -> Vec<Mutation> {
    let mut mutations = systematic(source, SUITE_BOUNDARIES);
    mutations.extend(random(seed_name, source, SUITE_RANDOM));
    mutations
}

/// The SHA-256 of every mutation's name and source, so stored verdicts
/// can tell when the generator or a seed changed under them.
pub fn digest(mutations: &[Mutation]) -> String {
    let mut hasher = Sha256::new();
    for m in mutations {
        for part in [&m.name, &m.source] {
            hasher.update(part);
            hasher.update([0]);
        }
    }
    nomi_base::hex(&hasher.finalize())
}

/// One change to a source at a character boundary.
#[derive(Clone, Copy, Debug)]
enum Op {
    Insert(&'static str),
    /// Of the character at the boundary.
    Delete,
    /// Of the character at the boundary.
    Duplicate,
}

impl Op {
    /// The `index`th of the operations [`random`] picks from: each
    /// insertion, then deletion, then duplication.
    fn nth(index: usize) -> Op {
        match INSERTIONS.get(index) {
            Some(insertion) => Op::Insert(insertion),
            None if index == INSERTIONS.len() => Op::Delete,
            None => Op::Duplicate,
        }
    }

    /// The operation's name in a mutation's.
    fn verb(self) -> &'static str {
        match self {
            Op::Insert(_) => "insert",
            Op::Delete => "delete",
            Op::Duplicate => "duplicate",
        }
    }

    /// Makes the change in `source` at `at`, a character boundary. A
    /// deletion or duplication at the end changes nothing.
    fn apply(self, source: &mut String, at: usize) {
        let next = source[at..].chars().next();
        match (self, next) {
            (Op::Insert(text), _) => source.insert_str(at, text),
            (Op::Delete, Some(c)) => source.replace_range(at..at + c.len_utf8(), ""),
            (Op::Duplicate, Some(c)) => source.insert(at, c),
            (Op::Delete | Op::Duplicate, None) => {}
        }
    }
}

/// The seed itself, then every insertion at up to `max_boundaries` evenly
/// sampled character boundaries, then the deletion and duplication of the
/// character at each of them. Insertions rotate with the boundary, so
/// neighbouring boundaries do not try them in the same order.
fn systematic(source: &str, max_boundaries: usize) -> Vec<Mutation> {
    let boundaries = sampled_boundaries(source, max_boundaries);
    let mut mutations = Vec::with_capacity(boundaries.len() * (INSERTIONS.len() + 2) + 1);
    mutations.push(Mutation {
        name: "original".to_owned(),
        source: source.to_owned(),
    });
    let mut push = |op: Op, at: usize, name: String| {
        let mut mutated = source.to_owned();
        op.apply(&mut mutated, at);
        mutations.push(Mutation {
            name: format!("{} {name} at {at}", op.verb()),
            source: mutated,
        });
    };
    for (i, &boundary) in boundaries.iter().enumerate() {
        for offset in 0..INSERTIONS.len() {
            let insertion = INSERTIONS[(i + offset) % INSERTIONS.len()];
            push(Op::Insert(insertion), boundary, format!("{insertion:?}"));
        }
    }
    for &start in &boundaries {
        let Some(c) = source[start..].chars().next() else {
            continue;
        };
        for op in [Op::Delete, Op::Duplicate] {
            push(op, start, format!("{c:?}"));
        }
    }
    mutations
}

/// `count` mutations of two to four random insertions, deletions or
/// duplications each, from a generator seeded by `seed_name`.
fn random(seed_name: &str, source: &str, count: usize) -> Vec<Mutation> {
    let mut random = Random::new(fnv1a(seed_name.as_bytes()));
    (0..count)
        .map(|n| {
            let mut mutated = source.to_owned();
            for _ in 0..2 + random.index(3) {
                let boundaries = boundaries(&mutated);
                let at = boundaries[random.index(boundaries.len())];
                Op::nth(random.index(INSERTIONS.len() + 2)).apply(&mut mutated, at);
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

    #[derive(Debug, PartialEq)]
    struct App;

    impl FromStr for App {
        type Err = ();

        fn from_str(text: &str) -> Result<App, ()> {
            (text == "application").then_some(App).ok_or(())
        }
    }

    #[test]
    fn seeds_are_read_as_listed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        fs::create_dir_all(dir.join("seeds")).unwrap();
        fs::write(dir.join("seeds/A.elm"), "module A exposing (..)\n").unwrap();
        fs::write(
            dir.join("seeds.txt"),
            "# name kind file\n\na application A.elm\n",
        )
        .unwrap();
        assert_eq!(
            seeds::<App>(dir).unwrap(),
            [Seed {
                name: "a".to_owned(),
                kind: App,
                text: "module A exposing (..)\n".to_owned(),
            }]
        );
        fs::write(dir.join("seeds.txt"), "a package A.elm\n").unwrap();
        assert_eq!(
            seeds::<App>(dir),
            Err("seeds.txt: unknown kind `package`".to_owned())
        );
        fs::write(dir.join("seeds.txt"), "a application\n").unwrap();
        assert!(seeds::<App>(dir).is_err());
    }
}
