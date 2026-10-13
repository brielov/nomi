//! `Parse/Module.hs`: the module, and the checks `checkModule` makes of
//! its ports and effects.

use super::declaration;
use super::engine::{
    P, check_fresh_line, check_indent, chomp, chomp_and_check_indent, comma_until, doc_comment,
    fold_more, one_of, one_of_with_fallback, specialize, word1, word2,
};
use super::error::{Decl, Error, Exposing, Module, Position, Region};
use super::literal::{keyword, lower, module_name, operator, upper};
use crate::Source;

/// What the header says of effects, and where it says it.
#[expect(clippy::enum_variant_names, reason = "upstream's `Src.Effects`")]
enum Effects {
    NoEffects(Region),
    Ports(Region),
    Manager(Region),
}

/// `fromByteString` then `checkModule`.
pub(super) fn module(p: &mut P<'_>, source: Source) -> Result<(), Error> {
    let (effects, last_port) = chomp_module(p, source).map_err(Error::ParseError)?;
    if p.pos != p.len() {
        return Err(Error::ParseError(Module::ModuleBadEnd(p.cur)));
    }
    let package = source != Source::Application;
    match effects {
        None => Ok(()),
        Some(Effects::NoEffects(region)) => match last_port {
            None => Ok(()),
            Some(port) if package => Err(Error::NoPortsInPackage(port)),
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

/// The header's effects, and where the last port's name is, as
/// `categorizeDecls`, which gathers the ports last first, has it.
fn chomp_module(
    p: &mut P<'_>,
    source: Source,
) -> Result<(Option<Effects>, Option<Region>), Module> {
    let effects = header(p)?;
    fold_more(p, (), |p, ()| import(p))?;
    if source == Source::Kernel {
        fold_more(p, (), |p, ()| declaration::infix(p))?;
    }
    let last_port = specialize(
        p,
        |decl, _| Module::Declarations(decl),
        |p| {
            let first = declaration::declaration(p)?;
            fold_more(p, first, |p, last_port| {
                check_fresh_line(p, Decl::DeclStart)?;
                Ok(declaration::declaration(p)?.or(last_port))
            })
        },
    )?;
    Ok((effects, last_port))
}

fn fresh_line(p: &mut P<'_>, to_error: fn(Position) -> Module) -> Result<(), Module> {
    chomp(p, Module::ModuleSpace)?;
    check_fresh_line(p, to_error)
}

/// `chompModuleDocCommentSpace`.
fn doc_comment_space(p: &mut P<'_>) -> Result<(), Module> {
    fresh_line(p, Module::FreshLine)?;
    one_of_with_fallback(
        p,
        &mut [&mut |p| {
            doc_comment(p, Module::ImportStart, Module::ModuleSpace)?;
            chomp(p, Module::ModuleSpace)?;
            check_fresh_line(p, Module::FreshLine)
        }],
        (),
    )
}

// HEADER

fn header(p: &mut P<'_>) -> Result<Option<Effects>, Module> {
    fresh_line(p, Module::FreshLine)?;
    let start = p.cur;
    let effects_at = |p: &P<'_>| Region { start, end: p.cur };
    one_of_with_fallback(
        p,
        &mut [
            &mut |p| {
                keyword(p, b"module", Module::ModuleProblem)?;
                let effects = Effects::NoEffects(effects_at(p));
                chomp_and_check_indent(p, Module::ModuleSpace, Module::ModuleProblem)?;
                module_name(p, Module::ModuleName)?;
                chomp_and_check_indent(p, Module::ModuleSpace, Module::ModuleProblem)?;
                keyword(p, b"exposing", Module::ModuleProblem)?;
                chomp_and_check_indent(p, Module::ModuleSpace, Module::ModuleProblem)?;
                specialize(p, Module::ModuleExposing, exposing)?;
                doc_comment_space(p)?;
                Ok(Some(effects))
            },
            &mut |p| {
                keyword(p, b"port", Module::PortModuleProblem)?;
                chomp_and_check_indent(p, Module::ModuleSpace, Module::PortModuleProblem)?;
                keyword(p, b"module", Module::PortModuleProblem)?;
                let effects = Effects::Ports(effects_at(p));
                chomp_and_check_indent(p, Module::ModuleSpace, Module::PortModuleProblem)?;
                module_name(p, Module::PortModuleName)?;
                chomp_and_check_indent(p, Module::ModuleSpace, Module::PortModuleProblem)?;
                keyword(p, b"exposing", Module::PortModuleProblem)?;
                chomp_and_check_indent(p, Module::ModuleSpace, Module::PortModuleProblem)?;
                specialize(p, Module::PortModuleExposing, exposing)?;
                doc_comment_space(p)?;
                Ok(Some(effects))
            },
            &mut |p| {
                keyword(p, b"effect", Module::Effect)?;
                effect_space(p)?;
                keyword(p, b"module", Module::Effect)?;
                let effects = Effects::Manager(effects_at(p));
                effect_space(p)?;
                module_name(p, Module::ModuleName)?;
                effect_space(p)?;
                keyword(p, b"where", Module::Effect)?;
                effect_space(p)?;
                manager(p)?;
                effect_space(p)?;
                keyword(p, b"exposing", Module::Effect)?;
                effect_space(p)?;
                specialize(p, |_, at| Module::Effect(at), exposing)?;
                doc_comment_space(p)?;
                Ok(Some(effects))
            },
        ],
        None,
    )
}

/// The spaces between the words of an effect module's header.
fn effect_space(p: &mut P<'_>) -> Result<(), Module> {
    chomp_and_check_indent(p, Module::ModuleSpace, Module::Effect)
}

/// `chompManager`: `{ command = A }`, `{ subscription = B }`, or both
/// in either order.
fn manager(p: &mut P<'_>) -> Result<(), Module> {
    word1(p, b'{', Module::Effect)?;
    effect_space(p)?;
    let one = |first: &'static [u8], second: &'static [u8]| {
        move |p: &mut P<'_>| -> Result<(), Module> {
            effect_type(p, first)?;
            effect_space(p)?;
            one_of(
                p,
                Module::Effect,
                &mut [
                    &mut |p| {
                        word1(p, b'}', Module::Effect)?;
                        effect_space(p)
                    },
                    &mut |p| {
                        word1(p, b',', Module::Effect)?;
                        effect_space(p)?;
                        effect_type(p, second)?;
                        effect_space(p)?;
                        word1(p, b'}', Module::Effect)?;
                        effect_space(p)
                    },
                ],
            )
        }
    };
    one_of(
        p,
        Module::Effect,
        &mut [
            &mut one(b"command", b"subscription"),
            &mut one(b"subscription", b"command"),
        ],
    )
}

/// `chompCommand` or `chompSubscription`.
fn effect_type(p: &mut P<'_>, word: &[u8]) -> Result<(), Module> {
    keyword(p, word, Module::Effect)?;
    effect_space(p)?;
    word1(p, b'=', Module::Effect)?;
    effect_space(p)?;
    upper(p, Module::Effect).map(drop)
}

// IMPORTS

fn import(p: &mut P<'_>) -> Result<(), Module> {
    keyword(p, b"import", Module::ImportStart)?;
    chomp_and_check_indent(p, Module::ModuleSpace, Module::ImportIndentName)?;
    module_name(p, Module::ImportName)?;
    let end = p.cur;
    chomp(p, Module::ModuleSpace)?;
    one_of(
        p,
        Module::ImportEnd,
        &mut [&mut |p| check_fresh_line(p, Module::ImportEnd), &mut |p| {
            check_indent(p, end, Module::ImportEnd)?;
            one_of(
                p,
                Module::ImportAs,
                &mut [&mut import_as, &mut import_exposing],
            )
        }],
    )
}

fn import_as(p: &mut P<'_>) -> Result<(), Module> {
    keyword(p, b"as", Module::ImportAs)?;
    chomp_and_check_indent(p, Module::ModuleSpace, Module::ImportIndentAlias)?;
    upper(p, Module::ImportAlias)?;
    let end = p.cur;
    chomp(p, Module::ModuleSpace)?;
    one_of(
        p,
        Module::ImportEnd,
        &mut [&mut |p| check_fresh_line(p, Module::ImportEnd), &mut |p| {
            check_indent(p, end, Module::ImportEnd)?;
            import_exposing(p)
        }],
    )
}

fn import_exposing(p: &mut P<'_>) -> Result<(), Module> {
    keyword(p, b"exposing", Module::ImportExposing)?;
    chomp_and_check_indent(p, Module::ModuleSpace, Module::ImportIndentExposingList)?;
    specialize(p, Module::ImportExposingList, exposing)?;
    fresh_line(p, Module::ImportEnd)
}

// LISTING

fn exposing(p: &mut P<'_>) -> Result<(), Exposing> {
    word1(p, b'(', Exposing::ExposingStart)?;
    chomp_and_check_indent(p, Exposing::ExposingSpace, Exposing::ExposingIndentValue)?;
    one_of(
        p,
        Exposing::ExposingValue,
        &mut [
            &mut |p| {
                word2(p, b'.', b'.', Exposing::ExposingValue)?;
                chomp_and_check_indent(p, Exposing::ExposingSpace, Exposing::ExposingIndentEnd)?;
                word1(p, b')', Exposing::ExposingEnd)
            },
            &mut |p| {
                exposed(p)?;
                chomp_and_check_indent(p, Exposing::ExposingSpace, Exposing::ExposingIndentEnd)?;
                comma_until(p, b')', Exposing::ExposingEnd, |p| {
                    chomp_and_check_indent(
                        p,
                        Exposing::ExposingSpace,
                        Exposing::ExposingIndentValue,
                    )?;
                    exposed(p)?;
                    chomp_and_check_indent(p, Exposing::ExposingSpace, Exposing::ExposingIndentEnd)
                })
            },
        ],
    )
}

fn exposed(p: &mut P<'_>) -> Result<(), Exposing> {
    one_of(
        p,
        Exposing::ExposingValue,
        &mut [
            &mut |p| lower(p, Exposing::ExposingValue).map(drop),
            &mut |p| {
                word1(p, b'(', Exposing::ExposingValue)?;
                operator(
                    p,
                    Exposing::ExposingOperator,
                    Exposing::ExposingOperatorReserved,
                )?;
                word1(p, b')', Exposing::ExposingOperatorRightParen)
            },
            &mut |p| {
                upper(p, Exposing::ExposingValue)?;
                chomp_and_check_indent(p, Exposing::ExposingSpace, Exposing::ExposingIndentEnd)?;
                privacy(p)
            },
        ],
    )
}

fn privacy(p: &mut P<'_>) -> Result<(), Exposing> {
    one_of_with_fallback(
        p,
        &mut [&mut |p| {
            word1(p, b'(', Exposing::ExposingTypePrivacy)?;
            chomp_and_check_indent(p, Exposing::ExposingSpace, Exposing::ExposingTypePrivacy)?;
            word2(p, b'.', b'.', Exposing::ExposingTypePrivacy)?;
            chomp_and_check_indent(p, Exposing::ExposingSpace, Exposing::ExposingTypePrivacy)?;
            word1(p, b')', Exposing::ExposingTypePrivacy)
        }],
        (),
    )
}
