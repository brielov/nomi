//! The solver (`Type/Solve.hs`): constraints worked through in order,
//! unifying as they go, with `let`-polymorphism by ranked pools. A `let`
//! solves its header one rank deeper, then generalizes whatever didn't
//! escape to an outer rank; a use of a name copies its generalized parts
//! fresh.
//!
//! Upstream copies the environment for each `let`; nomi keeps one map
//! and undoes a scope's names when it ends, which scopes nesting strictly
//! makes the same.

use crate::canonical::Text;
use std::collections::BTreeMap;

use nomi_base::TextRange;

use super::expected::{Category, Expected, PCategory, PExpected};
use super::occurs::occurs;
use super::unify::{Answer, unify};
use super::{
    Constraint, Content, Descriptor, FlatType, Mark, NO_MARK, NO_RANK, OUTERMOST_RANK, SuperType,
    Type, Variable, Variables, error, in_name_order,
};
use crate::canonical::{self, AliasType};

/// `Error.Error`: a type error, as the solver finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeError {
    BadExpr(TextRange, Category, error::Type, Expected<error::Type>),
    BadPattern(TextRange, PCategory, error::Type, PExpected<error::Type>),
    InfiniteType(TextRange, String, error::Type),
}

/// `run`: the annotation of every top-level value, or the errors, the
/// latest first, as upstream's list has them.
pub fn run(
    vars: &mut Variables,
    constraint: &Constraint,
) -> Result<BTreeMap<String, canonical::Annotation>, Vec<TypeError>> {
    let mut solver = Solver {
        vars,
        pools: vec![Vec::new(); 8],
        env: BTreeMap::new(),
        saved: BTreeMap::new(),
        mark: NO_MARK.next(),
        errors: Vec::new(),
    };
    solver.solve(OUTERMOST_RANK, constraint);
    let Solver {
        vars,
        saved,
        mut errors,
        ..
    } = solver;
    if errors.is_empty() {
        Ok(saved
            .into_iter()
            .map(|(name, var)| (name, vars.to_annotation(var)))
            .collect())
    } else {
        errors.reverse();
        Err(errors)
    }
}

struct Solver<'v> {
    vars: &'v mut Variables,
    /// Each rank's variables, upstream's list reversed: the end is its
    /// head, so prepending is a push.
    pools: Vec<Vec<Variable>>,
    /// The names in scope.
    env: BTreeMap<String, Variable>,
    /// The environment at `CSaveTheEnvironment`.
    saved: BTreeMap<String, Variable>,
    mark: Mark,
    /// Earliest first.
    errors: Vec<TypeError>,
}

impl Solver<'_> {
    fn solve(&mut self, rank: u32, constraint: &Constraint) {
        stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, || {
            self.solve_(rank, constraint);
        });
    }

    fn solve_(&mut self, rank: u32, constraint: &Constraint) {
        match constraint {
            Constraint::True => {}
            Constraint::SaveTheEnvironment => self.saved = self.env.clone(),
            Constraint::Equal(region, category, ty, expectation) => {
                let actual = self.type_to_variable(rank, ty);
                let expected = self.type_to_variable(rank, expectation.ty());
                if let Some((actual, expected)) = self.unify(rank, actual, expected) {
                    let err = TypeError::BadExpr(
                        *region,
                        category.clone(),
                        actual,
                        expectation.replace(expected),
                    );
                    self.errors.push(err);
                }
            }
            Constraint::Local(region, name, expectation) => {
                let var = self.env[name];
                let actual = self.make_copy(rank, var);
                let expected = self.type_to_variable(rank, expectation.ty());
                if let Some((actual, expected)) = self.unify(rank, actual, expected) {
                    let category = Category::Local(name.clone());
                    self.errors.push(TypeError::BadExpr(
                        *region,
                        category,
                        actual,
                        expectation.replace(expected),
                    ));
                }
            }
            Constraint::Foreign(region, name, annotation, expectation) => {
                let actual = self.src_type_to_variable(rank, &annotation.free, &annotation.ty);
                let expected = self.type_to_variable(rank, expectation.ty());
                if let Some((actual, expected)) = self.unify(rank, actual, expected) {
                    let category = Category::Foreign(name.clone());
                    self.errors.push(TypeError::BadExpr(
                        *region,
                        category,
                        actual,
                        expectation.replace(expected),
                    ));
                }
            }
            Constraint::Pattern(region, category, ty, expectation) => {
                let actual = self.type_to_variable(rank, ty);
                let expected = self.type_to_variable(rank, expectation.ty());
                if let Some((actual, expected)) = self.unify(rank, actual, expected) {
                    let err = TypeError::BadPattern(
                        *region,
                        category.clone(),
                        actual,
                        expectation.replace(expected),
                    );
                    self.errors.push(err);
                }
            }
            Constraint::And(constraints) => {
                for c in constraints {
                    self.solve(rank, c);
                }
            }
            Constraint::Let {
                rigid_vars,
                flex_vars,
                header_con,
                body_con,
                ..
            } if rigid_vars.is_empty() && matches!(**body_con, Constraint::True) => {
                self.introduce(rank, flex_vars);
                self.solve(rank, header_con);
            }
            Constraint::Let {
                rigid_vars,
                flex_vars,
                header,
                header_con,
                body_con,
            } if rigid_vars.is_empty() && flex_vars.is_empty() => {
                self.solve(rank, header_con);
                let locals = self.locals(rank, header);
                let added = self.extend_env(&locals);
                self.solve(rank, body_con);
                self.restore_env(added);
                self.occurs_checks(&locals);
            }
            Constraint::Let {
                rigid_vars,
                flex_vars,
                header,
                header_con,
                body_con,
            } => {
                // Work in the next pool, to localize the header.
                let next = rank + 1;
                if next as usize >= self.pools.len() {
                    let len = self.pools.len();
                    self.pools.resize(len * 2, Vec::new());
                }
                let all: Vec<Variable> = rigid_vars.iter().chain(flex_vars).copied().collect();
                for &var in &all {
                    self.vars.get_mut(var).rank = next;
                }
                self.pools[next as usize] = all.into_iter().rev().collect();
                let locals = self.locals(next, header);
                self.solve(next, header_con);
                let young = self.mark;
                let visit = young.next();
                let final_mark = visit.next();
                self.generalize(young, visit, next);
                self.pools[next as usize].clear();
                for &rigid in rigid_vars {
                    self.is_generic(rigid);
                }
                self.mark = final_mark;
                let added = self.extend_env(&locals);
                self.solve(rank, body_con);
                self.restore_env(added);
                self.occurs_checks(&locals);
            }
        }
    }

    /// Unifies, pooling the variables unification made; on a mismatch,
    /// the two types as an error shows them.
    fn unify(
        &mut self,
        rank: u32,
        actual: Variable,
        expected: Variable,
    ) -> Option<(error::Type, error::Type)> {
        match unify(self.vars, actual, expected) {
            Answer::Ok(made) => {
                self.introduce(rank, &made);
                None
            }
            Answer::Err(made, actual, expected) => {
                self.introduce(rank, &made);
                Some((*actual, *expected))
            }
        }
    }

    /// The header's types as variables, by name with their regions.
    fn locals(
        &mut self,
        rank: u32,
        header: &BTreeMap<String, (TextRange, Type)>,
    ) -> Vec<(String, TextRange, Variable)> {
        (in_name_order(self.vars.compat, header).into_iter())
            .map(|(name, (region, ty))| (name.clone(), *region, self.type_to_variable(rank, ty)))
            .collect()
    }

    /// `Map.union env locals`: a name in scope already keeps its
    /// variable. The names added, to take out again.
    fn extend_env(&mut self, locals: &[(String, TextRange, Variable)]) -> Vec<String> {
        let mut added = Vec::new();
        for (name, _, var) in locals {
            if !self.env.contains_key(name) {
                self.env.insert(name.clone(), *var);
                added.push(name.clone());
            }
        }
        added
    }

    fn restore_env(&mut self, added: Vec<String>) {
        for name in added {
            self.env.remove(&name);
        }
    }

    /// `occurs`, for each local in name order: a type containing itself
    /// is an error, and its variable becomes one.
    fn occurs_checks(&mut self, locals: &[(String, TextRange, Variable)]) {
        for (name, region, var) in locals {
            if occurs(self.vars, *var) {
                let ty = self.vars.to_error_type(*var);
                self.vars.get_mut(*var).content = Content::Error;
                self.errors
                    .push(TypeError::InfiniteType(*region, name.clone(), ty));
            }
        }
    }

    /// `isGeneric`: a rigid variable generalized, as every one must be.
    fn is_generic(&mut self, var: Variable) {
        let rank = self.vars.get(var).rank;
        if rank != NO_RANK {
            let ty = self.vars.to_error_type(var);
            panic!("a rigid variable escaped its annotation: {ty:?} [rank = {rank}]");
        }
    }

    /// `introduce`: `vars` into `rank`'s pool, at that rank.
    fn introduce(&mut self, rank: u32, vars: &[Variable]) {
        // `vars ++ pool`.
        self.pools[rank as usize].extend(vars.iter().rev());
        for &var in vars {
            self.vars.get_mut(var).rank = rank;
        }
    }

    /// `register`: a fresh variable for `content`, pooled.
    fn register(&mut self, rank: u32, content: Content) -> Variable {
        let var = self.vars.fresh_descriptor(Descriptor {
            content,
            rank,
            mark: NO_MARK,
            copy: None,
        });
        self.pools[rank as usize].push(var);
        var
    }

    /// `typeToVariable`.
    fn type_to_variable(&mut self, rank: u32, ty: &Type) -> Variable {
        self.type_to_var(rank, &BTreeMap::new(), ty)
    }

    /// `typeToVar`.
    fn type_to_var(
        &mut self,
        rank: u32,
        aliases: &BTreeMap<String, Variable>,
        ty: &Type,
    ) -> Variable {
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || match ty {
            Type::Var(v) => *v,
            Type::App(home, name, args) => {
                let args = args
                    .iter()
                    .map(|a| self.type_to_var(rank, aliases, a))
                    .collect();
                self.register(
                    rank,
                    Content::Structure(FlatType::App1(home.clone(), name.clone(), args)),
                )
            }
            Type::Fun(a, b) => {
                let a = self.type_to_var(rank, aliases, a);
                let b = self.type_to_var(rank, aliases, b);
                self.register(rank, Content::Structure(FlatType::Fun1(a, b)))
            }
            Type::Alias(home, name, args, real) => {
                let args: Vec<(String, Variable)> = args
                    .iter()
                    .map(|(n, a)| (n.clone(), self.type_to_var(rank, aliases, a)))
                    .collect();
                let dict: BTreeMap<String, Variable> = args.iter().cloned().collect();
                let real = self.type_to_var(rank, &dict, real);
                self.register(rank, Content::Alias(home.clone(), name.clone(), args, real))
            }
            Type::PlaceHolder(name) => aliases[name],
            Type::Record(fields, ext) => {
                let fields = fields
                    .iter()
                    .map(|(n, f)| (n.clone(), self.type_to_var(rank, aliases, f)))
                    .collect();
                let ext = self.type_to_var(rank, aliases, ext);
                self.register(rank, Content::Structure(FlatType::Record1(fields, ext)))
            }
            Type::EmptyRecord => self.register(rank, Content::Structure(FlatType::EmptyRecord1)),
            Type::Unit => self.register(rank, Content::Structure(FlatType::Unit1)),
            Type::Tuple(a, b, c) => {
                let a = self.type_to_var(rank, aliases, a);
                let b = self.type_to_var(rank, aliases, b);
                let c = c.as_ref().map(|c| self.type_to_var(rank, aliases, c));
                self.register(rank, Content::Structure(FlatType::Tuple1(a, b, c)))
            }
        })
    }

    /// `srcTypeToVariable`: an annotation's type, its free variables
    /// fresh and flexible, named as written.
    fn src_type_to_variable(
        &mut self,
        rank: u32,
        free: &[String],
        ty: &canonical::Type,
    ) -> Variable {
        let mut names: Vec<&String> = free.iter().collect();
        names.sort();
        names.dedup();
        let mut flex = BTreeMap::new();
        let mut made = Vec::new();
        for name in names {
            let content = match SuperType::of_name(name) {
                Some(s) => Content::FlexSuper(s, Some(name.clone())),
                None => Content::FlexVar(Some(name.clone())),
            };
            let var = self.vars.fresh_descriptor(Descriptor {
                content,
                rank,
                mark: NO_MARK,
                copy: None,
            });
            flex.insert(name.clone(), var);
            made.push(var);
        }
        self.pools[rank as usize].extend(made.into_iter().rev());
        self.src_type_to_var(rank, &flex, ty)
    }

    /// `srcTypeToVar`.
    fn src_type_to_var(
        &mut self,
        rank: u32,
        flex: &BTreeMap<String, Variable>,
        ty: &canonical::Type,
    ) -> Variable {
        use canonical::Type as C;
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || match ty {
            C::Lambda(a, b) => {
                let a = self.src_type_to_var(rank, flex, a);
                let b = self.src_type_to_var(rank, flex, b);
                self.register(rank, Content::Structure(FlatType::Fun1(a, b)))
            }
            C::Var(name) => flex[name],
            C::Type(home, name, args) => {
                let args = args
                    .iter()
                    .map(|a| self.src_type_to_var(rank, flex, a))
                    .collect();
                self.register(
                    rank,
                    Content::Structure(FlatType::App1(home.clone(), Text::from(name), args)),
                )
            }
            C::Record(fields, ext) => {
                let fields = (fields.iter())
                    .map(|(n, f)| (Text::new(n), self.src_type_to_var(rank, flex, &f.ty)))
                    .collect();
                let ext = match ext {
                    None => self.register(rank, Content::Structure(FlatType::EmptyRecord1)),
                    Some(ext) => flex[ext],
                };
                self.register(rank, Content::Structure(FlatType::Record1(fields, ext)))
            }
            C::Unit => self.register(rank, Content::Structure(FlatType::Unit1)),
            C::Tuple(a, b, c) => {
                let a = self.src_type_to_var(rank, flex, a);
                let b = self.src_type_to_var(rank, flex, b);
                let c = c.as_ref().map(|c| self.src_type_to_var(rank, flex, c));
                self.register(rank, Content::Structure(FlatType::Tuple1(a, b, c)))
            }
            C::Alias(home, name, args, aliased) => {
                let args: Vec<(String, Variable)> = args
                    .iter()
                    .map(|(n, a)| (n.clone(), self.src_type_to_var(rank, flex, a)))
                    .collect();
                let real = match aliased {
                    AliasType::Holey(real) => {
                        let dict: BTreeMap<String, Variable> = args.iter().cloned().collect();
                        self.src_type_to_var(rank, &dict, real)
                    }
                    AliasType::Filled(real) => self.src_type_to_var(rank, flex, real),
                };
                self.register(
                    rank,
                    Content::Alias(home.clone(), Text::from(name), args, real),
                )
            }
        })
    }

    /// `makeCopy`: `var`'s generalized parts copied fresh at `rank`, then
    /// the originals restored.
    fn make_copy(&mut self, rank: u32, var: Variable) -> Variable {
        let copy = self.copy_help(rank, var);
        self.restore(var);
        copy
    }

    /// `makeCopyHelp`.
    fn copy_help(&mut self, max_rank: u32, var: Variable) -> Variable {
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
            let Descriptor {
                content,
                rank,
                copy,
                ..
            } = self.vars.get(var).clone();
            if let Some(copy) = copy {
                return copy;
            }
            if rank != NO_RANK {
                return var;
            }
            let fresh = |content| Descriptor {
                content,
                rank: max_rank,
                mark: NO_MARK,
                copy: None,
            };
            let copy = self.vars.fresh_descriptor(fresh(content.clone()));
            self.pools[max_rank as usize].push(copy);
            // Linked before copying the parts, so a cycle stops here.
            self.vars.set(
                var,
                Descriptor {
                    content: content.clone(),
                    rank,
                    mark: NO_MARK,
                    copy: Some(copy),
                },
            );
            let new_content = match content {
                Content::Structure(flat) => Content::Structure(self.copy_flat(max_rank, flat)),
                Content::FlexVar(_) | Content::FlexSuper(..) | Content::Error => return copy,
                Content::RigidVar(name) => Content::FlexVar(Some(name)),
                Content::RigidSuper(s, name) => Content::FlexSuper(s, Some(name)),
                Content::Alias(home, name, args, real) => {
                    let args = args
                        .into_iter()
                        .map(|(n, a)| (n, self.copy_help(max_rank, a)))
                        .collect();
                    let real = self.copy_help(max_rank, real);
                    Content::Alias(home, name, args, real)
                }
            };
            self.vars.set(copy, fresh(new_content));
            copy
        })
    }

    /// `traverseFlatType (makeCopyHelp …)`.
    fn copy_flat(&mut self, max_rank: u32, flat: FlatType) -> FlatType {
        let mut go = |v| self.copy_help(max_rank, v);
        match flat {
            FlatType::App1(home, name, args) => {
                FlatType::App1(home, name, args.into_iter().map(&mut go).collect())
            }
            FlatType::Fun1(a, b) => {
                let a = go(a);
                FlatType::Fun1(a, go(b))
            }
            FlatType::EmptyRecord1 => FlatType::EmptyRecord1,
            FlatType::Record1(fields, ext) => {
                let fields = fields.into_iter().map(|(n, f)| (n, go(f))).collect();
                FlatType::Record1(fields, go(ext))
            }
            FlatType::Unit1 => FlatType::Unit1,
            FlatType::Tuple1(a, b, c) => {
                let a = go(a);
                let b = go(b);
                FlatType::Tuple1(a, b, c.map(go))
            }
        }
    }

    /// `restore`: the copy links undone.
    fn restore(&mut self, var: Variable) {
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
            let Descriptor { content, copy, .. } = self.vars.get(var).clone();
            if copy.is_none() {
                return;
            }
            self.vars.set(
                var,
                Descriptor {
                    content: content.clone(),
                    rank: NO_RANK,
                    mark: NO_MARK,
                    copy: None,
                },
            );
            for child in children(&content) {
                self.restore(child);
            }
        });
    }

    /// `generalize`: the young pool's variables sorted by rank, their
    /// ranks adjusted, those of lower rank moved to their pools, and the
    /// rest generalized.
    fn generalize(&mut self, young: Mark, visit: Mark, young_rank: u32) {
        let young_vars = std::mem::take(&mut self.pools[young_rank as usize]);
        // `poolToRankTable`: buckets by rank, built by prepending, so kept
        // reversed like the pools.
        let mut table: Vec<Vec<Variable>> = vec![Vec::new(); young_rank as usize + 1];
        for &var in young_vars.iter().rev() {
            let descriptor = self.vars.get_mut(var);
            descriptor.mark = young;
            let rank = descriptor.rank as usize;
            table[rank].push(var);
        }
        self.pools[young_rank as usize] = young_vars;
        for (rank, bucket) in table.iter().enumerate() {
            for &var in bucket.iter().rev() {
                self.adjust_rank(young, visit, rank as u32, var);
            }
        }
        for bucket in &table[..table.len() - 1] {
            for &var in bucket.iter().rev() {
                if !self.vars.redundant(var) {
                    let rank = self.vars.get(var).rank;
                    self.pools[rank as usize].push(var);
                }
            }
        }
        for &var in table[table.len() - 1].iter().rev() {
            if !self.vars.redundant(var) {
                let rank = self.vars.get(var).rank;
                if rank < young_rank {
                    self.pools[rank as usize].push(var);
                } else {
                    self.vars.get_mut(var).rank = NO_RANK;
                }
            }
        }
    }

    /// `adjustRank`: ranks never increase going deeper, so a structure's
    /// outermost rank stands for all of it.
    fn adjust_rank(&mut self, young: Mark, visit: Mark, group_rank: u32, var: Variable) -> u32 {
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
            let Descriptor {
                content,
                rank,
                mark,
                ..
            } = self.vars.get(var).clone();
            if mark == young {
                // Marked first, since it may be cyclic.
                self.vars.get_mut(var).mark = visit;
                let max_rank = self.adjust_content(young, visit, group_rank, &content);
                self.vars.get_mut(var).rank = max_rank;
                max_rank
            } else if mark == visit {
                rank
            } else {
                let min_rank = group_rank.min(rank);
                let descriptor = self.vars.get_mut(var);
                descriptor.rank = min_rank;
                descriptor.mark = visit;
                min_rank
            }
        })
    }

    /// `adjustRankContent`.
    fn adjust_content(
        &mut self,
        young: Mark,
        visit: Mark,
        group_rank: u32,
        content: &Content,
    ) -> u32 {
        let mut go = |v| self.adjust_rank(young, visit, group_rank, v);
        match content {
            Content::FlexVar(_)
            | Content::FlexSuper(..)
            | Content::RigidVar(_)
            | Content::RigidSuper(..)
            | Content::Error => group_rank,
            Content::Structure(flat) => match flat {
                FlatType::App1(_, _, args) => {
                    args.iter().fold(OUTERMOST_RANK, |r, a| r.max(go(*a)))
                }
                FlatType::Fun1(a, b) => {
                    let a = go(*a);
                    a.max(go(*b))
                }
                FlatType::EmptyRecord1 | FlatType::Unit1 => OUTERMOST_RANK,
                FlatType::Record1(fields, ext) => {
                    let ext = go(*ext);
                    fields.values().fold(ext, |r, f| r.max(go(*f)))
                }
                FlatType::Tuple1(a, b, c) => {
                    let ma = go(*a);
                    let mb = go(*b);
                    let m = ma.max(mb);
                    match c {
                        Some(c) => m.max(go(*c)),
                        None => m,
                    }
                }
            },
            Content::Alias(_, _, args, _) => {
                args.iter().fold(OUTERMOST_RANK, |r, (_, a)| r.max(go(*a)))
            }
        }
    }
}

/// The variables a content holds, as `restoreContent` visits them.
fn children(content: &Content) -> Vec<Variable> {
    match content {
        Content::FlexVar(_)
        | Content::FlexSuper(..)
        | Content::RigidVar(_)
        | Content::RigidSuper(..)
        | Content::Error => Vec::new(),
        Content::Structure(flat) => match flat {
            FlatType::App1(_, _, args) => args.clone(),
            FlatType::Fun1(a, b) => vec![*a, *b],
            FlatType::EmptyRecord1 | FlatType::Unit1 => Vec::new(),
            FlatType::Record1(fields, ext) => fields.values().copied().chain([*ext]).collect(),
            FlatType::Tuple1(a, b, c) => [Some(*a), Some(*b), *c].into_iter().flatten().collect(),
        },
        Content::Alias(_, _, args, real) => args.iter().map(|(_, a)| *a).chain([*real]).collect(),
    }
}
