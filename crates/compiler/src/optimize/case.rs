//! `case` expressions (`Optimize/DecisionTree.hs` and `Optimize/Case.hs`):
//! the branches' patterns made into a decision tree with the heuristics
//! of Scott and Ramsey's "When Do Match-Compilation Heuristics Matter?",
//! then into a decider whose leaves are the branches, inlined where one
//! leaf reaches them and jumped to where several do. The order things
//! are gathered in is upstream's, ties included, since it shapes the tree.

use std::collections::BTreeMap;

use nomi_base::Compat;

use super::literal;
use crate::canonical::{Pattern, PatternKind};
use crate::optimized::{Choice, Decider, DtPath, Expr, Test};

/// `Case.optimize`: the case on `root`, labelled `temp`, of `branches`,
/// each a pattern and its optimized body.
pub fn optimize(
    compat: Compat,
    temp: String,
    root: String,
    branches: Vec<(Pattern, Expr)>,
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
    Expr::Case(
        temp,
        root,
        Box::new(insert_choices(&mut choices, decider)),
        jumps,
    )
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
    patterns: Vec<(DtPath, Pattern)>,
}

/// `compile`.
fn compile(compat: Compat, branches: Vec<(Pattern, usize)>) -> Tree {
    let branches = (branches.into_iter())
        .map(|(pattern, goal)| Branch {
            goal,
            patterns: vec![(DtPath::Empty, pattern)],
        })
        .collect();
    to_tree(compat, branches)
}

/// `toDecisionTree`.
fn to_tree(compat: Compat, branches: Vec<Branch>) -> Tree {
    let branches: Vec<Branch> = branches.into_iter().map(flatten_patterns).collect();
    if let Some(goal) = check_for_match(&branches) {
        return Tree::Match(goal);
    }
    let path = pick_path(compat, &branches);
    let (edges, fallback) = gather_edges(compat, &branches, &path);
    let mut edges: Vec<(Test, Tree)> = (edges.into_iter())
        .map(|(test, branches)| (test, to_tree(compat, branches)))
        .collect();
    match (edges.len(), fallback.is_empty()) {
        (1, true) => edges.pop().expect("one edge").1,
        (_, true) => Tree::Decision {
            path,
            edges,
            default: None,
        },
        (0, false) => to_tree(compat, fallback),
        (_, false) => Tree::Decision {
            path,
            edges,
            default: Some(Box::new(to_tree(compat, fallback))),
        },
    }
}

/// `isComplete`: whether the tests cover every value at their path.
fn is_complete(tests: &[Test]) -> bool {
    match tests.first() {
        Some(Test::IsCtor(_, _, _, alts, _)) => *alts == tests.len() as i64,
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
        flatten(path, pattern, &mut patterns);
    }
    Branch {
        goal: branch.goal,
        patterns,
    }
}

/// `flatten`, in order onto `out`.
fn flatten(path: DtPath, mut pattern: Pattern, out: &mut Vec<(DtPath, Pattern)>) {
    let index = |i: usize, path: &DtPath| DtPath::Index(i, Box::new(path.clone()));
    let region = pattern.region;
    match std::mem::take(&mut pattern.kind) {
        PatternKind::Ctor { union, args, .. } if union.count == 1 => {
            match <[_; 1]>::try_from(args) {
                Ok([arg]) => flatten(DtPath::Unbox(Box::new(path)), arg.arg, out),
                Err(args) => {
                    for (i, arg) in args.into_iter().enumerate() {
                        flatten(index(i, &path), arg.arg, out);
                    }
                }
            }
        }
        PatternKind::Tuple(a, b, c) => {
            flatten(index(0, &path), *a, out);
            flatten(index(1, &path), *b, out);
            if let Some(c) = c {
                flatten(index(2, &path), *c, out);
            }
        }
        PatternKind::Unit => {}
        PatternKind::Alias(real, alias) => {
            flatten(path.clone(), *real, out);
            let kind = PatternKind::Var(alias);
            out.push((path, Pattern { region, kind }));
        }
        kind => out.push((path, Pattern { region, kind })),
    }
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
fn gather_edges(
    compat: Compat,
    branches: &[Branch],
    path: &DtPath,
) -> (Vec<(Test, Vec<Branch>)>, Vec<Branch>) {
    let tests = tests_at_path(compat, path, branches);
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
                .filter_map(|b| to_relevant_branch(compat, &test, path, b))
                .collect();
            (test, relevant)
        })
        .collect();
    (edges, fallback)
}

/// `testsAtPath`: each test once, where it last appears, as upstream's
/// right fold keeps them.
fn tests_at_path(compat: Compat, path: &DtPath, branches: &[Branch]) -> Vec<Test> {
    let all: Vec<Test> = (branches.iter())
        .filter_map(|b| test_at_path(compat, path, b))
        .collect();
    let mut unique: Vec<Test> = Vec::new();
    for test in all.into_iter().rev() {
        if !unique.contains(&test) {
            unique.insert(0, test);
        }
    }
    unique
}

/// The pattern a branch has at `path`.
fn lookup<'b>(path: &DtPath, branch: &'b Branch) -> Option<&'b Pattern> {
    (branch.patterns.iter())
        .find(|(p, _)| p == path)
        .map(|(_, pattern)| pattern)
}

/// `testAtPath`.
fn test_at_path(compat: Compat, path: &DtPath, branch: &Branch) -> Option<Test> {
    match &lookup(path, branch)?.kind {
        PatternKind::Ctor {
            home,
            union,
            name,
            index,
            ..
        } => Some(Test::IsCtor(
            home.clone(),
            name.clone(),
            *index,
            union.count as i64,
            union.opts,
        )),
        PatternKind::List(items) if items.is_empty() => Some(Test::IsNil),
        PatternKind::List(_) | PatternKind::Cons(..) => Some(Test::IsCons),
        PatternKind::Tuple(..) | PatternKind::Unit => Some(Test::IsTuple),
        PatternKind::Int(n) => Some(Test::IsInt(*n)),
        PatternKind::Str(token) => Some(Test::IsStr(literal::string(compat, token))),
        PatternKind::Chr(token) => Some(Test::IsChr(literal::char(compat, token))),
        PatternKind::Bool(_, b) => Some(Test::IsBool(*b)),
        PatternKind::Var(_) | PatternKind::Anything | PatternKind::Record(_) => None,
        PatternKind::Alias(..) => unreachable!("aliases are flattened before testing"),
    }
}

/// `toRelevantBranch`: what `branch` still needs once `test` passed at
/// `path`, or `None` if it can't pass.
fn to_relevant_branch(
    compat: Compat,
    test: &Test,
    path: &DtPath,
    branch: &Branch,
) -> Option<Branch> {
    let Some(at) = branch.patterns.iter().position(|(p, _)| p == path) else {
        return Some(branch.clone());
    };
    let (start, rest) = branch.patterns.split_at(at);
    let (pattern, end) = (&rest[0].1, &rest[1..]);
    let index = |i: usize| DtPath::Index(i, Box::new(path.clone()));
    let with = |middle: Vec<(DtPath, Pattern)>| {
        let mut patterns = start.to_vec();
        patterns.extend(middle);
        patterns.extend_from_slice(end);
        Some(Branch {
            goal: branch.goal,
            patterns,
        })
    };
    let positions = |patterns: Vec<Pattern>| {
        (patterns.into_iter().enumerate())
            .map(|(i, p)| (index(i), p))
            .collect::<Vec<_>>()
    };
    match &pattern.kind {
        PatternKind::Ctor {
            union, name, args, ..
        } => match test {
            Test::IsCtor(_, test_name, ..) if name == test_name => {
                let args: Vec<Pattern> = args.iter().map(|a| a.arg.clone()).collect();
                if args.len() == 1 && union.count == 1 {
                    let arg = args.into_iter().next().expect("one argument");
                    with(vec![(DtPath::Unbox(Box::new(path.clone())), arg)])
                } else {
                    with(positions(args))
                }
            }
            _ => None,
        },
        PatternKind::List(items) if items.is_empty() => {
            matches!(test, Test::IsNil).then(|| with(Vec::new()))?
        }
        PatternKind::List(items) => match test {
            Test::IsCons => {
                let tail = Pattern {
                    region: pattern.region,
                    kind: PatternKind::List(items[1..].to_vec()),
                };
                with(positions(vec![items[0].clone(), tail]))
            }
            _ => None,
        },
        PatternKind::Cons(head, tail) => match test {
            Test::IsCons => with(positions(vec![(**head).clone(), (**tail).clone()])),
            _ => None,
        },
        PatternKind::Chr(token) => match test {
            Test::IsChr(c) if *c == literal::char(compat, token) => with(Vec::new()),
            _ => None,
        },
        PatternKind::Str(token) => match test {
            Test::IsStr(s) if *s == literal::string(compat, token) => with(Vec::new()),
            _ => None,
        },
        PatternKind::Int(n) => match test {
            Test::IsInt(m) if m == n => with(Vec::new()),
            _ => None,
        },
        PatternKind::Bool(_, b) => match test {
            Test::IsBool(c) if c == b => with(Vec::new()),
            _ => None,
        },
        PatternKind::Unit => with(Vec::new()),
        PatternKind::Tuple(a, b, c) => {
            let mut parts = vec![(**a).clone(), (**b).clone()];
            parts.extend(c.as_deref().cloned());
            with(positions(parts))
        }
        PatternKind::Var(_)
        | PatternKind::Anything
        | PatternKind::Record(_)
        | PatternKind::Alias(..) => Some(branch.clone()),
    }
}

/// `isIrrelevantTo`: whether `branch` doesn't care what is at `path`.
fn is_irrelevant_to(path: &DtPath, branch: &Branch) -> bool {
    lookup(path, branch).is_none_or(|p| !needs_tests(p))
}

/// `needsTests`.
fn needs_tests(pattern: &Pattern) -> bool {
    match pattern.kind {
        PatternKind::Var(_) | PatternKind::Anything | PatternKind::Record(_) => false,
        PatternKind::Alias(..) => unreachable!("aliases are flattened before testing"),
        _ => true,
    }
}

/// `pickPath`: the path whose test leaves the fewest branches to the
/// default, then the one with the fewest edges.
fn pick_path(compat: Compat, branches: &[Branch]) -> DtPath {
    let all: Vec<DtPath> = (branches.iter())
        .flat_map(|b| &b.patterns)
        .filter(|(_, p)| needs_tests(p))
        .map(|(path, _)| path.clone())
        .collect();
    let defaults = |path: &DtPath| {
        (branches.iter())
            .filter(|b| is_irrelevant_to(path, b))
            .count()
    };
    let tied = bests(all.into_iter().map(|p| {
        let weight = defaults(&p);
        (p, weight)
    }));
    if tied.len() == 1 {
        return tied.into_iter().next().expect("one path");
    }
    let branching = |path: &DtPath| {
        let (edges, fallback) = gather_edges(compat, branches, path);
        edges.len() + usize::from(!fallback.is_empty())
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
            Some(min) if weight == min => paths.insert(0, path),
            Some(min) if weight > min => {}
            _ => {
                least = Some(weight);
                paths = vec![path];
            }
        }
    }
    paths
}

/// `treeToDecider`.
fn to_decider(tree: Tree) -> Decider<usize> {
    match tree {
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
            (_, None) => {
                let (_, fallback) = edges.pop().expect("many edges");
                Decider::FanOut {
                    path,
                    tests: (edges.into_iter())
                        .map(|(test, tree)| (test, to_decider(tree)))
                        .collect(),
                    fallback: Box::new(to_decider(fallback)),
                }
            }
            (_, Some(fallback)) => Decider::FanOut {
                path,
                tests: (edges.into_iter())
                    .map(|(test, tree)| (test, to_decider(tree)))
                    .collect(),
                fallback: Box::new(to_decider(*fallback)),
            },
        },
    }
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
            tests.insert(0, (path, test));
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
    match decider {
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
    }
}

/// `insertChoices`: each leaf's branch, inlined or a jump.
fn insert_choices(
    choices: &mut BTreeMap<usize, Choice>,
    decider: Decider<usize>,
) -> Decider<Choice> {
    match decider {
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
            tests,
            success,
            failure,
        } => Decider::Chain {
            tests,
            success: Box::new(insert_choices(choices, *success)),
            failure: Box::new(insert_choices(choices, *failure)),
        },
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
    }
}
