//! Unification (`Type/Unify.hs`): making two variables stand for one
//! type, or finding that they can't. Upstream writes it in continuation
//! style; here a failed step is an `Err` that `?` passes up, which
//! stops the same steps. Where upstream goes on after a failure, so does
//! this: the other arguments of a type, and the other fields of a
//! record, are still unified before the whole fails.

use std::collections::BTreeMap;

use nomi_base::Text;

use super::names::to_error_type;
use super::{
    Content, Descriptor, FlatType, SuperType, Variable, Variables, error, in_name_order, is_char,
    is_float, is_int, is_list, is_string, occurs::occurs,
};
use crate::canonical::Home;

/// `Answer`: the variables unification made, and on a failure the two
/// types as an error shows them.
#[derive(Debug)]
pub(super) struct Answer {
    pub(super) fresh: Vec<Variable>,
    pub(super) mismatch: Option<Box<Mismatched>>,
}

/// The two types that didn't unify, as an error shows them.
#[derive(Debug)]
pub(super) struct Mismatched {
    pub(super) actual: error::Type,
    pub(super) expected: error::Type,
}

/// A step that found a mismatch.
struct Mismatch;

type Unify<T = ()> = Result<T, Mismatch>;

/// `unify`: one type of `v1`'s and `v2`'s. On a mismatch both become an
/// error, which unifies with anything, so one mistake shows once.
pub(super) fn unify(vars: &mut Variables, v1: Variable, v2: Variable) -> Answer {
    let mut state = State {
        vars,
        fresh: Vec::new(),
    };
    let result = state.guarded(v1, v2);
    let State { vars, mut fresh } = state;
    // Upstream's list has the latest first.
    fresh.reverse();
    let mismatch = result.is_err().then(|| {
        let actual = to_error_type(vars, v1);
        let expected = to_error_type(vars, v2);
        vars.union(v1, v2, Descriptor::new(Content::Error));
        Box::new(Mismatched { actual, expected })
    });
    Answer { fresh, mismatch }
}

struct State<'v> {
    vars: &'v mut Variables,
    /// `register`'s list: the variables made along the way, in order.
    fresh: Vec<Variable>,
}

/// `Context`: the two variables being unified, and the lower of their
/// ranks, which whatever they become takes.
#[derive(Clone, Copy)]
struct Context {
    first: Variable,
    second: Variable,
    rank: u32,
}

impl Context {
    /// `reorient`.
    fn reorient(self) -> Context {
        Context {
            first: self.second,
            second: self.first,
            rank: self.rank,
        }
    }
}

/// `atomMatchesSuper`.
fn atom_matches_super(super_type: SuperType, home: &Home, name: &str) -> bool {
    let number = || is_int(home, name) || is_float(home, name);
    match super_type {
        SuperType::Number => number(),
        SuperType::Comparable => number() || is_string(home, name) || is_char(home, name),
        SuperType::Appendable | SuperType::CompAppend => is_string(home, name),
    }
}

/// `combineRigidSupers`: whether a rigid variable of one kind can stand
/// for a flexible one of another.
fn combine_rigid_supers(rigid: SuperType, flex: SuperType) -> bool {
    use SuperType::{Appendable, CompAppend, Comparable, Number};
    rigid == flex
        || matches!(
            (rigid, flex),
            (Number, Comparable) | (CompAppend, Comparable | Appendable)
        )
}

impl State<'_> {
    /// `merge`: both variables one, with `content`, at the lower rank.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "each unifier step ends in a merge or a mismatch, so a merge is a `Unify` too, as upstream's is"
    )]
    fn merge(&mut self, context: Context, content: Content) -> Unify {
        let descriptor = Descriptor::at_rank(content, context.rank);
        self.vars.union(context.first, context.second, descriptor);
        Ok(())
    }

    /// `fresh`: a new variable at `rank`, registered.
    fn fresh(&mut self, rank: u32, content: Content) -> Variable {
        let var = self
            .vars
            .fresh_descriptor(Descriptor::at_rank(content, rank));
        self.fresh.push(var);
        var
    }

    /// `guardedUnify`, which `subUnify` is.
    fn guarded(&mut self, left: Variable, right: Variable) -> Unify {
        if self.vars.equivalent(left, right) {
            return Ok(());
        }
        let first = self.vars.get(left);
        let (first_rank, first) = (first.rank, first.content.clone());
        let second = self.vars.get(right);
        let (second_rank, second) = (second.rank, second.content.clone());
        let context = Context {
            first: left,
            second: right,
            rank: first_rank.min(second_rank),
        };
        nomi_base::grow(|| self.actually(context, first, second))
    }

    /// `actuallyUnify`.
    fn actually(&mut self, context: Context, first: Content, second: Content) -> Unify {
        match first {
            Content::FlexVar(_) => self.flex(context, first, second),
            Content::FlexSuper(s, _) => self.flex_super(context, s, first, second),
            Content::RigidVar(_) => self.rigid(context, None, first, &second),
            Content::RigidSuper(s, _) => self.rigid(context, Some(s), first, &second),
            Content::Alias(home, name, args, real) => {
                self.alias(context, home, name, args, real, second)
            }
            Content::Structure(flat) => self.structure(context, flat, second),
            // Pretend it is fine, so one error doesn't cascade.
            Content::Error => self.merge(context, Content::Error),
        }
    }

    /// `unifyFlex`.
    fn flex(&mut self, context: Context, content: Content, other: Content) -> Unify {
        match other {
            Content::Error => self.merge(context, Content::Error),
            Content::FlexVar(None) => self.merge(context, content),
            other => self.merge(context, other),
        }
    }

    /// `unifyRigid`.
    fn rigid(
        &mut self,
        context: Context,
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
        context: Context,
        s: SuperType,
        content: Content,
        other: Content,
    ) -> Unify {
        use SuperType::{Appendable, CompAppend, Comparable, Number};
        let compappend = || Content::FlexSuper(CompAppend, None);
        match other {
            Content::Structure(flat) => self.flex_super_structure(context, s, flat),
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
    fn flex_super_structure(&mut self, context: Context, s: SuperType, flat: FlatType) -> Unify {
        match &flat {
            FlatType::App1(home, name, args) if args.is_empty() => {
                if atom_matches_super(s, home, name) {
                    self.merge(context, Content::Structure(flat))
                } else {
                    Err(Mismatch)
                }
            }
            FlatType::App1(home, name, args) if args.len() == 1 && is_list(home, name) => match s {
                SuperType::Number => Err(Mismatch),
                SuperType::Appendable => self.merge(context, Content::Structure(flat)),
                SuperType::Comparable | SuperType::CompAppend => {
                    self.comparable_occurs_check(context)?;
                    self.comparable_recursive(args[0])?;
                    self.merge(context, Content::Structure(flat))
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
                    self.merge(context, Content::Structure(flat))
                }
            },
            FlatType::App1(..)
            | FlatType::Fun1(..)
            | FlatType::EmptyRecord1
            | FlatType::Record1(..)
            | FlatType::Unit1 => Err(Mismatch),
        }
    }

    /// `comparableOccursCheck`: of the second variable, as upstream's.
    fn comparable_occurs_check(&mut self, context: Context) -> Unify {
        if occurs(self.vars, context.second) {
            Err(Mismatch)
        } else {
            Ok(())
        }
    }

    /// `unifyComparableRecursive`.
    fn comparable_recursive(&mut self, var: Variable) -> Unify {
        let rank = self.vars.get(var).rank;
        let comp = self.fresh(rank, Content::FlexSuper(SuperType::Comparable, None));
        self.guarded(comp, var)
    }

    /// `unifyAlias`.
    fn alias(
        &mut self,
        context: Context,
        home: Home,
        name: Text,
        args: Vec<(String, Variable)>,
        real: Variable,
        other: Content,
    ) -> Unify {
        match other {
            Content::FlexVar(_) => self.merge(context, Content::Alias(home, name, args, real)),
            Content::FlexSuper(..)
            | Content::RigidVar(_)
            | Content::RigidSuper(..)
            | Content::Structure(_) => self.guarded(real, context.second),
            Content::Alias(ref other_home, ref other_name, ref other_args, other_real) => {
                if name == *other_name && home == *other_home {
                    let others = other_args.iter().map(|(_, a)| *a);
                    self.all_args(args.iter().map(|(_, a)| *a), others)?;
                    self.merge(context, other)
                } else {
                    self.guarded(real, other_real)
                }
            }
            Content::Error => self.merge(context, Content::Error),
        }
    }

    /// `unifyArgs` and `unifyAliasArgs`: each pair, going on past a
    /// failure, then the whole fails if one did or one list was longer.
    fn all_args(
        &mut self,
        left: impl ExactSizeIterator<Item = Variable>,
        right: impl ExactSizeIterator<Item = Variable>,
    ) -> Unify {
        let same_length = left.len() == right.len();
        let mut failed = false;
        for (a, b) in left.zip(right) {
            failed |= self.guarded(a, b).is_err();
        }
        if failed || !same_length {
            Err(Mismatch)
        } else {
            Ok(())
        }
    }

    /// `unifyStructure`.
    fn structure(&mut self, context: Context, flat: FlatType, other: Content) -> Unify {
        match other {
            Content::FlexVar(_) => self.merge(context, Content::Structure(flat)),
            Content::FlexSuper(s, _) => self.flex_super_structure(context.reorient(), s, flat),
            Content::RigidVar(_) | Content::RigidSuper(..) => Err(Mismatch),
            Content::Alias(_, _, _, real) => self.guarded(context.first, real),
            Content::Error => self.merge(context, Content::Error),
            Content::Structure(ref other_flat) => match (flat, other_flat) {
                (
                    FlatType::App1(home, name, args),
                    FlatType::App1(other_home, other_name, other_args),
                ) if home == *other_home && name == *other_name => {
                    let others = other_args.iter().copied();
                    self.all_args(args.into_iter(), others)?;
                    self.merge(context, other)
                }
                (FlatType::Fun1(arg1, res1), FlatType::Fun1(arg2, res2)) => {
                    self.guarded(arg1, *arg2)?;
                    self.guarded(res1, *res2)?;
                    self.merge(context, other)
                }
                (FlatType::EmptyRecord1, FlatType::EmptyRecord1)
                | (FlatType::Unit1, FlatType::Unit1) => self.merge(context, other),
                (FlatType::Record1(fields, ext), FlatType::EmptyRecord1) if fields.is_empty() => {
                    self.guarded(ext, context.second)
                }
                (FlatType::EmptyRecord1, FlatType::Record1(fields, ext)) if fields.is_empty() => {
                    self.guarded(context.first, *ext)
                }
                (FlatType::Record1(fields1, ext1), FlatType::Record1(fields2, ext2)) => {
                    let one = self.gather_fields(fields1, ext1);
                    let two = self.gather_fields(fields2.clone(), *ext2);
                    self.record(context, one, two)
                }
                (FlatType::Tuple1(a, b, None), FlatType::Tuple1(x, y, None)) => {
                    self.guarded(a, *x)?;
                    self.guarded(b, *y)?;
                    self.merge(context, other)
                }
                (FlatType::Tuple1(a, b, Some(c)), FlatType::Tuple1(x, y, Some(z))) => {
                    self.guarded(a, *x)?;
                    self.guarded(b, *y)?;
                    self.guarded(c, *z)?;
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
            match &self.vars.get(var).content {
                Content::Structure(FlatType::Record1(sub_fields, sub_ext)) => {
                    for (name, field) in sub_fields {
                        fields.entry(name.clone()).or_insert(*field);
                    }
                    var = *sub_ext;
                }
                Content::Alias(_, _, _, real) => var = *real,
                Content::FlexVar(_)
                | Content::FlexSuper(..)
                | Content::RigidVar(_)
                | Content::RigidSuper(..)
                | Content::Structure(_)
                | Content::Error => return (fields, var),
            }
        }
    }

    /// `unifyRecord`.
    fn record(
        &mut self,
        context: Context,
        (fields1, ext1): (BTreeMap<Text, Variable>, Variable),
        (fields2, ext2): (BTreeMap<Text, Variable>, Variable),
    ) -> Unify {
        let mut shared = BTreeMap::new();
        let mut unique1 = BTreeMap::new();
        let mut unique2 = fields2;
        for (name, a) in fields1 {
            if let Some(b) = unique2.remove(&name) {
                shared.insert(name, (a, b));
            } else {
                unique1.insert(name, a);
            }
        }
        let rank = context.rank;
        match (unique1.is_empty(), unique2.is_empty()) {
            (true, true) => {
                self.guarded(ext1, ext2)?;
                self.shared_fields(context, &shared, BTreeMap::new(), ext1)
            }
            (true, false) => {
                let sub = self.fresh(rank, Content::Structure(FlatType::Record1(unique2, ext2)));
                self.guarded(ext1, sub)?;
                self.shared_fields(context, &shared, BTreeMap::new(), sub)
            }
            (false, true) => {
                let sub = self.fresh(rank, Content::Structure(FlatType::Record1(unique1, ext1)));
                self.guarded(sub, ext2)?;
                self.shared_fields(context, &shared, BTreeMap::new(), sub)
            }
            (false, false) => {
                let mut others = unique1.clone();
                others.extend(unique2.clone());
                let ext = self.fresh(rank, Content::FlexVar(None));
                let sub1 = self.fresh(rank, Content::Structure(FlatType::Record1(unique1, ext)));
                let sub2 = self.fresh(rank, Content::Structure(FlatType::Record1(unique2, ext)));
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
        context: Context,
        shared: &BTreeMap<Text, (Variable, Variable)>,
        others: BTreeMap<Text, Variable>,
        ext: Variable,
    ) -> Unify {
        let mut matching = BTreeMap::new();
        // `Map.traverseMaybeWithKey`: in the release's name order.
        for (name, &(actual, expected)) in in_name_order(self.vars.compat, shared) {
            if self.guarded(actual, expected).is_ok() {
                matching.insert(name.clone(), actual);
            }
        }
        if matching.len() != shared.len() {
            return Err(Mismatch);
        }
        // `Map.union matchingFields otherFields`: the shared win.
        let mut fields = others;
        fields.extend(matching);
        self.merge(context, Content::Structure(FlatType::Record1(fields, ext)))
    }
}
