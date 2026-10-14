//! A file's structure as elm-language-server shows it:
//! folding ranges (`foldingProvider.ts`), selection ranges
//! (`selectionRangeProvider.ts`), the linked editing of a name and its
//! annotation (`linkedEditingRangesProvider.ts`), and the project's
//! symbols matching a query (`workspaceSymbolProvider.ts`). Each walks
//! tree-sitter-elm's nodes there, so each walks their counterparts here.

use std::path::{Path, PathBuf};

use nomi_base::{LineIndex, TextRange, TextSize};
use nomi_engine::Snapshot;
use nomi_engine::index::Index;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, child_token, parse_module, tokens};

use crate::navigation::{annotated_name, annotation_name};
use crate::symbols::{Symbol, symbols};
use crate::tree::{child, code_range, declaration_range};

/// What a fold holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldKind {
    /// Code, or a `-- region` to its `-- endregion`.
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

/// The fold of `range`, from its own line.
fn fold(range: TextRange, kind: FoldKind) -> Fold {
    Fold {
        start: range.start(),
        end: range.end(),
        line_before: false,
        kind,
    }
}

/// The fold of `range`, a branch, from the line before it.
fn branch(range: TextRange) -> Fold {
    Fold {
        line_before: true,
        ..fold(range, FoldKind::Region)
    }
}

/// The folding ranges of the file at `path`, in the order of its tree.
pub fn folding_ranges(snapshot: &Snapshot, path: &Path) -> std::io::Result<Vec<Fold>> {
    let text = snapshot.read(path)?;
    let root = parse_module(&text).syntax();
    let lines = LineIndex::new(&text);
    // Each fold with where its tree-sitter node starts: the walk there
    // meets a doc comment before the declaration, where here the
    // declaration holds it.
    let mut keyed: Vec<(TextSize, Fold)> = Vec::new();
    let mut regions: Vec<TextSize> = Vec::new();
    for element in root.descendants_with_tokens() {
        match element {
            SyntaxElement::Token(token) => {
                let range = token.text_range();
                match token.kind() {
                    SyntaxKind::BlockComment | SyntaxKind::DocComment => {
                        keyed.push((range.start(), fold(range, FoldKind::Comment)));
                    }
                    SyntaxKind::LineComment if is_region(token.text(), "endregion") => {
                        let start = regions.pop();
                        if let Some(start) =
                            start.filter(|s| lines.line(*s) < lines.line(range.end()))
                        {
                            let region = TextRange::new(start, range.end());
                            keyed.push((range.start(), fold(region, FoldKind::Region)));
                        }
                    }
                    SyntaxKind::LineComment if is_region(token.text(), "region") => {
                        regions.push(range.start());
                    }
                    _ => {}
                }
            }
            SyntaxElement::Node(node) => {
                let folds = node_folds(&node);
                if !folds.is_empty() {
                    let key =
                        code_range(&node).map_or_else(|| node.text_range().start(), |r| r.start());
                    keyed.extend(folds.into_iter().map(|f| (key, f)));
                }
            }
        }
    }
    keyed.sort_by_key(|(key, _)| *key);
    Ok(keyed.into_iter().map(|(_, f)| f).collect())
}

/// The folds tree-sitter-elm's counterpart of `node` makes.
fn node_folds(node: &SyntaxNode) -> Vec<Fold> {
    let whole = |range: Option<TextRange>| {
        range
            .map(|r| fold(r, FoldKind::Region))
            .into_iter()
            .collect()
    };
    match node.kind() {
        SyntaxKind::Import => import_folds(node),
        SyntaxKind::LetExpr => let_folds(node),
        SyntaxKind::IfExpr if !is_else_if(node) => if_folds(node),
        // tree-sitter-elm's `case` in parentheses takes them in.
        SyntaxKind::CaseExpr
            if node
                .parent()
                .is_some_and(|p| p.kind() == SyntaxKind::ParenExpr) =>
        {
            whole(node.parent().as_ref().and_then(code_range))
        }
        SyntaxKind::Definition
        | SyntaxKind::DestructureDecl
        | SyntaxKind::LetDestructure
        | SyntaxKind::TypeAliasDecl
        | SyntaxKind::CaseExpr
        | SyntaxKind::CaseBranch
        | SyntaxKind::RecordExpr
        | SyntaxKind::RecordUpdateExpr => whole(code_range(node)),
        SyntaxKind::CustomTypeDecl => whole(declaration_range(node)),
        _ => Vec::new(),
    }
}

/// An import's fold, where it starts a run of imports no comment parts:
/// through the run's last.
fn import_folds(node: &SyntaxNode) -> Vec<Fold> {
    let import = |n: &SyntaxNode| n.kind() == SyntaxKind::Import;
    if !node
        .prev_sibling()
        .filter(import)
        .is_none_or(|p| comment_between(&p, node))
    {
        return Vec::new();
    }
    let mut last = node.clone();
    while let Some(next) = last.next_sibling().filter(import) {
        if comment_between(&last, &next) {
            break;
        }
        last = next;
    }
    let range = code_range(node).zip(code_range(&last));
    (range.into_iter())
        .map(|(first, last)| fold(first.cover(last), FoldKind::Imports))
        .collect()
}

/// A `let`'s folds: through its last declaration, and its body from the
/// line before it.
fn let_folds(node: &SyntaxNode) -> Vec<Fold> {
    let Some(range) = code_range(node) else {
        return Vec::new();
    };
    let mut folds = Vec::new();
    let last = (node.children())
        .filter(|c| matches!(c.kind(), SyntaxKind::LetValue | SyntaxKind::LetDestructure))
        .last()
        .and_then(|d| match d.kind() {
            SyntaxKind::LetValue => {
                child(&d, SyntaxKind::Definition).and_then(|def| code_range(&def))
            }
            _ => code_range(&d),
        });
    if let Some(last) = last {
        folds.push(fold(
            TextRange::new(range.start(), last.end()),
            FoldKind::Region,
        ));
    }
    let body = (node.children())
        .filter(|c| ast::Expr::can_cast(c.kind()))
        .last();
    if let Some(body) = body.and_then(|b| code_range(&b)) {
        folds.push(branch(TextRange::new(body.start(), range.end())));
    }
    folds
}

/// An `else if` chain's folds: it is one expression, as tree-sitter-elm
/// reads it, whose conditions, branches and comments fold, but the first,
/// and the `if` after an `else` doesn't again.
fn if_folds(node: &SyntaxNode) -> Vec<Fold> {
    let mut parts = Vec::new();
    let mut chain = Some(node.clone());
    while let Some(link) = chain.take() {
        for part in (link.children()).filter(|c| ast::Expr::can_cast(c.kind())) {
            // Comments are parts of their own to tree-sitter-elm, those
            // before a part too.
            parts.extend(
                tokens(&part)
                    .take_while(|t| t.kind().is_trivia())
                    .filter(|t| {
                        matches!(t.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment)
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
    parts.into_iter().skip(1).map(branch).collect()
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
    let b_start = code_range(b).map_or_else(|| b.text_range().start(), |r| r.start());
    let mut next = a.last_token().and_then(|t| t.next_token());
    while let Some(token) = next {
        if token.text_range().start() >= b.text_range().start() && !token.kind().is_trivia() {
            return false;
        }
        if matches!(
            token.kind(),
            SyntaxKind::LineComment | SyntaxKind::BlockComment | SyntaxKind::DocComment
        ) && token.text_range().end() <= b_start
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
            .and_then(annotated_name)
            .filter(|n| n.text() == token.text())
            .map(|n| vec![token.text_range(), n.text_range()]),
        Some(SyntaxKind::Definition) => (parent.as_ref())
            .filter(|d| child_token(d, SyntaxKind::LowerIdent).as_ref() == Some(&token))
            .and_then(annotation_name)
            .filter(|n| n.text() == token.text())
            .map(|n| vec![n.text_range(), token.text_range()]),
        _ => None,
    };
    Ok(pair.unwrap_or_else(|| lines_linked(&text, offset)))
}

/// Without names, two lines' starts: one starting with `:` and one
/// ending in `=` below it.
fn lines_linked(text: &str, offset: TextSize) -> Vec<TextRange> {
    let lines = LineIndex::new(text);
    let line = lines.line(offset);
    // A line's start, and its text without the space around it.
    let line_at = |line: u32| {
        (lines.line_content(text, line).ok()).map(|r| (TextRange::empty(r.start()), text[r].trim()))
    };
    let Some((here, written)) = line_at(line) else {
        return Vec::new();
    };
    if written.starts_with(':')
        && let Some((below, next)) = line_at(line + 1)
        && next.ends_with('=')
    {
        return vec![here, below];
    }
    if written.ends_with('=')
        && let Some((above, previous)) = line.checked_sub(1).and_then(line_at)
        && previous.starts_with(':')
    {
        return vec![above, here];
    }
    Vec::new()
}

/// The project's symbols whose names have `query`'s letters in order,
/// each with its file, the closest matches first.
pub fn workspace_symbols(index: &Index, query: &str) -> Vec<(PathBuf, Symbol)> {
    let mut modules: Vec<_> = index.modules.iter().collect();
    modules.sort_by_cached_key(|m| glob_order(&m.path));
    modules.dedup_by(|a, b| a.path == b.path);
    let query = Query::new(query);
    let mut found = Vec::new();
    for module in modules {
        for symbol in symbols(&module.root()) {
            if let Some(matched) = query.match_in(&symbol.name) {
                found.push((matched, module.path.clone(), symbol));
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
pub(crate) fn glob_order(path: &Path) -> Vec<(bool, String)> {
    let parts: Vec<String> = path
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let last = parts.len().saturating_sub(1);
    (parts.into_iter().enumerate())
        .map(|(i, part)| (i < last, part))
        .collect()
}

/// A workspace query, as `matchInSymbol` reads it for every symbol.
struct Query<'q> {
    typed: &'q str,
    lower: Vec<char>,
    /// How many characters it has.
    length: usize,
}

/// How a symbol matches a query, as its sorter compares them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SymbolMatch {
    /// Where the query's letters end in the name.
    position: isize,
    /// How much longer the name is.
    length_difference: isize,
    /// Whether the casing differs.
    casing: u8,
}

impl Query<'_> {
    fn new(typed: &str) -> Query<'_> {
        Query {
            typed,
            lower: typed.to_lowercase().chars().collect(),
            length: typed.chars().count(),
        }
    }

    /// `matchInSymbol`: how `symbol` has the query's letters in order,
    /// where it does.
    fn match_in(&self, symbol: &str) -> Option<SymbolMatch> {
        let typed_lower = &self.lower;
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
            let (a, b) = (position.max(0) as usize, self.length);
            (a.min(b).min(chars.len()), a.max(b).min(chars.len()))
        };
        let casing = u8::from(!chars[from..to].iter().copied().eq(self.typed.chars()));
        Some(SymbolMatch {
            position,
            length_difference,
            casing,
        })
    }
}

/// Whether `node`, an `if`, is the `else` branch of another.
fn is_else_if(node: &SyntaxNode) -> bool {
    node.parent().is_some_and(|parent| {
        parent.kind() == SyntaxKind::IfExpr
            && (parent.children())
                .filter(|c| ast::Expr::can_cast(c.kind()))
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
        let found = |typed, symbol| {
            let m = Query::new(typed).match_in(symbol)?;
            Some((m.position, m.length_difference, m.casing))
        };
        assert_eq!(found("tw", "twice"), Some((0, 3, 0)));
        assert_eq!(found("ce", "twice"), Some((3, 3, 1)));
        assert_eq!(found("zz", "twice"), None);
    }

    #[test]
    fn regions_open_and_close_by_their_words() {
        assert!(is_region("-- region helpers", "region"));
        assert!(is_region("--#endregion", "endregion"));
        assert!(!is_region("-- regions", "region"));
    }
}
