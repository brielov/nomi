//! The validation pass. The grammar accepts the union of what elm and
//! elm-format accept, legacy syntax included, so one tree serves every
//! tool; this pass flags what the given tool rejects. What elm allows
//! also depends on the project a module belongs to (`Parse/Module.hs`,
//! `checkEffects`).

use nomi_base::{Compat, TextRange, TextSize};

use crate::ast::{self, AstNode};
use crate::glsl::{self, Rejection};
use crate::lexer::is_elm_operator_char;
use crate::{
    LexErrorKind, Parse, SyntaxError, SyntaxErrorKind as E, SyntaxKind as K, SyntaxNode,
    SyntaxToken,
};

/// The tool whose rules a tree must follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    /// A compiler release, reading a module of this kind of project.
    Elm(Source, Compat),
    ElmFormat,
}

/// What a module belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Application,
    Package,
    /// A package under `elm/` or `elm-explorations/`, which may declare
    /// infix operators and effect modules.
    Kernel,
}

/// What `tool` rejects in a tree the grammar accepted, in source order.
/// The parse's own errors are not repeated.
pub fn validate(parse: &Parse, tool: Tool) -> Vec<SyntaxError> {
    let mut errors = Vec::new();
    let root = parse.syntax();
    match tool {
        Tool::Elm(source, compat) => {
            errors.extend(parse.unindented.iter().map(|&range| SyntaxError {
                kind: E::ExpectedIndent,
                range,
            }));
            if let Some(wrap) = compat.column_wrap() {
                wrapped_columns(parse, wrap, &mut errors);
            }
            strings_spanning_lines(&root, &mut errors);
            comment_tabs(&root, &mut errors);
            operator_chars(&root, &mut errors);
            hex_escapes(&root, &mut errors);
            shaders(&root, &mut errors);
            elm(&root, source, &mut errors);
        }
        Tool::ElmFormat => {
            if let Some(module) = ast::Module::cast(root.clone()) {
                last_import(&module, &mut errors);
            }
            dot_dot_operators(&root, &mut errors);
            hex_characters(&root, &mut errors);
            bare_carriage_returns(&root, &mut errors);
        }
    }
    code_block(&root, &mut errors);
    errors.sort_by_key(|e| e.range.start());
    errors
}

fn push(errors: &mut Vec<SyntaxError>, kind: E, range: TextRange) {
    errors.push(SyntaxError { kind, range });
}

/// elm's rules: everything legacy, and what the project allows.
fn elm(root: &SyntaxNode, source: Source, errors: &mut Vec<SyntaxError>) {
    for node in root.descendants() {
        match node.kind() {
            K::DestructureDecl => {
                let range = ast::DestructureDecl::cast(node.clone())
                    .and_then(|decl| decl.pattern())
                    .map_or_else(|| trimmed(&node), |p| trimmed(p.syntax()));
                push(errors, E::DestructureDeclaration, range);
            }
            K::ValueDecl => {
                let decl = ast::ValueDecl::cast(node).expect("the kind matched");
                names_match(decl.signature(), decl.definition(), errors);
            }
            K::LetValue => {
                let decl = ast::LetValue::cast(node).expect("the kind matched");
                names_match(decl.signature(), decl.definition(), errors);
            }
            K::TupleConstructorType | K::TupleConstructorExpr => {
                // The `(,)` itself, not a constructor type's arguments.
                let start = trimmed(&node).start();
                let end = token(&node, K::RParen)
                    .map_or(node.text_range().end(), |t| t.text_range().end());
                push(errors, E::TupleConstructor, TextRange::new(start, end));
            }
            K::FieldType => {
                if let Some(eq) = token(&node, K::Eq) {
                    push(errors, E::LegacyFieldType, eq.text_range());
                }
            }
            K::Field => {
                if let Some(colon) = token(&node, K::Colon) {
                    push(errors, E::LegacyField, colon.text_range());
                }
            }
            K::LiteralPattern => literal_pattern(&node, errors),
            K::WildcardPattern => joined_wildcard(&node, errors),
            K::Op if node.parent().is_some_and(|n| n.kind() == K::OperatorChain) => {
                dot_operator(&node, errors);
            }
            K::Op => {
                if let Some(dot) = token(&node, K::Dot) {
                    push(errors, E::DotOperator, dot.text_range());
                }
            }
            K::ModuleHeader => header(&node, errors),
            K::OperatorName => push(errors, E::OperatorDefinition, trimmed(&node)),
            K::RangeExpr => push(errors, E::ListRange, trimmed(&node)),
            K::DocDecl => {
                let end = node.text_range().end();
                push(errors, E::ExpectedDeclaration, TextRange::empty(end));
            }
            K::InfixDecl => joined_precedence(&node, errors),
            K::ExposingList
                if token(&node, K::ExposingKw).is_none() || token(&node, K::LParen).is_none() =>
            {
                push(errors, E::LegacyExposing, trimmed(&node));
            }
            K::ExposedVariants => match token(&node, K::DotDot) {
                None => push(errors, E::LegacyExposedVariants, trimmed(&node)),
                // elm reads `..--` as an operator.
                Some(dots) => {
                    if let Some(next) = dots.next_token()
                        && next.kind() == K::LineComment
                    {
                        let at = TextRange::empty(next.text_range().start());
                        push(errors, E::Expected(K::RParen), at);
                    }
                }
            },
            _ => {}
        }
    }
    if let Some(module) = ast::Module::cast(root.clone()) {
        effects(&module, source, errors);
        infixes(&module, source, errors);
        declarations(&module, errors);
    }
}

/// Indentation checks that fail once columns wrap at `wrap`: upstream
/// wants a 1-based column past the enclosing construct's, and past 1
/// (`Parse/Space.hs`, `chompAndCheckIndent`), and wraps both.
fn wrapped_columns(parse: &Parse, wrap: u32, errors: &mut Vec<SyntaxError>) {
    for check in &parse.far {
        let column = (check.column + 1) % wrap;
        if column <= (check.min_column % wrap).max(1) {
            push(errors, E::ExpectedIndent, check.range);
        }
    }
}

/// A node's range without the trivia it leads with or ends on.
fn trimmed(node: &SyntaxNode) -> TextRange {
    let mut tokens = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia());
    let Some(first) = tokens.next() else {
        return node.text_range();
    };
    let last = tokens.last().unwrap_or_else(|| first.clone());
    TextRange::new(first.text_range().start(), last.text_range().end())
}

/// The first direct child token of `kind`.
/// elm reads the associativity as a keyword, which no digit may follow
/// (`Parse/Module.hs`, `infix_`); elm-format reads its characters.
fn joined_precedence(node: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    let precedence = token(node, K::Int);
    let previous = precedence.as_ref().and_then(|t| t.prev_token());
    if let (Some(precedence), Some(previous)) = (precedence, previous)
        && matches!(previous.kind(), K::LeftKw | K::RightKw | K::NonKw)
    {
        let range = TextRange::new(previous.text_range().start(), precedence.text_range().end());
        push(errors, E::BadPrecedence, range);
    }
}

fn token(node: &SyntaxNode, kind: K) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == kind)
}

/// elm reads a definition after each signature, by the signature's name
/// (`Parse/Declaration.hs`, `chompMatchingName`).
fn names_match(
    signature: Option<ast::Signature>,
    definition: Option<ast::Definition>,
    errors: &mut Vec<SyntaxError>,
) {
    let name = |t: Option<SyntaxToken>| t.map(|t| t.text().to_owned());
    let Some(definition) = definition else {
        if let Some(signature) = signature {
            let end = trimmed(signature.syntax()).end();
            push(errors, E::Expected(K::Definition), TextRange::empty(end));
        }
        return;
    };
    let Some(expected) = name(signature.and_then(|s| s.lower_ident_token())) else {
        return;
    };
    if let Some(found) = definition.lower_ident_token()
        && found.text() != expected
    {
        push(errors, E::DefinitionNameMismatch, found.text_range());
    }
}

/// elm's patterns take neither `-` nor floats (`Parse/Pattern.hs`).
fn literal_pattern(node: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    if let Some(minus) = token(node, K::Minus) {
        push(errors, E::NegativePattern, minus.text_range());
    } else if let Some(float) = token(node, K::Float) {
        push(errors, E::FloatPattern, float.text_range());
    }
}

/// elm reads a wildcard's inner characters, `_` among them, as a name
/// it rejects (`Parse/Pattern.hs`, `wildcard`), so `__` is no pair of
/// wildcards there.
fn joined_wildcard(node: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    let Some(underscore) = token(node, K::Underscore) else {
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
fn header(node: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    let header = ast::ModuleHeader::cast(node.clone()).expect("the caller matched the kind");
    if header.exposing_list().is_none() {
        let range = header
            .module_name()
            .map_or_else(|| trimmed(node), |n| trimmed(n.syntax()));
        push(errors, E::MissingExposing, range);
    }
    if let Some(stray) = header.where_token() {
        push(errors, E::LegacyWhere, stray.text_range());
    }
    match (header.effect_token(), header.effect_fields()) {
        (Some(_), None) => push(errors, E::ExpectedEffectFields, keywords(&header)),
        (None, Some(fields)) => {
            push(errors, E::UnexpectedEffectFields, trimmed(fields.syntax()));
        }
        (Some(_), Some(fields)) => effect_fields(&fields, errors),
        (None, None) => {}
    }
}

/// One `command`, one `subscription`, or both, in either order
/// (`Parse/Module.hs`, `chompManager`).
fn effect_fields(fields: &ast::EffectFields, errors: &mut Vec<SyntaxError>) {
    let mut seen = Vec::new();
    for field in fields
        .syntax()
        .children()
        .filter(|n| n.kind() == K::EffectField)
    {
        let Some(name) = token(&field, K::LowerIdent) else {
            continue;
        };
        let known = matches!(name.text(), "command" | "subscription");
        if !known || seen.contains(&name.text().to_owned()) {
            push(errors, E::BadEffectField, name.text_range());
        }
        seen.push(name.text().to_owned());
    }
}

/// From the header's first keyword through `module`, where upstream
/// reports what the module kind allows.
fn keywords(header: &ast::ModuleHeader) -> TextRange {
    let node = header.syntax();
    let start = trimmed(node).start();
    let end = header
        .module_token()
        .map_or(node.text_range().end(), |t| t.text_range().end());
    TextRange::new(start, end)
}

/// Ports and effect managers against the module and project kind
/// (`Parse/Module.hs`, `checkEffects`). A module without a header may
/// declare ports.
fn effects(module: &ast::Module, source: Source, errors: &mut Vec<SyntaxError>) {
    let Some(header) = module.module_header() else {
        return;
    };
    let ports: Vec<ast::PortDecl> = module
        .syntax()
        .children()
        .filter_map(ast::PortDecl::cast)
        .collect();
    let region = keywords(&header);
    if header.effect_token().is_some() {
        if source != Source::Kernel {
            push(errors, E::EffectModuleOutsideKernel, region);
        } else if !ports.is_empty() {
            push(errors, E::UnexpectedPort, region);
        }
    } else if header.port_token().is_some() {
        if source != Source::Application {
            push(errors, E::PortModuleInPackage, region);
        } else if ports.is_empty() {
            push(errors, E::NoPorts, region);
        }
    } else if source == Source::Application {
        if !ports.is_empty() {
            push(errors, E::UnexpectedPort, region);
        }
    } else if let Some(port) = ports.last() {
        // Upstream names the last port: it gathers them in reverse.
        let range = port
            .lower_ident_token()
            .map_or_else(|| trimmed(port.syntax()), |t| t.text_range());
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
            push(errors, E::UnexpectedInfix, trimmed(decl.syntax()));
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

/// elm takes no tab in a block comment, where elm-format takes any.
fn comment_tabs(root: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    let comments = (root.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| matches!(t.kind(), K::BlockComment | K::DocComment));
    for token in comments {
        for (i, _) in token.text().match_indices('\t') {
            let start = token.text_range().start() + TextSize::new(i as u32);
            push(
                errors,
                E::Lexical(LexErrorKind::Tab),
                TextRange::at(start, TextSize::new(1)),
            );
        }
    }
}

/// elm-format takes `$`, `#`, `@`, `~` and Unicode symbols in an
/// operator, `→` alone being its `->`; elm takes none of them.
fn operator_chars(root: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    let operators = (root.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| matches!(t.kind(), K::Operator | K::Arrow));
    for token in operators {
        if let Some((i, c)) = (token.text().char_indices()).find(|&(_, c)| !is_elm_operator_char(c))
        {
            let start = token.text_range().start() + TextSize::new(i as u32);
            push(
                errors,
                E::Lexical(LexErrorKind::InvalidToken),
                TextRange::at(start, TextSize::of(c)),
            );
        }
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

fn literals(root: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> {
    (root.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| matches!(t.kind(), K::String | K::MultilineString | K::Char))
}

/// elm knows no `\x` escape.
fn hex_escapes(root: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    for token in literals(root) {
        for (range, _) in hex_escapes_in(&token) {
            push(errors, E::Lexical(LexErrorKind::UnknownEscape), range);
        }
    }
}

/// elm parses each shader block's GLSL (`Parse/Shader.hs`). An
/// unterminated block is the lexer's to report.
fn shaders(root: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    let shaders = (root.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == K::Glsl);
    for token in shaders {
        let Some(glsl) = (token.text())
            .strip_prefix("[glsl|")
            .and_then(|rest| rest.strip_suffix("|]"))
        else {
            continue;
        };
        if let Err(Rejection::At(offset) | Rejection::TooDeep(offset)) = glsl::check(glsl) {
            let at =
                token.text_range().start() + TextSize::of("[glsl|") + TextSize::of(&glsl[..offset]);
            push(errors, E::ShaderProblem, TextRange::empty(at));
        }
    }
}

/// elm-format reads a character's `\x` digits into a wrapping 64-bit
/// integer, and its character type holds only what comes out of that
/// non-negative and, masked to 26 bits, below `0x400000`. Strings take
/// any digits.
fn hex_characters(root: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    for token in literals(root).filter(|t| t.kind() == K::Char) {
        for (range, digits) in hex_escapes_in(&token) {
            let value = (digits.chars())
                .filter_map(|c| c.to_digit(16))
                .fold(0_i64, |v, d| v.wrapping_mul(16).wrapping_add(i64::from(d)));
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
}

/// elm ends a `"` string unterminated at a line feed, which
/// elm-format's take in.
fn strings_spanning_lines(root: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    let strings = (root.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == K::String);
    for token in strings {
        if let Some(i) = token.text().find('\n') {
            let range = token.text_range();
            let end = range.start() + TextSize::new(i as u32);
            push(
                errors,
                E::Lexical(LexErrorKind::UnterminatedString),
                TextRange::new(range.start(), end),
            );
        }
    }
}

/// elm-format's whitespace is spaces, comments and line ends, `\r\n`
/// among them (`Parse/Whitespace.hs`), where elm skips any carriage
/// return.
fn bare_carriage_returns(root: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    let whitespace = (root.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == K::Whitespace);
    for token in whitespace {
        if let Some(i) = token.text().find('\r') {
            let start = token.text_range().start() + TextSize::new(i as u32);
            push(
                errors,
                E::BareCarriageReturn,
                TextRange::at(start, TextSize::new(1)),
            );
        }
    }
}

/// elm-format's operators leave out `..`, which elm reads as one.
fn dot_dot_operators(root: &SyntaxNode, errors: &mut Vec<SyntaxError>) {
    for node in root.descendants().filter(|n| n.kind() == K::Op) {
        if let Some(dots) = token(&node, K::DotDot) {
            push(errors, E::DotDotOperator, dots.text_range());
        }
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
    let range = last.as_ref().map(trimmed);
    let end = range.map_or(0.into(), TextRange::end);
    let text = root.text().to_string();
    let rest = &text[usize::from(end)..];
    let eol_comment = rest.trim_start_matches(' ');
    let one_line = |node: &SyntaxNode, range: TextRange| {
        !node
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == K::Newline && range.contains_range(t.text_range()))
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
        && eol_item.is_some_and(|node| one_line(&node, trimmed(&node)));
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
