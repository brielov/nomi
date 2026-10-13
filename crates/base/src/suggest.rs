//! Upstream's suggestions (`Reporting/Suggest.hs`): candidates ranked by
//! restricted Damerau–Levenshtein distance, the optimal string alignment
//! that `edit-distance` computes, between lowercased strings.

/// `distance`: the fewest insertions, deletions, substitutions and swaps
/// of adjacent elements that turn `a` into `b`, no element edited twice.
pub fn distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    // Three rows of the table: two back, one back, and this one.
    let mut two_back: Vec<usize> = vec![0; b.len() + 1];
    let mut one_back: Vec<usize> = (0..=b.len()).collect();
    let mut row = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        row[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (one_back[j] + 1)
                .min(row[j - 1] + 1)
                .min(one_back[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(two_back[j - 2] + 1);
            }
            row[j] = d;
        }
        std::mem::swap(&mut two_back, &mut one_back);
        std::mem::swap(&mut one_back, &mut row);
    }
    one_back[b.len()]
}

/// `sort`: `values` by the distance of their lowercased strings from the
/// lowercased `target`, stably. Strings are code points, since 0.19.1's
/// unchecked UTF-8 decoding can make values no `char` holds; those have
/// no case.
pub fn sort<V>(target: &[u32], values: &mut [V], to_codes: impl Fn(&V) -> Vec<u32>) {
    let lower = |codes: &[u32]| -> Vec<u32> {
        codes
            .iter()
            .map(|&code| char::from_u32(code).map_or(code, |c| crate::ghc::to_lower(c).into()))
            .collect()
    };
    let target = lower(target);
    values.sort_by_cached_key(|v| distance(&target, &lower(&to_codes(v))));
}

/// `text`'s code points, as [`sort`] compares strings.
pub fn code_points(text: &str) -> Vec<u32> {
    text.chars().map(u32::from).collect()
}

/// [`sort`] of texts against a text.
pub fn sort_texts<V: AsRef<str>>(target: &str, values: &mut [V]) {
    sort(&code_points(target), values, |v| code_points(v.as_ref()));
}

#[cfg(test)]
mod tests {
    use super::{distance, sort_texts};

    fn d(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        distance(&a, &b)
    }

    #[test]
    fn optimal_string_alignment() {
        assert_eq!(d("", ""), 0);
        assert_eq!(d("abc", ""), 3);
        assert_eq!(d("", "ab"), 2);
        assert_eq!(d("kitten", "sitting"), 3);
        assert_eq!(d("ab", "ba"), 1);
        // A swapped pair can't then be edited again.
        assert_eq!(d("ca", "abc"), 3);
        assert_eq!(d("MIT", "mit"), 3);
        assert_eq!(d("BSD3", "BSD-3"), 1);
    }

    #[test]
    fn texts_sort_by_distance_ignoring_case_stably() {
        let mut values = ["map", "Maybe", "mop", "List", "MAP"];
        sort_texts("map", &mut values);
        assert_eq!(values, ["map", "MAP", "mop", "Maybe", "List"]);
    }
}
