//! `Generate/JavaScript.hs`: a program's JavaScript. From each `main`,
//! every definition it reaches is written once, after what it needs,
//! visiting what each needs in the release's order of globals; kernels'
//! code comes first, then the rest, then `_Platform_export` with each
//! `main` under its module's path.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;

use nomi_base::diagnostic::render::naked_link;
use nomi_base::{Compat, Platform};
use nomi_compiler::canonical::Home;
use nomi_compiler::optimized::binary::homes;
use nomi_compiler::optimized::{Chunk, Def, EffectsType, Expr, Global, GlobalGraph, Main, Node};

use crate::builder::{self, Expr as Js, LValue, Stmt};
use crate::expression::{Generator, code_to_expr, code_to_stmt_list};
use crate::ffi::Startup;
use crate::mode::Mode;
use crate::name::Name;
use crate::platform::{Format, Target, probe, shim};
use crate::reach::{deps_of, identity, manager_functions};
use crate::runtime;

const FUNCTIONS: &str = include_str!("functions.js");

/// `generate`: the program of `mains`, each a module and its `main`,
/// from `graph`, which holds every definition they may reach, for
/// `target`. A Node program carries the runtime its kernels run on, and
/// a Node module of one `main` is an entry, which probes Node for what
/// the program needs before its kernels load, and starts it. A module's
/// FFI module is imported first and is its scope's `ffi`, where its
/// kernel finds it, once the module has checked what `startup` asks.
pub fn generate(
    compat: Compat,
    mode: &Mode,
    target: &Target,
    startup: &Startup,
    graph: &GlobalGraph,
    mains: &[(Home, Main)],
) -> Vec<u8> {
    let mut mains: Vec<&(Home, Main)> = mains.iter().collect();
    mains.sort_by(|a, b| homes(compat, &a.0, &b.0));
    let mut kernels = match (target.format, &target.ffi) {
        (Format::Iife, _) => Vec::new(),
        (Format::EsModule, None) => b"const scope = {};\n".to_vec(),
        (Format::EsModule, Some(ffi)) => format!(
            "import * as __nomi_ffi from '{}';\n{}const scope = {{ ffi: __nomi_ffi }};\n",
            ffi.specifier,
            startup.js("__nomi_ffi")
        )
        .into_bytes(),
    };
    kernels.extend_from_slice(b"(function(scope){\n'use strict';");
    kernels.extend_from_slice(FUNCTIONS.as_bytes());
    kernels.extend_from_slice(perf_note(compat, mode).as_bytes());
    if target.platform == Platform::Node {
        kernels.extend_from_slice(runtime::prologue().as_bytes());
    }
    let entry = matches!(
        (target.platform, target.format, mains.as_slice()),
        (Platform::Node, Format::EsModule, [_])
    );
    let runtime_end = kernels.len();
    let mut state = State {
        generator: Generator { compat, mode },
        graph,
        kernels,
        builders: String::new(),
        seen: HashSet::new(),
    };
    // `Map.foldrWithKey`: the last module first.
    for (home, _) in mains.iter().rev() {
        state.add_global(&Global {
            home: home.clone(),
            name: "main".to_owned(),
        });
    }
    main_exports(&mut state.builders, &state.generator, &mains);
    let mut out = state.kernels;
    if entry {
        let kernels = (state.seen.iter())
            .filter(|global| global.home.package == "elm/kernel")
            .map(|global| global.home.module.as_str());
        let probe = probe(kernels);
        out.splice(runtime_end..runtime_end, probe.bytes());
    }
    out.extend_from_slice(state.builders.as_bytes());
    if let (true, [(home, _)]) = (entry, mains.as_slice()) {
        out.extend_from_slice(shim(&home.module).as_bytes());
    }
    let end: &[u8] = match target.format {
        Format::Iife => b"}(this));",
        Format::EsModule => b"}(scope));\nexport const Elm = scope.Elm;\n",
    };
    out.extend_from_slice(end);
    out
}

/// `perfNote`.
fn perf_note(compat: Compat, mode: &Mode) -> String {
    let which = match mode {
        Mode::Prod(_) => return String::new(),
        Mode::Dev { debug: None } => "DEV",
        Mode::Dev { debug: Some(_) } => "DEBUG",
    };
    format!(
        "console.warn('Compiled in {which} mode. Follow the advice at {} for better \
         performance and smaller assets.');",
        naked_link(compat, "optimize")
    )
}

/// The traversal's state: the program's code, each definition written as
/// it is reached, and the globals visited.
struct State<'a> {
    generator: Generator<'a>,
    graph: &'a GlobalGraph,
    /// The program's start, then the kernels' code.
    kernels: Vec<u8>,
    /// The rest's code, which follows the kernels'.
    builders: String,
    seen: HashSet<&'a Global>,
}

impl State<'_> {
    /// `addGlobal`.
    fn add_global(&mut self, global: &Global) {
        if self.seen.contains(global) {
            return;
        }
        let (global, node) = self.graph.nodes.get_key_value(global).unwrap_or_else(|| {
            panic!(
                "{} {}.{} isn't in the graph, which holds everything a main reaches",
                global.home.package, global.home.module, global.name
            )
        });
        self.seen.insert(global);
        self.add_global_help(global, node);
    }

    fn add_stmt(&mut self, stmt: &Stmt) {
        builder::stmt(&mut self.builders, stmt);
    }

    /// `addGlobalHelp`: what the node needs, then the node.
    fn add_global_help(&mut self, global: &Global, node: &Node) {
        let Some(deps) = deps_of(self.generator.mode.is_debug(), global, node) else {
            return;
        };
        let compat = self.generator.compat;
        deps.each(compat, &global.home, |dep| self.add_global(dep));
        match node {
            Node::Define(expr, _) => {
                let stmt = var(global, self.generator.js(expr));
                self.add_stmt(&stmt);
            }
            Node::DefineTailFunc { params, body, .. } => {
                let code = self.generator.tail_def(&global.name, params, body);
                self.add_stmt(&var(global, code_to_expr(code)));
            }
            Node::Ctor(index, arity) => {
                let code = self.generator.ctor(global, *index, *arity as usize);
                self.add_stmt(&var(global, code_to_expr(code)));
            }
            Node::Link(_) => {}
            Node::Cycle {
                names,
                values,
                functions,
                ..
            } => {
                let stmt = self.cycle(&global.home, names, values, functions);
                self.add_stmt(&stmt);
            }
            Node::Manager(effects) => self.manager(&global.home, *effects),
            Node::Kernel(chunks, _) => self.kernel(chunks),
            Node::Enum(index) => {
                let expr = match self.generator.mode {
                    Mode::Dev { .. } => code_to_expr(self.generator.ctor(global, *index, 0)),
                    Mode::Prod(_) => Js::Int(*index as i64),
                };
                self.add_stmt(&var(global, expr));
            }
            Node::Box => {
                let identity = identity();
                let expr = match self.generator.mode {
                    Mode::Dev { .. } => code_to_expr(self.generator.ctor(global, 0, 1)),
                    Mode::Prod(_) => Js::Ref(Name::from_global(&identity.home, &identity.name)),
                };
                self.add_stmt(&var(global, expr));
            }
            Node::PortIncoming(decoder, _) => self.port(global, "incomingPort", decoder),
            Node::PortOutgoing(encoder, _) => self.port(global, "outgoingPort", encoder),
        }
    }

    /// `generateCycle`: the functions, then each value as a function that
    /// computes it, then each computed and its function replaced by one
    /// returning it; in development, a value needing itself throws.
    fn cycle(
        &self,
        home: &Home,
        names: &[String],
        values: &[(String, Expr)],
        functions: &[Def],
    ) -> Stmt {
        let generator = &self.generator;
        let functions = (functions.iter())
            .map(|def| {
                let (name, value) = generator.def(def);
                Stmt::Var(Name::from_global(home, name), value)
            })
            .collect();
        let safe = (values.iter())
            .map(|(name, expr)| Stmt::Function {
                name: Name::from_cycle(home, name),
                params: Vec::new(),
                body: code_to_stmt_list(generator.generate(expr)),
            })
            .collect();
        let real: Vec<Stmt> = (values.iter())
            .map(|(name, _)| {
                let safe = Name::from_cycle(home, name);
                let real = Name::from_global(home, name);
                Stmt::Block(vec![
                    Stmt::Var(
                        real.clone(),
                        Js::Call(Box::new(Js::Ref(safe.clone())), Vec::new()),
                    ),
                    Stmt::Expr(Js::Assign(
                        LValue::Ref(safe),
                        Box::new(Js::Function {
                            params: Vec::new(),
                            body: vec![Stmt::Return(Js::Ref(real))],
                        }),
                    )),
                ])
            })
            .collect();
        let real = if real.is_empty() {
            Stmt::Empty
        } else {
            match generator.mode {
                Mode::Prod(_) => Stmt::Block(real),
                Mode::Dev { .. } => Stmt::Try {
                    body: Box::new(Stmt::Block(real)),
                    error: Name::dollar(),
                    catch: Box::new(Stmt::Throw(Js::String(format!(
                        "Some top-level definitions from `{}` are causing infinite \
                         recursion:\\n{}\\n\\nThese errors are very tricky, so read {} to \
                         learn how to fix it!",
                        home.module,
                        draw_cycle(names),
                        naked_link(generator.compat, "bad-recursion")
                    )))),
                },
            }
        };
        Stmt::Block(vec![Stmt::Block(functions), Stmt::Block(safe), real])
    }

    /// `generateKernel`, onto the kernels' code.
    fn kernel(&mut self, chunks: &[Chunk]) {
        let prod = matches!(self.generator.mode, Mode::Prod(_));
        let out = &mut self.kernels;
        for chunk in chunks {
            match chunk {
                Chunk::Js(js) => out.extend_from_slice(js),
                Chunk::ElmVar(home, name) => {
                    out.extend_from_slice(Name::from_global(home, name).as_str().as_bytes());
                }
                Chunk::JsVar { module, name } => {
                    out.extend_from_slice(Name::from_kernel(module, name).as_str().as_bytes());
                }
                Chunk::ElmField(name) => {
                    out.extend_from_slice(self.generator.field(name).as_str().as_bytes());
                }
                Chunk::JsField(int) => {
                    out.extend_from_slice(Name::from_int(*int as usize).as_str().as_bytes());
                }
                Chunk::JsEnum(int) => _ = write!(out, "{int}"),
                Chunk::Debug if prod => out.extend_from_slice(b"_UNUSED"),
                Chunk::Prod if !prod => out.extend_from_slice(b"_UNUSED"),
                Chunk::Debug | Chunk::Prod => {}
            }
        }
    }

    /// `generatePort`: `make` is the kernel's function for its direction.
    fn port(&mut self, global: &Global, make: &str, converter: &Expr) {
        let call = Js::Call(
            Box::new(Js::Ref(Name::from_kernel("Platform", make))),
            vec![
                Js::String(global.name.clone()),
                self.generator.js(converter),
            ],
        );
        self.add_stmt(&var(global, call));
    }

    /// `generateManager`.
    fn manager(&mut self, home: &Home, effects: EffectsType) {
        let names = manager_functions(effects);
        let leaves: &[&str] = match effects {
            EffectsType::Cmd => &["command"],
            EffectsType::Sub => &["subscription"],
            EffectsType::Fx => &["command", "subscription"],
        };
        let mut args: Vec<Js> = (names.iter())
            .map(|name| Js::Ref(Name::from_global(home, name)))
            .collect();
        if effects == EffectsType::Sub {
            args.insert(3, Js::Int(0));
        }
        let platform = |name| Box::new(Js::Ref(Name::from_kernel("Platform", name)));
        let manager = LValue::Bracket(
            platform("effectManagers"),
            Box::new(Js::String(home.module.to_string())),
        );
        let create = Stmt::Expr(Js::Assign(
            manager,
            Box::new(Js::Call(platform("createManager"), args)),
        ));
        let mut stmts = vec![create];
        stmts.extend(leaves.iter().map(|leaf| {
            Stmt::Var(
                Name::from_global(home, leaf),
                Js::Call(platform("leaf"), vec![Js::String(home.module.to_string())]),
            )
        }));
        self.add_stmt(&Stmt::Block(stmts));
    }
}

/// `var`.
fn var(global: &Global, expr: Js) -> Stmt {
    Stmt::Var(Name::from_global(&global.home, &global.name), expr)
}

/// `drawCycle`, its line breaks escaped.
fn draw_cycle(names: &[String]) -> String {
    let lines: Vec<String> = (names.iter())
        .map(|name| format!("\\n  │    {name}"))
        .collect();
    format!("\\n  ┌─────┐{}\\n  └─────┘", lines.join("\\n  │     ↓"))
}

/// A module path's tree of `main`s (`Trie`).
#[derive(Default)]
struct Trie<'a> {
    main: Option<&'a (Home, Main)>,
    subs: BTreeMap<&'a str, Trie<'a>>,
}

/// `toMainExports`, onto `out`.
fn main_exports(out: &mut String, generator: &Generator, mains: &[&(Home, Main)]) {
    let mut trie = Trie::default();
    for main in mains {
        let mut at = &mut trie;
        for segment in main.0.module.split('.') {
            at = at.subs.entry(segment).or_default();
        }
        at.main = Some(main);
    }
    out.push_str(Name::from_kernel("Platform", "export").as_str());
    out.push('(');
    exports(out, generator, &trie);
    out.push_str(");");
}

/// `generateExports`: the subtrees by segment in the release's order of
/// names, except that upstream folds all but the first onto the end it
/// has so far, each before the last, so they come out reversed.
fn exports(out: &mut String, generator: &Generator, trie: &Trie) {
    out.push('{');
    if let Some((home, main)) = trie.main {
        out.push_str("'init':");
        builder::expr(out, &generator.main(home, main));
    }
    let mut subs = generator.compat.in_name_order(&trie.subs);
    if subs.len() > 1 {
        subs[1..].reverse();
    }
    for (i, (name, sub)) in subs.into_iter().enumerate() {
        if i > 0 || trie.main.is_some() {
            out.push(',');
        }
        out.push('\'');
        out.push_str(name);
        out.push_str("':");
        exports(out, generator, sub);
    }
    out.push('}');
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::ffi::{Exports, Ffi};

    const DEV: Mode = Mode::Dev { debug: None };

    /// Nothing to check of an FFI module.
    const NONE: Startup = Startup {
        names: BTreeMap::new(),
    };

    fn global(module: &str, name: &str) -> Global {
        Global {
            home: Home::author_project(module),
            name: name.to_owned(),
        }
    }

    /// A graph of `modules`' static `main`s, and the mains.
    fn static_mains(modules: &[&str]) -> (GlobalGraph, Vec<(Home, Main)>) {
        let mut graph = GlobalGraph::default();
        let mut mains = Vec::new();
        for module in modules {
            let define = Node::Define(Expr::Int(0), BTreeSet::new());
            graph.nodes.insert(global(module, "main"), define);
            mains.push((Home::author_project(module), Main::Static));
        }
        (graph, mains)
    }

    /// The program's code past the helpers and the note.
    fn program(compat: Compat, mode: &Mode, graph: &GlobalGraph, mains: &[(Home, Main)]) -> String {
        let js = String::from_utf8(generate(
            compat,
            mode,
            &Target::default(),
            &NONE,
            graph,
            mains,
        ))
        .unwrap();
        let start = js
            .find("assets.');")
            .map_or(0, |at| at + "assets.');".len());
        js[start..].to_owned()
    }

    #[test]
    fn mains_are_exported_by_module_path_in_the_release_order() {
        let (graph, mains) = static_mains(&["Page.About", "Page.Home"]);
        let exports = |compat| {
            let js = program(compat, &DEV, &graph, &mains);
            js[js.find("_Platform_export").unwrap()..].to_owned()
        };
        let init = |module: &str| {
            format!("{{'init':_VirtualDom_init($author$project${module}$main)(0)(0)}}")
        };
        assert_eq!(
            exports(Compat::V0_19_1),
            format!(
                "_Platform_export({{'Page':{{'About':{},'Home':{}}}}});}}(this));",
                init("Page$About"),
                init("Page$Home")
            )
        );
        assert_eq!(
            exports(Compat::V0_19_3),
            format!(
                "_Platform_export({{'Page':{{'Home':{},'About':{}}}}});}}(this));",
                init("Page$Home"),
                init("Page$About")
            )
        );
    }

    #[test]
    fn mains_after_the_first_are_exported_in_reverse() {
        let (graph, mains) = static_mains(&["Page", "Post", "Posts", "Tag"]);
        let js = program(Compat::V0_19_1, &DEV, &graph, &mains);
        let exports = &js[js.find("_Platform_export").unwrap()..];
        let order: Vec<&str> = ["'Page'", "'Post'", "'Posts'", "'Tag'"]
            .into_iter()
            .map(|name| (exports.find(name).unwrap(), name))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect();
        // elm's own output for elmstatic's four layouts.
        assert_eq!(order, ["'Page'", "'Tag'", "'Posts'", "'Post'"]);
    }

    #[test]
    fn a_module_wraps_the_same_function_and_exports_elm() {
        let (graph, mains) = static_mains(&["Main"]);
        let iife = generate(
            Compat::V0_19_1,
            &DEV,
            &Target::default(),
            &NONE,
            &graph,
            &mains,
        );
        let module = Target {
            format: Format::EsModule,
            ..Target::default()
        };
        let module = generate(Compat::V0_19_1, &DEV, &module, &NONE, &graph, &mains);
        let iife = String::from_utf8(iife).unwrap();
        let body = iife.strip_suffix("}(this));").unwrap();
        assert_eq!(
            String::from_utf8(module).unwrap(),
            format!("const scope = {{}};\n{body}}}(scope));\nexport const Elm = scope.Elm;\n")
        );
    }

    #[test]
    fn a_node_module_of_one_main_starts_it_and_of_more_starts_none() {
        let browser = Target {
            format: Format::EsModule,
            ..Target::default()
        };
        let node = Target {
            platform: Platform::Node,
            ..browser.clone()
        };
        let built = |target: &Target, modules: &[&str]| {
            let (graph, mains) = static_mains(modules);
            let js = generate(Compat::V0_19_1, &DEV, target, &NONE, &graph, &mains);
            String::from_utf8(js).unwrap()
        };
        // Node's is the browser's with the runtime after the warning, and
        // an entry's probe after the runtime.
        let note = perf_note(Compat::V0_19_1, &DEV);
        let with = |module: String, after: &str| {
            module.replacen(&note, &format!("{note}{}{after}", runtime::prologue()), 1)
        };
        let with_runtime = |module: String| with(module, "");
        let module = with(built(&browser, &["Page.Home"]), &probe([]));
        let (body, end) = module.split_at(module.rfind("}(scope));").unwrap());
        assert_eq!(
            built(&node, &["Page.Home"]),
            format!("{body}{}{end}", shim("Page.Home"))
        );
        assert_eq!(
            built(&node, &["Main", "Other"]),
            with_runtime(built(&browser, &["Main", "Other"]))
        );
        // A module with an FFI module imports it, and its scope holds it.
        let ffi = Target {
            ffi: Some(Ffi {
                specifier: "../src/ffi.mjs".to_owned(),
                exports: Exports::default(),
            }),
            ..node
        };
        let plain = built(&node, &["Main"]);
        assert_eq!(
            built(&ffi, &["Main"]),
            plain.replacen(
                "const scope = {};\n",
                "import * as __nomi_ffi from '../src/ffi.mjs';\n\
                 const scope = { ffi: __nomi_ffi };\n",
                1
            )
        );
    }

    #[test]
    fn a_cycle_of_values_throws_in_development() {
        let mut graph = GlobalGraph::default();
        let cycle = Node::Cycle {
            names: vec!["a".to_owned(), "b".to_owned()],
            values: vec![(
                "a".to_owned(),
                Expr::VarCycle(Home::author_project("Main"), "b".to_owned()),
            )],
            functions: Vec::new(),
            deps: BTreeSet::new(),
        };
        graph.nodes.insert(global("Main", "a"), cycle);
        let main = Node::Define(
            Expr::VarGlobal(global("Main", "a")),
            [global("Main", "a")].into(),
        );
        graph.nodes.insert(global("Main", "main"), main);
        let mains = [(Home::author_project("Main"), Main::Static)];
        let js = program(Compat::V0_19_1, &DEV, &graph, &mains);
        let expected = "function $author$project$Main$cyclic$a() {\n\treturn $author$project$Main$cyclic$b();\n}\ntry {\n\tvar $author$project$Main$a = $author$project$Main$cyclic$a();\n\t$author$project$Main$cyclic$a = function () {\n\t\treturn $author$project$Main$a;\n\t};\n} catch ($) {\n\tthrow 'Some top-level definitions from `Main` are causing infinite recursion:\\n\\n  ┌─────┐\\n  │    a\\n  │     ↓\\n  │    b\\n  └─────┘\\n\\nThese errors are very tricky, so read https://elm-lang.org/0.19.1/bad-recursion to learn how to fix it!';}\nvar $author$project$Main$main = $author$project$Main$a;\n";
        assert!(js.starts_with(expected), "{js}");
        let prod = Mode::Prod(BTreeMap::new());
        assert!(!program(Compat::V0_19_1, &prod, &graph, &mains).contains("try"));
    }
}
