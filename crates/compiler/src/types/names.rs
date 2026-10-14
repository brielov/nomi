//! Naming a type's variables for annotations and errors (`Type/Type.hs`'s
//! `toAnnotation` and `toErrorType`). Names the user wrote are kept, the
//! same name on different variables numbered apart, and every unnamed
//! variable takes the first free name of its kind.

use std::collections::{BTreeMap, BTreeSet};

use super::error::{self, Extension};
use super::{
    Content, FlatType, GET_VAR_NAMES_MARK, OCCURS_MARK, SuperType, Variable, Variables,
    in_name_order,
};
use crate::canonical::{self, AliasType, FieldType};

/// `Name.fromTypeVariable`: `name` numbered `index`, after an `_` where
/// it ends in a digit; `name` itself for 0.
pub(super) fn from_type_variable(name: &str, index: usize) -> String {
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
pub(super) fn from_type_variable_scheme(scheme: usize) -> String {
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
    /// `getVarNames`, then `makeNameState`: the names the variables under
    /// `var` have, each unnamed one to be named from the rest.
    fn of(vars: &mut Variables, var: Variable) -> NameState {
        let mut taken = BTreeMap::new();
        var_names(vars, var, &mut taken);
        NameState {
            taken: taken.into_keys().collect(),
            normals: 0,
            numbers: 0,
            comparables: 0,
            appendables: 0,
            compappends: 0,
        }
    }

    /// The name of the unnamed flexible variable `var`, ranging over
    /// `super_type`: the first free one of its kind, written back.
    fn name(
        &mut self,
        vars: &mut Variables,
        var: Variable,
        super_type: Option<SuperType>,
    ) -> String {
        let name = match super_type {
            None => self.fresh_var(),
            Some(s) => self.fresh_super(s),
        };
        vars.get_mut(var).content = match super_type {
            None => Content::FlexVar(Some(name.clone())),
            Some(s) => Content::FlexSuper(s, Some(name.clone())),
        };
        name
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
    let descriptor = vars.get_mut(var);
    if descriptor.mark == GET_VAR_NAMES_MARK {
        return;
    }
    descriptor.mark = GET_VAR_NAMES_MARK;
    let content = descriptor.content.clone();
    nomi_base::grow(|| match content {
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
                for (_, &field) in in_name_order(vars.compat, &fields).into_iter().rev() {
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
    });
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

/// `toAnnotation`: the type `var` stands for, each unnamed variable
/// named, with its free variables.
pub(super) fn to_annotation(vars: &mut Variables, var: Variable) -> canonical::Annotation {
    let mut state = NameState::of(vars, var);
    let ty = to_can_type(vars, var, &mut state);
    canonical::Annotation {
        free: state.taken.into_iter().collect(),
        ty: ty.into(),
    }
}

/// `variableToCanType`.
fn to_can_type(vars: &mut Variables, var: Variable, state: &mut NameState) -> canonical::Type {
    use canonical::Type as T;
    nomi_base::grow(|| match vars.content(var) {
        Content::Structure(flat) => flat_to_can_type(vars, flat, state),
        Content::FlexVar(Some(name)) | Content::FlexSuper(_, Some(name)) => T::Var(name),
        Content::FlexVar(None) => T::Var(state.name(vars, var, None)),
        Content::FlexSuper(s, None) => T::Var(state.name(vars, var, Some(s))),
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
    })
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

/// `toErrorType`: the type `var` stands for, as an error shows it.
pub(super) fn to_error_type(vars: &mut Variables, var: Variable) -> error::Type {
    let mut state = NameState::of(vars, var);
    to_err(vars, var, &mut state)
}

/// `variableToErrorType`: a variable met again under itself is `∞`.
fn to_err(vars: &mut Variables, var: Variable, state: &mut NameState) -> error::Type {
    let descriptor = vars.get_mut(var);
    let mark = descriptor.mark;
    if mark == OCCURS_MARK {
        return error::Type::Infinite;
    }
    descriptor.mark = OCCURS_MARK;
    let content = descriptor.content.clone();
    let ty = nomi_base::grow(|| content_to_err(vars, var, content, state));
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
        Content::FlexVar(None) => E::FlexVar(state.name(vars, var, None)),
        Content::FlexSuper(s, Some(name)) => E::FlexSuper(s, name),
        Content::FlexSuper(s, None) => E::FlexSuper(s, state.name(vars, var, Some(s))),
        Content::RigidVar(name) => E::RigidVar(name),
        Content::RigidSuper(s, name) => E::RigidSuper(s, name),
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
        // `a -> b -> c` is one arrow of three parts: each function along
        // the result's spine is marked while its argument converts, as
        // `to_err` would mark it.
        FlatType::Fun1(a, b) => {
            let mut parts = vec![to_err(vars, a, state)];
            let mut spine = Vec::new();
            let mut result = b;
            loop {
                let descriptor = vars.get_mut(result);
                let Content::Structure(FlatType::Fun1(a, b)) = descriptor.content else {
                    break;
                };
                if descriptor.mark == OCCURS_MARK {
                    break;
                }
                spine.push((result, descriptor.mark));
                descriptor.mark = OCCURS_MARK;
                parts.push(to_err(vars, a, state));
                result = b;
            }
            parts.push(to_err(vars, result, state));
            for (var, mark) in spine.into_iter().rev() {
                vars.get_mut(var).mark = mark;
            }
            let mut parts = parts.into_iter();
            let arg = parts.next().expect("an argument");
            let result = parts.next().expect("a result");
            E::Lambda(Box::new(arg), Box::new(result), parts.collect())
        }
        FlatType::EmptyRecord1 => E::Record(BTreeMap::new(), Extension::Closed),
        FlatType::Record1(fields, extension) => {
            let mut fields: BTreeMap<String, error::Type> = (in_name_order(vars.compat, &fields)
                .into_iter())
            .map(|(n, &v)| (n.to_string(), to_err(vars, v, state)))
            .collect();
            // `iteratedDealias`, keeping what it stands for.
            let mut ext = to_err(vars, extension, state);
            while let E::Alias(_, _, _, real) = ext {
                ext = *real;
            }
            match ext {
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
