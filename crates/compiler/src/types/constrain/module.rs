//! A module's constraint (`Type/Constrain/Module.hs`): its declarations,
//! each over those after it, down to the point where the solver saves
//! every top-level type; ports and an effect manager's functions bound
//! around them.

use std::collections::BTreeMap;

use nomi_base::{Text, TextRange};

use super::expr::{Rtv, constrain_def, constrain_recursive_defs, new_rigids};
use crate::canonical::{Decl, Effects, Home, Manager, Port};
use crate::canonicalize::module::Canonical;
use crate::types::expected::{Category, Expected};
use crate::types::instantiate::from_src_type;
use crate::types::{self, Constraint, Type, Variable, Variables, fun};

/// `constrain`.
pub(crate) fn constrain(vars: &mut Variables, module: &Canonical) -> Constraint {
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
            let constraint = decls(vars, &module.decls, Constraint::SaveTheEnvironment);
            (lets.into_iter().rev()).fold(constraint, |body, (rigid_vars, header)| {
                Constraint::Let {
                    rigid_vars,
                    flex_vars: Vec::new(),
                    header,
                    header_con: Box::new(Constraint::True),
                    body_con: Box::new(body),
                }
            })
        }
        Effects::Manager {
            init,
            on_effects,
            on_self_msg,
            manager,
        } => {
            let effects = constrain_effects(vars, home, *init, *on_effects, *on_self_msg, manager);
            let constraint = decls(vars, &module.decls, effects);
            (effects_of(manager).rev()).fold(constraint, |body, (effect, tipe)| {
                let_map(vars, home, effect, tipe, body)
            })
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
) -> (Vec<Variable>, BTreeMap<String, (TextRange, Type)>) {
    let (rigids, free) = new_rigids(vars, &Rtv::new(), &port.free);
    let ty = from_src_type(&free, &port.func);
    let header = BTreeMap::from([(name.to_owned(), (TextRange::default(), ty))]);
    (rigids, header)
}

/// What an effect manager manages: commands or subscriptions.
#[derive(Clone, Copy)]
enum Effect {
    Cmd,
    Sub,
}

impl Effect {
    /// The value that makes one: `command` or `subscription`.
    fn binding(self) -> &'static str {
        match self {
            Effect::Cmd => "command",
            Effect::Sub => "subscription",
        }
    }

    /// The platform's type of them, and its module.
    fn platform_type(self) -> (&'static str, &'static str) {
        match self {
            Effect::Cmd => ("Platform.Cmd", "Cmd"),
            Effect::Sub => ("Platform.Sub", "Sub"),
        }
    }

    /// The manager's map function over them.
    fn map_name(self) -> &'static str {
        match self {
            Effect::Cmd => "cmdMap",
            Effect::Sub => "subMap",
        }
    }
}

/// The effects `manager` manages, each with its type's name, commands
/// first.
fn effects_of(manager: &Manager) -> impl DoubleEndedIterator<Item = (Effect, &str)> {
    let (cmd, sub) = match manager {
        Manager::Cmd(cmd) => (Some(cmd), None),
        Manager::Sub(sub) => (None, Some(sub)),
        Manager::Fx { cmd, sub } => (Some(cmd), Some(sub)),
    };
    let cmd = cmd.map(|t| (Effect::Cmd, t.as_str()));
    cmd.into_iter()
        .chain(sub.map(|t| (Effect::Sub, t.as_str())))
}

/// `letCmd` and `letSub`: `command` or `subscription` bound to a function
/// from the manager's effect type `tipe` to the platform's.
fn let_map(
    vars: &mut Variables,
    home: &Home,
    effect: Effect,
    tipe: &str,
    constraint: Constraint,
) -> Constraint {
    let msg_var = vars.flex_var();
    let msg = Type::Var(msg_var);
    let (module, name) = effect.platform_type();
    let ty = fun(
        Type::App(home.clone(), Text::from(tipe), vec![msg.clone()]),
        Type::App(Home::core(module), Text::from(name), vec![msg]),
    );
    Constraint::Let {
        rigid_vars: vec![msg_var],
        flex_vars: Vec::new(),
        header: BTreeMap::from([(effect.binding().to_owned(), (TextRange::default(), ty))]),
        header_con: Box::new(Constraint::True),
        body_con: Box::new(constraint),
    }
}

fn task(answer: Type) -> Type {
    Type::App(
        Home::core("Platform"),
        Text::from("Task"),
        vec![types::never(), answer],
    )
}

fn router(msg: Type, self_msg: Type) -> Type {
    Type::App(
        Home::core("Platform"),
        Text::from("Router"),
        vec![msg, self_msg],
    )
}

/// `constrainEffects`: the manager's functions, defined at `init`,
/// `on_effects` and `on_self_msg`, against the types the platform calls
/// them at.
fn constrain_effects(
    vars: &mut Variables,
    home: &Home,
    init: TextRange,
    on_effects: TextRange,
    on_self_msg: TextRange,
    manager: &Manager,
) -> Constraint {
    let made: [Variable; 7] = std::array::from_fn(|_| vars.flex_var());
    let [s0, s1, s2, m1, m2, sm1, sm2] = made;
    let v = Type::Var;
    let self_msg_type = fun(router(v(m2), v(sm2)), fun(v(sm2), fun(v(s2), task(v(s2)))));
    let effects = effects_of(manager).map(|(_, tipe)| {
        let effect = Type::App(home.clone(), Text::from(tipe), vec![v(m1)]);
        types::list(effect)
    });
    let effects_type = (effects.rev()).fold(fun(v(s1), task(v(s1))), |result, effects| {
        fun(effects, result)
    });
    let effects_type = fun(router(v(m1), v(sm1)), effects_type);
    let no = Expected::NoExpectation;
    let local = |region, name: &str, ty| Constraint::Local {
        region,
        name: name.to_owned(),
        expected: no(ty),
    };
    let equal = |region, ty, expected| Constraint::Equal {
        region,
        category: Category::Effects,
        ty,
        expected: no(expected),
    };
    let effect_cons = Constraint::And(vec![
        local(init, "init", task(v(s0))),
        local(on_effects, "onEffects", effects_type),
        local(on_self_msg, "onSelfMsg", self_msg_type),
        equal(on_effects, v(s0), v(s1)),
        equal(on_self_msg, v(s0), v(s2)),
        equal(on_self_msg, v(sm1), v(sm2)),
    ]);
    let body = (effects_of(manager).rev())
        .fold(Constraint::SaveTheEnvironment, |body, (effect, tipe)| {
            check_map(vars, effect, home, tipe, body)
        });
    Constraint::Let {
        rigid_vars: Vec::new(),
        flex_vars: made.to_vec(),
        header: BTreeMap::new(),
        header_con: Box::new(effect_cons),
        body_con: Box::new(body),
    }
}

/// `checkMap`: the manager's map function, `(a -> b) -> t a -> t b`.
fn check_map(
    vars: &mut Variables,
    effect: Effect,
    home: &Home,
    tipe: &str,
    constraint: Constraint,
) -> Constraint {
    let a = vars.flex_var();
    let b = vars.flex_var();
    let applied = |x| Type::App(home.clone(), Text::from(tipe), vec![Type::Var(x)]);
    let map_type = fun(fun(Type::Var(a), Type::Var(b)), fun(applied(a), applied(b)));
    Constraint::Let {
        rigid_vars: vec![a, b],
        flex_vars: Vec::new(),
        header: BTreeMap::new(),
        header_con: Box::new(Constraint::Local {
            region: TextRange::default(),
            name: effect.map_name().to_owned(),
            expected: Expected::NoExpectation(map_type),
        }),
        body_con: Box::new(constraint),
    }
}
