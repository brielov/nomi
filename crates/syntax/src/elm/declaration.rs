//! `Parse/Declaration.hs`.

use std::ops::ControlFlow;

use super::engine::{
    P, check_fresh_line, check_indent, chomp, chomp_and_check_indent, doc_comment, fold_more,
    in_context, one_of, one_of_with_fallback, repeat, specialize, word1,
};
use super::error::{
    CustomType, Decl, DeclDef, DeclType, Module, Port, Position, Region, Space, TypeAlias,
};
use super::literal::{keyword, lower, matching_name, operator, upper};
use super::{expression, pattern, types};

/// `declaration`, and where its name is if it is a port: all that
/// `checkModule` reads of one.
pub(super) fn declaration(p: &mut P<'_>) -> Result<Option<Region>, Decl> {
    one_of_with_fallback(
        p,
        &mut [&mut |p| {
            doc_comment(p, Decl::DeclStart, Decl::DeclSpace)?;
            chomp(p, Decl::DeclSpace)?;
            check_fresh_line(p, Decl::DeclFreshLineAfterDocComment)
        }],
        (),
    )?;
    one_of(
        p,
        Decl::DeclStart,
        &mut [
            &mut |p| type_decl(p).map(|()| None),
            &mut |p| port_decl(p).map(Some),
            &mut |p| value_decl(p).map(|()| None),
        ],
    )
}

// DEFINITION and ANNOTATION

fn value_decl(p: &mut P<'_>) -> Result<(), Decl> {
    let name = lower(p, Decl::DeclStart)?;
    let wrap = |e, c| Decl::DeclDef(name.clone(), e, c);
    specialize(p, wrap, |p| {
        chomp_and_check_indent(p, DeclDef::DeclDefSpace, DeclDef::DeclDefIndentEquals)?;
        one_of(
            p,
            DeclDef::DeclDefEquals,
            &mut [
                &mut |p| {
                    word1(p, b':', DeclDef::DeclDefEquals)?;
                    chomp_and_check_indent(p, DeclDef::DeclDefSpace, DeclDef::DeclDefIndentType)?;
                    specialize(p, DeclDef::DeclDefType, types::expression)?;
                    check_fresh_line(p, DeclDef::DeclDefNameRepeat)?;
                    matching_name(
                        p,
                        &name,
                        DeclDef::DeclDefNameRepeat,
                        DeclDef::DeclDefNameMatch,
                    )?;
                    chomp_and_check_indent(p, DeclDef::DeclDefSpace, DeclDef::DeclDefIndentEquals)?;
                    args_and_body(p)
                },
                &mut args_and_body,
            ],
        )
    })
}

fn args_and_body(p: &mut P<'_>) -> Result<(), DeclDef> {
    repeat(p, |p| {
        one_of(
            p,
            DeclDef::DeclDefEquals,
            &mut [
                &mut |p| {
                    specialize(p, DeclDef::DeclDefArg, pattern::term)?;
                    chomp_and_check_indent(p, DeclDef::DeclDefSpace, DeclDef::DeclDefIndentEquals)?;
                    Ok(ControlFlow::Continue(()))
                },
                &mut |p| {
                    word1(p, b'=', DeclDef::DeclDefEquals)?;
                    chomp_and_check_indent(p, DeclDef::DeclDefSpace, DeclDef::DeclDefIndentBody)?;
                    specialize(p, DeclDef::DeclDefBody, expression::expression)?;
                    Ok(ControlFlow::Break(()))
                },
            ],
        )
    })
}

// TYPE DECLARATIONS

fn type_decl(p: &mut P<'_>) -> Result<(), Decl> {
    in_context(
        p,
        Decl::DeclType,
        |p| keyword(p, b"type", Decl::DeclStart),
        |p| {
            chomp_and_check_indent(p, DeclType::DtSpace, DeclType::DtIndentName)?;
            one_of(
                p,
                DeclType::DtName,
                &mut [
                    &mut |p| {
                        in_context(
                            p,
                            DeclType::DtAlias,
                            |p| keyword(p, b"alias", DeclType::DtName),
                            type_alias,
                        )
                    },
                    &mut |p| specialize(p, DeclType::DtUnion, custom_type),
                ],
            )
        },
    )
}

/// `typeAlias`, after `type alias`.
fn type_alias(p: &mut P<'_>) -> Result<(), TypeAlias> {
    chomp_and_check_indent(p, TypeAlias::AliasSpace, TypeAlias::AliasIndentEquals)?;
    upper(p, TypeAlias::AliasName)?;
    chomp_and_check_indent(p, TypeAlias::AliasSpace, TypeAlias::AliasIndentEquals)?;
    vars_to_equals(
        p,
        TypeAlias::AliasEquals,
        TypeAlias::AliasSpace,
        TypeAlias::AliasIndentEquals,
    )?;
    chomp_and_check_indent(p, TypeAlias::AliasSpace, TypeAlias::AliasIndentBody)?;
    specialize(p, TypeAlias::AliasBody, types::expression).map(drop)
}

fn custom_type(p: &mut P<'_>) -> Result<(), CustomType> {
    upper(p, CustomType::CtName)?;
    chomp_and_check_indent(p, CustomType::CtSpace, CustomType::CtIndentEquals)?;
    vars_to_equals(
        p,
        CustomType::CtEquals,
        CustomType::CtSpace,
        CustomType::CtIndentEquals,
    )?;
    chomp_and_check_indent(p, CustomType::CtSpace, CustomType::CtIndentAfterEquals)?;
    let first = types::variant(p)?;
    fold_more(p, first, |p, end| {
        check_indent(p, end, CustomType::CtIndentBar)?;
        word1(p, b'|', CustomType::CtBar)?;
        chomp_and_check_indent(p, CustomType::CtSpace, CustomType::CtIndentAfterBar)?;
        types::variant(p)
    })
    .map(drop)
}

/// `chompAliasNameToEqualsHelp` and `chompCustomNameToEqualsHelp`: a
/// type's variables, up to its `=`.
fn vars_to_equals<X>(
    p: &mut P<'_>,
    to_equals: fn(Position) -> X,
    to_space: fn(Space, Position) -> X,
    to_indent_equals: fn(Position) -> X,
) -> Result<(), X> {
    repeat(p, |p| {
        one_of(
            p,
            to_equals,
            &mut [
                &mut |p| {
                    lower(p, to_equals)?;
                    chomp_and_check_indent(p, to_space, to_indent_equals)?;
                    Ok(ControlFlow::Continue(()))
                },
                &mut |p| word1(p, b'=', to_equals).map(ControlFlow::Break),
            ],
        )
    })
}

// PORT

/// A port, and where its name is.
fn port_decl(p: &mut P<'_>) -> Result<Region, Decl> {
    in_context(
        p,
        Decl::Port,
        |p| keyword(p, b"port", Decl::DeclStart),
        |p| {
            chomp_and_check_indent(p, Port::PortSpace, Port::PortIndentName)?;
            let start = p.cur;
            lower(p, Port::PortName)?;
            let name = Region { start, end: p.cur };
            chomp_and_check_indent(p, Port::PortSpace, Port::PortIndentColon)?;
            word1(p, b':', Port::PortColon)?;
            chomp_and_check_indent(p, Port::PortSpace, Port::PortIndentType)?;
            specialize(p, Port::PortType, types::expression)?;
            Ok(name)
        },
    )
}

// INFIX

/// `infix_`, which always chomps to a fresh line.
pub(super) fn infix(p: &mut P<'_>) -> Result<(), Module> {
    let err = Module::Infix;
    let space = |_, c| Module::Infix(c);
    keyword(p, b"infix", err)?;
    chomp_and_check_indent(p, space, err)?;
    one_of(
        p,
        err,
        &mut [
            &mut |p| keyword(p, b"left", err),
            &mut |p| keyword(p, b"right", err),
            &mut |p| keyword(p, b"non", err),
        ],
    )?;
    chomp_and_check_indent(p, space, err)?;
    precedence(p, err)?;
    chomp_and_check_indent(p, space, err)?;
    word1(p, b'(', err)?;
    operator(p, err, |_, c| Module::Infix(c))?;
    word1(p, b')', err)?;
    chomp_and_check_indent(p, space, err)?;
    word1(p, b'=', err)?;
    chomp_and_check_indent(p, space, err)?;
    lower(p, err)?;
    chomp(p, space)?;
    check_fresh_line(p, err)
}

/// `precedence`: one digit.
fn precedence<X>(p: &mut P<'_>, to_expectation: impl FnOnce(Position) -> X) -> Result<(), X> {
    match p.peek(0) {
        Some(b'0'..=b'9') => {
            p.advance(1, 1);
            Ok(())
        }
        _ => Err(to_expectation(p.cur)),
    }
}
