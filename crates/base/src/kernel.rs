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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_names() {
        assert!(is_module("Elm.Kernel.List") && !is_module("Elm.Kernelish"));
        assert_eq!(short("Elm.Kernel.List"), Some("List"));
        assert_eq!(short("List"), None);
        assert!(is_author("elm") && is_author("elm-explorations") && !is_author("elm-community"));
    }
}
