//! Upstream's decoders (`Json/Decode.hs`) as functions over a parsed
//! [`Value`], failing with the same tree of problems. Decoders run their
//! parts in order and stop at the first failure, as upstream's applicative
//! does, so writing them as sequences of `?` keeps upstream's choice of
//! error.

use super::cursor::{Cursor, Region};
use super::parse::{Kind, Value};

/// Why a parsed value doesn't decode; `X` is the caller's own failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem<X> {
    /// In the field with these raw key bytes.
    Field(Vec<u8>, Box<Problem<X>>),
    /// In the array's entry at this index.
    Index(usize, Box<Problem<X>>),
    /// Every alternative failed: the first one's problem, then the rest's.
    OneOf(Box<Problem<X>>, Vec<Problem<X>>),
    /// The caller's decoder rejected the value in this region.
    Failure(Region, X),
    /// The value in this region isn't the kind expected.
    Expecting(Region, Expectation),
}

/// The kind of value a decoder wanted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expectation {
    Object,
    Array,
    String,
    /// An object with this field.
    ObjectWith(Vec<u8>),
}

/// A decoder's result.
pub type Decoded<T, X> = Result<T, Problem<X>>;

/// Of a [`Problem::OneOf`]'s problems, the one upstream reports: the
/// deepest, the first of those as deep.
pub fn deepest<'p, X>(first: &'p Problem<X>, rest: &'p [Problem<X>]) -> &'p Problem<X> {
    std::iter::once(first)
        .chain(rest)
        .rev()
        .max_by_key(|p| depth(p))
        .unwrap_or(first)
}

/// `getMaxDepth`: how many fields and indexes deep the problem is, through
/// its deepest alternative.
fn depth<X>(problem: &Problem<X>) -> usize {
    match problem {
        Problem::Field(_, p) | Problem::Index(_, p) => 1 + depth(p),
        Problem::OneOf(p, ps) => depth(deepest(p, ps)),
        Problem::Failure(..) | Problem::Expecting(..) => 0,
    }
}

fn expecting<T, X>(value: &Value<'_>, expectation: Expectation) -> Decoded<T, X> {
    Err(Problem::Expecting(value.region, expectation))
}

/// `string`: the bytes between the quotes, escapes as written.
pub fn string<'a, X>(value: &Value<'a>) -> Decoded<&'a [u8], X> {
    match value.kind {
        Kind::String(snippet) => Ok(snippet.bytes),
        _ => expecting(value, Expectation::String),
    }
}

/// `customString`: a string read by `parser`, which starts where the
/// string's bytes do and fails with the caller's problem.
pub fn custom_string<T, X>(
    value: &Value<'_>,
    parser: impl FnOnce(&[u8], Cursor) -> Result<T, X>,
) -> Decoded<T, X> {
    match value.kind {
        Kind::String(snippet) => {
            parser(snippet.bytes, snippet.start).map_err(|x| Problem::Failure(value.region, x))
        }
        _ => expecting(value, Expectation::String),
    }
}

/// `list`: each entry of an array, failing in the first that fails.
pub fn list<'a, T, X>(
    value: &Value<'a>,
    mut entry: impl FnMut(&Value<'a>) -> Decoded<T, X>,
) -> Decoded<Vec<T>, X> {
    match &value.kind {
        Kind::Array(entries) => entries
            .iter()
            .enumerate()
            .map(|(i, e)| entry(e).map_err(|p| Problem::Index(i, Box::new(p))))
            .collect(),
        _ => expecting(value, Expectation::Array),
    }
}

/// `nonEmptyList`: a list, failing with `empty` on the array if it has no
/// entries.
pub(crate) fn non_empty_list<'a, T, X>(
    value: &Value<'a>,
    entry: impl FnMut(&Value<'a>) -> Decoded<T, X>,
    empty: X,
) -> Decoded<Vec<T>, X> {
    let entries = list(value, entry)?;
    if entries.is_empty() {
        return Err(Problem::Failure(value.region, empty));
    }
    Ok(entries)
}

/// `pairs`: an object's fields in source order, each key read by `key`,
/// which fails on the key's own region, and each value by `entry`.
pub(crate) fn pairs<'a, K, V, X>(
    value: &Value<'a>,
    mut key: impl FnMut(&[u8], Cursor) -> Result<K, X>,
    mut entry: impl FnMut(&Value<'a>) -> Decoded<V, X>,
) -> Decoded<Vec<(K, V)>, X> {
    let Kind::Object(fields) = &value.kind else {
        return expecting(value, Expectation::Object);
    };
    fields
        .iter()
        .map(|(snippet, v)| {
            let k = key(snippet.bytes, snippet.start)
                .map_err(|x| Problem::Failure(snippet.region(), x))?;
            let v = entry(v).map_err(|p| Problem::Field(snippet.bytes.to_vec(), Box::new(p)))?;
            Ok((k, v))
        })
        .collect()
}

/// `dict`: [`pairs`] in key order, the last of equal keys winning, as
/// `Map.fromList` keeps it.
pub fn dict<'a, K: Ord, V, X>(
    value: &Value<'a>,
    key: impl FnMut(&[u8], Cursor) -> Result<K, X>,
    entry: impl FnMut(&Value<'a>) -> Decoded<V, X>,
) -> Decoded<std::collections::BTreeMap<K, V>, X> {
    Ok(pairs(value, key, entry)?.into_iter().collect())
}

/// `field`: the first field with exactly these key bytes.
pub fn field<'a, T, X>(
    value: &Value<'a>,
    key: &[u8],
    entry: impl FnOnce(&Value<'a>) -> Decoded<T, X>,
) -> Decoded<T, X> {
    let Kind::Object(fields) = &value.kind else {
        return expecting(value, Expectation::Object);
    };
    match fields.iter().find(|(k, _)| k.bytes == key) {
        Some((_, v)) => entry(v).map_err(|p| Problem::Field(key.to_vec(), Box::new(p))),
        None => expecting(value, Expectation::ObjectWith(key.to_vec())),
    }
}

/// `failure`: the caller's problem, on this value.
pub fn failure<T, X>(value: &Value<'_>, x: X) -> Decoded<T, X> {
    Err(Problem::Failure(value.region, x))
}
