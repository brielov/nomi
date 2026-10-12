//! The general completion list (plan 9.9), as elm-language-server makes
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

use std::collections::HashMap;

use nomi_base::{TextRange, TextSize};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::Edit;
use crate::completion::{
    Completion, CompletionKind, Completions, Context, Exposed, default_imports, exposed,
    importable_modules, item,
};
use crate::completion_data::{LIST_MARKDOWN, RANKING, SNIPPETS_INLINE, SNIPPETS_START_OF_LINE};
use crate::symbols::{child, code_range, token, words};

/// The general list at `offset`, replacing `range`, `word` typed.
pub(crate) fn general(
    context: &Context<'_>,
    offset: TextSize,
    range: TextRange,
    word: &str,
) -> Completions {
    let mut items = top_levels(context, range);
    items.extend(scope(context, offset, range));
    let imports = Imports::of(context);
    items.extend(imports.completions(context, range, word));
    items.push(Completion {
        documentation: Some(LIST_MARKDOWN.to_owned()),
        ..item("List", CompletionKind::Enum, "d0000", range, None, None)
    });
    items.extend(snippets(SNIPPETS_INLINE));
    items.extend(keywords(&[
        "if", "then", "else", "let", "in", "case", "alias", "exposing",
    ]));
    let (possible, incomplete) = possible_imports(context, &imports, range, word);
    items.extend(possible);
    Completions::Incomplete(items, incomplete)
}

/// The list at a line's start.
pub(crate) fn start_of_line(context: &Context<'_>, range: TextRange) -> Vec<Completion> {
    let root = &context.module.root;
    let exposed_values: Vec<String> = (root.descendants())
        .filter(|n| n.kind() == SyntaxKind::ExposedValue)
        .map(|n| words(&n))
        .collect();
    let top_levels: Vec<String> = (root.children())
        .filter(|n| n.kind() == SyntaxKind::ValueDecl)
        .filter_map(|d| child(&d, SyntaxKind::Definition))
        .filter_map(|d| crate::lint::first_token(&d, SyntaxKind::LowerIdent))
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
        missing
            .iter()
            .map(|name| item(name, CompletionKind::Text, "a", range, None, None)),
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
            completion.label_detail = written_annotation(&context.module.root, &completion.label);
        }
    }
    found
}

/// A top-level value's or port's annotation as written, as label
/// details.
fn written_annotation(root: &SyntaxNode, name: &str) -> Option<String> {
    let decl = (root.children()).find(|d| match d.kind() {
        SyntaxKind::ValueDecl => child(d, SyntaxKind::Definition)
            .and_then(|def| crate::lint::first_token(&def, SyntaxKind::LowerIdent))
            .is_some_and(|t| t.text() == name),
        SyntaxKind::PortDecl => token(d, SyntaxKind::LowerIdent).as_deref() == Some(name),
        _ => false,
    })?;
    let signature = match decl.kind() {
        SyntaxKind::PortDecl => decl.clone(),
        _ => child(&decl, SyntaxKind::Signature)?,
    };
    let ty = signature
        .children()
        .find(|c| crate::lint::is_type(c.kind()))?;
    Some(format!(" : {}", text_of(&ty)?))
}

/// What a scope binds, as the binder calls it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bound {
    Let,
    Parameter,
    Lambda,
    Case,
}

/// `findDefinitionsForScope`: the names each scope around `offset`
/// binds, innermost first, each sorted by its place among them all.
fn scope(context: &Context<'_>, offset: TextSize, range: TextRange) -> Vec<Completion> {
    let root = &context.module.root;
    let Some(token) = root.token_at_offset(offset).left_biased() else {
        return Vec::new();
    };
    let mut symbols: Vec<(String, TextRange, Bound, Option<SyntaxNode>)> = Vec::new();
    for scope in token.parent_ancestors() {
        match scope.kind() {
            SyntaxKind::LetExpr => {
                for decl in scope.children() {
                    match decl.kind() {
                        SyntaxKind::LetValue => {
                            let definition = child(&decl, SyntaxKind::Definition);
                            let name = definition
                                .as_ref()
                                .and_then(|d| crate::lint::first_token(d, SyntaxKind::LowerIdent));
                            if let (Some(name), Some(definition)) = (name, definition) {
                                symbols.push((
                                    name.text().to_owned(),
                                    name.text_range(),
                                    Bound::Let,
                                    Some(definition),
                                ));
                            }
                        }
                        SyntaxKind::LetDestructure => {
                            for pattern in decl
                                .children()
                                .filter(|c| crate::lint::is_pattern(c.kind()))
                            {
                                for (name, at) in crate::lint::bindings(&pattern) {
                                    symbols.push((name, at, Bound::Let, None));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            SyntaxKind::Definition | SyntaxKind::LambdaExpr | SyntaxKind::CaseBranch => {
                let bound = match scope.kind() {
                    SyntaxKind::Definition => Bound::Parameter,
                    SyntaxKind::LambdaExpr => Bound::Lambda,
                    _ => Bound::Case,
                };
                for pattern in scope
                    .children()
                    .filter(|c| crate::lint::is_pattern(c.kind()))
                {
                    for (name, at) in crate::lint::bindings(&pattern) {
                        symbols.push((name, at, bound, None));
                    }
                }
            }
            _ => {}
        }
    }
    if symbols.is_empty() {
        return Vec::new();
    }
    // The module as it would be with something written at the cursor,
    // for the types of the names.
    let typed = typed_module(context, range);
    let names = crate::hover::Names::of(context.index, context.module);
    let type_at = |at: TextRange| -> Option<nomi_compiler::types::error::Type> {
        let (root, types) = typed.as_ref()?;
        (types.iter())
            .rev()
            .find(|(r, _)| crate::hover::trimmed(root, *r) == Some(at))
            .map(|(_, ty)| ty.clone())
    };
    let mut found = Vec::new();
    for (i, (name, at, bound, definition)) in symbols.into_iter().enumerate() {
        let sort = format!("a{i}");
        match bound {
            Bound::Let => {
                let detail = match &definition {
                    Some(definition) => definition_type(definition, &type_at, &names),
                    None => type_at(at).map(|ty| crate::hover::render_error(&names, &ty)),
                }
                .map(|ty| format!(" : {ty}"));
                let documentation = Some("Defined in local let scope".to_owned());
                found.push(item(
                    &name,
                    CompletionKind::Function,
                    &sort,
                    range,
                    documentation,
                    detail,
                ));
            }
            Bound::Case => {
                let detail =
                    type_at(at).map(|ty| format!(" : {}", crate::hover::render_error(&names, &ty)));
                let documentation = Some("Defined in local case branch".to_owned());
                found.push(item(
                    &name,
                    CompletionKind::Variable,
                    &sort,
                    range,
                    documentation,
                    detail,
                ));
            }
            Bound::Parameter | Bound::Lambda => {
                let ty = type_at(at);
                let detail = ty
                    .as_ref()
                    .map(|ty| format!(" : {}", crate::hover::render_error(&names, ty)));
                let documentation = Some("Local parameter".to_owned());
                found.push(item(
                    &name,
                    CompletionKind::Variable,
                    &sort,
                    range,
                    documentation,
                    detail,
                ));
                // A record parameter's fields, written through it.
                let record = match &ty {
                    Some(nomi_compiler::types::error::Type::Alias(_, _, _, body)) => Some(&**body),
                    Some(ty) => Some(ty),
                    None => None,
                };
                if let Some(nomi_compiler::types::error::Type::Record(fields, _)) = record {
                    for (field, field_type) in fields {
                        let rendered = crate::hover::render_error(&names, field_type);
                        let label = format!("{name}.{field}");
                        let mut completion =
                            crate::record_fields::field_completion(field, &rendered, &name, range);
                        completion.label = label.clone();
                        completion.edit = Some(Edit { range, text: label });
                        found.push(completion);
                    }
                }
            }
        }
    }
    found
}

/// The module with `Debug.todo ""` in place of the word at the cursor,
/// typed, where that canonicalizes.
fn typed_module(
    context: &Context<'_>,
    range: TextRange,
) -> Option<(SyntaxNode, nomi_compiler::types::ExprTypes)> {
    if let Some(canonical) = &context.module.canonical {
        let (_, types) = nomi_compiler::types::infer_types(context.index.compat, canonical);
        return Some((context.module.root.clone(), types));
    }
    crate::record_fields::patched(context.index, context.module, range, "Debug.todo \"\"")
}

/// A `let` declaration's type: its parameters', then its body's.
fn definition_type(
    definition: &SyntaxNode,
    type_at: &dyn Fn(TextRange) -> Option<nomi_compiler::types::error::Type>,
    names: &crate::hover::Names,
) -> Option<String> {
    let mut parts = Vec::new();
    for part in definition.children() {
        let at = code_range(&part)?;
        let ty = crate::hover::render_error(names, &type_at(at)?);
        if crate::lint::is_pattern(part.kind()) && ty.contains("->") {
            parts.push(format!("({ty})"));
        } else {
            parts.push(ty);
        }
    }
    (!parts.is_empty()).then(|| parts.join(" -> "))
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
        match self.values.get_mut(key) {
            Some(values) => {
                let same = values
                    .iter()
                    .any(|v| v.module == value.module && v.package == value.package);
                if !same {
                    values.push(value);
                }
            }
            None => {
                self.keys.push(key.to_owned());
                self.values.insert(key.to_owned(), vec![value]);
            }
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

impl Imports {
    fn of(context: &Context<'_>) -> Imports {
        let mut imports = Imports::default();
        let mut written: Vec<(String, Option<String>, Option<SyntaxNode>)> = Vec::new();
        if context.index.package != "elm/core" {
            for (name, alias) in default_imports() {
                written.push((name, alias, None));
            }
        }
        for import in context
            .module
            .root
            .children()
            .filter(|n| n.kind() == SyntaxKind::Import)
        {
            let Some(name) = child(&import, SyntaxKind::ModuleName).map(|n| words(&n)) else {
                continue;
            };
            let alias = child(&import, SyntaxKind::ImportAlias)
                .and_then(|a| token(&a, SyntaxKind::UpperIdent));
            written.push((name, alias, child(&import, SyntaxKind::ExposingList)));
        }
        let packages: HashMap<String, Option<String>> =
            (importable_modules(context.snapshot, context.index, context.path).into_iter())
                .map(|(module, _, package)| (module, package))
                .collect();
        for (i, (name, alias, list)) in written.iter().enumerate() {
            let Some((root, _)) = context.module_source(name) else {
                continue;
            };
            let package = packages.get(name).cloned().flatten();
            imports.modules.push(name.clone());
            let prefix = alias.as_deref().unwrap_or(name);
            let all = exposed(&root);
            let entry = |label: &str, kind, name_token: &SyntaxToken| Imported {
                label: label.to_owned(),
                kind,
                module: name.clone(),
                package: package.clone(),
                root: root.clone(),
                name: name_token.clone(),
            };
            // Qualified, everything the module exposes.
            for exposed in &all {
                let label = format!("{prefix}.{}", exposed.label);
                match &exposed.kind {
                    Exposed::Value => imports
                        .vars
                        .set(&label, entry(&label, ImportedKind::Function, &exposed.name)),
                    Exposed::Operator => {}
                    Exposed::Alias => {
                        imports
                            .types
                            .set(&label, entry(&label, ImportedKind::Alias, &exposed.name));
                        if record_alias(&exposed.name) {
                            imports.constructors.set(
                                &label,
                                entry(&label, ImportedKind::AliasConstructor, &exposed.name),
                            );
                        }
                    }
                    Exposed::Type { variants } => {
                        imports
                            .types
                            .set(&label, entry(&label, ImportedKind::Type, &exposed.name));
                        for variant in variants {
                            let label = format!("{prefix}.{}", variant.text());
                            imports
                                .constructors
                                .set(&label, entry(&label, ImportedKind::Constructor, variant));
                        }
                    }
                }
            }
            // Unqualified, what the import's list exposes.
            let exposing =
                virtual_exposing(i, name, context).or_else(|| list.as_ref().map(exposing_of));
            match exposing {
                None => {}
                Some(Exposing::All) => {
                    for exposed in &all {
                        let label = exposed.label.as_str();
                        match &exposed.kind {
                            Exposed::Value | Exposed::Operator => {
                                let kind = if matches!(exposed.kind, Exposed::Operator) {
                                    ImportedKind::Operator
                                } else {
                                    ImportedKind::Function
                                };
                                imports.vars.set(label, entry(label, kind, &exposed.name));
                            }
                            Exposed::Alias => {
                                imports
                                    .types
                                    .set(label, entry(label, ImportedKind::Alias, &exposed.name));
                                if record_alias(&exposed.name) {
                                    imports.constructors.set(
                                        label,
                                        entry(label, ImportedKind::AliasConstructor, &exposed.name),
                                    );
                                }
                            }
                            Exposed::Type { variants } => {
                                imports
                                    .types
                                    .set(label, entry(label, ImportedKind::Type, &exposed.name));
                                for variant in variants {
                                    imports.constructors.set(
                                        variant.text(),
                                        entry(variant.text(), ImportedKind::Constructor, variant),
                                    );
                                }
                            }
                        }
                    }
                }
                Some(Exposing::Explicit {
                    operators,
                    values,
                    types,
                }) => {
                    let find = |name: &str| all.iter().find(|e| e.label == name);
                    for op in operators.iter().chain(&values) {
                        if let Some(exposed) = find(op) {
                            let kind = if matches!(exposed.kind, Exposed::Operator) {
                                ImportedKind::Operator
                            } else {
                                ImportedKind::Function
                            };
                            imports.vars.set(op, entry(op, kind, &exposed.name));
                        }
                    }
                    for (type_name, open) in &types {
                        let Some(exposed) = find(type_name) else {
                            continue;
                        };
                        match &exposed.kind {
                            Exposed::Type { variants } => {
                                imports.types.replace(
                                    type_name,
                                    entry(type_name, ImportedKind::Type, &exposed.name),
                                );
                                if *open {
                                    for variant in variants {
                                        imports.constructors.set(
                                            variant.text(),
                                            entry(
                                                variant.text(),
                                                ImportedKind::Constructor,
                                                variant,
                                            ),
                                        );
                                    }
                                }
                            }
                            Exposed::Alias => {
                                imports.types.replace(
                                    type_name,
                                    entry(type_name, ImportedKind::Alias, &exposed.name),
                                );
                                if !*open && record_alias(&exposed.name) {
                                    imports.constructors.set(
                                        type_name,
                                        entry(
                                            type_name,
                                            ImportedKind::AliasConstructor,
                                            &exposed.name,
                                        ),
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        imports
    }

    /// `getCompletionsFromOtherFile`: what the imports bring, values,
    /// then types, then constructors.
    fn completions(&self, context: &Context<'_>, range: TextRange, word: &str) -> Vec<Completion> {
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
            let exposes = self.exposing_list(context, &element.module);
            let filter = if !exposes
                && element.module != "Basics"
                && value.to_lowercase().starts_with(&word.to_lowercase())
            {
                value
            } else {
                label
            };
            let mut sort = "d".to_owned();
            if dot.is_some() && self.vars.has(value, &element.module) {
                sort = "e".to_owned();
            }
            if let Some(rank) = element.package.as_deref().and_then(rank) {
                sort = format!("e{rank}");
            }
            let documentation =
                || crate::hover::declaration_hint(&element.root, &element.name, None);
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
            let mut completion = item(label, kind, &sort, range, documentation, detail);
            if kind != CompletionKind::Operator {
                completion.filter_text = Some(filter.to_owned());
            }
            completion.label_description = Some(element.module.clone());
            found.push(completion);
        }
        found
    }

    /// Whether the import of `module` has an exposing list.
    fn exposing_list(&self, context: &Context<'_>, module: &str) -> bool {
        let defaults: Vec<&str> = vec![
            "Basics",
            "List",
            "Maybe",
            "Result",
            "String",
            "Char",
            "Platform",
            "Platform.Cmd",
            "Platform.Sub",
        ];
        let written = (context.module.root.children())
            .filter(|n| n.kind() == SyntaxKind::Import)
            .find(|i| child(i, SyntaxKind::ModuleName).is_some_and(|n| words(&n) == module));
        match written {
            // The last import of a module stands, as a map's set does.
            Some(import) => child(&import, SyntaxKind::ExposingList).is_some(),
            None => defaults.contains(&module),
        }
    }
}

/// How an import exposes names.
enum Exposing {
    All,
    Explicit {
        operators: Vec<String>,
        values: Vec<String>,
        types: Vec<(String, bool)>,
    },
}

/// The exposing list of the `i`th import, where it's one of the default
/// imports elm-language-server writes out.
fn virtual_exposing(i: usize, name: &str, context: &Context<'_>) -> Option<Exposing> {
    if context.index.package == "elm/core" || i >= default_imports().len() {
        return None;
    }
    let explicit = |types: &[(&str, bool)], operators: &[&str]| Exposing::Explicit {
        operators: operators.iter().map(|o| (*o).to_owned()).collect(),
        values: Vec::new(),
        types: types.iter().map(|(t, o)| ((*t).to_owned(), *o)).collect(),
    };
    Some(match name {
        "Basics" => Exposing::All,
        "List" => explicit(&[("List", false)], &["::"]),
        "Maybe" => explicit(&[("Maybe", true)], &[]),
        "Result" => explicit(&[("Result", true)], &[]),
        "String" => explicit(&[("String", false)], &[]),
        "Char" => explicit(&[("Char", false)], &[]),
        "Platform" => explicit(&[("Program", false)], &[]),
        "Platform.Cmd" => explicit(&[("Cmd", false)], &[]),
        "Platform.Sub" => explicit(&[("Sub", false)], &[]),
        _ => return None,
    })
}

/// An import's exposing list, as `Imports` reads it.
fn exposing_of(list: &SyntaxNode) -> Exposing {
    if crate::lint::first_token(list, SyntaxKind::DotDot).is_some() {
        return Exposing::All;
    }
    let mut operators = Vec::new();
    let mut values = Vec::new();
    let mut types = Vec::new();
    for item in list.children() {
        match item.kind() {
            SyntaxKind::ExposedOperator => {
                let written = words(&item);
                operators.push(
                    written
                        .trim_start_matches('(')
                        .trim_end_matches(')')
                        .trim()
                        .to_owned(),
                );
            }
            SyntaxKind::ExposedValue => values.push(words(&item)),
            SyntaxKind::ExposedType => {
                let name = token(&item, SyntaxKind::UpperIdent).unwrap_or_default();
                types.push((name, child(&item, SyntaxKind::ExposedVariants).is_some()));
            }
            _ => {}
        }
    }
    Exposing::Explicit {
        operators,
        values,
        types,
    }
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
    pub package: Option<String>,
    kind: ImportedKind,
    root: SyntaxNode,
    name: SyntaxToken,
}

/// `ImportUtils.getPossibleImports`: what the project's other modules
/// expose, the project's own first, then the packages by rank; the
/// project's by how much of the module's name they share.
pub(crate) fn importable(context: &Context<'_>) -> Vec<Possible> {
    let mut possible = Vec::new();
    for (module, path, package) in importable_modules(context.snapshot, context.index, context.path)
    {
        if module == "Basics" || path.as_deref() == Some(context.path) {
            continue;
        }
        let Some((root, _)) = context.module_source(&module) else {
            continue;
        };
        for exposed in exposed(&root) {
            let kind = match &exposed.kind {
                Exposed::Value => ImportedKind::Function,
                Exposed::Operator => ImportedKind::Operator,
                Exposed::Alias => ImportedKind::Alias,
                Exposed::Type { .. } => ImportedKind::Type,
            };
            possible.push(Possible {
                module: module.clone(),
                value: exposed.label.clone(),
                value_to_import: None,
                package: package.clone(),
                kind,
                root: root.clone(),
                name: exposed.name.clone(),
            });
            if let Exposed::Type { variants } = &exposed.kind {
                for variant in variants {
                    possible.push(Possible {
                        module: module.clone(),
                        value: variant.text().to_owned(),
                        value_to_import: Some(format!("{}(..)", exposed.name.text())),
                        package: package.clone(),
                        kind: ImportedKind::Constructor,
                        root: root.clone(),
                        name: variant.clone(),
                    });
                }
            }
        }
    }
    let current = &context.module.name;
    let score = |module: &str| -> usize {
        current
            .chars()
            .zip(module.chars())
            .take_while(|(a, b)| a == b)
            .count()
    };
    possible.sort_by(|a, b| match (&a.package, &b.package) {
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a), Some(b)) => compare_ranking(a, b),
        (None, None) => score(&b.module).cmp(&score(&a.module)),
    });
    possible
}

/// The auto-imports: what [`importable`] offers that no import brings
/// yet, the closest to `word` first.
fn possible_imports(
    context: &Context<'_>,
    imports: &Imports,
    range: TextRange,
    word: &str,
) -> (Vec<Completion>, bool) {
    let mut possible = importable(context);
    possible.retain(|p| {
        p.module != context.module.name
            && !(imports.vars.has(&p.value, &p.module)
                || imports.types.has(&p.value, &p.module)
                || imports.constructors.has(&p.value, &p.module))
    });
    let filter = word.to_lowercase();
    let imported = |module: &str| imports.modules.iter().any(|m| m == module);
    possible.sort_by(|a, b| {
        let (av, bv) = (a.value.to_lowercase(), b.value.to_lowercase());
        let starts = |v: &str| v.starts_with(&filter);
        let matches = |v: &str| v.contains(&filter);
        starts(&bv)
            .cmp(&starts(&av))
            .then(matches(&bv).cmp(&matches(&av)))
            .then(imported(&b.module).cmp(&imported(&a.module)))
    });
    let incomplete = possible.len() > 50;
    let mut found = Vec::new();
    for (i, p) in possible.iter().take(49).enumerate() {
        let detail = format!("Auto import from module '{}'", p.module);
        let edit = add_import_exposing(
            context,
            &p.module,
            p.value_to_import.as_deref().unwrap_or(&p.value),
        );
        let sort = if i < 10 {
            format!("f0{i}")
        } else {
            format!("f{i}")
        };
        let documentation = || crate::hover::declaration_hint(&p.root, &p.name, None);
        let mut completion = match p.kind {
            ImportedKind::Function => {
                let detail = context.foreign_detail(&p.module, &p.value);
                item(
                    &p.value,
                    CompletionKind::Function,
                    &sort,
                    range,
                    documentation(),
                    detail,
                )
            }
            ImportedKind::Alias => item(
                &p.value,
                CompletionKind::Struct,
                &sort,
                range,
                documentation(),
                None,
            ),
            ImportedKind::Type => item(
                &p.value,
                CompletionKind::Enum,
                &sort,
                range,
                documentation(),
                None,
            ),
            ImportedKind::Constructor => {
                let detail = context.foreign_detail(&p.module, &p.value);
                item(
                    &p.value,
                    CompletionKind::EnumMember,
                    &format!("f{i}"),
                    range,
                    None,
                    detail,
                )
            }
            ImportedKind::Operator | ImportedKind::AliasConstructor => continue,
        };
        completion.detail = Some(detail);
        completion.additional_edits = vec![edit];
        completion.label_description = Some(p.module.clone());
        found.push(completion);
    }
    (found, incomplete)
}

/// `RefactorEditUtils.addImport` with a value: `import M exposing (v)`.
fn add_import_exposing(context: &Context<'_>, module: &str, value: &str) -> Edit {
    let mut edit = context.add_import(module, module);
    edit.text = format!("import {module} exposing ({value})\n");
    edit
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
        label: label.to_owned(),
        kind,
        sort_text: Some(format!("s_{label}")),
        filter_text: None,
        documentation: documentation.map(str::to_owned),
        detail: None,
        label_detail: None,
        label_description: None,
        preselect: false,
        edit: None,
        snippet: Some(text.to_owned()),
        additional_edits: Vec::new(),
    }
}

/// `createKeywordCompletion`s.
fn keywords(words: &[&str]) -> Vec<Completion> {
    (words.iter())
        .map(|word| Completion {
            label: (*word).to_owned(),
            kind: CompletionKind::Keyword,
            sort_text: Some(format!("a_{word}")),
            filter_text: None,
            documentation: None,
            detail: None,
            label_detail: None,
            label_description: None,
            preselect: false,
            edit: None,
            snippet: None,
            additional_edits: Vec::new(),
        })
        .collect()
}

fn text_of(node: &SyntaxNode) -> Option<String> {
    let range = code_range(node)?;
    let start = range.start() - node.text_range().start();
    Some(node.text().slice(start..start + range.len()).to_string())
}
