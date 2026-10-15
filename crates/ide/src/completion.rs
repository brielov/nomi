//! Completion as elm-language-server answers it
//! (`completionProvider.ts`). Some places have lists of their own:
//!
//! - after `import`, the modules the project may import;
//! - in an exposing list, what the module or the imported one exposes;
//! - after a qualifier, its submodules and what its module exposes;
//! - after a record's dot, or in a record, its fields by type;
//! - nothing in a comment, a pattern's name or an import's alias.
//!
//! Each item is written as it writes them: its kind, a sort text of a
//! prefix and its label, its type as label details, Markdown
//! documentation as hover's, and the word before the cursor replaced.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nomi_base::{TextRange, TextSize};
use nomi_compiler::canonical::{Home, Type};
use nomi_compiler::interface::Interface;
use nomi_engine::Snapshot;
use nomi_engine::graph::module_path;
use nomi_engine::index::{Index, Indexed};
use nomi_project::Outline;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{
    SyntaxKind, SyntaxNode, SyntaxToken, child_token, parse_module, significant_tokens,
};

use crate::completion_general::{Possible, possible_imports};
use crate::hover::{Names, declaration_hint, render};
use crate::structure::glob_order;
use crate::tree::{child, code_range, last_child_token, words, written_imports};
use crate::{Edit, next_line};

/// What a completion is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Text,
    Function,
    Field,
    Variable,
    Module,
    Enum,
    Keyword,
    Snippet,
    EnumMember,
    Struct,
    Operator,
}

/// One completion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    /// What the editor sorts it by.
    pub sort_text: Option<String>,
    /// What the editor matches the typed word against, where not its
    /// label.
    pub filter_text: Option<String>,
    /// Markdown.
    pub documentation: Option<String>,
    /// What an auto-import does.
    pub detail: Option<String>,
    /// The type after the label: ` : Int`.
    pub label_detail: Option<String>,
    /// The module after the label, for an auto-import.
    pub label_description: Option<String>,
    /// Whether the editor selects it first.
    pub preselect: bool,
    /// What the completion replaces, and with what; none for a snippet
    /// or a keyword, which the editor inserts as it sees fit.
    pub edit: Option<Edit>,
    /// A snippet's text.
    pub snippet: Option<String>,
    pub additional_edits: Vec<Edit>,
}

impl Completion {
    /// `label`, of `kind`, and nothing else.
    pub(crate) fn of(label: &str, kind: CompletionKind) -> Completion {
        Completion {
            label: label.to_owned(),
            kind,
            sort_text: None,
            filter_text: None,
            documentation: None,
            detail: None,
            label_detail: None,
            label_description: None,
            preselect: false,
            edit: None,
            snippet: None,
            additional_edits: Vec::new(),
        }
    }

    /// `label`, replacing `range` with it, sorted under `sort`.
    pub(crate) fn new(
        label: &str,
        kind: CompletionKind,
        sort: &str,
        range: TextRange,
    ) -> Completion {
        Completion {
            sort_text: Some(format!("{sort}_{label}")),
            edit: Some(Edit {
                range,
                text: label.to_owned(),
            }),
            ..Completion::of(label, kind)
        }
    }

    /// With `documentation`, where it says anything.
    pub(crate) fn documented(self, documentation: Option<String>) -> Completion {
        Completion {
            documentation: documentation.filter(|d| !d.is_empty()),
            ..self
        }
    }

    /// With `detail` as its label details, where it knows every type.
    pub(crate) fn typed(self, detail: Option<String>) -> Completion {
        Completion {
            label_detail: detail.filter(|d| !d.contains("unknown")),
            ..self
        }
    }
}

/// The completions at a place, in the protocol's two shapes; none where
/// elm-language-server answers nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completions {
    /// Items alone.
    Items(Vec<Completion>),
    /// A completion list: items, and whether typing on would change them.
    List {
        items: Vec<Completion>,
        incomplete: bool,
    },
}

/// The completions at `offset` in the file at `path`.
pub fn completions(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
) -> Option<Completions> {
    let module = index.module(path)?;
    let text = &*module.text;
    let cursor = Cursor::of(text, offset);
    let (line, name_at) = (cursor.line, cursor.name());
    let root = module.root();
    if in_comment(&root, offset, &line[..cursor.column]) {
        return Some(Completions::Items(Vec::new()));
    }
    let before = cursor.before();
    // A module's name, after `module`, `port module` or `effect module`.
    if before.ends_with("module")
        && matches!(
            before.split_whitespace().collect::<Vec<_>>()[..],
            ["module"] | ["port" | "effect", "module"]
        )
    {
        return None;
    }
    // Not a pattern's name, nor an import's alias.
    let named = root
        .token_at_offset(offset)
        .find(|t| t.kind() == SyntaxKind::LowerIdent)
        .and_then(|t| t.parent());
    // Nor an import's `as`, which tree-sitter-elm reads as part of it.
    let import = line.trim_start().starts_with("import ");
    if named.is_some_and(|p| p.kind() == SyntaxKind::VarPattern)
        || (import && before.ends_with(" as"))
        || (import && name_at == "as")
    {
        return Some(Completions::Items(Vec::new()));
    }
    if cursor.word_start == 0
        && let Some(label) = declared_around(text, &cursor)
    {
        let found = Completion::new(&label, CompletionKind::Text, "a", cursor.range(0));
        return Some(Completions::Items(vec![found]));
    }
    let context = Context::new(snapshot, index, module, path);
    place_completions(&context, &cursor, offset)
}

/// The completions at `offset`, where `cursor` is, in a place that has
/// some: an exposing list, an import, after a dot, in a record, at a
/// line's start, or anywhere else.
fn place_completions(
    context: &Context<'_>,
    cursor: &Cursor<'_>,
    offset: TextSize,
) -> Option<Completions> {
    let (index, module) = (context.index, context.module);
    let (text, line, before) = (&*module.text, cursor.line, cursor.before());
    let (word, name_at) = (cursor.word(), cursor.name());
    // An exposing list: the module's own, or an import's. A header's
    // `exposing` is part of its list to tree-sitter-elm.
    let list_at = match line[cursor.name_end..].find('(') {
        Some(open) if is_header(line) && name_at == "exposing" => {
            cursor.line_start + cursor.name_end + open + 1
        }
        _ => usize::from(offset),
    };
    if let Some(list) = exposing_list(text, list_at) {
        let replace = cursor.range(cursor.word_start);
        return Some(Completions::Items(match list {
            Exposing::Module(written) => context
                .top_levels(replace, TopLevels::Exposing)
                .into_iter()
                .filter(|c| !written.contains(&c.label))
                .collect(),
            // An import of no module has nothing to say.
            Exposing::Import(name) => context.exposed_by(&name, replace)?,
        }));
    }
    // After `import`, the modules.
    if before.ends_with("import") && !word.contains('.') {
        let found = context.modules(cursor.range(cursor.word_start), None);
        return Some(Completions::Items(found));
    }
    // After a dot: a module's names, or a record's fields; but an
    // accessor being typed, `.name`, gets the general list.
    let accessor = word.starts_with('.') && word.len() > 1 && !word[1..].contains('.');
    if let Some(dot) = word.rfind('.').filter(|_| !accessor) {
        let target = &word[..dot];
        let replace = cursor.range(cursor.word_start + dot + 1);
        let importing = before.ends_with("import");
        if target.starts_with(|c: char| c.is_uppercase()) {
            let found = context.qualified(target, replace, importing);
            if !found.is_empty() || importing {
                return Some(Completions::Items(found));
            }
        }
        let record = TextSize::new((cursor.line_start + cursor.word_start + dot) as u32);
        let fields = crate::record_fields::after(index, module, record, replace);
        return Some(Completions::Items(fields));
    }
    // In a record: its fields.
    let replace = cursor.range(cursor.word_start);
    if let Some(fields) = crate::record_fields::inside(index, module, offset, replace) {
        return Some(Completions::Items(fields));
    }
    // At a line's start: what may start a declaration.
    if cursor.word_start == 0 {
        let found = crate::completion_general::start_of_line(context, replace);
        return Some(Completions::Items(found));
    }
    // Everything else: the general list. Its auto-imports rank by the
    // whole name the cursor is in, as tree-sitter's node there has it. A
    // keyword is no node of its own, so the expression around it is, which
    // ranks nothing first.
    let name = if KEYWORDS.contains(&name_at) {
        ""
    } else {
        name_at
    };
    // An accessor's name is replaced, its dot stays.
    let start = if accessor {
        cursor.word_start + 1
    } else {
        cursor.word_start
    };
    Some(crate::completion_general::general(
        context,
        offset,
        cursor.range(start),
        &line[start..cursor.column],
        name,
    ))
}

/// Elm's reserved words.
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "case", "of", "let", "in", "type", "alias", "module", "exposing",
    "import", "as", "port", "where",
];

/// The cursor's place on its line.
struct Cursor<'t> {
    line: &'t str,
    /// Where the line starts in the text.
    line_start: usize,
    /// How far into the line the cursor is.
    column: usize,
    /// Where the word before the cursor starts: letters, digits, `_` and
    /// dots, qualified names included.
    word_start: usize,
    /// Where the whole name the cursor is in ends, what follows it too.
    name_end: usize,
}

impl<'t> Cursor<'t> {
    fn of(text: &'t str, offset: TextSize) -> Cursor<'t> {
        let at = usize::from(offset);
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let line = &text[line_start..];
        let line = &line[..line.find('\n').unwrap_or(line.len())];
        let column = at - line_start;
        let word_start = (line[..column].char_indices().rev())
            .take_while(|&(_, c)| c.is_alphanumeric() || c == '_' || c == '.')
            .last()
            .map_or(column, |(i, _)| i);
        let name_end = line[column..]
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map_or(line.len(), |i| column + i);
        Cursor {
            line,
            line_start,
            column,
            word_start,
            name_end,
        }
    }

    /// The word before the cursor.
    fn word(&self) -> &'t str {
        &self.line[self.word_start..self.column]
    }

    /// The whole name the cursor is in.
    fn name(&self) -> &'t str {
        &self.line[self.word_start..self.name_end]
    }

    /// What comes before the word on the line, but trailing space.
    fn before(&self) -> &'t str {
        self.line[..self.word_start].trim_end()
    }

    /// From `from` in the line to the cursor.
    fn range(&self, from: usize) -> TextRange {
        let offset = |at: usize| TextSize::new((self.line_start + at) as u32);
        TextRange::new(offset(from), offset(self.column))
    }
}

/// At a line's start: the name an annotation above declares, or the
/// annotation a definition below wants.
fn declared_around(text: &str, cursor: &Cursor<'_>) -> Option<String> {
    let above = text[..cursor.line_start].lines().next_back().unwrap_or("");
    let below = text[cursor.line_start + cursor.line.len()..].trim_start_matches('\n');
    let below = below.lines().next().unwrap_or("");
    let name = |l: &str| -> Option<String> {
        let name: String = l
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        name.starts_with(|c: char| c.is_lowercase()).then_some(name)
    };
    if let Some(name) = name(above).filter(|n| above[n.len()..].trim_start().starts_with(':')) {
        return Some(name);
    }
    // An annotation below too, under a line of just a word being typed,
    // which tree-sitter-elm then reads as running on into it.
    let alone = !cursor.word().is_empty() && cursor.line.trim_end() == cursor.name();
    let wanted = |n: &String| {
        let rest = &below[n.len()..];
        rest.contains('=') || (alone && rest.trim_start().starts_with(':'))
    };
    name(below).filter(wanted).map(|name| format!("{name} : "))
}

/// Whether `line` starts a module's header.
fn is_header(line: &str) -> bool {
    ["module ", "port module ", "effect module "]
        .iter()
        .any(|h| line.starts_with(h))
}

/// Whether `offset` is in a comment.
fn in_comment(root: &SyntaxNode, offset: TextSize, before: &str) -> bool {
    let in_token = root.token_at_offset(offset).any(|t| {
        matches!(
            t.kind(),
            SyntaxKind::LineComment | SyntaxKind::BlockComment | SyntaxKind::DocComment
        ) && t.text_range().start() < offset
    });
    in_token || before.contains("--")
}

enum Exposing {
    /// The module's own, and the names it already has.
    Module(Vec<String>),
    /// An import's, by the module it imports.
    Import(String),
}

/// The exposing list `at` is in, by the text before it: an open `(`
/// after `exposing`, in a header or an import.
fn exposing_list(text: &str, at: usize) -> Option<Exposing> {
    let before = &text[..at];
    let open = before.rfind("exposing")?;
    let rest = &before[open + "exposing".len()..];
    let inside = rest.trim_start().strip_prefix('(')?;
    // Still open: `Msg(..)` closes its own parenthesis.
    if closing_paren(inside).is_some() {
        return None;
    }
    // The declaration the list belongs to: its line's start.
    let start = before[..open].rfind('\n').map_or(0, |i| i + 1);
    let head = before[start..open].trim_start();
    if let Some(name) = head.strip_prefix("import ") {
        let name = name.split_whitespace().next()?;
        return Some(Exposing::Import(name.to_owned()));
    }
    if !is_header(head) {
        return None;
    }
    // The whole list, through the name at the cursor and after it.
    let after = &text[at..];
    let close = closing_paren(after).unwrap_or(after.len());
    let whole = format!("{inside}{}", &after[..close]);
    let written: Vec<String> = (whole.split(','))
        .map(|w| w.trim().to_owned())
        .filter(|w| !w.is_empty())
        .collect();
    Some(Exposing::Module(written))
}

/// Where the `)` that closes a parenthesis open before `text` is.
fn closing_paren(text: &str) -> Option<usize> {
    let mut depth = 0;
    text.char_indices()
        .find(|&(_, c)| {
            match c {
                '(' => depth += 1,
                ')' if depth == 0 => return true,
                ')' => depth -= 1,
                _ => {}
            }
            false
        })
        .map(|(i, _)| i)
}

/// A module the project may import.
pub(crate) struct Importable {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    /// The package it's of: a dependency, or the project, where that's a
    /// package.
    pub(crate) package: Option<String>,
}

/// What completions and code actions read, each read once a request.
pub(crate) struct Context<'c> {
    pub(crate) snapshot: &'c Snapshot,
    pub(crate) index: &'c Index,
    pub(crate) module: &'c Indexed,
    pub(crate) path: &'c Path,
    importable: OnceCell<Vec<Importable>>,
    possible: OnceCell<Vec<Possible>>,
    import_at: OnceCell<TextSize>,
    sources: RefCell<HashMap<String, Option<(SyntaxNode, PathBuf)>>>,
    names: OnceCell<Names>,
    signatures: RefCell<HashMap<String, Rc<HashMap<String, String>>>>,
    exposed: RefCell<HashMap<String, Option<Rc<Vec<ExposedName>>>>>,
}

impl<'c> Context<'c> {
    pub(crate) fn new(
        snapshot: &'c Snapshot,
        index: &'c Index,
        module: &'c Indexed,
        path: &'c Path,
    ) -> Context<'c> {
        Context {
            snapshot,
            index,
            module,
            path,
            importable: OnceCell::new(),
            possible: OnceCell::new(),
            import_at: OnceCell::new(),
            sources: RefCell::default(),
            names: OnceCell::new(),
            signatures: RefCell::default(),
            exposed: RefCell::default(),
        }
    }

    /// [`importable_modules`] of the module's project.
    pub(crate) fn importable_modules(&self) -> &[Importable] {
        self.importable
            .get_or_init(|| importable_modules(self.snapshot, self.index, self.path))
    }

    /// [`possible_imports`] of the module.
    pub(crate) fn possible_imports(&self) -> &[Possible] {
        self.possible.get_or_init(|| possible_imports(self))
    }

    /// How the module writes types' names.
    pub(crate) fn names(&self) -> &Names {
        self.names
            .get_or_init(|| Names::of(self.index, self.module))
    }
}

impl Context<'_> {
    /// `getSameFileTopLevelCompletions`: the module's own values, types
    /// with their variants, aliases and ports, as `b`; with a type's
    /// `(..)` too in its own exposing list.
    pub(crate) fn top_levels(&self, range: TextRange, list: TopLevels) -> Vec<Completion> {
        let root = &self.module.root();
        let item = |name: &SyntaxToken, kind| {
            Completion::new(name.text(), kind, "b", range)
                .documented(declaration_hint(root, name, None))
        };
        let value = |name: &SyntaxToken| {
            item(name, CompletionKind::Function).typed(self.value_detail(name.text()))
        };
        let mut found = Vec::new();
        for decl in root.children() {
            match decl.kind() {
                SyntaxKind::ValueDecl => {
                    let Some(definition) = child(&decl, SyntaxKind::Definition) else {
                        continue;
                    };
                    // One without a body yet is no declaration to it.
                    if !definition.children().any(|c| ast::Expr::can_cast(c.kind())) {
                        continue;
                    }
                    found.extend(
                        child_token(&definition, SyntaxKind::LowerIdent).map(|n| value(&n)),
                    );
                }
                SyntaxKind::PortDecl => {
                    found.extend(child_token(&decl, SyntaxKind::LowerIdent).map(|n| value(&n)));
                }
                SyntaxKind::CustomTypeDecl => {
                    let Some(name) = child_token(&decl, SyntaxKind::UpperIdent) else {
                        continue;
                    };
                    let union = item(&name, CompletionKind::Enum);
                    let open = (list == TopLevels::Exposing).then(|| {
                        let label = format!("{}(..)", name.text());
                        Completion::new(&label, CompletionKind::Enum, "b", range)
                            .documented(union.documentation.clone())
                    });
                    found.push(union);
                    found.extend(open);
                    for variant in decl.children().filter(|c| c.kind() == SyntaxKind::Variant) {
                        let Some(name) = child_token(&variant, SyntaxKind::UpperIdent) else {
                            continue;
                        };
                        found.push(
                            Completion::new(name.text(), CompletionKind::EnumMember, "b", range)
                                .typed(self.value_detail(name.text())),
                        );
                    }
                }
                SyntaxKind::TypeAliasDecl => {
                    let name = child_token(&decl, SyntaxKind::UpperIdent);
                    found.extend(name.map(|name| item(&name, CompletionKind::Struct)));
                }
                _ => {}
            }
        }
        found
    }

    /// A top-level value's or constructor's type, as label details, as the
    /// module writes it.
    fn value_detail(&self, name: &str) -> Option<String> {
        let Some(canonical) = self.module.canonical.as_ref() else {
            // A constructor's type is in its declaration.
            return written_constructor_type(&self.module.root(), name);
        };
        // The module's types, inferred once for every name asked about.
        let inferred = match self.module.types(self.index.compat) {
            Some((Ok(inferred), _)) => inferred.get(name),
            _ => None,
        };
        let Some(annotation) = inferred.or_else(|| canonical.annotations.get(name)) else {
            // A constructor: its type from its union, or as written where
            // the module has no interface yet.
            return match self.index.interfaces.get(&self.module.name) {
                Some(interface) => {
                    constructor_type(self.names(), interface, &self.module.name, name)
                }
                None => written_constructor_type(&self.module.root(), name),
            };
        };
        Some(format!(" : {}", render(self.names(), &annotation.ty)))
    }

    /// `getExposedFromModule`: what the module `name` exposes, for an
    /// import's exposing list, as `c`; none where there's no such module.
    fn exposed_by(&self, name: &str, range: TextRange) -> Option<Vec<Completion>> {
        let (root, _) = self.module_source(name)?;
        let mut found = Vec::new();
        for exposed in self.exposed_names(name)?.iter() {
            let label = exposed.label.as_str();
            let documentation = declaration_hint(&root, &exposed.name, None);
            let item = |label: &str, kind| {
                Completion::new(label, kind, "c", range).documented(documentation.clone())
            };
            match &exposed.kind {
                Exposed::Alias => found.push(item(label, CompletionKind::Struct)),
                Exposed::Type { variants } => {
                    if !variants.is_empty() {
                        found.push(item(&format!("{label}(..)"), CompletionKind::Enum));
                    }
                    found.push(item(label, CompletionKind::Enum));
                }
                Exposed::Value | Exposed::Operator => found.push(
                    item(label, CompletionKind::Function).typed(self.foreign_detail(name, label)),
                ),
            }
        }
        Some(found)
    }

    /// The type of `name` in the module `home`, as label details, as this
    /// module writes it.
    pub(crate) fn foreign_detail(&self, home: &str, name: &str) -> Option<String> {
        let written = self.signature(home, name);
        let names = self.names().clone().written_as(written);
        let interface = self.index.interfaces.get(home)?;
        if let Some(annotation) = interface.values.get(name) {
            return Some(format!(" : {}", render(&names, &annotation.ty)));
        }
        constructor_type(&names, interface, home, name)
    }

    /// `getImportableModules`: the modules the project may import, its
    /// own then its direct dependencies' exposed ones, each package as
    /// its `elm.json` lists them; those under `under.` alone, named past
    /// it, where given.
    fn modules(&self, range: TextRange, under: Option<&str>) -> Vec<Completion> {
        let mut found = Vec::new();
        for module in self.importable_modules() {
            if module.name == self.module.name {
                continue;
            }
            let label = match under {
                Some(under) => match module.name.strip_prefix(under) {
                    Some(rest) if rest.starts_with('.') => &rest[1..],
                    _ => continue,
                },
                None => &module.name,
            };
            let documentation = self.tree_at(&module.path).and_then(|root| {
                let header = child(&root, SyntaxKind::ModuleHeader)?;
                let name = significant_tokens(&child(&header, SyntaxKind::ModuleName)?).next()?;
                let make = || declaration_hint(&root, &name, None);
                if !self.index.is_package_file(&module.path) {
                    return make();
                }
                let at = name.text_range().start();
                self.index.package_files.hint(&module.path, at, make)
            });
            found.push(
                Completion::new(label, CompletionKind::Module, "b", range)
                    .documented(documentation),
            );
        }
        found
    }

    /// `getSubmodulesOrValues`: after `target.`, the submodules under it,
    /// then, outside an import, what its modules expose: the ones
    /// imported as it, or else the ones of that name, with an import.
    fn qualified(&self, target: &str, range: TextRange, importing: bool) -> Vec<Completion> {
        let mut found = self.modules(range, Some(target));
        if importing {
            return found;
        }
        let imports: Vec<_> = written_imports(&self.module.root()).collect();
        let written = (imports.iter()).map(|i| (i.module.as_str(), i.alias.as_deref()));
        let defaults = DEFAULT_IMPORTS.iter().map(|d| (d.module, d.alias));
        // Each module, and whether it's imported.
        let mut matched: Vec<(&str, bool)> = Vec::new();
        for (name, alias) in written.chain(defaults) {
            let named = alias == Some(target) || (alias.is_none() && name == target);
            if named && !matched.iter().any(|(m, _)| *m == name) {
                matched.push((name, true));
            }
        }
        if matched.is_empty() && !imports.iter().any(|i| i.module == target) {
            for module in self.importable_modules() {
                let name = module.name.as_str();
                let ends = (name.strip_suffix(target)).is_some_and(|rest| rest.ends_with('.'));
                if name == target || (ends && !target.contains('.')) {
                    matched.push((name, false));
                }
            }
        }
        for (name, imported) in matched {
            let (Some((root, _)), Some(exposed)) =
                (self.module_source(name), self.exposed_names(name))
            else {
                continue;
            };
            let import = (!imported).then(|| {
                let alias = if target == name {
                    String::new()
                } else {
                    format!(" as '{target}'")
                };
                let detail = format!("Auto import module '{name}'{alias}");
                (detail, self.add_import(name, Some(target), None))
            });
            for exposed in exposed.iter() {
                let start = found.len();
                self.qualified_items(name, &root, exposed, range, &mut found);
                if let Some((detail, edit)) = &import {
                    for completion in &mut found[start..] {
                        completion.detail = Some(detail.clone());
                        completion.additional_edits = vec![edit.clone()];
                    }
                }
            }
        }
        found
    }

    /// What `exposed` in the module `home`, whose tree is `root`, gives
    /// after its qualifier, as `a`: a value with its type, a type with its
    /// variants.
    fn qualified_items(
        &self,
        home: &str,
        root: &SyntaxNode,
        exposed: &ExposedName,
        range: TextRange,
        found: &mut Vec<Completion>,
    ) {
        let label = exposed.label.as_str();
        let item = |label: &str, kind, name: &SyntaxToken, annotation| {
            Completion::new(label, kind, "a", range)
                .documented(declaration_hint(root, name, annotation))
        };
        match &exposed.kind {
            // `getSubmodulesOrValues` has no operators.
            Exposed::Operator => {}
            Exposed::Value => {
                let detail = self.foreign_detail(home, label);
                let annotation = (detail.as_deref())
                    .map(|d| format!("{label} : {}", d.trim_start_matches(" : ")));
                let kind = CompletionKind::Function;
                found.push(item(label, kind, &exposed.name, annotation).typed(detail));
            }
            Exposed::Alias => found.push(item(label, CompletionKind::Struct, &exposed.name, None)),
            Exposed::Type { variants } => {
                found.push(item(label, CompletionKind::Enum, &exposed.name, None));
                for variant in variants {
                    let detail = self.foreign_detail(home, variant.text());
                    let kind = CompletionKind::EnumMember;
                    found.push(item(variant.text(), kind, variant, None).typed(detail));
                }
            }
        }
    }

    /// `RefactorEditUtils.addImport`: `import module`, as `alias` where
    /// that's another name, exposing `value` where given, on the line
    /// after the last import, or after the module's doc comment or header.
    pub(crate) fn add_import(
        &self,
        module: &str,
        alias: Option<&str>,
        value: Option<&str>,
    ) -> Edit {
        let at = *self.import_at.get_or_init(|| {
            let root = &self.module.root();
            let last = (root.children())
                .filter(|n| n.kind() == SyntaxKind::Import)
                .last()
                .and_then(|n| code_range(&n))
                .or_else(|| child_token(root, SyntaxKind::DocComment).map(|t| t.text_range()))
                .or_else(|| child(root, SyntaxKind::ModuleHeader).and_then(|h| code_range(&h)));
            next_line(
                &self.module.text,
                last.map_or(TextSize::new(0), TextRange::end),
            )
        });
        let mut text = format!("import {module}");
        if let Some(alias) = alias.filter(|a| *a != module) {
            let _ = write!(text, " as {alias}");
        }
        if let Some(value) = value {
            let _ = write!(text, " exposing ({value})");
        }
        text.push('\n');
        Edit::insert(at, text)
    }

    /// What the module `name` exposes, as [`exposed`] reads it from
    /// [`Context::module_source`], each module's worked out once.
    pub(crate) fn exposed_names(&self, name: &str) -> Option<Rc<Vec<ExposedName>>> {
        if let Some(names) = self.exposed.borrow().get(name) {
            return names.clone();
        }
        let names = self
            .module_source(name)
            .map(|(root, _)| Rc::new(exposed(&root)));
        (self.exposed.borrow_mut()).insert(name.to_owned(), names.clone());
        names
    }

    /// [`declaration_hint`] of `name` in the module `module`, whose tree is
    /// `root`: made once where the module is a package's, whose sources
    /// never change.
    pub(crate) fn hint(
        &self,
        module: &str,
        root: &SyntaxNode,
        name: &SyntaxToken,
    ) -> Option<String> {
        let make = || declaration_hint(root, name, None);
        let package = (self.module_source(module))
            .map(|(_, path)| path)
            .filter(|path| self.index.is_package_file(path));
        match package {
            Some(path) => {
                let at = name.text_range().start();
                self.index.package_files.hint(&path, at, make)
            }
            None => make(),
        }
    }

    /// The annotation of `name` in the module `home` as written, each
    /// module's read once.
    fn signature(&self, home: &str, name: &str) -> Option<String> {
        let cached = self.signatures.borrow().get(home).cloned();
        let all = if let Some(all) = cached {
            all
        } else {
            let mut all = HashMap::new();
            if let Some((root, _)) = self.module_source(home) {
                for decl in root.children().filter_map(ast::ValueDecl::cast) {
                    let name = decl.definition().and_then(|d| d.lower_ident_token());
                    if let (Some(name), Some(signature)) = (name, decl.signature()) {
                        all.entry(name.text().to_owned())
                            .or_insert_with(|| signature.syntax().text().to_string());
                    }
                }
            }
            let all = Rc::new(all);
            (self.signatures.borrow_mut()).insert(home.to_owned(), all.clone());
            all
        };
        all.get(name).cloned()
    }

    /// The tree and file of the module `name`: a dependency's, as
    /// elm-language-server prefers where the project has one of the same
    /// name too, which elm itself refuses; or else the project's.
    pub(crate) fn module_source(&self, name: &str) -> Option<(SyntaxNode, PathBuf)> {
        if let Some(found) = self.sources.borrow().get(name) {
            return found.clone();
        }
        let found = self.read_source(name);
        (self.sources.borrow_mut()).insert(name.to_owned(), found.clone());
        found
    }

    /// [`Context::module_source`], read.
    fn read_source(&self, name: &str) -> Option<(SyntaxNode, PathBuf)> {
        // The project's own, as the index has them parsed already,
        // where no dependency has one of the name.
        let dependency = (self.importable_modules().iter())
            .skip(self.index.modules.len())
            .find(|m| m.name == name);
        let path = match (dependency, self.index.named(name)) {
            (Some(dependency), _) => dependency.path.clone(),
            (None, Some(own)) => return Some((own.root(), own.path.clone())),
            (None, None) => crate::navigation::module_file(self.snapshot, self.index, name)?,
        };
        Some((self.tree_at(&path)?, path))
    }

    /// The tree of the module at `path`: the index's where it's the
    /// project's, parsed once where it's a package's, whose sources never
    /// change, or else read and parsed.
    fn tree_at(&self, path: &Path) -> Option<SyntaxNode> {
        let own = (self.index.modules.iter()).find(|m| m.path.as_os_str() == path.as_os_str());
        if let Some(own) = own {
            return Some(own.root());
        }
        if self.index.is_package_file(path) {
            return self.index.package_files.tree(path);
        }
        Some(parse_module(&self.snapshot.read(path).ok()?).syntax())
    }
}

/// A constructor's type as its custom type's declaration writes it: its
/// arguments, then the type with its parameters.
fn written_constructor_type(root: &SyntaxNode, name: &str) -> Option<String> {
    let variant = (root.children())
        .filter_map(ast::CustomTypeDecl::cast)
        .flat_map(|d| d.variants())
        .find(|v| v.upper_ident_token().is_some_and(|t| t.text() == name))?;
    let union = variant.syntax().parent()?;
    let mut result = child_token(&union, SyntaxKind::UpperIdent)?
        .text()
        .to_owned();
    for param in union
        .children()
        .filter(|c| c.kind() == SyntaxKind::TypeParam)
    {
        result.push(' ');
        result.push_str(&words(&param));
    }
    let mut parts: Vec<String> = variant.types().map(|t| words(t.syntax())).collect();
    parts.push(result);
    Some(format!(" : {}", parts.join(" -> ")))
}

/// A constructor's type as label details: its arguments, then its
/// union, which an interface of `module` declares.
fn constructor_type(
    names: &Names,
    interface: &Interface,
    module: &str,
    name: &str,
) -> Option<String> {
    let (union_name, union, ctor) = interface.unions.iter().find_map(|(union_name, union)| {
        let union = union.whole();
        let ctor = union.ctors.iter().find(|c| c.name == name)?;
        Some((union_name, union, ctor))
    })?;
    let home = Home {
        package: interface.home.as_str().into(),
        module: module.into(),
    };
    let result = Type::Type {
        home,
        name: union_name.clone(),
        args: union.vars.iter().map(|v| Type::Var(v.clone())).collect(),
    };
    let ty = Type::function(ctor.args.iter().cloned(), result);
    Some(format!(" : {}", render(names, &ty)))
}

/// What a module exposes, in its exposing list's order.
pub(crate) enum Exposed {
    /// An operator, which an explicit list exposes, and its infix
    /// declaration.
    Operator,
    Value,
    Alias,
    Type {
        variants: Vec<SyntaxToken>,
    },
}

/// Whether an exposing list's type comes with its variants, as `T(..)`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Variants {
    With,
    Without,
}

impl Variants {
    /// What the exposed type `item` says.
    pub(crate) fn of(item: &SyntaxNode) -> Variants {
        match child(item, SyntaxKind::ExposedVariants) {
            Some(_) => Variants::With,
            None => Variants::Without,
        }
    }
}

/// The list the module's own top-level names are offered in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopLevels {
    /// Its exposing list, which offers a type's `(..)` too.
    Exposing,
    General,
}

/// A name a module exposes.
pub(crate) struct ExposedName {
    /// What it's called: an operator's symbol.
    pub(crate) label: String,
    /// The name its declaration has: an operator's function's, as
    /// `TreeUtils.findOperator` finds it.
    pub(crate) name: SyntaxToken,
    pub(crate) kind: Exposed,
}

impl ExposedName {
    /// The declaration `decl`, whose name is `name`, exposed: a custom
    /// type with its variants where `variants` says.
    fn of(decl: &SyntaxNode, name: &SyntaxToken, variants: Variants) -> ExposedName {
        let kind = match decl.kind() {
            SyntaxKind::TypeAliasDecl => Exposed::Alias,
            SyntaxKind::CustomTypeDecl => Exposed::Type {
                variants: (decl.children())
                    .filter(|_| variants == Variants::With)
                    .filter(|c| c.kind() == SyntaxKind::Variant)
                    .filter_map(|v| child_token(&v, SyntaxKind::UpperIdent))
                    .collect(),
            },
            _ => Exposed::Value,
        };
        ExposedName {
            label: name.text().to_owned(),
            name: name.clone(),
            kind,
        }
    }
}

/// The names a module's header exposes, as `ISourceFile.exposing` has
/// them (`binder.ts`). An explicit list gives its values, then its types,
/// each in the list's order. `(..)` gives the module's names in the order
/// they first went into its scope: the default imports' names first,
/// then its declarations'.
pub(crate) fn exposed(root: &SyntaxNode) -> Vec<ExposedName> {
    let list = (ast::Module::cast(root.clone()))
        .and_then(|m| m.module_header())
        .and_then(|header| header.exposing_list());
    let Some(list) = list else {
        return Vec::new();
    };
    let declarations: Vec<(SyntaxNode, SyntaxToken)> = (root.children())
        .filter_map(|decl| {
            let token = match decl.kind() {
                SyntaxKind::ValueDecl => child(&decl, SyntaxKind::Definition)
                    .and_then(|d| child_token(&d, SyntaxKind::LowerIdent)),
                SyntaxKind::PortDecl => child_token(&decl, SyntaxKind::LowerIdent),
                SyntaxKind::CustomTypeDecl | SyntaxKind::TypeAliasDecl => {
                    child_token(&decl, SyntaxKind::UpperIdent)
                }
                _ => None,
            }?;
            Some((decl, token))
        })
        .collect();
    if list.dot_dot_token().is_none() {
        return listed(root, &list, &declarations);
    }
    let mut order: Vec<&(SyntaxNode, SyntaxToken)> = declarations.iter().collect();
    order.sort_by_key(|(_, name)| {
        (DEFAULT_IMPORTS.iter())
            .position(|d| d.alias.unwrap_or(d.module) == name.text())
            .unwrap_or(DEFAULT_IMPORTS.len())
    });
    (order.into_iter())
        .map(|(decl, name)| ExposedName::of(decl, name, Variants::With))
        .collect()
}

/// The names an explicit exposing `list` of the module `root` exposes,
/// as `declarations` declare them: its operators, then its values, then
/// its types.
fn listed(
    root: &SyntaxNode,
    list: &ast::ExposingList,
    declarations: &[(SyntaxNode, SyntaxToken)],
) -> Vec<ExposedName> {
    // Each name's first declaration, for the list's names to look up.
    let mut by_name: HashMap<&str, &(SyntaxNode, SyntaxToken)> = HashMap::new();
    for declaration in declarations {
        by_name.entry(declaration.1.text()).or_insert(declaration);
    }
    let declared = |name: &str| by_name.get(name).copied();
    let items: Vec<SyntaxNode> = list.syntax().children().collect();
    let mut found = Vec::new();
    for item in (items.iter()).filter(|i| i.kind() == SyntaxKind::ExposedOperator) {
        let op = operator(item);
        let infix = (root.children())
            .filter(|n| n.kind() == SyntaxKind::InfixDecl)
            .find(|n| {
                n.children()
                    .any(|c| c.kind() == SyntaxKind::Op && words(&c) == op)
            });
        let function = infix.and_then(|i| last_child_token(&i, SyntaxKind::LowerIdent));
        let name = function.and_then(|f| declared(f.text()).map(|(_, token)| token.clone()));
        if let Some(name) = name {
            found.push(ExposedName {
                label: op,
                name,
                kind: Exposed::Operator,
            });
        }
    }
    for item in (items.iter()).filter(|i| i.kind() == SyntaxKind::ExposedValue) {
        if let Some((decl, name)) = declared(&words(item)) {
            found.push(ExposedName::of(decl, name, Variants::Without));
        }
    }
    for item in (items.iter()).filter(|i| i.kind() == SyntaxKind::ExposedType) {
        let name = child_token(item, SyntaxKind::UpperIdent);
        if let Some((decl, token)) = name.and_then(|name| declared(name.text())) {
            found.push(ExposedName::of(decl, token, Variants::of(item)));
        }
    }
    found
}

/// The symbol an exposing list's operator entry names.
pub(crate) fn operator(item: &SyntaxNode) -> String {
    let written = words(item);
    (written.trim_start_matches('(').trim_end_matches(')').trim()).to_owned()
}

/// One of the imports every module has, as elm-language-server writes
/// them out.
pub(crate) struct DefaultImport {
    pub(crate) module: &'static str,
    pub(crate) alias: Option<&'static str>,
    /// Its exposing list, where it has one.
    pub(crate) exposing: Option<DefaultExposing>,
}

/// A default import's exposing list: everything, or these types, each
/// with its variants or not, and operators.
pub(crate) enum DefaultExposing {
    All,
    Explicit {
        types: &'static [(&'static str, Variants)],
        operators: &'static [&'static str],
    },
}

/// The modules every module imports, in elm-language-server's order.
pub(crate) const DEFAULT_IMPORTS: [DefaultImport; 11] = {
    const fn import(module: &'static str, exposing: Option<DefaultExposing>) -> DefaultImport {
        DefaultImport {
            module,
            alias: None,
            exposing,
        }
    }
    const fn types(
        module: &'static str,
        types: &'static [(&'static str, Variants)],
    ) -> DefaultImport {
        let operators = &[];
        import(module, Some(DefaultExposing::Explicit { types, operators }))
    }
    [
        import("Basics", Some(DefaultExposing::All)),
        import(
            "List",
            Some(DefaultExposing::Explicit {
                types: &[("List", Variants::Without)],
                operators: &["::"],
            }),
        ),
        types("Maybe", &[("Maybe", Variants::With)]),
        types("Result", &[("Result", Variants::With)]),
        types("String", &[("String", Variants::Without)]),
        types("Char", &[("Char", Variants::Without)]),
        import("Tuple", None),
        import("Debug", None),
        types("Platform", &[("Program", Variants::Without)]),
        DefaultImport {
            alias: Some("Cmd"),
            ..types("Platform.Cmd", &[("Cmd", Variants::Without)])
        },
        DefaultImport {
            alias: Some("Sub"),
            ..types("Platform.Sub", &[("Sub", Variants::Without)])
        },
    ]
};

/// The modules the project of `path` may import: its own, then each
/// direct dependency's exposed modules, in `elm.json` order.
fn importable_modules(snapshot: &Snapshot, index: &Index, path: &Path) -> Vec<Importable> {
    let mut found: Vec<Importable> = (index.modules.iter())
        .map(|m| Importable {
            name: m.name.clone(),
            path: m.path.clone(),
            package: None,
        })
        .collect();
    // As elm-language-server finds the files: a directory's files, then
    // its subdirectories', each by name.
    found.sort_by_cached_key(|m| glob_order(&m.path));
    let direct: Vec<String> = match crate::project_outline(snapshot, path) {
        None => return found,
        Some((_, Outline::App(app), _)) => app.direct.keys().map(ToString::to_string).collect(),
        Some((_, Outline::Pkg(pkg), _)) => {
            // A package's own modules are of its package, which ranks them
            // among the dependencies.
            for module in &mut found {
                module.package = Some(pkg.name.to_string());
            }
            // elm-language-server offers its test dependencies' modules
            // too, everywhere in it.
            (pkg.deps.keys().chain(pkg.test_deps.keys()))
                .map(ToString::to_string)
                .collect()
        }
    };
    for package in direct {
        let (Some(dir), Some(exposed)) =
            (index.packages.get(&package), index.exposed.get(&package))
        else {
            continue;
        };
        let src = dir.join("src");
        let mut files: Vec<(&String, PathBuf)> = (exposed.iter())
            .map(|module| (module, src.join(module_path(module, "elm"))))
            .filter(|(_, file)| snapshot.is_file(file))
            .collect();
        // In the order elm-language-server finds them.
        files.sort_by_cached_key(|(_, file)| glob_order(file));
        files.dedup_by(|a, b| a.1 == b.1);
        found.extend(files.into_iter().map(|(module, path)| Importable {
            name: module.clone(),
            path,
            package: Some(package.clone()),
        }));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposing_lists_are_told_by_the_text_before() {
        let text = "module Test exposing (a, b, ";
        assert!(
            matches!(exposing_list(text, text.len()), Some(Exposing::Module(w)) if w == ["a", "b"])
        );
        let text = "module Test exposing (..)\n\nimport Foo exposing (";
        assert!(matches!(exposing_list(text, text.len()), Some(Exposing::Import(n)) if n == "Foo"));
        let text = "module Test exposing (a)\n\nf = ";
        assert!(exposing_list(text, text.len()).is_none());
    }
}
