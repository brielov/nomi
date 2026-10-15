//! The validation pass. The grammar accepts the union of what elm and
//! elm-format accept, legacy syntax included, so one tree serves every
//! tool; this pass flags what the given tool rejects. What elm allows
//! also depends on the project a module belongs to (`Parse/Module.hs`,
//! `checkEffects`).

use std::str::FromStr;

use nomi_base::diagnostic::code::BINOP_CHARS;
use nomi_base::{Compat, TextRange, TextSize};

use crate::ast::{self, AstNode};
use crate::glsl;
use crate::{
    LexErrorKind, Parse, SyntaxError, SyntaxErrorKind as E, SyntaxKind as K, SyntaxNode,
    SyntaxToken, child_token, int_digits, literal_body, tokens, trimmed_range,
};

/// The tool whose rules a tree must follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    /// A compiler release, reading a module of this kind of project.
    Elm(Source, Compat),
    /// elm-format, which reads every project's modules alike.
    ElmFormat,
}

/// What a module belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// An application.
    Application,
    /// A package outside `elm/` and `elm-explorations/`.
    Package,
    /// A package under `elm/` or `elm-explorations/`, which may declare
    /// infix operators and effect modules.
    Kernel,
}

/// A word that names no [`Source`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownSource;

impl FromStr for Source {
    type Err = UnknownSource;

    /// `application`, `package` or `kernel`, as the mutation suite's
    /// seeds name them.
    fn from_str(text: &str) -> Result<Source, UnknownSource> {
        match text {
            "application" => Ok(Source::Application),
            "package" => Ok(Source::Package),
            "kernel" => Ok(Source::Kernel),
            _ => Err(UnknownSource),
        }
    }
}

/// What `tool` rejects in a tree the grammar accepted, in source order.
/// The parse's own errors are not repeated.
pub fn validate(parse: &Parse, tool: Tool) -> Vec<SyntaxError> {
    let mut errors = Vec::new();
    let root = parse.syntax();
    match tool {
        Tool::Elm(source, compat) => {
            errors.extend(parse.unindented_errors());
            wrapped_columns(parse, compat, &mut errors);
            elm_tokens(&root, &mut errors);
            elm(&root, source, &mut errors);
        }
        Tool::ElmFormat => {
            if let Some(module) = ast::Module::cast(root.clone()) {
                last_import(&module, &mut errors);
            }
            elm_format_tokens(&root, &mut errors);
        }
    }
    code_block(&root, &mut errors);
    errors.sort_by_key(|e| e.range.start());
    errors
}

fn push(errors: &mut Vec<SyntaxError>, kind: E, range: TextRange) {
    errors.push(SyntaxError { kind, range });
}

/// `node` as the view of its kind, which the caller matched.
fn view<N: AstNode>(node: SyntaxNode) -> N {
    N::cast(node).expect("the caller matched the kind")
}

/// Where the `i`th byte of `token` is.
fn offset_in(token: &SyntaxToken, i: usize) -> TextSize {
    token.text_range().start() + TextSize::new(i as u32)
}

/// elm's rules: everything legacy, and what the project allows.
fn elm(root: &SyntaxNode, source: Source, errors: &mut Vec<SyntaxError>) {
    for node in root.descendants() {
        match node.kind() {
            K::DestructureDecl => {
                let range = (view::<ast::DestructureDecl>(node.clone()).pattern())
                    .map_or_else(|| trimmed_range(&node), |p| trimmed_range(p.syntax()));
                push(errors, E::DestructureDeclaration, range);
            }
            K::ValueDecl => {
                let decl = view::<ast::ValueDecl>(node);
                names_match(decl.signature(), decl.definition(), errors);
            }
            K::LetValue => {
                let decl = view::<ast::LetValue>(node);
                names_match(decl.signature(), decl.definition(), errors);
            }
            K::TupleConstructorType | K::TupleConstructorExpr => {
                // The `(,)` itself, not a constructor type's arguments.
                let start = trimmed_range(&node).start();
                let end = child_token(&node, K::RParen)
                    .map_or_else(|| node.text_range().end(), |t| t.text_range().end());
                push(errors, E::TupleConstructor, TextRange::new(start, end));
            }
            K::FieldType => {
                if let Some(eq) = view::<ast::FieldType>(node).eq_token() {
                    push(errors, E::LegacyFieldType, eq.text_range());
                }
            }
            K::Field => {
                if let Some(colon) = view::<ast::Field>(node).colon_token() {
                    push(errors, E::LegacyField, colon.text_range());
                }
            }
            K::LiteralPattern => literal_pattern(&view(node), errors),
            K::WildcardPattern => joined_wildcard(&view(node), errors),
            K::Op if node.parent().is_some_and(|n| n.kind() == K::OperatorChain) => {
                dot_operator(&node, errors);
            }
            K::Op => {
                if let Some(dot) = view::<ast::Op>(node).dot_token() {
                    push(errors, E::DotOperator, dot.text_range());
                }
            }
            K::ModuleHeader => header(&view(node), errors),
            K::OperatorName => push(errors, E::OperatorDefinition, trimmed_range(&node)),
            K::RangeExpr => push(errors, E::ListRange, trimmed_range(&node)),
            K::DocDecl => {
                let end = node.text_range().end();
                push(errors, E::ExpectedDeclaration, TextRange::empty(end));
            }
            K::InfixDecl => joined_precedence(&view(node), errors),
            K::LegacyInfixDecl => push(errors, E::LegacyInfix, trimmed_range(&node)),
            K::LegacyPortDecl => push(errors, E::LegacyPort, trimmed_range(&node)),
            K::CaseExpr => {
                if let Some(then) = view::<ast::CaseExpr>(node).then_token() {
                    push(errors, E::CaseThen, then.text_range());
                }
            }
            K::ExposingList => {
                let list = view::<ast::ExposingList>(node.clone());
                if list.exposing_token().is_none() || list.l_paren_token().is_none() {
                    push(errors, E::LegacyExposing, trimmed_range(&node));
                }
            }
            K::ExposedVariants => {
                match view::<ast::ExposedVariants>(node.clone()).dot_dot_token() {
                    None => push(errors, E::LegacyExposedVariants, trimmed_range(&node)),
                    // elm reads `..--` as an operator.
                    Some(dots) => {
                        if let Some(next) = dots.next_token()
                            && next.kind() == K::LineComment
                        {
                            let at = TextRange::empty(next.text_range().start());
                            push(errors, E::Expected(K::RParen), at);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(module) = ast::Module::cast(root.clone()) {
        effects(&module, source, errors);
        infixes(&module, source, errors);
        declarations(&module, errors);
    }
}

/// Indentation checks that fail where `compat` wraps columns: upstream
/// wants a 1-based column past the enclosing construct's, and past 1
/// (`Parse/Space.hs`, `chompAndCheckIndent`), and wraps both.
fn wrapped_columns(parse: &Parse, compat: Compat, errors: &mut Vec<SyntaxError>) {
    for check in &parse.far {
        if compat.wrap(check.column + 1) <= compat.wrap(check.min_column).max(1) {
            push(errors, E::ExpectedIndent, check.range);
        }
    }
}

/// elm reads the associativity as a keyword, which no digit may follow
/// (`Parse/Module.hs`, `infix_`); elm-format reads its characters.
fn joined_precedence(decl: &ast::InfixDecl, errors: &mut Vec<SyntaxError>) {
    let precedence = decl.int_token();
    let previous = precedence.as_ref().and_then(rowan::SyntaxToken::prev_token);
    if let (Some(precedence), Some(previous)) = (precedence, previous)
        && matches!(previous.kind(), K::LeftKw | K::RightKw | K::NonKw)
    {
        let range = TextRange::new(previous.text_range().start(), precedence.text_range().end());
        push(errors, E::BadPrecedence, range);
    }
}

/// elm reads a definition after each signature, by the signature's name
/// (`Parse/Declaration.hs`, `chompMatchingName`).
fn names_match(
    signature: Option<ast::Signature>,
    definition: Option<ast::Definition>,
    errors: &mut Vec<SyntaxError>,
) {
    let Some(definition) = definition else {
        if let Some(signature) = signature {
            let end = trimmed_range(signature.syntax()).end();
            push(errors, E::Expected(K::Definition), TextRange::empty(end));
        }
        return;
    };
    let Some(expected) = signature.and_then(|s| s.lower_ident_token()) else {
        return;
    };
    if let Some(found) = definition.lower_ident_token()
        && found.text() != expected.text()
    {
        push(errors, E::DefinitionNameMismatch, found.text_range());
    }
}

/// elm's patterns take neither `-` nor floats (`Parse/Pattern.hs`).
fn literal_pattern(pattern: &ast::LiteralPattern, errors: &mut Vec<SyntaxError>) {
    if let Some(minus) = pattern.minus_token() {
        push(errors, E::NegativePattern, minus.text_range());
    } else if let Some(float) = pattern.float_token() {
        push(errors, E::FloatPattern, float.text_range());
    }
}

/// elm reads a wildcard's inner characters, `_` among them, as a name
/// it rejects (`Parse/Pattern.hs`, `wildcard`), so `__` is no pair of
/// wildcards there.
fn joined_wildcard(pattern: &ast::WildcardPattern, errors: &mut Vec<SyntaxError>) {
    let Some(underscore) = pattern.underscore_token() else {
        return;
    };
    if let Some(next) = underscore.next_token()
        && next.kind() == K::Underscore
    {
        push(errors, E::NamedWildcard, next.text_range());
    }
}

/// elm takes a `.` where an argument may start, or right after any
/// operand, for an accessor (`Parse/Expression.hs`, `term`,
/// `accessible`), so no operator of a chain can start with one. Joined to
/// an operand that takes field accesses, the grammar reports it.
fn dot_operator(node: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    let Some(op) = node.first_token() else {
        return;
    };
    let accessed = op.prev_token().is_some_and(|t| match t.kind() {
        K::RParen | K::RBrace | K::UpperIdent => true,
        K::LowerIdent => t.parent().is_none_or(|n| n.kind() != K::AccessorExpr),
        _ => false,
    });
    if op.text().starts_with('.') && !accessed {
        push(errors, E::DotOperator, op.text_range());
    }
}

/// The header's legacy forms, and effect fields where elm wants them.
fn header(header: &ast::ModuleHeader, errors: &mut Vec<SyntaxError>) {
    if header.exposing_list().is_none() {
        let range = (header.module_name()).map_or_else(
            || trimmed_range(header.syntax()),
            |n| trimmed_range(n.syntax()),
        );
        push(errors, E::MissingExposing, range);
    }
    if let Some(stray) = header.where_token() {
        push(errors, E::LegacyWhere, stray.text_range());
    }
    match (header.effect_token(), header.effect_fields()) {
        (Some(_), None) => push(errors, E::ExpectedEffectFields, keywords(header)),
        (None, Some(fields)) => {
            push(
                errors,
                E::UnexpectedEffectFields,
                trimmed_range(fields.syntax()),
            );
        }
        (Some(_), Some(fields)) => effect_fields(&fields, errors),
        (None, None) => {}
    }
}

/// The effect manager an effect field names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Manager {
    Command,
    Subscription,
}

/// One `command`, one `subscription`, or both, in either order
/// (`Parse/Module.hs`, `chompManager`).
fn effect_fields(fields: &ast::EffectFields, errors: &mut Vec<SyntaxError>) {
    let mut seen = Vec::new();
    for field in fields.effect_fields() {
        let Some(name) = field.lower_ident_token() else {
            continue;
        };
        let manager = match name.text() {
            "command" => Some(Manager::Command),
            "subscription" => Some(Manager::Subscription),
            _ => None,
        };
        if manager.is_none_or(|manager| seen.contains(&manager)) {
            push(errors, E::BadEffectField, name.text_range());
        }
        seen.extend(manager);
    }
}

/// From the header's first keyword through `module`, where upstream
/// reports what the module kind allows.
fn keywords(header: &ast::ModuleHeader) -> TextRange {
    let node = header.syntax();
    let start = trimmed_range(node).start();
    let end = header
        .module_token()
        .map_or_else(|| node.text_range().end(), |t| t.text_range().end());
    TextRange::new(start, end)
}

/// Ports and effect managers against the module and project kind
/// (`Parse/Module.hs`, `checkEffects`). A module without a header may
/// declare ports.
fn effects(module: &ast::Module, source: Source, errors: &mut Vec<SyntaxError>) {
    let Some(header) = module.module_header() else {
        return;
    };
    let last_port = (module.syntax().children())
        .filter_map(ast::PortDecl::cast)
        .last();
    let region = keywords(&header);
    if header.effect_token().is_some() {
        if source != Source::Kernel {
            push(errors, E::EffectModuleOutsideKernel, region);
        } else if last_port.is_some() {
            push(errors, E::UnexpectedPort, region);
        }
    } else if header.port_token().is_some() {
        if source != Source::Application {
            push(errors, E::PortModuleInPackage, region);
        } else if last_port.is_none() {
            push(errors, E::NoPorts, region);
        }
    } else if source == Source::Application {
        if last_port.is_some() {
            push(errors, E::UnexpectedPort, region);
        }
    } else if let Some(port) = last_port {
        // Upstream names the last port: it gathers them in reverse.
        let range = port
            .lower_ident_token()
            .map_or_else(|| trimmed_range(port.syntax()), |t| t.text_range());
        push(errors, E::PortInPackage, range);
    }
}

/// Upstream reads infix declarations only in kernel packages, after the
/// imports and before any other declaration (`Parse/Module.hs`,
/// `chompInfixes`).
fn infixes(module: &ast::Module, source: Source, errors: &mut Vec<SyntaxError>) {
    let mut leading = source == Source::Kernel;
    for decl in module.decls() {
        let is_infix = matches!(decl, ast::Decl::InfixDecl(_));
        if is_infix && !leading {
            push(errors, E::UnexpectedInfix, trimmed_range(decl.syntax()));
        }
        leading &= is_infix;
    }
}

/// Upstream reads at least one declaration besides the infixes, and
/// reports a module without one at its end (`Parse/Module.hs`,
/// `chompDecls`).
fn declarations(module: &ast::Module, errors: &mut Vec<SyntaxError>) {
    if module
        .decls()
        .all(|decl| matches!(decl, ast::Decl::InfixDecl(_)))
    {
        let end = module.syntax().text_range().end();
        push(errors, E::NoDeclarations, TextRange::empty(end));
    }
}

/// elm-format reads a fresh line after every import (`imports` in its
/// `Parse/Module.hs`), so the file can't end on the last import's line.
fn last_import(module: &ast::Module, errors: &mut Vec<SyntaxError>) {
    let Some(import) = module.imports().last() else {
        return;
    };
    let mut next = import.syntax().last_token();
    while let Some(token) = next {
        if token.kind() == K::Newline {
            return;
        }
        next = token.next_token().filter(|t| t.kind().is_trivia());
        if next.is_none() && token.next_token().is_some() {
            // Code follows on the same line: the grammar's concern.
            return;
        }
    }
    let end = module.syntax().text_range().end();
    push(errors, E::ImportWithoutNewline, TextRange::empty(end));
}

/// elm's rules on tokens, in one walk over them.
fn elm_tokens(root: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    for token in tokens(root) {
        match token.kind() {
            K::BlockComment | K::DocComment => comment_tabs(&token, errors),
            K::Operator | K::Arrow => operator_chars(&token, errors),
            K::String => {
                string_spanning_lines(&token, errors);
                hex_escapes(&token, errors);
            }
            K::MultilineString | K::Char => hex_escapes(&token, errors),
            K::Glsl => shader(&token, errors),
            _ => {}
        }
    }
}

/// elm takes no tab in a block comment, where elm-format takes any.
fn comment_tabs(token: &SyntaxToken, errors: &mut Vec<SyntaxError>) {
    for (i, _) in token.text().match_indices('\t') {
        let at = TextRange::at(offset_in(token, i), TextSize::new(1));
        push(errors, E::Lexical(LexErrorKind::Tab), at);
    }
}

/// elm-format takes `$`, `#`, `@`, `~` and Unicode symbols in an
/// operator, `→` alone being its `->`, and `=>` for `->`; elm takes none
/// of them.
fn operator_chars(token: &SyntaxToken, errors: &mut Vec<SyntaxError>) {
    if token.kind() == K::Arrow && token.text() == "=>" {
        push(errors, E::FatArrow, token.text_range());
    }
    if let Some((i, c)) = (token.text().char_indices()).find(|&(_, c)| !BINOP_CHARS.contains(c)) {
        let at = TextRange::at(offset_in(token, i), TextSize::of(c));
        push(errors, E::Lexical(LexErrorKind::InvalidToken), at);
    }
}

/// The `\x` escapes of string and character literals, each with its
/// digits.
fn hex_escapes_in(token: &SyntaxToken) -> impl Iterator<Item = (TextRange, &str)> {
    let text = token.text();
    let start = token.text_range().start();
    let mut i = 0;
    std::iter::from_fn(move || {
        while let Some(at) = text[i..].find('\\').map(|at| i + at) {
            // Past the escaped character, which an unterminated literal
            // may lack.
            let escaped = text[at + 1..].chars().next()?;
            i = at + 1 + escaped.len_utf8();
            if escaped == 'x' {
                let digits = text[i..]
                    .find(|c: char| !c.is_ascii_hexdigit())
                    .map_or(&text[i..], |end| &text[i..i + end]);
                let range = TextRange::at(start + TextSize::of(&text[..at]), TextSize::new(2));
                return Some((range, digits));
            }
        }
        None
    })
}

/// elm knows no `\x` escape.
fn hex_escapes(token: &SyntaxToken, errors: &mut Vec<SyntaxError>) {
    for (range, _) in hex_escapes_in(token) {
        push(errors, E::Lexical(LexErrorKind::UnknownEscape), range);
    }
}

/// elm parses each shader block's GLSL (`Parse/Shader.hs`). An
/// unterminated block is the lexer's to report.
fn shader(token: &SyntaxToken, errors: &mut Vec<SyntaxError>) {
    let Some(glsl) = literal_body(token.text()) else {
        return;
    };
    if let Some(offset) = glsl::rejection_offset(glsl) {
        let at =
            token.text_range().start() + TextSize::of("[glsl|") + TextSize::of(&glsl[..offset]);
        push(errors, E::ShaderProblem, TextRange::empty(at));
    }
}

/// elm-format's rules on tokens, in one walk over them.
fn elm_format_tokens(root: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    for token in tokens(root) {
        match token.kind() {
            K::Whitespace => bare_carriage_returns(&token, errors),
            K::Char => hex_characters(&token, errors),
            // elm-format's operators leave out `..`, which elm reads as one.
            K::DotDot if token.parent().is_some_and(|n| n.kind() == K::Op) => {
                push(errors, E::DotDotOperator, token.text_range());
            }
            _ => {}
        }
    }
}

/// elm-format reads a character's `\x` digits into a wrapping 64-bit
/// integer, and its character type holds only what comes out of that
/// non-negative and, masked to 26 bits, below `0x400000`. Strings take
/// any digits.
fn hex_characters(token: &SyntaxToken, errors: &mut Vec<SyntaxError>) {
    for (range, digits) in hex_escapes_in(token) {
        let value = int_digits(digits, 16);
        if value < 0 || value & 0x3FF_FFFF >= 0x40_0000 {
            let end = range.end() + TextSize::of(digits);
            push(
                errors,
                E::Lexical(LexErrorKind::InvalidUnicodeCodePoint),
                TextRange::new(range.start(), end),
            );
        }
    }
}

/// elm ends a `"` string unterminated at a line feed, which
/// elm-format's take in.
fn string_spanning_lines(token: &SyntaxToken, errors: &mut Vec<SyntaxError>) {
    if let Some(i) = token.text().find('\n') {
        let range = TextRange::new(token.text_range().start(), offset_in(token, i));
        push(errors, E::Lexical(LexErrorKind::UnterminatedString), range);
    }
}

/// elm-format's whitespace is spaces, comments and line ends, `\r\n`
/// among them (`Parse/Whitespace.hs`), where elm skips any carriage
/// return.
fn bare_carriage_returns(token: &SyntaxToken, errors: &mut Vec<SyntaxError>) {
    if let Some(i) = token.text().find('\r') {
        let at = TextRange::at(offset_in(token, i), TextSize::new(1));
        push(errors, E::BareCarriageReturn, at);
    }
}

/// elm-format reads a code block's declarations or expressions up to
/// the end of input (`parseDeclarations` and `parseExpressions`), so
/// nothing may follow the last one: no comment, not even a newline. An
/// expression on one line keeps a `--` comment ending it (`withEol`).
fn code_block(root: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    let last = if let Some(fragment) = ast::DeclarationsFragment::cast(root.clone()) {
        fragment.decls().last().map(|d| d.syntax().clone())
    } else if let Some(fragment) = ast::ExpressionsFragment::cast(root.clone()) {
        fragment.exprs().last().map(|e| e.syntax().clone())
    } else {
        return;
    };
    if root.kind() == K::ExpressionsFragment
        && let Some(first) = root.first_token()
        && first.kind() == K::BlockComment
    {
        push(errors, E::LeadingBlockComment, first.text_range());
    }
    let range = last.as_ref().map(trimmed_range);
    let end = range.map_or_else(TextSize::default, TextRange::end);
    let rest = root.text().slice(end..).to_string();
    let eol_comment = rest.trim_start_matches(' ');
    let one_line = |node: &SyntaxNode, range: TextRange| {
        !tokens(node).any(|t| t.kind() == K::Newline && range.contains_range(t.text_range()))
    };
    // Upstream's `withEol` reads a `--` comment ending the line of an
    // expression, or of a custom type's variant, that fits on one line.
    let eol_item = last.and_then(|node| match root.kind() {
        K::ExpressionsFragment => Some(node),
        _ => ast::CustomTypeDecl::cast(node)?
            .variants()
            .last()
            .map(|v| v.syntax().clone()),
    });
    let keeps_eol_comment = eol_comment.starts_with("--")
        && !eol_comment.contains('\n')
        && eol_item.is_some_and(|node| one_line(&node, trimmed_range(&node)));
    if !rest.is_empty() && !keeps_eol_comment {
        push(
            errors,
            E::TrailingText,
            TextRange::new(end, root.text_range().end()),
        );
    }
}

#[cfg(test)]
mod tests;
