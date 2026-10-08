//! `Reporting/Error/Syntax.hs`: the report elm gives for each syntax
//! error, from 0.19.3 with the differences earlier releases keep. Its
//! text is upstream's, word for word, since `--report=json` carries it.

use std::borrow::Cow;

use nomi_base::Compat;
use nomi_base::diagnostic::code::{self, Next, Position, Region};
use nomi_base::diagnostic::doc::{self, Color, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::error::{BadOperator, Cursor, Error, Exposing, Module, Space};

/// `D.fillSep` of words and documents.
macro_rules! fill {
    ($($doc:expr),* $(,)?) => {
        doc::fill_sep([$(Doc::from($doc)),*])
    };
}

mod decl;
mod types;

/// `toReport`: what elm `compat` reports of `error` in `source`, or
/// `None` where the port doesn't reach yet.
pub fn to_problem(source: &code::Source<'_>, error: &Error, compat: Compat) -> Option<Problem> {
    let r = Reporter { source, compat };
    match error {
        Error::UnexpectedPort(region) => Some(r.at(
            "UNEXPECTED PORTS",
            r.region(*region),
            reflow("You are declaring ports in a normal module."),
            stack([
                fill![
                    "Switch",
                    "this",
                    "to",
                    "say",
                    cyan("port module"),
                    "instead,",
                    "marking",
                    "that",
                    "this",
                    "module",
                    "contains",
                    "port",
                    "declarations."
                ],
                doc::link(
                    "Note",
                    "Ports are not a traditional FFI for calling JS functions directly. They \
                     need a different mindset! Read",
                    "ports",
                    "to learn the syntax and how to use it effectively.",
                ),
            ]),
        )),
        Error::NoPorts(region) => Some(r.at(
            "NO PORTS",
            r.region(*region),
            reflow("This module does not declare any ports, but it says it will:"),
            fill![
                "Switch",
                "this",
                "to",
                cyan("module"),
                "and",
                "you",
                "should",
                "be",
                "all",
                "set!"
            ],
        )),
        Error::NoPortsInPackage(_, region) => Some(r.at(
            "PACKAGES CANNOT HAVE PORTS",
            r.region(*region),
            reflow("Packages cannot declare any ports, so I am getting stuck here:"),
            stack([
                reflow("Remove this port declaration."),
                note_for_ports_in_package(),
            ]),
        )),
        Error::NoPortModulesInPackage(region) => Some(r.at(
            "PACKAGES CANNOT HAVE PORTS",
            r.region(*region),
            reflow("Packages cannot declare any ports, so I am getting stuck here:"),
            stack([
                fill![
                    "Remove",
                    "the",
                    cyan("port"),
                    "keyword",
                    "and",
                    "I",
                    "should",
                    "be",
                    "able",
                    "to",
                    "continue."
                ],
                note_for_ports_in_package(),
            ]),
        )),
        Error::NoEffectsOutsideKernel(region) => Some(r.at(
            "INVALID EFFECT MODULE",
            r.region(*region),
            reflow(
                "It is not possible to declare an `effect module` outside the @elm \
                 organization, so I am getting stuck here:",
            ),
            stack([
                reflow("Switch to a normal module declaration."),
                doc::simple_note(
                    "Effect modules are designed to allow certain core functionality to be \
                     defined separately from the compiler. So the @elm organization has access \
                     to this so that certain changes, extensions, and fixes can be introduced \
                     without needing to release new Elm binaries. For example, we want to make \
                     it possible to test effects, but this may require changes to the design \
                     of effect modules. By only having them defined in the @elm organization, \
                     that kind of design work can proceed much more smoothly.",
                ),
            ]),
        )),
        Error::ParseError(module) => r.module(module),
    }
}

fn note_for_ports_in_package() -> Doc {
    stack([
        doc::simple_note(
            "One of the major goals of the package ecosystem is to be completely written in \
             Elm. This means when you install an Elm package, you can be sure you are safe \
             from security issues on install and that you are not going to get any runtime \
             exceptions coming from your new dependency. This design also sets the ecosystem \
             up to target other platforms more easily (like mobile phones, WebAssembly, etc.) \
             since no community code explicitly depends on JavaScript even existing.",
        ),
        reflow(
            "Given that overall goal, allowing ports in packages would lead to some pretty \
             surprising behavior. If ports were allowed in packages, you could install a \
             package but not realize that it brings in an indirect dependency that defines a \
             port. Now you have a program that does not work and the fix is to realize that \
             some JavaScript needs to be added for a dependency you did not even know about. \
             That would be extremely frustrating! \"So why not allow the package author to \
             include the necessary JS code as well?\" Now we are back in conflict with our \
             overall goal to keep all community packages free from runtime exceptions.",
        ),
    ])
}

// COLORS

fn cyan(d: impl Into<Doc>) -> Doc {
    d.into().color(Color::Cyan)
}

fn green(d: impl Into<Doc>) -> Doc {
    d.into().color(Color::Green)
}

fn dullyellow(d: impl Into<Doc>) -> Doc {
    d.into().color(Color::DullYellow)
}

fn blue(d: impl Into<Doc>) -> Doc {
    d.into().color(Color::Blue)
}

/// Reports from one module's text, as one release words them.
struct Reporter<'s, 'a> {
    source: &'s code::Source<'a>,
    compat: Compat,
}

impl Reporter<'_, '_> {
    /// A report with a snippet of `region`.
    fn at(
        &self,
        title: impl Into<Cow<'static, str>>,
        region: Region,
        before: Doc,
        after: Doc,
    ) -> Problem {
        let message = self
            .source
            .snippet(self.compat, before, region, None, after);
        Problem::new(title, region, message)
    }

    /// A report on `region`, with a snippet of `surroundings` that marks
    /// it.
    fn within(
        &self,
        title: impl Into<Cow<'static, str>>,
        surroundings: Region,
        region: Region,
        before: Doc,
        after: Doc,
    ) -> Problem {
        let message = (self.source).snippet(self.compat, before, surroundings, Some(region), after);
        Problem::new(title, region, message)
    }

    fn next(&self, cur: Cursor) -> Next {
        self.source.what_is_next(position(cur), self.compat)
    }

    fn region(&self, region: super::error::Region) -> Region {
        Region {
            start: position(region.start),
            end: position(region.end),
        }
    }

    /// `toWiderRegion`, and `toKeywordRegion` with the keyword's length:
    /// `n` columns from `cur`, which 0.19.1 counts in 16 bits.
    fn wider(&self, cur: Cursor, n: usize) -> Region {
        #[allow(clippy::cast_possible_truncation, reason = "a column")]
        let col = cur.col.wrapping_add(n as u32);
        let end = Position {
            row: cur.row,
            col: self.compat.column_wrap().map_or(col, |wrap| col % wrap),
        };
        Region {
            start: position(cur),
            end,
        }
    }

    fn keyword_region(&self, cur: Cursor, keyword: &str) -> Region {
        self.wider(cur, keyword.chars().count())
    }

    // MODULES

    /// `toParseErrorReport`.
    fn module(&self, module: &Module) -> Option<Problem> {
        Some(match module {
            Module::ModuleSpace(space, cur) => self.space(*space, *cur),
            // 0.19.1 checks for column 1, later releases for the line's
            // start (`isLineStart`), each its first column.
            Module::ModuleBadEnd(cur) if cur.col == self.compat.first_position() => {
                self.decl_start(*cur)
            }
            Module::ModuleBadEnd(cur) => self.weird_end(*cur),
            Module::ModuleProblem(cur) => self.at(
                "UNFINISHED MODULE DECLARATION",
                to_region(*cur),
                reflow("I am parsing an `module` declaration, but I got stuck here:"),
                stack([
                    reflow("Here are some examples of valid `module` declarations:"),
                    vcat([
                        fill![cyan("module"), "Main", cyan("exposing"), "(..)"],
                        fill![
                            cyan("module"),
                            "Dict",
                            cyan("exposing"),
                            "(Dict, empty, get)"
                        ],
                    ])
                    .indent(4),
                    reflow(
                        "I generally recommend using an explicit exposing list. I can skip \
                         compiling a bunch of files when the public interface of a module \
                         stays the same, so exposing fewer values can help improve compile \
                         times!",
                    ),
                ]),
            ),
            Module::ModuleName(cur) => self.at(
                "EXPECTING MODULE NAME",
                to_region(*cur),
                reflow("I was parsing an `module` declaration until I got stuck here:"),
                stack([
                    reflow("I was expecting to see the module name next, like in these examples:"),
                    vcat([
                        fill![cyan("module"), "Dict", cyan("exposing"), "(..)"],
                        fill![cyan("module"), "Maybe", cyan("exposing"), "(..)"],
                        fill![cyan("module"), "Html.Attributes", cyan("exposing"), "(..)"],
                        fill![cyan("module"), "Json.Decode", cyan("exposing"), "(..)"],
                    ])
                    .indent(4),
                    reflow(
                        "Notice that the module names all start with capital letters. That is \
                         required!",
                    ),
                ]),
            ),
            Module::ModuleExposing(exposing, cur) => self.exposing(exposing, *cur),
            Module::PortModuleProblem(cur) => self.at(
                "UNFINISHED PORT MODULE DECLARATION",
                to_region(*cur),
                reflow("I am parsing an `port module` declaration, but I got stuck here:"),
                stack([
                    reflow("Here are some examples of valid `port module` declarations:"),
                    port_module_examples(),
                    doc::link("Note", "Read", "ports", "for more help."),
                ]),
            ),
            Module::PortModuleName(cur) => self.at(
                "EXPECTING MODULE NAME",
                to_region(*cur),
                reflow("I was parsing an `module` declaration until I got stuck here:"),
                stack([
                    reflow("I was expecting to see the module name next, like in these examples:"),
                    port_module_examples(),
                    reflow(
                        "Notice that the module names start with capital letters. That is \
                         required!",
                    ),
                ]),
            ),
            Module::PortModuleExposing(exposing, cur) => self.exposing(exposing, *cur),
            Module::Effect(cur) => self.at(
                "BAD MODULE DECLARATION",
                to_region(*cur),
                reflow("I cannot parse this module declaration:"),
                reflow(
                    "This type of module is reserved for the @elm organization. It is used to \
                     define certain effects, avoiding building them into the compiler.",
                ),
            ),
            Module::FreshLine(cur) => self.fresh_line(*cur),
            Module::ImportStart(cur)
            | Module::ImportAs(cur)
            | Module::ImportExposing(cur)
            | Module::ImportEnd(cur)
            | Module::ImportIndentName(cur)
            | Module::ImportIndentAlias(cur) => self.import(*cur),
            Module::ImportName(cur) => self.at(
                "EXPECTING IMPORT NAME",
                to_region(*cur),
                reflow("I was parsing an `import` until I got stuck here:"),
                stack([
                    reflow("I was expecting to see a module name next, like in these examples:"),
                    vcat([
                        fill![cyan("import"), "Dict"],
                        fill![cyan("import"), "Maybe"],
                        fill![cyan("import"), "Html.Attributes", cyan("as"), "A"],
                        fill![cyan("import"), "Json.Decode", cyan("exposing"), "(..)"],
                    ])
                    .indent(4),
                    reflow(
                        "Notice that the module names all start with capital letters. That is \
                         required!",
                    ),
                    doc::reflow_link("Read", "imports", "to learn more."),
                ]),
            ),
            Module::ImportAlias(cur) => self.at(
                "EXPECTING IMPORT ALIAS",
                to_region(*cur),
                reflow("I was parsing an `import` until I got stuck here:"),
                stack([
                    reflow("I was expecting to see an alias next, like in these examples:"),
                    vcat([
                        fill![cyan("import"), "Html.Attributes", cyan("as"), "Attr"],
                        fill![cyan("import"), "WebGL.Texture", cyan("as"), "Texture"],
                        fill![cyan("import"), "Json.Decode", cyan("as"), "D"],
                    ])
                    .indent(4),
                    reflow(
                        "Notice that the alias always starts with a capital letter. That is \
                         required!",
                    ),
                    doc::reflow_link("Read", "imports", "to learn more."),
                ]),
            ),
            Module::ImportExposingList(exposing, cur) => self.exposing(exposing, *cur),
            Module::ImportIndentExposingList(cur) => self.at(
                "UNFINISHED IMPORT",
                to_region(*cur),
                reflow("I was parsing an `import` until I got stuck here:"),
                stack([
                    reflow(
                        "I was expecting to see the list of exposed values next. For example, \
                         here are two ways to expose values from the `Html` module:",
                    ),
                    vcat([
                        fill![cyan("import"), "Html", cyan("exposing"), "(..)"],
                        fill![
                            cyan("import"),
                            "Html",
                            cyan("exposing"),
                            "(Html, div, text)"
                        ],
                    ])
                    .indent(4),
                    reflow(
                        "I generally recommend the second style. It is more explicit, making it \
                         much easier to figure out where values are coming from in large \
                         projects!",
                    ),
                ]),
            ),
            Module::Infix(cur) => self.at(
                "BAD INFIX",
                to_region(*cur),
                reflow("Something went wrong in this infix operator declaration:"),
                reflow(
                    "This feature is used by the @elm organization to define the languages \
                     built-in operators.",
                ),
            ),
            Module::Declarations(decl, _) => return self.declarations(decl),
        })
    }

    fn fresh_line(&self, cur: Cursor) -> Problem {
        let region = to_region(cur);
        match self.next(cur) {
            Next::Keyword(keyword @ ("module" | "import" | "type" | "port")) => self.at(
                "TOO MUCH INDENTATION",
                region,
                reflow(&format!(
                    "This `{keyword}` should not have any spaces before it:"
                )),
                reflow(&format!(
                    "Delete the spaces before `{keyword}` until there are none left!"
                )),
            ),
            _ => self.at(
                "SYNTAX PROBLEM",
                region,
                reflow("I got stuck here:"),
                stack([
                    reflow(
                        "I am not sure what is going on, but I recommend starting an Elm file \
                         with the following lines:",
                    ),
                    vcat([
                        fill![cyan("import"), "Html"],
                        Doc::empty(),
                        Doc::from("main ="),
                        Doc::from("  Html.text ").append(dullyellow("\"Hello!\"")),
                    ])
                    .indent(4),
                    reflow(
                        "You should be able to copy those lines directly into your file. Check \
                         out the examples at <https://elm-lang.org/examples> for more help \
                         getting started!",
                    ),
                    doc::simple_note("This can also happen when something is indented too much!"),
                ]),
            ),
        }
    }

    // WEIRD END

    /// `toWeirdEndReport`.
    fn weird_end(&self, cur: Cursor) -> Problem {
        let confusing = || {
            reflow(
                "It is confusing me a lot! Normally I can give fairly specific hints, but \
                 something is really tripping me up this time.",
            )
        };
        match self.next(cur) {
            Next::Keyword(keyword) => self.at(
                "RESERVED WORD",
                self.keyword_region(cur, keyword),
                reflow("I got stuck on this reserved word:"),
                reflow(&format!(
                    "The name `{keyword}` is reserved, so try using a different name?"
                )),
            ),
            Next::Operator(op) => self.at(
                "UNEXPECTED SYMBOL",
                self.keyword_region(cur, &op),
                reflow("I ran into an unexpected symbol:"),
                reflow(&format!(
                    "I was not expecting to see a {op} here. Try deleting it? Maybe I can give \
                     a better hint from there?"
                )),
            ),
            Next::Close(term, bracket) => self.at(
                format!("UNEXPECTED {}", term.to_uppercase()),
                to_region(cur),
                reflow(&format!("I ran into an unexpected {term}:")),
                reflow(&format!(
                    "This {bracket} does not match up with an earlier open {term}. Try \
                     deleting it?"
                )),
            ),
            Next::Lower(c, cs) | Next::Upper(c, cs) => self.at(
                "UNEXPECTED NAME",
                self.keyword_region(cur, &format!("{c}{cs}")),
                reflow("I got stuck on this name:"),
                confusing(),
            ),
            Next::Other(c) => self.weird_char(cur, c),
        }
    }

    fn weird_char(&self, cur: Cursor, c: Option<char>) -> Problem {
        let region = to_region(cur);
        match c {
            Some(';') => self.at(
                "UNEXPECTED SEMICOLON",
                region,
                reflow("I got stuck on this semicolon:"),
                stack([
                    reflow("Try removing it?"),
                    doc::simple_note(
                        "Some languages require semicolons at the end of each statement. These \
                         are often called C-like languages, and they usually share a lot of \
                         language design choices. (E.g. side-effects, for loops, etc.) Elm \
                         manages effects with commands and subscriptions instead, so there is \
                         no special syntax for \"statements\" and therefore no need to use \
                         semicolons to separate them. I think this will make more sense as you \
                         work through <https://guide.elm-lang.org> though!",
                    ),
                ]),
            ),
            Some(',') => self.at(
                "UNEXPECTED COMMA",
                region,
                reflow("I got stuck on this comma:"),
                stack([
                    reflow(
                        "I do not think I am parsing a list or tuple right now. Try deleting \
                         the comma?",
                    ),
                    doc::simple_note(
                        "If this is supposed to be part of a list, the problem may be a bit \
                         earlier. Perhaps the opening [ is missing? Or perhaps some value in \
                         the list has an extra closing ] that is making me think the list ended \
                         earlier? The same kinds of things could be going wrong if this is \
                         supposed to be a tuple.",
                    ),
                ]),
            ),
            Some('`') => self.at(
                "UNEXPECTED CHARACTER",
                region,
                reflow("I got stuck on this character:"),
                stack([
                    reflow(
                        "It is not used for anything in Elm syntax. It is used for multi-line \
                         strings in some languages though, so if you want a string that spans \
                         multiple lines, you can use Elm's multi-line string syntax like this:",
                    ),
                    dullyellow(
                        vcat(
                            [
                                "\"\"\"",
                                "# Multi-line Strings",
                                "",
                                "- start with triple double quotes",
                                "- write whatever you want",
                                "- no need to escape newlines or double quotes",
                                "- end with triple double quotes",
                                "\"\"\"",
                            ]
                            .map(Doc::from),
                        )
                        .indent(4),
                    ),
                    reflow("Otherwise I do not know what is going on! Try removing the character?"),
                ]),
            ),
            Some('$') => self.at(
                "UNEXPECTED SYMBOL",
                region,
                reflow("I got stuck on this dollar sign:"),
                reflow(
                    "It is not used for anything in Elm syntax. Are you coming from a language \
                     where dollar signs can be used in variable names? If so, try a name that \
                     (1) starts with a letter and (2) only contains letters, numbers, and \
                     underscores.",
                ),
            ),
            Some('#' | '@' | '!' | '%' | '~') => self.at(
                "UNEXPECTED SYMBOL",
                region,
                reflow("I got stuck on this symbol:"),
                reflow("It is not used for anything in Elm syntax. Try removing it?"),
            ),
            _ => self.at(
                "SYNTAX PROBLEM",
                region,
                reflow("I got stuck here:"),
                reflow(
                    "Whatever I am running into is confusing me a lot! Normally I can give \
                     fairly specific hints, but something is really tripping me up this time.",
                ),
            ),
        }
    }

    // IMPORTS

    /// `toImportReport`.
    fn import(&self, cur: Cursor) -> Problem {
        self.at(
            "UNFINISHED IMPORT",
            to_region(cur),
            reflow("I am partway through parsing an import, but I got stuck here:"),
            stack([
                reflow("Here are some examples of valid `import` declarations:"),
                vcat([
                    fill![cyan("import"), "Html"],
                    fill![cyan("import"), "Html", cyan("as"), "H"],
                    fill![
                        cyan("import"),
                        "Html",
                        cyan("as"),
                        "H",
                        cyan("exposing"),
                        "(..)"
                    ],
                    fill![
                        cyan("import"),
                        "Html",
                        cyan("exposing"),
                        "(Html, div, text)"
                    ],
                ])
                .indent(4),
                reflow(
                    "You are probably trying to import a different module, but try to make it \
                     look like one of these examples!",
                ),
                doc::reflow_link("Read", "imports", "to learn more."),
            ]),
        )
    }

    // EXPOSING

    /// `toExposingReport`.
    fn exposing(&self, exposing: &Exposing, start: Cursor) -> Problem {
        let within = |cur: Cursor| Region {
            start: position(start),
            end: position(cur),
        };
        let partway =
            || reflow("I was partway through parsing exposed values, but I got stuck here:");
        match exposing {
            Exposing::ExposingSpace(space, cur) => self.space(*space, *cur),
            Exposing::ExposingStart(cur) => self.within(
                "PROBLEM IN EXPOSING",
                within(*cur),
                to_region(*cur),
                reflow("I want to parse exposed values, but I am getting stuck here:"),
                stack([
                    fill![
                        "Exposed",
                        "values",
                        "are",
                        "always",
                        "surrounded",
                        "by",
                        "parentheses.",
                        "So",
                        "try",
                        "adding",
                        "a",
                        green("("),
                        "here?"
                    ],
                    doc::simple_note("Here are some valid examples of `exposing` for reference:"),
                    vcat([
                        fill![cyan("import"), "Html", cyan("exposing"), "(..)"],
                        fill![
                            cyan("import"),
                            "Html",
                            cyan("exposing"),
                            "(Html, div, text)"
                        ],
                    ])
                    .indent(4),
                    reflow(
                        "If you are getting tripped up, you can just expose everything for now. \
                         It should get easier to make an explicit exposing list as you see more \
                         examples in the wild.",
                    ),
                ]),
            ),
            Exposing::ExposingValue(cur) => self.exposing_value(within(*cur), *cur),
            Exposing::ExposingOperator(cur) => self.within(
                "PROBLEM IN EXPOSING",
                within(*cur),
                to_region(*cur),
                reflow("I just saw an open parenthesis, so I was expecting an operator next:"),
                fill![
                    "It",
                    "is",
                    "possible",
                    "to",
                    "expose",
                    "operators,",
                    "so",
                    "I",
                    "was",
                    "expecting",
                    "to",
                    "see",
                    "something",
                    "like",
                    dullyellow("(+)"),
                    "or",
                    dullyellow("(|=)"),
                    "or",
                    dullyellow("(||)"),
                    "after",
                    "I",
                    "saw",
                    "that",
                    "open",
                    "parenthesis."
                ],
            ),
            Exposing::ExposingOperatorReserved(op, cur) => {
                let instead = |op: &str| fill!["Maybe", "you", "want", dullyellow(op), "instead?"];
                let rid = || {
                    reflow(
                        "Try getting rid of this entry? Maybe I can give you a better hint \
                         after that?",
                    )
                };
                self.within(
                    "RESERVED SYMBOL",
                    within(*cur),
                    to_region(*cur),
                    reflow("I cannot expose this as an operator:"),
                    match op {
                        BadOperator::BadDot | BadOperator::BadArrow => rid(),
                        BadOperator::BadPipe => instead("(||)"),
                        BadOperator::BadEquals => instead("(==)"),
                        BadOperator::BadHasType => instead("(::)"),
                    },
                )
            }
            Exposing::ExposingOperatorRightParen(cur) => self.within(
                "PROBLEM IN EXPOSING",
                within(*cur),
                to_region(*cur),
                reflow("It looks like you are exposing an operator, but I got stuck here:"),
                fill![
                    "I",
                    "was",
                    "expecting",
                    "to",
                    "see",
                    "the",
                    "closing",
                    "parenthesis",
                    "immediately",
                    "after",
                    "the",
                    "operator.",
                    "Try",
                    "adding",
                    "a",
                    green(")"),
                    "right",
                    "here?"
                ],
            ),
            Exposing::ExposingEnd(cur) => self.within(
                "UNFINISHED EXPOSING",
                within(*cur),
                to_region(*cur),
                partway(),
                reflow("Maybe there is a comma missing before this?"),
            ),
            Exposing::ExposingTypePrivacy(cur) => self.within(
                "PROBLEM EXPOSING CUSTOM TYPE VARIANTS",
                within(*cur),
                to_region(*cur),
                reflow("It looks like you are trying to expose the variants of a custom type:"),
                stack([
                    fill![
                        "You",
                        "need",
                        "to",
                        "write",
                        "something",
                        "like",
                        dullyellow("Status(..)"),
                        "or",
                        dullyellow("Entity(..)"),
                        "though.",
                        "It",
                        "is",
                        "all",
                        "or",
                        "nothing,",
                        "otherwise",
                        "`case`",
                        "expressions",
                        "could",
                        "miss",
                        "a",
                        "variant",
                        "and",
                        "crash!"
                    ],
                    doc::simple_note(
                        "It is often best to keep the variants hidden! If someone pattern \
                         matches on the variants, it is a MAJOR change if any new variants are \
                         added. Suddenly their `case` expressions do not cover all variants! So \
                         if you do not need people to pattern match, keep the variants hidden \
                         and expose functions to construct values of this type. This way you \
                         can add new variants as a MINOR change!",
                    ),
                ]),
            ),
            Exposing::ExposingIndentEnd(cur) => self.within(
                "UNFINISHED EXPOSING",
                within(*cur),
                to_region(*cur),
                partway(),
                stack([
                    fill![
                        "I",
                        "was",
                        "expecting",
                        "a",
                        "closing",
                        "parenthesis.",
                        "Try",
                        "adding",
                        "a",
                        green(")"),
                        "right",
                        "here?"
                    ],
                    doc::simple_note(
                        "I can get confused when there is not enough indentation, so if you \
                         already have a closing parenthesis, it probably just needs some spaces \
                         in front of it.",
                    ),
                ]),
            ),
            Exposing::ExposingIndentValue(cur) => self.within(
                "UNFINISHED EXPOSING",
                within(*cur),
                to_region(*cur),
                partway(),
                reflow("I was expecting another value to expose."),
            ),
        }
    }

    fn exposing_value(&self, surroundings: Region, cur: Cursor) -> Problem {
        match self.next(cur) {
            Next::Keyword(keyword) => self.within(
                "RESERVED WORD",
                surroundings,
                self.keyword_region(cur, keyword),
                reflow("I got stuck on this reserved word:"),
                reflow(&format!(
                    "It looks like you are trying to expose `{keyword}` but that is a reserved \
                     word. Is there a typo?"
                )),
            ),
            Next::Operator(op) => self.within(
                "UNEXPECTED SYMBOL",
                surroundings,
                self.keyword_region(cur, &op),
                reflow("I got stuck on this symbol:"),
                stack([
                    reflow(
                        "If you are trying to expose an operator, add parentheses around it \
                         like this:",
                    ),
                    dullyellow(op.as_str())
                        .append(Doc::from(" -> "))
                        .append(green(format!("({op})")))
                        .indent(4),
                ]),
            ),
            _ => self.within(
                "PROBLEM IN EXPOSING",
                surroundings,
                to_region(cur),
                reflow("I got stuck while parsing these exposed values:"),
                stack([
                    reflow(
                        "I do not have an exact recommendation, so here are some valid examples \
                         of `exposing` for reference:",
                    ),
                    vcat([
                        fill![cyan("import"), "Html", cyan("exposing"), "(..)"],
                        fill![
                            cyan("import"),
                            "Basics",
                            cyan("exposing"),
                            "(Int, Float, Bool(..), (+), not, sqrt)"
                        ],
                    ])
                    .indent(4),
                    reflow(
                        "These examples show how to expose types, variants, operators, and \
                         functions. Everything should be some permutation of these examples, \
                         just with different names.",
                    ),
                ]),
            ),
        }
    }

    // SPACES

    /// `toSpaceReport`.
    fn space(&self, space: Space, cur: Cursor) -> Problem {
        match space {
            Space::HasTab => self.at(
                "NO TABS",
                to_region(cur),
                reflow("I ran into a tab, but tabs are not allowed in Elm files."),
                reflow("Replace the tab with spaces."),
            ),
            Space::HasNonUtf8 => self.at(
                "UNEXPECTED ENCODING",
                to_region(cur),
                reflow(
                    "Elm files use UTF-8 character encoding, but I ran into something outside \
                     of that format:",
                ),
                reflow("Is there a way to convert this to a valid UTF-8 character?"),
            ),
            Space::EndlessMultiComment => self.at(
                "ENDLESS COMMENT",
                self.wider(cur, 2),
                reflow("I cannot find the end of this multi-line comment:"),
                stack([
                    reflow("Add a -} somewhere after this to end the comment."),
                    doc::simple_hint(
                        "Multi-line comments can be nested in Elm, so {- {- -} -} is a comment \
                         that happens to contain another comment. Like parentheses and curly \
                         braces, the start and end markers must always be balanced. Maybe that \
                         is the problem?",
                    ),
                ]),
            ),
        }
    }
}

fn port_module_examples() -> Doc {
    vcat([
        fill![
            cyan("port"),
            cyan("module"),
            "WebSockets",
            cyan("exposing"),
            "(send, listen, keepAlive)"
        ],
        fill![
            cyan("port"),
            cyan("module"),
            "Maps",
            cyan("exposing"),
            "(Location, goto)"
        ],
    ])
    .indent(4)
}

fn position(cur: Cursor) -> Position {
    Position {
        row: cur.row,
        col: cur.col,
    }
}

/// `toRegion`.
fn to_region(cur: Cursor) -> Region {
    Region {
        start: position(cur),
        end: position(cur),
    }
}

/// `A.Region startCur cur`.
fn surroundings(start: Cursor, cur: Cursor) -> Region {
    Region {
        start: position(start),
        end: position(cur),
    }
}
