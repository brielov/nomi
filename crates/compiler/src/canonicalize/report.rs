//! The reports of naming errors (`Reporting/Error/Canonicalize.hs`),
//! as each release words and orders them. A report's ranges are trimmed of the
//! trivia nomi's nodes keep in front by the caller's `trim`, which knows
//! the tree.

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::{self, Source};
use nomi_base::diagnostic::doc::{
    self, Color, Doc, comma_sep, fill_sep, link, pair, reflow, reflow_link, simple_note, snippet,
    stack, text, vcat,
};
use nomi_base::{Compat, TextRange, suggest};

use super::error::{
    ArityContext, DuplicatePatternContext, Error, InvalidPayload, NameKind, PortProblem,
    PossibleNames,
};
use crate::canonical::Home;
use crate::hir::{self, TypeId};

/// What a report needs around its error.
pub struct Context<'a> {
    pub source: &'a Source<'a>,
    pub module: &'a hir::Module,
    pub compat: Compat,
    pub trim: &'a dyn Fn(TextRange) -> TextRange,
}

impl std::fmt::Debug for Context<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("compat", &self.compat)
            .finish_non_exhaustive()
    }
}

/// `toReport`.
pub fn to_problem(error: &Error, cx: &Context<'_>) -> Problem {
    let (title, range, message) = report(error, cx);
    let range = (cx.trim)(range);
    let region = cx.source.region(range, cx.compat);
    Problem::new(title, region, code::expand(&message, cx.source, cx.compat))
}

/// A snippet of `range`, trimmed, between `before` and `after`.
fn snip(
    cx: &Context<'_>,
    range: TextRange,
    highlight: Option<TextRange>,
    before: Doc,
    after: Doc,
) -> Doc {
    snippet(before, (cx.trim)(range), highlight.map(cx.trim), after)
}

fn yellow(s: impl Into<String>) -> Doc {
    text(s).color(Color::DullYellow)
}

fn words(s: &str) -> Vec<Doc> {
    s.split(' ').filter(|w| !w.is_empty()).map(text).collect()
}

/// `Suggest.sort` of `candidates` against `target`, the first `n`.
fn nearest(target: &str, mut candidates: Vec<String>, n: usize) -> Vec<String> {
    let codes = |s: &str| s.chars().map(u32::from).collect::<Vec<u32>>();
    suggest::sort(&codes(target), &mut candidates, |c| codes(c));
    candidates.truncate(n);
    candidates
}

/// `names` in `compat`'s order, as the maps upstream lists them from
/// keep them, which decides how ties among suggestions break.
fn in_order(compat: Compat, names: &[String]) -> Vec<String> {
    super::sorted(compat, names.iter())
        .into_iter()
        .cloned()
        .collect()
}

/// Upstream's order of modules: by package, author then project, then by
/// name; each name as `compat` orders names.
fn sorted_homes(homes: &[Home], compat: Compat) -> Vec<Home> {
    let order = super::name_order(compat);
    let mut homes = homes.to_vec();
    homes.sort_by(|a, b| {
        let split = |h: &Home| {
            let (author, project) = h.package.split_once('/').unwrap_or((&h.package, ""));
            (author.to_owned(), project.to_owned())
        };
        let ((a_author, a_project), (b_author, b_project)) = (split(a), split(b));
        order(&a_author, &b_author)
            .then_with(|| order(&a_project, &b_project))
            .then_with(|| order(&a.module, &b.module))
    });
    homes
}

fn report(error: &Error, cx: &Context<'_>) -> (&'static str, TextRange, Doc) {
    match error {
        Error::AnnotationTooShort {
            range,
            name,
            index,
            leftover,
        } => {
            let message = snip(
                cx,
                *range,
                None,
                reflow(&format!(
                    "The type annotation for `{name}` says it can accept {}, but the definition says it has {}:",
                    doc::args(*index),
                    doc::args(index + leftover)
                )),
                reflow(&format!(
                    "Is the type annotation missing something? Should some argument{} be deleted? Maybe some parentheses are missing?",
                    if *leftover == 1 { "" } else { "s" }
                )),
            );
            ("BAD TYPE ANNOTATION", *range, message)
        }
        Error::Ambiguous {
            kind,
            range,
            prefix,
            name,
            homes,
        } => {
            let thing = match kind {
                NameKind::Type => "type",
                NameKind::Variant => "variant",
                NameKind::Operator => "operator",
                NameKind::Value | NameKind::Pattern => "variable",
            };
            (
                "AMBIGUOUS NAME",
                *range,
                ambiguous(cx, *range, prefix.as_deref(), name, homes, thing),
            )
        }
        Error::AmbiguousBinop { range, op, homes } => (
            "AMBIGUOUS NAME",
            *range,
            ambiguous(cx, *range, None, op, homes, "operator"),
        ),
        Error::BadArity {
            range,
            context,
            name,
            expected,
            actual,
        } => {
            let thing = match context {
                ArityContext::Type => "type",
                ArityContext::Pattern => "variant",
            };
            let before = reflow(&format!(
                "The `{name}` {thing} needs {}, but I see {actual} instead:",
                doc::args(*expected)
            ));
            if actual < expected {
                let after = reflow("What is missing? Are some parentheses misplaced?");
                (
                    "TOO FEW ARGS",
                    *range,
                    snip(cx, *range, None, before, after),
                )
            } else {
                let after = text(if actual - expected == 1 {
                    "Which is the extra one? Maybe some parentheses are missing?"
                } else {
                    "Which are the extra ones? Maybe some parentheses are missing?"
                });
                (
                    "TOO MANY ARGS",
                    *range,
                    snip(cx, *range, None, before, after),
                )
            }
        }
        Error::Binop { range, left, right } => {
            let message = snip(
                cx,
                *range,
                None,
                reflow(&format!(
                    "You cannot mix ({left}) and ({right}) without parentheses."
                )),
                reflow("I do not know how to group these expressions. Add parentheses for me!"),
            );
            ("INFIX PROBLEM", *range, message)
        }
        Error::DuplicateDecl {
            name,
            first,
            second,
        } => name_clash(
            cx,
            *first,
            *second,
            &format!("This file has multiple `{name}` declarations."),
        ),
        Error::DuplicateType {
            name,
            first,
            second,
        } => name_clash(
            cx,
            *first,
            *second,
            &format!("This file defines multiple `{name}` types."),
        ),
        Error::DuplicateCtor {
            name,
            first,
            second,
        } => name_clash(
            cx,
            *first,
            *second,
            &format!("This file defines multiple `{name}` type constructors."),
        ),
        Error::DuplicateBinop {
            name,
            first,
            second,
        } => name_clash(
            cx,
            *first,
            *second,
            &format!("This file defines multiple ({name}) operators."),
        ),
        Error::DuplicateField {
            name,
            first,
            second,
        } => name_clash(
            cx,
            *first,
            *second,
            &format!("This record has multiple `{name}` fields."),
        ),
        Error::DuplicateAliasArg {
            alias,
            name,
            first,
            second,
        } => name_clash(
            cx,
            *first,
            *second,
            &format!("The `{alias}` type alias has multiple `{name}` type variables."),
        ),
        Error::DuplicateUnionArg {
            union,
            name,
            first,
            second,
        } => name_clash(
            cx,
            *first,
            *second,
            &format!("The `{union}` type has multiple `{name}` type variables."),
        ),
        Error::DuplicatePattern {
            context,
            name,
            first,
            second,
        } => {
            let message = match context {
                DuplicatePatternContext::LambdaArgs => {
                    format!("This anonymous function has multiple `{name}` arguments.")
                }
                DuplicatePatternContext::FuncArgs(function) => {
                    format!("The `{function}` function has multiple `{name}` arguments.")
                }
                DuplicatePatternContext::CaseBranch => {
                    format!("This `case` pattern has multiple `{name}` variables.")
                }
                DuplicatePatternContext::LetBinding => {
                    format!("This `let` expression defines `{name}` more than once!")
                }
                DuplicatePatternContext::Destruct => {
                    format!("This pattern contains multiple `{name}` variables.")
                }
            };
            name_clash(cx, *first, *second, &message)
        }
        Error::EffectNotFound { range, name } => {
            let message = snip(
                cx,
                *range,
                None,
                reflow(&format!(
                    "You have declared that `{name}` is an effect type:"
                )),
                reflow(&format!(
                    "But I cannot find a custom type named `{name}` in this file!"
                )),
            );
            ("EFFECT PROBLEM", *range, message)
        }
        Error::EffectFunctionNotFound { range, name } => {
            let message = snip(
                cx,
                *range,
                None,
                reflow(&format!(
                    "This kind of effect module must define a `{name}` function."
                )),
                reflow(&format!("But I cannot find `{name}` in this file!")),
            );
            ("EFFECT PROBLEM", *range, message)
        }
        Error::ExportDuplicate {
            name,
            first,
            second,
            ..
        } => {
            let message = format!("You are trying to expose `{name}` multiple times!");
            let fix = "Remove one of them and you should be all set!";
            let doc = pair(
                (cx.trim)(*first),
                (cx.trim)(*second),
                (reflow(&message), text(fix)),
                (
                    reflow(&format!("{message} Once here:")),
                    text("And again right here:"),
                    text(fix),
                ),
            );
            ("REDUNDANT EXPORT", *second, doc)
        }
        Error::ExportNotFound {
            range,
            kind,
            name,
            possible,
        } => {
            let suggestions = nearest(name, possible.clone(), 4);
            let (a, thing, shown) = kind_info(*kind, name);
            let found = match &suggestions[..] {
                [] => reflow(
                    "I do not see any super similar names in this file. Is the definition missing?",
                ),
                [alt] => fill_sep([
                    text("Maybe"),
                    text("you"),
                    text("want"),
                    yellow(alt.clone()),
                    text("instead?"),
                ]),
                alts => stack([
                    text("These names seem close though:"),
                    vcat(alts.iter().map(|a| yellow(a.clone()))).indent(4),
                ]),
            };
            let mut overview: Vec<Doc> = words("You are trying to expose");
            overview.extend([text(a), text(thing), text("named"), shown]);
            overview.extend(words("but I cannot find its definition."));
            // 0.19.3 shows where.
            let doc = match cx.compat {
                Compat::V0_19_1 | Compat::V0_19_2 => stack([fill_sep(overview), found]),
                Compat::V0_19_3 => snip(cx, *range, None, fill_sep(overview), found),
            };
            ("UNKNOWN EXPORT", *range, doc)
        }
        Error::ExportOpenAlias { range, name } => {
            let message = snip(
                cx,
                *range,
                None,
                reflow(&format!(
                    "The (..) syntax is for exposing variants of a custom type. It cannot be used with a type alias like `{name}` though."
                )),
                reflow("Remove the (..) and you should be fine!"),
            );
            ("BAD EXPORT", *range, message)
        }
        Error::ImportCtorByName { range, ctor, union } => {
            let mut fix: Vec<Doc> = words("Try importing");
            fix.push(text(format!("{union}(..)")).color(Color::Green));
            fix.extend(words("instead. The dots mean “expose the"));
            fix.push(text(union.clone()));
            fix.extend(words("type and all its variants so it gives you access to"));
            fix.push(text(format!("{ctor}.")));
            let message = snip(
                cx,
                *range,
                None,
                reflow(&format!(
                    "You are trying to import the `{ctor}` variant by name:"
                )),
                fill_sep(fix),
            );
            ("BAD IMPORT", *range, message)
        }
        Error::ImportOpenAlias { range, name } => {
            let message = snip(
                cx,
                *range,
                None,
                reflow(&format!(
                    "The `{name}` type alias cannot be followed by (..) like this:"
                )),
                reflow("Remove the (..) and it should work."),
            );
            ("BAD IMPORT", *range, message)
        }
        Error::ImportExposingNotFound {
            range,
            home,
            name,
            possible,
            ..
        } => {
            let suggestions = nearest(&home.module, in_order(cx.compat, possible), 4);
            let after = match &suggestions[..] {
                [] => text("I cannot find any super similar exposed names. Maybe it is private?"),
                [alt] => fill_sep([
                    text("Maybe"),
                    text("you"),
                    text("want"),
                    yellow(alt.clone()),
                    text("instead?"),
                ]),
                alts => stack([
                    text("These names seem close though:"),
                    vcat(alts.iter().map(|a| yellow(a.clone()))).indent(4),
                ]),
            };
            let before = reflow(&format!(
                "The `{}` module does not expose `{name}`:",
                home.module
            ));
            ("BAD IMPORT", *range, snip(cx, *range, None, before, after))
        }
        Error::NotFound {
            kind,
            range,
            prefix,
            name,
            possible,
        } => {
            let thing = match kind {
                NameKind::Type => "type",
                NameKind::Variant => "variant",
                _ => "variable",
            };
            (
                "NAMING ERROR",
                *range,
                not_found(cx, *range, prefix.as_deref(), name, thing, possible),
            )
        }
        Error::NotFoundBinop {
            range,
            op,
            possible,
        } => (
            "UNKNOWN OPERATOR",
            *range,
            unknown_operator(cx, *range, op, possible),
        ),
        Error::PatternHasRecordCtor { range, name } => {
            let message = snip(
                cx,
                *range,
                None,
                reflow(&format!(
                    "You can construct records by using `{name}` as a function, but it is not available in pattern matching like this:"
                )),
                reflow("I recommend matching the record as a variable and unpacking it later."),
            );
            ("BAD PATTERN", *range, message)
        }
        Error::PortPayloadInvalid {
            range,
            name,
            problem,
            ..
        } => {
            let (thing, elaboration) = payload_details(problem);
            let message = snip(
                cx,
                *range,
                None,
                reflow(&format!("The `{name}` port is trying to transmit {thing}:")),
                stack([
                    elaboration,
                    link(
                        "Hint",
                        "Ports are not a traditional FFI, so if you have tons of annoying ports, definitely read",
                        "ports",
                        "to learn how they are meant to work. They require a different mindset!",
                    ),
                ]),
            );
            ("PORT ERROR", *range, message)
        }
        Error::PortTypeInvalid {
            range,
            name,
            problem,
        } => {
            let (before, after) = port_details(name, problem);
            let message = snip(
                cx,
                *range,
                None,
                reflow(&before),
                stack([
                    after,
                    link(
                        "Hint",
                        "Read",
                        "ports",
                        "for more advice. For example, do not end up with one port per JS function!",
                    ),
                ]),
            );
            ("BAD PORT", *range, message)
        }
        Error::RecursiveAlias {
            range,
            name,
            args,
            body,
            others,
        } => (
            "ALIAS PROBLEM",
            *range,
            alias_recursion(cx, *range, name, args, *body, others),
        ),
        Error::RecursiveDecl {
            range,
            name,
            others,
        } => {
            let message = if others.is_empty() {
                snip(
                    cx,
                    *range,
                    None,
                    reflow(&format!(
                        "The `{name}` value is defined directly in terms of itself, causing an infinite loop."
                    )),
                    self_recursion(cx.compat, name),
                )
            } else {
                snip(
                    cx,
                    *range,
                    None,
                    reflow(&format!(
                        "The `{name}` definition is causing a very tricky infinite loop."
                    )),
                    chain(
                        name,
                        others,
                        "for more detailed advice, especially if you actually do want mutually recursive values.",
                    ),
                )
            };
            ("CYCLIC DEFINITION", *range, message)
        }
        Error::RecursiveLet { name, others } => {
            let range = name.range;
            let name = &name.name;
            let message = if others.is_empty() {
                snip(
                    cx,
                    range,
                    None,
                    reflow(&format!(
                        "The `{name}` value is defined directly in terms of itself, causing an infinite loop."
                    )),
                    self_recursion(cx.compat, name),
                )
            } else {
                snip(
                    cx,
                    range,
                    None,
                    reflow("I do not allow cyclic values in `let` expressions."),
                    chain(
                        name,
                        others,
                        "for more detailed advice, especially if you actually do want mutually recursive values.",
                    ),
                )
            };
            ("CYCLIC VALUE", range, message)
        }
        Error::Shadowing {
            name,
            first,
            second,
        } => {
            let advice = || {
                stack([
                    reflow(
                        "Think of a more helpful name for one of them and you should be all set!",
                    ),
                    link(
                        "Note",
                        "Linters advise against shadowing, so Elm makes “best practices” the default. Read",
                        "shadowing",
                        "for more details on this choice.",
                    ),
                ])
            };
            let doc = pair(
                (cx.trim)(*first),
                (cx.trim)(*second),
                (text("These variables cannot have the same name:"), advice()),
                (
                    reflow(&format!("The name `{name}` is first defined here:")),
                    text("But then it is defined AGAIN over here:"),
                    advice(),
                ),
            );
            ("SHADOWING", *second, doc)
        }
        Error::TupleLargerThanThree { range } => {
            let message = snip(
                cx,
                *range,
                None,
                text("I only accept tuples with two or three items. This has too many:"),
                stack([
                    reflow(
                        "I recommend switching to records. Each item will be named, and you can use the `point.x` syntax to access them.",
                    ),
                    link(
                        "Note",
                        "Read",
                        "tuples",
                        "for more comprehensive advice on working with large chunks of data in Elm.",
                    ),
                ]),
            );
            ("BAD TUPLE", *range, message)
        }
        Error::TypeVarsUnboundInUnion {
            range,
            name,
            vars,
            first,
            rest,
        } => {
            let rest: Vec<(String, TextRange)> =
                rest.iter().map(|a| (a.name.clone(), a.range)).collect();
            let (title, doc) = unbound_type_vars(
                cx,
                *range,
                &["type"],
                name,
                vars,
                (&first.name, first.range),
                &rest,
            );
            (title, *range, doc)
        }
        Error::TypeVarsMessedUpInAlias {
            range,
            name,
            vars,
            unused,
            unbound,
        } => messed_up_alias(cx, *range, name, vars, unused, unbound),
    }
}

/// `toKindInfo`: an article, the thing, and its name as shown.
fn kind_info(kind: NameKind, name: &str) -> (&'static str, &'static str, Doc) {
    match kind {
        NameKind::Operator => ("an", "operator", text(format!("({name})"))),
        NameKind::Value => ("a", "value", text(format!("`{name}`"))),
        NameKind::Pattern => ("a", "pattern", text(format!("`{name}`"))),
        NameKind::Type | NameKind::Variant => ("a", "type", text(format!("`{name}`"))),
    }
}

/// `nameClash`.
fn name_clash(
    cx: &Context<'_>,
    first: TextRange,
    second: TextRange,
    message: &str,
) -> (&'static str, TextRange, Doc) {
    let fix = "How can I know which one you want? Rename one of them!";
    let doc = pair(
        (cx.trim)(first),
        (cx.trim)(second),
        (reflow(message), text(fix)),
        (
            reflow(&format!("{message} One here:")),
            text("And another one here:"),
            text(fix),
        ),
    );
    ("NAME CLASH", second, doc)
}

/// `ambiguousName`.
fn ambiguous(
    cx: &Context<'_>,
    range: TextRange,
    prefix: Option<&str>,
    name: &str,
    homes: &[Home],
    thing: &str,
) -> Doc {
    let homes = match cx.compat {
        Compat::V0_19_1 => homes.to_vec(),
        Compat::V0_19_2 | Compat::V0_19_3 => sorted_homes(homes, cx.compat),
    };
    match prefix {
        None => snip(
            cx,
            range,
            None,
            reflow(&format!("This usage of `{name}` is ambiguous:")),
            stack([
                reflow(&format!(
                    "This name is exposed by {} of your imports, so I am not sure which one to use:",
                    homes.len()
                )),
                vcat(homes.iter().map(|h| yellow(format!("{}.{name}", h.module)))).indent(4),
                reflow(
                    "I recommend using qualified names for imported values. I also recommend having at most one `exposing (..)` per file to make name clashes like this less common in the long run.",
                ),
                link(
                    "Note",
                    "Check out",
                    "imports",
                    "for more info on the import syntax.",
                ),
            ]),
        ),
        Some(prefix) => {
            let import = |h: &Home| {
                let import = text("import").color(Color::Cyan);
                if h.module == prefix {
                    import.space(text(h.module.clone()))
                } else {
                    import
                        .space(text(h.module.clone()))
                        .space(text("as").color(Color::Cyan))
                        .space(text(prefix))
                }
            };
            let either = if homes.len() == 2 { "either" } else { "any" };
            snip(
                cx,
                range,
                None,
                reflow(&format!("This usage of `{prefix}.{name}` is ambiguous.")),
                stack([
                    reflow(&format!(
                        "It could refer to a {thing} from {either} of these imports:"
                    )),
                    vcat(homes.iter().map(import)).indent(4),
                    reflow_link(
                        "Read",
                        "imports",
                        "to learn how to clarify which one you want.",
                    ),
                ]),
            )
        }
    }
}

/// `notFound`.
fn not_found(
    cx: &Context<'_>,
    range: TextRange,
    prefix: Option<&str>,
    name: &str,
    thing: &str,
    possible: &PossibleNames,
) -> Doc {
    let given = match prefix {
        Some(prefix) => format!("{prefix}.{name}"),
        None => name.to_owned(),
    };
    // The qualified names, prefix by prefix, then the unqualified ones,
    // as `Map.foldrWithKey` lists them.
    let mut candidates: Vec<String> = Vec::new();
    let prefixes: Vec<String> = possible.qualified.iter().map(|(p, _)| p.clone()).collect();
    for prefix in in_order(cx.compat, &prefixes) {
        let (_, names) = possible
            .qualified
            .iter()
            .find(|(p, _)| *p == prefix)
            .expect("a prefix");
        candidates.extend(
            in_order(cx.compat, names)
                .iter()
                .map(|n| format!("{prefix}.{n}")),
        );
    }
    candidates.extend(in_order(cx.compat, &possible.locals));
    let nearby = nearest(&given, candidates, 4);
    let details = |none: &str, some: &str| {
        let hint = link(
            "Hint",
            "Read",
            "imports",
            "to see how `import` declarations work in Elm.",
        );
        if nearby.is_empty() {
            stack([reflow(none), hint])
        } else {
            stack([
                reflow(some),
                vcat(nearby.iter().map(|n| yellow(n.clone()))).indent(4),
                hint,
            ])
        }
    };
    let after = match prefix {
        None => details(
            "Is there an `import` or `exposing` missing up top?",
            "These names seem close though:",
        ),
        Some(prefix) if !possible.qualified.iter().any(|(p, _)| p == prefix) => details(
            &format!("I cannot find a `{prefix}` module. Is there an `import` for it?"),
            &format!("I cannot find a `{prefix}` import. These names seem close though:"),
        ),
        Some(prefix) => details(
            &format!("The `{prefix}` module does not expose a `{name}` {thing}."),
            &format!(
                "The `{prefix}` module does not expose a `{name}` {thing}. These names seem close though:"
            ),
        ),
    };
    snip(
        cx,
        range,
        None,
        reflow(&format!("I cannot find a `{given}` {thing}:")),
        after,
    )
}

fn unknown_operator(cx: &Context<'_>, range: TextRange, op: &str, possible: &[String]) -> Doc {
    match op {
        "===" => snip(
            cx,
            range,
            None,
            text("Elm does not have a (===) operator like JavaScript."),
            text("Switch to (==) instead."),
        ),
        "!=" | "!==" => snip(
            cx,
            range,
            None,
            reflow("Elm uses a different name for the “not equal” operator:"),
            stack([
                reflow("Switch to (/=) instead."),
                simple_note(&format!(
                    "Our (/=) operator is supposed to look like a real “not equal” sign (≠). I hope that history will remember ({op}) as a weird and temporary choice."
                )),
            ]),
        ),
        "**" => snip(
            cx,
            range,
            None,
            reflow("I do not recognize the (**) operator:"),
            reflow("Switch to (^) for exponentiation. Or switch to (*) for multiplication."),
        ),
        "%" => snip(
            cx,
            range,
            None,
            reflow("Elm does not use (%) as the remainder operator:"),
            stack([
                reflow(
                    "If you want the behavior of (%) like in JavaScript, switch to: <https://package.elm-lang.org/packages/elm/core/latest/Basics#remainderBy>",
                ),
                reflow(
                    "If you want modular arithmetic like in math, switch to: <https://package.elm-lang.org/packages/elm/core/latest/Basics#modBy>",
                ),
                reflow("The difference is how things work when negative numbers are involved."),
            ]),
        ),
        _ => {
            let suggestions = nearest(op, in_order(cx.compat, possible), 2);
            let mut after = words("Is there an `import` and `exposing` entry for it?");
            if !suggestions.is_empty() {
                after.extend(words("Maybe you want"));
                let alts = suggestions
                    .iter()
                    .map(|s| text(format!("({s})")).color(Color::Green))
                    .collect();
                after.extend(comma_sep("or", alts));
                after.push(text("instead?"));
            }
            snip(
                cx,
                range,
                None,
                reflow(&format!("I do not recognize the ({op}) operator.")),
                fill_sep(after),
            )
        }
    }
}

fn payload_details(problem: &InvalidPayload) -> (String, Doc) {
    match problem {
        InvalidPayload::ExtendedRecord => (
            "an extended record".to_owned(),
            reflow("But the exact shape of the record must be known at compile time. No type variables!"),
        ),
        InvalidPayload::Function => (
            "a function".to_owned(),
            reflow(
                "But functions cannot be sent in and out ports. If we allowed functions in from JS they may perform some side-effects. If we let functions out, they could produce incorrect results because Elm optimizations assume there are no side-effects.",
            ),
        ),
        InvalidPayload::TypeVariable(name) => (
            "an unspecified type".to_owned(),
            reflow(&format!(
                "But type variables like `{name}` cannot flow through ports. I need to know exactly what type of data I am getting, so I can guarantee that unexpected data cannot sneak in and crash the Elm program."
            )),
        ),
        InvalidPayload::UnsupportedType(name) => (
            format!("a `{name}` value"),
            stack([
                reflow("I cannot handle that. The types that CAN flow in and out of Elm include:"),
                reflow("Ints, Floats, Bools, Strings, Maybes, Lists, Arrays, tuples, records, and JSON values.").indent(4),
                reflow(
                    "Since JSON values can flow through, you can use JSON encoders and decoders to allow other types through as well. More advanced users often just do everything with encoders and decoders for more control and better errors.",
                ),
            ]),
        ),
    }
}

fn port_details(name: &str, problem: &PortProblem) -> (String, Doc) {
    match problem {
        PortProblem::CmdNoArg => (
            format!("The `{name}` port cannot be just a command."),
            reflow(
                "It can be (() -> Cmd msg) if you just need to trigger a JavaScript function, but there is often a better way to set things up.",
            ),
        ),
        PortProblem::CmdExtraArgs(n) => {
            let items = match n {
                2 => "both of these items into a tuple or record".to_owned(),
                3 => format!("these {n} items into a tuple or record"),
                n => format!("these {n} items into a record"),
            };
            (
                format!("The `{name}` port can only send ONE value out to JavaScript."),
                reflow(&format!("You can put {items} to send them out though.")),
            )
        }
        PortProblem::CmdBadMsg => (
            format!("The `{name}` port cannot send any messages to the `update` function."),
            reflow(
                "It must produce a (Cmd msg) type. Notice the lower case `msg` type variable. The command will trigger some JS code, but it will not send anything particular back to Elm.",
            ),
        ),
        PortProblem::SubBad => (
            format!("There is something off about this `{name}` port declaration."),
            stack([
                reflow("To receive messages from JavaScript, you need to define a port like this:"),
                yellow(format!("port {name} : (Int -> msg) -> Sub msg")).indent(4),
                reflow(
                    "Now every time JS sends an `Int` to this port, it is converted to a `msg`. And if you subscribe, those `msg` values will be piped into your `update` function. The only thing you can customize here is the `Int` type.",
                ),
            ]),
        ),
        PortProblem::NotCmdOrSub => (
            format!("I am confused about the `{name}` port declaration."),
            reflow(
                "Ports need to produce a command (Cmd) or a subscription (Sub) but this is neither. I do not know how to handle this.",
            ),
        ),
    }
}

/// The advice for a value defined as itself; 0.19.2 fixed the typo
/// "Are you are trying".
fn self_recursion(compat: Compat, name: &str) -> Doc {
    let theory = |question: &str, details: &str| {
        let mut docs: Vec<Doc> = question
            .split(' ')
            .filter(|w| !w.is_empty())
            .map(yellow)
            .collect();
        docs.extend(words(details));
        fill_sep(docs)
    };
    let question = match compat {
        Compat::V0_19_1 => "Are you are trying to mutate a variable?",
        Compat::V0_19_2 | Compat::V0_19_3 => "Are you trying to mutate a variable?",
    };
    stack([
        theory(
            question,
            &format!(
                "Elm does not have mutation, so when I see {name} defined in terms of {name}, I treat it as a recursive definition. Try giving the new value a new name!"
            ),
        ),
        theory(
            "Maybe you DO want a recursive value?",
            &format!(
                "To define {name} we need to know what {name} is, so let’s expand it. Wait, but now we need to know what {name} is, so let’s expand it... This will keep going infinitely!"
            ),
        ),
        link(
            "Hint",
            "The root problem is often a typo in some variable name, but I recommend reading",
            "bad-recursion",
            "for more detailed advice, especially if you actually do need a recursive value.",
        ),
    ])
}

/// A value that depends on itself through others.
fn chain(name: &str, others: &[String], advice: &str) -> Doc {
    stack([
        reflow(&format!(
            "The `{name}` value depends on itself through the following chain of definitions:"
        )),
        doc::cycle(std::iter::once(name.to_owned()).chain(others.iter().cloned())),
        link(
            "Hint",
            "The root problem is often a typo in some variable name, but I recommend reading",
            "bad-recursion",
            advice,
        ),
    ])
}

/// `unboundTypeVars`.
fn unbound_type_vars(
    cx: &Context<'_>,
    range: TextRange,
    tipe: &[&str],
    name: &str,
    vars: &[String],
    (unbound, var_range): (&str, TextRange),
    others: &[(String, TextRange)],
) -> (&'static str, Doc) {
    let quoted = |n: &str| text(format!("`{n}`"));
    let tipe_docs = || tipe.iter().map(|t| text(*t));
    let (title, highlight, overview) = if others.is_empty() {
        let mut overview = vec![text("The"), quoted(name)];
        overview.extend(tipe_docs());
        overview.extend(words("uses an unbound type variable"));
        overview.push(quoted(unbound).color(Color::DullYellow));
        overview.extend(words("in its definition:"));
        ("UNBOUND TYPE VARIABLE", Some(var_range), overview)
    } else {
        let mut overview = words("Type variables");
        let names = std::iter::once(unbound).chain(others.iter().map(|(n, _)| n.as_str()));
        overview.extend(comma_sep("and", names.map(yellow).collect()));
        overview.extend(words("are unbound in the"));
        overview.push(quoted(name));
        overview.extend(tipe_docs());
        overview.push(text("definition:"));
        ("UNBOUND TYPE VARIABLES", None, overview)
    };
    let mut declaration: Vec<Doc> = tipe_docs().collect();
    declaration.push(text(name));
    declaration.extend(vars.iter().map(|v| text(v.clone())));
    declaration.push(text(unbound).color(Color::Green));
    declaration.extend(
        others
            .iter()
            .map(|(n, _)| text(n.clone()).color(Color::Green)),
    );
    declaration.extend([text("="), text("...")]);
    let doc = snip(
        cx,
        range,
        highlight,
        fill_sep(overview),
        stack([
            reflow("You probably need to change the declaration to something like this:"),
            doc::hsep(declaration).indent(4),
            reflow(&format!(
                "Why? Well, imagine one `{name}` where `{unbound}` is an Int and another where it is a Bool. When we explicitly list the type variables, the type checker can see that they are actually different types."
            )),
        ]),
    );
    (title, doc)
}

/// `TypeVarsMessedUpInAlias`.
fn messed_up_alias(
    cx: &Context<'_>,
    range: TextRange,
    name: &str,
    vars: &[String],
    unused: &[super::error::At],
    unbound: &[super::error::At],
) -> (&'static str, TextRange, Doc) {
    let quoted = |n: &str| text(format!("`{n}`"));
    match (unused, unbound) {
        ([first, rest @ ..], []) => {
            let names: Vec<&str> = unused.iter().map(|a| a.name.as_str()).collect();
            let (title, highlight, overview, stuff) = if rest.is_empty() {
                let mut overview = words("Type alias");
                overview.push(quoted(name));
                overview.extend(words("does not use the"));
                overview.push(quoted(&first.name));
                overview.extend(words("type variable."));
                (
                    "UNUSED TYPE VARIABLE",
                    Some(first.range),
                    overview,
                    vec![quoted(&first.name).color(Color::DullYellow)],
                )
            } else {
                let mut overview = words("Type variables");
                overview.extend(comma_sep("and", names.iter().map(|n| text(*n)).collect()));
                overview.extend(words("are unused in the"));
                overview.push(quoted(name));
                overview.push(text("definition."));
                (
                    "UNUSED TYPE VARIABLES",
                    None,
                    overview,
                    comma_sep("and", names.iter().map(|n| yellow(*n)).collect()),
                )
            };
            let mut recommend = words("I recommend removing");
            recommend.extend(stuff);
            recommend.extend(words("from the declaration, like this:"));
            let mut declaration = vec![text("type"), text("alias"), text(name).color(Color::Green)];
            declaration.extend(
                vars.iter()
                    .filter(|v| !names.contains(&v.as_str()))
                    .map(|v| text(v.clone())),
            );
            declaration.extend([text("="), text("...")]);
            let doc = snip(
                cx,
                range,
                highlight,
                fill_sep(overview),
                stack([
                    fill_sep(recommend),
                    doc::hsep(declaration).indent(4),
                    reflow(
                        "Why? Well, if I allowed `type alias Height a = Float` I would need to answer some weird questions. Is `Height Bool` the same as `Float`? Is `Height Bool` the same as `Height Int`? My solution is to not need to ask them!",
                    ),
                ]),
            );
            (title, range, doc)
        }
        ([], [first, rest @ ..]) => {
            let rest: Vec<(String, TextRange)> =
                rest.iter().map(|a| (a.name.clone(), a.range)).collect();
            let (title, doc) = unbound_type_vars(
                cx,
                range,
                &["type", "alias"],
                name,
                vars,
                (&first.name, first.range),
                &rest,
            );
            (title, range, doc)
        }
        _ => {
            let unused: Vec<&str> = unused.iter().map(|a| a.name.as_str()).collect();
            let unbound: Vec<&str> = unbound.iter().map(|a| a.name.as_str()).collect();
            let mut used = match &unbound[..] {
                [x] => {
                    let mut docs = words("Type variable");
                    docs.push(yellow(format!("`{x}`")));
                    docs.extend(words(
                        "appears in the definition, but I do not see it declared.",
                    ));
                    docs
                }
                _ => {
                    let mut docs = words("Type variables");
                    docs.extend(comma_sep(
                        "and",
                        unbound.iter().map(|n| yellow(*n)).collect(),
                    ));
                    docs.extend(words(
                        "are used in the definition, but I do not see them declared.",
                    ));
                    docs
                }
            };
            match &unused[..] {
                [x] => {
                    used.extend(words("Likewise, type variable"));
                    used.push(yellow(format!("`{x}`")));
                    used.extend(words("is delared, but not used."));
                }
                _ => {
                    used.extend(words("Likewise, type variables"));
                    used.extend(comma_sep(
                        "and",
                        unused.iter().map(|n| yellow(*n)).collect(),
                    ));
                    used.extend(words("are delared, but not used."));
                }
            }
            let mut declaration = vec![text("type"), text("alias"), text(name)];
            declaration.extend(
                vars.iter()
                    .filter(|v| !unused.contains(&v.as_str()))
                    .map(|v| text(v.clone())),
            );
            declaration.extend(unbound.iter().map(|n| text(*n).color(Color::Green)));
            declaration.extend([text("="), text("...")]);
            let doc = snip(
                cx,
                range,
                None,
                reflow(&format!(
                    "Type alias `{name}` has some type variable problems."
                )),
                stack([
                    fill_sep(used),
                    reflow("My guess is that a definition like this will work better:"),
                    doc::hsep(declaration).indent(4),
                ]),
            );
            ("TYPE VARIABLE PROBLEMS", range, doc)
        }
    }
}

/// `aliasRecursionReport`.
fn alias_recursion(
    cx: &Context<'_>,
    range: TextRange,
    name: &str,
    args: &[String],
    body: TypeId,
    others: &[String],
) -> Doc {
    if others.is_empty() {
        let header = args.iter().fold(text("type").space(text(name)), |d, a| {
            d.space(text(a.clone()))
        });
        let union = vcat([
            header.space(text("=")).color(Color::DullYellow),
            text(name).indent(4).color(Color::Green),
            source_type(cx.module, body, Parens::App)
                .indent(8)
                .color(Color::DullYellow),
        ]);
        snip(
            cx,
            range,
            None,
            text("This type alias is recursive, forming an infinite type!"),
            stack([
                reflow(
                    "When I expand a recursive type alias, it just keeps getting bigger and bigger. So dealiasing results in an infinitely large type! Try this instead:",
                ),
                union.indent(4),
                link(
                    "Hint",
                    "This is kind of a subtle distinction. I suggested the naive fix, but I recommend reading",
                    "recursive-alias",
                    "for ideas on how to do better.",
                ),
            ]),
        )
    } else {
        snip(
            cx,
            range,
            None,
            text("This type alias is part of a mutually recursive set of type aliases."),
            stack([
                text("It is part of this cycle of type aliases:"),
                doc::cycle(std::iter::once(name.to_owned()).chain(others.iter().cloned())),
                reflow("You need to convert at least one of these type aliases into a `type`."),
                link(
                    "Note",
                    "Read",
                    "recursive-alias",
                    "to learn why this `type` vs `type alias` distinction matters. It is subtle but important!",
                ),
            ]),
        )
    }
}

/// Where a type sits, which decides its parentheses (`RT.Context`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Parens {
    None,
    Func,
    App,
}

/// `RT.srcToDoc`: a type as the source wrote it.
fn source_type(module: &hir::Module, id: TypeId, context: Parens) -> Doc {
    match &module.types[id] {
        hir::Type::Lambda(arg, result) => {
            let mut parts = vec![source_type(module, *arg, Parens::Func)];
            let mut result = *result;
            while let hir::Type::Lambda(a, r) = &module.types[result] {
                parts.push(source_type(module, *a, Parens::Func));
                result = *r;
            }
            parts.push(source_type(module, result, Parens::Func));
            let mut parts = parts.into_iter();
            let first = parts.next().expect("an argument");
            let lambda =
                doc::sep(std::iter::once(first).chain(parts.map(|p| text("->").space(p)))).align();
            match context {
                Parens::None => lambda,
                Parens::Func | Parens::App => doc::cat([text("("), lambda, text(")")]),
            }
        }
        hir::Type::Var(name) => text(name.clone()),
        hir::Type::Named {
            qualifier,
            name,
            args,
        } => {
            let name = match qualifier {
                Some(q) => text(format!("{q}.{}", name.text)),
                None => text(name.text.clone()),
            };
            if args.is_empty() {
                return name;
            }
            let args = args.iter().map(|&a| source_type(module, a, Parens::App));
            let applied = doc::sep(std::iter::once(name).chain(args)).hang(4);
            match context {
                Parens::App => doc::cat([text("("), applied, text(")")]),
                Parens::Func | Parens::None => applied,
            }
        }
        hir::Type::Record { fields, extends } => {
            let entries: Vec<Doc> = (fields.iter())
                .map(|(n, t)| {
                    doc::sep([
                        text(n.text.to_string()).space(text(":")),
                        source_type(module, *t, Parens::None),
                    ])
                    .hang(4)
                })
                .collect();
            match (entries.is_empty(), extends) {
                (true, None) => text("{}"),
                (false, None) => {
                    let opened = entries
                        .into_iter()
                        .enumerate()
                        .map(|(i, e)| text(if i == 0 { "{" } else { "," }).space(e));
                    doc::sep([doc::cat(opened), text("}")]).align()
                }
                (_, Some(ext)) => {
                    let fields = entries
                        .into_iter()
                        .enumerate()
                        .map(|(i, e)| text(if i == 0 { "|" } else { "," }).space(e));
                    doc::sep([
                        doc::sep([text("{").space(text(ext.text.clone())), doc::cat(fields)])
                            .hang(4),
                        text("}"),
                    ])
                    .align()
                }
            }
        }
        hir::Type::Unit | hir::Type::Missing => text("()"),
        hir::Type::Tuple(items) => {
            let entries = (items.iter()).enumerate().map(|(i, &t)| {
                text(if i == 0 { "(" } else { "," }).space(source_type(module, t, Parens::None))
            });
            doc::sep([doc::cat(entries), text(")")]).align()
        }
    }
}
