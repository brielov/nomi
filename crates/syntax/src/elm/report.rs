//! `Reporting/Error/Syntax.hs`: the report elm gives for each syntax
//! error, from 0.19.3 with the differences earlier releases keep. Its
//! text is upstream's, word for word, since `--report=json` carries it.

use std::borrow::Cow;

use nomi_base::Compat;
use nomi_base::diagnostic::code::{self, Bracket, Keyword, Next};
use nomi_base::diagnostic::doc::{self, cyan, dull_yellow, green, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::error::{BadOperator, Error, Exposing, Module, Position, Region, Space};

/// Upstream's lists of words, written as phrases: a string stands for
/// its words, a document for itself.
macro_rules! words {
    ($($part:expr),* $(,)?) => {
        [$($crate::elm::report::Words::words($part)),*].into_iter().flatten()
    };
}

/// `D.fillSep` of [`words!`]: a sentence, wrapped.
macro_rules! fill {
    ($($part:expr),* $(,)?) => {
        nomi_base::diagnostic::doc::fill_sep(words![$($part),*])
    };
}

mod blocks;
mod collections;
mod decl;
mod expr;
mod literal;
mod pattern;
mod types;

/// `toReport`: what elm `compat` reports of `error` in `source`.
pub fn to_problem(source: &code::Source<'_>, error: &Error, compat: Compat) -> Problem {
    let r = Reporter { source, compat };
    match error {
        Error::UnexpectedPort(region) => r.at(
            "UNEXPECTED PORTS",
            *region,
            reflow("You are declaring ports in a normal module."),
            stack([
                fill![
                    "Switch this to say",
                    cyan("port module"),
                    "instead, marking that this module contains port declarations."
                ],
                doc::link(
                    "Note",
                    "Ports are not a traditional FFI for calling JS functions directly. They \
                     need a different mindset! Read",
                    "ports",
                    "to learn the syntax and how to use it effectively.",
                ),
            ]),
        ),
        Error::NoPorts(region) => r.at(
            "NO PORTS",
            *region,
            reflow("This module does not declare any ports, but it says it will:"),
            fill![
                "Switch this to",
                cyan("module"),
                "and you should be all set!"
            ],
        ),
        Error::NoPortsInPackage(region) => r.at(
            "PACKAGES CANNOT HAVE PORTS",
            *region,
            reflow("Packages cannot declare any ports, so I am getting stuck here:"),
            stack([
                reflow("Remove this port declaration."),
                note_for_ports_in_package(),
            ]),
        ),
        Error::NoPortModulesInPackage(region) => r.at(
            "PACKAGES CANNOT HAVE PORTS",
            *region,
            reflow("Packages cannot declare any ports, so I am getting stuck here:"),
            stack([
                fill![
                    "Remove the",
                    cyan("port"),
                    "keyword and I should be able to continue."
                ],
                note_for_ports_in_package(),
            ]),
        ),
        Error::NoEffectsOutsideKernel(region) => r.at(
            "INVALID EFFECT MODULE",
            *region,
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
        ),
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

/// What [`words!`] takes.
trait Words {
    fn words(self) -> Vec<Doc>;
}

impl Words for &str {
    fn words(self) -> Vec<Doc> {
        doc::words(self).collect()
    }
}

impl Words for Doc {
    fn words(self) -> Vec<Doc> {
        vec![self]
    }
}

/// `toSyntaxProblemReport`: what elm `compat` reports of a module
/// documentation it can't parse (`Reporting/Error/Docs.hs`).
pub fn docs_problem(
    source: &code::Source<'_>,
    problem: &super::docs::SyntaxProblem,
    compat: Compat,
) -> Problem {
    use super::docs::SyntaxProblem as S;
    let r = Reporter { source, compat };
    let (cur, details) = match *problem {
        S::Op(cur) => (
            cur,
            "I am trying to parse an operator like (+) or (*) but something is going wrong.",
        ),
        S::OpBad(cur) => (
            cur,
            "I am trying to parse an operator like (+) or (*) but it looks like you are using \
             a reserved symbol in this case.",
        ),
        S::Name(cur) => (
            cur,
            "I was expecting to see the name of another exposed value from this module.",
        ),
        S::Space(space, cur) => return r.space(space, cur),
        S::Comma(cur) => (cur, "I was expecting to see a comma next."),
        S::BadEnd(cur) => (
            cur,
            "I am not really sure what I am getting stuck on though.",
        ),
    };
    r.at(
        "PROBLEM IN DOCS",
        to_region(cur),
        reflow("I was partway through parsing your module documentation, but I got stuck here:"),
        stack([
            reflow(details),
            doc::simple_hint(
                "Read through <https://package.elm-lang.org/help/documentation-format> for \
                 tips on how to write module documentation!",
            ),
        ]),
    )
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

    /// A report on `cur`, with a snippet from `start` on that marks it:
    /// how upstream reports getting stuck partway through something.
    fn stuck(
        &self,
        title: impl Into<Cow<'static, str>>,
        start: Position,
        cur: Position,
        before: Doc,
        after: Doc,
    ) -> Problem {
        self.within(
            title,
            surroundings(start, cur),
            to_region(cur),
            before,
            after,
        )
    }

    /// A "RESERVED WORD" report on `keyword` at `cur`, with a snippet
    /// from `start` on.
    fn reserved(
        &self,
        start: Position,
        cur: Position,
        keyword: Keyword,
        before: Doc,
        after: Doc,
    ) -> Problem {
        let region = self.keyword_region(cur, keyword.as_str());
        self.within(
            "RESERVED WORD",
            surroundings(start, cur),
            region,
            before,
            after,
        )
    }

    fn next(&self, cur: Position) -> Next {
        self.source.what_is_next(cur, self.compat)
    }

    /// The reserved word at `cur`, if one is next.
    fn next_keyword(&self, cur: Position) -> Option<Keyword> {
        match self.next(cur) {
            Next::Keyword(keyword) => Some(keyword),
            Next::Operator(_)
            | Next::Close(_)
            | Next::Upper(..)
            | Next::Lower(..)
            | Next::Other(_) => None,
        }
    }

    /// `toWiderRegion`, and `toKeywordRegion` with the keyword's length:
    /// `n` columns from `cur`, which 0.19.1 counts in 16 bits.
    fn wider(&self, cur: Position, n: u32) -> Region {
        let end = Position {
            row: cur.row,
            col: self.compat.wrap(cur.col.wrapping_add(n)),
        };
        Region { start: cur, end }
    }

    fn keyword_region(&self, cur: Position, keyword: &str) -> Region {
        #[expect(clippy::cast_possible_truncation, reason = "a column")]
        let width = keyword.chars().count() as u32;
        self.wider(cur, width)
    }

    // MODULES

    /// `toParseErrorReport`.
    fn module(&self, module: &Module) -> Problem {
        match module {
            Module::ModuleSpace(space, cur) => self.space(*space, *cur),
            // 0.19.1 checks for column 1, later releases for the line's
            // start (`isLineStart`), each its first column.
            Module::ModuleBadEnd(cur) if cur.col == self.compat.first_position() => {
                self.decl_start(*cur)
            }
            Module::ModuleBadEnd(cur) => self.weird_end(*cur),
            Module::ModuleProblem(cur) => self.unfinished_module(*cur),
            Module::ModuleName(cur) => self.expecting_module_name(
                *cur,
                vcat([
                    fill![cyan("module"), "Dict", cyan("exposing"), "(..)"],
                    fill![cyan("module"), "Maybe", cyan("exposing"), "(..)"],
                    fill![cyan("module"), "Html.Attributes", cyan("exposing"), "(..)"],
                    fill![cyan("module"), "Json.Decode", cyan("exposing"), "(..)"],
                ])
                .indent(4),
                "Notice that the module names all start with capital letters. That is required!",
            ),
            Module::ModuleExposing(exposing, cur)
            | Module::PortModuleExposing(exposing, cur)
            | Module::ImportExposingList(exposing, cur) => self.exposing(exposing, *cur),
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
            Module::PortModuleName(cur) => self.expecting_module_name(
                *cur,
                port_module_examples(),
                "Notice that the module names start with capital letters. That is required!",
            ),
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
            Module::ImportName(cur) => self.expecting_import(
                *cur,
                "EXPECTING IMPORT NAME",
                "I was expecting to see a module name next, like in these examples:",
                vcat([
                    fill![cyan("import"), "Dict"],
                    fill![cyan("import"), "Maybe"],
                    fill![cyan("import"), "Html.Attributes", cyan("as"), "A"],
                    fill![cyan("import"), "Json.Decode", cyan("exposing"), "(..)"],
                ]),
                "Notice that the module names all start with capital letters. That is required!",
            ),
            Module::ImportAlias(cur) => self.expecting_import(
                *cur,
                "EXPECTING IMPORT ALIAS",
                "I was expecting to see an alias next, like in these examples:",
                vcat([
                    fill![cyan("import"), "Html.Attributes", cyan("as"), "Attr"],
                    fill![cyan("import"), "WebGL.Texture", cyan("as"), "Texture"],
                    fill![cyan("import"), "Json.Decode", cyan("as"), "D"],
                ]),
                "Notice that the alias always starts with a capital letter. That is required!",
            ),
            Module::ImportIndentExposingList(cur) => self.at(
                "UNFINISHED IMPORT",
                to_region(*cur),
                reflow("I was parsing an `import` until I got stuck here:"),
                stack([
                    reflow(
                        "I was expecting to see the list of exposed values next. For example, \
                         here are two ways to expose values from the `Html` module:",
                    ),
                    html_exposing_examples(),
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
            Module::Declarations(decl) => self.declarations(decl),
        }
    }

    /// `ModuleProblem`.
    fn unfinished_module(&self, cur: Position) -> Problem {
        self.at(
            "UNFINISHED MODULE DECLARATION",
            to_region(cur),
            reflow("I am parsing an `module` declaration, but I got stuck here:"),
            stack([
                reflow("Here are some examples of valid `module` declarations:"),
                vcat([
                    fill![cyan("module"), "Main", cyan("exposing"), "(..)"],
                    fill![
                        cyan("module"),
                        "Dict",
                        cyan("exposing"),
                        Doc::from("(Dict, empty, get)")
                    ],
                ])
                .indent(4),
                reflow(
                    "I generally recommend using an explicit exposing list. I can skip compiling \
                     a bunch of files when the public interface of a module stays the same, so \
                     exposing fewer values can help improve compile times!",
                ),
            ]),
        )
    }

    /// `ModuleName` and `PortModuleName`: the examples, and what they show.
    fn expecting_module_name(&self, cur: Position, examples: Doc, notice: &str) -> Problem {
        self.at(
            "EXPECTING MODULE NAME",
            to_region(cur),
            reflow("I was parsing an `module` declaration until I got stuck here:"),
            stack([
                reflow("I was expecting to see the module name next, like in these examples:"),
                examples,
                reflow(notice),
            ]),
        )
    }

    /// `ImportName` and `ImportAlias`: what was expected, examples of it,
    /// and what they show.
    fn expecting_import(
        &self,
        cur: Position,
        title: &'static str,
        expecting: &str,
        examples: Doc,
        notice: &str,
    ) -> Problem {
        self.at(
            title,
            to_region(cur),
            reflow("I was parsing an `import` until I got stuck here:"),
            stack([
                reflow(expecting),
                examples.indent(4),
                reflow(notice),
                doc::reflow_link("Read", "imports", "to learn more."),
            ]),
        )
    }

    fn fresh_line(&self, cur: Position) -> Problem {
        let region = to_region(cur);
        match self.next_keyword(cur) {
            Some(keyword @ (Keyword::Module | Keyword::Import | Keyword::Type | Keyword::Port)) => {
                self.at(
                    "TOO MUCH INDENTATION",
                    region,
                    reflow(&format!(
                        "This `{keyword}` should not have any spaces before it:"
                    )),
                    reflow(&format!(
                        "Delete the spaces before `{keyword}` until there are none left!"
                    )),
                )
            }
            Some(_) | None => self.at(
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
                        Doc::from("  Html.text ").append(dull_yellow("\"Hello!\"")),
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
    fn weird_end(&self, cur: Position) -> Problem {
        let confusing = || {
            reflow(
                "It is confusing me a lot! Normally I can give fairly specific hints, but \
                 something is really tripping me up this time.",
            )
        };
        match self.next(cur) {
            Next::Keyword(keyword) => self.at(
                "RESERVED WORD",
                self.keyword_region(cur, keyword.as_str()),
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
            Next::Close(close) => {
                let (term, bracket) = (close.name(), close.char());
                self.at(
                    format!("UNEXPECTED {}", term.to_uppercase()),
                    to_region(cur),
                    reflow(&format!("I ran into an unexpected {term}:")),
                    reflow(&format!(
                        "This {bracket} does not match up with an earlier open {term}. Try \
                     deleting it?"
                    )),
                )
            }
            Next::Lower(c, cs) | Next::Upper(c, cs) => self.at(
                "UNEXPECTED NAME",
                self.keyword_region(cur, &format!("{c}{cs}")),
                reflow("I got stuck on this name:"),
                confusing(),
            ),
            Next::Other(c) => self.weird_char(cur, c),
        }
    }

    fn weird_char(&self, cur: Position, c: Option<char>) -> Problem {
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
                    literal::multi_line_string_example(),
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
    fn import(&self, cur: Position) -> Problem {
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
                        Doc::from("(Html, div, text)")
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
    fn exposing(&self, exposing: &Exposing, start: Position) -> Problem {
        let partway =
            || reflow("I was partway through parsing exposed values, but I got stuck here:");
        match exposing {
            Exposing::ExposingSpace(space, cur) => self.space(*space, *cur),
            Exposing::ExposingStart(cur) => self.stuck(
                "PROBLEM IN EXPOSING",
                start,
                *cur,
                reflow("I want to parse exposed values, but I am getting stuck here:"),
                stack([
                    fill![
                        "Exposed values are always surrounded by parentheses. So try adding a",
                        green("("),
                        "here?"
                    ],
                    doc::simple_note("Here are some valid examples of `exposing` for reference:"),
                    html_exposing_examples(),
                    reflow(
                        "If you are getting tripped up, you can just expose everything for now. \
                         It should get easier to make an explicit exposing list as you see more \
                         examples in the wild.",
                    ),
                ]),
            ),
            Exposing::ExposingValue(cur) => self.exposing_value(start, *cur),
            Exposing::ExposingOperator(cur) => self.stuck(
                "PROBLEM IN EXPOSING",
                start,
                *cur,
                reflow("I just saw an open parenthesis, so I was expecting an operator next:"),
                fill![
                    "It is possible to expose operators, so I was expecting to see something like",
                    dull_yellow("(+)"),
                    "or",
                    dull_yellow("(|=)"),
                    "or",
                    dull_yellow("(||)"),
                    "after I saw that open parenthesis."
                ],
            ),
            Exposing::ExposingOperatorReserved(op, cur) => self.stuck(
                "RESERVED SYMBOL",
                start,
                *cur,
                reflow("I cannot expose this as an operator:"),
                reserved_operator_hint(*op),
            ),
            Exposing::ExposingOperatorRightParen(cur) => self.stuck(
                "PROBLEM IN EXPOSING",
                start,
                *cur,
                reflow("It looks like you are exposing an operator, but I got stuck here:"),
                fill![
                    "I was expecting to see the closing parenthesis immediately after the \
                     operator. Try adding a",
                    green(")"),
                    "right here?"
                ],
            ),
            Exposing::ExposingEnd(cur) => self.stuck(
                "UNFINISHED EXPOSING",
                start,
                *cur,
                partway(),
                reflow("Maybe there is a comma missing before this?"),
            ),
            Exposing::ExposingTypePrivacy(cur) => self.exposing_privacy(start, *cur),
            Exposing::ExposingIndentEnd(cur) => self.stuck(
                "UNFINISHED EXPOSING",
                start,
                *cur,
                partway(),
                stack([
                    fill![
                        "I was expecting a closing parenthesis. Try adding a",
                        green(")"),
                        "right here?"
                    ],
                    doc::simple_note(
                        "I can get confused when there is not enough indentation, so if you \
                         already have a closing parenthesis, it probably just needs some spaces \
                         in front of it.",
                    ),
                ]),
            ),
            Exposing::ExposingIndentValue(cur) => self.stuck(
                "UNFINISHED EXPOSING",
                start,
                *cur,
                partway(),
                reflow("I was expecting another value to expose."),
            ),
        }
    }

    /// `ExposingTypePrivacy`.
    fn exposing_privacy(&self, start: Position, cur: Position) -> Problem {
        self.stuck(
            "PROBLEM EXPOSING CUSTOM TYPE VARIANTS",
            start,
            cur,
            reflow("It looks like you are trying to expose the variants of a custom type:"),
            stack([
                fill![
                    "You need to write something like",
                    dull_yellow("Status(..)"),
                    "or",
                    dull_yellow("Entity(..)"),
                    "though. It is all or nothing, otherwise `case` expressions could miss a \
                     variant and crash!"
                ],
                doc::simple_note(
                    "It is often best to keep the variants hidden! If someone pattern matches \
                     on the variants, it is a MAJOR change if any new variants are added. \
                     Suddenly their `case` expressions do not cover all variants! So if you do \
                     not need people to pattern match, keep the variants hidden and expose \
                     functions to construct values of this type. This way you can add new \
                     variants as a MINOR change!",
                ),
            ]),
        )
    }

    /// `ExposingValue`, by what is there.
    fn exposing_value(&self, start: Position, cur: Position) -> Problem {
        match self.next(cur) {
            Next::Keyword(keyword) => self.reserved(
                start,
                cur,
                keyword,
                reflow("I got stuck on this reserved word:"),
                reflow(&format!(
                    "It looks like you are trying to expose `{keyword}` but that is a reserved \
                     word. Is there a typo?"
                )),
            ),
            Next::Operator(op) => self.within(
                "UNEXPECTED SYMBOL",
                surroundings(start, cur),
                self.keyword_region(cur, &op),
                reflow("I got stuck on this symbol:"),
                stack([
                    reflow(
                        "If you are trying to expose an operator, add parentheses around it \
                         like this:",
                    ),
                    dull_yellow(op.as_str())
                        .append(Doc::from(" -> "))
                        .append(green(format!("({op})")))
                        .indent(4),
                ]),
            ),
            Next::Close(_) | Next::Upper(..) | Next::Lower(..) | Next::Other(_) => self.stuck(
                "PROBLEM IN EXPOSING",
                start,
                cur,
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
                            Doc::from("(Int, Float, Bool(..), (+), not, sqrt)")
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
    fn space(&self, space: Space, cur: Position) -> Problem {
        match space {
            Space::HasTab => self.at(
                "NO TABS",
                to_region(cur),
                reflow("I ran into a tab, but tabs are not allowed in Elm files."),
                reflow("Replace the tab with spaces."),
            ),
            Space::HasNonUtf8 => self.not_utf8(cur),
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

/// What `ExposingOperatorReserved` suggests of each reserved symbol.
fn reserved_operator_hint(op: BadOperator) -> Doc {
    let instead = |op: &str| fill!["Maybe you want", dull_yellow(op), "instead?"];
    match op {
        BadOperator::BadDot | BadOperator::BadArrow => {
            reflow("Try getting rid of this entry? Maybe I can give you a better hint after that?")
        }
        BadOperator::BadPipe => instead("(||)"),
        BadOperator::BadEquals => instead("(==)"),
        BadOperator::BadHasType => instead("(::)"),
    }
}

/// Two ways to expose values from `Html`.
fn html_exposing_examples() -> Doc {
    vcat([
        fill![cyan("import"), "Html", cyan("exposing"), "(..)"],
        fill![
            cyan("import"),
            "Html",
            cyan("exposing"),
            Doc::from("(Html, div, text)")
        ],
    ])
    .indent(4)
}

fn port_module_examples() -> Doc {
    vcat([
        fill![
            cyan("port"),
            cyan("module"),
            "WebSockets",
            cyan("exposing"),
            Doc::from("(send, listen, keepAlive)")
        ],
        fill![
            cyan("port"),
            cyan("module"),
            "Maps",
            cyan("exposing"),
            Doc::from("(Location, goto)")
        ],
    ])
    .indent(4)
}

/// `toRegion`.
fn to_region(cur: Position) -> Region {
    Region {
        start: cur,
        end: cur,
    }
}

/// `A.Region startCur cur`.
fn surroundings(start: Position, cur: Position) -> Region {
    Region { start, end: cur }
}

// FRAGMENTS SEVERAL REPORTS SHARE

impl Reporter<'_, '_> {
    /// "NEED MORE INDENTATION", if the line after `cur` starts with a
    /// closing curly brace: the end of a record or record type, which
    /// `partway` names, not indented enough.
    fn curly_needs_indent(
        &self,
        start: Position,
        cur: Position,
        partway: &str,
        note: fn() -> Doc,
    ) -> Option<Problem> {
        let curly = (self.source).next_line_starts_with_close_curly(cur, self.compat)?;
        Some(self.within(
            "NEED MORE INDENTATION",
            Region { start, end: curly },
            to_region(curly),
            reflow(partway),
            stack([
                reflow(
                    "I need this curly brace to be indented more. Try adding some spaces before \
                     it!",
                ),
                note(),
            ]),
        ))
    }
}

/// "Try adding a `close` to see if that helps", and the sentence's end.
fn try_adding(close: Bracket, ending: char) -> Doc {
    fill![
        "Try adding a",
        dull_yellow(String::from(close.char())),
        "to see if that",
        Doc::from(format!("helps{ending}"))
    ]
}

/// What a record field or record type field named by a reserved word
/// gets told.
fn reserved_field(keyword: Keyword) -> Doc {
    reflow(&format!(
        "It looks like you are trying to use `{keyword}` as a field name, but that is a reserved \
         word. Try using a different name!"
    ))
}

/// The end of a record or record type that has no closing curly brace.
fn closing_curly_next() -> Doc {
    fill![
        "I was expecting to see a closing curly brace next. Try putting a",
        green("}"),
        "next and see if that helps?"
    ]
}

/// The end of a record or record type that something else stops.
fn curly_before_this() -> Doc {
    stack([
        fill![
            "I was expecting to see a closing curly brace before this, so try adding a",
            dull_yellow("}"),
            "and see if that helps?"
        ],
        missing_bracket_note(),
    ])
}

/// The note on a closing bracket missing before a stray one.
fn missing_bracket_note() -> Doc {
    doc::simple_note(
        "When I get stuck like this, it usually means that there is a missing parenthesis or \
         bracket somewhere earlier. It could also be a stray keyword or operator.",
    )
}

/// The note on a closing parenthesis missing after something unexpected.
fn earlier_trouble_note() -> Doc {
    doc::simple_note(
        "I can get stuck when I run into keywords, operators, parentheses, or brackets \
         unexpectedly. So there may be some earlier syntax trouble (like extra parenthesis or \
         missing brackets) that is confusing me.",
    )
}

/// The note on a closing parenthesis not indented enough.
fn closing_paren_note() -> Doc {
    doc::simple_note(
        "I can get confused by indentation in cases like this, so maybe you have a closing \
         parenthesis but it is not indented enough?",
    )
}

/// The style the record, record type and list examples follow, and how
/// the sentence saying so ends.
fn convention(ending: char) -> Doc {
    reflow(&format!(
        "Notice that each line starts with some indentation. Usually two or four spaces. This is \
         the stylistic convention in the Elm ecosystem{ending}"
    ))
}
