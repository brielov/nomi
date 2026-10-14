//! elm-language-server's own lint rules
//! (`elmLsDiagnostics.ts`), over nomi's syntax tree. They are
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
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, child_token, parse_module};

use crate::navigation::Declaration;
use crate::problems::{Code, Problem};
use crate::scopes::bindings;
use crate::tree::{child, code_range, code_text, words};

macro_rules! rules {
    ($($(#[$doc:meta])* $rule:ident,)*) => {
        /// elm-language-server's lint rules, as `elm-analyse.json` names
        /// them.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Rule {
            $($(#[$doc])* $rule,)*
        }

        impl Rule {
            /// The rule `elm-analyse.json` names `name`, where it names
            /// one.
            pub fn from_name(name: &str) -> Option<Rule> {
                match name {
                    $(stringify!($rule) => Some(Rule::$rule),)*
                    _ => None,
                }
            }
        }
    };
}

rules! {
    UnusedImport,
    UnusedImportedVariable,
    UnusedImportAlias,
    UnusedPatternVariable,
    MapNothingToNothing,
    BooleanCase,
    DropConcatOfLists,
    DropConsOfItemAndList,
    UseConsOverConcat,
    SingleFieldRecord,
    UnnecessaryListConcat,
    UnnecessaryPortModule,
    UnusedIncomingPort,
    NoUncurriedPrefix,
    UnusedTypeAlias,
    UnusedValueConstructor,
    /// The annotations a module leaves out, which elm-language-server's
    /// type checker reports.
    MissingTypeAnnotation,
}

/// Which rules run: elm-analyse.json's `checks`, each on unless set to
/// false, `SingleFieldRecord` off unless set to true.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checks {
    set: Vec<(Rule, bool)>,
}

impl Checks {
    /// The rules `set` turns on or off.
    pub fn new(set: impl IntoIterator<Item = (Rule, bool)>) -> Checks {
        Checks {
            set: set.into_iter().collect(),
        }
    }

    /// Whether `rule` runs.
    pub fn on(&self, rule: Rule) -> bool {
        let set = self.set.iter().find(|(r, _)| *r == rule);
        set.map_or(rule != Rule::SingleFieldRecord, |(_, on)| *on)
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
    let imports: Vec<ast::Import> = imports(&root, &errors).collect();
    let mut found = Lint {
        path,
        problems: Vec::new(),
    };
    let unused_exposures =
        if checks.on(Rule::UnusedImport) || checks.on(Rule::UnusedImportedVariable) {
            unused_imported_values(&imports, &facts)
        } else {
            HashSet::new()
        };
    if checks.on(Rule::UnusedImport) {
        found.unused_imports(&imports, &facts, &unused_exposures);
    }
    if checks.on(Rule::UnusedImportedVariable) {
        found.unused_exposures(&imports, &unused_exposures);
    }
    if checks.on(Rule::UnusedImportAlias) {
        found.unused_aliases(&imports, &facts);
    }
    if checks.on(Rule::UnusedPatternVariable) {
        found.unused_patterns(&root, &facts);
    }
    if checks.on(Rule::MapNothingToNothing) {
        found.map_nothing_to_nothing(&root);
    }
    if checks.on(Rule::BooleanCase) {
        found.boolean_cases(&root);
    }
    if checks.on(Rule::DropConcatOfLists) {
        found.chains(&root, Pair::ListThenList);
    }
    if checks.on(Rule::DropConsOfItemAndList) {
        found.chains(&root, Pair::AnyThenList);
    }
    if checks.on(Rule::UseConsOverConcat) {
        found.chains(&root, Pair::SingletonThenAny);
    }
    if checks.on(Rule::SingleFieldRecord) {
        found.single_field_records(&root);
    }
    if checks.on(Rule::UnnecessaryListConcat) {
        found.unnecessary_list_concats(&root);
    }
    if checks.on(Rule::UnnecessaryPortModule) {
        found.unnecessary_port_module(&root);
    }
    if checks.on(Rule::UnusedIncomingPort)
        && let Some(index) = index
    {
        found.unused_incoming_ports(snapshot, index, &root);
    }
    if checks.on(Rule::NoUncurriedPrefix) {
        found.uncurried_prefixes(&root);
    }
    if checks.on(Rule::UnusedTypeAlias) {
        found.unused_type_aliases(&root, &facts);
    }
    if checks.on(Rule::UnusedValueConstructor) {
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
        let named = |token: SyntaxToken| (token.text().to_owned(), token.text_range());
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
                    facts
                        .values
                        .extend(child_token(&node, SyntaxKind::LowerIdent).map(named));
                }
                SyntaxKind::RecordUpdateExpr => {
                    facts
                        .record_bases
                        .extend(child_token(&node, SyntaxKind::LowerIdent).map(named));
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
                    facts.exposes_all = (ast::ModuleHeader::cast(node))
                        .and_then(|header| header.exposing_list())
                        .is_some_and(|list| list.dot_dot_token().is_some());
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
    fn warn(&mut self, range: TextRange, message: String, code: Code) {
        (self.problems).push(Problem::new(self.path.to_owned(), range, code, message));
    }

    /// `getUnusedImportDiagnostics`: an import whose exposed names are
    /// all unused, or that exposes none, and that nothing qualifies a name
    /// with.
    fn unused_imports(
        &mut self,
        imports: &[ast::Import],
        facts: &Facts,
        unused: &HashSet<TextRange>,
    ) {
        for import in imports {
            let exposes_nothing_used = import.exposing_list().is_none_or(|list| {
                list.dot_dot_token().is_none()
                    && (list.syntax().children())
                        .filter_map(|item| code_range(&item))
                        .all(|range| unused.contains(&range))
            });
            if !exposes_nothing_used {
                continue;
            }
            let name = (import.import_alias())
                .and_then(|alias| alias.upper_ident_token())
                .map(|alias| alias.text().to_owned())
                .or_else(|| import.module_name().map(|n| words(n.syntax())));
            let Some(name) = name else { continue };
            if facts.qualifiers.contains(&name) {
                continue;
            }
            if let Some(range) = code_range(import.syntax()) {
                self.warn(range, format!("Unused import `{name}`"), Code::UnusedImport);
            }
        }
    }

    /// `getUnusedImportValueAndTypeDiagnostics`' problems, for the
    /// exposed names `unused` holds, in order.
    fn unused_exposures(&mut self, imports: &[ast::Import], unused: &HashSet<TextRange>) {
        for (exposed, range) in import_exposures(imports) {
            if !unused.contains(&range) {
                continue;
            }
            let what = if exposed.kind() == SyntaxKind::ExposedType {
                "type"
            } else {
                "value"
            };
            let message = format!("Unused imported {what} `{}`", text(&exposed));
            self.warn(range, message, Code::UnusedImportedValue);
        }
    }

    /// `getUnusedImportAliasDiagnostics`: an alias nothing qualifies a
    /// name with, on an import that exposes something; without that,
    /// the import is unused.
    fn unused_aliases(&mut self, imports: &[ast::Import], facts: &Facts) {
        for import in imports {
            let Some(alias) = import.import_alias() else {
                continue;
            };
            if import.exposing_list().is_none() {
                continue;
            }
            let Some(name) = alias.upper_ident_token() else {
                continue;
            };
            if !facts.qualifiers.iter().any(|q| q == name.text())
                && let Some(range) = code_range(alias.syntax())
            {
                let message = format!("Unused import alias `{}`", name.text());
                self.warn(range, message, Code::UnusedAlias);
            }
        }
    }

    /// `getUnusedPatternVariableDiagnostics`: every variable a pattern
    /// binds that nothing in its scope writes, and every top-level value
    /// nothing outside its own declaration writes, unless the module
    /// exposes everything.
    fn unused_patterns(&mut self, root: &SyntaxNode, facts: &Facts) {
        let first = self.problems.len();
        let patterns =
            |node: &SyntaxNode| (node.children()).filter(|c| ast::Pattern::can_cast(c.kind()));
        for node in root.descendants() {
            match node.kind() {
                SyntaxKind::ValueDecl | SyntaxKind::LetValue => {
                    let Some(definition) = child(&node, SyntaxKind::Definition) else {
                        continue;
                    };
                    // A declaration's parameters are in scope in it.
                    self.unused_bindings(facts, &node, patterns(&definition));
                    let Some(name) = child_token(&definition, SyntaxKind::LowerIdent) else {
                        continue;
                    };
                    if node.kind() == SyntaxKind::ValueDecl {
                        let declaration = code_range(&node);
                        let outside = facts.exposes_all
                            || (facts.value_uses(name.text()))
                                .any(|used| !declaration.is_some_and(|r| r.contains_range(used)));
                        if !outside {
                            let message = format!("Unused top level definition `{}`", name.text());
                            self.warn(name.text_range(), message, Code::UnusedTopLevel);
                        }
                    } else if let Some(scope) =
                        node.ancestors().find(|a| a.kind() == SyntaxKind::LetExpr)
                    {
                        self.unused_in(facts, &scope, name.text(), name.text_range());
                    }
                }
                SyntaxKind::DestructureDecl | SyntaxKind::LetDestructure => {
                    let scope = if node.kind() == SyntaxKind::LetDestructure {
                        node.ancestors().find(|a| a.kind() == SyntaxKind::LetExpr)
                    } else {
                        Some(node.clone())
                    };
                    if let Some(scope) = scope {
                        self.unused_bindings(facts, &scope, patterns(&node).take(1));
                    }
                }
                SyntaxKind::CaseBranch | SyntaxKind::LambdaExpr => {
                    self.unused_bindings(facts, &node, patterns(&node));
                }
                _ => {}
            }
        }
        // tree-sitter gives a match once it has all its nodes, so in the
        // order of the names bound.
        self.problems[first..].sort_by_key(|p| p.range.start());
    }

    /// The problem of each variable `patterns` bind that nothing in
    /// `scope` writes.
    fn unused_bindings(
        &mut self,
        facts: &Facts,
        scope: &SyntaxNode,
        patterns: impl Iterator<Item = SyntaxNode>,
    ) {
        for pattern in patterns {
            for (name, range) in bindings(&pattern) {
                self.unused_in(facts, scope, &name, range);
            }
        }
    }

    /// The problem of a variable `name` nothing in `scope` writes, if
    /// nothing does.
    fn unused_in(&mut self, facts: &Facts, scope: &SyntaxNode, name: &str, range: TextRange) {
        let scope = scope.text_range();
        if !facts
            .value_uses(name)
            .any(|used| scope.contains_range(used))
        {
            let message = format!("Unused pattern variable `{name}`");
            self.warn(range, message, Code::UnusedPattern);
        }
    }

    /// `getCaseBranchMapNothingToNothingDiagnostics`: a branch
    /// `Nothing -> Nothing`.
    fn map_nothing_to_nothing(&mut self, root: &SyntaxNode) {
        for branch in root
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::CaseBranch)
        {
            let pattern = branch.children().find(|c| ast::Pattern::can_cast(c.kind()));
            let body = (branch.children())
                .filter(|c| ast::Expr::can_cast(c.kind()))
                .last();
            let nothing =
                |node: &Option<SyntaxNode>| node.as_ref().is_some_and(|n| text(n) == "Nothing");
            let body_is_name = body
                .as_ref()
                .is_some_and(|b| b.kind() == SyntaxKind::NameExpr);
            if nothing(&pattern)
                && body_is_name
                && nothing(&body)
                && let Some(range) = code_range(&branch)
            {
                let message = "`Nothing` mapped to `Nothing` in case expression. Use Maybe.map or Maybe.andThen instead.";
                self.warn(range, message.to_owned(), Code::MapNothingToNothing);
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
                .filter_map(|b| b.children().find(|c| ast::Pattern::can_cast(c.kind())))
                .map(|p| text(&p))
                .collect();
            let boolean = |p: &String| p == "True" || p == "False";
            let found = (patterns.iter().enumerate()).any(|(i, first)| {
                boolean(first) && patterns[i + 1..].iter().any(|p| boolean(p) || p == "_")
            });
            if found && let Some(range) = code_range(&case) {
                let message = "Use an if expression instead of a case expression.".to_owned();
                self.warn(range, message, Code::BooleanCaseExpr);
            }
        }
    }

    /// The rules on two operands of an operator side by side in a chain,
    /// as `pair` takes them.
    fn chains(&mut self, root: &SyntaxNode, pair: Pair) {
        let (op, message, code) = match pair {
            Pair::ListThenList => (
                "++",
                "If you concatenate two lists, then you can merge them into one list.",
                Code::DropConcatOfLists,
            ),
            Pair::AnyThenList => (
                "::",
                "If you cons an item to a literal list, then you can just put the item into the list.",
                Code::DropConsOfItemAndList,
            ),
            Pair::SingletonThenAny => (
                "++",
                "If you concatenate two lists, but the first item is a single element list, then you should use the cons operator.",
                Code::UseConsOverConcat,
            ),
        };
        let list = |n: &SyntaxNode| n.kind() == SyntaxKind::ListExpr;
        let singleton = |n: &SyntaxNode| {
            list(n)
                && (n.children())
                    .filter(|c| ast::Expr::can_cast(c.kind()))
                    .count()
                    == 1
        };
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
                let fits = match pair {
                    Pair::ListThenList => list(left) && list(right),
                    Pair::AnyThenList => list(right),
                    Pair::SingletonThenAny => singleton(left),
                };
                if fits && let (Some(start), Some(end)) = (code_range(left), code_range(right)) {
                    let range = TextRange::new(start.start(), end.end());
                    self.warn(range, message.to_owned(), code);
                }
            }
        }
    }

    /// `getSingleFieldRecordDiagnostics`: a record type of one field,
    /// unless it's given to an alias of a record it extends, which its
    /// type checker sees as a record of more fields.
    fn single_field_records(&mut self, root: &SyntaxNode) {
        let extending: Vec<String> = (root.children())
            .filter_map(ast::TypeAliasDecl::cast)
            .filter(|alias| {
                (alias.ty())
                    .is_some_and(|ty| child(ty.syntax(), SyntaxKind::RecordExtension).is_some())
            })
            .filter_map(|alias| Some(alias.upper_ident_token()?.text().to_owned()))
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
                self.warn(range, message.to_owned(), Code::SingleFieldRecord);
            }
        }
    }

    /// `getUnnecessaryListConcatDiagnostics`: `List.concat` of a literal
    /// list of literal lists.
    fn unnecessary_list_concats(&mut self, root: &SyntaxNode) {
        let exprs = |node: &SyntaxNode| (node.children()).filter(|c| ast::Expr::can_cast(c.kind()));
        for call in root
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::CallExpr)
        {
            let mut parts = exprs(&call);
            let Some(function) = parts.next() else {
                continue;
            };
            if function.kind() != SyntaxKind::NameExpr || text(&function) != "List.concat" {
                continue;
            }
            let of_lists = parts.any(|arg| {
                let mut items = exprs(&arg).peekable();
                arg.kind() == SyntaxKind::ListExpr
                    && items.peek().is_some()
                    && items.all(|i| i.kind() == SyntaxKind::ListExpr)
            });
            if of_lists && let Some(range) = code_range(&call) {
                let message =
                    "You should just merge the arguments of `List.concat` to a single list.";
                self.warn(range, message.to_owned(), Code::UnnecessaryListConcat);
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
            let parts: Vec<SyntaxNode> = (call.children())
                .filter(|c| ast::Expr::can_cast(c.kind()))
                .collect();
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
                self.warn(range, message.to_owned(), Code::NoUncurriedPrefix);
            }
        }
    }

    /// `getUnnecessaryPortModuleDiagnostics`: a `port module` without
    /// ports.
    fn unnecessary_port_module(&mut self, root: &SyntaxNode) {
        let has_ports = root.children().any(|n| n.kind() == SyntaxKind::PortDecl);
        let Some(header) = root.children().find_map(ast::ModuleHeader::cast) else {
            return;
        };
        if !has_ports
            && header.port_token().is_some()
            && let Some(range) = code_range(header.syntax())
        {
            let message =
                "Module is defined as a `port` module, but does not define any ports.".to_owned();
            self.warn(range, message, Code::UnnecessaryPortModule);
        }
    }

    /// `getUnusedIncomingPortDiagnostics`: an incoming port, one whose
    /// type ends in a `Sub`, that no expression in the project uses.
    fn unused_incoming_ports(&mut self, snapshot: &Snapshot, index: &Index, root: &SyntaxNode) {
        for port in root.children().filter_map(ast::PortDecl::cast) {
            let (Some(name), Some(ty)) = (port.lower_ident_token(), port.ty()) else {
                continue;
            };
            if !incoming(ty.syntax()) {
                continue;
            }
            let at = name.text_range().start();
            let references =
                crate::references(snapshot, index, self.path, at, Declaration::Exclude);
            if !references
                .iter()
                .any(|location| in_expression(index, location))
            {
                let message = format!(
                    "Incoming port `{}` is never referenced in Elm code.",
                    name.text()
                );
                self.warn(name.text_range(), message, Code::UnusedIncomingPort);
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
        for alias in root.children().filter_map(ast::TypeAliasDecl::cast) {
            let Some(name) = alias.upper_ident_token() else {
                continue;
            };
            let name = name.text();
            let used = facts.values.iter().any(|(t, _)| t == name)
                || facts.exposed_types.iter().any(|t| t == name)
                || facts.types.iter().any(|t| t == name);
            if !used && let Some(range) = code_range(alias.syntax()) {
                let message = format!("Type alias `{name}` is not used.");
                self.warn(range, message, Code::UnusedTypeAlias);
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
        for ty in root.children().filter_map(ast::CustomTypeDecl::cast) {
            let Some(type_name) = ty.upper_ident_token() else {
                continue;
            };
            let exposed = (facts.exposed_types.iter())
                .any(|t| t.strip_suffix("(..)") == Some(type_name.text()));
            for variant in ty.variants() {
                let Some(name) = variant.upper_ident_token() else {
                    continue;
                };
                let name = name.text();
                let used = exposed
                    || facts.values.iter().any(|(t, _)| t == name)
                    || facts.types.iter().any(|t| t == name)
                    || facts.branch_constructors.iter().any(|t| t == name);
                if !used && let Some(range) = code_range(variant.syntax()) {
                    let message = format!("Value constructor `{name}` is not used.");
                    self.warn(range, message, Code::UnusedValueConstructor);
                }
            }
        }
    }
}

/// `getUnusedImportValueAndTypeDiagnostics`: the exposed values and
/// types of the imports that nothing writes, each but `T(..)`. Gives
/// back where they are, for `getUnusedImportDiagnostics`.
fn unused_imported_values(imports: &[ast::Import], facts: &Facts) -> HashSet<TextRange> {
    let mut unused = HashSet::new();
    for (exposed, range) in import_exposures(imports) {
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

/// The exposed values and types of every import, in order, with their
/// ranges.
fn import_exposures(imports: &[ast::Import]) -> impl Iterator<Item = (SyntaxNode, TextRange)> + '_ {
    (imports.iter())
        .filter_map(ast::Import::exposing_list)
        .flat_map(|list| list.syntax().children())
        .filter(|item| {
            matches!(
                item.kind(),
                SyntaxKind::ExposedValue | SyntaxKind::ExposedType
            )
        })
        .filter_map(|item| {
            let range = code_range(&item)?;
            Some((item, range))
        })
}

/// Whether a port's type ends in a `Sub`: an incoming port.
fn incoming(ty: &SyntaxNode) -> bool {
    let mut ty = ty.clone();
    let mut function = false;
    while ty.kind() == SyntaxKind::FunctionType {
        function = true;
        let result = (ty.children())
            .filter(|c| ast::Type::can_cast(c.kind()))
            .nth(1);
        let Some(result) = result else {
            return false;
        };
        ty = result;
    }
    function
        && ty.kind() == SyntaxKind::NamedType
        && child_token(&ty, SyntaxKind::UpperIdent).is_some_and(|name| name.text() == "Sub")
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
    let mut name = child(node, SyntaxKind::Qualifier)
        .map(|q| words(&q))
        .unwrap_or_default();
    name.extend(child_token(node, SyntaxKind::UpperIdent).map(|t| t.text().to_owned()));
    name
}

/// A node's text as tree-sitter's `text` reads it, empty where the node
/// is.
fn text(node: &SyntaxNode) -> String {
    code_text(node).unwrap_or_default()
}

/// The imports that parsed whole: a half-written one, such as `import`
/// or `import X as`, has an error from its start up to the next node,
/// and is no import to lint.
fn imports<'r>(
    root: &'r SyntaxNode,
    errors: &'r [TextSize],
) -> impl Iterator<Item = ast::Import> + 'r {
    root.children()
        .filter_map(ast::Import::cast)
        .filter(move |import| {
            let n = import.syntax();
            let end = n
                .next_sibling()
                .map_or_else(|| n.text_range().end(), |next| next.text_range().start());
            !errors
                .iter()
                .any(|&at| n.text_range().start() <= at && at <= end)
        })
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
