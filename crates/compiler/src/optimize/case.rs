//! `case` expressions (`Optimize/DecisionTree.hs` and `Optimize/Case.hs`):
//! the branches' patterns made into a decision tree with the heuristics
//! of Scott and Ramsey's "When Do Match-Compilation Heuristics Matter?",
//! then into a decider whose leaves are the branches, inlined where one
//! leaf reaches them and jumped to where several do. The order things
//! are gathered in is upstream's, ties included, since it shapes the tree.

use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use nomi_base::Compat;

use super::literal;
use crate::canonical::{CtorOpts, Home, Pattern, PatternKind};
use crate::optimized::{Choice, Decider, DtPath, Expr, Test};

/// `Case.optimize`: the case on `root`, labelled `temp`, of `branches`,
/// each a pattern and its optimized body.
pub(super) fn optimize(
    compat: Compat,
    temp: String,
    root: String,
    branches: Vec<(&Pattern, Expr)>,
) -> Expr {
    let (patterns, bodies): (Vec<_>, Vec<_>) = (branches.into_iter().enumerate())
        .map(|(i, (pattern, body))| ((pattern, i), (i, body)))
        .unzip();
    let decider = to_decider(compile(compat, patterns));
    let mut counts = BTreeMap::new();
    count_targets(&decider, &mut counts);
    let mut choices = BTreeMap::new();
    let mut jumps = Vec::new();
    for (target, body) in bodies {
        if counts.get(&target) == Some(&1) {
            choices.insert(target, Choice::Inline(body));
        } else {
            choices.insert(target, Choice::Jump(target));
            jumps.push((target, body));
        }
    }
    Expr::Case {
        label: temp,
        root,
        decider: Box::new(insert_choices(&mut choices, decider)),
        jumps,
    }
}

/// `DecisionTree`.
#[derive(Clone, Debug, PartialEq)]
enum Tree {
    Match(usize),
    Decision {
        path: DtPath,
        edges: Vec<(Test, Tree)>,
        default: Option<Box<Tree>>,
    },
}

/// `Branch`: a goal, and what is left to test to reach it.
#[derive(Clone)]
struct Branch {
    goal: usize,
    patterns: Vec<(DtPath, Rc<DtPattern>)>,
}

/// A pattern as the decision tree reads it. Every node of the tree copies
/// every branch, so its parts are shared, as upstream's are, and a list's
/// tail is the same list from a later item.
#[derive(Debug)]
enum DtPattern {
    Ctor {
        home: Home,
        name: String,
        index: usize,
        /// How many constructors its union has.
        count: usize,
        opts: CtorOpts,
        args: Vec<Rc<DtPattern>>,
    },
    /// The items from the second field on.
    List(Rc<[Rc<DtPattern>]>, usize),
    Cons(Rc<DtPattern>, Rc<DtPattern>),
    Chr(String),
    Str(String),
    Int(i64),
    Bool(bool),
    Unit,
    Tuple(Vec<Rc<DtPattern>>),
    Alias(Rc<DtPattern>),
    /// A variable, `_` or a record: nothing to test.
    Wild,
}

/// `pattern` as [`DtPattern`], each character and string literal as the
/// release keeps it.
fn lower(compat: Compat, pattern: &Pattern) -> Rc<DtPattern> {
    let go = |p: &Pattern| lower(compat, p);
    Rc::new(nomi_base::grow(|| match &pattern.kind {
        PatternKind::Ctor {
            home,
            union,
            name,
            index,
            args,
            ..
        } => DtPattern::Ctor {
            home: home.clone(),
            name: name.clone(),
            index: *index,
            count: union.ctors.len(),
            opts: union.opts,
            args: args.iter().map(|a| go(&a.arg)).collect(),
        },
        PatternKind::List(items) => DtPattern::List(items.iter().map(go).collect(), 0),
        PatternKind::Cons(head, tail) => DtPattern::Cons(go(head), go(tail)),
        PatternKind::Chr(text) => DtPattern::Chr(literal::char(compat, text)),
        PatternKind::Str(text) => DtPattern::Str(literal::string(compat, text)),
        PatternKind::Int(n) => DtPattern::Int(*n),
        PatternKind::Bool(_, b) => DtPattern::Bool(*b),
        PatternKind::Unit => DtPattern::Unit,
        PatternKind::Tuple(a, b, c) => {
            let mut parts = vec![go(a), go(b)];
            parts.extend(c.as_deref().map(go));
            DtPattern::Tuple(parts)
        }
        PatternKind::Alias(real, _) => DtPattern::Alias(go(real)),
        PatternKind::Var(_) | PatternKind::Anything | PatternKind::Record(_) => DtPattern::Wild,
    }))
}

/// `compile`. Character and string literals are converted once, to the
/// values the release tests, which the patterns then hold.
fn compile(compat: Compat, branches: Vec<(&Pattern, usize)>) -> Tree {
    let branches = (branches.into_iter())
        .map(|(pattern, goal)| Branch {
            goal,
            patterns: vec![(DtPath::Empty, lower(compat, pattern))],
        })
        .collect();
    to_tree(branches)
}

/// `toDecisionTree`.
fn to_tree(branches: Vec<Branch>) -> Tree {
    nomi_base::grow(|| to_tree_(branches))
}

fn to_tree_(branches: Vec<Branch>) -> Tree {
    let branches: Vec<Branch> = branches.into_iter().map(flatten_patterns).collect();
    if let Some(goal) = check_for_match(&branches) {
        return Tree::Match(goal);
    }
    let path = pick_path(&branches);
    let (edges, fallback) = gather_edges(&branches, &path);
    let mut edges: Vec<(Test, Tree)> = (edges.into_iter())
        .map(|(test, branches)| (test, to_tree(branches)))
        .collect();
    match (edges.len(), fallback.is_empty()) {
        (1, true) => edges.pop().expect("one edge").1,
        (_, true) => Tree::Decision {
            path,
            edges,
            default: None,
        },
        (0, false) => to_tree(fallback),
        (_, false) => Tree::Decision {
            path,
            edges,
            default: Some(Box::new(to_tree(fallback))),
        },
    }
}

/// `isComplete`: whether the tests cover every value at their path.
fn is_complete(tests: &[Test]) -> bool {
    match tests.first() {
        Some(Test::IsCtor { count, .. }) => *count == tests.len() as i64,
        Some(Test::IsCons | Test::IsNil | Test::IsBool(_)) => tests.len() == 2,
        Some(Test::IsTuple) => true,
        Some(Test::IsChr(_) | Test::IsStr(_) | Test::IsInt(_)) | None => false,
    }
}

/// `flattenPatterns`: aliases split, and unions of one constructor and
/// tuples looked through, since they need no test.
fn flatten_patterns(branch: Branch) -> Branch {
    let mut patterns = Vec::new();
    for (path, pattern) in branch.patterns {
        flatten(path, &pattern, &mut patterns);
    }
    Branch {
        goal: branch.goal,
        patterns,
    }
}

/// `flatten`, in order onto `out`.
fn flatten(path: DtPath, pattern: &Rc<DtPattern>, out: &mut Vec<(DtPath, Rc<DtPattern>)>) {
    let index = |i: usize, path: &DtPath| DtPath::Index(i, Arc::new(path.clone()));
    nomi_base::grow(|| match &**pattern {
        DtPattern::Ctor { count: 1, args, .. } => match args.as_slice() {
            [arg] => flatten(DtPath::Unbox(Arc::new(path)), arg, out),
            args => {
                for (i, arg) in args.iter().enumerate() {
                    flatten(index(i, &path), arg, out);
                }
            }
        },
        DtPattern::Tuple(parts) => {
            for (i, part) in parts.iter().enumerate() {
                flatten(index(i, &path), part, out);
            }
        }
        DtPattern::Unit => {}
        DtPattern::Alias(real) => {
            flatten(path.clone(), real, out);
            out.push((path, Rc::new(DtPattern::Wild)));
        }
        DtPattern::Ctor { .. }
        | DtPattern::List(..)
        | DtPattern::Cons(..)
        | DtPattern::Chr(_)
        | DtPattern::Str(_)
        | DtPattern::Int(_)
        | DtPattern::Bool(_)
        | DtPattern::Wild => out.push((path, pattern.clone())),
    });
}

/// `checkForMatch`: the first branch's goal, once it needs no more tests.
fn check_for_match(branches: &[Branch]) -> Option<usize> {
    let first = branches.first()?;
    (first.patterns.iter())
        .all(|(_, p)| !needs_tests(p))
        .then_some(first.goal)
}

/// `gatherEdges`: a branch list per test at `path`, and the branches for
/// when none matches.
fn gather_edges(branches: &[Branch], path: &DtPath) -> (Vec<(Test, Vec<Branch>)>, Vec<Branch>) {
    let tests = tests_at_path(path, branches);
    let fallback = if is_complete(&tests) {
        Vec::new()
    } else {
        (branches.iter())
            .filter(|b| is_irrelevant_to(path, b))
            .cloned()
            .collect()
    };
    let edges = (tests.into_iter())
        .map(|test| {
            let relevant = (branches.iter())
                .filter_map(|b| to_relevant_branch(&test, path, b))
                .collect();
            (test, relevant)
        })
        .collect();
    (edges, fallback)
}

/// `testsAtPath`: each test once, where it last appears, as upstream's
/// right fold keeps them.
fn tests_at_path(path: &DtPath, branches: &[Branch]) -> Vec<Test> {
    let mut seen = HashSet::new();
    let mut unique: Vec<Test> = Vec::new();
    for test in branches.iter().rev().filter_map(|b| test_at_path(path, b)) {
        if seen.insert(test.clone()) {
            unique.push(test);
        }
    }
    unique.reverse();
    unique
}

/// The pattern a branch has at `path`.
fn lookup<'b>(path: &DtPath, branch: &'b Branch) -> Option<&'b DtPattern> {
    (branch.patterns.iter())
        .find(|(p, _)| p == path)
        .map(|(_, pattern)| &**pattern)
}

/// `testAtPath`.
fn test_at_path(path: &DtPath, branch: &Branch) -> Option<Test> {
    match lookup(path, branch)? {
        DtPattern::Ctor {
            home,
            name,
            index,
            count,
            opts,
            ..
        } => Some(Test::IsCtor {
            home: home.clone(),
            name: name.clone(),
            index: *index,
            count: *count as i64,
            opts: *opts,
        }),
        DtPattern::List(items, from) if *from == items.len() => Some(Test::IsNil),
        DtPattern::List(..) | DtPattern::Cons(..) => Some(Test::IsCons),
        DtPattern::Tuple(..) | DtPattern::Unit => Some(Test::IsTuple),
        DtPattern::Int(n) => Some(Test::IsInt(*n)),
        DtPattern::Str(text) => Some(Test::IsStr(text.clone())),
        DtPattern::Chr(text) => Some(Test::IsChr(text.clone())),
        DtPattern::Bool(b) => Some(Test::IsBool(*b)),
        DtPattern::Wild => None,
        DtPattern::Alias(..) => unreachable!("aliases are flattened before testing"),
    }
}

/// `toRelevantBranch`: what `branch` still needs once `test` passed at
/// `path`, or `None` if it can't pass.
fn to_relevant_branch(test: &Test, path: &DtPath, branch: &Branch) -> Option<Branch> {
    let Some(at) = branch.patterns.iter().position(|(p, _)| p == path) else {
        return Some(branch.clone());
    };
    let (start, rest) = branch.patterns.split_at(at);
    let (pattern, end) = (&rest[0].1, &rest[1..]);
    let shared = Arc::new(path.clone());
    let with = |middle: Vec<(DtPath, Rc<DtPattern>)>| {
        let mut patterns = start.to_vec();
        patterns.extend(middle);
        patterns.extend_from_slice(end);
        Some(Branch {
            goal: branch.goal,
            patterns,
        })
    };
    let positions = |patterns: Vec<Rc<DtPattern>>| {
        (patterns.into_iter().enumerate())
            .map(|(i, p)| (DtPath::Index(i, shared.clone()), p))
            .collect::<Vec<_>>()
    };
    match &**pattern {
        DtPattern::Ctor {
            name, args, count, ..
        } => match test {
            Test::IsCtor {
                name: test_name, ..
            } if name == test_name => {
                if args.len() == 1 && *count == 1 {
                    with(vec![(DtPath::Unbox(shared.clone()), args[0].clone())])
                } else {
                    with(positions(args.clone()))
                }
            }
            _ => None,
        },
        DtPattern::List(items, from) if *from == items.len() => {
            matches!(test, Test::IsNil).then(|| with(Vec::new()))?
        }
        DtPattern::List(items, from) => match test {
            Test::IsCons => {
                let tail = Rc::new(DtPattern::List(items.clone(), from + 1));
                with(positions(vec![items[*from].clone(), tail]))
            }
            _ => None,
        },
        DtPattern::Cons(head, tail) => match test {
            Test::IsCons => with(positions(vec![head.clone(), tail.clone()])),
            _ => None,
        },
        DtPattern::Chr(text) => match test {
            Test::IsChr(c) if c == text => with(Vec::new()),
            _ => None,
        },
        DtPattern::Str(text) => match test {
            Test::IsStr(s) if s == text => with(Vec::new()),
            _ => None,
        },
        DtPattern::Int(n) => match test {
            Test::IsInt(m) if m == n => with(Vec::new()),
            _ => None,
        },
        DtPattern::Bool(b) => match test {
            Test::IsBool(c) if c == b => with(Vec::new()),
            _ => None,
        },
        DtPattern::Unit => with(Vec::new()),
        DtPattern::Tuple(parts) => with(positions(parts.clone())),
        DtPattern::Wild | DtPattern::Alias(..) => Some(branch.clone()),
    }
}

/// `isIrrelevantTo`: whether `branch` doesn't care what is at `path`.
fn is_irrelevant_to(path: &DtPath, branch: &Branch) -> bool {
    lookup(path, branch).is_none_or(|p| !needs_tests(p))
}

/// `needsTests`.
fn needs_tests(pattern: &DtPattern) -> bool {
    match pattern {
        DtPattern::Wild => false,
        DtPattern::Alias(..) => unreachable!("aliases are flattened before testing"),
        DtPattern::Ctor { .. }
        | DtPattern::List(..)
        | DtPattern::Cons(..)
        | DtPattern::Chr(_)
        | DtPattern::Str(_)
        | DtPattern::Int(_)
        | DtPattern::Bool(_)
        | DtPattern::Unit
        | DtPattern::Tuple(_) => true,
    }
}

/// `pickPath`: the path whose test leaves the fewest branches to the
/// default, then the one with the fewest edges.
fn pick_path(branches: &[Branch]) -> DtPath {
    let defaults = |path: &DtPath| {
        (branches.iter())
            .filter(|b| is_irrelevant_to(path, b))
            .count()
    };
    let tied = bests(
        (branches.iter())
            .flat_map(|b| &b.patterns)
            .filter(|(_, p)| needs_tests(p))
            .map(|(path, _)| (path.clone(), defaults(path))),
    );
    if tied.len() == 1 {
        return tied.into_iter().next().expect("one path");
    }
    // `smallBranchingFactor` counts the edges `gatherEdges` would make,
    // which upstream's laziness never builds, and whether any branch
    // falls back.
    let branching = |path: &DtPath| {
        let tests = tests_at_path(path, branches);
        let fallback = !is_complete(&tests) && branches.iter().any(|b| is_irrelevant_to(path, b));
        tests.len() + usize::from(fallback)
    };
    bests(tied.into_iter().map(|p| {
        let weight = branching(&p);
        (p, weight)
    }))
    .into_iter()
    .next()
    .expect("a path to test")
}

/// `bests`: the paths of the least weight, the later first, as upstream
/// gathers them.
fn bests(weighted: impl Iterator<Item = (DtPath, usize)>) -> Vec<DtPath> {
    let mut least: Option<usize> = None;
    let mut paths: Vec<DtPath> = Vec::new();
    for (path, weight) in weighted {
        match least {
            Some(min) if weight == min => paths.push(path),
            Some(min) if weight > min => {}
            _ => {
                least = Some(weight);
                paths = vec![path];
            }
        }
    }
    paths.reverse();
    paths
}

/// `treeToDecider`. A chain's tests are kept last first while chains
/// merge, and [`insert_choices`] turns them round.
fn to_decider(tree: Tree) -> Decider<usize> {
    nomi_base::grow(|| match tree {
        Tree::Match(target) => Decider::Leaf(target),
        Tree::Decision {
            path,
            mut edges,
            default,
        } => match (edges.len(), default) {
            (0, None) => unreachable!("an empty decision tree"),
            (1, None) => to_decider(edges.pop().expect("one edge").1),
            (0, Some(sub)) => to_decider(*sub),
            (1, Some(failure)) => {
                let (test, success) = edges.pop().expect("one edge");
                to_chain(path, test, success, *failure)
            }
            (2, None) => {
                let (_, failure) = edges.pop().expect("two edges");
                let (test, success) = edges.pop().expect("two edges");
                to_chain(path, test, success, failure)
            }
            (_, default) => {
                let fallback = match default {
                    Some(fallback) => *fallback,
                    None => edges.pop().expect("many edges").1,
                };
                Decider::FanOut {
                    path,
                    tests: (edges.into_iter())
                        .map(|(test, tree)| (test, to_decider(tree)))
                        .collect(),
                    fallback: Box::new(to_decider(fallback)),
                }
            }
        },
    })
}

/// `toChain`: a test, with the tests after it merged in where they share
/// its failure.
fn to_chain(path: DtPath, test: Test, success: Tree, failure: Tree) -> Decider<usize> {
    let failure = to_decider(failure);
    match to_decider(success) {
        Decider::Chain {
            mut tests,
            success,
            failure: sub_failure,
        } if *sub_failure == failure => {
            tests.push((path, test));
            Decider::Chain {
                tests,
                success,
                failure: Box::new(failure),
            }
        }
        success => Decider::Chain {
            tests: vec![(path, test)],
            success: Box::new(success),
            failure: Box::new(failure),
        },
    }
}

/// `countTargets`: how many leaves reach each branch.
fn count_targets(decider: &Decider<usize>, counts: &mut BTreeMap<usize, usize>) {
    nomi_base::grow(|| match decider {
        Decider::Leaf(target) => *counts.entry(*target).or_default() += 1,
        Decider::Chain {
            success, failure, ..
        } => {
            count_targets(success, counts);
            count_targets(failure, counts);
        }
        Decider::FanOut {
            tests, fallback, ..
        } => {
            count_targets(fallback, counts);
            for (_, d) in tests {
                count_targets(d, counts);
            }
        }
    });
}

/// `insertChoices`: each leaf's branch, inlined or a jump; and each
/// chain's tests in order.
fn insert_choices(
    choices: &mut BTreeMap<usize, Choice>,
    decider: Decider<usize>,
) -> Decider<Choice> {
    nomi_base::grow(|| match decider {
        Decider::Leaf(target) => {
            let choice = match choices.get(&target) {
                // A branch reached once is taken where it is.
                Some(Choice::Inline(_)) => choices.remove(&target).expect("a choice"),
                Some(Choice::Jump(n)) => Choice::Jump(*n),
                None => unreachable!("a leaf without a branch"),
            };
            Decider::Leaf(choice)
        }
        Decider::Chain {
            mut tests,
            success,
            failure,
        } => {
            tests.reverse();
            Decider::Chain {
                tests,
                success: Box::new(insert_choices(choices, *success)),
                failure: Box::new(insert_choices(choices, *failure)),
            }
        }
        Decider::FanOut {
            path,
            tests,
            fallback,
        } => Decider::FanOut {
            path,
            tests: (tests.into_iter())
                .map(|(test, d)| (test, insert_choices(choices, d)))
                .collect(),
            fallback: Box::new(insert_choices(choices, *fallback)),
        },
    })
}
