//! The names a module's doc comment lists after `@docs`, which order its
//! `exposing` list.
//!
//! Upstream reads them from its Markdown parse (Cheapskate's
//! `processElts`): a top-level text line starting `@docs`, and each text
//! line right after it, is a group of comma-separated names. Until plan
//! 2.13 parses doc comments as Markdown, this scans lines for the blocks
//! that keep a line from being top-level text: fenced and indented code,
//! block quotes, lists, headers and rules.

/// Each `@docs` line's names, in order; groups with no names left out.
pub(super) fn documented(docs: &str) -> Vec<Vec<String>> {
    let mut groups = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut previous = Previous::Blank;
    let mut in_container = false;
    for line in docs.lines() {
        let indent = indentation(line);
        let rest = line.trim_start_matches([' ', '\t']);
        if let Some((mark, length)) = fence {
            if indent < 4 && closes(rest, mark, length) {
                fence = None;
            }
            continue;
        }
        if rest.is_empty() {
            previous = Previous::Blank;
            continue;
        }
        if in_container {
            // A container goes on through lazy continuation lines, and
            // through indented ones after a blank.
            let continues = previous != Previous::Blank || indent >= 2 || rest.starts_with('>');
            if continues {
                previous = Previous::Other;
                continue;
            }
            in_container = false;
        }
        let text = previous.is_text();
        if indent >= 4 && !text {
            previous = Previous::Other;
            continue;
        }
        let stripped = &line[line.len() - rest.len()..];
        let leaf = if indent >= 4 {
            strip_three(line)
        } else {
            stripped
        };
        if let Some(open) = opens_fence(leaf) {
            fence = Some(open);
            previous = Previous::Other;
            continue;
        }
        if leaf.starts_with('>') || is_list_item(leaf) && !is_rule(leaf) {
            in_container = true;
            previous = Previous::Other;
            continue;
        }
        if is_header(leaf) || is_rule(leaf) || text && is_setext_underline(leaf) {
            previous = Previous::Other;
            continue;
        }
        match leaf.strip_prefix("@docs") {
            Some(names) => {
                groups.push(names);
                previous = Previous::Docs;
            }
            None if previous == Previous::Docs => groups.push(leaf),
            None => previous = Previous::Text,
        }
    }
    groups
        .into_iter()
        .map(|group| {
            group
                .split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|group| !group.is_empty())
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Previous {
    Blank,
    /// A paragraph's line.
    Text,
    /// An `@docs` line, or a text line after one.
    Docs,
    Other,
}

impl Previous {
    fn is_text(self) -> bool {
        matches!(self, Previous::Text | Previous::Docs)
    }
}

/// The column `line`'s text starts at, with tabs to multiples of 4.
fn indentation(line: &str) -> usize {
    let mut column = 0;
    for c in line.chars() {
        match c {
            ' ' => column += 1,
            '\t' => column += 4 - column % 4,
            _ => break,
        }
    }
    column
}

/// `line` without up to three leading spaces.
fn strip_three(line: &str) -> &str {
    let spaces = line.bytes().take(3).take_while(|&b| b == b' ').count();
    &line[spaces..]
}

fn opens_fence(leaf: &str) -> Option<(char, usize)> {
    let mark = leaf.chars().next().filter(|&c| c == '`' || c == '~')?;
    let length = leaf.chars().take_while(|&c| c == mark).count();
    (length >= 3).then_some((mark, length))
}

fn closes(rest: &str, mark: char, length: usize) -> bool {
    let run = rest.chars().take_while(|&c| c == mark).count();
    run >= length && rest[run * mark.len_utf8()..].trim().is_empty()
}

fn is_list_item(leaf: &str) -> bool {
    let bullet = leaf
        .strip_prefix(['-', '+', '*'])
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']));
    let digits = leaf.bytes().take_while(u8::is_ascii_digit).count();
    let ordered = digits > 0
        && leaf[digits..]
            .strip_prefix(['.', ')'])
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']));
    bullet || ordered
}

fn is_header(leaf: &str) -> bool {
    let hashes = leaf.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&hashes) && (leaf.len() == hashes || leaf[hashes..].starts_with([' ', '\t']))
}

/// `***`, `---` or `___`, with spaces between.
fn is_rule(leaf: &str) -> bool {
    ['*', '-', '_'].into_iter().any(|mark| {
        leaf.chars().filter(|&c| c == mark).count() >= 3
            && leaf.chars().all(|c| c == mark || c == ' ' || c == '\t')
    })
}

fn is_setext_underline(leaf: &str) -> bool {
    let trimmed = leaf.trim_end();
    !trimmed.is_empty()
        && (trimmed.bytes().all(|b| b == b'=') || trimmed.bytes().all(|b| b == b'-'))
}

#[cfg(test)]
mod tests;
