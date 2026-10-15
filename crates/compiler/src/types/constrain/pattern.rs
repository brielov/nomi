//! Constraints for patterns (`Type/Constrain/Pattern.hs`): what type a
//! pattern matches, and the names it binds, each with its type.

use std::collections::BTreeMap;

use nomi_base::{Text, TextRange};

use crate::canonical::{Home, Pattern, PatternCtorArg, PatternKind};
use crate::types::expected::{PCategory, PContext, PExpected};
use crate::types::instantiate::from_src_type;
use crate::types::{self, Constraint, Type, Variable, Variables, list};

/// `State`: the names bound so far, the variables made, and the
/// constraints, in the order upstream finally reads them.
#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) headers: BTreeMap<String, (TextRange, Type)>,
    /// Upstream's list, latest first, reversed: what it prepends is
    /// pushed here.
    pub(crate) vars: Vec<Variable>,
    /// Earliest first: upstream's `reverse revCons`.
    pub(crate) cons: Vec<Constraint>,
}

impl State {
    /// `front`, in order, put before the variables made so far.
    fn add_vars(
        &mut self,
        front: impl IntoIterator<Item = Variable, IntoIter: DoubleEndedIterator>,
    ) {
        self.vars.extend(front.into_iter().rev());
    }

    /// The variables in upstream's order.
    pub(crate) fn take_vars(&mut self) -> Vec<Variable> {
        let mut vars = std::mem::take(&mut self.vars);
        vars.reverse();
        vars
    }

    /// `addToHeaders`.
    fn add_header(&mut self, region: TextRange, name: &str, expectation: &PExpected<Type>) {
        self.headers
            .insert(name.to_owned(), (region, expectation.ty().clone()));
    }

    /// The pattern constraint that `ty` matches `expected`, of `category`.
    fn matches(
        &mut self,
        region: TextRange,
        category: PCategory,
        ty: Type,
        expected: PExpected<Type>,
    ) {
        self.cons.push(Constraint::Pattern {
            region,
            category,
            ty,
            expected,
        });
    }
}

/// `add`.
pub(crate) fn add(
    vars: &mut Variables,
    pattern: &Pattern,
    expectation: PExpected<Type>,
    state: &mut State,
) {
    nomi_base::grow(|| add_(vars, pattern, expectation, state));
}

#[expect(
    clippy::too_many_lines,
    reason = "upstream's `add`, one arm per pattern"
)]
fn add_(vars: &mut Variables, pattern: &Pattern, expectation: PExpected<Type>, state: &mut State) {
    let region = pattern.region;
    // Patterns' types too, for the editor to read records' fields by.
    if let Some(recorded) = &mut vars.recorded {
        recorded.push((region, expectation.ty().clone()));
    }
    let simple = |category, ty, state: &mut State| {
        state.matches(region, category, ty, expectation.clone());
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
            state.matches(region, PCategory::PTuple, ty, expectation);
            state.add_vars([Some(a_var), Some(b_var), c_var].into_iter().flatten());
        }
        PatternKind::Ctor {
            home,
            type_name,
            union,
            name,
            args,
            ..
        } => {
            let ctor = CtorPattern {
                home,
                type_name,
                type_vars: &union.vars,
                name,
                args,
            };
            add_ctor(vars, region, &ctor, expectation, state);
        }
        PatternKind::List(patterns) => {
            let entry_var = vars.flex_var();
            let entry = Type::Var(entry_var);
            for (index, p) in patterns.iter().enumerate() {
                let expected = PExpected::PFromContext {
                    region,
                    context: PContext::PListEntry(index),
                    ty: entry.clone(),
                };
                add(vars, p, expected, state);
            }
            state.matches(region, PCategory::PList, list(entry), expectation);
            state.add_vars([entry_var]);
        }
        PatternKind::Cons(head, tail) => {
            let entry_var = vars.flex_var();
            let entry = Type::Var(entry_var);
            let tail_expected = PExpected::PFromContext {
                region,
                context: PContext::PTail,
                ty: list(entry.clone()),
            };
            add(vars, tail, tail_expected, state);
            add(vars, head, PExpected::PNoExpectation(entry.clone()), state);
            state.matches(region, PCategory::PList, list(entry), expectation);
            state.add_vars([entry_var]);
        }
        PatternKind::Record(fields) => {
            let ext_var = vars.flex_var();
            let field_vars: Vec<(Text, Variable)> = fields
                .iter()
                .map(|f| (Text::new(f), vars.flex_var()))
                .collect();
            let field_types: BTreeMap<Text, Type> = field_vars
                .iter()
                .map(|(n, v)| (n.clone(), Type::Var(*v)))
                .collect();
            let ty = Type::Record(field_types.clone(), Box::new(Type::Var(ext_var)));
            state.matches(region, PCategory::PRecord, ty, expectation);
            // `Map.union headers …`: a name bound already keeps its type.
            for (name, ty) in field_types {
                state
                    .headers
                    .entry(name.to_string())
                    .or_insert((region, ty));
            }
            state.add_vars(field_vars.iter().map(|(_, v)| *v).chain([ext_var]));
        }
        PatternKind::Int(_) => simple(PCategory::PInt, types::int(), state),
        PatternKind::Str(_) => simple(PCategory::PStr, types::string(), state),
        PatternKind::Chr(_) => simple(PCategory::PChr, types::char(), state),
        PatternKind::Bool(..) => simple(PCategory::PBool, types::bool(), state),
    }
}

/// A constructor pattern, as `addCtor` takes it.
struct CtorPattern<'p> {
    home: &'p Home,
    type_name: &'p str,
    /// Its union's type variables.
    type_vars: &'p [String],
    name: &'p str,
    args: &'p [PatternCtorArg],
}

/// `addCtor`: the union's variables made flexible, each argument against
/// the type the constructor gives it, then the union itself.
fn add_ctor(
    vars: &mut Variables,
    region: TextRange,
    ctor: &CtorPattern<'_>,
    expectation: PExpected<Type>,
    state: &mut State,
) {
    let flex: Vec<Variable> = (ctor.type_vars.iter())
        .map(|v| vars.name_to_flex(v))
        .collect();
    let free: BTreeMap<String, Type> = (ctor.type_vars.iter().cloned())
        .zip(flex.iter().map(|v| Type::Var(*v)))
        .collect();
    for (index, arg) in ctor.args.iter().enumerate() {
        let ty = from_src_type(&free, &arg.ty);
        let context = PContext::PCtorArg(ctor.name.to_owned(), index);
        add(
            vars,
            &arg.arg,
            PExpected::PFromContext {
                region,
                context,
                ty,
            },
            state,
        );
    }
    let args = flex.iter().map(|v| Type::Var(*v)).collect();
    let ty = Type::App {
        home: ctor.home.clone(),
        name: Text::from(ctor.type_name),
        args,
    };
    let category = PCategory::PCtor(ctor.name.to_owned());
    state.matches(region, category, ty, expectation);
    state.add_vars(flex);
}
