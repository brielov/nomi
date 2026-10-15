//! The general completion list, as elm-language-server makes
//! it where no context has a list of its own:
//!
//! 1. the module's top-level names, as `b`;
//! 2. the names in scope, innermost first, as `a0`, `a1`…, a record
//!    parameter's fields too;
//! 3. what the imports bring, as its `Imports` map holds them, with its
//!    sort prefixes and package ranking, and `List`;
//! 4. its inline snippets and keywords;
//! 5. up to 49 auto-imports, the list incomplete past 50.
//!
//! At a line's start it offers declarations instead: a snippet for each
//! function used but not defined, its keywords and its snippets.

use std::borrow::Cow;
use std::collections::HashMap;

use nomi_base::{TextRange, TextSize};
use nomi_compiler::types::{ExprTypes, error};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, child_token};

use crate::Edit;
use crate::completion::{
    Completion, CompletionKind, Completions, Context, DEFAULT_IMPORTS, DefaultExposing, Exposed,
    ExposedName, Importable, operator,
};
use crate::completion_data::{LIST_MARKDOWN, RANKING, SNIPPETS_INLINE, SNIPPETS_START_OF_LINE};
use crate::hover::{Names, composed, render_error, type_of};
use crate::tree::{child, code_range, code_text, words, written_imports};

/// The general list at `offset`, replacing `range`, `word` typed of the
/// name `name` the cursor is in.
pub(crate) fn general(
    context: &Context<'_>,
    offset: TextSize,
    range: TextRange,
    word: &str,
    name: &str,
) -> Completions {
    let mut items = top_levels(context, range);
    items.extend(scope(context, offset, range));
    let imports = Imports::of(context);
    items.extend(imports.completions(context, range, word));
    items.push(
        Completion::new("List", CompletionKind::Enum, "d0000", range)
            .documented(Some(LIST_MARKDOWN.to_owned())),
    );
    items.extend(snippets(SNIPPETS_INLINE));
    items.extend(keywords(&[
        "if", "then", "else", "let", "in", "case", "alias", "exposing",
    ]));
    let (possible, incomplete) = auto_imports(context, &imports, range, name);
    items.extend(possible);
    Completions::List { items, incomplete }
}

/// The list at a line's start.
pub(crate) fn start_of_line(context: &Context<'_>, range: TextRange) -> Vec<Completion> {
    let root = &context.module.root();
    let exposed_values: Vec<String> = (root.descendants())
        .filter(|n| n.kind() == SyntaxKind::ExposedValue)
        .map(|n| words(&n))
        .collect();
    // elm-language-server compares a call's name with a declaration's
    // name and parameters as written, so only a value without them
    // declares it.
    let top_levels: Vec<String> = (root.children())
        .filter(|n| n.kind() == SyntaxKind::ValueDecl)
        .filter_map(|d| child(&d, SyntaxKind::Definition))
        .filter(|d| !d.children().any(|c| ast::Pattern::can_cast(c.kind())))
        .filter_map(|d| child_token(&d, SyntaxKind::LowerIdent))
        .map(|t| t.text().to_owned())
        .collect();
    // Each call of an unqualified function nothing declares or exposes.
    let missing: Vec<String> = (root.descendants())
        .filter(|n| n.kind() == SyntaxKind::CallExpr)
        .filter_map(|call| call.children().next())
        .map(|function| words(&function))
        .filter(|name| !name.contains('.'))
        .filter(|name| !exposed_values.contains(name) && !top_levels.contains(name))
        .collect();
    let mut found: Vec<Completion> = (missing.iter())
        .map(|name| {
            let text = format!(
                "{name} : ${{1:ArgumentType}} -> ${{2:ReturnType}}\n{name} ${{3:arguments}} =\n    ${{4}}"
            );
            snippet(&format!("func {name}"), &text, Some("Function with type annotation"), false)
        })
        .collect();
    found.extend(
        (missing.iter()).map(|name| Completion::new(name, CompletionKind::Text, "a", range)),
    );
    found.extend(keywords(&["type", "import", "module", "port"]));
    found.extend(snippets(SNIPPETS_START_OF_LINE));
    found
}

/// `getSameFileTopLevelCompletions`, with a value's written annotation
/// where inference has no type for it.
fn top_levels(context: &Context<'_>, range: TextRange) -> Vec<Completion> {
    let mut found = context.top_levels(range, false);
    for completion in &mut found {
        if completion.kind == CompletionKind::Function && completion.label_detail.is_none() {
            completion.label_detail = written_annotation(&context.module.root(), &completion.label);
        }
    }
    found
}

/// A top-level value's or port's annotation as written, as label
/// details.
fn written_annotation(root: &SyntaxNode, name: &str) -> Option<String> {
    let named = |token: Option<SyntaxToken>| token.is_some_and(|t| t.text() == name);
    let decl = (root.children()).find(|d| match d.kind() {
        SyntaxKind::ValueDecl => named(
            child(d, SyntaxKind::Definition).and_then(|d| child_token(&d, SyntaxKind::LowerIdent)),
        ),
        SyntaxKind::PortDecl => named(child_token(d, SyntaxKind::LowerIdent)),
        _ => false,
    })?;
    let signature = match decl.kind() {
        SyntaxKind::PortDecl => decl,
        _ => child(&decl, SyntaxKind::Signature)?,
    };
    let ty = signature
        .children()
        .find(|c| ast::Type::can_cast(c.kind()))?;
    Some(format!(" : {}", code_text(&ty)?))
}

/// What a scope binds, as the binder calls it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bound {
    Let,
    /// A declaration's or a lambda's parameter.
    Parameter,
    Case,
}

/// A name a scope around the cursor binds.
struct Binding {
    name: String,
    at: TextRange,
    bound: Bound,
    /// A `let` value's definition.
    definition: Option<SyntaxNode>,
}

/// `findDefinitionsForScope`: the names each scope around `offset`
/// binds, innermost first, each sorted by its place among them all.
fn scope(context: &Context<'_>, offset: TextSize, range: TextRange) -> Vec<Completion> {
    let root = &context.module.root();
    let Some(token) = root.token_at_offset(offset).left_biased() else {
        return Vec::new();
    };
    let bindings = bindings_around(&token);
    if bindings.is_empty() {
        return Vec::new();
    }
    // The module as it would be with something written at the cursor,
    // for the types of the names.
    let typed = typed_module(context, range);
    let names = context.names();
    let type_at = |at: TextRange| -> Option<&error::Type> {
        let (root, types) = typed.as_ref()?;
        let find = |at: TextRange| type_of(root, types, at);
        // An alias's type is its whole pattern's.
        find(at).or_else(|| {
            let alias = (root.token_at_offset(at.start()).right_biased())
                .and_then(|t| t.parent())
                .filter(|p| p.kind() == SyntaxKind::AliasPattern)?;
            find(code_range(&alias)?)
        })
    };
    let bare = Names::bare();
    let mut found = Vec::new();
    for (i, binding) in bindings.into_iter().enumerate() {
        let Binding {
            name,
            at,
            bound,
            definition,
        } = binding;
        let ty = definition.is_none().then(|| type_at(at)).flatten();
        let detail = match &definition {
            Some(definition) => {
                let typed = |at| type_at(at).map(|ty| render_error(names, ty));
                composed(definition, typed).filter(|ty| !ty.is_empty())
            }
            None => ty.map(|ty| render_error(names, ty)),
        };
        let (kind, documentation) = match bound {
            Bound::Let => (CompletionKind::Function, "Defined in local let scope"),
            Bound::Case => (CompletionKind::Variable, "Defined in local case branch"),
            Bound::Parameter => (CompletionKind::Variable, "Local parameter"),
        };
        found.push(
            Completion::new(&name, kind, &format!("a{i}"), range)
                .documented(Some(documentation.to_owned()))
                .typed(detail.map(|ty| format!(" : {ty}"))),
        );
        // A record parameter's fields, written through it.
        let record = match ty {
            Some(error::Type::Alias { real: body, .. }) => Some(&**body),
            ty => ty,
        };
        if bound == Bound::Parameter
            && let Some(error::Type::Record(fields, _)) = record
        {
            for (field, field_type) in fields {
                let rendered = render_error(&bare, field_type);
                let label = format!("{name}.{field}");
                found.push(Completion {
                    label: label.clone(),
                    edit: Some(Edit { range, text: label }),
                    ..crate::record_fields::field_completion(field, &rendered, &name, range)
                });
            }
        }
    }
    found
}

/// The names each scope around `token` binds, innermost first: a
/// `let`'s, a declaration's or a lambda's parameters, a case branch's.
fn bindings_around(token: &SyntaxToken) -> Vec<Binding> {
    let mut found = Vec::new();
    for scope in token.parent_ancestors() {
        match scope.kind() {
            SyntaxKind::LetExpr => {
                for decl in scope.children() {
                    match decl.kind() {
                        SyntaxKind::LetValue => {
                            let definition = child(&decl, SyntaxKind::Definition);
                            let name = (definition.as_ref())
                                .and_then(|d| child_token(d, SyntaxKind::LowerIdent));
                            if let (Some(name), Some(definition)) = (name, definition) {
                                found.push(Binding {
                                    name: name.text().to_owned(),
                                    at: name.text_range(),
                                    bound: Bound::Let,
                                    definition: Some(definition),
                                });
                            }
                        }
                        SyntaxKind::LetDestructure => {
                            found.extend(pattern_bindings(&decl, Bound::Let));
                        }
                        _ => {}
                    }
                }
            }
            SyntaxKind::Definition | SyntaxKind::LambdaExpr => {
                found.extend(pattern_bindings(&scope, Bound::Parameter));
            }
            SyntaxKind::CaseBranch => found.extend(pattern_bindings(&scope, Bound::Case)),
            _ => {}
        }
    }
    found
}

/// The names the patterns among `node`'s children bind, as `bound`.
fn pattern_bindings(node: &SyntaxNode, bound: Bound) -> impl Iterator<Item = Binding> + use<> {
    (node.children())
        .filter(|c| ast::Pattern::can_cast(c.kind()))
        .flat_map(move |pattern| {
            (crate::scopes::bindings(&pattern).into_iter()).map(move |(name, at)| Binding {
                name,
                at,
                bound,
                definition: None,
            })
        })
}

/// The module with `Debug.todo ""` in place of the word at the cursor,
/// typed, where that canonicalizes.
fn typed_module<'c>(
    context: &'c Context<'_>,
    range: TextRange,
) -> Option<(SyntaxNode, Cow<'c, ExprTypes>)> {
    if let Some((_, types)) = context.module.types(context.index.compat) {
        return Some((context.module.root(), Cow::Borrowed(types)));
    }
    crate::record_fields::patched(context.index, context.module, range, "Debug.todo \"\"")
        .map(|(root, types)| (root, Cow::Owned(types)))
}

/// What an import brings, as elm-language-server's `Imports` holds it.
#[derive(Clone)]
struct Imported {
    label: String,
    kind: ImportedKind,
    module: String,
    package: Option<String>,
    /// The declaring module's tree, and the name the symbol's node has.
    root: SyntaxNode,
    name: SyntaxToken,
    /// Whether the module's import has an exposing list.
    listed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportedKind {
    Function,
    Operator,
    Type,
    Alias,
    Constructor,
    /// A record alias as its constructor, which the list leaves out.
    AliasConstructor,
}

impl ImportedKind {
    /// The kind of an exposed value or operator.
    fn of_value(exposed: &ExposedName) -> ImportedKind {
        match exposed.kind {
            Exposed::Operator => ImportedKind::Operator,
            Exposed::Value | Exposed::Alias | Exposed::Type { .. } => ImportedKind::Function,
        }
    }
}

/// A `MultiMap`: keys in the order they were first set, each with its
/// values.
#[derive(Default)]
struct Multi {
    keys: Vec<String>,
    values: HashMap<String, Vec<Imported>>,
}

impl Multi {
    /// `set` with `importModuleEqual`: one per module and package.
    fn set(&mut self, key: &str, value: Imported) {
        if let Some(values) = self.values.get_mut(key) {
            let same = values
                .iter()
                .any(|v| v.module == value.module && v.package == value.package);
            if !same {
                values.push(value);
            }
        } else {
            self.keys.push(key.to_owned());
            self.values.insert(key.to_owned(), vec![value]);
        }
    }

    fn replace(&mut self, key: &str, value: Imported) {
        if !self.values.contains_key(key) {
            self.keys.push(key.to_owned());
        }
        self.values.insert(key.to_owned(), vec![value]);
    }

    fn has(&self, key: &str, module: &str) -> bool {
        self.values
            .get(key)
            .is_some_and(|v| v.iter().any(|i| i.module == module))
    }

    fn iter(&self) -> impl Iterator<Item = &Imported> {
        self.keys.iter().flat_map(|k| self.values[k].iter())
    }
}

/// `Imports`: what the module's imports, the default ones first, bring
/// into scope.
#[derive(Default)]
struct Imports {
    vars: Multi,
    types: Multi,
    constructors: Multi,
    /// The modules imported, by name.
    modules: Vec<String>,
}

/// An import as `Imports` reads it.
struct Import {
    module: String,
    alias: Option<String>,
    exposing: Option<Exposing>,
}

impl Imports {
    fn of(context: &Context<'_>) -> Imports {
        let written: Vec<Import> = (written_imports(&context.module.root()))
            .map(|import| Import {
                exposing: (import.node.exposing_list()).map(|list| exposing_of(list.syntax())),
                module: import.module,
                alias: import.alias,
            })
            .collect();
        // Whether a module's import has an exposing list: the first
        // written import of it, or else its default one.
        let listed = |module: &str| match written.iter().find(|i| i.module == module) {
            Some(import) => import.exposing.is_some(),
            None => (DEFAULT_IMPORTS.iter()).any(|d| d.module == module && d.exposing.is_some()),
        };
        let defaults: Vec<Import> = (DEFAULT_IMPORTS.iter())
            .filter(|_| context.index.package != "elm/core")
            .map(|d| Import {
                module: d.module.to_owned(),
                alias: d.alias.map(str::to_owned),
                exposing: d.exposing.as_ref().map(Exposing::of_default),
            })
            .collect();
        let packages: HashMap<&str, &Option<String>> = (context.importable_modules().iter())
            .map(|m| (m.name.as_str(), &m.package))
            .collect();
        let mut imports = Imports::default();
        for import in defaults.iter().chain(&written) {
            let name = import.module.as_str();
            let (Some((root, _)), Some(all)) =
                (context.module_source(name), context.exposed_names(name))
            else {
                continue;
            };
            let package = packages.get(name).copied().cloned().flatten();
            let listed = listed(name);
            imports.modules.push(name.to_owned());
            let entry = |label: &str, kind, name_token: &SyntaxToken| Imported {
                label: label.to_owned(),
                kind,
                module: name.to_owned(),
                package: package.clone(),
                root: root.clone(),
                name: name_token.clone(),
                listed,
            };
            // Qualified, everything the module exposes.
            let prefix = import.alias.as_deref().unwrap_or(name);
            for exposed in all.iter() {
                imports.add_exposed(&entry, exposed, Some(prefix));
            }
            // Unqualified, what the import's list exposes.
            match &import.exposing {
                None => {}
                Some(Exposing::All) => {
                    for exposed in all.iter() {
                        imports.add_exposed(&entry, exposed, None);
                    }
                }
                Some(Exposing::Explicit(listed)) => imports.add_listed(&entry, &all, listed),
            }
        }
        imports
    }

    /// `exposed`, as an import of its whole module brings it: qualified
    /// with `prefix` where given, but for an operator, or else bare; an
    /// alias of a record as its constructor too, a type with its variants.
    fn add_exposed(
        &mut self,
        entry: &impl Fn(&str, ImportedKind, &SyntaxToken) -> Imported,
        exposed: &ExposedName,
        prefix: Option<&str>,
    ) {
        fn qualified<'l>(prefix: Option<&str>, label: &'l str) -> Cow<'l, str> {
            match prefix {
                Some(prefix) => Cow::Owned(format!("{prefix}.{label}")),
                None => Cow::Borrowed(label),
            }
        }
        let label = qualified(prefix, &exposed.label);
        match &exposed.kind {
            Exposed::Operator if prefix.is_some() => {}
            Exposed::Value | Exposed::Operator => {
                let kind = ImportedKind::of_value(exposed);
                self.vars.set(&label, entry(&label, kind, &exposed.name));
            }
            Exposed::Alias => {
                (self.types).set(&label, entry(&label, ImportedKind::Alias, &exposed.name));
                if record_alias(&exposed.name) {
                    let kind = ImportedKind::AliasConstructor;
                    (self.constructors).set(&label, entry(&label, kind, &exposed.name));
                }
            }
            Exposed::Type { variants } => {
                (self.types).set(&label, entry(&label, ImportedKind::Type, &exposed.name));
                for variant in variants {
                    let label = qualified(prefix, variant.text());
                    let constructor = entry(&label, ImportedKind::Constructor, variant);
                    self.constructors.set(&label, constructor);
                }
            }
        }
    }

    /// What an explicit exposing list brings of `all` a module exposes:
    /// its operators and values, then its types, each replacing what came
    /// before, with their variants where `(..)` says.
    fn add_listed(
        &mut self,
        entry: &impl Fn(&str, ImportedKind, &SyntaxToken) -> Imported,
        all: &[ExposedName],
        listed: &Listed,
    ) {
        let find = |name: &str| all.iter().find(|e| e.label == name);
        for op in listed.operators.iter().chain(&listed.values) {
            if let Some(exposed) = find(op) {
                let kind = ImportedKind::of_value(exposed);
                self.vars.set(op, entry(op, kind, &exposed.name));
            }
        }
        for (label, open) in &listed.types {
            let Some(exposed) = find(label) else {
                continue;
            };
            match &exposed.kind {
                Exposed::Type { variants } => {
                    (self.types).replace(label, entry(label, ImportedKind::Type, &exposed.name));
                    for variant in variants.iter().filter(|_| *open) {
                        let constructor = entry(variant.text(), ImportedKind::Constructor, variant);
                        self.constructors.set(variant.text(), constructor);
                    }
                }
                Exposed::Alias => {
                    (self.types).replace(label, entry(label, ImportedKind::Alias, &exposed.name));
                    if !*open && record_alias(&exposed.name) {
                        let kind = ImportedKind::AliasConstructor;
                        (self.constructors).set(label, entry(label, kind, &exposed.name));
                    }
                }
                Exposed::Value | Exposed::Operator => {}
            }
        }
    }

    /// `getCompletionsFromOtherFile`: what the imports bring, values,
    /// then types, then constructors.
    fn completions(&self, context: &Context<'_>, range: TextRange, word: &str) -> Vec<Completion> {
        let word = word.to_lowercase();
        let mut found = Vec::new();
        let elements = self.vars.iter().chain(self.types.iter()).chain(
            self.constructors
                .iter()
                .filter(|c| c.kind != ImportedKind::AliasConstructor),
        );
        for element in elements {
            let label = element.label.as_str();
            let dot = label.rfind('.');
            let value = dot.map_or(label, |d| &label[d + 1..]);
            let filter = if !element.listed
                && element.module != "Basics"
                && value.to_lowercase().starts_with(&word)
            {
                value
            } else {
                label
            };
            let sort = match element.package.as_deref().and_then(rank) {
                Some(rank) => format!("e{rank}"),
                None if dot.is_some() && self.vars.has(value, &element.module) => "e".to_owned(),
                None => "d".to_owned(),
            };
            let documentation = || context.hint(&element.module, &element.root, &element.name);
            let (kind, documentation, detail) = match element.kind {
                ImportedKind::Function => (
                    CompletionKind::Function,
                    documentation(),
                    context.foreign_detail(&element.module, value),
                ),
                ImportedKind::Constructor => (
                    CompletionKind::EnumMember,
                    None,
                    context.foreign_detail(&element.module, value),
                ),
                ImportedKind::Operator => (CompletionKind::Operator, documentation(), None),
                ImportedKind::Type => (CompletionKind::Enum, documentation(), None),
                ImportedKind::Alias => (CompletionKind::Struct, documentation(), None),
                ImportedKind::AliasConstructor => continue,
            };
            let mut completion = Completion::new(label, kind, &sort, range)
                .documented(documentation)
                .typed(detail);
            if kind != CompletionKind::Operator {
                completion.filter_text = Some(filter.to_owned());
            }
            completion.label_description = Some(element.module.clone());
            found.push(completion);
        }
        found
    }
}

/// How an import exposes names.
enum Exposing {
    All,
    Explicit(Listed),
}

/// The names an explicit exposing list has: its types each with whether
/// it has its variants.
struct Listed {
    operators: Vec<String>,
    values: Vec<String>,
    types: Vec<(String, bool)>,
}

impl Exposing {
    /// A default import's list, as elm-language-server writes it out.
    fn of_default(exposing: &DefaultExposing) -> Exposing {
        match exposing {
            DefaultExposing::All => Exposing::All,
            DefaultExposing::Explicit { types, operators } => Exposing::Explicit(Listed {
                operators: operators.iter().map(|o| (*o).to_owned()).collect(),
                values: Vec::new(),
                types: types.iter().map(|(t, o)| ((*t).to_owned(), *o)).collect(),
            }),
        }
    }
}

/// An import's exposing list, as `Imports` reads it.
fn exposing_of(list: &SyntaxNode) -> Exposing {
    if child_token(list, SyntaxKind::DotDot).is_some() {
        return Exposing::All;
    }
    let mut operators = Vec::new();
    let mut values = Vec::new();
    let mut types = Vec::new();
    for item in list.children() {
        match item.kind() {
            SyntaxKind::ExposedOperator => operators.push(operator(&item)),
            SyntaxKind::ExposedValue => values.push(words(&item)),
            SyntaxKind::ExposedType => {
                let name = child_token(&item, SyntaxKind::UpperIdent);
                let name = name.map(|t| t.text().to_owned()).unwrap_or_default();
                types.push((name, child(&item, SyntaxKind::ExposedVariants).is_some()));
            }
            _ => {}
        }
    }
    Exposing::Explicit(Listed {
        operators,
        values,
        types,
    })
}

/// Whether the alias whose name is `name` is a record's, so also its
/// constructor.
fn record_alias(name: &SyntaxToken) -> bool {
    name.parent()
        .is_some_and(|decl| decl.children().any(|c| c.kind() == SyntaxKind::RecordType))
}

/// A package's rank, where it has one.
fn rank(package: &str) -> Option<&'static str> {
    RANKING
        .binary_search_by(|(name, _)| (*name).cmp(package))
        .ok()
        .map(|i| RANKING[i].1)
}

/// `comparePackageRanking`, a ranked package after an unranked one.
pub(crate) fn compare_ranking(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (rank(a), rank(b)) {
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

/// An auto-import's candidate, as `IPossibleImport` has it.
pub(crate) struct Possible {
    pub module: String,
    pub value: String,
    pub value_to_import: Option<String>,
    kind: ImportedKind,
    /// The name its declaration has in its module.
    name: SyntaxToken,
}

/// `ImportUtils.getPossibleImports`: what the project's other modules
/// expose, the project's own first, then the packages by rank; the
/// project's by how much of the module's name they share.
pub(crate) fn possible_imports(context: &Context<'_>) -> Vec<Possible> {
    let current = &context.module.name;
    let score = |module: &str| -> usize {
        current
            .chars()
            .zip(module.chars())
            .take_while(|(a, b)| a == b)
            .count()
    };
    // A module's names all share its place: the modules are sorted,
    // stably, then their names listed.
    let mut modules: Vec<&Importable> = (context.importable_modules().iter())
        .filter(|m| m.name != "Basics" && m.path != context.path)
        .collect();
    modules.sort_by(|a, b| match (&a.package, &b.package) {
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a), Some(b)) => compare_ranking(a, b),
        (None, None) => score(&b.name).cmp(&score(&a.name)),
    });
    let mut possible = Vec::new();
    for module in modules {
        let Some(exposed) = context.exposed_names(&module.name) else {
            continue;
        };
        let candidate = |value: &str, kind, name: &SyntaxToken| Possible {
            module: module.name.clone(),
            value: value.to_owned(),
            value_to_import: None,
            kind,
            name: name.clone(),
        };
        for exposed in exposed.iter() {
            let kind = match &exposed.kind {
                Exposed::Value => ImportedKind::Function,
                Exposed::Operator => ImportedKind::Operator,
                Exposed::Alias => ImportedKind::Alias,
                Exposed::Type { .. } => ImportedKind::Type,
            };
            possible.push(candidate(&exposed.label, kind, &exposed.name));
            if let Exposed::Type { variants } = &exposed.kind {
                for variant in variants {
                    possible.push(Possible {
                        value_to_import: Some(format!("{}(..)", exposed.name.text())),
                        ..candidate(variant.text(), ImportedKind::Constructor, variant)
                    });
                }
            }
        }
    }
    possible
}

/// The auto-imports: what [`possible_imports`] offers that no import
/// brings yet, the closest to `word` first.
fn auto_imports(
    context: &Context<'_>,
    imports: &Imports,
    range: TextRange,
    word: &str,
) -> (Vec<Completion>, bool) {
    let mut possible: Vec<&Possible> = (context.possible_imports().iter())
        .filter(|p| {
            p.module != context.module.name
                && !(imports.vars.has(&p.value, &p.module)
                    || imports.types.has(&p.value, &p.module)
                    || imports.constructors.has(&p.value, &p.module))
        })
        .collect();
    let filter = word.to_lowercase();
    let imported = |module: &str| imports.modules.iter().any(|m| m == module);
    // Those starting with it first, then those with it in them, then those
    // of imported modules; a stable sort keeps the rest in order.
    possible.sort_by_cached_key(|p| {
        let value = p.value.to_lowercase();
        (
            !value.starts_with(&filter),
            !value.contains(&filter),
            !imported(&p.module),
        )
    });
    let incomplete = possible.len() > 50;
    let mut found = Vec::new();
    for (i, p) in possible.iter().take(49).enumerate() {
        let documentation = || {
            let (root, _) = context.module_source(&p.module)?;
            context.hint(&p.module, &root, &p.name)
        };
        let detail = || context.foreign_detail(&p.module, &p.value);
        let (kind, documentation, detail) = match p.kind {
            ImportedKind::Function => (CompletionKind::Function, documentation(), detail()),
            ImportedKind::Alias => (CompletionKind::Struct, documentation(), None),
            ImportedKind::Type => (CompletionKind::Enum, documentation(), None),
            ImportedKind::Constructor => (CompletionKind::EnumMember, None, detail()),
            ImportedKind::Operator | ImportedKind::AliasConstructor => continue,
        };
        // A constructor's index isn't padded.
        let sort = if i < 10 && kind != CompletionKind::EnumMember {
            format!("f0{i}")
        } else {
            format!("f{i}")
        };
        let value = p.value_to_import.as_deref().unwrap_or(&p.value);
        found.push(Completion {
            detail: Some(format!("Auto import from module '{}'", p.module)),
            additional_edits: vec![context.add_import(&p.module, None, Some(value))],
            label_description: Some(p.module.clone()),
            ..Completion::new(&p.value, kind, &sort, range)
                .documented(documentation)
                .typed(detail)
        });
    }
    (found, incomplete)
}

/// Snippets from a table.
fn snippets(table: &[(&str, &str, Option<&str>, bool)]) -> Vec<Completion> {
    (table.iter())
        .map(|(label, text, documentation, keyword)| snippet(label, text, *documentation, *keyword))
        .collect()
}

/// `createSnippet`.
fn snippet(label: &str, text: &str, documentation: Option<&str>, keyword: bool) -> Completion {
    let kind = if keyword {
        CompletionKind::Keyword
    } else {
        CompletionKind::Snippet
    };
    Completion {
        sort_text: Some(format!("s_{label}")),
        documentation: documentation.map(str::to_owned),
        snippet: Some(text.to_owned()),
        ..Completion::of(label, kind)
    }
}

/// `createKeywordCompletion`s.
fn keywords(words: &[&str]) -> Vec<Completion> {
    (words.iter())
        .map(|word| Completion {
            sort_text: Some(format!("a_{word}")),
            ..Completion::of(word, CompletionKind::Keyword)
        })
        .collect()
}
