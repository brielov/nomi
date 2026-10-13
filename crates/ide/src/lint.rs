//! elm-language-server's own lint rules (plan 9.4,
//! `elmLsDiagnostics.ts`), over nomi's syntax tree. They are
//! tree-sitter queries there that compare the text of nodes, so they are
//! ported as such: a value is used where some expression writes its name
//! as written, unqualified, shadowing and all, not where it resolves.
//! Each tree-sitter node a rule reads has its counterpart here: a
//! `value_expr` is a [`SyntaxKind::NameExpr`], a `type_ref` a
//! [`SyntaxKind::NamedType`], a `value_declaration` a value or `let`
//! declaration, and a node's range leaves out its doc comment and
//! trivia, as tree-sitter's do.

use std::collections::HashSet;
use std::path::Path;

use nomi_base::{TextRange, TextSize};
use nomi_engine::Snapshot;
use nomi_engine::index::Index;
use nomi_syntax::{SyntaxKind, SyntaxNode, parse_module};

use crate::problems::{Problem, Severity};
use crate::symbols::{child, code_range, token, words};

/// Which rules run: elm-analyse.json's `checks`, each on unless set to
/// false, `SingleFieldRecord` off unless set to true.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checks {
    set: Vec<(String, bool)>,
}

impl Checks {
    pub fn new(set: impl IntoIterator<Item = (String, bool)>) -> Checks {
        Checks {
            set: set.into_iter().collect(),
        }
    }

    fn get(&self, name: &str) -> Option<bool> {
        self.set.iter().find(|(n, _)| n == name).map(|(_, on)| *on)
    }

    pub fn on(&self, name: &str) -> bool {
        self.get(name) != Some(false)
    }

    pub fn opted_in(&self, name: &str) -> bool {
        self.get(name) == Some(true)
    }
}

/// The lint problems of the file at `path`, in the order
/// elm-language-server finds them: rule by rule. `index` is its
/// project's, for the rules that look across modules.
pub fn lint(
    snapshot: &Snapshot,
    index: Option<&Index>,
    path: &Path,
    checks: &Checks,
) -> Vec<Problem> {
    let Ok(text) = snapshot.read(path) else {
        return Vec::new();
    };
    let parse = parse_module(&text);
    let root = parse.syntax();
    let facts = Facts::of(&root);
    let errors: Vec<TextSize> = parse.errors().iter().map(|e| e.range.start()).collect();
    let imports: Vec<SyntaxNode> = imports(&root, &errors).collect();
    let mut found = Lint {
        path,
        problems: Vec::new(),
    };
    let unused_exposures = if checks.on("UnusedImport") || checks.on("UnusedImportedVariable") {
        found.unused_imported_values(&imports, &facts)
    } else {
        HashSet::new()
    };
    if checks.on("UnusedImport") {
        found.unused_imports(&imports, &facts, &unused_exposures);
    }
    if checks.on("UnusedImportedVariable") {
        found
            .problems
            .extend(found_values(&imports, &unused_exposures, path));
    }
    if checks.on("UnusedImportAlias") {
        found.unused_aliases(&imports, &facts);
    }
    if checks.on("UnusedPatternVariable") {
        found.unused_patterns(&root, &facts);
    }
    if checks.on("MapNothingToNothing") {
        found.map_nothing_to_nothing(&root);
    }
    if checks.on("BooleanCase") {
        found.boolean_cases(&root);
    }
    if checks.on("DropConcatOfLists") {
        found.chains(&root, "++", Pair::ListThenList);
    }
    if checks.on("DropConsOfItemAndList") {
        found.chains(&root, "::", Pair::AnyThenList);
    }
    if checks.on("UseConsOverConcat") {
        found.chains(&root, "++", Pair::SingletonThenAny);
    }
    if checks.opted_in("SingleFieldRecord") {
        found.single_field_records(&root);
    }
    if checks.on("UnnecessaryListConcat") {
        found.unnecessary_list_concats(&root);
    }
    if checks.on("UnnecessaryPortModule") {
        found.unnecessary_port_module(&root);
    }
    if checks.on("UnusedIncomingPort")
        && let Some(index) = index
    {
        found.unused_incoming_ports(snapshot, index, &root);
    }
    if checks.on("NoUncurriedPrefix") {
        found.uncurried_prefixes(&root);
    }
    if checks.on("UnusedTypeAlias") {
        found.unused_type_aliases(&root, &facts);
    }
    if checks.on("UnusedValueConstructor") {
        found.unused_constructors(&root, &facts);
    }
    found.problems
}

/// What the rules compare: the text of every name an expression, a type
/// or an exposing list writes.
#[derive(Default)]
struct Facts {
    /// Each `value_expr`'s text, qualified as written, and its range.
    values: Vec<(String, TextRange)>,
    /// Each `record_base_identifier`: the record of an update.
    record_bases: Vec<(String, TextRange)>,
    /// Each `type_ref`'s name, qualified as written.
    types: Vec<String>,
    /// Each qualifier outside the imports, without its last dot.
    qualifiers: Vec<String>,
    /// Each exposed value's text in the module's own exposing list.
    exposed_values: Vec<(String, TextRange)>,
    /// Each exposed type's text, in any exposing list.
    exposed_types: Vec<String>,
    /// The constructors case branches match on directly.
    branch_constructors: Vec<String>,
    /// Whether the module exposes everything.
    exposes_all: bool,
}

impl Facts {
    fn of(root: &SyntaxNode) -> Facts {
        let mut facts = Facts::default();
        for node in root.descendants() {
            match node.kind() {
                SyntaxKind::NameExpr => {
                    if let Some(range) = code_range(&node) {
                        facts.values.push((words(&node), range));
                    }
                    facts.qualifiers.extend(qualifier(&node));
                }
                // `infix left 0 (|>) = apR` writes `apR` as an
                // expression.
                SyntaxKind::InfixDecl => {
                    if let Some(name) = first_token(&node, SyntaxKind::LowerIdent) {
                        facts
                            .values
                            .push((name.text().to_owned(), name.text_range()));
                    }
                }
                SyntaxKind::RecordUpdateExpr => {
                    if let Some(base) = first_token(&node, SyntaxKind::LowerIdent) {
                        facts
                            .record_bases
                            .push((base.text().to_owned(), base.text_range()));
                    }
                }
                SyntaxKind::NamedType => {
                    facts.types.push(qualified_name(&node));
                    facts.qualifiers.extend(qualifier(&node));
                }
                SyntaxKind::ConstructorPattern => {
                    facts.qualifiers.extend(qualifier(&node));
                    let in_branch = node
                        .parent()
                        .is_some_and(|p| p.kind() == SyntaxKind::CaseBranch);
                    if in_branch {
                        facts.branch_constructors.push(qualified_name(&node));
                    }
                }
                SyntaxKind::ExposedValue => {
                    if let Some(range) = code_range(&node) {
                        facts.exposed_values.push((words(&node), range));
                    }
                }
                SyntaxKind::ExposedType => facts.exposed_types.push(text(&node)),
                SyntaxKind::ModuleHeader => {
                    facts.exposes_all = child(&node, SyntaxKind::ExposingList)
                        .is_some_and(|list| first_token(&list, SyntaxKind::DotDot).is_some());
                }
                _ => {}
            }
        }
        facts
    }

    /// Whether some `value_expr`, `record_base_identifier` or
    /// `exposed_value` writes `name`, and where.
    fn value_uses<'f>(&'f self, name: &'f str) -> impl Iterator<Item = TextRange> + 'f {
        (self
            .values
            .iter()
            .chain(&self.record_bases)
            .chain(&self.exposed_values))
        .filter(move |(text, _)| text == name)
        .map(|(_, range)| *range)
    }
}

struct Lint<'p> {
    path: &'p Path,
    problems: Vec<Problem>,
}

impl Lint<'_> {
    fn warn(&mut self, range: TextRange, message: String, code: &'static str, unnecessary: bool) {
        self.problems.push(Problem {
            path: self.path.to_owned(),
            range,
            severity: Severity::Warning,
            message,
            source: "ElmLS",
            code,
            unnecessary,
            relative: None,
            fixes: Vec::new(),
        });
    }

    /// `getUnusedImportValueAndTypeDiagnostics`: the exposed values and
    /// types of the imports that nothing writes, each but `T(..)`. Gives
    /// back where they are, for `getUnusedImportDiagnostics`.
    fn unused_imported_values(&self, imports: &[SyntaxNode], facts: &Facts) -> HashSet<TextRange> {
        let mut unused = HashSet::new();
        for (exposed, _, range) in import_exposures(imports) {
            let text = text(&exposed);
            if text.ends_with("(..)") {
                continue;
            }
            let used = (facts.values.iter().chain(&facts.record_bases)).any(|(t, _)| *t == text)
                || facts.types.contains(&text);
            if !used {
                unused.insert(range);
            }
        }
        unused
    }

    /// `getUnusedImportDiagnostics`: an import whose exposed names are
    /// all unused, or that exposes none, and that nothing qualifies a name
    /// with.
    fn unused_imports(
        &mut self,
        imports: &[SyntaxNode],
        facts: &Facts,
        unused: &HashSet<TextRange>,
    ) {
        for import in imports {
            let exposing = child(import, SyntaxKind::ExposingList);
            let exposes_nothing_used = exposing.as_ref().is_none_or(|list| {
                first_token(list, SyntaxKind::DotDot).is_none()
                    && list
                        .children()
                        .filter_map(|item| code_range(&item))
                        .all(|range| unused.contains(&range))
            });
            if !exposes_nothing_used {
                continue;
            }
            let name = child(import, SyntaxKind::ImportAlias)
                .and_then(|alias| token(&alias, SyntaxKind::UpperIdent))
                .or_else(|| child(import, SyntaxKind::ModuleName).map(|n| words(&n)));
            let Some(name) = name else { continue };
            if facts.qualifiers.contains(&name) {
                continue;
            }
            if let Some(range) = code_range(import) {
                self.warn(
                    range,
                    format!("Unused import `{name}`"),
                    "unused_import",
                    true,
                );
            }
        }
    }

    /// `getUnusedImportAliasDiagnostics`: an alias nothing qualifies a
    /// name with, on an import that exposes something; without that,
    /// the import is unused.
    fn unused_aliases(&mut self, imports: &[SyntaxNode], facts: &Facts) {
        for import in imports {
            let Some(alias) = child(import, SyntaxKind::ImportAlias) else {
                continue;
            };
            if child(import, SyntaxKind::ExposingList).is_none() {
                continue;
            }
            let Some(name) = token(&alias, SyntaxKind::UpperIdent) else {
                continue;
            };
            if !facts.qualifiers.contains(&name)
                && let Some(range) = code_range(&alias)
            {
                let message = format!("Unused import alias `{name}`");
                self.warn(range, message, "unused_alias", true);
            }
        }
    }

    /// `getUnusedPatternVariableDiagnostics`: every variable a pattern
    /// binds that nothing in its scope writes, and every top-level value
    /// nothing outside its own declaration writes, unless the module
    /// exposes everything.
    fn unused_patterns(&mut self, root: &SyntaxNode, facts: &Facts) {
        let mut found: Vec<(TextSize, Problem)> = Vec::new();
        let mut push = |at: TextRange, problem: Problem| found.push((at.start(), problem));
        for node in root.descendants() {
            match node.kind() {
                SyntaxKind::ValueDecl | SyntaxKind::LetValue => {
                    let Some(definition) = child(&node, SyntaxKind::Definition) else {
                        continue;
                    };
                    // A declaration's parameters are in scope in it.
                    let scope = node.clone();
                    for pattern in definition.children().filter(|c| is_pattern(c.kind())) {
                        for (name, range) in bindings(&pattern) {
                            if let Some(problem) = self.unused_in(&scope, &name, range) {
                                push(range, problem);
                            }
                        }
                    }
                    let Some(name) = first_token(&definition, SyntaxKind::LowerIdent) else {
                        continue;
                    };
                    let (text, range) = (name.text().to_owned(), name.text_range());
                    if node.kind() == SyntaxKind::ValueDecl {
                        let outside = facts.exposes_all
                            || facts.value_uses(&text).any(|used| {
                                !code_range(&node).is_some_and(|r| r.contains_range(used))
                            });
                        if !outside {
                            let message = format!("Unused top level definition `{text}`");
                            let problem = self.problem(range, message, "unused_top_level");
                            push(range, problem);
                        }
                    } else if let Some(scope) =
                        node.ancestors().find(|a| a.kind() == SyntaxKind::LetExpr)
                        && let Some(problem) = self.unused_in(&scope, &text, range)
                    {
                        push(range, problem);
                    }
                }
                SyntaxKind::DestructureDecl | SyntaxKind::LetDestructure => {
                    let scope = if node.kind() == SyntaxKind::LetDestructure {
                        node.ancestors().find(|a| a.kind() == SyntaxKind::LetExpr)
                    } else {
                        Some(node.clone())
                    };
                    let (Some(scope), Some(pattern)) =
                        (scope, node.children().find(|c| is_pattern(c.kind())))
                    else {
                        continue;
                    };
                    for (name, range) in bindings(&pattern) {
                        if let Some(problem) = self.unused_in(&scope, &name, range) {
                            push(range, problem);
                        }
                    }
                }
                SyntaxKind::CaseBranch | SyntaxKind::LambdaExpr => {
                    for pattern in node.children().filter(|c| is_pattern(c.kind())) {
                        for (name, range) in bindings(&pattern) {
                            if let Some(problem) = self.unused_in(&node, &name, range) {
                                push(range, problem);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // tree-sitter gives a match once it has all its nodes, so in the
        // order of the names bound.
        found.sort_by_key(|(at, _)| *at);
        self.problems.extend(found.into_iter().map(|(_, p)| p));
    }

    fn problem(&self, range: TextRange, message: String, code: &'static str) -> Problem {
        Problem {
            path: self.path.to_owned(),
            range,
            severity: Severity::Warning,
            message,
            source: "ElmLS",
            code,
            unnecessary: true,
            relative: None,
            fixes: Vec::new(),
        }
    }

    /// The problem of a variable `name` nothing in `scope` writes.
    fn unused_in(&self, scope: &SyntaxNode, name: &str, range: TextRange) -> Option<Problem> {
        let used = Facts::of(scope).value_uses(name).next().is_some();
        let message = format!("Unused pattern variable `{name}`");
        (!used).then(|| self.problem(range, message, "unused_pattern"))
    }

    /// `getCaseBranchMapNothingToNothingDiagnostics`: a branch
    /// `Nothing -> Nothing`.
    fn map_nothing_to_nothing(&mut self, root: &SyntaxNode) {
        for branch in root
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::CaseBranch)
        {
            let pattern = branch.children().find(|c| is_pattern(c.kind()));
            let body = branch.children().filter(|c| is_expr(c.kind())).last();
            let nothing = |node: Option<SyntaxNode>| node.is_some_and(|n| text(&n) == "Nothing");
            let body_is_name = body
                .as_ref()
                .is_some_and(|b| b.kind() == SyntaxKind::NameExpr);
            if nothing(pattern)
                && body_is_name
                && nothing(body)
                && let Some(range) = code_range(&branch)
            {
                let message = "`Nothing` mapped to `Nothing` in case expression. Use Maybe.map or Maybe.andThen instead.";
                self.warn(range, message.to_owned(), "map_nothing_to_nothing", false);
            }
        }
    }

    /// `getBooleanCaseExpressionDiagnostics`: a `case` with a branch on
    /// `True` or `False` and a later one on `True`, `False` or `_`.
    fn boolean_cases(&mut self, root: &SyntaxNode) {
        for case in root
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::CaseExpr)
        {
            let patterns: Vec<String> = (case.children())
                .filter(|c| c.kind() == SyntaxKind::CaseBranch)
                .filter_map(|b| b.children().find(|c| is_pattern(c.kind())))
                .map(|p| text(&p))
                .collect();
            let boolean = |p: &String| p == "True" || p == "False";
            let found = (patterns.iter().enumerate()).any(|(i, first)| {
                boolean(first) && patterns[i + 1..].iter().any(|p| boolean(p) || p == "_")
            });
            if found && let Some(range) = code_range(&case) {
                let message = "Use an if expression instead of a case expression.".to_owned();
                self.warn(range, message, "boolean_case_expr", false);
            }
        }
    }

    /// The rules on two operands of `op` side by side in a chain, as
    /// `pair` takes them.
    fn chains(&mut self, root: &SyntaxNode, op: &str, pair: Pair) {
        for chain in root
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::OperatorChain)
        {
            let parts: Vec<SyntaxNode> = chain.children().collect();
            for window in parts.windows(3) {
                let [left, operator, right] = window else {
                    continue;
                };
                if operator.kind() != SyntaxKind::Op || text(operator) != op {
                    continue;
                }
                let list = |n: &SyntaxNode| n.kind() == SyntaxKind::ListExpr;
                let singleton = |n: &SyntaxNode| {
                    list(n) && n.children().filter(|c| is_expr(c.kind())).count() == 1
                };
                let (fits, message, code) = match pair {
                    Pair::ListThenList => (
                        list(left) && list(right),
                        "If you concatenate two lists, then you can merge them into one list.",
                        "drop_concat_of_lists",
                    ),
                    Pair::AnyThenList => (
                        list(right),
                        "If you cons an item to a literal list, then you can just put the item into the list.",
                        "drop_cons_of_item_and_list",
                    ),
                    Pair::SingletonThenAny => (
                        singleton(left),
                        "If you concatenate two lists, but the first item is a single element list, then you should use the cons operator.",
                        "use_cons_over_concat",
                    ),
                };
                if fits && let (Some(start), Some(end)) = (code_range(left), code_range(right)) {
                    let range = TextRange::new(start.start(), end.end());
                    self.warn(range, message.to_owned(), code, false);
                }
            }
        }
    }

    /// `getSingleFieldRecordDiagnostics`: a record type of one field,
    /// unless it's given to an alias of a record it extends, which its
    /// type checker sees as a record of more fields.
    fn single_field_records(&mut self, root: &SyntaxNode) {
        let extending: Vec<String> = (root.children())
            .filter(|n| n.kind() == SyntaxKind::TypeAliasDecl)
            .filter(|alias| {
                (alias.children().find(|c| is_type(c.kind())))
                    .is_some_and(|ty| child(&ty, SyntaxKind::RecordExtension).is_some())
            })
            .filter_map(|alias| token(&alias, SyntaxKind::UpperIdent))
            .collect();
        for record in root
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::RecordType)
        {
            let extended = (record.parent())
                .filter(|p| p.kind() == SyntaxKind::NamedType)
                .is_some_and(|p| extending.contains(&qualified_name(&p)));
            let single = !extended
                && child(&record, SyntaxKind::RecordExtension).is_none()
                && record
                    .children()
                    .filter(|c| c.kind() == SyntaxKind::FieldType)
                    .count()
                    == 1;
            if single && let Some(range) = code_range(&record) {
                let message =
                    "Using a record is obsolete if you only plan to store a single field in it.";
                self.warn(range, message.to_owned(), "single_field_record", false);
            }
        }
    }

    /// `getUnnecessaryListConcatDiagnostics`: `List.concat` of a literal
    /// list of literal lists.
    fn unnecessary_list_concats(&mut self, root: &SyntaxNode) {
        for call in root
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::CallExpr)
        {
            let mut parts = call.children().filter(|c| is_expr(c.kind()));
            let Some(function) = parts.next() else {
                continue;
            };
            if function.kind() != SyntaxKind::NameExpr || text(&function) != "List.concat" {
                continue;
            }
            let of_lists = parts.any(|arg| {
                let items: Vec<SyntaxNode> = arg.children().filter(|c| is_expr(c.kind())).collect();
                arg.kind() == SyntaxKind::ListExpr
                    && !items.is_empty()
                    && items.iter().all(|i| i.kind() == SyntaxKind::ListExpr)
            });
            if of_lists && let Some(range) = code_range(&call) {
                let message =
                    "You should just merge the arguments of `List.concat` to a single list.";
                self.warn(range, message.to_owned(), "unnecessary_list_concat", false);
            }
        }
    }

    /// `getFullyAppliedOperatorAsPrefixDiagnostics`: an operator called
    /// as a function on two arguments or more.
    fn uncurried_prefixes(&mut self, root: &SyntaxNode) {
        for call in root
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::CallExpr)
        {
            let parts: Vec<SyntaxNode> = call.children().filter(|c| is_expr(c.kind())).collect();
            // tree-sitter-elm knows the operators Elm's packages define,
            // and no others.
            let known = parts.first().is_some_and(|f| {
                f.kind() == SyntaxKind::OperatorFunction
                    && OPERATORS
                        .contains(&text(f).trim_start_matches('(').trim_end_matches(')').trim())
            });
            if parts.len() >= 3
                && known
                && let Some(range) = code_range(&call)
            {
                let message = "Don't use fully applied prefix notation for operators.";
                self.warn(range, message.to_owned(), "no_uncurried_prefix", false);
            }
        }
    }

    /// `getUnnecessaryPortModuleDiagnostics`: a `port module` without
    /// ports.
    fn unnecessary_port_module(&mut self, root: &SyntaxNode) {
        let has_ports = root.children().any(|n| n.kind() == SyntaxKind::PortDecl);
        let Some(header) = child(root, SyntaxKind::ModuleHeader) else {
            return;
        };
        if !has_ports
            && first_token(&header, SyntaxKind::PortKw).is_some()
            && let Some(range) = code_range(&header)
        {
            let message =
                "Module is defined as a `port` module, but does not define any ports.".to_owned();
            self.warn(range, message, "unnecessary_port_module", false);
        }
    }

    /// `getUnusedIncomingPortDiagnostics`: an incoming port, one whose
    /// type ends in a `Sub`, that no expression in the project uses.
    fn unused_incoming_ports(&mut self, snapshot: &Snapshot, index: &Index, root: &SyntaxNode) {
        for port in root.children().filter(|n| n.kind() == SyntaxKind::PortDecl) {
            let Some(name) = first_token(&port, SyntaxKind::LowerIdent) else {
                continue;
            };
            let Some(ty) = port.children().find(|c| is_type(c.kind())) else {
                continue;
            };
            if !incoming(&ty) {
                continue;
            }
            let at = name.text_range().start();
            let used = (crate::references(snapshot, index, self.path, at, false).iter())
                .any(|location| in_expression(index, location));
            if !used {
                let message = format!(
                    "Incoming port `{}` is never referenced in Elm code.",
                    name.text()
                );
                self.warn(name.text_range(), message, "unused_incoming_port", true);
            }
        }
    }

    /// `getUnusedTypeAliasDiagnostics`: a type alias nothing writes, as a
    /// type, a record constructor or an exposed type, unless the module
    /// exposes everything.
    fn unused_type_aliases(&mut self, root: &SyntaxNode, facts: &Facts) {
        if facts.exposes_all {
            return;
        }
        for alias in root
            .children()
            .filter(|n| n.kind() == SyntaxKind::TypeAliasDecl)
        {
            let Some(name) = token(&alias, SyntaxKind::UpperIdent) else {
                continue;
            };
            let used = facts.values.iter().any(|(t, _)| *t == name)
                || facts.exposed_types.contains(&name)
                || facts.types.contains(&name);
            if !used && let Some(range) = code_range(&alias) {
                let message = format!("Type alias `{name}` is not used.");
                self.warn(range, message, "unused_type_alias", true);
            }
        }
    }

    /// `getUnusedValueConstructorDiagnostics`: a variant nothing writes,
    /// as a value, a type, or a case branch's own pattern, and that no
    /// `T(..)` exposes, unless the module exposes everything.
    fn unused_constructors(&mut self, root: &SyntaxNode, facts: &Facts) {
        if facts.exposes_all {
            return;
        }
        for ty in root
            .children()
            .filter(|n| n.kind() == SyntaxKind::CustomTypeDecl)
        {
            let Some(type_name) = token(&ty, SyntaxKind::UpperIdent) else {
                continue;
            };
            let exposed = facts.exposed_types.contains(&format!("{type_name}(..)"));
            for variant in ty.children().filter(|n| n.kind() == SyntaxKind::Variant) {
                let Some(name) = token(&variant, SyntaxKind::UpperIdent) else {
                    continue;
                };
                let used = exposed
                    || facts.values.iter().any(|(t, _)| *t == name)
                    || facts.types.contains(&name)
                    || facts.branch_constructors.contains(&name);
                if !used && let Some(range) = code_range(&variant) {
                    let message = format!("Value constructor `{name}` is not used.");
                    self.warn(range, message, "unused_value_constructor", true);
                }
            }
        }
    }
}

/// The problems for the unused exposed names `unused` holds, in order.
fn found_values(imports: &[SyntaxNode], unused: &HashSet<TextRange>, path: &Path) -> Vec<Problem> {
    let mut problems = Vec::new();
    for (exposed, kind, range) in import_exposures(imports) {
        if !unused.contains(&range) {
            continue;
        }
        let what = if kind == SyntaxKind::ExposedType {
            "type"
        } else {
            "value"
        };
        problems.push(Problem {
            path: path.to_owned(),
            range,
            severity: Severity::Warning,
            message: format!("Unused imported {what} `{}`", text(&exposed)),
            source: "ElmLS",
            code: "unused_imported_value",
            unnecessary: true,
            relative: None,
            fixes: Vec::new(),
        });
    }
    problems
}

/// The exposed values and types of every import, in order, with their
/// kinds and ranges.
fn import_exposures(imports: &[SyntaxNode]) -> Vec<(SyntaxNode, SyntaxKind, TextRange)> {
    let mut found = Vec::new();
    for import in imports {
        let Some(list) = child(import, SyntaxKind::ExposingList) else {
            continue;
        };
        for item in list.children() {
            let kind = item.kind();
            if matches!(kind, SyntaxKind::ExposedValue | SyntaxKind::ExposedType)
                && let Some(range) = code_range(&item)
            {
                found.push((item, kind, range));
            }
        }
    }
    found
}

/// The variables `pattern` binds, as tree-sitter's `lower_pattern`s:
/// variables, record fields and `as` names, in order.
pub(crate) fn bindings(pattern: &SyntaxNode) -> Vec<(String, TextRange)> {
    let mut found = Vec::new();
    for node in std::iter::once(pattern.clone()).chain(pattern.descendants().skip(1)) {
        match node.kind() {
            // tree-sitter reads an empty `{}` as a record pattern of one
            // variable with no name, which is never used. It's reported
            // over its parent's parent: a declaration's name and
            // parameters, or the pattern around it.
            SyntaxKind::RecordPattern if child(&node, SyntaxKind::FieldPattern).is_none() => {
                let parent = node.parent();
                let range = match parent.filter(|p| p.kind() == SyntaxKind::Definition) {
                    Some(definition) => {
                        let start = code_range(&definition).map(|r| r.start());
                        let end = (definition.children())
                            .filter(|c| is_pattern(c.kind()))
                            .last()
                            .and_then(|last| code_range(&last));
                        start
                            .zip(end)
                            .map(|(start, end)| TextRange::new(start, end.end()))
                    }
                    None => code_range(&node),
                };
                if let Some(range) = range {
                    found.push((String::new(), range));
                }
            }
            SyntaxKind::VarPattern | SyntaxKind::FieldPattern => {
                if let Some(name) = first_token(&node, SyntaxKind::LowerIdent) {
                    found.push((name.text().to_owned(), name.text_range()));
                }
            }
            SyntaxKind::AliasPattern => {
                let name = (node.children_with_tokens())
                    .filter_map(|e| e.into_token())
                    .filter(|t| t.kind() == SyntaxKind::LowerIdent)
                    .last();
                if let Some(name) = name {
                    found.push((name.text().to_owned(), name.text_range()));
                }
            }
            _ => {}
        }
    }
    found.sort_by_key(|(_, range)| range.start());
    found
}

/// Whether a port's type ends in a `Sub`: an incoming port.
fn incoming(ty: &SyntaxNode) -> bool {
    let mut ty = ty.clone();
    let mut function = false;
    while ty.kind() == SyntaxKind::FunctionType {
        function = true;
        let Some(result) = ty.children().filter(|c| is_type(c.kind())).nth(1) else {
            return false;
        };
        ty = result;
    }
    function
        && ty.kind() == SyntaxKind::NamedType
        && token(&ty, SyntaxKind::UpperIdent).as_deref() == Some("Sub")
}

/// Whether `location` is in an expression, rather than a declaration or
/// an exposing list.
fn in_expression(index: &Index, location: &crate::Location) -> bool {
    let Some(module) = index.module(&location.path) else {
        return false;
    };
    let Some(token) = module
        .root()
        .token_at_offset(location.range.start())
        .right_biased()
    else {
        return false;
    };
    token
        .parent_ancestors()
        .any(|a| a.kind() == SyntaxKind::NameExpr)
}

/// A node's qualifier, as a module name: `Html.Attributes` of
/// `Html.Attributes.class`.
fn qualifier(node: &SyntaxNode) -> Option<String> {
    let qualifier = child(node, SyntaxKind::Qualifier)?;
    Some(words(&qualifier).trim_end_matches('.').to_owned())
}

/// A name with its qualifier: `Html.Html`.
fn qualified_name(node: &SyntaxNode) -> String {
    let qualifier = child(node, SyntaxKind::Qualifier)
        .map(|q| words(&q))
        .unwrap_or_default();
    let name = token(node, SyntaxKind::UpperIdent).unwrap_or_default();
    format!("{qualifier}{name}")
}

/// A node's source text from its first token that isn't trivia to its
/// last: tree-sitter's `text`.
fn text(node: &SyntaxNode) -> String {
    let Some(range) = code_range(node) else {
        return String::new();
    };
    let start = range.start() - node.text_range().start();
    node.text().slice(start..start + range.len()).to_string()
}

/// The imports that parsed whole: a half-written one, such as `import`
/// or `import X as`, has an error from its start up to the next node,
/// and is no import to lint.
fn imports<'r>(
    root: &'r SyntaxNode,
    errors: &'r [TextSize],
) -> impl Iterator<Item = SyntaxNode> + 'r {
    root.children().filter(move |n| {
        let end = n
            .next_sibling()
            .map_or(n.text_range().end(), |next| next.text_range().start());
        n.kind() == SyntaxKind::Import
            && !errors
                .iter()
                .any(|&at| n.text_range().start() <= at && at <= end)
    })
}

pub(crate) fn first_token(node: &SyntaxNode, kind: SyntaxKind) -> Option<nomi_syntax::SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == kind)
}

pub(crate) fn is_pattern(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::VarPattern
            | SyntaxKind::WildcardPattern
            | SyntaxKind::LiteralPattern
            | SyntaxKind::UnitPattern
            | SyntaxKind::ParenPattern
            | SyntaxKind::TuplePattern
            | SyntaxKind::ListPattern
            | SyntaxKind::RecordPattern
            | SyntaxKind::ConstructorPattern
            | SyntaxKind::ConsPattern
            | SyntaxKind::AliasPattern
    )
}

/// The operators Elm's packages define, which tree-sitter-elm reads.
const OPERATORS: &[&str] = &[
    "+", "-", "*", "/", "//", "^", "==", "/=", "<", ">", "<=", ">=", "&&", "||", "++", "<|", "|>",
    "<<", ">>", "::", "</>", "<?>", "|.", "|=",
];

/// Which two operands a chain rule takes.
#[derive(Clone, Copy)]
enum Pair {
    ListThenList,
    AnyThenList,
    SingletonThenAny,
}

fn is_expr(kind: SyntaxKind) -> bool {
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

pub(crate) fn is_type(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::NamedType
            | SyntaxKind::TypeVar
            | SyntaxKind::FunctionType
            | SyntaxKind::UnitType
            | SyntaxKind::ParenType
            | SyntaxKind::TupleType
            | SyntaxKind::TupleConstructorType
            | SyntaxKind::RecordType
    )
}
