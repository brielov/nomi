//! Arranging an operator chain by precedence and associativity
//! (`Expression.hs`'s `runBinopStepper`), which turns `a + b * c` into
//! `a + (b * c)`, and refuses operators of one precedence that group
//! different ways, or not at all.

use nomi_base::{TextRange, grow_large};

use super::error::{Error, R, throw};
use crate::hir::{Associativity, Fixity};

/// An operator of a chain: its place in it, and how it groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Op {
    /// Which of the chain's operators, from the left.
    pub(super) index: usize,
    pub(super) name: String,
    pub(super) fixity: Fixity,
}

/// A chain arranged: an operand, by its place, or an operator applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Tree {
    Operand(usize),
    Binop {
        op: usize,
        left: Box<Tree>,
        right: Box<Tree>,
    },
}

/// `makeBinop`: a tree with a hole where its right operand goes.
type Hole = Box<dyn FnOnce(Tree) -> Tree>;

enum Step {
    Done(Tree),
    More(Vec<(Tree, Op)>, Tree),
    Error(Op, Op),
}

fn binop(op: &Op, left: Tree, right: Tree) -> Tree {
    Tree::Binop {
        op: op.index,
        left: Box::new(left),
        right: Box::new(right),
    }
}

/// `runBinopStepper`: the chain `operands[0] ops[0] operands[1] …`, where
/// `chain` pairs each operand but the last with the operator after it,
/// arranged; or the first two operators that can't be, at `region`.
pub(super) fn arrange(chain: Vec<(Tree, Op)>, last: Tree, region: TextRange) -> R<Tree> {
    let mut step = Step::More(chain, last);
    loop {
        step = match step {
            Step::Done(tree) => return Ok(tree),
            Step::More(chain, last) if chain.is_empty() => return Ok(last),
            Step::More(chain, last) => {
                let mut chain = chain.into_iter();
                let (expr, op) = chain.next().expect("a pair");
                let root = op.clone();
                let hole: Hole = Box::new(move |right| binop(&op, expr, right));
                next(hole, root, chain.collect(), last)
            }
            Step::Error(first, second) => {
                return throw(Error::Binop {
                    range: region,
                    left: first.name,
                    right: second.name,
                });
            }
        };
    }
}

/// `toBinopStep`.
fn next(make: Hole, root: Op, middle: Vec<(Tree, Op)>, last: Tree) -> Step {
    grow_large(|| next_(make, root, middle, last))
}

fn next_(make: Hole, root: Op, middle: Vec<(Tree, Op)>, last: Tree) -> Step {
    let mut middle = middle.into_iter();
    let Some((expr, op)) = middle.next() else {
        return Step::Done(make(last));
    };
    let rest: Vec<(Tree, Op)> = middle.collect();
    if op.fixity.precedence < root.fixity.precedence {
        let mut more = vec![(make(expr), op)];
        more.extend(rest);
        return Step::More(more, last);
    }
    if op.fixity.precedence > root.fixity.precedence {
        let inner_op = op.clone();
        let inner: Hole = Box::new(move |right| binop(&inner_op, expr, right));
        return match next(inner, op, rest, last) {
            Step::Done(new_last) => Step::Done(make(new_last)),
            Step::More(new_middle, new_last) => next(make, root, new_middle, new_last),
            error @ Step::Error(..) => error,
        };
    }
    match (root.fixity.associativity, op.fixity.associativity) {
        (Associativity::Left, Associativity::Left) => {
            let left = make(expr);
            let next_op = op.clone();
            next(
                Box::new(move |right| binop(&next_op, left, right)),
                op,
                rest,
                last,
            )
        }
        (Associativity::Right, Associativity::Right) => {
            let next_op = op.clone();
            let hole: Hole = Box::new(move |right| make(binop(&next_op, expr, right)));
            next(hole, op, rest, last)
        }
        _ => Step::Error(root, op),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(index: usize, name: &str, associativity: Associativity, precedence: i64) -> Op {
        Op {
            index,
            name: name.to_owned(),
            fixity: Fixity {
                associativity,
                precedence,
            },
        }
    }

    fn leaf(i: usize) -> Tree {
        Tree::Operand(i)
    }

    fn node(index: usize, left: Tree, right: Tree) -> Tree {
        binop(&op(index, "", Associativity::Left, 0), left, right)
    }

    /// `a + b * c - d`.
    #[test]
    fn precedence_and_associativity_shape_the_tree() {
        let plus = |i| op(i, "+", Associativity::Left, 6);
        let times = op(1, "*", Associativity::Left, 7);
        let chain = vec![(leaf(0), plus(0)), (leaf(1), times), (leaf(2), plus(2))];
        let tree = arrange(chain, leaf(3), TextRange::default()).unwrap();
        assert_eq!(
            tree,
            node(2, node(0, leaf(0), node(1, leaf(1), leaf(2))), leaf(3))
        );
        // `a :: b :: c` groups right.
        let cons = |i| op(i, "::", Associativity::Right, 5);
        let tree = arrange(
            vec![(leaf(0), cons(0)), (leaf(1), cons(1))],
            leaf(2),
            TextRange::default(),
        )
        .unwrap();
        assert_eq!(tree, node(0, leaf(0), node(1, leaf(1), leaf(2))));
    }

    #[test]
    fn operators_that_group_differently_are_refused() {
        let eq = |i| op(i, "==", Associativity::Non, 4);
        let errors = arrange(
            vec![(leaf(0), eq(0)), (leaf(1), eq(1))],
            leaf(2),
            TextRange::default(),
        )
        .unwrap_err();
        assert!(
            matches!(&errors[..], [Error::Binop { left, right, .. }] if left == "==" && right == "==")
        );
        let pipe = op(0, "|>", Associativity::Left, 0);
        let back = op(1, "<|", Associativity::Right, 0);
        assert!(
            arrange(
                vec![(leaf(0), back), (leaf(1), pipe)],
                leaf(2),
                TextRange::default()
            )
            .is_err()
        );
    }
}
