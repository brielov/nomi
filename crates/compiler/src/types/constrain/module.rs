//! A module's constraint (`Type/Constrain/Module.hs`): its declarations,
//! each over those after it, down to the point where the solver saves
//! every top-level type; ports and an effect manager's functions bound
//! around them.

use std::collections::BTreeMap;

use nomi_base::{Text, TextRange};

use super::expr::{constrain_def, constrain_recursive_defs};
use crate::canonical::{Decl, Effects, Home, Manager, Port};
use crate::canonicalize::module::Canonical;
use crate::types::expected::{Category, Expected};
use crate::types::instantiate::from_src_type;
use crate::types::{self, Constraint, Type, Variables, core, fun};

/// `constrain`.
pub fn constrain(vars: &mut Variables, module: &Canonical) -> Constraint {
    let home = &module.home;
    match &module.effects {
        Effects::NoEffects => decls(vars, &module.decls, Constraint::SaveTheEnvironment),
        Effects::Ports(ports) => {
            // `Map.foldrWithKey letPort`: each port's variables made in
            // name order before the declarations', the last port's
            // binding innermost.
            let lets: Vec<_> = ports
                .iter()
                .map(|(name, port)| let_port(vars, name, port))
                .collect();
            let mut constraint = decls(vars, &module.decls, Constraint::SaveTheEnvironment);
            for (rigids, header) in lets.into_iter().rev() {
                constraint = Constraint::Let {
                    rigid_vars: rigids,
                    flex_vars: Vec::new(),
                    header,
                    header_con: Box::new(Constraint::True),
                    body_con: Box::new(constraint),
                };
            }
            constraint
        }
        Effects::Manager {
            init,
            on_effects,
            on_self_msg,
            manager,
        } => {
            let effects = constrain_effects(vars, home, *init, *on_effects, *on_self_msg, manager);
            let constraint = decls(vars, &module.decls, effects);
            match manager {
                Manager::Cmd(cmd) => let_map(
                    vars,
                    home,
                    cmd,
                    "command",
                    "Platform.Cmd",
                    "Cmd",
                    constraint,
                ),
                Manager::Sub(sub) => let_map(
                    vars,
                    home,
                    sub,
                    "subscription",
                    "Platform.Sub",
                    "Sub",
                    constraint,
                ),
                Manager::Fx { cmd, sub } => {
                    let inner = let_map(
                        vars,
                        home,
                        sub,
                        "subscription",
                        "Platform.Sub",
                        "Sub",
                        constraint,
                    );
                    let_map(vars, home, cmd, "command", "Platform.Cmd", "Cmd", inner)
                }
            }
        }
    }
}

/// `constrainDecls`: the declarations, the last innermost, its constraint
/// made first.
fn decls(vars: &mut Variables, decls: &[Decl], final_con: Constraint) -> Constraint {
    decls.iter().rev().fold(final_con, |body, decl| match decl {
        Decl::Declare(def) => constrain_def(vars, &BTreeMap::new(), def, body),
        Decl::DeclareRec(def, defs) => {
            let all = std::iter::once(def).chain(defs);
            constrain_recursive_defs(vars, &BTreeMap::new(), all, body)
        }
    })
}

/// `letPort`: the port's name bound to its type, its variables rigid.
fn let_port(
    vars: &mut Variables,
    name: &str,
    port: &Port,
) -> (Vec<types::Variable>, BTreeMap<String, (TextRange, Type)>) {
    let mut names = port.free.clone();
    names.sort();
    names.dedup();
    let rigids: Vec<(String, types::Variable)> = names
        .into_iter()
        .map(|n| {
            let v = vars.name_to_rigid(&n);
            (n, v)
        })
        .collect();
    let free: BTreeMap<String, Type> = rigids
        .iter()
        .map(|(n, v)| (n.clone(), Type::Var(*v)))
        .collect();
    let ty = from_src_type(&free, &port.func);
    let header = BTreeMap::from([(name.to_owned(), (TextRange::default(), ty))]);
    (rigids.into_iter().map(|(_, v)| v).collect(), header)
}

/// `letCmd` and `letSub`: `command` or `subscription` bound to a function
/// from the manager's effect type to the platform's.
fn let_map(
    vars: &mut Variables,
    home: &Home,
    tipe: &str,
    name: &str,
    platform: &str,
    platform_type: &str,
    constraint: Constraint,
) -> Constraint {
    let msg_var = vars.flex_var();
    let msg = Type::Var(msg_var);
    let ty = fun(
        Type::App(home.clone(), Text::from(tipe), vec![msg.clone()]),
        Type::App(core(platform), Text::from(platform_type), vec![msg]),
    );
    Constraint::Let {
        rigid_vars: vec![msg_var],
        flex_vars: Vec::new(),
        header: BTreeMap::from([(name.to_owned(), (TextRange::default(), ty))]),
        header_con: Box::new(Constraint::True),
        body_con: Box::new(constraint),
    }
}

fn task(answer: Type) -> Type {
    Type::App(
        core("Platform"),
        Text::from("Task"),
        vec![types::never(), answer],
    )
}

fn router(msg: Type, self_msg: Type) -> Type {
    Type::App(core("Platform"), Text::from("Router"), vec![msg, self_msg])
}

fn effect_list(home: &Home, name: &str, msg: Type) -> Type {
    Type::App(
        core("List"),
        Text::from("List"),
        vec![Type::App(home.clone(), Text::from(name), vec![msg])],
    )
}

/// `constrainEffects`: the manager's functions against the types the
/// platform calls them at.
fn constrain_effects(
    vars: &mut Variables,
    home: &Home,
    r0: TextRange,
    r1: TextRange,
    r2: TextRange,
    manager: &Manager,
) -> Constraint {
    let made: Vec<types::Variable> = (0..7).map(|_| vars.flex_var()).collect();
    let [s0, s1, s2, m1, m2, sm1, sm2] = made[..] else {
        unreachable!("seven variables")
    };
    let v = Type::Var;
    let on_self_msg = fun(router(v(m2), v(sm2)), fun(v(sm2), fun(v(s2), task(v(s2)))));
    let effects_of = |t: &str| effect_list(home, t, v(m1));
    let on_effects = match manager {
        Manager::Cmd(t) | Manager::Sub(t) => fun(
            router(v(m1), v(sm1)),
            fun(effects_of(t), fun(v(s1), task(v(s1)))),
        ),
        Manager::Fx { cmd, sub } => fun(
            router(v(m1), v(sm1)),
            fun(
                effects_of(cmd),
                fun(effects_of(sub), fun(v(s1), task(v(s1)))),
            ),
        ),
    };
    let no = Expected::NoExpectation;
    let effect_cons = Constraint::And(vec![
        Constraint::Local(r0, "init".to_owned(), no(task(v(s0)))),
        Constraint::Local(r1, "onEffects".to_owned(), no(on_effects)),
        Constraint::Local(r2, "onSelfMsg".to_owned(), no(on_self_msg)),
        Constraint::Equal(r1, Category::Effects, v(s0), no(v(s1))),
        Constraint::Equal(r2, Category::Effects, v(s0), no(v(s2))),
        Constraint::Equal(r2, Category::Effects, v(sm1), no(v(sm2))),
    ]);
    let body = match manager {
        Manager::Cmd(cmd) => check_map(vars, "cmdMap", home, cmd, Constraint::SaveTheEnvironment),
        Manager::Sub(sub) => check_map(vars, "subMap", home, sub, Constraint::SaveTheEnvironment),
        Manager::Fx { cmd, sub } => {
            let inner = check_map(vars, "subMap", home, sub, Constraint::SaveTheEnvironment);
            check_map(vars, "cmdMap", home, cmd, inner)
        }
    };
    Constraint::Let {
        rigid_vars: Vec::new(),
        flex_vars: made,
        header: BTreeMap::new(),
        header_con: Box::new(effect_cons),
        body_con: Box::new(body),
    }
}

/// `checkMap`: the manager's map function, `(a -> b) -> t a -> t b`.
fn check_map(
    vars: &mut Variables,
    name: &str,
    home: &Home,
    tipe: &str,
    constraint: Constraint,
) -> Constraint {
    let a = vars.flex_var();
    let b = vars.flex_var();
    let applied = |x| Type::App(home.clone(), Text::from(&*tipe), vec![Type::Var(x)]);
    let map_type = fun(fun(Type::Var(a), Type::Var(b)), fun(applied(a), applied(b)));
    Constraint::Let {
        rigid_vars: vec![a, b],
        flex_vars: Vec::new(),
        header: BTreeMap::new(),
        header_con: Box::new(Constraint::Local(
            TextRange::default(),
            name.to_owned(),
            Expected::NoExpectation(map_type),
        )),
        body_con: Box::new(constraint),
    }
}
