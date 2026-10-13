//! The optimizer (`Optimize/Module.hs`): a module's canonical AST, with
//! its top-level annotations, made into the graph code generation reads.
//! Each top-level definition is optimized on its own, its generated names
//! counted from `_v0`, and a group of mutually recursive ones together.

mod case;
mod expression;
mod literal;
mod names;
mod port;

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::diagnostic::code::Region;
use nomi_base::{Compat, TextRange};

use crate::canonical::{
    Annotation, CtorOpts, Decl, Def as CDef, Effects, Home, MainKind, Manager, PortDirection, Type,
};
use crate::canonicalize::module::Canonical;
use crate::optimized::{EffectsType, Expr, Global, LocalGraph, Main, Node};
use expression::{Context, Cycle};
use names::Tracker;

/// `optimize`: the graph of `module`, whose top-level values have
/// `annotations`, with `region` giving an expression's rows and columns.
pub fn module(
    compat: Compat,
    annotations: &BTreeMap<String, Annotation>,
    module: &Canonical,
    region: &dyn Fn(TextRange) -> Region,
) -> LocalGraph {
    let cx = Context {
        compat,
        home: &module.home,
        region,
    };
    let mut graph = LocalGraph::default();
    let home = &module.home;
    for (name, alias) in &module.aliases {
        add_alias(compat, home, name, &alias.ty, &mut graph);
    }
    for union in module.unions.values() {
        for ctor in &union.ctors {
            let node = match union.opts {
                CtorOpts::Normal => Node::Ctor(ctor.index, ctor.args.len() as i64),
                CtorOpts::Unbox => Node::Box,
                CtorOpts::Enum => Node::Enum(ctor.index),
            };
            graph.nodes.insert(global(home, &ctor.name), node);
        }
    }
    add_effects(compat, home, &module.effects, &mut graph);
    for decl in &module.decls {
        match decl {
            Decl::Declare(def) => add_def(&cx, annotations, def, &mut graph),
            Decl::DeclareRec(def, defs) => {
                let all: Vec<&CDef> = std::iter::once(def).chain(defs).collect();
                add_rec_defs(&cx, &all, &mut graph);
            }
        }
    }
    graph
}

fn global(home: &Home, name: &str) -> Global {
    Global {
        home: home.clone(),
        name: name.to_owned(),
    }
}

/// `addAlias`: a record alias without an extension is a constructor
/// taking its fields in their written order.
fn add_alias(compat: Compat, home: &Home, name: &str, ty: &Type, graph: &mut LocalGraph) {
    let Type::Record(fields, None) = ty else {
        return;
    };
    let mut ordered: Vec<(&String, u16)> = fields.iter().map(|(n, f)| (n, f.order)).collect();
    ordered.sort_by(|a, b| compat.compare_names(a.0, b.0));
    ordered.sort_by_key(|(_, order)| *order);
    let function = Expr::Function(
        ordered.iter().map(|(n, _)| (*n).clone()).collect(),
        Box::new(Expr::Record(
            fields
                .keys()
                .map(|field| (field.clone(), Expr::VarLocal(field.clone())))
                .collect(),
        )),
    );
    graph
        .nodes
        .insert(global(home, name), Node::Define(function, BTreeSet::new()));
    graph.count_fields(fields.keys().map(|field| (field.clone(), 1)));
}

/// `addEffects`: each port's encoder or decoder, or a manager's nodes.
fn add_effects(compat: Compat, home: &Home, effects: &Effects, graph: &mut LocalGraph) {
    let manager = match effects {
        Effects::NoEffects => return,
        Effects::Ports(ports) => {
            for (name, port) in ports {
                let mut tracker = Tracker::new(compat);
                let node = match port.direction {
                    PortDirection::Incoming => {
                        let decoder = port::to_decoder(compat, &mut tracker, &port.payload);
                        Node::PortIncoming(decoder, std::mem::take(&mut tracker.deps))
                    }
                    PortDirection::Outgoing => {
                        let encoder = port::to_encoder(compat, &mut tracker, &port.payload);
                        Node::PortOutgoing(encoder, std::mem::take(&mut tracker.deps))
                    }
                };
                add_to_graph(graph, global(home, name), node, tracker.fields);
            }
            return;
        }
        Effects::Manager { manager, .. } => manager,
    };
    let fx = global(home, "$fx$");
    let link = || Node::Link(fx.clone());
    let kind = match manager {
        Manager::Cmd(_) => {
            graph.nodes.insert(global(home, "command"), link());
            EffectsType::Cmd
        }
        Manager::Sub(_) => {
            graph.nodes.insert(global(home, "subscription"), link());
            EffectsType::Sub
        }
        Manager::Fx { .. } => {
            graph.nodes.insert(global(home, "command"), link());
            graph.nodes.insert(global(home, "subscription"), link());
            EffectsType::Fx
        }
    };
    graph.nodes.insert(fx, Node::Manager(kind));
}

/// `addToGraph`.
fn add_to_graph(graph: &mut LocalGraph, global: Global, node: Node, fields: BTreeMap<String, i64>) {
    graph.nodes.insert(global, node);
    graph.count_fields(fields);
}

/// `addDef` and `addDefHelp`: `main` also becomes the graph's main.
fn add_def(
    cx: &Context,
    annotations: &BTreeMap<String, Annotation>,
    def: &CDef,
    graph: &mut LocalGraph,
) {
    let (name, params, body) = expression::parts(def);
    let mut main_deps = BTreeSet::new();
    if name == "main"
        && let Some(annotation) = annotations.get(name)
    {
        let mut tracker = Tracker::new(cx.compat);
        let main = match annotation.ty.main_kind() {
            MainKind::Static => {
                tracker.kernel("VirtualDom");
                Some(Main::Static)
            }
            MainKind::Program { flags, message } => {
                let decoder = port::to_flags_decoder(cx.compat, &mut tracker, &flags);
                Some(Main::Dynamic {
                    message,
                    decoder: Box::new(decoder),
                })
            }
            // Anything else the main check has refused.
            MainKind::Other => None,
        };
        if let Some(main) = main {
            graph.main = Some(main);
            graph.count_fields(std::mem::take(&mut tracker.fields));
            main_deps = tracker.deps;
        }
    }
    let mut tracker = Tracker::new(cx.compat);
    let value = if params.is_empty() {
        expression::optimize(cx, &mut tracker, &Cycle::new(), body)
    } else {
        let (names, destructors) = expression::destruct_args(cx, &mut tracker, params);
        let body = expression::optimize(cx, &mut tracker, &Cycle::new(), body);
        Expr::Function(names, Box::new(expression::wrap(destructors, body)))
    };
    let mut deps = tracker.deps;
    deps.extend(main_deps);
    add_to_graph(
        graph,
        global(cx.home, name),
        Node::Define(value, deps),
        tracker.fields,
    );
}

/// `addRecDefs`: mutually recursive definitions become one cycle node,
/// named after the last of them, and each a link to it.
fn add_rec_defs(cx: &Context, defs: &[&CDef], graph: &mut LocalGraph) {
    let names: Vec<String> = defs.iter().rev().map(|d| d.name().to_owned()).collect();
    let cycle_name = global(cx.home, &format!("_M${}", names[0]));
    let cycle: Cycle = (defs.iter())
        .filter(|d| !d.has_params())
        .map(|d| d.name().to_owned())
        .collect();
    let mut tracker = Tracker::new(cx.compat);
    let mut values = Vec::new();
    let mut functions = Vec::new();
    for def in defs {
        let (name, params, body) = expression::parts(def);
        if params.is_empty() {
            let value = expression::optimize(cx, &mut tracker, &cycle, body);
            values.push((name.to_owned(), value));
        } else {
            let def = expression::potential_tail_call(cx, &mut tracker, &cycle, name, params, body);
            functions.push(def);
        }
    }
    // Upstream conses each onto its list.
    values.reverse();
    functions.reverse();
    for def in defs {
        graph
            .nodes
            .insert(global(cx.home, def.name()), Node::Link(cycle_name.clone()));
    }
    let node = Node::Cycle {
        names,
        values,
        functions,
        deps: tracker.deps,
    };
    add_to_graph(graph, cycle_name, node, tracker.fields);
}
