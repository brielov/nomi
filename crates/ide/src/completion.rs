//! Completion (plans 9.8 and 9.9) as elm-language-server answers it
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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nomi_base::{TextRange, TextSize};
use nomi_engine::Snapshot;
use nomi_engine::index::{Index, Indexed};
use nomi_project::Outline;
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, parse_module};

use crate::Edit;
use crate::symbols::{child, code_range, token, words};

/// What a completion is, as the protocol numbers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Text = 1,
    Function = 3,
    Field = 5,
    Variable = 6,
    Module = 9,
    Enum = 13,
    Keyword = 14,
    Snippet = 15,
    EnumMember = 20,
    Struct = 22,
    Operator = 24,
}

/// One completion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    pub sort_text: Option<String>,
    pub filter_text: Option<String>,
    /// Markdown.
    pub documentation: Option<String>,
    pub detail: Option<String>,
    /// The type after the label: ` : Int`.
    pub label_detail: Option<String>,
    /// The module after the label, for an auto-import.
    pub label_description: Option<String>,
    pub preselect: bool,
    /// What the completion replaces, and with what; none for a snippet
    /// or a keyword, which the editor inserts as it sees fit.
    pub edit: Option<Edit>,
    /// A snippet's text.
    pub snippet: Option<String>,
    pub additional_edits: Vec<Edit>,
}

/// The completions at a place: a list, or a list and whether it's
/// incomplete; none where elm-language-server answers nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completions {
    List(Vec<Completion>),
    Incomplete(Vec<Completion>, bool),
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
    let at = usize::from(offset);
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[line_start..];
    let line = &line[..line.find('\n').unwrap_or(line.len())];
    let column = at - line_start;
    // The word before the cursor, qualified names included.
    let word_start = word_start(line, column);
    let word = &line[word_start..column];
    // The whole name the cursor is in, what follows it too.
    let name_end = line[column..]
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map_or(line.len(), |i| column + i);
    let name_at = &line[word_start..name_end];
    let range = |from: usize| TextRange::new(TextSize::new((line_start + from) as u32), offset);
    let context = Context::new(snapshot, index, module, path);
    if in_comment(&module.root, offset, &line[..column]) {
        return Some(Completions::List(Vec::new()));
    }
    let before = line[..word_start].trim_end();
    // A module's name, after `module`, `port module` or `effect module`.
    if before.ends_with("module")
        && matches!(
            before.split_whitespace().collect::<Vec<_>>()[..],
            ["module"] | ["port", "module"] | ["effect", "module"]
        )
    {
        return None;
    }
    // Not a pattern's name, nor an import's alias.
    let named = module
        .root
        .token_at_offset(offset)
        .find(|t| t.kind() == SyntaxKind::LowerIdent)
        .and_then(|t| t.parent());
    // Nor an import's `as`, which tree-sitter-elm reads as part of it.
    let import = line.trim_start().starts_with("import ");
    if named.is_some_and(|p| p.kind() == SyntaxKind::VarPattern)
        || (import && before.ends_with(" as"))
        || (import && name_at == "as")
    {
        return Some(Completions::List(Vec::new()));
    }
    // At a line's start: the name an annotation above declares, or the
    // annotation a definition below wants.
    if word_start == 0 {
        let mut lines = text[..line_start].lines().rev();
        let above = lines.next().unwrap_or("");
        let below = text[line_start + line.len()..].trim_start_matches('\n');
        let below = below.lines().next().unwrap_or("");
        let name = |l: &str| -> Option<String> {
            let name: String = l
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            name.starts_with(|c: char| c.is_lowercase()).then_some(name)
        };
        if let Some(name) = name(above).filter(|n| above[n.len()..].trim_start().starts_with(':')) {
            return Some(Completions::List(vec![item(
                &name,
                CompletionKind::Text,
                "a",
                range(0),
                None,
                None,
            )]));
        }
        // An annotation below too, under a line of just a word being
        // typed, which tree-sitter-elm then reads as running on into it.
        let alone = !word.is_empty() && line.trim_end() == name_at;
        let wanted = |n: &String| {
            let rest = &below[n.len()..];
            rest.contains('=') || (alone && rest.trim_start().starts_with(':'))
        };
        if let Some(name) = name(below).filter(wanted) {
            let label = format!("{name} : ");
            return Some(Completions::List(vec![item(
                &label,
                CompletionKind::Text,
                "a",
                range(0),
                None,
                None,
            )]));
        }
    }
    // An exposing list: the module's own, or an import's. A header's
    // `exposing` is part of its list to tree-sitter-elm.
    let header = ["module ", "port module ", "effect module "]
        .iter()
        .any(|h| line.starts_with(h));
    let list_at = match line[name_end..].find('(') {
        Some(open) if header && name_at == "exposing" => line_start + name_end + open + 1,
        _ => at,
    };
    if let Some(list) = exposing_list(text, list_at) {
        let replace = range(word_start);
        return Some(Completions::List(match list {
            Exposing::Module(written) => context
                .top_levels(replace, true)
                .into_iter()
                .filter(|c| !written.contains(&c.label))
                .collect(),
            // An import of no module has nothing to say.
            Exposing::Import(name) => context.exposed_by(&name, replace)?,
        }));
    }
    // After `import`, the modules.
    if before.ends_with("import") && !word.contains('.') {
        return Some(Completions::List(
            context.importable(range(word_start), None),
        ));
    }
    // After a dot: a module's names, or a record's fields; but an
    // accessor being typed, `.name`, gets the general list.
    let accessor = word.starts_with('.') && word.len() > 1 && !word[1..].contains('.');
    if let Some(dot) = word.rfind('.').filter(|_| !accessor) {
        let target = &word[..dot];
        let replace = range(word_start + dot + 1);
        let importing = before.ends_with("import");
        if !target.is_empty() && target.starts_with(|c: char| c.is_uppercase()) {
            let found = context.qualified(target, replace, importing);
            if !found.is_empty() || importing {
                return Some(Completions::List(found));
            }
        }
        let record = TextSize::new((line_start + word_start + dot) as u32);
        let fields = crate::record_fields::after(snapshot, index, module, record, replace);
        return Some(Completions::List(fields));
    }
    // In a record: its fields.
    if let Some(fields) =
        crate::record_fields::inside(snapshot, index, module, offset, range(word_start))
    {
        return Some(Completions::List(fields));
    }
    // At a line's start: what may start a declaration.
    if word_start == 0 {
        return Some(Completions::List(crate::completion_general::start_of_line(
            &context,
            range(0),
        )));
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
    let start = if accessor { word_start + 1 } else { word_start };
    Some(crate::completion_general::general(
        &context,
        offset,
        range(start),
        &line[start..column],
        name,
    ))
}

/// Elm's reserved words.
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "case", "of", "let", "in", "type", "alias", "module", "exposing",
    "import", "as", "port", "where",
];

/// Where the word before `column` starts: letters, digits, `_` and dots.
fn word_start(line: &str, column: usize) -> usize {
    let mut start = column;
    for (i, c) in line[..column].char_indices().rev() {
        if c.is_alphanumeric() || c == '_' || c == '.' {
            start = i;
        } else {
            break;
        }
    }
    start
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
    let rest = rest.trim_start();
    let inside = rest.strip_prefix('(')?;
    // Still open: `Msg(..)` closes its own parenthesis.
    let mut depth = 0;
    for c in inside.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return None,
            ')' => depth -= 1,
            _ => {}
        }
    }
    // The declaration the list belongs to: its line's start.
    let start = before[..open].rfind('\n').map_or(0, |i| i + 1);
    let head = &before[start..open];
    if let Some(name) = head.trim_start().strip_prefix("import ") {
        let name = name.split_whitespace().next()?;
        return Some(Exposing::Import(name.to_owned()));
    }
    let header = head.trim_start();
    if header.starts_with("module ")
        || header.starts_with("port module ")
        || header.starts_with("effect module ")
    {
        // The whole list, through the name at the cursor and after it.
        let after = &text[at..];
        let mut depth = 0;
        let close = after
            .char_indices()
            .find(|&(_, c)| {
                match c {
                    '(' => depth += 1,
                    ')' if depth == 0 => return true,
                    ')' => depth -= 1,
                    _ => {}
                }
                false
            })
            .map_or(after.len(), |(i, _)| i);
        let whole = format!("{inside}{}", &after[..close]);
        let written: Vec<String> = (whole.split(','))
            .map(|w| w.trim().to_owned())
            .filter(|w| !w.is_empty())
            .collect();
        return Some(Exposing::Module(written));
    }
    None
}

/// What completions read.
/// A module the project may import, its file and its package.
pub(crate) type Importable = (String, Option<PathBuf>, Option<String>);

pub(crate) struct Context<'c> {
    pub snapshot: &'c Snapshot,
    pub index: &'c Index,
    pub module: &'c Indexed,
    pub path: &'c Path,
    /// What one completion reads again and again, read once.
    importable: std::cell::OnceCell<Vec<Importable>>,
    sources: std::cell::RefCell<HashMap<String, Option<(SyntaxNode, PathBuf)>>>,
    names: std::cell::OnceCell<crate::hover::Names>,
    signatures: std::cell::RefCell<HashMap<String, std::rc::Rc<HashMap<String, String>>>>,
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
            importable: std::cell::OnceCell::new(),
            sources: std::cell::RefCell::new(HashMap::new()),
            names: std::cell::OnceCell::new(),
            signatures: std::cell::RefCell::new(HashMap::new()),
        }
    }

    /// [`importable_modules`] of the module's project.
    pub(crate) fn importable_modules(&self) -> &[Importable] {
        self.importable
            .get_or_init(|| importable_modules(self.snapshot, self.index, self.path))
    }

    /// How the module writes types' names.
    pub(crate) fn names(&self) -> &crate::hover::Names {
        self.names
            .get_or_init(|| crate::hover::Names::of(self.index, self.module))
    }
}

impl Context<'_> {
    /// `getSameFileTopLevelCompletions`: the module's own values, types
    /// with their variants, aliases and ports, as `b`; with a type's
    /// `(..)` too in its own exposing list.
    pub(crate) fn top_levels(&self, range: TextRange, exposing: bool) -> Vec<Completion> {
        let root = &self.module.root;
        let mut found = Vec::new();
        for decl in root.children() {
            match decl.kind() {
                SyntaxKind::ValueDecl => {
                    let Some(definition) = child(&decl, SyntaxKind::Definition) else {
                        continue;
                    };
                    // One without a body yet is no declaration to it.
                    if !definition.children().any(|c| is_expr(c.kind())) {
                        continue;
                    }
                    let Some(name) = crate::lint::first_token(&definition, SyntaxKind::LowerIdent)
                    else {
                        continue;
                    };
                    let documentation = crate::hover::declaration_hint(root, &name, None);
                    let detail = self.value_detail(name.text());
                    found.push(item(
                        name.text(),
                        CompletionKind::Function,
                        "b",
                        range,
                        documentation,
                        detail,
                    ));
                }
                SyntaxKind::PortDecl => {
                    let Some(name) = crate::lint::first_token(&decl, SyntaxKind::LowerIdent) else {
                        continue;
                    };
                    let documentation = crate::hover::declaration_hint(root, &name, None);
                    let detail = self.value_detail(name.text());
                    found.push(item(
                        name.text(),
                        CompletionKind::Function,
                        "b",
                        range,
                        documentation,
                        detail,
                    ));
                }
                SyntaxKind::CustomTypeDecl => {
                    let Some(name) = crate::lint::first_token(&decl, SyntaxKind::UpperIdent) else {
                        continue;
                    };
                    let documentation = crate::hover::declaration_hint(root, &name, None);
                    found.push(item(
                        name.text(),
                        CompletionKind::Enum,
                        "b",
                        range,
                        documentation.clone(),
                        None,
                    ));
                    if exposing {
                        let label = format!("{}(..)", name.text());
                        found.push(item(
                            &label,
                            CompletionKind::Enum,
                            "b",
                            range,
                            documentation,
                            None,
                        ));
                    }
                    for variant in decl.children().filter(|c| c.kind() == SyntaxKind::Variant) {
                        let Some(name) = token(&variant, SyntaxKind::UpperIdent) else {
                            continue;
                        };
                        let detail = self.value_detail(&name);
                        found.push(item(
                            &name,
                            CompletionKind::EnumMember,
                            "b",
                            range,
                            None,
                            detail,
                        ));
                    }
                }
                SyntaxKind::TypeAliasDecl => {
                    let Some(name) = crate::lint::first_token(&decl, SyntaxKind::UpperIdent) else {
                        continue;
                    };
                    let documentation = crate::hover::declaration_hint(root, &name, None);
                    found.push(item(
                        name.text(),
                        CompletionKind::Struct,
                        "b",
                        range,
                        documentation,
                        None,
                    ));
                }
                _ => {}
            }
        }
        found
    }

    /// A top-level value's or constructor's type, as label details, as the
    /// module writes it.
    pub(crate) fn value_detail(&self, name: &str) -> Option<String> {
        let Some(canonical) = self.module.canonical.as_ref() else {
            // A constructor's type is in its declaration.
            return written_constructor_type(&self.module.root, name);
        };
        let localizer = self.names();
        // The module's types, inferred once for every name asked about.
        let inferred = self
            .module
            .types(self.index.compat)
            .map(|(inferred, _)| inferred);
        let annotation = match inferred {
            Some(Ok(inferred)) => inferred.get(name).cloned(),
            _ => None,
        }
        .or_else(|| canonical.annotations.get(name).cloned());
        let ty = match annotation {
            Some(annotation) => (*annotation.ty).clone(),
            None => {
                // A constructor: its type from its union, or as written
                // where the module has no interface yet.
                return match self.index.interfaces.get(&self.module.name) {
                    Some(interface) => {
                        constructor_type(localizer, interface, &self.module.name, name)
                    }
                    None => written_constructor_type(&self.module.root, name),
                };
            }
        };
        Some(format!(" : {}", crate::hover::render(localizer, &ty)))
    }

    /// `getExposedFromModule`: what the module `name` exposes, for an
    /// import's exposing list, as `c`; none where there's no such module.
    fn exposed_by(&self, name: &str, range: TextRange) -> Option<Vec<Completion>> {
        let (root, _) = self.module_source(name)?;
        let mut found = Vec::new();
        for exposed in exposed(&root) {
            let documentation = crate::hover::declaration_hint(&root, &exposed.name, None);
            match exposed.kind {
                Exposed::Alias => found.push(item(
                    exposed.label.as_str(),
                    CompletionKind::Struct,
                    "c",
                    range,
                    documentation,
                    None,
                )),
                Exposed::Type { variants } => {
                    if !variants.is_empty() {
                        let label = format!("{}(..)", exposed.label.as_str());
                        found.push(item(
                            &label,
                            CompletionKind::Enum,
                            "c",
                            range,
                            documentation.clone(),
                            None,
                        ));
                    }
                    found.push(item(
                        exposed.label.as_str(),
                        CompletionKind::Enum,
                        "c",
                        range,
                        documentation,
                        None,
                    ));
                }
                Exposed::Value | Exposed::Operator => {
                    let detail = self.foreign_detail(name, exposed.label.as_str());
                    found.push(item(
                        exposed.label.as_str(),
                        CompletionKind::Function,
                        "c",
                        range,
                        documentation,
                        detail,
                    ));
                }
            }
        }
        Some(found)
    }

    /// The type of `name` in the module `home`, as label details, as this
    /// module writes it.
    pub(crate) fn foreign_detail(&self, home: &str, name: &str) -> Option<String> {
        let written = self.signature(home, name);
        let localizer = self.names().clone().written_as(written);
        let interface = self.index.interfaces.get(home)?;
        if let Some(annotation) = interface.values.get(name) {
            return Some(format!(
                " : {}",
                crate::hover::render(&localizer, &annotation.ty)
            ));
        }
        constructor_type(&localizer, interface, home, name)
    }

    /// `getImportableModules`: the modules the project may import, its
    /// own then its direct dependencies' exposed ones, each package as
    /// its `elm.json` lists them; those under `under.` alone, named past
    /// it, where given.
    fn importable(&self, range: TextRange, under: Option<&str>) -> Vec<Completion> {
        let mut found = Vec::new();
        for (name, path, _) in self.importable_modules().to_vec() {
            if name == self.module.name {
                continue;
            }
            let label = match under {
                Some(under) => match name.strip_prefix(&format!("{under}.")) {
                    Some(rest) => rest.to_owned(),
                    None => continue,
                },
                None => name.clone(),
            };
            let documentation = path
                .and_then(|path| self.snapshot.read(&path).ok())
                .and_then(|text| {
                    let root = parse_module(&text).syntax();
                    let header = child(&root, SyntaxKind::ModuleHeader)?;
                    let name = child(&header, SyntaxKind::ModuleName)?.first_token()?;
                    crate::hover::declaration_hint(&root, &name, None)
                });
            found.push(item(
                &label,
                CompletionKind::Module,
                "b",
                range,
                documentation,
                None,
            ));
        }
        found
    }

    /// `getSubmodulesOrValues`: after `target.`, the submodules under it,
    /// then, outside an import, what its modules expose: the ones
    /// imported as it, or else the ones of that name, with an import.
    fn qualified(&self, target: &str, range: TextRange, importing: bool) -> Vec<Completion> {
        let mut found = self.importable(range, Some(target));
        if importing {
            return found;
        }
        let imports: Vec<(String, Option<String>)> = (self.module.root.children())
            .filter(|n| n.kind() == SyntaxKind::Import)
            .filter_map(|import| {
                let name = words(&child(&import, SyntaxKind::ModuleName)?);
                let alias = child(&import, SyntaxKind::ImportAlias)
                    .and_then(|a| token(&a, SyntaxKind::UpperIdent));
                Some((name, alias))
            })
            .collect();
        let defaults = default_imports();
        let mut matched: Vec<(String, bool)> = Vec::new();
        for (name, alias) in imports
            .iter()
            .map(|(n, a)| (n.clone(), a.clone()))
            .chain(defaults)
        {
            let named = alias.as_deref() == Some(target) || (alias.is_none() && name == target);
            if named && !matched.iter().any(|(m, _)| *m == name) {
                matched.push((name, true));
            }
        }
        if matched.is_empty() && !imports.iter().any(|(n, _)| n == target) {
            for (name, _, _) in self.importable_modules().to_vec() {
                let fits = if target.contains('.') {
                    name == target
                } else {
                    name == target || name.ends_with(&format!(".{target}"))
                };
                if fits {
                    matched.push((name, false));
                }
            }
        }
        for (name, imported) in matched {
            let Some((root, _)) = self.module_source(&name) else {
                continue;
            };
            let import_edit = (!imported).then(|| self.add_import(&name, target));
            for exposed in exposed(&root) {
                let mut completions = Vec::new();
                match &exposed.kind {
                    // `getSubmodulesOrValues` has no operators.
                    Exposed::Operator => {}
                    Exposed::Value => {
                        let detail = self.foreign_detail(&name, exposed.label.as_str());
                        let ty = detail
                            .as_deref()
                            .map(|d| d.trim_start_matches(" : ").to_owned());
                        let annotation = ty.map(|ty| format!("{} : {ty}", exposed.label.as_str()));
                        let documentation =
                            crate::hover::declaration_hint(&root, &exposed.name, annotation);
                        completions.push(item(
                            exposed.label.as_str(),
                            CompletionKind::Function,
                            "a",
                            range,
                            documentation,
                            detail,
                        ));
                    }
                    Exposed::Alias => {
                        let documentation =
                            crate::hover::declaration_hint(&root, &exposed.name, None);
                        completions.push(item(
                            exposed.label.as_str(),
                            CompletionKind::Struct,
                            "a",
                            range,
                            documentation,
                            None,
                        ));
                    }
                    Exposed::Type { variants } => {
                        let documentation =
                            crate::hover::declaration_hint(&root, &exposed.name, None);
                        completions.push(item(
                            exposed.label.as_str(),
                            CompletionKind::Enum,
                            "a",
                            range,
                            documentation,
                            None,
                        ));
                        for variant in variants {
                            let detail = self.foreign_detail(&name, variant.text());
                            let documentation =
                                crate::hover::declaration_hint(&root, variant, None);
                            completions.push(item(
                                variant.text(),
                                CompletionKind::EnumMember,
                                "a",
                                range,
                                documentation,
                                detail,
                            ));
                        }
                    }
                }
                for mut completion in completions {
                    if let Some(edit) = &import_edit {
                        let alias = if target != name {
                            format!(" as '{target}'")
                        } else {
                            String::new()
                        };
                        completion.detail = Some(format!("Auto import module '{name}'{alias}"));
                        completion.additional_edits = vec![edit.clone()];
                    }
                    found.push(completion);
                }
            }
        }
        found
    }

    /// `RefactorEditUtils.addImport`: `import name` (as `alias`) on the
    /// line after the last import, or after the header.
    pub(crate) fn add_import(&self, name: &str, alias: &str) -> Edit {
        let root = &self.module.root;
        let text = &*self.module.text;
        // After the last import, or the module's doc comment, or its
        // header.
        let last = (root.children())
            .filter(|n| n.kind() == SyntaxKind::Import)
            .last()
            .and_then(|n| code_range(&n))
            .or_else(|| {
                crate::lint::first_token(root, SyntaxKind::DocComment).map(|t| t.text_range())
            })
            .or_else(|| child(root, SyntaxKind::ModuleHeader).and_then(|h| code_range(&h)));
        let line = last.map_or(1, |r| {
            text[..usize::from(r.end())].matches('\n').count() + 1
        });
        let at = text
            .match_indices('\n')
            .nth(line.saturating_sub(1))
            .map_or(text.len(), |(i, _)| i + 1);
        let at = if line == 0 { 0 } else { at };
        let alias = if alias != name {
            format!(" as {alias}")
        } else {
            String::new()
        };
        Edit {
            range: TextRange::empty(TextSize::new(at as u32)),
            text: format!("import {name}{alias}\n"),
        }
    }

    /// The source of the module `name`: a dependency's, as
    /// elm-language-server prefers where the project has one of the same
    /// name too, which elm itself refuses; or else the project's.
    /// The annotation of `name` in the module `home` as written, each
    /// module's read once.
    fn signature(&self, home: &str, name: &str) -> Option<String> {
        let cached = self.signatures.borrow().get(home).cloned();
        let all = match cached {
            Some(all) => all,
            None => {
                let mut all = HashMap::new();
                if let Some((root, _)) = self.module_source(home) {
                    for decl in root
                        .children()
                        .filter(|d| d.kind() == SyntaxKind::ValueDecl)
                    {
                        let name = child(&decl, SyntaxKind::Definition)
                            .and_then(|def| crate::lint::first_token(&def, SyntaxKind::LowerIdent));
                        if let (Some(name), Some(signature)) =
                            (name, child(&decl, SyntaxKind::Signature))
                        {
                            all.entry(name.text().to_owned())
                                .or_insert_with(|| signature.text().to_string());
                        }
                    }
                }
                let all = std::rc::Rc::new(all);
                self.signatures
                    .borrow_mut()
                    .insert(home.to_owned(), all.clone());
                all
            }
        };
        all.get(name).cloned()
    }

    pub(crate) fn module_source(&self, name: &str) -> Option<(SyntaxNode, PathBuf)> {
        if let Some(found) = self.sources.borrow().get(name) {
            return found.clone();
        }
        let found = (|| {
            // The project's own, as the index has them parsed already,
            // where no dependency has one of the name.
            let own = self.index.named(name);
            let dependency = (self.importable_modules().iter())
                .skip(self.index.modules.len())
                .find(|(module, _, _)| module == name)
                .and_then(|(_, path, _)| path.clone());
            let path = match (dependency, own) {
                (Some(path), _) => path,
                (None, Some(own)) => return Some((own.root.clone(), own.path.clone())),
                (None, None) => crate::navigation::module_path(self.index, name)?,
            };
            // A package's sources never change: each is parsed once.
            if self
                .index
                .packages
                .values()
                .any(|dir| path.starts_with(dir))
            {
                return Some((package_root(&path)?, path));
            }
            let text = self.snapshot.read(&path).ok()?;
            Some((parse_module(&text).syntax(), path))
        })();
        self.sources
            .borrow_mut()
            .insert(name.to_owned(), found.clone());
        found
    }
}

/// The tree of the package module at `path`, parsed the first time and
/// kept for the life of the process, as a package's sources never change.
fn package_root(path: &Path) -> Option<SyntaxNode> {
    use std::sync::{Mutex, OnceLock};
    static TREES: OnceLock<Mutex<HashMap<PathBuf, nomi_syntax::GreenNode>>> = OnceLock::new();
    let trees = TREES.get_or_init(|| Mutex::new(HashMap::new()));
    let cached = (trees
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner))
    .get(path)
    .cloned();
    let green = match cached {
        Some(green) => green,
        None => {
            let text = std::fs::read_to_string(path).ok()?;
            let green = parse_module(&text).green().clone();
            (trees
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner))
            .insert(path.to_owned(), green.clone());
            green
        }
    };
    Some(SyntaxNode::new_root(green))
}

/// A constructor's type as its custom type's declaration writes it: its
/// arguments, then the type with its parameters.
fn written_constructor_type(root: &SyntaxNode, name: &str) -> Option<String> {
    let variant = root
        .children()
        .filter(|n| n.kind() == SyntaxKind::CustomTypeDecl)
        .flat_map(|d| {
            d.children()
                .filter(|c| c.kind() == SyntaxKind::Variant)
                .collect::<Vec<_>>()
        })
        .find(|v| token(v, SyntaxKind::UpperIdent).as_deref() == Some(name))?;
    let union = variant.parent()?;
    let mut result = token(&union, SyntaxKind::UpperIdent)?;
    for param in union
        .children()
        .filter(|c| c.kind() == SyntaxKind::TypeParam)
    {
        result.push(' ');
        result.push_str(&words(&param));
    }
    let mut parts: Vec<String> = (variant.children())
        .filter(|c| crate::lint::is_type(c.kind()))
        .map(|t| words(&t))
        .collect();
    parts.push(result);
    Some(format!(" : {}", parts.join(" -> ")))
}

/// A constructor's type as label details: its arguments, then its
/// union, which an interface of `module` declares.
fn constructor_type(
    localizer: &crate::hover::Names,
    interface: &nomi_compiler::interface::Interface,
    module: &str,
    name: &str,
) -> Option<String> {
    use nomi_compiler::canonical::{Home, Type};
    use nomi_compiler::interface::Union;
    let (union_name, union) = interface.unions.iter().find_map(|(union_name, union)| {
        let union = match union {
            Union::Open(u) | Union::Closed(u) | Union::Private(u) => u,
        };
        union
            .ctors
            .iter()
            .any(|c| c.name == name)
            .then_some((union_name, union))
    })?;
    let ctor = union.ctors.iter().find(|c| c.name == name)?;
    let home = Home {
        package: interface.home.as_str().into(),
        module: module.into(),
    };
    let result = Type::Type(
        home,
        union_name.clone(),
        union.vars.iter().map(|v| Type::Var(v.clone())).collect(),
    );
    let ty = (ctor.args.iter().rev()).fold(result, |acc, arg| {
        Type::Lambda(Box::new(arg.clone()), Box::new(acc))
    });
    Some(format!(" : {}", crate::hover::render(localizer, &ty)))
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

pub(crate) struct ExposedName {
    /// What it's called: an operator's symbol.
    pub(crate) label: String,
    /// The name its declaration has: an operator's function's, as
    /// `TreeUtils.findOperator` finds it.
    pub(crate) name: SyntaxToken,
    pub(crate) kind: Exposed,
}

/// The names a module's header exposes, as `ISourceFile.exposing` has
/// them (`binder.ts`). An explicit list gives its values, then its types,
/// each in the list's order. `(..)` gives the module's names in the order
/// they first went into its scope: the default imports' names first,
/// then its declarations'.
pub(crate) fn exposed(root: &SyntaxNode) -> Vec<ExposedName> {
    let Some(header) = child(root, SyntaxKind::ModuleHeader) else {
        return Vec::new();
    };
    let Some(list) = child(&header, SyntaxKind::ExposingList) else {
        return Vec::new();
    };
    let declarations: Vec<(SyntaxNode, SyntaxToken)> = (root.children())
        .filter_map(|decl| {
            let token = match decl.kind() {
                SyntaxKind::ValueDecl => child(&decl, SyntaxKind::Definition)
                    .and_then(|d| crate::lint::first_token(&d, SyntaxKind::LowerIdent)),
                SyntaxKind::PortDecl => crate::lint::first_token(&decl, SyntaxKind::LowerIdent),
                SyntaxKind::CustomTypeDecl | SyntaxKind::TypeAliasDecl => {
                    crate::lint::first_token(&decl, SyntaxKind::UpperIdent)
                }
                _ => None,
            }?;
            Some((decl, token))
        })
        .collect();
    let entry = |decl: &SyntaxNode, name: &SyntaxToken, with_variants: bool| -> ExposedName {
        let kind = match decl.kind() {
            SyntaxKind::TypeAliasDecl => Exposed::Alias,
            SyntaxKind::CustomTypeDecl => Exposed::Type {
                variants: if with_variants {
                    (decl.children())
                        .filter(|c| c.kind() == SyntaxKind::Variant)
                        .filter_map(|v| crate::lint::first_token(&v, SyntaxKind::UpperIdent))
                        .collect()
                } else {
                    Vec::new()
                },
            },
            _ => Exposed::Value,
        };
        ExposedName {
            label: name.text().to_owned(),
            name: name.clone(),
            kind,
        }
    };
    if crate::lint::first_token(&list, SyntaxKind::DotDot).is_some() {
        let defaults: Vec<String> = default_imports()
            .into_iter()
            .map(|(name, alias)| alias.unwrap_or(name))
            .collect();
        let mut order: Vec<&(SyntaxNode, SyntaxToken)> = declarations.iter().collect();
        order.sort_by_key(|(_, name)| {
            defaults
                .iter()
                .position(|d| d == name.text())
                .unwrap_or(defaults.len())
        });
        return order
            .into_iter()
            .map(|(decl, name)| entry(decl, name, true))
            .collect();
    }
    let declared = |name: &str| declarations.iter().find(|(_, token)| token.text() == name);
    let items: Vec<SyntaxNode> = list.children().collect();
    let mut found = Vec::new();
    for item in items
        .iter()
        .filter(|i| i.kind() == SyntaxKind::ExposedOperator)
    {
        let written = words(item);
        let op = written
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim()
            .to_owned();
        let infix = (root.children())
            .filter(|n| n.kind() == SyntaxKind::InfixDecl)
            .find(|n| {
                n.children()
                    .any(|c| c.kind() == SyntaxKind::Op && words(&c) == op)
            });
        let function = infix.and_then(|i| {
            (i.children_with_tokens())
                .filter_map(|e| e.into_token())
                .filter(|t| t.kind() == SyntaxKind::LowerIdent)
                .last()
        });
        let name = function.and_then(|f| declared(f.text()).map(|(_, token)| token.clone()));
        if let Some(name) = name {
            found.push(ExposedName {
                label: op,
                name,
                kind: Exposed::Operator,
            });
        }
    }
    for item in items
        .iter()
        .filter(|i| i.kind() == SyntaxKind::ExposedValue)
    {
        if let Some((decl, name)) = declared(&words(item)) {
            found.push(entry(decl, name, false));
        }
    }
    for item in items.iter().filter(|i| i.kind() == SyntaxKind::ExposedType) {
        let with_variants = child(item, SyntaxKind::ExposedVariants).is_some();
        let name = token(item, SyntaxKind::UpperIdent).unwrap_or_default();
        if let Some((decl, token)) = declared(&name) {
            found.push(entry(decl, token, with_variants));
        }
    }
    found
}

/// The modules every module imports, by name and alias.
pub(crate) fn default_imports() -> Vec<(String, Option<String>)> {
    [
        ("Basics", None),
        ("List", None),
        ("Maybe", None),
        ("Result", None),
        ("String", None),
        ("Char", None),
        ("Tuple", None),
        ("Debug", None),
        ("Platform", None),
        ("Platform.Cmd", Some("Cmd")),
        ("Platform.Sub", Some("Sub")),
    ]
    .into_iter()
    .map(|(n, a)| (n.to_owned(), a.map(str::to_owned)))
    .collect()
}

/// The modules the project of `path` may import, each with its file and
/// package: its own, then each direct dependency's exposed modules, in
/// `elm.json` order.
pub(crate) fn importable_modules(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
) -> Vec<(String, Option<PathBuf>, Option<String>)> {
    let mut found: Vec<(String, Option<PathBuf>, Option<String>)> = (index.modules.iter())
        .map(|m| (m.name.clone(), Some(m.path.clone()), None))
        .collect();
    // As elm-language-server finds the files: a directory's files, then
    // its subdirectories', each by name.
    found.sort_by_cached_key(|(_, path, _)| {
        let components: Vec<_> = path.iter().flatten().collect();
        let last = components.len().saturating_sub(1);
        (components.into_iter().enumerate())
            .map(|(i, c)| (i != last, c.to_owned()))
            .collect::<Vec<_>>()
    });
    let Some(manifest) = nomi_engine::index::manifest(snapshot, path) else {
        return found;
    };
    let Ok(bytes) = snapshot.read_bytes(&manifest) else {
        return found;
    };
    let Ok((outline, compat)) = Outline::detect(&bytes) else {
        return found;
    };
    let direct: Vec<String> = match &outline {
        Outline::App(app) => app.direct.keys().map(ToString::to_string).collect(),
        Outline::Pkg(pkg) => {
            // A package's own modules are of its package, which ranks them
            // among the dependencies.
            for module in &mut found {
                module.2 = Some(pkg.name.to_string());
            }
            // elm-language-server offers its test dependencies' modules
            // too, everywhere in it.
            (pkg.deps.keys().chain(pkg.test_deps.keys()))
                .map(ToString::to_string)
                .collect()
        }
    };
    for package in direct {
        let Some(dir) = index.packages.get(&package) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(dir.join("elm.json")) else {
            continue;
        };
        let Ok(Outline::Pkg(pkg)) = Outline::decode(&bytes, compat) else {
            continue;
        };
        // In the order elm-language-server finds their files: a
        // directory's files, then its subdirectories', each by name.
        let exposed: Vec<String> = pkg.exposed.modules().map(ToString::to_string).collect();
        for file in walk(&dir.join("src")) {
            let Ok(relative) = file.strip_prefix(dir.join("src")) else {
                continue;
            };
            let parts: Vec<String> = relative
                .with_extension("")
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            let module = parts.join(".");
            if exposed.contains(&module) {
                found.push((module, Some(file), Some(package.clone())));
            }
        }
    }
    found
}

/// Whether a node is an expression.
pub(crate) fn is_expr(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::LiteralExpr
            | SyntaxKind::NameExpr
            | SyntaxKind::OperatorFunction
            | SyntaxKind::TupleConstructorExpr
            | SyntaxKind::NegateExpr
            | SyntaxKind::CallExpr
            | SyntaxKind::OperatorChain
            | SyntaxKind::UnitExpr
            | SyntaxKind::ParenExpr
            | SyntaxKind::TupleExpr
            | SyntaxKind::ListExpr
            | SyntaxKind::RangeExpr
            | SyntaxKind::RecordExpr
            | SyntaxKind::RecordUpdateExpr
            | SyntaxKind::FieldAccessExpr
            | SyntaxKind::AccessorExpr
            | SyntaxKind::IfExpr
            | SyntaxKind::CaseExpr
            | SyntaxKind::LetExpr
            | SyntaxKind::LambdaExpr
            | SyntaxKind::GlslExpr
    )
}

/// The `.elm` files under `dir`: its own by name, then each
/// subdirectory's.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| Some(e.ok()?.path())).collect();
    entries.sort();
    let (dirs, files): (Vec<PathBuf>, Vec<PathBuf>) = entries.into_iter().partition(|p| p.is_dir());
    let mut found: Vec<PathBuf> = files
        .into_iter()
        .filter(|f| f.extension().is_some_and(|e| e == "elm"))
        .collect();
    for dir in dirs {
        found.extend(walk(&dir));
    }
    found
}

/// A completion of a label, replacing `range` with it.
pub(crate) fn item(
    label: &str,
    kind: CompletionKind,
    sort: &str,
    range: TextRange,
    documentation: Option<String>,
    label_detail: Option<String>,
) -> Completion {
    Completion {
        label: label.to_owned(),
        kind,
        sort_text: Some(format!("{sort}_{label}")),
        filter_text: None,
        documentation: documentation.filter(|d| !d.is_empty()),
        detail: None,
        label_detail: label_detail.filter(|d| !d.contains("unknown")),
        label_description: None,
        preselect: false,
        edit: Some(Edit {
            range,
            text: label.to_owned(),
        }),
        snippet: None,
        additional_edits: Vec::new(),
    }
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
