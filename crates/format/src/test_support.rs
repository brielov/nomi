//! Builders and harnesses the unit tests share.

use nomi_syntax::{Tool, parse_module, validate};

use crate::tree::{
    Comment, Commented, Comments, ForceMultiline, Lower, Module, Namespace, SeparatorComments,
    Type, TypeConstructor, Upper, UpperRef,
};

/// A `--` comment, `text` after the dashes.
pub(crate) fn line(text: &str) -> Comment {
    Comment::Line(text.to_owned())
}

/// `{- text -}`, on one line.
pub(crate) fn block(text: &str) -> Comment {
    Comment::Block(vec![text.to_owned()])
}

/// The comments of an item after a separator, and its `--` comment.
pub(crate) fn separated(before: Comments, after: Comments, eol: Option<&str>) -> SeparatorComments {
    SeparatorComments {
        before_separator: before,
        after_separator: after,
        eol: eol.map(str::to_owned),
    }
}

pub(crate) fn lower(name: &str) -> Lower {
    Lower(name.to_owned())
}

pub(crate) fn upper(name: &str) -> Upper {
    Upper(name.to_owned())
}

pub(crate) fn names(names: &[&str]) -> Namespace {
    names.iter().map(|n| upper(n)).collect()
}

pub(crate) fn upper_ref(namespace: &[&str], name: &str) -> UpperRef {
    UpperRef {
        namespace: names(namespace),
        name: upper(name),
    }
}

pub(crate) fn code_points(text: &str) -> Vec<u32> {
    text.chars().map(u32::from).collect()
}

/// The type `name` in `namespace`, applied to `args`, all on one line.
pub(crate) fn named(namespace: &[&str], name: &str, args: Vec<Type>) -> Type {
    Type::Construction {
        ctor: TypeConstructor::Named(upper_ref(namespace, name)),
        args: args.into_iter().map(Commented::bare).collect(),
        force_multiline: ForceMultiline(false),
    }
}

/// The formatter tree of the module `text`, which must parse and
/// validate.
pub(crate) fn lowered(text: &str) -> Module {
    let parse = parse_module(text);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    assert_eq!(validate(&parse, Tool::ElmFormat), []);
    crate::lower::module(&parse.syntax(), text)
}

/// Checks that `text` formats to `expected`, elm-format 0.8.8's output.
#[track_caller]
pub(crate) fn assert_formats(text: &str, expected: &str) {
    assert_eq!(crate::format(text).as_deref(), Some(expected), "{text}");
}
