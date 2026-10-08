//! `Parse/Module.hs`: the module, and the checks `checkModule` makes of
//! its ports and effects.

use super::declaration::{self, Declared};
use super::engine::{
    P, check_fresh_line, check_indent, chomp, chomp_and_check_indent, doc_comment, one_of,
    one_of_with_fallback, specialize, word1, word2,
};
use super::error::{Error, Exposing as X, Module as M, Region};
use super::literal::{keyword, lower, module_name, operator, upper};
use crate::Source;

/// What the header says of effects, and where it says it.
enum Effects {
    None(Region),
    Ports(Region),
    Manager(Region),
}

/// `fromByteString` then `checkModule`.
pub(super) fn module(p: &mut P<'_>, source: Source) -> Result<(), Error> {
    let (effects, ports) = chomp_module(p, source).map_err(Error::ParseError)?;
    if p.pos != p.len() {
        return Err(Error::ParseError(M::ModuleBadEnd(p.cur)));
    }
    let package = source != Source::Application;
    // `categorizeDecls` gathers the ports last first.
    let last_port = ports.last();
    match effects {
        None => Ok(()),
        Some(Effects::None(region)) => match last_port {
            None => Ok(()),
            Some((name, name_region)) if package => {
                Err(Error::NoPortsInPackage(name.clone(), *name_region))
            }
            Some(_) => Err(Error::UnexpectedPort(region)),
        },
        Some(Effects::Ports(region)) if package => Err(Error::NoPortModulesInPackage(region)),
        Some(Effects::Ports(region)) => match last_port {
            None => Err(Error::NoPorts(region)),
            Some(_) => Ok(()),
        },
        Some(Effects::Manager(region)) if source == Source::Kernel => match last_port {
            None => Ok(()),
            Some(_) => Err(Error::UnexpectedPort(region)),
        },
        Some(Effects::Manager(region)) => Err(Error::NoEffectsOutsideKernel(region)),
    }
}

type Ports = Vec<(super::error::Name, Region)>;

fn chomp_module(p: &mut P<'_>, source: Source) -> Result<(Option<Effects>, Ports), M> {
    let effects = header(p)?;
    loop {
        let more = one_of_with_fallback(p, &mut [&mut |p| import(p).map(|()| true)], false)?;
        if !more {
            break;
        }
    }
    if source == Source::Kernel {
        loop {
            let more = one_of_with_fallback(
                p,
                &mut [&mut |p| declaration::infix(p).map(|()| true)],
                false,
            )?;
            if !more {
                break;
            }
        }
    }
    let ports = specialize(p, M::Declarations, |p| {
        let mut ports = Vec::new();
        let mut declared = Some(declaration::declaration(p)?);
        while let Some(decl) = declared {
            if let Declared::Port(name, region) = decl {
                ports.push((name, region));
            }
            declared = one_of_with_fallback(
                p,
                &mut [&mut |p| {
                    check_fresh_line(p, super::error::Decl::DeclStart)?;
                    declaration::declaration(p).map(Some)
                }],
                None,
            )?;
        }
        Ok(ports)
    })?;
    Ok((effects, ports))
}

fn fresh_line(p: &mut P<'_>, to_error: fn(super::error::Cursor) -> M) -> Result<(), M> {
    chomp(p, M::ModuleSpace)?;
    check_fresh_line(p, to_error)
}

/// `chompModuleDocCommentSpace`.
fn doc_comment_space(p: &mut P<'_>) -> Result<(), M> {
    fresh_line(p, M::FreshLine)?;
    one_of_with_fallback(
        p,
        &mut [&mut |p| {
            doc_comment(p, M::ImportStart, M::ModuleSpace)?;
            chomp(p, M::ModuleSpace)?;
            check_fresh_line(p, M::FreshLine)
        }],
        (),
    )
}

// HEADER

fn header(p: &mut P<'_>) -> Result<Option<Effects>, M> {
    fresh_line(p, M::FreshLine)?;
    let start = p.cur;
    one_of_with_fallback(
        p,
        &mut [
            &mut |p| {
                keyword(p, b"module", M::ModuleProblem)?;
                let effect_end = p.cur;
                chomp_and_check_indent(p, M::ModuleSpace, M::ModuleProblem)?;
                module_name(p, M::ModuleName)?;
                chomp_and_check_indent(p, M::ModuleSpace, M::ModuleProblem)?;
                keyword(p, b"exposing", M::ModuleProblem)?;
                chomp_and_check_indent(p, M::ModuleSpace, M::ModuleProblem)?;
                specialize(p, M::ModuleExposing, exposing)?;
                doc_comment_space(p)?;
                Ok(Some(Effects::None(Region {
                    start,
                    end: effect_end,
                })))
            },
            &mut |p| {
                keyword(p, b"port", M::PortModuleProblem)?;
                chomp_and_check_indent(p, M::ModuleSpace, M::PortModuleProblem)?;
                keyword(p, b"module", M::PortModuleProblem)?;
                let effect_end = p.cur;
                chomp_and_check_indent(p, M::ModuleSpace, M::PortModuleProblem)?;
                module_name(p, M::PortModuleName)?;
                chomp_and_check_indent(p, M::ModuleSpace, M::PortModuleProblem)?;
                keyword(p, b"exposing", M::PortModuleProblem)?;
                chomp_and_check_indent(p, M::ModuleSpace, M::PortModuleProblem)?;
                specialize(p, M::PortModuleExposing, exposing)?;
                doc_comment_space(p)?;
                Ok(Some(Effects::Ports(Region {
                    start,
                    end: effect_end,
                })))
            },
            &mut |p| {
                keyword(p, b"effect", M::Effect)?;
                chomp_and_check_indent(p, M::ModuleSpace, M::Effect)?;
                keyword(p, b"module", M::Effect)?;
                let effect_end = p.cur;
                chomp_and_check_indent(p, M::ModuleSpace, M::Effect)?;
                module_name(p, M::ModuleName)?;
                chomp_and_check_indent(p, M::ModuleSpace, M::Effect)?;
                keyword(p, b"where", M::Effect)?;
                chomp_and_check_indent(p, M::ModuleSpace, M::Effect)?;
                manager(p)?;
                chomp_and_check_indent(p, M::ModuleSpace, M::Effect)?;
                keyword(p, b"exposing", M::Effect)?;
                chomp_and_check_indent(p, M::ModuleSpace, M::Effect)?;
                specialize(p, |_, c| M::Effect(c), exposing)?;
                doc_comment_space(p)?;
                Ok(Some(Effects::Manager(Region {
                    start,
                    end: effect_end,
                })))
            },
        ],
        None,
    )
}

fn spaces_em(p: &mut P<'_>) -> Result<(), M> {
    chomp_and_check_indent(p, M::ModuleSpace, M::Effect)
}

/// `chompManager`: `{ command = A }`, `{ subscription = B }`, or both
/// in either order.
fn manager(p: &mut P<'_>) -> Result<(), M> {
    word1(p, b'{', M::Effect)?;
    spaces_em(p)?;
    let one = |first: &'static [u8], second: &'static [u8]| {
        move |p: &mut P<'_>| -> Result<(), M> {
            effect_type(p, first)?;
            spaces_em(p)?;
            one_of(
                p,
                M::Effect,
                &mut [
                    &mut |p| {
                        word1(p, b'}', M::Effect)?;
                        spaces_em(p)
                    },
                    &mut |p| {
                        word1(p, b',', M::Effect)?;
                        spaces_em(p)?;
                        effect_type(p, second)?;
                        spaces_em(p)?;
                        word1(p, b'}', M::Effect)?;
                        spaces_em(p)
                    },
                ],
            )
        }
    };
    one_of(
        p,
        M::Effect,
        &mut [
            &mut one(b"command", b"subscription"),
            &mut one(b"subscription", b"command"),
        ],
    )
}

/// `chompCommand` or `chompSubscription`.
fn effect_type(p: &mut P<'_>, word: &[u8]) -> Result<(), M> {
    keyword(p, word, M::Effect)?;
    spaces_em(p)?;
    word1(p, b'=', M::Effect)?;
    spaces_em(p)?;
    upper(p, M::Effect).map(drop)
}

// IMPORTS

fn import(p: &mut P<'_>) -> Result<(), M> {
    keyword(p, b"import", M::ImportStart)?;
    chomp_and_check_indent(p, M::ModuleSpace, M::ImportIndentName)?;
    module_name(p, M::ImportName)?;
    let end = p.cur;
    chomp(p, M::ModuleSpace)?;
    one_of(
        p,
        M::ImportEnd,
        &mut [&mut |p| check_fresh_line(p, M::ImportEnd), &mut |p| {
            check_indent(p, end, M::ImportEnd)?;
            one_of(p, M::ImportAs, &mut [&mut import_as, &mut import_exposing])
        }],
    )
}

fn import_as(p: &mut P<'_>) -> Result<(), M> {
    keyword(p, b"as", M::ImportAs)?;
    chomp_and_check_indent(p, M::ModuleSpace, M::ImportIndentAlias)?;
    upper(p, M::ImportAlias)?;
    let end = p.cur;
    chomp(p, M::ModuleSpace)?;
    one_of(
        p,
        M::ImportEnd,
        &mut [&mut |p| check_fresh_line(p, M::ImportEnd), &mut |p| {
            check_indent(p, end, M::ImportEnd)?;
            import_exposing(p)
        }],
    )
}

fn import_exposing(p: &mut P<'_>) -> Result<(), M> {
    keyword(p, b"exposing", M::ImportExposing)?;
    chomp_and_check_indent(p, M::ModuleSpace, M::ImportIndentExposingList)?;
    specialize(p, M::ImportExposingList, exposing)?;
    fresh_line(p, M::ImportEnd)
}

// LISTING

fn exposing(p: &mut P<'_>) -> Result<(), X> {
    word1(p, b'(', X::ExposingStart)?;
    chomp_and_check_indent(p, X::ExposingSpace, X::ExposingIndentValue)?;
    one_of(
        p,
        X::ExposingValue,
        &mut [
            &mut |p| {
                word2(p, b'.', b'.', X::ExposingValue)?;
                chomp_and_check_indent(p, X::ExposingSpace, X::ExposingIndentEnd)?;
                word1(p, b')', X::ExposingEnd)
            },
            &mut |p| {
                exposed(p)?;
                chomp_and_check_indent(p, X::ExposingSpace, X::ExposingIndentEnd)?;
                loop {
                    let more = one_of(
                        p,
                        X::ExposingEnd,
                        &mut [
                            &mut |p| {
                                word1(p, b',', X::ExposingEnd)?;
                                chomp_and_check_indent(
                                    p,
                                    X::ExposingSpace,
                                    X::ExposingIndentValue,
                                )?;
                                exposed(p)?;
                                chomp_and_check_indent(p, X::ExposingSpace, X::ExposingIndentEnd)?;
                                Ok(true)
                            },
                            &mut |p| word1(p, b')', X::ExposingEnd).map(|()| false),
                        ],
                    )?;
                    if !more {
                        return Ok(());
                    }
                }
            },
        ],
    )
}

fn exposed(p: &mut P<'_>) -> Result<(), X> {
    one_of(
        p,
        X::ExposingValue,
        &mut [
            &mut |p| lower(p, X::ExposingValue).map(drop),
            &mut |p| {
                word1(p, b'(', X::ExposingValue)?;
                operator(p, X::ExposingOperator, X::ExposingOperatorReserved)?;
                word1(p, b')', X::ExposingOperatorRightParen)
            },
            &mut |p| {
                upper(p, X::ExposingValue)?;
                chomp_and_check_indent(p, X::ExposingSpace, X::ExposingIndentEnd)?;
                privacy(p)
            },
        ],
    )
}

fn privacy(p: &mut P<'_>) -> Result<(), X> {
    one_of_with_fallback(
        p,
        &mut [&mut |p| {
            word1(p, b'(', X::ExposingTypePrivacy)?;
            chomp_and_check_indent(p, X::ExposingSpace, X::ExposingTypePrivacy)?;
            word2(p, b'.', b'.', X::ExposingTypePrivacy)?;
            chomp_and_check_indent(p, X::ExposingSpace, X::ExposingTypePrivacy)?;
            word1(p, b')', X::ExposingTypePrivacy)
        }],
        (),
    )
}
