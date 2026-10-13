//! Unification (`Type/Unify.hs`): making two variables stand for one
//! type, or finding that they can't. Upstream writes it in continuation
//! style; here a failed step is an `Err` that `?` passes up, which
//! stops the same steps. Where upstream goes on after a failure, so does
//! this: the other arguments of a type, and the other fields of a
//! record, are still unified before the whole fails.

use std::collections::BTreeMap;

use nomi_base::Text;

use super::{
    Content, Descriptor, FlatType, NO_MARK, SuperType, Variable, Variables, core, error,
    in_name_order, occurs::occurs,
};
use crate::canonical::Home;

/// `Answer`: the variables unification made, and on a failure the two
/// types as an error shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Ok(Vec<Variable>),
    Err(Vec<Variable>, Box<error::Type>, Box<error::Type>),
}

/// A step that found a mismatch.
struct Mismatch;

type Unify<T = ()> = Result<T, Mismatch>;

/// `unify`: one type of `v1`'s and `v2`'s. On a mismatch both become an
/// error, which unifies with anything, so one mistake shows once.
pub fn unify(vars: &mut Variables, v1: Variable, v2: Variable) -> Answer {
    let mut state = State {
        vars,
        fresh: Vec::new(),
    };
    let result = state.guarded(v1, v2);
    let State { vars, mut fresh } = state;
    // Upstream's list has the latest first.
    fresh.reverse();
    if let Ok(()) = result {
        Answer::Ok(fresh)
    } else {
        let t1 = vars.to_error_type(v1);
        let t2 = vars.to_error_type(v2);
        vars.union(v1, v2, Descriptor::new(Content::Error));
        Answer::Err(fresh, Box::new(t1), Box::new(t2))
    }
}

struct State<'v> {
    vars: &'v mut Variables,
    /// `register`'s list: the variables made along the way, in order.
    fresh: Vec<Variable>,
}

/// `Context`: the two variables being unified, as found.
#[derive(Clone)]
struct Context {
    first: Variable,
    first_desc: Descriptor,
    second: Variable,
    second_desc: Descriptor,
}

impl Context {
    /// `reorient`.
    fn reorient(self) -> Context {
        Context {
            first: self.second,
            first_desc: self.second_desc,
            second: self.first,
            second_desc: self.first_desc,
        }
    }

    fn rank(&self) -> u32 {
        self.first_desc.rank.min(self.second_desc.rank)
    }
}

fn is_number(home: &Home, name: &str) -> bool {
    *home == core("Basics") && (name == "Int" || name == "Float")
}

fn is_string(home: &Home, name: &str) -> bool {
    *home == core("String") && name == "String"
}

fn is_char(home: &Home, name: &str) -> bool {
    *home == core("Char") && name == "Char"
}

fn is_list(home: &Home, name: &str) -> bool {
    *home == core("List") && name == "List"
}

/// `atomMatchesSuper`.
fn atom_matches_super(super_type: SuperType, home: &Home, name: &str) -> bool {
    match super_type {
        SuperType::Number => is_number(home, name),
        SuperType::Comparable => {
            is_number(home, name) || is_string(home, name) || is_char(home, name)
        }
        SuperType::Appendable | SuperType::CompAppend => is_string(home, name),
    }
}

/// `combineRigidSupers`: whether a rigid variable of one kind can stand
/// for a flexible one of another.
fn combine_rigid_supers(rigid: SuperType, flex: SuperType) -> bool {
    rigid == flex
        || (rigid == SuperType::Number && flex == SuperType::Comparable)
        || (rigid == SuperType::CompAppend
            && (flex == SuperType::Comparable || flex == SuperType::Appendable))
}

impl State<'_> {
    /// `merge`: both variables one, with `content`, at the lower rank.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "each unifier step ends in a merge or a mismatch, so a merge is a `Unify` too, as upstream's is"
    )]
    fn merge(&mut self, context: &Context, content: Content) -> Unify {
        let descriptor = Descriptor {
            content,
            rank: context.rank(),
            mark: NO_MARK,
            copy: None,
        };
        self.vars.union(context.first, context.second, descriptor);
        Ok(())
    }

    /// `fresh`: a new variable at the lower rank, registered.
    fn fresh(&mut self, context: &Context, content: Content) -> Variable {
        let var = self.vars.fresh_descriptor(Descriptor {
            content,
            rank: context.rank(),
            mark: NO_MARK,
            copy: None,
        });
        self.fresh.push(var);
        var
    }

    /// `guardedUnify`, which `subUnify` is.
    fn guarded(&mut self, left: Variable, right: Variable) -> Unify {
        if self.vars.equivalent(left, right) {
            return Ok(());
        }
        let context = Context {
            first: left,
            first_desc: self.vars.get(left).clone(),
            second: right,
            second_desc: self.vars.get(right).clone(),
        };
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.actually(&context))
    }

    /// `actuallyUnify`.
    fn actually(&mut self, context: &Context) -> Unify {
        let first = context.first_desc.content.clone();
        let second = context.second_desc.content.clone();
        match &first {
            Content::FlexVar(_) => self.flex(context, first, second),
            Content::FlexSuper(s, _) => self.flex_super(context, *s, first.clone(), second),
            Content::RigidVar(_) => self.rigid(context, None, first, &second),
            Content::RigidSuper(s, _) => self.rigid(context, Some(*s), first.clone(), &second),
            Content::Alias(home, name, args, real) => {
                self.alias(context, home, name, args, *real, second)
            }
            Content::Structure(flat) => self.structure(context, flat, first.clone(), second),
            // Pretend it is fine, so one error doesn't cascade.
            Content::Error => self.merge(context, Content::Error),
        }
    }

    /// `unifyFlex`.
    fn flex(&mut self, context: &Context, content: Content, other: Content) -> Unify {
        match other {
            Content::Error => self.merge(context, Content::Error),
            Content::FlexVar(None) => self.merge(context, content),
            other => self.merge(context, other),
        }
    }

    /// `unifyRigid`.
    fn rigid(
        &mut self,
        context: &Context,
        super_type: Option<SuperType>,
        content: Content,
        other: &Content,
    ) -> Unify {
        match other {
            Content::FlexVar(_) => self.merge(context, content),
            Content::FlexSuper(other_super, _) => match super_type {
                Some(s) if combine_rigid_supers(s, *other_super) => self.merge(context, content),
                _ => Err(Mismatch),
            },
            Content::RigidVar(_)
            | Content::RigidSuper(..)
            | Content::Alias(..)
            | Content::Structure(_) => Err(Mismatch),
            Content::Error => self.merge(context, Content::Error),
        }
    }

    /// `unifyFlexSuper`.
    fn flex_super(
        &mut self,
        context: &Context,
        s: SuperType,
        content: Content,
        other: Content,
    ) -> Unify {
        use SuperType::{Appendable, CompAppend, Comparable, Number};
        let compappend = || Content::FlexSuper(CompAppend, None);
        match other {
            Content::Structure(flat) => self.flex_super_structure(context, s, &flat),
            Content::RigidVar(_) => Err(Mismatch),
            Content::RigidSuper(other_super, _) => {
                if combine_rigid_supers(other_super, s) {
                    self.merge(context, other)
                } else {
                    Err(Mismatch)
                }
            }
            Content::FlexVar(_) => self.merge(context, content),
            Content::FlexSuper(other_super, _) => match (s, other_super) {
                (Number, Number | Comparable) => self.merge(context, content),
                (Number, Appendable | CompAppend) => Err(Mismatch),
                (Comparable, Comparable | Number | CompAppend) => self.merge(context, other),
                (Comparable, Appendable) => self.merge(context, compappend()),
                (Appendable, Appendable | CompAppend) => self.merge(context, other),
                (Appendable, Comparable) => self.merge(context, compappend()),
                (Appendable, Number) => Err(Mismatch),
                (CompAppend, Comparable | Appendable | CompAppend) => self.merge(context, content),
                (CompAppend, Number) => Err(Mismatch),
            },
            Content::Alias(_, _, _, real) => self.guarded(context.first, real),
            Content::Error => self.merge(context, Content::Error),
        }
    }

    /// `unifyFlexSuperStructure`.
    fn flex_super_structure(&mut self, context: &Context, s: SuperType, flat: &FlatType) -> Unify {
        let structure = || Content::Structure(flat.clone());
        match flat {
            FlatType::App1(home, name, args) if args.is_empty() => {
                if atom_matches_super(s, home, name) {
                    self.merge(context, structure())
                } else {
                    Err(Mismatch)
                }
            }
            FlatType::App1(home, name, args) if args.len() == 1 && is_list(home, name) => match s {
                SuperType::Number => Err(Mismatch),
                SuperType::Appendable => self.merge(context, structure()),
                SuperType::Comparable | SuperType::CompAppend => {
                    self.comparable_occurs_check(context)?;
                    self.comparable_recursive(args[0])?;
                    self.merge(context, structure())
                }
            },
            FlatType::Tuple1(a, b, c) => match s {
                SuperType::Number | SuperType::Appendable | SuperType::CompAppend => Err(Mismatch),
                SuperType::Comparable => {
                    self.comparable_occurs_check(context)?;
                    self.comparable_recursive(*a)?;
                    self.comparable_recursive(*b)?;
                    if let Some(c) = c {
                        self.comparable_recursive(*c)?;
                    }
                    self.merge(context, structure())
                }
            },
            _ => Err(Mismatch),
        }
    }

    /// `comparableOccursCheck`: of the second variable, as upstream's.
    fn comparable_occurs_check(&mut self, context: &Context) -> Unify {
        if occurs(self.vars, context.second) {
            Err(Mismatch)
        } else {
            Ok(())
        }
    }

    /// `unifyComparableRecursive`.
    fn comparable_recursive(&mut self, var: Variable) -> Unify {
        let rank = self.vars.get(var).rank;
        let comp = self.vars.fresh_descriptor(Descriptor {
            content: Content::FlexSuper(SuperType::Comparable, None),
            rank,
            mark: NO_MARK,
            copy: None,
        });
        self.fresh.push(comp);
        self.guarded(comp, var)
    }

    /// `unifyAlias`.
    fn alias(
        &mut self,
        context: &Context,
        home: &Home,
        name: &str,
        args: &[(String, Variable)],
        real: Variable,
        other: Content,
    ) -> Unify {
        match other {
            Content::FlexVar(_) => {
                let content = Content::Alias(home.clone(), Text::from(name), args.to_vec(), real);
                self.merge(context, content)
            }
            Content::FlexSuper(..)
            | Content::RigidVar(_)
            | Content::RigidSuper(..)
            | Content::Structure(_) => self.guarded(real, context.second),
            Content::Alias(other_home, other_name, other_args, other_real) => {
                if name == other_name && *home == other_home {
                    let pairs: Vec<(Variable, Variable)> = (args.iter().map(|(_, a)| *a))
                        .zip(other_args.iter().map(|(_, a)| *a))
                        .collect();
                    self.all_args(pairs, args.len() == other_args.len())?;
                    let content = Content::Alias(other_home, other_name, other_args, other_real);
                    self.merge(context, content)
                } else {
                    self.guarded(real, other_real)
                }
            }
            Content::Error => self.merge(context, Content::Error),
        }
    }

    /// `unifyArgs` and `unifyAliasArgs`: each pair, going on past a
    /// failure, then the whole fails if one did or one list was longer.
    fn all_args(&mut self, pairs: Vec<(Variable, Variable)>, same_length: bool) -> Unify {
        let mut failed = false;
        for (a, b) in pairs {
            failed |= self.guarded(a, b).is_err();
        }
        if failed || !same_length {
            Err(Mismatch)
        } else {
            Ok(())
        }
    }

    /// `unifyStructure`.
    fn structure(
        &mut self,
        context: &Context,
        flat: &FlatType,
        content: Content,
        other: Content,
    ) -> Unify {
        match other {
            Content::FlexVar(_) => self.merge(context, content),
            Content::FlexSuper(s, _) => {
                self.flex_super_structure(&context.clone().reorient(), s, flat)
            }
            Content::RigidVar(_) | Content::RigidSuper(..) => Err(Mismatch),
            Content::Alias(_, _, _, real) => self.guarded(context.first, real),
            Content::Error => self.merge(context, Content::Error),
            Content::Structure(ref other_flat) => match (flat, other_flat) {
                (
                    FlatType::App1(home, name, args),
                    FlatType::App1(other_home, other_name, other_args),
                ) if home == other_home && name == other_name => {
                    let pairs = args
                        .iter()
                        .copied()
                        .zip(other_args.iter().copied())
                        .collect();
                    self.all_args(pairs, args.len() == other_args.len())?;
                    self.merge(context, other)
                }
                (FlatType::Fun1(arg1, res1), FlatType::Fun1(arg2, res2)) => {
                    self.guarded(*arg1, *arg2)?;
                    self.guarded(*res1, *res2)?;
                    self.merge(context, other)
                }
                (FlatType::EmptyRecord1, FlatType::EmptyRecord1)
                | (FlatType::Unit1, FlatType::Unit1) => self.merge(context, other),
                (FlatType::Record1(fields, ext), FlatType::EmptyRecord1) if fields.is_empty() => {
                    self.guarded(*ext, context.second)
                }
                (FlatType::EmptyRecord1, FlatType::Record1(fields, ext)) if fields.is_empty() => {
                    self.guarded(context.first, *ext)
                }
                (FlatType::Record1(fields1, ext1), FlatType::Record1(fields2, ext2)) => {
                    let one = self.gather_fields(fields1.clone(), *ext1);
                    let two = self.gather_fields(fields2.clone(), *ext2);
                    self.record(context, one, two)
                }
                (FlatType::Tuple1(a, b, None), FlatType::Tuple1(x, y, None)) => {
                    self.guarded(*a, *x)?;
                    self.guarded(*b, *y)?;
                    self.merge(context, other)
                }
                (FlatType::Tuple1(a, b, Some(c)), FlatType::Tuple1(x, y, Some(z))) => {
                    self.guarded(*a, *x)?;
                    self.guarded(*b, *y)?;
                    self.guarded(*c, *z)?;
                    self.merge(context, other)
                }
                _ => Err(Mismatch),
            },
        }
    }

    /// `gatherFields`: a record's fields, those of the records it extends
    /// included (the outer field winning a clash), and what it extends at
    /// last.
    fn gather_fields(
        &mut self,
        mut fields: BTreeMap<Text, Variable>,
        mut var: Variable,
    ) -> (BTreeMap<Text, Variable>, Variable) {
        loop {
            match self.vars.content(var) {
                Content::Structure(FlatType::Record1(sub_fields, sub_ext)) => {
                    for (name, field) in sub_fields {
                        fields.entry(name).or_insert(field);
                    }
                    var = sub_ext;
                }
                Content::Alias(_, _, _, real) => var = real,
                _ => return (fields, var),
            }
        }
    }

    /// `unifyRecord`.
    fn record(
        &mut self,
        context: &Context,
        (fields1, ext1): (BTreeMap<Text, Variable>, Variable),
        (fields2, ext2): (BTreeMap<Text, Variable>, Variable),
    ) -> Unify {
        let shared: BTreeMap<Text, (Variable, Variable)> = (fields1.iter())
            .filter_map(|(n, &a)| fields2.get(n).map(|&b| (n.clone(), (a, b))))
            .collect();
        let unique1: BTreeMap<Text, Variable> = (fields1.iter())
            .filter(|(n, _)| !fields2.contains_key(*n))
            .map(|(n, v)| (n.clone(), *v))
            .collect();
        let unique2: BTreeMap<Text, Variable> = (fields2.iter())
            .filter(|(n, _)| !fields1.contains_key(*n))
            .map(|(n, v)| (n.clone(), *v))
            .collect();
        match (unique1.is_empty(), unique2.is_empty()) {
            (true, true) => {
                self.guarded(ext1, ext2)?;
                self.shared_fields(context, &shared, BTreeMap::new(), ext1)
            }
            (true, false) => {
                let sub = self.fresh(
                    context,
                    Content::Structure(FlatType::Record1(unique2, ext2)),
                );
                self.guarded(ext1, sub)?;
                self.shared_fields(context, &shared, BTreeMap::new(), sub)
            }
            (false, true) => {
                let sub = self.fresh(
                    context,
                    Content::Structure(FlatType::Record1(unique1, ext1)),
                );
                self.guarded(sub, ext2)?;
                self.shared_fields(context, &shared, BTreeMap::new(), sub)
            }
            (false, false) => {
                let mut others = unique1.clone();
                others.extend(unique2.clone());
                let ext = self.fresh(context, Content::FlexVar(None));
                let sub1 = self.fresh(context, Content::Structure(FlatType::Record1(unique1, ext)));
                let sub2 = self.fresh(context, Content::Structure(FlatType::Record1(unique2, ext)));
                self.guarded(ext1, sub2)?;
                self.guarded(sub1, ext2)?;
                self.shared_fields(context, &shared, others, ext)
            }
        }
    }

    /// `unifySharedFields`: every shared field, going on past a failure,
    /// each keeping the first record's variable.
    fn shared_fields(
        &mut self,
        context: &Context,
        shared: &BTreeMap<Text, (Variable, Variable)>,
        others: BTreeMap<Text, Variable>,
        ext: Variable,
    ) -> Unify {
        let count = shared.len();
        let mut matching = BTreeMap::new();
        // `Map.traverseMaybeWithKey`: in the release's name order.
        let shared: Vec<(Text, (Variable, Variable))> = (in_name_order(self.vars.compat, shared)
            .into_iter())
        .map(|(n, pair)| (n.clone(), *pair))
        .collect();
        for (name, (actual, expected)) in shared {
            if self.guarded(actual, expected).is_ok() {
                matching.insert(name, actual);
            }
        }
        if matching.len() != count {
            return Err(Mismatch);
        }
        // `Map.union matchingFields otherFields`: the shared win.
        let mut fields = others;
        fields.extend(matching);
        self.merge(context, Content::Structure(FlatType::Record1(fields, ext)))
    }
}
