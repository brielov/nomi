//! The reports of naming errors (`Reporting/Error/Canonicalize.hs`),
//! as each release words and orders them. A report's ranges are trimmed of the
//! trivia nomi's nodes keep in front by the caller's `trim`, which knows
//! the tree.

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::{self, Source};
use nomi_base::diagnostic::doc::{
    self, Conjunction, Doc, OneLine, TwoSnippets, comma_sep, cyan, dull_yellow, fill_sep, green,
    link, pair, reflow, reflow_link, simple_note, snippet, stack, text, vcat, words,
};
use nomi_base::suggest::nearest;
use nomi_base::{Compat, TextRange};

use super::error::{
    ArityContext, DuplicatePatternContext, Duplicated, Error, InvalidPayload, NameKind,
    PortProblem, PossibleNames,
};
use crate::canonical::Home;
use crate::hir::{self, Spanned, TypeId};
use crate::report::Report;
use crate::types::render::{self, Context as Parens};

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
    let report = report(error, cx);
    let region = cx.source.region((cx.trim)(report.range), cx.compat);
    let message = code::expand(&report.message, cx.source, cx.compat);
    Problem::new(report.title, region, message)
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

/// The suggestions of names close to a missing one: one as a question,
/// more as a list; `none` where there are none.
fn suggestions(found: &[String], none: Doc) -> Doc {
    match found {
        [] => none,
        [alt] => {
            fill_sep(words("Maybe you want").chain([dull_yellow(alt.clone()), text("instead?")]))
        }
        alts => stack([
            text("These names seem close though:"),
            vcat(alts.iter().map(|a| dull_yellow(a.clone()))).indent(4),
        ]),
    }
}

/// Upstream's order of modules: by package, author then project, then by
/// name; each name as `compat` orders names.
fn sorted_homes(homes: &[Home], compat: Compat) -> Vec<Home> {
    let mut homes = homes.to_vec();
    homes.sort_by(|a, b| {
        let split = |h: &Home| {
            h.package
                .as_str()
                .split_once('/')
                .unwrap_or((h.package.as_str(), ""))
        };
        let ((a_author, a_project), (b_author, b_project)) = (split(a), split(b));
        compat
            .compare_names(a_author, b_author)
            .then_with(|| compat.compare_names(a_project, b_project))
            .then_with(|| compat.compare_names(&a.module, &b.module))
    });
    homes
}

/// What a sort of name is called, as a missing or ambiguous one is
/// reported.
fn thing(kind: NameKind) -> &'static str {
    match kind {
        NameKind::Value => "variable",
        NameKind::Type => "type",
        NameKind::Variant => "variant",
        NameKind::Operator => "operator",
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "upstream's `toReport`, one message per error"
)]
fn report(error: &Error, cx: &Context<'_>) -> Report {
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
            Report::new("BAD TYPE ANNOTATION", *range, message)
        }
        Error::Ambiguous {
            kind,
            range,
            prefix,
            name,
            homes,
        } => {
            let message = ambiguous(cx, *range, prefix.as_deref(), name, homes, *kind);
            Report::new("AMBIGUOUS NAME", *range, message)
        }
        Error::AmbiguousBinop { range, op, homes } => {
            let message = ambiguous(cx, *range, None, op, homes, NameKind::Operator);
            Report::new("AMBIGUOUS NAME", *range, message)
        }
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
            let (title, after) = if actual < expected {
                let after = reflow("What is missing? Are some parentheses misplaced?");
                ("TOO FEW ARGS", after)
            } else {
                let after = text(if actual - expected == 1 {
                    "Which is the extra one? Maybe some parentheses are missing?"
                } else {
                    "Which are the extra ones? Maybe some parentheses are missing?"
                });
                ("TOO MANY ARGS", after)
            };
            Report::new(title, *range, snip(cx, *range, None, before, after))
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
            Report::new("INFIX PROBLEM", *range, message)
        }
        Error::Duplicate {
            what,
            name,
            first,
            second,
        } => {
            let message = match what {
                Duplicated::Decl => format!("This file has multiple `{name}` declarations."),
                Duplicated::Type => format!("This file defines multiple `{name}` types."),
                Duplicated::Ctor => {
                    format!("This file defines multiple `{name}` type constructors.")
                }
                Duplicated::Field => format!("This record has multiple `{name}` fields."),
            };
            name_clash(cx, *first, *second, &message)
        }
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
            Report::new("EFFECT PROBLEM", *range, message)
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
            Report::new("EFFECT PROBLEM", *range, message)
        }
        Error::ExportDuplicate {
            name,
            first,
            second,
        } => {
            let message = format!("You are trying to expose `{name}` multiple times!");
            let fix = "Remove one of them and you should be all set!";
            let doc = pair(
                (cx.trim)(*first),
                (cx.trim)(*second),
                OneLine {
                    before: reflow(&message),
                    after: text(fix),
                },
                TwoSnippets {
                    before: reflow(&format!("{message} Once here:")),
                    between: text("And again right here:"),
                    after: text(fix),
                },
            );
            Report::new("REDUNDANT EXPORT", *second, doc)
        }
        Error::ExportNotFound {
            range,
            kind,
            name,
            possible,
        } => {
            let found = suggestions(
                &nearest(name, possible.clone(), 4),
                reflow(
                    "I do not see any super similar names in this file. Is the definition missing?",
                ),
            );
            let (a, thing, shown) = match kind {
                NameKind::Operator => ("an", "operator", text(format!("({name})"))),
                NameKind::Value => ("a", "value", text(format!("`{name}`"))),
                NameKind::Type | NameKind::Variant => ("a", "type", text(format!("`{name}`"))),
            };
            let overview = (words("You are trying to expose"))
                .chain([text(a), text(thing), text("named"), shown])
                .chain(words("but I cannot find its definition."));
            // 0.19.3 shows where.
            let doc = match cx.compat {
                Compat::V0_19_1 | Compat::V0_19_2 => stack([fill_sep(overview), found]),
                Compat::V0_19_3 => snip(cx, *range, None, fill_sep(overview), found),
            };
            Report::new("UNKNOWN EXPORT", *range, doc)
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
            Report::new("BAD EXPORT", *range, message)
        }
        Error::ImportCtorByName { range, ctor, union } => {
            let fix = (words("Try importing"))
                .chain([green(text(format!("{union}(..)")))])
                .chain(words("instead. The dots mean “expose the"))
                .chain([text(union.clone())])
                .chain(words("type and all its variants so it gives you access to"))
                .chain([text(format!("{ctor}."))]);
            let message = snip(
                cx,
                *range,
                None,
                reflow(&format!(
                    "You are trying to import the `{ctor}` variant by name:"
                )),
                fill_sep(fix),
            );
            Report::new("BAD IMPORT", *range, message)
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
            Report::new("BAD IMPORT", *range, message)
        }
        Error::ImportExposingNotFound {
            range,
            home,
            name,
            possible,
        } => {
            let possible = cx.compat.sorted_names(possible.iter().cloned());
            let after = suggestions(
                &nearest(&home.module, possible, 4),
                text("I cannot find any super similar exposed names. Maybe it is private?"),
            );
            let before = reflow(&format!(
                "The `{}` module does not expose `{name}`:",
                home.module
            ));
            Report::new("BAD IMPORT", *range, snip(cx, *range, None, before, after))
        }
        Error::NotFound {
            kind,
            range,
            prefix,
            name,
            possible,
        } => {
            let message = not_found(cx, *range, prefix.as_deref(), name, *kind, possible);
            Report::new("NAMING ERROR", *range, message)
        }
        Error::NotFoundBinop {
            range,
            op,
            possible,
        } => {
            let message = unknown_operator(cx, *range, op, possible);
            Report::new("UNKNOWN OPERATOR", *range, message)
        }
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
            Report::new("BAD PATTERN", *range, message)
        }
        Error::PortPayloadInvalid {
            range,
            name,
            problem,
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
            Report::new("PORT ERROR", *range, message)
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
            Report::new("BAD PORT", *range, message)
        }
        Error::RecursiveAlias {
            range,
            name,
            args,
            body,
            others,
        } => {
            let message = alias_recursion(cx, *range, name, args, *body, others);
            Report::new("ALIAS PROBLEM", *range, message)
        }
        Error::RecursiveDecl {
            range,
            name,
            others,
        } => {
            let intro = format!("The `{name}` definition is causing a very tricky infinite loop.");
            let message = cyclic(cx, *range, name, others, &intro);
            Report::new("CYCLIC DEFINITION", *range, message)
        }
        Error::RecursiveLet { name, others } => {
            let intro = "I do not allow cyclic values in `let` expressions.";
            let message = cyclic(cx, name.range, &name.text, others, intro);
            Report::new("CYCLIC VALUE", name.range, message)
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
                OneLine {
                    before: text("These variables cannot have the same name:"),
                    after: advice(),
                },
                TwoSnippets {
                    before: reflow(&format!("The name `{name}` is first defined here:")),
                    between: text("But then it is defined AGAIN over here:"),
                    after: advice(),
                },
            );
            Report::new("SHADOWING", *second, doc)
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
            Report::new("BAD TUPLE", *range, message)
        }
        Error::TypeVarsUnboundInUnion {
            range,
            name,
            vars,
            first,
            rest,
        } => {
            let declaration = Declaration::Union;
            unbound_type_vars(cx, *range, declaration, name, vars, first, rest)
        }
        Error::TypeVarsMessedUpInAlias {
            range,
            name,
            vars,
            unused,
            unbound,
        } => match (&unused[..], &unbound[..]) {
            ([first, rest @ ..], []) => unused_type_vars(cx, *range, name, vars, first, rest),
            ([], [first, rest @ ..]) => {
                let declaration = Declaration::Alias;
                unbound_type_vars(cx, *range, declaration, name, vars, first, rest)
            }
            _ => type_var_problems(cx, *range, name, vars, unused, unbound),
        },
    }
}

/// `nameClash`.
fn name_clash(cx: &Context<'_>, first: TextRange, second: TextRange, message: &str) -> Report {
    let fix = "How can I know which one you want? Rename one of them!";
    let doc = pair(
        (cx.trim)(first),
        (cx.trim)(second),
        OneLine {
            before: reflow(message),
            after: text(fix),
        },
        TwoSnippets {
            before: reflow(&format!("{message} One here:")),
            between: text("And another one here:"),
            after: text(fix),
        },
    );
    Report::new("NAME CLASH", second, doc)
}

/// `ambiguousName`.
fn ambiguous(
    cx: &Context<'_>,
    range: TextRange,
    prefix: Option<&str>,
    name: &str,
    homes: &[Home],
    kind: NameKind,
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
                vcat(
                    homes
                        .iter()
                        .map(|h| dull_yellow(format!("{}.{name}", h.module))),
                )
                .indent(4),
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
                let import = cyan(text("import")).space(text(h.module.clone()));
                if h.module == prefix {
                    import
                } else {
                    import.space(cyan(text("as"))).space(text(prefix))
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
                        "It could refer to a {} from {either} of these imports:",
                        thing(kind)
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
    kind: NameKind,
    possible: &PossibleNames,
) -> Doc {
    let thing = thing(kind);
    let given = match prefix {
        Some(prefix) => format!("{prefix}.{name}"),
        None => name.to_owned(),
    };
    // The qualified names, prefix by prefix, then the unqualified ones,
    // as `Map.foldrWithKey` lists them.
    let mut qualified: Vec<&(String, Vec<String>)> = possible.qualified.iter().collect();
    qualified.sort_by(|a, b| cx.compat.compare_names(&a.0, &b.0));
    let mut candidates: Vec<String> = Vec::new();
    for (prefix, names) in qualified {
        let names = cx.compat.sorted_names(names);
        candidates.extend(names.into_iter().map(|n| format!("{prefix}.{n}")));
    }
    candidates.extend(cx.compat.sorted_names(possible.locals.iter().cloned()));
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
                vcat(nearby.iter().map(|n| dull_yellow(n.clone()))).indent(4),
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
            let possible = cx.compat.sorted_names(possible.iter().cloned());
            let suggestions = nearest(op, possible, 2);
            let mut after: Vec<Doc> =
                words("Is there an `import` and `exposing` entry for it?").collect();
            if !suggestions.is_empty() {
                after.extend(words("Maybe you want"));
                let alts = suggestions.iter().map(|s| green(text(format!("({s})"))));
                after.extend(comma_sep(Conjunction::Or, alts.collect()));
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
                dull_yellow(format!("port {name} : (Int -> msg) -> Sub msg")).indent(4),
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
        fill_sep(words(question).map(dull_yellow).chain(words(details)))
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

/// `name`, then `others`, and back to `name`.
fn cycle(name: &str, others: &[String]) -> Doc {
    doc::cycle(std::iter::once(name.to_owned()).chain(others.iter().cloned()))
}

/// A value defined in terms of itself, directly, or through `others`
/// after `intro`.
fn cyclic(cx: &Context<'_>, range: TextRange, name: &str, others: &[String], intro: &str) -> Doc {
    if others.is_empty() {
        return snip(
            cx,
            range,
            None,
            reflow(&format!(
                "The `{name}` value is defined directly in terms of itself, causing an infinite loop."
            )),
            self_recursion(cx.compat, name),
        );
    }
    let chain = stack([
        reflow(&format!(
            "The `{name}` value depends on itself through the following chain of definitions:"
        )),
        cycle(name, others),
        link(
            "Hint",
            "The root problem is often a typo in some variable name, but I recommend reading",
            "bad-recursion",
            "for more detailed advice, especially if you actually do want mutually recursive values.",
        ),
    ]);
    snip(cx, range, None, reflow(intro), chain)
}

/// Which declaration a type variable problem is in.
#[derive(Clone, Copy)]
enum Declaration {
    Union,
    Alias,
}

impl Declaration {
    /// The keywords that declare it.
    fn keywords(self) -> impl Iterator<Item = Doc> {
        let keywords: &[&str] = match self {
            Declaration::Union => &["type"],
            Declaration::Alias => &["type", "alias"],
        };
        keywords.iter().map(|k| text(*k))
    }
}

/// `unboundTypeVars`.
fn unbound_type_vars(
    cx: &Context<'_>,
    range: TextRange,
    declaration: Declaration,
    name: &str,
    vars: &[String],
    first: &Spanned,
    rest: &[Spanned],
) -> Report {
    let quoted = |n: &str| text(format!("`{n}`"));
    let unbound = &first.text;
    let (title, highlight, overview) = if rest.is_empty() {
        let mut overview = vec![text("The"), quoted(name)];
        overview.extend(declaration.keywords());
        overview.extend(words("uses an unbound type variable"));
        overview.push(dull_yellow(quoted(unbound)));
        overview.extend(words("in its definition:"));
        ("UNBOUND TYPE VARIABLE", Some(first.range), overview)
    } else {
        let mut overview: Vec<Doc> = words("Type variables").collect();
        let names = std::iter::once(first)
            .chain(rest)
            .map(|n| dull_yellow(n.text.clone()));
        overview.extend(comma_sep(Conjunction::And, names.collect()));
        overview.extend(words("are unbound in the"));
        overview.push(quoted(name));
        overview.extend(declaration.keywords());
        overview.push(text("definition:"));
        ("UNBOUND TYPE VARIABLES", None, overview)
    };
    let mut declared: Vec<Doc> = declaration.keywords().collect();
    declared.push(text(name));
    declared.extend(vars.iter().map(|v| text(v.clone())));
    declared.extend(
        std::iter::once(first)
            .chain(rest)
            .map(|n| green(text(n.text.clone()))),
    );
    declared.extend([text("="), text("...")]);
    let doc = snip(
        cx,
        range,
        highlight,
        fill_sep(overview),
        stack([
            reflow("You probably need to change the declaration to something like this:"),
            doc::hsep(declared).indent(4),
            reflow(&format!(
                "Why? Well, imagine one `{name}` where `{unbound}` is an Int and another where it is a Bool. When we explicitly list the type variables, the type checker can see that they are actually different types."
            )),
        ]),
    );
    Report::new(title, range, doc)
}

/// `TypeVarsMessedUpInAlias` with unused variables alone.
fn unused_type_vars(
    cx: &Context<'_>,
    range: TextRange,
    name: &str,
    vars: &[String],
    first: &Spanned,
    rest: &[Spanned],
) -> Report {
    let quoted = |n: &str| text(format!("`{n}`"));
    let unused: Vec<&str> = std::iter::once(first)
        .chain(rest)
        .map(|a| a.text.as_str())
        .collect();
    let (title, highlight, overview, stuff) = if rest.is_empty() {
        let mut overview: Vec<Doc> = words("Type alias").collect();
        overview.push(quoted(name));
        overview.extend(words("does not use the"));
        overview.push(quoted(&first.text));
        overview.extend(words("type variable."));
        let stuff = vec![dull_yellow(quoted(&first.text))];
        ("UNUSED TYPE VARIABLE", Some(first.range), overview, stuff)
    } else {
        let mut overview: Vec<Doc> = words("Type variables").collect();
        overview.extend(comma_sep(
            Conjunction::And,
            unused.iter().map(|n| text(*n)).collect(),
        ));
        overview.extend(words("are unused in the"));
        overview.push(quoted(name));
        overview.push(text("definition."));
        let stuff = comma_sep(
            Conjunction::And,
            unused.iter().map(|n| dull_yellow(*n)).collect(),
        );
        ("UNUSED TYPE VARIABLES", None, overview, stuff)
    };
    let recommend = (words("I recommend removing"))
        .chain(stuff)
        .chain(words("from the declaration, like this:"));
    let mut declaration = vec![text("type"), text("alias"), green(text(name))];
    declaration.extend(
        vars.iter()
            .filter(|v| !unused.contains(&v.as_str()))
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
    Report::new(title, range, doc)
}

/// `TypeVarsMessedUpInAlias` with both unused and unbound variables.
fn type_var_problems(
    cx: &Context<'_>,
    range: TextRange,
    name: &str,
    vars: &[String],
    unused: &[Spanned],
    unbound: &[Spanned],
) -> Report {
    let unused: Vec<&str> = unused.iter().map(|a| a.text.as_str()).collect();
    let unbound: Vec<&str> = unbound.iter().map(|a| a.text.as_str()).collect();
    let mut used: Vec<Doc> = if let [x] = &unbound[..] {
        (words("Type variable"))
            .chain([dull_yellow(format!("`{x}`"))])
            .chain(words(
                "appears in the definition, but I do not see it declared.",
            ))
            .collect()
    } else {
        (words("Type variables"))
            .chain(comma_sep(
                Conjunction::And,
                unbound.iter().map(|n| dull_yellow(*n)).collect(),
            ))
            .chain(words(
                "are used in the definition, but I do not see them declared.",
            ))
            .collect()
    };
    if let [x] = &unused[..] {
        used.extend(words("Likewise, type variable"));
        used.push(dull_yellow(format!("`{x}`")));
        used.extend(words("is delared, but not used."));
    } else {
        used.extend(words("Likewise, type variables"));
        used.extend(comma_sep(
            Conjunction::And,
            unused.iter().map(|n| dull_yellow(*n)).collect(),
        ));
        used.extend(words("are delared, but not used."));
    }
    let mut declaration = vec![text("type"), text("alias"), text(name)];
    declaration.extend(
        vars.iter()
            .filter(|v| !unused.contains(&v.as_str()))
            .map(|v| text(v.clone())),
    );
    declaration.extend(unbound.iter().map(|n| green(text(*n))));
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
    Report::new("TYPE VARIABLE PROBLEMS", range, doc)
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
            dull_yellow(header.space(text("="))),
            green(text(name).indent(4)),
            dull_yellow(source_type(cx.module, body, Parens::App).indent(8)),
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
                cycle(name, others),
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

/// `RT.srcToDoc`: a type as the source wrote it, parenthesized as its
/// place, `context`, needs.
fn source_type(module: &hir::Module, id: TypeId, context: Parens) -> Doc {
    let go = |id, context| source_type(module, id, context);
    nomi_base::grow(|| match &module.types[id] {
        hir::Type::Lambda(arg, result) => {
            let mut rest = Vec::new();
            let mut result = *result;
            while let hir::Type::Lambda(a, r) = &module.types[result] {
                rest.push(go(*a, Parens::Func));
                result = *r;
            }
            rest.push(go(result, Parens::Func));
            let arg2 = rest.remove(0);
            render::lambda(context, go(*arg, Parens::Func), arg2, rest)
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
            let args = args.iter().map(|&a| go(a, Parens::App)).collect();
            render::apply(context, name, args)
        }
        hir::Type::Record { fields, extends } => {
            let entries = (fields.iter())
                .map(|(n, t)| (text(n.text.clone()), go(*t, Parens::None)))
                .collect();
            render::record(entries, extends.as_ref().map(|e| text(e.text.clone())))
        }
        hir::Type::Unit | hir::Type::Missing => text("()"),
        hir::Type::Tuple(items) => {
            let mut items = items.iter().map(|&t| go(t, Parens::None));
            let a = items.next().expect("two or more");
            let b = items.next().expect("two or more");
            render::tuple(a, b, items.collect())
        }
    })
}
