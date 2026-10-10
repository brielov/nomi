//! Cases made by editing a seed case: one replacement in one of its
//! files. A table of edits is a TSV whose replacement text escapes `\`,
//! tab, line feed and carriage return, so each edit is one line.

use crate::transcript::Case;

/// A replacement of `start..end` in the seed's `file` by `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub id: String,
    pub seed: String,
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub text: String,
    /// What the edit does, for people.
    pub note: String,
}

pub const HEADER: &str = "id\tseed\tfile\tstart\tend\ttext\tnote";

impl Edit {
    /// `seed` with the edit made. Panics if the edit doesn't fit it.
    pub fn apply(&self, seed: &Case) -> Case {
        let mut case = seed.clone();
        let (_, text) = (case.files.iter_mut())
            .find(|(path, _)| *path == self.file)
            .unwrap_or_else(|| panic!("{}: the seed has no {}", self.id, self.file));
        text.replace_range(self.start..self.end, &self.text);
        case
    }

    pub fn to_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.id,
            self.seed,
            self.file,
            self.start,
            self.end,
            escape(&self.text),
            escape(&self.note)
        )
    }
}

/// The edits of a table, skipping `#` comments and the header.
pub fn parse_table(text: &str) -> Result<Vec<Edit>, String> {
    let mut edits = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#') && *l != HEADER) {
        let cells: Vec<&str> = line.split('\t').collect();
        let [id, seed, file, start, end, replacement, note] = cells[..] else {
            return Err(format!("not seven cells: {line}"));
        };
        let number = |s: &str| s.parse::<usize>().map_err(|e| format!("{line}: {e}"));
        edits.push(Edit {
            id: id.to_owned(),
            seed: seed.to_owned(),
            file: file.to_owned(),
            start: number(start)?,
            end: number(end)?,
            text: unescape(replacement)?,
            note: unescape(note)?,
        });
    }
    Ok(edits)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next() {
            Some('\\') => '\\',
            Some('t') => '\t',
            Some('n') => '\n',
            Some('r') => '\r',
            other => return Err(format!("bad escape \\{other:?} in {text}")),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_round_trip_and_apply() {
        let edit = Edit {
            id: "s/1".to_owned(),
            seed: "s".to_owned(),
            file: "A.elm".to_owned(),
            start: 2,
            end: 3,
            text: "x\t\\\n".to_owned(),
            note: "a note".to_owned(),
        };
        let table = format!("# comment\n{HEADER}\n{}\n", edit.to_line());
        assert_eq!(parse_table(&table).unwrap(), std::slice::from_ref(&edit));
        let seed = Case {
            args: vec![],
            stdin: String::new(),
            files: vec![("A.elm".to_owned(), "abcd".to_owned())],
            ..Case::default()
        };
        assert_eq!(edit.apply(&seed).files[0].1, "abx\t\\\nd");
    }
}
