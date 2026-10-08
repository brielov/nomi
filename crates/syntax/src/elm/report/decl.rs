//! The reports on declarations: `toDeclarationsReport` and the reports
//! it leads to, but for the types, expressions and patterns inside them.

use nomi_base::diagnostic::code::Next;
use nomi_base::diagnostic::doc::{self, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::expr::Context;
use super::pattern::PContext;
use super::types::TypeContext;
use super::{Reporter, blue, cyan, dullyellow, green, surroundings, to_region};
use crate::elm::error::{Cursor, CustomType, Decl, DeclDef, DeclType, Port, TypeAlias};

impl Reporter<'_, '_> {
    /// `toDeclarationsReport`.
    pub(super) fn declarations(&self, decl: &Decl) -> Option<Problem> {
        Some(match decl {
            Decl::DeclStart(cur) => self.decl_start(*cur),
            Decl::DeclSpace(space, cur) => self.space(*space, *cur),
            Decl::Port(port, cur) => return self.port(port, *cur),
            Decl::DeclType(decl_type, cur) => return self.decl_type(decl_type, *cur),
            Decl::DeclDef(name, decl_def, cur) => return self.decl_def(name, decl_def, *cur),
            Decl::DeclFreshLineAfterDocComment(cur) => self.at(
                "EXPECTING DECLARATION",
                to_region(*cur),
                reflow("I just saw a doc comment, but then I got stuck here:"),
                reflow(
                    "I was expecting to see the corresponding declaration next, starting on a \
                     fresh line with no indentation.",
                ),
            ),
        })
    }

    /// `toDeclStartReport`.
    pub(super) fn decl_start(&self, cur: Cursor) -> Problem {
        let region = to_region(cur);
        match self.next(cur) {
            Next::Close(term, bracket) => self.at(
                format!("STRAY {}", term.to_uppercase()),
                region,
                reflow(&format!("I was not expecting to see a {term} here:")),
                reflow(&format!(
                    "This {bracket} does not match up with an earlier open {term}. Try \
                     deleting it?"
                )),
            ),
            Next::Keyword(keyword) => self.at(
                "RESERVED WORD",
                self.keyword_region(cur, keyword),
                reflow(&format!(
                    "I was not expecting to run into the `{keyword}` keyword here:"
                )),
                match keyword {
                    "import" => reflow(
                        "It is reserved for declaring imports at the top of your module. If you \
                         want another import, try moving it up top with the other imports. If \
                         you want to define a value or function, try changing the name to \
                         something else!",
                    ),
                    "case" => stack([
                        reflow(
                            "It is reserved for writing `case` expressions. Try using a \
                             different name?",
                        ),
                        doc::simple_note(
                            "If you are trying to write a `case` expression, it needs to be \
                             part of a definition. So you could write something like this \
                             instead:",
                        ),
                        vcat([
                            fill!["getWidth", "maybeWidth", "="].indent(0),
                            fill![cyan("case"), "maybeWidth", cyan("of")].indent(2),
                            fill![blue("Just"), "width", "->"].indent(4),
                            fill!["width", "+", dullyellow("200")].indent(6),
                            Doc::empty(),
                            fill![blue("Nothing"), "->"].indent(4),
                            fill![dullyellow("400")].indent(6),
                        ])
                        .indent(4),
                        reflow(
                            "This defines a `getWidth` function that you can use elsewhere in \
                             your program.",
                        ),
                    ]),
                    "if" => stack([
                        reflow(
                            "It is reserved for writing `if` expressions. Try using a different \
                             name?",
                        ),
                        doc::simple_note(
                            "If you are trying to write an `if` expression, it needs to be part \
                             of a definition. So you could write something like this instead:",
                        ),
                        vcat([
                            Doc::from("greet name ="),
                            fill![
                                " ",
                                cyan("if"),
                                "name",
                                "==",
                                dullyellow("\"Abraham Lincoln\""),
                                cyan("then"),
                                dullyellow("\"Greetings Mr. President.\""),
                                cyan("else"),
                                dullyellow("\"Hey!\""),
                            ],
                        ])
                        .indent(4),
                        reflow(
                            "This defines a `reviewPowerLevel` function that you can use \
                             elsewhere in your program.",
                        ),
                    ]),
                    _ => reflow("It is a reserved word. Try changing the name to something else?"),
                },
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
                        "Try",
                        "a",
                        "name",
                        "like",
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
            _ => self.at(
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
    fn port(&self, port: &Port, start: Cursor) -> Option<Problem> {
        let within = |cur: Cursor| surroundings(start, cur);
        let stuck =
            || reflow("I just saw the start of a `port` declaration, but then I got stuck here:");
        let expecting_name = || {
            stack([
                fill![
                    "I",
                    "was",
                    "expecting",
                    "to",
                    "see",
                    "a",
                    "name",
                    "like",
                    dullyellow("send"),
                    "or",
                    dullyellow("receive"),
                    "next.",
                    "Something",
                    "that",
                    "starts",
                    "with",
                    "a",
                    "lower-case",
                    "letter."
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
        Some(match port {
            Port::PortSpace(space, cur) => self.space(*space, *cur),
            Port::PortName(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    within(*cur),
                    self.keyword_region(*cur, keyword),
                    reflow("I cannot handle ports with names like this:"),
                    reflow(&format!(
                        "You are trying to make a port named `{keyword}` but that is a reserved \
                         word. Try using some other name?"
                    )),
                ),
                _ => self.within(
                    "PORT PROBLEM",
                    within(*cur),
                    to_region(*cur),
                    stuck(),
                    expecting_name(),
                ),
            },
            Port::PortColon(cur) => self.within(
                "PORT PROBLEM",
                within(*cur),
                to_region(*cur),
                stuck(),
                expecting_colon(),
            ),
            Port::PortType(tipe, cur) => return self.tipe(&TypeContext::Port, tipe, *cur),
            Port::PortIndentName(cur) => self.within(
                "UNFINISHED PORT",
                within(*cur),
                to_region(*cur),
                stuck(),
                expecting_name(),
            ),
            Port::PortIndentColon(cur) => self.within(
                "UNFINISHED PORT",
                within(*cur),
                to_region(*cur),
                stuck(),
                expecting_colon(),
            ),
            Port::PortIndentType(cur) => self.within(
                "UNFINISHED PORT",
                within(*cur),
                to_region(*cur),
                stuck(),
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
        })
    }

    // DECL TYPE

    /// `toDeclTypeReport`.
    fn decl_type(&self, decl_type: &DeclType, start: Cursor) -> Option<Problem> {
        let expecting_name = |cur: Cursor| {
            self.within(
                "EXPECTING TYPE NAME",
                surroundings(start, cur),
                to_region(cur),
                reflow("I think I am parsing a type declaration, but I got stuck here:"),
                stack([expecting_type_name(), custom_type_note()]),
            )
        };
        Some(match decl_type {
            DeclType::DtSpace(space, cur) => self.space(*space, *cur),
            DeclType::DtName(cur) | DeclType::DtIndentName(cur) => expecting_name(*cur),
            DeclType::DtAlias(alias, cur) => return self.type_alias(alias, *cur),
            DeclType::DtUnion(custom, cur) => return self.custom_type(custom, *cur),
        })
    }

    /// `toTypeAliasReport`.
    fn type_alias(&self, alias: &TypeAlias, start: Cursor) -> Option<Problem> {
        let partway = || reflow("I am partway through parsing a type alias, but I got stuck here:");
        let report = |title: &'static str, cur: Cursor, message: Doc| {
            self.within(
                title,
                surroundings(start, cur),
                to_region(cur),
                partway(),
                stack([message, type_alias_note()]),
            )
        };
        let variable_or_equals =
            || reflow("I was expecting to see a type variable or an equals sign next.");
        Some(match alias {
            TypeAlias::AliasSpace(space, cur) => self.space(*space, *cur),
            TypeAlias::AliasName(cur) => report(
                "EXPECTING TYPE ALIAS NAME",
                *cur,
                fill![
                    "I",
                    "was",
                    "expecting",
                    "a",
                    "name",
                    "like",
                    dullyellow("Person"),
                    "or",
                    dullyellow("Point"),
                    "next.",
                    "Just",
                    "make",
                    "sure",
                    "it",
                    "is",
                    "a",
                    "name",
                    "that",
                    "starts",
                    "with",
                    "a",
                    "capital",
                    "letter!"
                ],
            ),
            TypeAlias::AliasEquals(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, keyword),
                    reflow(
                        "I ran into a reserved word unexpectedly while parsing this type alias:",
                    ),
                    stack([reserved_type_variable(keyword), type_alias_note()]),
                ),
                _ => report("PROBLEM IN TYPE ALIAS", *cur, variable_or_equals()),
            },
            TypeAlias::AliasBody(tipe, cur) => {
                return self.tipe(&TypeContext::TypeAlias, tipe, *cur);
            }
            TypeAlias::AliasIndentEquals(cur) => {
                report("UNFINISHED TYPE ALIAS", *cur, variable_or_equals())
            }
            TypeAlias::AliasIndentBody(cur) => report(
                "UNFINISHED TYPE ALIAS",
                *cur,
                fill![
                    "I",
                    "was",
                    "expecting",
                    "to",
                    "see",
                    "a",
                    "type",
                    "next.",
                    "Something",
                    "as",
                    "simple",
                    "as",
                    dullyellow("Int"),
                    "or",
                    dullyellow("Float"),
                    "would",
                    "work!"
                ],
            ),
        })
    }

    /// `toCustomTypeReport`.
    fn custom_type(&self, custom: &CustomType, start: Cursor) -> Option<Problem> {
        let report = |title: &'static str, before: Doc, cur: Cursor, message: Doc| {
            self.within(
                title,
                surroundings(start, cur),
                to_region(cur),
                before,
                stack([message, custom_type_note()]),
            )
        };
        let partway =
            || reflow("I am partway through parsing a custom type, but I got stuck here:");
        let variable_or_equals =
            || reflow("I was expecting to see a type variable or an equals sign next.");
        let bar = || reflow("I was expecting to see a vertical bar like | next.");
        Some(match custom {
            CustomType::CtSpace(space, cur) => self.space(*space, *cur),
            CustomType::CtName(cur) => report(
                "EXPECTING TYPE NAME",
                reflow("I think I am parsing a type declaration, but I got stuck here:"),
                *cur,
                expecting_type_name(),
            ),
            CustomType::CtEquals(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, keyword),
                    reflow(
                        "I ran into a reserved word unexpectedly while parsing this custom type:",
                    ),
                    stack([reserved_type_variable(keyword), custom_type_note()]),
                ),
                _ => report(
                    "PROBLEM IN CUSTOM TYPE",
                    partway(),
                    *cur,
                    variable_or_equals(),
                ),
            },
            CustomType::CtBar(cur) => report("PROBLEM IN CUSTOM TYPE", partway(), *cur, bar()),
            CustomType::CtVariant(cur) => report(
                "PROBLEM IN CUSTOM TYPE",
                partway(),
                *cur,
                fill![
                    "I",
                    "was",
                    "expecting",
                    "to",
                    "see",
                    "a",
                    "variant",
                    "name",
                    "next.",
                    "Something",
                    "like",
                    dullyellow("Success"),
                    "or",
                    dullyellow("Sandwich").append(Doc::from(".")),
                    "Any",
                    "name",
                    "that",
                    "starts",
                    "with",
                    "a",
                    "capital",
                    "letter",
                    "really!"
                ],
            ),
            CustomType::CtVariantArg(tipe, cur) => {
                return self.tipe(&TypeContext::CustomType, tipe, *cur);
            }
            CustomType::CtIndentEquals(cur) => report(
                "UNFINISHED CUSTOM TYPE",
                partway(),
                *cur,
                variable_or_equals(),
            ),
            CustomType::CtIndentBar(cur) => {
                report("UNFINISHED CUSTOM TYPE", partway(), *cur, bar())
            }
            CustomType::CtIndentAfterBar(cur) => report(
                "UNFINISHED CUSTOM TYPE",
                partway(),
                *cur,
                reflow(
                    "I just saw a vertical bar, so I was expecting to see another variant \
                     defined next.",
                ),
            ),
            CustomType::CtIndentAfterEquals(cur) => report(
                "UNFINISHED CUSTOM TYPE",
                partway(),
                *cur,
                reflow(
                    "I just saw an equals sign, so I was expecting to see the first variant \
                     defined next.",
                ),
            ),
        })
    }

    // DECL DEF

    /// `toDeclDefReport`, but for the patterns and expressions inside.
    fn decl_def(&self, name: &str, decl_def: &DeclDef, start: Cursor) -> Option<Problem> {
        let unfinished = |before: Doc, cur: Cursor, message: &str| {
            self.within(
                "UNFINISHED DEFINITION",
                surroundings(start, cur),
                to_region(cur),
                before,
                stack([reflow(message), decl_def_note()]),
            )
        };
        let stuck_definition = || {
            reflow(&format!(
                "I got stuck while parsing the `{name}` definition:"
            ))
        };
        Some(match decl_def {
            DeclDef::DeclDefSpace(space, cur) => self.space(*space, *cur),
            DeclDef::DeclDefEquals(cur) => self.def_equals(name, *cur, start),
            DeclDef::DeclDefType(tipe, cur) => {
                return self.tipe(&TypeContext::Annotation(name), tipe, *cur);
            }
            DeclDef::DeclDefArg(pattern, cur) => {
                return self.pattern(PContext::Arg, pattern, *cur);
            }
            DeclDef::DeclDefBody(expr, cur) => {
                return self.expr(&Context::Def(name, start), expr, *cur);
            }
            DeclDef::DeclDefNameRepeat(cur) => self.within(
                "EXPECTING DEFINITION",
                surroundings(start, *cur),
                to_region(*cur),
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
            DeclDef::DeclDefIndentType(cur) => unfinished(
                reflow(&format!(
                    "I got stuck while parsing the `{name}` type annotation:"
                )),
                *cur,
                "I just saw a colon, so I am expecting to see a type next.",
            ),
            DeclDef::DeclDefIndentEquals(cur) => unfinished(
                stuck_definition(),
                *cur,
                "I was expecting to see an argument or an equals sign next.",
            ),
            DeclDef::DeclDefIndentBody(cur) => unfinished(
                stuck_definition(),
                *cur,
                "I was expecting to see an expression next. What is it equal to?",
            ),
        })
    }

    /// `DeclDefEquals` and `DefEquals`: what can stop a definition
    /// before its equals sign.
    pub(super) fn def_equals(&self, name: &str, cur: Cursor, start: Cursor) -> Problem {
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
            Next::Keyword(keyword) => self.within(
                "RESERVED WORD",
                surroundings(start, cur),
                self.keyword_region(cur, keyword),
                fill![
                    "The",
                    "name",
                    Doc::from("`").append(cyan(keyword)).append(Doc::from("`")),
                    "is",
                    "reserved",
                    "in",
                    "Elm,",
                    "so",
                    "it",
                    "cannot",
                    "be",
                    "used",
                    "as",
                    "an",
                    "argument",
                    "here:"
                ],
                stack([
                    reflow("Try renaming it to something else."),
                    if keyword == "as" {
                        doc::note(
                            [
                                "This", "keyword", "is", "reserved", "for", "pattern", "matches",
                                "like", "((x,y)",
                            ]
                            .into_iter()
                            .map(Doc::from)
                            .chain([cyan("as")])
                            .chain(
                                [
                                    "point)",
                                    "where",
                                    "you",
                                    "want",
                                    "to",
                                    "name",
                                    "a",
                                    "tuple",
                                    "and",
                                    "the",
                                    "values",
                                    "it",
                                    "contains.",
                                ]
                                .map(Doc::from),
                            ),
                        )
                    } else {
                        doc::simple_note(&format!(
                            "The `{keyword}` keyword has a special meaning in Elm, so \
                                     it can only be used in certain situations."
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
                        "This",
                        "usually",
                        "means",
                        "a",
                        green(":"),
                        "is",
                        "missing",
                        "a",
                        "bit",
                        "earlier",
                        "in",
                        "a",
                        "type",
                        "annotation.",
                        "It",
                        "could",
                        "be",
                        "something",
                        "else",
                        "though,",
                        "so",
                        "here",
                        "is",
                        "a",
                        "valid",
                        "definition",
                        "for",
                        "reference:"
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
            _ => self.within(
                "PROBLEM IN DEFINITION",
                surroundings(start, cur),
                to_region(cur),
                reflow(&format!(
                    "I got stuck while parsing the `{name}` definition:"
                )),
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
        cur: Cursor,
        start: Cursor,
    ) -> Problem {
        self.within(
            "NAME MISMATCH",
            surroundings(start, cur),
            to_region(cur),
            reflow(&format!(
                "I just saw a type annotation for `{name}`, but it is followed by a definition \
                 for `{def_name}`:"
            )),
            stack([
                reflow("These names do not match! Is there a typo?"),
                fill![dullyellow(def_name), "->", green(name)].indent(4),
            ]),
        )
    }
}

/// The definition of `greet`, which many reports show.
pub(super) fn greet_example() -> Doc {
    vcat([
        Doc::from("greet : String -> String"),
        Doc::from("greet name ="),
        Doc::from("  ")
            .append(dullyellow("\"Hello \""))
            .append(Doc::from(" ++ name ++ "))
            .append(dullyellow("\"!\"")),
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
        fill![cyan("port"), "send", ":", "String -> Cmd msg"],
        fill![cyan("port"), "receive", ":", "(String -> msg) -> Sub msg"],
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
        "I",
        "was",
        "expecting",
        "a",
        "name",
        "like",
        dullyellow("Status"),
        "or",
        dullyellow("Style"),
        "next.",
        "Just",
        "make",
        "sure",
        "it",
        "is",
        "a",
        "name",
        "that",
        "starts",
        "with",
        "a",
        "capital",
        "letter!"
    ]
}

fn reserved_type_variable(keyword: &str) -> Doc {
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
            fill![cyan("type"), cyan("alias"), "Person", "="].indent(4),
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
            fill!["=", "Failure"].indent(6),
            fill!["|", "Waiting"].indent(6),
            fill!["|", "Success", "String"].indent(6),
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
