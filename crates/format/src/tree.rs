//! The formatter tree, mirroring elm-format's AST (`AST/V0_16.hs` and its
//! neighbors).

/// Whether a construct's parts go on one line or one per line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Multiline {
    JoinAll,
    SplitAll,
}

/// How a function application's lines break.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApplicationMultiline {
    /// The function and every argument on its own line.
    SplitFirst,
    /// The first argument on the function's line; the rest joined to it
    /// too with `JoinAll`.
    JoinFirst(Multiline),
}
