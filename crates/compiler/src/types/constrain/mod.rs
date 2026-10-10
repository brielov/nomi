//! Constraint generation (`Type/Constrain/*.hs`): what each expression,
//! pattern and declaration requires of the types involved, in the order
//! upstream generates it, which is the order the solver meets problems.

pub mod expr;
pub mod pattern;
