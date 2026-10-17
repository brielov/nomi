//! Upstream's two kernel rules: `Name.isKernel`, a module of hand-written
//! JavaScript, and `Pkg.isKernel`, an author whose packages may hold it.

/// What every kernel module's name starts with.
pub const MODULE_PREFIX: &str = "Elm.Kernel.";

/// A kernel module's name without [`MODULE_PREFIX`], if it is one.
#[inline]
pub fn short(module: &str) -> Option<&str> {
    module.strip_prefix(MODULE_PREFIX)
}

/// `Name.isKernel`.
#[inline]
pub fn is_module(module: &str) -> bool {
    short(module).is_some()
}

/// `Pkg.isKernel` of a package's author: `elm` or `elm-explorations`.
#[inline]
pub fn is_author(author: &str) -> bool {
    author == "elm" || author == "elm-explorations"
}

/// The author of the packages nomi bundles, whose kernel code nomi trusts
/// where it serves them from its bundle, never by their name.
pub const BUNDLED_AUTHOR: &str = "nomi";

/// Whether a package's modules may use kernel code: [`is_author`]'s
/// packages may, and so may those nomi bundles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kernels {
    Allowed,
    Refused,
}

impl Kernels {
    /// What `Pkg.isKernel` allows the package `author/project`.
    #[inline]
    pub fn of_name(package: &str) -> Kernels {
        match package.split_once('/') {
            Some((author, _)) if is_author(author) => Kernels::Allowed,
            _ => Kernels::Refused,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_names() {
        assert!(is_module("Elm.Kernel.List") && !is_module("Elm.Kernelish"));
        assert_eq!(short("Elm.Kernel.List"), Some("List"));
        assert_eq!(short("List"), None);
        assert!(is_author("elm") && is_author("elm-explorations"));
        assert!(!is_author("elm-community") && !is_author(BUNDLED_AUTHOR));
        assert_eq!(Kernels::of_name("elm/core"), Kernels::Allowed);
        assert_eq!(Kernels::of_name("nomi/core"), Kernels::Refused);
        assert_eq!(Kernels::of_name("author/project"), Kernels::Refused);
    }
}
