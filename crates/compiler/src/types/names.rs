//! Naming a type's variables for annotations and errors (`Type/Type.hs`'s
//! `toAnnotation` and `toErrorType`). Names the user wrote are kept, the
//! same name on different variables numbered apart, and every unnamed
//! variable takes the first free name of its kind.

use std::collections::{BTreeMap, BTreeSet};

use super::error::{self, Extension};
use super::{
    Content, Descriptor, FlatType, GET_VAR_NAMES_MARK, OCCURS_MARK, SuperType, Variable, Variables,
    in_name_order,
};
use crate::canonical::{self, AliasType, FieldType};

/// `Name.fromTypeVariable`: `name` numbered `index`, after an `_` where
/// it ends in a digit; `name` itself for 0.
pub fn from_type_variable(name: &str, index: usize) -> String {
    if index == 0 {
        name.to_owned()
    } else if name.ends_with(|c: char| c.is_ascii_digit()) {
        format!("{name}_{index}")
    } else {
        format!("{name}{index}")
    }
}

/// `Name.fromTypeVariableScheme`: `a` to `z`, then `a1` to `z1`, and so
/// on.
pub fn from_type_variable_scheme(scheme: usize) -> String {
    let letter = char::from(b'a' + (scheme % 26) as u8);
    match scheme / 26 {
        0 => letter.to_string(),
        extra => format!("{letter}{extra}"),
    }
}

/// `NameState`: the names in use, and where each kind's search for a
/// fresh one goes on from.
struct NameState {
    taken: BTreeSet<String>,
    normals: usize,
    numbers: usize,
    comparables: usize,
    appendables: usize,
    compappends: usize,
}

impl NameState {
    /// `makeNameState`.
    fn new(taken: &BTreeMap<String, Variable>) -> NameState {
        NameState {
            taken: taken.keys().cloned().collect(),
            normals: 0,
            numbers: 0,
            comparables: 0,
            appendables: 0,
            compappends: 0,
        }
    }

    /// `getFreshVarName`.
    fn fresh_var(&mut self) -> String {
        let mut index = self.normals;
        loop {
            let name = from_type_variable_scheme(index);
            index += 1;
            if self.taken.insert(name.clone()) {
                self.normals = index;
                return name;
            }
        }
    }

    /// `getFreshSuperName`.
    fn fresh_super(&mut self, super_type: SuperType) -> String {
        let (prefix, counter) = match super_type {
            SuperType::Number => ("number", &mut self.numbers),
            SuperType::Comparable => ("comparable", &mut self.comparables),
            SuperType::Appendable => ("appendable", &mut self.appendables),
            SuperType::CompAppend => ("compappend", &mut self.compappends),
        };
        let mut index = *counter;
        loop {
            let name = from_type_variable(prefix, index);
            index += 1;
            if self.taken.insert(name.clone()) {
                *counter = index;
                return name;
            }
        }
    }
}

/// `getVarNames`: every name the variables under `var` have, each on one
/// variable, a name on two different ones renamed apart. A child comes
/// before the one written before it, as upstream's `foldrM` visits them.
fn var_names(vars: &mut Variables, var: Variable, taken: &mut BTreeMap<String, Variable>) {
    let descriptor = vars.get(var).clone();
    if descriptor.mark == GET_VAR_NAMES_MARK {
        return;
    }
    vars.get_mut(var).mark = GET_VAR_NAMES_MARK;
    match descriptor.content {
        Content::Error | Content::FlexVar(None) | Content::FlexSuper(_, None) => {}
        Content::FlexVar(Some(name)) => {
            add_name(vars, 0, &name, var, |n| Content::FlexVar(Some(n)), taken);
        }
        Content::FlexSuper(s, Some(name)) => {
            add_name(
                vars,
                0,
                &name,
                var,
                |n| Content::FlexSuper(s, Some(n)),
                taken,
            );
        }
        Content::RigidVar(name) => add_name(vars, 0, &name, var, Content::RigidVar, taken),
        Content::RigidSuper(s, name) => {
            add_name(vars, 0, &name, var, |n| Content::RigidSuper(s, n), taken);
        }
        Content::Alias(_, _, args, _) => {
            for (_, arg) in args.iter().rev() {
                var_names(vars, *arg, taken);
            }
        }
        Content::Structure(flat) => match flat {
            FlatType::App1(_, _, args) => {
                for arg in args.iter().rev() {
                    var_names(vars, *arg, taken);
                }
            }
            FlatType::Fun1(arg, body) => {
                var_names(vars, body, taken);
                var_names(vars, arg, taken);
            }
            FlatType::EmptyRecord1 | FlatType::Unit1 => {}
            FlatType::Record1(fields, extension) => {
                let ordered: Vec<Variable> = (in_name_order(vars.compat, &fields).into_iter())
                    .map(|(_, v)| *v)
                    .collect();
                for field in ordered.into_iter().rev() {
                    var_names(vars, field, taken);
                }
                var_names(vars, extension, taken);
            }
            FlatType::Tuple1(a, b, c) => {
                if let Some(c) = c {
                    var_names(vars, c, taken);
                }
                var_names(vars, b, taken);
                var_names(vars, a, taken);
            }
        },
    }
}

/// `addName`: `given`, numbered `index`, for `var`, renamed further while
/// another variable has it.
fn add_name(
    vars: &mut Variables,
    mut index: usize,
    given: &str,
    var: Variable,
    content: impl Fn(String) -> Content,
    taken: &mut BTreeMap<String, Variable>,
) {
    loop {
        let indexed = from_type_variable(given, index);
        match taken.get(&indexed) {
            None => {
                if indexed != given {
                    vars.get_mut(var).content = content(indexed.clone());
                }
                taken.insert(indexed, var);
                return;
            }
            Some(&other) if vars.equivalent(var, other) => return,
            Some(_) => index += 1,
        }
    }
}

pub(super) fn to_annotation(vars: &mut Variables, var: Variable) -> canonical::Annotation {
    let mut taken = BTreeMap::new();
    var_names(vars, var, &mut taken);
    let mut state = NameState::new(&taken);
    let ty = to_can_type(vars, var, &mut state);
    canonical::Annotation {
        free: state.taken.into_iter().collect(),
        ty: ty.into(),
    }
}

/// `variableToCanType`.
fn to_can_type(vars: &mut Variables, var: Variable, state: &mut NameState) -> canonical::Type {
    use canonical::Type as T;
    match vars.content(var) {
        Content::Structure(flat) => flat_to_can_type(vars, flat, state),
        Content::FlexVar(Some(name)) | Content::FlexSuper(_, Some(name)) => T::Var(name),
        Content::FlexVar(None) => {
            let name = state.fresh_var();
            vars.get_mut(var).content = Content::FlexVar(Some(name.clone()));
            T::Var(name)
        }
        Content::FlexSuper(s, None) => {
            let name = state.fresh_super(s);
            vars.get_mut(var).content = Content::FlexSuper(s, Some(name.clone()));
            T::Var(name)
        }
        Content::RigidVar(name) | Content::RigidSuper(_, name) => T::Var(name),
        Content::Alias(home, name, args, real) => {
            let args = (args.into_iter())
                .map(|(n, a)| (n, to_can_type(vars, a, state)))
                .collect();
            let real = to_can_type(vars, real, state);
            T::Alias {
                home,
                name: String::from(name),
                args,
                aliased: AliasType::Filled(std::sync::Arc::new(real)),
            }
        }
        Content::Error => panic!("cannot handle Error types in variableToCanType"),
    }
}

/// `termToCanType`.
fn flat_to_can_type(
    vars: &mut Variables,
    flat: FlatType,
    state: &mut NameState,
) -> canonical::Type {
    use canonical::Type as T;
    match flat {
        FlatType::App1(home, name, args) => T::Type {
            home,
            name: String::from(name),
            args: (args.into_iter())
                .map(|a| to_can_type(vars, a, state))
                .collect(),
        },
        FlatType::Fun1(a, b) => {
            let a = to_can_type(vars, a, state);
            let b = to_can_type(vars, b, state);
            T::Lambda(Box::new(a), Box::new(b))
        }
        FlatType::EmptyRecord1 => T::Record(BTreeMap::new(), None),
        FlatType::Record1(fields, extension) => {
            let mut fields: BTreeMap<String, FieldType> = (in_name_order(vars.compat, &fields)
                .into_iter())
            .map(|(n, &v)| {
                (
                    n.to_string(),
                    FieldType {
                        order: 0,
                        ty: to_can_type(vars, v, state),
                    },
                )
            })
            .collect();
            match to_can_type(vars, extension, state).iterated_dealias() {
                // `Map.union subFields canFields`: the extension's fields
                // win.
                T::Record(sub_fields, sub_ext) => {
                    fields.extend(sub_fields);
                    T::Record(fields, sub_ext)
                }
                T::Var(name) => T::Record(fields, Some(name)),
                _ => panic!("Used toAnnotation on a type that is not well-formed"),
            }
        }
        FlatType::Unit1 => T::Unit,
        FlatType::Tuple1(a, b, c) => {
            let a = to_can_type(vars, a, state);
            let b = to_can_type(vars, b, state);
            let c = c.map(|c| Box::new(to_can_type(vars, c, state)));
            T::Tuple(Box::new(a), Box::new(b), c)
        }
    }
}

pub(super) fn to_error_type(vars: &mut Variables, var: Variable) -> error::Type {
    let mut taken = BTreeMap::new();
    var_names(vars, var, &mut taken);
    let mut state = NameState::new(&taken);
    to_err(vars, var, &mut state)
}

/// `variableToErrorType`: a variable met again under itself is `∞`.
fn to_err(vars: &mut Variables, var: Variable, state: &mut NameState) -> error::Type {
    let Descriptor { mark, content, .. } = vars.get(var).clone();
    if mark == OCCURS_MARK {
        return error::Type::Infinite;
    }
    vars.get_mut(var).mark = OCCURS_MARK;
    let ty = content_to_err(vars, var, content, state);
    vars.get_mut(var).mark = mark;
    ty
}

/// `contentToErrorType`.
fn content_to_err(
    vars: &mut Variables,
    var: Variable,
    content: Content,
    state: &mut NameState,
) -> error::Type {
    use error::Type as E;
    match content {
        Content::Structure(flat) => flat_to_err(vars, flat, state),
        Content::FlexVar(Some(name)) => E::FlexVar(name),
        Content::FlexVar(None) => {
            let name = state.fresh_var();
            vars.get_mut(var).content = Content::FlexVar(Some(name.clone()));
            E::FlexVar(name)
        }
        Content::FlexSuper(s, Some(name)) => E::FlexSuper(s.to_error(), name),
        Content::FlexSuper(s, None) => {
            let name = state.fresh_super(s);
            vars.get_mut(var).content = Content::FlexSuper(s, Some(name.clone()));
            E::FlexSuper(s.to_error(), name)
        }
        Content::RigidVar(name) => E::RigidVar(name),
        Content::RigidSuper(s, name) => E::RigidSuper(s.to_error(), name),
        Content::Alias(home, name, args, real) => {
            let args = (args.into_iter())
                .map(|(n, a)| (n, to_err(vars, a, state)))
                .collect();
            let real = to_err(vars, real, state);
            E::Alias(home, String::from(name), args, Box::new(real))
        }
        Content::Error => E::Error,
    }
}

/// `termToErrorType`.
fn flat_to_err(vars: &mut Variables, flat: FlatType, state: &mut NameState) -> error::Type {
    use error::Type as E;
    match flat {
        FlatType::App1(home, name, args) => E::Type(
            home,
            String::from(name),
            args.into_iter().map(|a| to_err(vars, a, state)).collect(),
        ),
        FlatType::Fun1(a, b) => {
            let arg = to_err(vars, a, state);
            match to_err(vars, b, state) {
                E::Lambda(arg1, arg2, others) => {
                    let mut rest = vec![*arg2];
                    rest.extend(others);
                    E::Lambda(Box::new(arg), arg1, rest)
                }
                result => E::Lambda(Box::new(arg), Box::new(result), Vec::new()),
            }
        }
        FlatType::EmptyRecord1 => E::Record(BTreeMap::new(), Extension::Closed),
        FlatType::Record1(fields, extension) => {
            let mut fields: BTreeMap<String, error::Type> = (in_name_order(vars.compat, &fields)
                .into_iter())
            .map(|(n, &v)| (n.to_string(), to_err(vars, v, state)))
            .collect();
            match error::iterated_dealias(to_err(vars, extension, state)) {
                E::Record(sub_fields, sub_ext) => {
                    fields.extend(sub_fields);
                    E::Record(fields, sub_ext)
                }
                E::FlexVar(ext) => E::Record(fields, Extension::FlexOpen(ext)),
                E::RigidVar(ext) => E::Record(fields, Extension::RigidOpen(ext)),
                _ => panic!("Used toErrorType on a type that is not well-formed"),
            }
        }
        FlatType::Unit1 => E::Unit,
        FlatType::Tuple1(a, b, c) => {
            let a = to_err(vars, a, state);
            let b = to_err(vars, b, state);
            let c = c.map(|c| Box::new(to_err(vars, c, state)));
            E::Tuple(Box::new(a), Box::new(b), c)
        }
    }
}
