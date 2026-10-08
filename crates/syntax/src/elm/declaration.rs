//! `Parse/Declaration.hs`.

use super::engine::{
    P, check_fresh_line, check_indent, chomp, chomp_and_check_indent, doc_comment, in_context,
    one_of, one_of_with_fallback, specialize, word1,
};
use super::error::{
    Cursor, CustomType as CT, Decl, DeclDef, DeclType as DT, Module, Name, Port, Region,
    TypeAlias as TA,
};
use super::literal::{keyword, lower, operator, upper};
use super::{expression, pattern, types};

/// What a declaration was, as far as `checkModule` cares.
pub(super) enum Declared {
    Other,
    /// A port, and where its name is.
    Port(Name, Region),
}

/// `declaration`.
pub(super) fn declaration(p: &mut P<'_>) -> Result<Declared, Decl> {
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
            &mut |p| type_decl(p).map(|()| Declared::Other),
            &mut port_decl,
            &mut |p| value_decl(p).map(|()| Declared::Other),
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
                    matching_name(p, &name)?;
                    chomp_and_check_indent(p, DeclDef::DeclDefSpace, DeclDef::DeclDefIndentEquals)?;
                    args_and_body(p)
                },
                &mut args_and_body,
            ],
        )
    })
}

fn args_and_body(p: &mut P<'_>) -> Result<(), DeclDef> {
    loop {
        let done = one_of(
            p,
            DeclDef::DeclDefEquals,
            &mut [
                &mut |p| {
                    specialize(p, DeclDef::DeclDefArg, pattern::term)?;
                    chomp_and_check_indent(p, DeclDef::DeclDefSpace, DeclDef::DeclDefIndentEquals)?;
                    Ok(false)
                },
                &mut |p| {
                    word1(p, b'=', DeclDef::DeclDefEquals)?;
                    chomp_and_check_indent(p, DeclDef::DeclDefSpace, DeclDef::DeclDefIndentBody)?;
                    specialize(p, DeclDef::DeclDefBody, expression::expression)?;
                    Ok(true)
                },
            ],
        )?;
        if done {
            return Ok(());
        }
    }
}

/// `chompMatchingName`: the name an annotation is for, again.
fn matching_name(p: &mut P<'_>, expected: &Name) -> Result<(), DeclDef> {
    let start = p.cur;
    let name = lower(p, DeclDef::DeclDefNameRepeat)?;
    if name == *expected {
        Ok(())
    } else {
        p.cerr(DeclDef::DeclDefNameMatch(name, start))
    }
}

// TYPE DECLARATIONS

fn type_decl(p: &mut P<'_>) -> Result<(), Decl> {
    in_context(
        p,
        Decl::DeclType,
        |p| keyword(p, b"type", Decl::DeclStart),
        |p| {
            chomp_and_check_indent(p, DT::DtSpace, DT::DtIndentName)?;
            one_of(
                p,
                DT::DtName,
                &mut [
                    &mut |p| {
                        in_context(
                            p,
                            DT::DtAlias,
                            |p| keyword(p, b"alias", DT::DtName),
                            |p| {
                                chomp_and_check_indent(p, TA::AliasSpace, TA::AliasIndentEquals)?;
                                upper(p, TA::AliasName)?;
                                chomp_and_check_indent(p, TA::AliasSpace, TA::AliasIndentEquals)?;
                                loop {
                                    let equals = one_of(
                                        p,
                                        TA::AliasEquals,
                                        &mut [
                                            &mut |p| {
                                                lower(p, TA::AliasEquals)?;
                                                chomp_and_check_indent(
                                                    p,
                                                    TA::AliasSpace,
                                                    TA::AliasIndentEquals,
                                                )?;
                                                Ok(false)
                                            },
                                            &mut |p| {
                                                word1(p, b'=', TA::AliasEquals)?;
                                                chomp_and_check_indent(
                                                    p,
                                                    TA::AliasSpace,
                                                    TA::AliasIndentBody,
                                                )?;
                                                Ok(true)
                                            },
                                        ],
                                    )?;
                                    if equals {
                                        break;
                                    }
                                }
                                specialize(p, TA::AliasBody, types::expression).map(drop)
                            },
                        )
                    },
                    &mut |p| specialize(p, DT::DtUnion, custom_type),
                ],
            )
        },
    )
}

fn custom_type(p: &mut P<'_>) -> Result<(), CT> {
    upper(p, CT::CtName)?;
    chomp_and_check_indent(p, CT::CtSpace, CT::CtIndentEquals)?;
    loop {
        let equals = one_of(
            p,
            CT::CtEquals,
            &mut [
                &mut |p| {
                    lower(p, CT::CtEquals)?;
                    chomp_and_check_indent(p, CT::CtSpace, CT::CtIndentEquals)?;
                    Ok(false)
                },
                &mut |p| {
                    word1(p, b'=', CT::CtEquals)?;
                    chomp_and_check_indent(p, CT::CtSpace, CT::CtIndentAfterEquals)?;
                    Ok(true)
                },
            ],
        )?;
        if equals {
            break;
        }
    }
    let mut end = types::variant(p)?;
    loop {
        let next = one_of_with_fallback(
            p,
            &mut [&mut |p| {
                check_indent(p, end, CT::CtIndentBar)?;
                word1(p, b'|', CT::CtBar)?;
                chomp_and_check_indent(p, CT::CtSpace, CT::CtIndentAfterBar)?;
                types::variant(p).map(Some)
            }],
            None,
        )?;
        match next {
            Some(new_end) => end = new_end,
            None => return Ok(()),
        }
    }
}

// PORT

fn port_decl(p: &mut P<'_>) -> Result<Declared, Decl> {
    in_context(
        p,
        Decl::Port,
        |p| keyword(p, b"port", Decl::DeclStart),
        |p| {
            chomp_and_check_indent(p, Port::PortSpace, Port::PortIndentName)?;
            let start = p.cur;
            let name = lower(p, Port::PortName)?;
            let region = Region { start, end: p.cur };
            chomp_and_check_indent(p, Port::PortSpace, Port::PortIndentColon)?;
            word1(p, b':', Port::PortColon)?;
            chomp_and_check_indent(p, Port::PortSpace, Port::PortIndentType)?;
            specialize(p, Port::PortType, types::expression)?;
            Ok(Declared::Port(name, region))
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
fn precedence<X>(p: &mut P<'_>, to_expectation: impl FnOnce(Cursor) -> X) -> Result<(), X> {
    match p.peek(0) {
        Some(b'0'..=b'9') => {
            p.ok_at(p.pos + 1, p.slide(p.cur, 1));
            Ok(())
        }
        _ => Err(to_expectation(p.cur)),
    }
}
