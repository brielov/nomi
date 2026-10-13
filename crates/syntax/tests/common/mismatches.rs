//! The parser ports' tests, which hold them to every case of the
//! mutation suite rather than ratchet them.

/// Passes when `mismatches` is empty; otherwise shows how many there are,
/// and the first 40.
#[track_caller]
pub(crate) fn assert_none(mismatches: &[String]) {
    assert!(
        mismatches.is_empty(),
        "{} mismatches, the first:\n{}",
        mismatches.len(),
        mismatches[..mismatches.len().min(40)].join("\n")
    );
}
