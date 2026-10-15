//! The reports on declarations: `toDeclarationsReport` and the reports
//! it leads to, but for the types, expressions and patterns inside them.

use nomi_base::diagnostic::code::{Keyword, Next};
use nomi_base::diagnostic::doc::{self, blue, cyan, dull_yellow, green, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::expr::Context;
use super::pattern::PContext;
use super::types::TypeContext;
use super::{Reporter, stray_title, surroundings, to_region, unmatched_close};
use crate::elm::error::{CustomType, Decl, DeclDef, DeclType, Port, Position, TypeAlias};

/// What a definition was missing where it stopped, as the `*IndentType`,
/// `*IndentEquals` and `*IndentBody` errors of `DeclDef` and `Def` say.
#[derive(Clone, Copy)]
pub(super) enum Missing {
    Type,
    Equals,
    Body,
}

impl Reporter<'_, '_> {
    /// `toDeclarationsReport`.
    pub(super) fn declarations(&self, decl: &Decl) -> Problem {
        match decl {
            Decl::DeclStart(cur) => self.decl_start(*cur),
            Decl::DeclSpace(space, cur) => self.space(*space, *cur),
            Decl::Port(port, cur) => self.port(port, *cur),
            Decl::DeclType(decl_type, cur) => self.decl_type(decl_type, *cur),
            Decl::DeclDef(name, decl_def, cur) => self.decl_def(name, decl_def, *cur),
            Decl::DeclFreshLineAfterDocComment(cur) => self.at(
                "EXPECTING DECLARATION",
                to_region(*cur),
                reflow("I just saw a doc comment, but then I got stuck here:"),
                reflow(
                    "I was expecting to see the corresponding declaration next, starting on a \
                     fresh line with no indentation.",
                ),
            ),
        }
    }

    /// `toDeclStartReport`.
    pub(super) fn decl_start(&self, cur: Position) -> Problem {
        let region = to_region(cur);
        match self.next(cur) {
            Next::Close(close) => self.at(
                stray_title(close),
                region,
                reflow(&format!(
                    "I was not expecting to see a {} here:",
                    close.name()
                )),
                unmatched_close(close),
            ),
            Next::Keyword(keyword) => self.at(
                "RESERVED WORD",
                self.keyword_region(cur, keyword.as_str()),
                reflow(&format!(
                    "I was not expecting to run into the `{keyword}` keyword here:"
                )),
                reserved_at_top(keyword),
            ),
            Next::Upper(c, cs) => self.at(
                "UNEXPECTED CAPITAL LETTER",
                region,
                reflow(
                    "Declarations always start with a lower-case letter, so I am getting stuck \
                     here:",
                ),
                stack([
                    fill![
                        "Try a name like",
                        green(format!("{}{cs}", c.to_lowercase())),
                        "instead?"
                    ],
                    doc::simple_note("Here are a couple valid declarations for reference:"),
                    declaration_examples(),
                    reflow(
                        "Notice that they always start with a lower-case letter. Capitalization \
                         matters!",
                    ),
                ]),
            ),
            Next::Other(Some(
                c @ ('(' | '{' | '[' | '+' | '-' | '*' | '/' | '^' | '&' | '|' | '"' | '\'' | '!'
                | '@' | '#' | '$' | '%'),
            )) => self.at(
                "UNEXPECTED SYMBOL",
                region,
                reflow(&format!(
                    "I am getting stuck because this line starts with the {c} symbol:"
                )),
                stack([
                    reflow(
                        "When a line has no spaces at the beginning, I expect it to be a \
                         declaration like one of these:",
                    ),
                    declaration_examples(),
                    reflow(
                        "If this is not supposed to be a declaration, try adding some spaces \
                         before it?",
                    ),
                ]),
            ),
            Next::Operator(_) | Next::Lower(..) | Next::Other(_) => self.at(
                "WEIRD DECLARATION",
                region,
                reflow("I am trying to parse a declaration, but I am getting stuck here:"),
                stack([
                    reflow(
                        "When a line has no spaces at the beginning, I expect it to be a \
                         declaration like one of these:",
                    ),
                    declaration_examples(),
                    reflow(
                        "Try to make your declaration look like one of those? Or if this is \
                         not supposed to be a declaration, try adding some spaces before it?",
                    ),
                ]),
            ),
        }
    }

    // PORT

    /// `toPortReport`.
    fn port(&self, port: &Port, start: Position) -> Problem {
        let partway =
            || reflow("I just saw the start of a `port` declaration, but then I got stuck here:");
        let expecting_name = || {
            stack([
                fill![
                    "I was expecting to see a name like",
                    dull_yellow("send"),
                    "or",
                    dull_yellow("receive"),
                    "next. Something that starts with a lower-case letter."
                ],
                port_note(),
            ])
        };
        let expecting_colon = || {
            stack([
                reflow(
                    "I was expecting to see a colon next. And then a type that tells me what \
                     type of values are going to flow through.",
                ),
                port_note(),
            ])
        };
        match port {
            Port::PortSpace(space, cur) => self.space(*space, *cur),
            Port::PortName(cur) => match self.next_keyword(*cur) {
                Some(keyword) => self.reserved(
                    start,
                    *cur,
                    keyword,
                    reflow("I cannot handle ports with names like this:"),
                    reflow(&format!(
                        "You are trying to make a port named `{keyword}` but that is a reserved \
                         word. Try using some other name?"
                    )),
                ),
                None => self.stuck("PORT PROBLEM", start, *cur, partway(), expecting_name()),
            },
            Port::PortColon(cur) => {
                self.stuck("PORT PROBLEM", start, *cur, partway(), expecting_colon())
            }
            Port::PortType(tipe, cur) => self.tipe(&TypeContext::Port, tipe, *cur),
            Port::PortIndentName(cur) => {
                self.stuck("UNFINISHED PORT", start, *cur, partway(), expecting_name())
            }
            Port::PortIndentColon(cur) => {
                self.stuck("UNFINISHED PORT", start, *cur, partway(), expecting_colon())
            }
            Port::PortIndentType(cur) => self.stuck(
                "UNFINISHED PORT",
                start,
                *cur,
                partway(),
                stack([
                    reflow(
                        "I was expecting to see a type next. Here are examples of outgoing and \
                         incoming ports for reference:",
                    ),
                    port_examples(),
                    reflow(
                        "The first line defines a `send` port so you can send strings out to \
                         JavaScript. Maybe you send them on a WebSocket or put them into \
                         IndexedDB. The second line defines a `receive` port so you can receive \
                         strings from JavaScript. Maybe you get receive messages when new \
                         WebSocket messages come in or when an entry in IndexedDB changes for \
                         some external reason.",
                    ),
                ]),
            ),
        }
    }

    // DECL TYPE

    /// `toDeclTypeReport`.
    fn decl_type(&self, decl_type: &DeclType, start: Position) -> Problem {
        match decl_type {
            DeclType::DtSpace(space, cur) => self.space(*space, *cur),
            DeclType::DtName(cur) | DeclType::DtIndentName(cur) => self.stuck(
                "EXPECTING TYPE NAME",
                start,
                *cur,
                reflow("I think I am parsing a type declaration, but I got stuck here:"),
                stack([expecting_type_name(), custom_type_note()]),
            ),
            DeclType::DtAlias(alias, cur) => self.type_alias(alias, *cur),
            DeclType::DtUnion(custom, cur) => self.custom_type(custom, *cur),
        }
    }

    /// `toTypeAliasReport`.
    fn type_alias(&self, alias: &TypeAlias, start: Position) -> Problem {
        let with_note = |title: &'static str, cur: Position, message: Doc| {
            self.stuck(
                title,
                start,
                cur,
                reflow("I am partway through parsing a type alias, but I got stuck here:"),
                stack([message, type_alias_note()]),
            )
        };
        let variable_or_equals =
            || reflow("I was expecting to see a type variable or an equals sign next.");
        match alias {
            TypeAlias::AliasSpace(space, cur) => self.space(*space, *cur),
            TypeAlias::AliasName(cur) => with_note(
                "EXPECTING TYPE ALIAS NAME",
                *cur,
                fill![
                    "I was expecting a name like",
                    dull_yellow("Person"),
                    "or",
                    dull_yellow("Point"),
                    "next. Just make sure it is a name that starts with a capital letter!"
                ],
            ),
            TypeAlias::AliasEquals(cur) => match self.next_keyword(*cur) {
                Some(keyword) => self.reserved(
                    start,
                    *cur,
                    keyword,
                    reflow(
                        "I ran into a reserved word unexpectedly while parsing this type alias:",
                    ),
                    stack([reserved_type_variable(keyword), type_alias_note()]),
                ),
                None => with_note("PROBLEM IN TYPE ALIAS", *cur, variable_or_equals()),
            },
            TypeAlias::AliasBody(tipe, cur) => self.tipe(&TypeContext::TypeAlias, tipe, *cur),
            TypeAlias::AliasIndentEquals(cur) => {
                with_note("UNFINISHED TYPE ALIAS", *cur, variable_or_equals())
            }
            TypeAlias::AliasIndentBody(cur) => with_note(
                "UNFINISHED TYPE ALIAS",
                *cur,
                fill![
                    "I was expecting to see a type next. Something as simple as",
                    dull_yellow("Int"),
                    "or",
                    dull_yellow("Float"),
                    "would work!"
                ],
            ),
        }
    }

    /// `toCustomTypeReport`.
    fn custom_type(&self, custom: &CustomType, start: Position) -> Problem {
        let with_note = |title: &'static str, cur: Position, message: Doc| {
            self.stuck(
                title,
                start,
                cur,
                reflow("I am partway through parsing a custom type, but I got stuck here:"),
                stack([message, custom_type_note()]),
            )
        };
        let variable_or_equals =
            || reflow("I was expecting to see a type variable or an equals sign next.");
        let bar = || reflow("I was expecting to see a vertical bar like | next.");
        match custom {
            CustomType::CtSpace(space, cur) => self.space(*space, *cur),
            CustomType::CtName(cur) => self.stuck(
                "EXPECTING TYPE NAME",
                start,
                *cur,
                reflow("I think I am parsing a type declaration, but I got stuck here:"),
                stack([expecting_type_name(), custom_type_note()]),
            ),
            CustomType::CtEquals(cur) => match self.next_keyword(*cur) {
                Some(keyword) => self.reserved(
                    start,
                    *cur,
                    keyword,
                    reflow(
                        "I ran into a reserved word unexpectedly while parsing this custom type:",
                    ),
                    stack([reserved_type_variable(keyword), custom_type_note()]),
                ),
                None => with_note("PROBLEM IN CUSTOM TYPE", *cur, variable_or_equals()),
            },
            CustomType::CtBar(cur) => with_note("PROBLEM IN CUSTOM TYPE", *cur, bar()),
            CustomType::CtVariant(cur) => with_note(
                "PROBLEM IN CUSTOM TYPE",
                *cur,
                fill![
                    "I was expecting to see a variant name next. Something like",
                    dull_yellow("Success"),
                    "or",
                    dull_yellow("Sandwich").append(Doc::from(".")),
                    "Any name that starts with a capital letter really!"
                ],
            ),
            CustomType::CtVariantArg(tipe, cur) => self.tipe(&TypeContext::CustomType, tipe, *cur),
            CustomType::CtIndentEquals(cur) => {
                with_note("UNFINISHED CUSTOM TYPE", *cur, variable_or_equals())
            }
            CustomType::CtIndentBar(cur) => with_note("UNFINISHED CUSTOM TYPE", *cur, bar()),
            CustomType::CtIndentAfterBar(cur) => with_note(
                "UNFINISHED CUSTOM TYPE",
                *cur,
                reflow(
                    "I just saw a vertical bar, so I was expecting to see another variant \
                     defined next.",
                ),
            ),
            CustomType::CtIndentAfterEquals(cur) => with_note(
                "UNFINISHED CUSTOM TYPE",
                *cur,
                reflow(
                    "I just saw an equals sign, so I was expecting to see the first variant \
                     defined next.",
                ),
            ),
        }
    }

    // DECL DEF

    /// `toDeclDefReport`, but for the patterns and expressions inside.
    fn decl_def(&self, name: &str, decl_def: &DeclDef, start: Position) -> Problem {
        match decl_def {
            DeclDef::DeclDefSpace(space, cur) => self.space(*space, *cur),
            DeclDef::DeclDefEquals(cur) => self.def_equals(name, *cur, start),
            DeclDef::DeclDefType(tipe, cur) => {
                self.tipe(&TypeContext::Annotation(name), tipe, *cur)
            }
            DeclDef::DeclDefArg(pattern, cur) => self.pattern(PContext::Arg, pattern, *cur),
            DeclDef::DeclDefBody(expr, cur) => self.expr(&Context::Def(name, start), expr, *cur),
            DeclDef::DeclDefNameRepeat(cur) => self.stuck(
                "EXPECTING DEFINITION",
                start,
                *cur,
                reflow(&format!(
                    "I just saw the type annotation for `{name}` so I was expecting to see its \
                     definition here:"
                )),
                stack([
                    reflow(
                        "Type annotations always appear directly above the relevant definition, \
                         without anything else in between. (Not even doc comments!)",
                    ),
                    decl_def_note(),
                ]),
            ),
            DeclDef::DeclDefNameMatch(def_name, cur) => {
                self.name_mismatch(name, def_name, *cur, start)
            }
            DeclDef::DeclDefIndentType(cur) => {
                self.unfinished_definition(name, start, *cur, Missing::Type)
            }
            DeclDef::DeclDefIndentEquals(cur) => {
                self.unfinished_definition(name, start, *cur, Missing::Equals)
            }
            DeclDef::DeclDefIndentBody(cur) => {
                self.unfinished_definition(name, start, *cur, Missing::Body)
            }
        }
    }

    /// "UNFINISHED DEFINITION": the definition of `name` stopped before
    /// what it is `missing`. Top-level and `let` definitions alike.
    pub(super) fn unfinished_definition(
        &self,
        name: &str,
        start: Position,
        cur: Position,
        missing: Missing,
    ) -> Problem {
        let (before, expecting) = match missing {
            Missing::Type => (
                reflow(&format!(
                    "I got stuck while parsing the `{name}` type annotation:"
                )),
                "I just saw a colon, so I am expecting to see a type next.",
            ),
            Missing::Equals => (
                stuck_in_definition(name),
                "I was expecting to see an argument or an equals sign next.",
            ),
            Missing::Body => (
                stuck_in_definition(name),
                "I was expecting to see an expression next. What is it equal to?",
            ),
        };
        self.stuck(
            "UNFINISHED DEFINITION",
            start,
            cur,
            before,
            stack([reflow(expecting), decl_def_note()]),
        )
    }

    /// `DeclDefEquals` and `DefEquals`: what can stop a definition
    /// before its equals sign.
    pub(super) fn def_equals(&self, name: &str, cur: Position, start: Position) -> Problem {
        let try_format = || {
            reflow(&format!(
                "Try to use that format with your `{name}` definition!"
            ))
        };
        let not_sure = || {
            reflow(
                "I am not sure what is going wrong exactly, so here is a valid definition (with \
                 an optional type annotation) for reference:",
            )
        };
        match self.next(cur) {
            Next::Keyword(keyword) => self.reserved(
                start,
                cur,
                keyword,
                fill![
                    "The name",
                    Doc::from("`")
                        .append(cyan(keyword.as_str()))
                        .append(Doc::from("`")),
                    "is reserved in Elm, so it cannot be used as an argument here:"
                ],
                stack([
                    reflow("Try renaming it to something else."),
                    if keyword == Keyword::As {
                        doc::note(words![
                            "This keyword is reserved for pattern matches like ((x,y)",
                            cyan("as"),
                            "point) where you want to name a tuple and the values it contains."
                        ])
                    } else {
                        doc::simple_note(&format!(
                            "The `{keyword}` keyword has a special meaning in Elm, so it can \
                             only be used in certain situations."
                        ))
                    },
                ]),
            ),
            Next::Operator(op) if op == "->" => self.within(
                "MISSING COLON?",
                surroundings(start, cur),
                self.wider(cur, 2),
                reflow("I was not expecting to see an arrow here:"),
                stack([
                    fill![
                        "This usually means a",
                        green(":"),
                        "is missing a bit earlier in a type annotation. It could be something else \
                         though, so here is a valid definition for reference:"
                    ],
                    greet_example().indent(4),
                    try_format(),
                ]),
            ),
            Next::Operator(op) => self.within(
                "UNEXPECTED SYMBOL",
                surroundings(start, cur),
                self.keyword_region(cur, &op),
                reflow("I was not expecting to see this symbol here:"),
                stack([not_sure(), greet_example().indent(4), try_format()]),
            ),
            Next::Close(_) | Next::Upper(..) | Next::Lower(..) | Next::Other(_) => self.stuck(
                "PROBLEM IN DEFINITION",
                start,
                cur,
                stuck_in_definition(name),
                stack([
                    not_sure(),
                    greet_example().indent(4),
                    reflow("Try to use that format!"),
                ]),
            ),
        }
    }

    /// `DeclDefNameMatch` and `DefNameMatch`.
    pub(super) fn name_mismatch(
        &self,
        name: &str,
        def_name: &str,
        cur: Position,
        start: Position,
    ) -> Problem {
        self.stuck(
            "NAME MISMATCH",
            start,
            cur,
            reflow(&format!(
                "I just saw a type annotation for `{name}`, but it is followed by a definition \
                 for `{def_name}`:"
            )),
            stack([
                reflow("These names do not match! Is there a typo?"),
                fill![dull_yellow(def_name), "->", green(name)].indent(4),
            ]),
        )
    }
}

/// What `toDeclStartReport` says of a reserved word starting a line.
fn reserved_at_top(keyword: Keyword) -> Doc {
    match keyword {
        Keyword::Import => reflow(
            "It is reserved for declaring imports at the top of your module. If you want another \
             import, try moving it up top with the other imports. If you want to define a value \
             or function, try changing the name to something else!",
        ),
        Keyword::Case => stack([
            reflow("It is reserved for writing `case` expressions. Try using a different name?"),
            doc::simple_note(
                "If you are trying to write a `case` expression, it needs to be part of a \
                 definition. So you could write something like this instead:",
            ),
            vcat([
                reflow("getWidth maybeWidth =").indent(0),
                fill![cyan("case"), "maybeWidth", cyan("of")].indent(2),
                fill![blue("Just"), "width ->"].indent(4),
                fill!["width +", dull_yellow("200")].indent(6),
                Doc::empty(),
                fill![blue("Nothing"), "->"].indent(4),
                dull_yellow("400").indent(6),
            ])
            .indent(4),
            reflow(
                "This defines a `getWidth` function that you can use elsewhere in your program.",
            ),
        ]),
        Keyword::If => stack([
            reflow("It is reserved for writing `if` expressions. Try using a different name?"),
            doc::simple_note(
                "If you are trying to write an `if` expression, it needs to be part of a \
                 definition. So you could write something like this instead:",
            ),
            vcat([
                Doc::from("greet name ="),
                fill![
                    Doc::from(" "),
                    cyan("if"),
                    "name ==",
                    dull_yellow("\"Abraham Lincoln\""),
                    cyan("then"),
                    dull_yellow("\"Greetings Mr. President.\""),
                    cyan("else"),
                    dull_yellow("\"Hey!\"")
                ],
            ])
            .indent(4),
            reflow(
                "This defines a `reviewPowerLevel` function that you can use elsewhere in your \
                 program.",
            ),
        ]),
        Keyword::As
        | Keyword::In
        | Keyword::Of
        | Keyword::Let
        | Keyword::Else
        | Keyword::Port
        | Keyword::Then
        | Keyword::Type
        | Keyword::Where
        | Keyword::Module
        | Keyword::Exposing => {
            reflow("It is a reserved word. Try changing the name to something else?")
        }
    }
}

/// "I got stuck while parsing the `name` definition:"
pub(super) fn stuck_in_definition(name: &str) -> Doc {
    reflow(&format!(
        "I got stuck while parsing the `{name}` definition:"
    ))
}

/// The definition of `greet`, which many reports show.
fn greet_example() -> Doc {
    vcat([
        Doc::from("greet : String -> String"),
        Doc::from("greet name ="),
        Doc::from("  ")
            .append(dull_yellow("\"Hello \""))
            .append(Doc::from(" ++ name ++ "))
            .append(dull_yellow("\"!\"")),
    ])
}

/// `greet` and a custom type, as declarations look.
fn declaration_examples() -> Doc {
    vcat([
        greet_example(),
        Doc::empty(),
        cyan("type").append(Doc::from(" User = Anonymous | LoggedIn String")),
    ])
    .indent(4)
}

fn port_examples() -> Doc {
    vcat([
        fill![cyan("port"), "send :", Doc::from("String -> Cmd msg")],
        fill![
            cyan("port"),
            "receive :",
            Doc::from("(String -> msg) -> Sub msg")
        ],
    ])
    .indent(4)
}

/// `portNote`.
fn port_note() -> Doc {
    stack([
        doc::simple_note("Here are some example `port` declarations for reference:"),
        port_examples(),
        reflow(
            "The first line defines a `send` port so you can send strings out to JavaScript. \
             Maybe you send them on a WebSocket or put them into IndexedDB. The second line \
             defines a `receive` port so you can receive strings from JavaScript. Maybe you get \
             receive messages when new WebSocket messages come in or when the IndexedDB is \
             changed for some external reason.",
        ),
    ])
}

fn expecting_type_name() -> Doc {
    fill![
        "I was expecting a name like",
        dull_yellow("Status"),
        "or",
        dull_yellow("Style"),
        "next. Just make sure it is a name that starts with a capital letter!"
    ]
}

fn reserved_type_variable(keyword: Keyword) -> Doc {
    reflow(&format!(
        "It looks like you are trying use `{keyword}` as a type variable, but it is a reserved \
         word. Try using a different name?"
    ))
}

/// `typeAliasNote`.
fn type_alias_note() -> Doc {
    stack([
        doc::simple_note("Here is an example of a valid `type alias` for reference:"),
        vcat([
            fill![cyan("type"), cyan("alias"), "Person ="].indent(4),
            vcat(["{ name : String", ", age : Int", ", height : Float", "}"].map(Doc::from))
                .indent(6),
        ]),
        reflow(
            "This would let us use `Person` as a shorthand for that record type. Using this \
             shorthand makes type annotations much easier to read, and makes changing code \
             easier if you decide later that there is more to a person than age and height!",
        ),
    ])
}

/// `customTypeNote`.
fn custom_type_note() -> Doc {
    stack([
        doc::simple_note("Here is an example of a valid `type` declaration for reference:"),
        vcat([
            fill![cyan("type"), "Status"].indent(4),
            fill!["= Failure"].indent(6),
            fill!["| Waiting"].indent(6),
            fill!["| Success String"].indent(6),
        ]),
        reflow(
            "This defines a new `Status` type with three variants. This could be useful if we \
             are waiting for an HTTP request. Maybe we start with `Waiting` and then switch to \
             `Failure` or `Success \"message from server\"` depending on how things go. Notice \
             that the Success variant has some associated data, allowing us to store a String \
             if the request goes well!",
        ),
    ])
}

/// `declDefNote`, and `defNote` word for word.
pub(super) fn decl_def_note() -> Doc {
    stack([
        reflow("Here is a valid definition (with a type annotation) for reference:"),
        greet_example().indent(4),
        reflow(
            "The top line (called a \"type annotation\") is optional. You can leave it off if \
             you want. As you get more comfortable with Elm and as your project grows, it \
             becomes more and more valuable to add them though! They work great as \
             compiler-verified documentation, and they often improve error messages!",
        ),
    ])
}
