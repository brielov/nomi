//! What a program is generated for: where it runs, and the shape of its
//! file. Upstream's is the browser's, an IIFE.

/// Where a program runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Platform {
    #[default]
    Browser,
    Node,
}

/// The shape of a program's file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// A function called on `this`, which it adds `Elm` to: upstream's.
    #[default]
    Iife,
    /// An ES module exporting `Elm`; a module's `this` is undefined.
    EsModule,
}

/// What a program is generated for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub platform: Platform,
    pub format: Format,
}
