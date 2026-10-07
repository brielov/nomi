//! Token and node kinds, and the rowan language that stores them.

/// Declares [`SyntaxKind`] and its tables from one list, so the tables
/// cannot drift from the enum.
macro_rules! syntax_kinds {
    (
        punctuation { $($punct:ident = $punct_text:literal,)* }
        keywords { $($keyword:ident = $keyword_text:literal,)* }
        contextual { $($contextual:ident = $contextual_text:literal,)* }
        classes { $($(#[$class_doc:meta])* $class:ident = $class_name:literal,)* }
        nodes { $($(#[$node_doc:meta])* $node:ident,)* }
    ) => {
        /// Every token and node kind. Each distinct token has its own kind,
        /// so grammar checks never compare text.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u16)]
        pub enum SyntaxKind {
            $($punct,)*
            $($keyword,)*
            $($contextual,)*
            $($(#[$class_doc])* $class,)*
            $($(#[$node_doc])* $node,)*
        }

        impl SyntaxKind {
            /// Every kind, in `repr` order.
            pub const ALL: &'static [SyntaxKind] = &[
                $(SyntaxKind::$punct,)*
                $(SyntaxKind::$keyword,)*
                $(SyntaxKind::$contextual,)*
                $(SyntaxKind::$class,)*
                $(SyntaxKind::$node,)*
            ];

            const TOKEN_COUNT: usize = [
                $(SyntaxKind::$punct,)*
                $(SyntaxKind::$keyword,)*
                $(SyntaxKind::$contextual,)*
                $(SyntaxKind::$class,)*
            ].len();

            /// The text of a token that is always spelled the same way.
            pub fn text(self) -> Option<&'static str> {
                match self {
                    $(SyntaxKind::$punct => Some($punct_text),)*
                    $(SyntaxKind::$keyword => Some($keyword_text),)*
                    $(SyntaxKind::$contextual => Some($contextual_text),)*
                    _ => None,
                }
            }

            /// The punctuation, keyword or contextual keyword spelled `text`.
            pub fn from_text(text: &str) -> Option<SyntaxKind> {
                match text {
                    $($punct_text => Some(SyntaxKind::$punct),)*
                    $($keyword_text => Some(SyntaxKind::$keyword),)*
                    $($contextual_text => Some(SyntaxKind::$contextual),)*
                    _ => None,
                }
            }

            /// A reserved word: never an identifier.
            pub fn is_keyword(self) -> bool {
                matches!(self, $(SyntaxKind::$keyword)|*)
            }

            /// A word that is a keyword only where the grammar expects one,
            /// such as `alias` after `type`. The lexer produces
            /// [`SyntaxKind::LowerIdent`] and the parser remaps it.
            pub fn is_contextual_keyword(self) -> bool {
                matches!(self, $(SyntaxKind::$contextual)|*)
            }

            /// How `elm.ungram` names this token: its text, or a class name
            /// such as `lower_ident`. `None` for nodes.
            pub fn ungram_name(self) -> Option<&'static str> {
                match self {
                    $(SyntaxKind::$class => Some($class_name),)*
                    _ => self.text(),
                }
            }
        }
    };
}

syntax_kinds! {
    punctuation {
        LParen = "(",
        RParen = ")",
        LBracket = "[",
        RBracket = "]",
        LBrace = "{",
        RBrace = "}",
        Comma = ",",
        Dot = ".",
        DotDot = "..",
        Arrow = "->",
        Eq = "=",
        Colon = ":",
        ColonColon = "::",
        Backslash = "\\",
        Pipe = "|",
        Underscore = "_",
        Minus = "-",
    }
    keywords {
        IfKw = "if",
        ThenKw = "then",
        ElseKw = "else",
        CaseKw = "case",
        OfKw = "of",
        LetKw = "let",
        InKw = "in",
        TypeKw = "type",
        ModuleKw = "module",
        WhereKw = "where",
        ImportKw = "import",
        ExposingKw = "exposing",
        AsKw = "as",
        PortKw = "port",
    }
    contextual {
        AliasKw = "alias",
        EffectKw = "effect",
        InfixKw = "infix",
        LeftKw = "left",
        RightKw = "right",
        NonKw = "non",
    }
    classes {
        // Trivia.
        Whitespace = "whitespace",
        Newline = "newline",
        Tab = "tab",
        LineComment = "line_comment",
        BlockComment = "block_comment",
        // Not trivia: it belongs to the declaration that follows.
        DocComment = "doc_comment",

        LowerIdent = "lower_ident",
        UpperIdent = "upper_ident",
        /// Any operator without a kind of its own, such as `+` or `|>`.
        Operator = "operator",
        Int = "int",
        Float = "float",
        Char = "char",
        String = "string",
        MultilineString = "multiline_string",
        Glsl = "glsl",
        /// Text the lexer could not make sense of.
        ErrorToken = "error_token",
        /// The end of input; never in a tree.
        Eof = "eof",
    }
    nodes {
        Module,
        /// The root of [`crate::parse_type`].
        TypeFragment,
        /// The root of [`crate::parse_pattern`].
        PatternFragment,
        /// The root of [`crate::parse_expr`].
        ExprFragment,
        ModuleHeader,
        ModuleName,
        EffectFields,
        EffectField,
        ExposingList,
        ExposedValue,
        ExposedType,
        ExposedVariants,
        ExposedOperator,
        Import,
        ImportAlias,
        Qualifier,
        Op,

        ValueDecl,
        Signature,
        Definition,
        TypeAliasDecl,
        CustomTypeDecl,
        TypeParam,
        Variant,
        PortDecl,
        InfixDecl,

        NamedType,
        TypeVar,
        FunctionType,
        UnitType,
        ParenType,
        TupleType,
        TupleConstructorType,
        RecordType,
        RecordExtension,
        FieldType,

        VarPattern,
        WildcardPattern,
        LiteralPattern,
        UnitPattern,
        ParenPattern,
        TuplePattern,
        ListPattern,
        RecordPattern,
        FieldPattern,
        ConstructorPattern,
        ConsPattern,
        AliasPattern,

        LiteralExpr,
        NameExpr,
        OperatorFunction,
        TupleConstructorExpr,
        NegateExpr,
        CallExpr,
        OperatorChain,
        UnitExpr,
        ParenExpr,
        TupleExpr,
        ListExpr,
        RecordExpr,
        RecordUpdateExpr,
        Field,
        FieldAccessExpr,
        AccessorExpr,
        IfExpr,
        CaseExpr,
        CaseBranch,
        LetExpr,
        LetValue,
        LetDestructure,
        LambdaExpr,
        GlslExpr,

        /// Where the parser expected something and found nothing.
        Missing,
        /// Tokens the parser skipped while recovering.
        Error,
    }
}

impl SyntaxKind {
    pub fn from_raw(raw: u16) -> Option<SyntaxKind> {
        SyntaxKind::ALL.get(usize::from(raw)).copied()
    }

    pub fn is_token(self) -> bool {
        (self as usize) < SyntaxKind::TOKEN_COUNT
    }

    pub fn is_node(self) -> bool {
        !self.is_token()
    }

    /// Whitespace and comments other than doc comments.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::Whitespace
                | SyntaxKind::Newline
                | SyntaxKind::Tab
                | SyntaxKind::LineComment
                | SyntaxKind::BlockComment
        )
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

/// The rowan language for Elm trees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ElmLanguage {}

impl rowan::Language for ElmLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        SyntaxKind::from_raw(raw.0).expect("trees hold only SyntaxKinds")
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        kind.into()
    }
}

pub type SyntaxNode = rowan::SyntaxNode<ElmLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<ElmLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<ElmLanguage>;
