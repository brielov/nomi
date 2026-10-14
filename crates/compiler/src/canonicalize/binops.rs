//! Arranging an operator chain by precedence and associativity
//! (`Expression.hs`'s `runBinopStepper`), which turns `a + b * c` into
//! `a + (b * c)`, and refuses operators of one precedence that group
//! different ways, or not at all.

use nomi_base::TextRange;

use super::error::{Error, R, throw};
use crate::hir::{Associativity, Fixity};

/// An operator of a chain: its name, which a report of it shows, and how
/// it groups.
#[derive(Clone, Copy, Debug)]
pub(super) struct Op<'a> {
    pub(super) name: &'a str,
    pub(super) fixity: Fixity,
}

/// `runBinopStepper`: the chain `operands[0] ops[0] operands[1] …`, where
/// `chain` pairs each operand but the last with the operator after it,
/// arranged, `join` applying the operator at a place in the chain to its
/// operands; or the first two operators that can't be, at `region`.
///
/// Upstream's stepper nests a level for each rise in precedence; the
/// operators still waiting for their right operand are those levels.
pub(super) fn arrange<'a, T>(
    chain: impl IntoIterator<Item = (T, Op<'a>)>,
    last: T,
    region: TextRange,
    mut join: impl FnMut(usize, T, T) -> T,
) -> R<T> {
    let mut operands = Vec::new();
    let mut waiting: Vec<(usize, Op<'a>)> = Vec::new();
    for (index, (operand, op)) in chain.into_iter().enumerate() {
        operands.push(operand);
        while let Some(&(at, root)) = waiting.last() {
            let (outer, inner) = (root.fixity, op.fixity);
            if outer.precedence < inner.precedence {
                break;
            }
            if outer.precedence == inner.precedence {
                match (outer.associativity, inner.associativity) {
                    (Associativity::Left, Associativity::Left) => {}
                    (Associativity::Right, Associativity::Right) => break,
                    _ => {
                        return throw(Error::Binop {
                            range: region,
                            left: root.name.to_owned(),
                            right: op.name.to_owned(),
                        });
                    }
                }
            }
            waiting.pop();
            apply(&mut operands, at, &mut join);
        }
        waiting.push((index, op));
    }
    operands.push(last);
    while let Some((at, _)) = waiting.pop() {
        apply(&mut operands, at, &mut join);
    }
    Ok(operands.pop().expect("one operand left"))
}

/// The operator at `at` applied to the last two operands.
fn apply<T>(operands: &mut Vec<T>, at: usize, join: &mut impl FnMut(usize, T, T) -> T) {
    let right = operands.pop().expect("a right operand");
    let left = operands.pop().expect("a left operand");
    operands.push(join(at, left, right));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chain `0 ops[0] 1 ops[1] …` arranged, each operator applied
    /// shown as `(left name right)`.
    fn arranged(ops: &[(&str, Associativity, i64)]) -> Result<String, (String, String)> {
        let chain = ops
            .iter()
            .enumerate()
            .map(|(i, &(name, associativity, precedence))| {
                let fixity = Fixity {
                    associativity,
                    precedence,
                };
                (i.to_string(), Op { name, fixity })
            });
        let join = |at: usize, left, right| format!("({left} {} {right})", ops[at].0);
        arrange(chain, ops.len().to_string(), TextRange::default(), join).map_err(|errors| {
            match &errors[..] {
                [Error::Binop { left, right, .. }] => (left.clone(), right.clone()),
                _ => panic!("one operator error: {errors:?}"),
            }
        })
    }

    /// `a + b * c - d`, and `a :: b :: c`.
    #[test]
    fn precedence_and_associativity_shape_the_tree() {
        use Associativity::{Left, Right};
        let ops = [("+", Left, 6), ("*", Left, 7), ("-", Left, 6)];
        assert_eq!(arranged(&ops).unwrap(), "((0 + (1 * 2)) - 3)");
        let ops = [("::", Right, 5), ("::", Right, 5)];
        assert_eq!(arranged(&ops).unwrap(), "(0 :: (1 :: 2))");
    }

    #[test]
    fn operators_that_group_differently_are_refused() {
        use Associativity::{Left, Non, Right};
        let ops = [("==", Non, 4), ("==", Non, 4)];
        assert_eq!(arranged(&ops), Err(("==".into(), "==".into())));
        let ops = [("<|", Right, 0), ("|>", Left, 0)];
        assert_eq!(arranged(&ops), Err(("<|".into(), "|>".into())));
    }
}
