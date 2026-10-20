//! Where a program runs, which every crate but the compiler may know
//! (rule R1): codegen for the kernels it links, the project for an
//! entry's.

/// Where a program runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Platform {
    #[default]
    Browser,
    Bun,
}
