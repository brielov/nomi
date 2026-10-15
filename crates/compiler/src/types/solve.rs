//! The solver (`Type/Solve.hs`): constraints worked through in order,
//! unifying as they go, with `let`-polymorphism by ranked pools. A `let`
//! solves its header one rank deeper, then generalizes whatever didn't
//! escape to an outer rank; a use of a name copies its generalized parts
//! fresh.
//!
//! Upstream copies the environment for each `let`; nomi keeps one map
//! and undoes a scope's names when it ends, which scopes nesting strictly
//! makes the same.

use std::collections::BTreeMap;

use nomi_base::{Text, TextRange};

use super::expected::{Category, Expected, PCategory, PExpected};
use super::names::{to_annotation, to_error_type};
use super::occurs::occurs;
use super::unify::{Mismatched, unify};
use super::{
    AliasReal, Constraint, Content, Descriptor, FlatType, Mark, NO_MARK, NO_RANK, OUTERMOST_RANK,
    Type, Variable, Variables, children, error, in_name_order,
};
use crate::canonical::{self, AliasType};

/// `Error.Error`: a type error, as the solver finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeError {
    BadExpr {
        region: TextRange,
        category: Category,
        actual: error::Type,
        expected: Expected<error::Type>,
    },
    BadPattern {
        region: TextRange,
        category: PCategory,
        actual: error::Type,
        expected: PExpected<error::Type>,
    },
    /// A local whose type contains itself: its name, and its type.
    InfiniteType {
        region: TextRange,
        name: String,
        ty: error::Type,
    },
}

/// `run`: the annotation of every top-level value, or the errors, the
/// latest first, as upstream's list has them.
pub(super) fn run(
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
        stack: Vec::new(),
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
            .map(|(name, var)| (name, to_annotation(vars, var)))
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
    /// The variables `adjust_rank` and `restore` have yet to visit, each
    /// call's above its caller's.
    stack: Vec<Variable>,
}

/// A name a `let` header binds, where, and its variable.
struct Local {
    name: String,
    region: TextRange,
    var: Variable,
}

impl Solver<'_> {
    fn solve(&mut self, rank: u32, constraint: &Constraint) {
        nomi_base::grow_large(|| self.solve_(rank, constraint));
    }

    fn solve_(&mut self, rank: u32, constraint: &Constraint) {
        match constraint {
            Constraint::True => {}
            Constraint::SaveTheEnvironment => self.saved = self.env.clone(),
            Constraint::Equal {
                region,
                category,
                ty,
                expected,
            } => {
                let actual = self.type_to_variable(rank, ty);
                self.check_expr(rank, *region, actual, expected, || category.clone());
            }
            Constraint::Local {
                region,
                name,
                expected,
            } => {
                let actual = self.make_copy(rank, self.env[name]);
                self.check_expr(rank, *region, actual, expected, || {
                    Category::Local(name.clone())
                });
            }
            Constraint::Foreign {
                region,
                name,
                annotation,
                expected,
            } => {
                let actual = self.src_type_to_variable(rank, &annotation.free, &annotation.ty);
                self.check_expr(rank, *region, actual, expected, || {
                    Category::Foreign(name.clone())
                });
            }
            Constraint::Pattern {
                region,
                category,
                ty,
                expected,
            } => {
                let actual = self.type_to_variable(rank, ty);
                let expected_var = self.type_to_variable(rank, expected.ty());
                if let Some(mismatch) = self.unify(rank, actual, expected_var) {
                    let Mismatched {
                        actual,
                        expected: found,
                    } = *mismatch;
                    self.errors.push(TypeError::BadPattern {
                        region: *region,
                        category: category.clone(),
                        actual,
                        expected: expected.replace(found),
                    });
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
                self.solve_body(rank, &locals, body_con);
            }
            Constraint::Let {
                rigid_vars,
                flex_vars,
                header,
                header_con,
                body_con,
            } => self.solve_let(rank, rigid_vars, flex_vars, header, header_con, body_con),
        }
    }

    /// The `Equal`, `Local` and `Foreign` constraints' end: `actual`
    /// against what `expected` expects, a mismatch an error of
    /// `category`.
    fn check_expr(
        &mut self,
        rank: u32,
        region: TextRange,
        actual: Variable,
        expected: &Expected<Type>,
        category: impl FnOnce() -> Category,
    ) {
        let expected_var = self.type_to_variable(rank, expected.ty());
        if let Some(mismatch) = self.unify(rank, actual, expected_var) {
            let Mismatched {
                actual,
                expected: found,
            } = *mismatch;
            self.errors.push(TypeError::BadExpr {
                region,
                category: category(),
                actual,
                expected: expected.replace(found),
            });
        }
    }

    /// A generalizing `CLet`: the header solved a rank deeper, what
    /// didn't escape generalized, then the body.
    fn solve_let(
        &mut self,
        rank: u32,
        rigid_vars: &[Variable],
        flex_vars: &[Variable],
        header: &BTreeMap<String, (TextRange, Type)>,
        header_con: &Constraint,
        body_con: &Constraint,
    ) {
        // Work in the next pool, to localize the header.
        let next = rank + 1;
        if next as usize >= self.pools.len() {
            let len = self.pools.len();
            self.pools.resize(len * 2, Vec::new());
        }
        let mut all: Vec<Variable> = rigid_vars.iter().chain(flex_vars).copied().collect();
        for &var in &all {
            self.vars.get_mut(var).rank = next;
        }
        all.reverse();
        self.pools[next as usize] = all;
        let locals = self.locals(next, header);
        self.solve(next, header_con);
        let young = self.mark;
        let visit = young.next();
        let final_mark = visit.next();
        self.generalize(young, visit, next);
        self.pools[next as usize].clear();
        for &rigid in rigid_vars {
            self.assert_generic(rigid);
        }
        self.mark = final_mark;
        self.solve_body(rank, &locals, body_con);
    }

    /// A `CLet`'s body, its header's locals in scope, then their occurs
    /// checks.
    fn solve_body(&mut self, rank: u32, locals: &[Local], body_con: &Constraint) {
        let added = self.extend_env(locals);
        self.solve(rank, body_con);
        self.restore_env(added);
        self.occurs_checks(locals);
    }

    /// Unifies, pooling the variables unification made; on a mismatch,
    /// the two types as an error shows them.
    fn unify(
        &mut self,
        rank: u32,
        actual: Variable,
        expected: Variable,
    ) -> Option<Box<Mismatched>> {
        let answer = unify(self.vars, actual, expected);
        self.introduce(rank, &answer.fresh);
        answer.mismatch
    }

    /// The header's types as variables, by name with their regions.
    fn locals(&mut self, rank: u32, header: &BTreeMap<String, (TextRange, Type)>) -> Vec<Local> {
        (in_name_order(self.vars.compat, header).into_iter())
            .map(|(name, (region, ty))| Local {
                name: name.clone(),
                region: *region,
                var: self.type_to_variable(rank, ty),
            })
            .collect()
    }

    /// `Map.union env locals`: a name in scope already keeps its
    /// variable. The names added, to take out again.
    fn extend_env<'l>(&mut self, locals: &'l [Local]) -> Vec<&'l str> {
        let mut added = Vec::new();
        for local in locals {
            if !self.env.contains_key(&local.name) {
                self.env.insert(local.name.clone(), local.var);
                added.push(&*local.name);
            }
        }
        added
    }

    fn restore_env(&mut self, added: Vec<&str>) {
        for name in added {
            self.env.remove(name);
        }
    }

    /// `occurs`, for each local in name order: a type containing itself
    /// is an error, and its variable becomes one.
    fn occurs_checks(&mut self, locals: &[Local]) {
        for local in locals {
            if occurs(self.vars, local.var) {
                let ty = to_error_type(self.vars, local.var);
                self.vars.get_mut(local.var).content = Content::Error;
                self.errors.push(TypeError::InfiniteType {
                    region: local.region,
                    name: local.name.clone(),
                    ty,
                });
            }
        }
    }

    /// `isGeneric`: panics unless the rigid variable `var` was
    /// generalized, as every one must be.
    fn assert_generic(&mut self, var: Variable) {
        let rank = self.vars.get(var).rank;
        if rank != NO_RANK {
            let ty = to_error_type(self.vars, var);
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
        let var = self
            .vars
            .fresh_descriptor(Descriptor::at_rank(content, rank));
        self.pools[rank as usize].push(var);
        var
    }

    /// `typeToVariable`. Constraint generation makes no place holders,
    /// so `typeToVar`'s aliases in scope are never looked at.
    fn type_to_variable(&mut self, rank: u32, ty: &Type) -> Variable {
        let pool = &mut self.pools[rank as usize];
        self.vars.type_to_var(ty, &mut |vars, content| {
            let var = vars.fresh_descriptor(Descriptor::at_rank(content, rank));
            pool.push(var);
            var
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
            let var = self
                .vars
                .fresh_descriptor(Descriptor::at_rank(Content::flex_named(name), rank));
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
        nomi_base::grow(|| match ty {
            C::Lambda(a, b) => {
                let a = self.src_type_to_var(rank, flex, a);
                let b = self.src_type_to_var(rank, flex, b);
                self.register(rank, Content::Structure(FlatType::Fun1(a, b)))
            }
            C::Var(name) => flex[name],
            C::Type { home, name, args } => {
                let args = args
                    .iter()
                    .map(|a| self.src_type_to_var(rank, flex, a))
                    .collect();
                self.register(
                    rank,
                    Content::Structure(FlatType::App1 {
                        home: home.clone(),
                        name: Text::from(name),
                        args,
                    }),
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
            C::Alias {
                home,
                name,
                args,
                aliased,
            } => {
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
                    Content::Alias {
                        home: home.clone(),
                        name: Text::from(name),
                        args,
                        real,
                    },
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
        nomi_base::grow(|| {
            let descriptor = self.vars.get(var);
            if let Some(copy) = descriptor.copy {
                return copy;
            }
            if descriptor.rank != NO_RANK {
                return var;
            }
            let content = descriptor.content.clone();
            // The copy's content is set once its parts are copied, and
            // nothing reads it before.
            let copy = (self.vars).fresh_descriptor(Descriptor::at_rank(Content::Error, max_rank));
            self.pools[max_rank as usize].push(copy);
            // Linked before copying the parts, so a cycle stops here.
            let descriptor = self.vars.get_mut(var);
            descriptor.mark = NO_MARK;
            descriptor.copy = Some(copy);
            let new_content = match content {
                Content::Structure(flat) => Content::Structure(self.copy_flat(max_rank, flat)),
                leaf @ (Content::FlexVar(_) | Content::FlexSuper(..) | Content::Error) => leaf,
                Content::RigidVar(name) => Content::FlexVar(Some(name)),
                Content::RigidSuper(s, name) => Content::FlexSuper(s, Some(name)),
                Content::Alias {
                    home,
                    name,
                    args,
                    real,
                } => {
                    let args = args
                        .into_iter()
                        .map(|(n, a)| (n, self.copy_help(max_rank, a)))
                        .collect();
                    let real = self.copy_help(max_rank, real);
                    Content::Alias {
                        home,
                        name,
                        args,
                        real,
                    }
                }
            };
            self.vars
                .set(copy, Descriptor::at_rank(new_content, max_rank));
            copy
        })
    }

    /// `traverseFlatType (makeCopyHelp …)`.
    fn copy_flat(&mut self, max_rank: u32, flat: FlatType) -> FlatType {
        let mut go = |v| self.copy_help(max_rank, v);
        match flat {
            FlatType::App1 { home, name, args } => FlatType::App1 {
                home,
                name,
                args: args.into_iter().map(&mut go).collect(),
            },
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
        nomi_base::grow(|| {
            let descriptor = self.vars.get_mut(var);
            if descriptor.copy.is_none() {
                return;
            }
            descriptor.rank = NO_RANK;
            descriptor.mark = NO_MARK;
            descriptor.copy = None;
            let from = self.stack.len();
            children(&descriptor.content, AliasReal::Include, &mut self.stack);
            for i in from..self.stack.len() {
                let child = self.stack[i];
                self.restore(child);
            }
            self.stack.truncate(from);
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
        nomi_base::grow(|| {
            let descriptor = self.vars.get_mut(var);
            let (rank, mark) = (descriptor.rank, descriptor.mark);
            if mark == young {
                // Marked first, since it may be cyclic.
                descriptor.mark = visit;
                // `adjustRankContent`: the children are read onto the
                // stack's top rather than the content copied.
                let from = self.stack.len();
                let max_rank = match rank_children(&descriptor.content, &mut self.stack) {
                    None => group_rank,
                    Some(start) => {
                        let to = self.stack.len();
                        let mut max_rank = start;
                        for i in from..to {
                            let child = self.stack[i];
                            max_rank =
                                max_rank.max(self.adjust_rank(young, visit, group_rank, child));
                        }
                        self.stack.truncate(from);
                        max_rank
                    }
                };
                self.vars.get_mut(var).rank = max_rank;
                max_rank
            } else if mark == visit {
                rank
            } else {
                let min_rank = group_rank.min(rank);
                descriptor.rank = min_rank;
                descriptor.mark = visit;
                min_rank
            }
        })
    }
}

/// The variables `adjustRankContent` visits in `content`, pushed onto
/// `into` in its order, and the rank their maximum starts from; `None`
/// for a variable or an error, which takes the group's rank.
fn rank_children(content: &Content, into: &mut Vec<Variable>) -> Option<u32> {
    match content {
        Content::FlexVar(_)
        | Content::FlexSuper(..)
        | Content::RigidVar(_)
        | Content::RigidSuper(..)
        | Content::Error => None,
        Content::Structure(flat) => match flat {
            FlatType::App1 { args, .. } => {
                into.extend(args);
                Some(OUTERMOST_RANK)
            }
            FlatType::Fun1(a, b) => {
                into.extend([*a, *b]);
                Some(NO_RANK)
            }
            FlatType::EmptyRecord1 | FlatType::Unit1 => Some(OUTERMOST_RANK),
            FlatType::Record1(fields, ext) => {
                into.push(*ext);
                into.extend(fields.values());
                Some(NO_RANK)
            }
            FlatType::Tuple1(a, b, c) => {
                into.extend([*a, *b]);
                into.extend(*c);
                Some(NO_RANK)
            }
        },
        Content::Alias { args, .. } => {
            into.extend(args.iter().map(|(_, a)| *a));
            Some(OUTERMOST_RANK)
        }
    }
}
