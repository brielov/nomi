//! Canonicalizing a build (plan 5.8): package interfaces from their
//! sources, then each local module after the modules it imports, as
//! upstream's `Build.hs` checks them. A module whose import failed is
//! blocked and stays silent.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::Source;
use nomi_base::diagnostic::doc::Doc;
use nomi_base::{Compat, TextRange};
use nomi_compiler::canonicalize::error::Error;
use nomi_compiler::canonicalize::module::{self, canonicalize, interface};
use nomi_compiler::hir;
use nomi_compiler::interface::Interface;
use nomi_project::{ModuleName, Name, Outline};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxNode, parse_module};

use crate::graph::{Env, Graph, Module, Owner, Root, Status};

/// Each package's exposed modules' interfaces, by package.
pub type PackageInterfaces = BTreeMap<Name, HashMap<String, Interface>>;

/// The interfaces of every exposed module of `packages`, each a package
/// name and the directory of its source, canonicalized from the source
/// in dependency order. A module nomi can't canonicalize is an error,
/// naming it.
pub fn package_interfaces(
    compat: Compat,
    packages: &[(Name, PathBuf)],
) -> Result<PackageInterfaces, String> {
    let mut outlines = BTreeMap::new();
    for (name, dir) in packages {
        let bytes = fs::read(dir.join("elm.json")).map_err(|e| format!("{name}: {e}"))?;
        match Outline::decode(&bytes, compat) {
            Ok(Outline::Pkg(pkg)) => {
                outlines.insert(name.clone(), (pkg, dir.clone()));
            }
            _ => return Err(format!("{name}'s elm.json isn't a package's")),
        }
    }
    let mut done: PackageInterfaces = BTreeMap::new();
    // Dependencies first.
    let mut pending: Vec<Name> = outlines.keys().cloned().collect();
    while !pending.is_empty() {
        let ready = pending
            .iter()
            .position(|name| {
                outlines[name]
                    .0
                    .deps
                    .keys()
                    .all(|d| done.contains_key(d) || !outlines.contains_key(d))
            })
            .ok_or("the packages depend on each other in a cycle")?;
        let name = pending.remove(ready);
        let (pkg, dir) = &outlines[&name];
        let mut visible: HashMap<String, Interface> = HashMap::new();
        for dep in pkg.deps.keys() {
            if let Some(exposed) = done.get(dep) {
                visible.extend(exposed.iter().map(|(m, i)| (m.clone(), i.clone())));
            }
        }
        let own = package(compat, &name.to_string(), &dir.join("src"), visible)?;
        let exposed: HashSet<String> = pkg.exposed.modules().map(|m| m.to_string()).collect();
        done.insert(
            name.clone(),
            own.into_iter()
                .filter(|(m, _)| exposed.contains(m))
                .collect(),
        );
    }
    Ok(done)
}

/// The interfaces of every module under `src`, canonicalized after the
/// modules of theirs they import.
fn package(
    compat: Compat,
    package: &str,
    src: &Path,
    mut interfaces: HashMap<String, Interface>,
) -> Result<HashMap<String, Interface>, String> {
    let mut modules: BTreeMap<String, hir::Module> = BTreeMap::new();
    for file in elm_files(src).map_err(|e| format!("{package}: {e}"))? {
        let text = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let parse = parse_module(&text);
        let module = hir::lower(&ast::Module::cast(parse.syntax()).expect("a module"));
        modules.insert(module::name(&module).to_owned(), module);
    }
    let mut own = HashMap::new();
    let mut state: HashMap<String, bool> = HashMap::new();
    let names: Vec<String> = modules.keys().cloned().collect();
    for name in names {
        visit(
            compat,
            package,
            &name,
            &modules,
            &mut state,
            &mut interfaces,
            &mut own,
        )?;
    }
    Ok(own)
}

fn visit(
    compat: Compat,
    package: &str,
    name: &str,
    modules: &BTreeMap<String, hir::Module>,
    state: &mut HashMap<String, bool>,
    interfaces: &mut HashMap<String, Interface>,
    own: &mut HashMap<String, Interface>,
) -> Result<(), String> {
    if state.contains_key(name) {
        return Ok(());
    }
    state.insert(name.to_owned(), false);
    let module = &modules[name];
    for import in &module.imports {
        if modules.contains_key(&import.name.text) {
            visit(
                compat,
                package,
                &import.name.text,
                modules,
                state,
                interfaces,
                own,
            )?;
        }
    }
    let canonical = canonicalize(compat, package, interfaces, module)
        .map_err(|errors| format!("{package} {name}: {errors:?}"))?;
    let iface = interface(&canonical);
    interfaces.insert(name.to_owned(), iface.clone());
    own.insert(name.to_owned(), iface);
    state.insert(name.to_owned(), true);
    Ok(())
}

/// The `.elm` files under `dir`, sorted; none if there is no `dir`, as in
/// the CLI tests' registry, whose packages hold only their `elm.json`.
fn elm_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "elm") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The interfaces a project of these direct dependencies imports from.
pub fn visible(packages: &PackageInterfaces, direct: &[Name]) -> HashMap<String, Interface> {
    direct
        .iter()
        .filter_map(|name| packages.get(name))
        .flat_map(|exposed| exposed.iter().map(|(m, i)| (m.clone(), i.clone())))
        .collect()
}

/// Each local module's naming problems, by path: every module the build
/// reached whose path isn't in `failed` and none of whose local imports
/// failed or was blocked, canonicalized after those imports.
pub fn canonicalize_graph(
    env: &Env,
    graph: &Graph,
    packages: &HashMap<String, Interface>,
    failed: &HashSet<PathBuf>,
) -> HashMap<PathBuf, Vec<Problem>> {
    let package = match env.owner() {
        Owner::Package(name) => name.to_string(),
        Owner::Application => "author/project".to_owned(),
    };
    let mut walk = Walk {
        env,
        graph,
        package,
        interfaces: packages.clone(),
        failed,
        state: HashMap::new(),
        problems: HashMap::new(),
    };
    let mut names: Vec<&ModuleName> = graph.modules.keys().collect();
    names.sort_by(|a, b| a.compare(b, env.compat()));
    for name in names {
        walk.local(name);
    }
    for root in &graph.roots {
        if let Root::Outside(Ok(module)) = root {
            walk.module(module);
        }
    }
    walk.problems
}

struct Walk<'g> {
    env: &'g Env,
    graph: &'g Graph,
    package: String,
    interfaces: HashMap<String, Interface>,
    failed: &'g HashSet<PathBuf>,
    /// Whether each module visited canonicalized cleanly.
    state: HashMap<ModuleName, bool>,
    problems: HashMap<PathBuf, Vec<Problem>>,
}

impl Walk<'_> {
    /// Whether the local module `name` is fine to import.
    fn local(&mut self, name: &ModuleName) -> bool {
        if let Some(&ok) = self.state.get(name) {
            return ok;
        }
        // A cycle is reported before canonicalization; mark it to stop.
        self.state.insert(name.clone(), false);
        let ok = match self.graph.modules.get(name) {
            Some(Status::Local(module)) => self.module(module),
            Some(Status::BadModule(_)) => false,
            _ => true,
        };
        self.state.insert(name.clone(), ok);
        ok
    }

    /// Canonicalizes `module` if it and its local imports are fine.
    fn module(&mut self, module: &Module) -> bool {
        if self.failed.contains(&module.path) {
            return false;
        }
        let mut blocked = false;
        for import in &module.imports {
            if matches!(
                self.graph.modules.get(&import.name),
                Some(Status::Local(_) | Status::BadModule(_))
            ) && !self.local(&import.name)
            {
                blocked = true;
            }
        }
        if blocked {
            return false;
        }
        let root = module.parse.syntax();
        let hir = hir::lower(&ast::Module::cast(root.clone()).expect("a module"));
        match canonicalize(self.env.compat(), &self.package, &self.interfaces, &hir) {
            Ok(canonical) => {
                self.interfaces
                    .insert(module.name.to_string(), interface(&canonical));
                true
            }
            Err(errors) => {
                let source = Source::new(&module.text);
                let problems = errors
                    .iter()
                    .map(|e| problem(e, &source, &root, self.env.compat()))
                    .collect();
                self.problems.insert(module.path.clone(), problems);
                false
            }
        }
    }
}

/// The report of a naming error, so far its title and region; the
/// message comes with plan 5.14.
pub fn problem(error: &Error, source: &Source, root: &SyntaxNode, compat: Compat) -> Problem {
    let (title, range) = title_and_range(error);
    let region = source.region(trim(root, range), compat);
    Problem::new(title, region, Doc::empty())
}

/// From the first token of `range` that isn't trivia: node ranges keep
/// the comments and spaces before them.
fn trim(root: &SyntaxNode, range: TextRange) -> TextRange {
    let Some(mut token) = root.token_at_offset(range.start()).right_biased() else {
        return range;
    };
    while token.kind().is_trivia() {
        match token.next_token() {
            Some(next) if next.text_range().start() < range.end() => token = next,
            _ => return range,
        }
    }
    let start = token.text_range().start().max(range.start());
    TextRange::new(start, range.end().max(start))
}

/// The title and range `toReport` gives each error.
fn title_and_range(error: &Error) -> (&'static str, TextRange) {
    match error {
        Error::AnnotationTooShort { range, .. } => ("BAD TYPE ANNOTATION", *range),
        Error::Ambiguous { range, .. } | Error::AmbiguousBinop { range, .. } => {
            ("AMBIGUOUS NAME", *range)
        }
        Error::BadArity {
            range,
            expected,
            actual,
            ..
        } => {
            let title = if actual < expected {
                "TOO FEW ARGS"
            } else {
                "TOO MANY ARGS"
            };
            (title, *range)
        }
        Error::Binop { range, .. } => ("INFIX PROBLEM", *range),
        Error::DuplicateDecl { second, .. }
        | Error::DuplicateType { second, .. }
        | Error::DuplicateCtor { second, .. }
        | Error::DuplicateBinop { second, .. }
        | Error::DuplicateField { second, .. }
        | Error::DuplicateAliasArg { second, .. }
        | Error::DuplicateUnionArg { second, .. }
        | Error::DuplicatePattern { second, .. } => ("NAME CLASH", *second),
        Error::EffectNotFound { range, .. } | Error::EffectFunctionNotFound { range, .. } => {
            ("EFFECT PROBLEM", *range)
        }
        Error::ExportDuplicate { second, .. } => ("REDUNDANT EXPORT", *second),
        Error::ExportNotFound { range, .. } => ("UNKNOWN EXPORT", *range),
        Error::ExportOpenAlias { range, .. } => ("BAD EXPORT", *range),
        Error::ImportCtorByName { range, .. }
        | Error::ImportOpenAlias { range, .. }
        | Error::ImportExposingNotFound { range, .. } => ("BAD IMPORT", *range),
        Error::NotFound { range, .. } => ("NAMING ERROR", *range),
        Error::NotFoundBinop { range, .. } => ("UNKNOWN OPERATOR", *range),
        Error::PatternHasRecordCtor { range, .. } => ("BAD PATTERN", *range),
        Error::PortPayloadInvalid { range, .. } => ("PORT ERROR", *range),
        Error::PortTypeInvalid { range, .. } => ("BAD PORT", *range),
        Error::RecursiveAlias { range, .. } => ("ALIAS PROBLEM", *range),
        Error::RecursiveDecl { range, .. } => ("CYCLIC DEFINITION", *range),
        Error::RecursiveLet { name, .. } => ("CYCLIC VALUE", name.range),
        Error::Shadowing { second, .. } => ("SHADOWING", *second),
        Error::TupleLargerThanThree { range } => ("BAD TUPLE", *range),
        Error::TypeVarsUnboundInUnion { range, rest, .. } => {
            let title = if rest.is_empty() {
                "UNBOUND TYPE VARIABLE"
            } else {
                "UNBOUND TYPE VARIABLES"
            };
            (title, *range)
        }
        Error::TypeVarsMessedUpInAlias {
            range,
            unused,
            unbound,
            ..
        } => {
            let title = match (unused.len(), unbound.len()) {
                (1, 0) => "UNUSED TYPE VARIABLE",
                (_, 0) => "UNUSED TYPE VARIABLES",
                (0, 1) => "UNBOUND TYPE VARIABLE",
                (0, _) => "UNBOUND TYPE VARIABLES",
                _ => "TYPE VARIABLE PROBLEMS",
            };
            (title, *range)
        }
    }
}
