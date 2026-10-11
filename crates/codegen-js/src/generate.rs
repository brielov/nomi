//! `Generate/JavaScript.hs`: a program's JavaScript. From each `main`,
//! every definition it reaches is written once, after what it needs,
//! visiting what each needs in the release's order of globals; kernels'
//! code comes first, then the rest, then `_Platform_export` with each
//! `main` under its module's path.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use nomi_base::Compat;
use nomi_compiler::canonical::Home;
use nomi_compiler::optimized::binary::{globals, homes, names};
use nomi_compiler::optimized::{Chunk, Def, EffectsType, Expr, Global, GlobalGraph, Main, Node};

use crate::builder::{self, Expr as Js, LValue, Stmt};
use crate::expression::{Gen, code_to_expr, code_to_stmt_list};
use crate::mode::Mode;
use crate::name::Name;

const FUNCTIONS: &str = include_str!("functions.js");

/// `generate`: the program of `mains`, each a module and its `main`,
/// from `graph`, which holds every definition they may reach.
pub fn generate(
    compat: Compat,
    mode: &Mode,
    graph: &GlobalGraph,
    mains: &[(Home, Main)],
) -> Vec<u8> {
    let mut mains: Vec<&(Home, Main)> = mains.iter().collect();
    mains.sort_by(|a, b| homes(compat, &a.0, &b.0));
    let mut state = State {
        cx: Gen { compat, mode },
        graph,
        kernels: Vec::new(),
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
    let mut out = b"(function(scope){\n'use strict';".to_vec();
    out.extend_from_slice(FUNCTIONS.as_bytes());
    out.extend_from_slice(perf_note(compat, mode).as_bytes());
    out.extend_from_slice(&state.kernels);
    out.extend_from_slice(state.builders.as_bytes());
    out.extend_from_slice(main_exports(&state.cx, &mains).as_bytes());
    out.extend_from_slice(b"}(this));");
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
        link(compat, "optimize")
    )
}

/// `makeNakedLink`.
fn link(compat: Compat, page: &str) -> String {
    format!("https://elm-lang.org/{compat}/{page}")
}

/// The traversal's state: the kernels' code and the rest's, each in the
/// order written, and the globals visited.
struct State<'a> {
    cx: Gen<'a>,
    graph: &'a GlobalGraph,
    kernels: Vec<u8>,
    builders: String,
    seen: HashSet<Global>,
}

impl State<'_> {
    /// `addGlobal`.
    fn add_global(&mut self, global: &Global) {
        if self.seen.insert(global.clone()) {
            self.add_global_help(global);
        }
    }

    /// `Set.foldl' addGlobal`: in the release's order of globals.
    fn add_deps(&mut self, deps: &BTreeSet<Global>) {
        let mut deps: Vec<&Global> = deps.iter().collect();
        deps.sort_by(|a, b| globals(self.cx.compat, a, b));
        for dep in deps {
            self.add_global(dep);
        }
    }

    fn add_stmt(&mut self, stmt: &Stmt) {
        self.builders.push_str(&builder::stmt(stmt));
    }

    /// `addGlobalHelp`.
    fn add_global_help(&mut self, global: &Global) {
        let graph = self.graph;
        let node = graph.nodes.get(global).unwrap_or_else(|| {
            panic!(
                "{}.{} in {} isn't in the graph",
                global.home.module, global.home.package, global.name
            )
        });
        let cx = &self.cx;
        match node {
            Node::Define(expr, deps) => {
                self.add_deps(deps);
                let stmt = var(global, code_to_expr(self.cx.generate(expr)));
                self.add_stmt(&stmt);
            }
            Node::DefineTailFunc(args, body, deps) => {
                self.add_deps(deps);
                let code = self.cx.tail_def(&global.name, args, body);
                self.add_stmt(&var(global, code_to_expr(code)));
            }
            Node::Ctor(index, arity) => {
                let code = cx.ctor(global, *index, *arity as usize);
                self.add_stmt(&var(global, code_to_expr(code)));
            }
            Node::Link(linked) => self.add_global(linked),
            Node::Cycle(names_, values, functions, deps) => {
                self.add_deps(deps);
                let stmt = self.cycle(&global.home, names_, values, functions);
                self.add_stmt(&stmt);
            }
            Node::Manager(effects) => self.manager(&global.home, *effects),
            Node::Kernel(chunks, deps) => {
                if global.home.module == "Debugger" && !cx.mode.is_debug() {
                    return;
                }
                self.add_deps(deps);
                let code = self.kernel(chunks);
                self.kernels.extend(code);
            }
            Node::Enum(index) => {
                let expr = match cx.mode {
                    Mode::Dev { .. } => code_to_expr(cx.ctor(global, *index, 0)),
                    Mode::Prod(_) => Js::Int(*index as i64),
                };
                self.add_stmt(&var(global, expr));
            }
            Node::Box => {
                let identity = Global {
                    home: basics(),
                    name: "identity".to_owned(),
                };
                self.add_global(&identity);
                let expr = match self.cx.mode {
                    Mode::Dev { .. } => code_to_expr(self.cx.ctor(global, 0, 1)),
                    Mode::Prod(_) => Js::Ref(Name::from_global(&identity.home, &identity.name)),
                };
                self.add_stmt(&var(global, expr));
            }
            Node::PortIncoming(decoder, deps) => {
                self.add_deps(deps);
                let stmt = self.port(global, "incomingPort", decoder);
                self.add_stmt(&stmt);
            }
            Node::PortOutgoing(encoder, deps) => {
                self.add_deps(deps);
                let stmt = self.port(global, "outgoingPort", encoder);
                self.add_stmt(&stmt);
            }
        }
    }

    /// `generateCycle`: the functions, then each value as a function that
    /// computes it, then each computed and its function replaced by one
    /// returning it; in development, a value needing itself throws.
    fn cycle(
        &self,
        home: &Home,
        names_: &[String],
        values: &[(String, Expr)],
        functions: &[Def],
    ) -> Stmt {
        let cx = &self.cx;
        let functions = (functions.iter())
            .map(|def| match def {
                Def::Def(name, expr) => Stmt::Var(
                    Name::from_global(home, name),
                    code_to_expr(cx.generate(expr)),
                ),
                Def::TailDef(name, args, expr) => Stmt::Var(
                    Name::from_global(home, name),
                    code_to_expr(cx.tail_def(name, args, expr)),
                ),
            })
            .collect();
        let safe = (values.iter())
            .map(|(name, expr)| {
                Stmt::Function(
                    Name::from_cycle(home, name),
                    Vec::new(),
                    code_to_stmt_list(cx.generate(expr)),
                )
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
                        Box::new(Js::Function(
                            None,
                            Vec::new(),
                            vec![Stmt::Return(Js::Ref(real))],
                        )),
                    )),
                ])
            })
            .collect();
        let real = if real.is_empty() {
            Stmt::Empty
        } else {
            match cx.mode {
                Mode::Prod(_) => Stmt::Block(real),
                Mode::Dev { .. } => Stmt::Try(
                    Box::new(Stmt::Block(real)),
                    Name::dollar(),
                    Box::new(Stmt::Throw(Js::String(format!(
                        "Some top-level definitions from `{}` are causing infinite \
                         recursion:\\n{}\\n\\nThese errors are very tricky, so read {} to \
                         learn how to fix it!",
                        home.module,
                        draw_cycle(names_),
                        link(cx.compat, "bad-recursion")
                    )))),
                ),
            }
        };
        Stmt::Block(vec![Stmt::Block(functions), Stmt::Block(safe), real])
    }

    /// `generateKernel`.
    fn kernel(&self, chunks: &[Chunk]) -> Vec<u8> {
        let prod = matches!(self.cx.mode, Mode::Prod(_));
        let mut out = Vec::new();
        for chunk in chunks {
            match chunk {
                Chunk::Js(js) => out.extend_from_slice(js),
                Chunk::ElmVar(home, name) => {
                    out.extend_from_slice(Name::from_global(home, name).as_str().as_bytes());
                }
                Chunk::JsVar(home, name) => {
                    out.extend_from_slice(Name::from_kernel(home, name).as_str().as_bytes());
                }
                Chunk::ElmField(name) => {
                    out.extend_from_slice(self.cx.field(name).as_str().as_bytes());
                }
                Chunk::JsField(int) => {
                    out.extend_from_slice(Name::from_int(*int as usize).as_str().as_bytes());
                }
                Chunk::JsEnum(int) => out.extend_from_slice(int.to_string().as_bytes()),
                Chunk::Debug if prod => out.extend_from_slice(b"_UNUSED"),
                Chunk::Prod if !prod => out.extend_from_slice(b"_UNUSED"),
                Chunk::Debug | Chunk::Prod => {}
            }
        }
        out
    }

    /// `generatePort`.
    fn port(&self, global: &Global, make: &str, converter: &Expr) -> Stmt {
        let call = Js::Call(
            Box::new(Js::Ref(Name::from_kernel("Platform", make))),
            vec![
                Js::String(global.name.clone()),
                code_to_expr(self.cx.generate(converter)),
            ],
        );
        var(global, call)
    }

    /// `generateManager`.
    fn manager(&mut self, home: &Home, effects: EffectsType) {
        let (names_, leaves): (&[&str], &[&str]) = match effects {
            EffectsType::Cmd => (&["init", "onEffects", "onSelfMsg", "cmdMap"], &["command"]),
            EffectsType::Sub => (
                &["init", "onEffects", "onSelfMsg", "subMap"],
                &["subscription"],
            ),
            EffectsType::Fx => (
                &["init", "onEffects", "onSelfMsg", "cmdMap", "subMap"],
                &["command", "subscription"],
            ),
        };
        for name in names_ {
            self.add_global(&Global {
                home: home.clone(),
                name: (*name).to_owned(),
            });
        }
        let mut args: Vec<Js> = (names_.iter())
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

fn basics() -> Home {
    Home {
        package: "elm/core".into(),
        module: "Basics".into(),
    }
}

/// `var`.
fn var(global: &Global, expr: Js) -> Stmt {
    Stmt::Var(Name::from_global(&global.home, &global.name), expr)
}

/// `drawCycle`, its line breaks escaped.
fn draw_cycle(names_: &[String]) -> String {
    let lines: Vec<String> = (names_.iter())
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

/// `toMainExports`.
fn main_exports(cx: &Gen, mains: &[&(Home, Main)]) -> String {
    let mut trie = Trie::default();
    for main in mains {
        let mut at = &mut trie;
        for segment in main.0.module.split('.') {
            at = at.subs.entry(segment).or_default();
        }
        at.main = Some(main);
    }
    format!(
        "{}({});",
        Name::from_kernel("Platform", "export"),
        exports(cx, &trie)
    )
}

/// `generateExports`: the subtrees by segment in the release's order of
/// names.
fn exports(cx: &Gen, trie: &Trie) -> String {
    let mut out = String::from("{");
    let mut items = Vec::new();
    if let Some((home, main)) = trie.main {
        items.push(format!("'init':{}", builder::expr(&cx.main(home, main))));
    }
    let mut subs: Vec<(&&str, &Trie)> = trie.subs.iter().collect();
    subs.sort_by(|a, b| names(cx.compat, a.0, b.0));
    for (name, sub) in subs {
        items.push(format!("'{name}':{}", exports(cx, sub)));
    }
    out.push_str(&items.join(","));
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV: Mode = Mode::Dev { debug: None };

    fn home(module: &str) -> Home {
        Home {
            package: "author/project".into(),
            module: module.into(),
        }
    }

    fn global(module: &str, name: &str) -> Global {
        Global {
            home: home(module),
            name: name.to_owned(),
        }
    }

    /// The program's code past the helpers and the note.
    fn program(compat: Compat, mode: &Mode, graph: &GlobalGraph, mains: &[(Home, Main)]) -> String {
        let js = String::from_utf8(generate(compat, mode, graph, mains)).unwrap();
        let start = js
            .find("assets.');")
            .map_or(0, |at| at + "assets.');".len());
        js[start..].to_owned()
    }

    #[test]
    fn mains_are_exported_by_module_path_in_the_release_order() {
        let mut graph = GlobalGraph::default();
        let mut mains = Vec::new();
        for module in ["Page.About", "Page.Home"] {
            let define = Node::Define(Expr::Int(0), BTreeSet::new());
            graph.nodes.insert(global(module, "main"), define);
            mains.push((home(module), Main::Static));
        }
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
    fn a_cycle_of_values_throws_in_development() {
        let mut graph = GlobalGraph::default();
        let cycle = Node::Cycle(
            vec!["a".to_owned(), "b".to_owned()],
            vec![("a".to_owned(), Expr::VarCycle(home("Main"), "b".to_owned()))],
            Vec::new(),
            BTreeSet::new(),
        );
        graph.nodes.insert(global("Main", "a"), cycle);
        let main = Node::Define(
            Expr::VarGlobal(global("Main", "a")),
            [global("Main", "a")].into(),
        );
        graph.nodes.insert(global("Main", "main"), main);
        let mains = [(home("Main"), Main::Static)];
        let js = program(Compat::V0_19_1, &DEV, &graph, &mains);
        let expected = "function $author$project$Main$cyclic$a() {\n\treturn $author$project$Main$cyclic$b();\n}\ntry {\n\tvar $author$project$Main$a = $author$project$Main$cyclic$a();\n\t$author$project$Main$cyclic$a = function () {\n\t\treturn $author$project$Main$a;\n\t};\n} catch ($) {\n\tthrow 'Some top-level definitions from `Main` are causing infinite recursion:\\n\\n  ┌─────┐\\n  │    a\\n  │     ↓\\n  │    b\\n  └─────┘\\n\\nThese errors are very tricky, so read https://elm-lang.org/0.19.1/bad-recursion to learn how to fix it!';}\nvar $author$project$Main$main = $author$project$Main$a;\n";
        assert!(js.starts_with(expected), "{js}");
        let prod = Mode::Prod(BTreeMap::new());
        assert!(!program(Compat::V0_19_1, &prod, &graph, &mains).contains("try"));
    }
}
