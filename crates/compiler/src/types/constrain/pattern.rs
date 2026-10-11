//! Constraints for patterns (`Type/Constrain/Pattern.hs`): what type a
//! pattern matches, and the names it binds, each with its type.

use crate::canonical::Text;
use std::collections::BTreeMap;

use nomi_base::TextRange;

use crate::canonical::{Pattern, PatternCtorArg, PatternKind};
use crate::types::expected::{PCategory, PContext, PExpected};
use crate::types::instantiate::from_src_type;
use crate::types::{self, Constraint, Type, Variable, Variables, core};

/// `State`: the names bound so far, the variables made, and the
/// constraints, in the order upstream finally reads them.
#[derive(Debug, Default)]
pub struct State {
    pub headers: BTreeMap<String, (TextRange, Type)>,
    /// Latest first, as upstream's list.
    pub vars: Vec<Variable>,
    /// Earliest first: upstream's `reverse revCons`.
    pub cons: Vec<Constraint>,
}

impl State {
    fn add_vars(&mut self, front: Vec<Variable>) {
        self.vars.splice(0..0, front);
    }

    /// `addToHeaders`.
    fn add_header(&mut self, region: TextRange, name: &str, expectation: &PExpected<Type>) {
        self.headers
            .insert(name.to_owned(), (region, expectation.ty().clone()));
    }
}

fn list(entry: Type) -> Type {
    Type::App(core("List"), Text::from("List"), vec![entry])
}

/// `add`.
pub fn add(
    vars: &mut Variables,
    pattern: &Pattern,
    expectation: PExpected<Type>,
    state: &mut State,
) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
        add_(vars, pattern, expectation, state)
    });
}

fn add_(vars: &mut Variables, pattern: &Pattern, expectation: PExpected<Type>, state: &mut State) {
    let region = pattern.region;
    let simple = |category, ty, state: &mut State| {
        state.cons.push(Constraint::Pattern(
            region,
            category,
            ty,
            expectation.clone(),
        ));
    };
    match &pattern.kind {
        PatternKind::Anything => {}
        PatternKind::Var(name) => state.add_header(region, name, &expectation),
        PatternKind::Alias(real, name) => {
            state.add_header(region, name, &expectation);
            add(vars, real, expectation, state);
        }
        PatternKind::Unit => simple(PCategory::PUnit, Type::Unit, state),
        PatternKind::Tuple(a, b, c) => {
            let a_var = vars.flex_var();
            let b_var = vars.flex_var();
            let c_var = c.as_ref().map(|_| vars.flex_var());
            add(vars, a, PExpected::PNoExpectation(Type::Var(a_var)), state);
            add(vars, b, PExpected::PNoExpectation(Type::Var(b_var)), state);
            if let (Some(c), Some(c_var)) = (c, c_var) {
                add(vars, c, PExpected::PNoExpectation(Type::Var(c_var)), state);
            }
            let ty = Type::Tuple(
                Box::new(Type::Var(a_var)),
                Box::new(Type::Var(b_var)),
                c_var.map(|v| Box::new(Type::Var(v))),
            );
            state.cons.push(Constraint::Pattern(
                region,
                PCategory::PTuple,
                ty,
                expectation,
            ));
            state.add_vars(
                [Some(a_var), Some(b_var), c_var]
                    .into_iter()
                    .flatten()
                    .collect(),
            );
        }
        PatternKind::Ctor {
            home,
            type_name,
            union,
            name,
            args,
            ..
        } => add_ctor(
            vars,
            region,
            home,
            type_name,
            &union.vars,
            name,
            args,
            expectation,
            state,
        ),
        PatternKind::List(patterns) => {
            let entry_var = vars.flex_var();
            let entry = Type::Var(entry_var);
            for (index, p) in patterns.iter().enumerate() {
                let expected =
                    PExpected::PFromContext(region, PContext::PListEntry(index), entry.clone());
                add(vars, p, expected, state);
            }
            state.cons.push(Constraint::Pattern(
                region,
                PCategory::PList,
                list(entry),
                expectation,
            ));
            state.add_vars(vec![entry_var]);
        }
        PatternKind::Cons(head, tail) => {
            let entry_var = vars.flex_var();
            let entry = Type::Var(entry_var);
            let tail_expected =
                PExpected::PFromContext(region, PContext::PTail, list(entry.clone()));
            add(vars, tail, tail_expected, state);
            add(vars, head, PExpected::PNoExpectation(entry.clone()), state);
            state.cons.push(Constraint::Pattern(
                region,
                PCategory::PList,
                list(entry),
                expectation,
            ));
            state.add_vars(vec![entry_var]);
        }
        PatternKind::Record(fields) => {
            let ext_var = vars.flex_var();
            let field_vars: Vec<(String, Variable)> = fields
                .iter()
                .map(|f| (f.clone(), vars.flex_var()))
                .collect();
            let field_types: BTreeMap<String, Type> = field_vars
                .iter()
                .map(|(n, v)| (n.clone(), Type::Var(*v)))
                .collect();
            let ty = Type::Record(field_types.clone(), Box::new(Type::Var(ext_var)));
            state.cons.push(Constraint::Pattern(
                region,
                PCategory::PRecord,
                ty,
                expectation,
            ));
            // `Map.union headers …`: a name bound already keeps its type.
            for (name, ty) in field_types {
                state.headers.entry(name).or_insert((region, ty));
            }
            let mut front: Vec<Variable> = field_vars.iter().map(|(_, v)| *v).collect();
            front.push(ext_var);
            state.add_vars(front);
        }
        PatternKind::Int(_) => simple(PCategory::PInt, types::int(), state),
        PatternKind::Str(_) => simple(PCategory::PStr, types::string(), state),
        PatternKind::Chr(_) => simple(PCategory::PChr, types::char(), state),
        PatternKind::Bool(..) => simple(PCategory::PBool, types::bool(), state),
    }
}

/// `addCtor`: the union's variables made flexible, each argument against
/// the type the constructor gives it, then the union itself.
#[allow(clippy::too_many_arguments)]
fn add_ctor(
    vars: &mut Variables,
    region: TextRange,
    home: &crate::canonical::Home,
    type_name: &str,
    type_vars: &[String],
    ctor: &str,
    args: &[PatternCtorArg],
    expectation: PExpected<Type>,
    state: &mut State,
) {
    let pairs: Vec<(String, Variable)> = type_vars
        .iter()
        .map(|v| (v.clone(), vars.name_to_flex(v)))
        .collect();
    let free: BTreeMap<String, Type> = pairs
        .iter()
        .map(|(n, v)| (n.clone(), Type::Var(*v)))
        .collect();
    for arg in args {
        let ty = from_src_type(&free, &arg.ty);
        let expected =
            PExpected::PFromContext(region, PContext::PCtorArg(ctor.to_owned(), arg.index), ty);
        add(vars, &arg.arg, expected, state);
    }
    let ty = Type::App(
        home.clone(),
        Text::from(type_name),
        pairs.iter().map(|(_, v)| Type::Var(*v)).collect(),
    );
    state.cons.push(Constraint::Pattern(
        region,
        PCategory::PCtor(ctor.to_owned()),
        ty,
        expectation,
    ));
    state.add_vars(pairs.into_iter().map(|(_, v)| v).collect());
}
