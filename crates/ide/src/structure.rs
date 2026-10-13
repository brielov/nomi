//! A file's structure as elm-language-server shows it:
//! folding ranges (`foldingProvider.ts`), selection ranges
//! (`selectionRangeProvider.ts`), the linked editing of a name and its
//! annotation (`linkedEditingRangesProvider.ts`), and the project's
//! symbols matching a query (`workspaceSymbolProvider.ts`). Each walks
//! tree-sitter-elm's nodes there, so each walks their counterparts here.

use std::path::{Path, PathBuf};

use nomi_base::{TextRange, TextSize};
use nomi_engine::Snapshot;
use nomi_engine::index::Index;
use nomi_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, parse_module};

use crate::symbols::{Symbol, child, code_range};

/// What a fold holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldKind {
    Region,
    Imports,
    Comment,
}

/// A folding range: from its start to its end, the start a line up
/// where it says so, as elm-language-server folds a branch from the line
/// before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fold {
    pub start: TextSize,
    pub end: TextSize,
    pub line_before: bool,
    pub kind: FoldKind,
}

/// The folding ranges of the file at `path`, in the order of its tree.
pub fn folding_ranges(snapshot: &Snapshot, path: &Path) -> std::io::Result<Vec<Fold>> {
    let text = snapshot.read(path)?;
    let root = parse_module(&text).syntax();
    // Each fold with where its tree-sitter node starts: the walk there
    // meets a doc comment before the declaration, where here the
    // declaration holds it.
    let mut keyed: Vec<(TextSize, Fold)> = Vec::new();
    let mut regions: Vec<TextSize> = Vec::new();
    let fold = |range: TextRange, kind| Fold {
        start: range.start(),
        end: range.end(),
        line_before: false,
        kind,
    };
    let line_of = |at: TextSize| text[..usize::from(at)].matches('\n').count();
    for element in root.descendants_with_tokens() {
        match element {
            SyntaxElement::Token(token) => match token.kind() {
                SyntaxKind::BlockComment | SyntaxKind::DocComment => {
                    keyed.push((
                        token.text_range().start(),
                        fold(token.text_range(), FoldKind::Comment),
                    ));
                }
                SyntaxKind::LineComment => {
                    let comment = token.text();
                    if is_region(comment, "endregion") {
                        let start = regions.pop();
                        if let Some(start) =
                            start.filter(|s| line_of(*s) < line_of(token.text_range().end()))
                        {
                            let range = TextRange::new(start, token.text_range().end());
                            keyed.push((token.text_range().start(), fold(range, FoldKind::Region)));
                        }
                    } else if is_region(comment, "region") {
                        regions.push(token.text_range().start());
                    }
                }
                _ => {}
            },
            SyntaxElement::Node(node) => {
                let key =
                    code_range(&node).map_or_else(|| node.text_range().start(), |r| r.start());
                let mut folds: Vec<Fold> = Vec::new();
                match node.kind() {
                    SyntaxKind::Import => {
                        let previous = node
                            .prev_sibling()
                            .filter(|p| p.kind() == SyntaxKind::Import);
                        if previous.is_none_or(|p| comment_between(&p, &node)) {
                            let mut last = node.clone();
                            while let Some(next) = last
                                .next_sibling()
                                .filter(|n| n.kind() == SyntaxKind::Import)
                            {
                                if comment_between(&last, &next) {
                                    break;
                                }
                                last = next;
                            }
                            if let (Some(first), Some(last)) =
                                (code_range(&node), code_range(&last))
                            {
                                folds.push(fold(first.cover(last), FoldKind::Imports));
                            }
                        }
                    }
                    SyntaxKind::LetExpr => {
                        let Some(range) = code_range(&node) else {
                            continue;
                        };
                        let decls: Vec<SyntaxNode> = (node.children())
                            .filter(|c| {
                                matches!(
                                    c.kind(),
                                    SyntaxKind::LetValue | SyntaxKind::LetDestructure
                                )
                            })
                            .collect();
                        let last = decls.last().and_then(|d| match d.kind() {
                            SyntaxKind::LetValue => {
                                child(d, SyntaxKind::Definition).and_then(|def| code_range(&def))
                            }
                            _ => code_range(d),
                        });
                        if let Some(last) = last {
                            folds.push(fold(
                                TextRange::new(range.start(), last.end()),
                                FoldKind::Region,
                            ));
                        }
                        let body = node
                            .children()
                            .filter(|c| crate::completion::is_expr(c.kind()))
                            .last();
                        if let Some(body) = body.and_then(|b| code_range(&b)) {
                            folds.push(Fold {
                                start: body.start(),
                                end: range.end(),
                                line_before: true,
                                kind: FoldKind::Region,
                            });
                        }
                    }
                    // An `else if` chain is one expression, as tree-sitter-elm
                    // reads it: its conditions, branches and comments fold,
                    // but the first, and the `if` after an `else` doesn't
                    // again.
                    SyntaxKind::IfExpr if !is_else_if(&node) => {
                        let mut parts = Vec::new();
                        let mut chain = Some(node.clone());
                        while let Some(link) = chain.take() {
                            for part in link
                                .children()
                                .filter(|c| crate::completion::is_expr(c.kind()))
                            {
                                // Comments are parts of their own to
                                // tree-sitter-elm, those before a part too.
                                parts.extend(
                                    (part.descendants_with_tokens())
                                        .filter_map(|e| e.into_token())
                                        .take_while(|t| t.kind().is_trivia())
                                        .filter(|t| {
                                            matches!(
                                                t.kind(),
                                                SyntaxKind::LineComment | SyntaxKind::BlockComment
                                            )
                                        })
                                        .map(|t| t.text_range()),
                                );
                                if part.kind() == SyntaxKind::IfExpr && is_else_if(&part) {
                                    chain = Some(part);
                                } else {
                                    parts.extend(code_range(&part));
                                }
                            }
                        }
                        for range in parts.into_iter().skip(1) {
                            folds.push(Fold {
                                start: range.start(),
                                end: range.end(),
                                line_before: true,
                                kind: FoldKind::Region,
                            });
                        }
                    }
                    // tree-sitter-elm's `case` in parentheses takes them in.
                    SyntaxKind::CaseExpr
                        if node
                            .parent()
                            .is_some_and(|p| p.kind() == SyntaxKind::ParenExpr) =>
                    {
                        if let Some(range) = node.parent().as_ref().and_then(code_range) {
                            folds.push(fold(range, FoldKind::Region));
                        }
                    }
                    SyntaxKind::Definition
                    | SyntaxKind::DestructureDecl
                    | SyntaxKind::LetDestructure
                    | SyntaxKind::TypeAliasDecl
                    | SyntaxKind::CaseExpr
                    | SyntaxKind::CaseBranch
                    | SyntaxKind::RecordExpr
                    | SyntaxKind::RecordUpdateExpr => {
                        if let Some(range) = code_range(&node) {
                            folds.push(fold(range, FoldKind::Region));
                        }
                    }
                    SyntaxKind::CustomTypeDecl => {
                        if let Some(range) = crate::symbols::declaration_range(&node) {
                            folds.push(fold(range, FoldKind::Region));
                        }
                    }
                    _ => {}
                }
                keyed.extend(folds.into_iter().map(|f| (key, f)));
            }
        }
    }
    keyed.sort_by_key(|(key, _)| *key);
    Ok(keyed.into_iter().map(|(_, f)| f).collect())
}

/// Whether a line comment opens or closes a region: `-- region`, or
/// `-- #endregion`.
fn is_region(comment: &str, word: &str) -> bool {
    let rest = comment.trim_start().trim_start_matches("--").trim_start();
    let rest = rest.strip_prefix('#').unwrap_or(rest);
    rest.strip_prefix(word)
        .is_some_and(|after| !after.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
}

/// Whether a comment lies between two nodes, which tree-sitter makes a
/// node between them.
fn comment_between(a: &SyntaxNode, b: &SyntaxNode) -> bool {
    let mut next = a.last_token().and_then(|t| t.next_token());
    while let Some(token) = next {
        if token.text_range().start() >= b.text_range().start() && !token.kind().is_trivia() {
            return false;
        }
        if matches!(
            token.kind(),
            SyntaxKind::LineComment | SyntaxKind::BlockComment | SyntaxKind::DocComment
        ) && token.text_range().end()
            <= code_range(b).map_or_else(|| b.text_range().start(), |r| r.start())
        {
            return true;
        }
        next = token.next_token();
    }
    false
}

/// The ranges around `offset` in the file at `path`, innermost first:
/// the name there, then each node around it, the file last, each once.
pub fn selection_ranges(
    snapshot: &Snapshot,
    path: &Path,
    offset: TextSize,
) -> std::io::Result<Vec<TextRange>> {
    let text = snapshot.read(path)?;
    let root = parse_module(&text).syntax();
    let mut found: Vec<TextRange> = Vec::new();
    // tree-sitter starts from the named node under the cursor: a name or
    // a literal; past punctuation and space, the node around them.
    let token = root.token_at_offset(offset).right_biased();
    let named = |kind: SyntaxKind| {
        matches!(
            kind,
            SyntaxKind::LowerIdent
                | SyntaxKind::UpperIdent
                | SyntaxKind::Int
                | SyntaxKind::Float
                | SyntaxKind::String
                | SyntaxKind::Char
                | SyntaxKind::MultilineString
        )
    };
    if let Some(token) = token.as_ref().filter(|t| named(t.kind())) {
        found.push(token.text_range());
    }
    let start = token
        .and_then(|t| t.parent())
        .unwrap_or_else(|| root.clone());
    for node in start.ancestors() {
        // Its tree keeps an annotation and a doc comment beside the
        // declaration, not in it.
        if matches!(node.kind(), SyntaxKind::ValueDecl | SyntaxKind::LetValue) {
            continue;
        }
        let range = if node.parent().is_none() {
            node.text_range()
        } else {
            match code_range(&node) {
                Some(range) => range,
                None => continue,
            }
        };
        if found.last() != Some(&range) {
            found.push(range);
        }
    }
    Ok(found)
}

/// The names linked with the one at `offset` in the file at `path`: an
/// annotation's and its definition's; or, on a line after an annotation
/// that ends in `=`, those lines' starts.
pub fn linked_editing(
    snapshot: &Snapshot,
    path: &Path,
    offset: TextSize,
) -> std::io::Result<Vec<TextRange>> {
    let text = snapshot.read(path)?;
    let root = parse_module(&text).syntax();
    let Some(token) = root
        .token_at_offset(offset)
        .find(|t| t.kind() == SyntaxKind::LowerIdent)
    else {
        return Ok(lines_linked(&text, offset));
    };
    let parent = token.parent();
    let pair = match parent.as_ref().map(SyntaxNode::kind) {
        Some(SyntaxKind::Signature) => (parent.as_ref())
            .and_then(|s| s.next_sibling())
            .filter(|n| n.kind() == SyntaxKind::Definition)
            .and_then(|d| crate::lint::first_token(&d, SyntaxKind::LowerIdent))
            .filter(|n| n.text() == token.text())
            .map(|n| vec![token.text_range(), n.text_range()]),
        Some(SyntaxKind::Definition) => (parent.as_ref())
            .filter(|d| {
                crate::lint::first_token(d, SyntaxKind::LowerIdent).as_ref() == Some(&token)
            })
            .and_then(|d| d.prev_sibling())
            .filter(|n| n.kind() == SyntaxKind::Signature)
            .and_then(|s| crate::lint::first_token(&s, SyntaxKind::LowerIdent))
            .filter(|n| n.text() == token.text())
            .map(|n| vec![n.text_range(), token.text_range()]),
        _ => None,
    };
    Ok(pair.unwrap_or_else(|| lines_linked(&text, offset)))
}

/// Without names, two lines' starts: one starting with `:` and one
/// ending in `=` below it.
fn lines_linked(text: &str, offset: TextSize) -> Vec<TextRange> {
    let at = usize::from(offset);
    let lines: Vec<&str> = text.split('\n').collect();
    let line = text[..at].matches('\n').count();
    let start_of = |l: usize| -> TextSize {
        TextSize::new(lines[..l].iter().map(|x| x.len() + 1).sum::<usize>() as u32)
    };
    let here = lines.get(line).map_or("", |l| l.trim());
    if here.starts_with(':') && lines.get(line + 1).is_some_and(|n| n.trim().ends_with('=')) {
        return vec![
            TextRange::empty(start_of(line)),
            TextRange::empty(start_of(line + 1)),
        ];
    }
    if here.ends_with('=') && line > 0 && lines[line - 1].trim().starts_with(':') {
        return vec![
            TextRange::empty(start_of(line - 1)),
            TextRange::empty(start_of(line)),
        ];
    }
    Vec::new()
}

/// The project's symbols whose names have `query`'s letters in order,
/// each with its file, the closest matches first.
pub fn workspace_symbols(
    snapshot: &Snapshot,
    index: &Index,
    query: &str,
) -> Vec<(PathBuf, Symbol)> {
    let mut paths: Vec<PathBuf> = index.modules.iter().map(|m| m.path.clone()).collect();
    paths.sort_by_key(|p| glob_order(p));
    paths.dedup();
    let mut found = Vec::new();
    for path in paths {
        let Ok(symbols) = crate::document_symbols(snapshot, &path) else {
            continue;
        };
        for symbol in symbols {
            if let Some(matched) = match_in_symbol(query, &symbol.name) {
                found.push((matched, path.clone(), symbol));
            }
        }
    }
    found.sort_by_key(|(m, _, _)| *m);
    found
        .into_iter()
        .map(|(_, path, symbol)| (path, symbol))
        .collect()
}

/// A path's place in the order a glob finds files: a directory's files,
/// then its subdirectories'.
fn glob_order(path: &Path) -> Vec<(bool, String)> {
    let parts: Vec<String> = path
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let last = parts.len().saturating_sub(1);
    (parts.into_iter().enumerate())
        .map(|(i, part)| (i < last, part))
        .collect()
}

/// `matchInSymbol`: where the query's letters, in order, end in the
/// name, how much longer the name is, and whether the casing differs,
/// as its sorter compares them.
fn match_in_symbol(typed: &str, symbol: &str) -> Option<(isize, isize, u8)> {
    let typed_lower: Vec<char> = typed.to_lowercase().chars().collect();
    let symbol_lower: Vec<char> = symbol.to_lowercase().chars().collect();
    let (mut t, mut s) = (0, 0);
    while t < typed_lower.len() && s < symbol_lower.len() {
        if typed_lower[t] == symbol_lower[s] {
            t += 1;
        }
        s += 1;
    }
    if t != typed_lower.len() {
        return None;
    }
    let position = s as isize - typed_lower.len() as isize;
    let length_difference = symbol_lower.len() as isize - typed_lower.len() as isize;
    // `substring(position, typedLength)`, as JavaScript reads it: from
    // the smaller to the larger.
    let chars: Vec<char> = symbol.chars().collect();
    let (from, to) = {
        let (a, b) = (position.max(0) as usize, typed.chars().count());
        (a.min(b).min(chars.len()), a.max(b).min(chars.len()))
    };
    let casing = u8::from(chars[from..to].iter().collect::<String>() != typed);
    Some((position, length_difference, casing))
}

/// Whether `node`, an `if`, is the `else` branch of another.
fn is_else_if(node: &SyntaxNode) -> bool {
    node.parent().is_some_and(|parent| {
        parent.kind() == SyntaxKind::IfExpr
            && (parent.children())
                .filter(|c| crate::completion::is_expr(c.kind()))
                .last()
                .as_ref()
                == Some(node)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_match_by_their_letters_in_order() {
        assert_eq!(match_in_symbol("tw", "twice"), Some((0, 3, 0)));
        assert_eq!(match_in_symbol("ce", "twice"), Some((3, 3, 1)));
        assert_eq!(match_in_symbol("zz", "twice"), None);
    }

    #[test]
    fn regions_open_and_close_by_their_words() {
        assert!(is_region("-- region helpers", "region"));
        assert!(is_region("--#endregion", "endregion"));
        assert!(!is_region("-- regions", "region"));
    }
}
