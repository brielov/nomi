//! Hover as elm-language-server shows it
//! (`hoverProvider.ts`, `hintHelper.ts`): the definition of the name
//! under the cursor, written as Markdown by what it is.
//!
//! - **Top-level value.** Its type, then its name and parameters, then
//!   its doc comment.
//! - **Type or alias.** Its declaration and doc comment.
//! - **Variant.** Its own text and the type it's of, or the type's doc
//!   comment where the type has one.
//! - **Field.** Its declaration and the alias it's on.
//! - **Port.** Its annotation and doc comment.
//! - **Module.** Its doc comment.
//! - **`let` value.** Its annotation or type.
//! - **Parameter.** Its type, with a link to where that type is declared.
//!
//! Types are written on one line, as elm-language-server's renderer
//! writes them, qualified as the hovering module would write them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nomi_base::{TextRange, TextSize};
use nomi_compiler::canonical::{self, Home};
use nomi_compiler::canonicalize::foreign::with_defaults;
use nomi_compiler::types::localizer::Localizer;
use nomi_compiler::types::{ExprTypes, error};
use nomi_engine::Snapshot;
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, child_token, significant_tokens};

use crate::navigation::Location;
use crate::tree::{child, code_range, code_text, declaration_range, skip_trivia, words};

/// What to show for the place under the cursor: Markdown, and a link to
/// the declaration of a parameter's type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    pub markdown: String,
    pub link: Option<Link>,
}

/// Where a type is declared, for the protocol to write as a URI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The type's name.
    pub name: String,
    pub path: PathBuf,
    /// The declaration's line, counted from 1.
    pub line: u32,
}

/// The hover at `offset` in the file at `path`.
pub fn hover(snapshot: &Snapshot, index: &Index, path: &Path, offset: TextSize) -> Option<Hover> {
    let module = index.module(path)?;
    let (found, root) = crate::navigation::defined(snapshot, index, path, offset)?;
    let declaring = index.module(&found.path);
    let names = Names::of(index, module);
    let name_at = name_token(&root, found.range)?;
    let parent = name_at.parent()?;
    let typed = |range: TextRange| -> Option<String> {
        let declaring = declaring?;
        let (_, types) = declaring.types(index.compat)?;
        let ty = type_of(&declaring.root(), types, range)?;
        Some(render_error(&names, ty))
    };
    let markdown = match parent.kind() {
        SyntaxKind::Definition
            if parent
                .parent()
                .is_some_and(|d| d.kind() == SyntaxKind::LetValue) =>
        {
            let annotation = match child(&parent.parent()?, SyntaxKind::Signature) {
                Some(signature) => code_text(&signature)?,
                None => format!("{} : {}", name_at.text(), composed(&parent, typed)?),
            };
            format_hint(&annotation, "Defined in local let scope", None)
        }
        SyntaxKind::ModuleName
        | SyntaxKind::Definition
        | SyntaxKind::TypeAliasDecl
        | SyntaxKind::CustomTypeDecl
        | SyntaxKind::Variant
        | SyntaxKind::FieldType
        | SyntaxKind::PortDecl => {
            let name = name_at.text();
            let annotation = (parent.kind() == SyntaxKind::Definition)
                .then(|| value_type(index, declaring, &root, name))
                .flatten()
                .map(|ty| format!("{name} : {}", render(&names, ty)));
            declaration_hint(&root, &name_at, annotation)?
        }
        _ => return binding_hover(snapshot, index, module, offset, &name_at, typed),
    };
    // A hint with nothing to say, such as a module's without a doc
    // comment, is no hover at all.
    (!markdown.is_empty()).then_some(Hover {
        markdown,
        link: None,
    })
}

/// The hover of `name_at`, a variable a pattern binds, asked about at
/// `offset` in `module`, typed by `typed`.
fn binding_hover(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    offset: TextSize,
    name_at: &SyntaxToken,
    typed: impl Fn(TextRange) -> Option<String>,
) -> Option<Hover> {
    let parent = name_at.parent()?;
    let within = parent.ancestors().find(|a| {
        matches!(
            a.kind(),
            SyntaxKind::LetDestructure
                | SyntaxKind::Definition
                | SyntaxKind::LambdaExpr
                | SyntaxKind::CaseBranch
        )
    })?;
    let ty = typed(name_at.text_range()).or_else(|| {
        // An alias's type is its whole pattern's.
        (parent.kind() == SyntaxKind::AliasPattern)
            .then(|| code_range(&parent).and_then(&typed))
            .flatten()
    });
    let name = name_at.text();
    if within.kind() == SyntaxKind::LetDestructure {
        let annotation = ty.map(|ty| format!("{name} : {ty}")).unwrap_or_default();
        let markdown = format_hint(&annotation, "Defined in local let scope", None);
        return Some(Hover {
            markdown,
            link: None,
        });
    }
    // A record update's base has no type of its own to
    // elm-language-server, so nothing to link to.
    let base = (module.root().token_at_offset(offset))
        .find(|t| t.kind() == SyntaxKind::LowerIdent)
        .and_then(|t| t.parent())
        .is_some_and(|p| p.kind() == SyntaxKind::RecordUpdateExpr);
    let link = (!base)
        .then(|| crate::navigation::type_defined(snapshot, index, &module.path, offset))
        .flatten()
        .and_then(|(location, root)| link(&location, &root));
    // `createHintFromFunctionParameter` tests the type it writes.
    let markdown = match ty {
        Some(ty) if ty != "unknown" => {
            format_hint(&format!("{name} : {ty}"), "Local parameter", None)
        }
        _ => "Local parameter".to_owned(),
    };
    Some(Hover { markdown, link })
}

/// `HintHelper.createHint` for the declaration whose name is `name_at`:
/// a module, a top-level value, a type, an alias, a variant, a field or a
/// port. A value's annotation is `annotation` where given, as hover
/// gives its type, or else the one it's written with.
pub(crate) fn declaration_hint(
    root: &SyntaxNode,
    name_at: &SyntaxToken,
    annotation: Option<String>,
) -> Option<String> {
    let parent = name_at.parent()?;
    Some(match parent.kind() {
        SyntaxKind::ModuleName => format_hint("", &doc_of(root), None),
        SyntaxKind::Definition => {
            let declaration = parent.parent()?;
            let comment = doc_of(&declaration);
            let signature = child(&declaration, SyntaxKind::Signature);
            let annotation = annotation
                .or_else(|| signature.as_ref().and_then(code_text))
                .unwrap_or_default();
            // Its name and parameters, where anything comes before.
            let preceded =
                signature.is_some() || !comment.is_empty() || declaration.prev_sibling().is_some();
            let end = (parent.children())
                .filter(|c| ast::Pattern::can_cast(c.kind()))
                .last()
                .map_or_else(|| name_at.text_range().end(), |p| p.text_range().end());
            let left = TextRange::new(name_at.text_range().start(), end);
            let code = preceded.then(|| text_in(&parent, left));
            format_hint(&annotation, &comment, code.as_deref())
        }
        SyntaxKind::TypeAliasDecl | SyntaxKind::CustomTypeDecl => {
            let range = declaration_range(&parent)?;
            format_hint("", &doc_of(&parent), Some(&text_in(root, range)))
        }
        SyntaxKind::Variant => {
            let union = parent.parent()?;
            let union_doc = doc_of(&union);
            if union_doc.is_empty() {
                let name = child_token(&union, SyntaxKind::UpperIdent);
                let name = name.as_ref().map_or("unknown", |n| n.text());
                let comment = format!("A variant on the union type `{name}`");
                format_hint("", &comment, Some(&code_text(&parent)?))
            } else {
                format_hint("", &union_doc, None)
            }
        }
        SyntaxKind::FieldType => {
            let alias = (parent.ancestors()).find(|a| a.kind() == SyntaxKind::TypeAliasDecl);
            let mut lines = Vec::new();
            if let Some(comment) = trailing_comment(&parent) {
                lines.push(comment);
            }
            lines.push(
                match alias.and_then(|a| child_token(&a, SyntaxKind::UpperIdent)) {
                    Some(name) => format!("Field on the type alias `{}`", name.text()),
                    None => "Field".to_owned(),
                },
            );
            format_hint(&code_text(&parent)?, &lines.join("\n\n"), None)
        }
        SyntaxKind::PortDecl => {
            let port = ast::PortDecl::cast(parent)?;
            let annotation = format!(
                "{} : {}",
                port.lower_ident_token()?.text(),
                code_text(port.ty()?.syntax())?
            );
            format_hint(&annotation, &doc_of(port.syntax()), None)
        }
        _ => return None,
    })
}

/// The name a declaring node's range starts with, past its trivia.
fn name_token(root: &SyntaxNode, range: TextRange) -> Option<SyntaxToken> {
    let token = skip_trivia(root.token_at_offset(range.start()).right_biased()?)?;
    let parent = token.parent()?;
    match parent.kind() {
        // A header's name, not its `module` keyword, nor a comment on a
        // line of its own above the name, which the name's node holds.
        SyntaxKind::ModuleHeader => {
            significant_tokens(&child(&parent, SyntaxKind::ModuleName)?).next()
        }
        // A declaration's name, not its `type` or `port` keyword.
        SyntaxKind::TypeAliasDecl | SyntaxKind::CustomTypeDecl => {
            child_token(&parent, SyntaxKind::UpperIdent)
        }
        SyntaxKind::PortDecl => child_token(&parent, SyntaxKind::LowerIdent),
        _ => Some(token),
    }
}

/// A `let` value's type as elm-language-server writes it: its
/// parameters' types, then its body's, by `typed`.
pub(crate) fn composed(
    definition: &SyntaxNode,
    typed: impl Fn(TextRange) -> Option<String>,
) -> Option<String> {
    let mut parts = Vec::new();
    for part in definition.children() {
        let ty = typed(code_range(&part)?)?;
        // elm-language-server writes a function's in parentheses.
        let function = ast::Pattern::can_cast(part.kind()) && ty.contains("->");
        parts.push(argument(ty, function));
    }
    Some(parts.join(" -> "))
}

/// The type of the value `definition` defines in `module`, as
/// elm-language-server's `typeToString` writes it: a top-level value's
/// inferred type, or a `let` value's [`composed`] one.
pub(crate) fn definition_type(
    index: &Index,
    module: &Indexed,
    definition: &SyntaxNode,
) -> Option<String> {
    let names = Names::of(index, module);
    let root = module.root();
    if (definition.parent()).is_some_and(|d| d.kind() == SyntaxKind::ValueDecl) {
        let name = child_token(definition, SyntaxKind::LowerIdent)?;
        let ty = value_type(index, Some(module), &root, name.text())?;
        return Some(render(&names, ty));
    }
    let (_, types) = module.types(index.compat)?;
    composed(definition, |range| {
        type_of(&root, types, range).map(|ty| render_error(&names, ty))
    })
}

/// The type of the top-level value `name` the module of `root`
/// declares: inferred where the module is the project's, `declaring`,
/// from its interface where it's a dependency's.
fn value_type<'i>(
    index: &'i Index,
    declaring: Option<&'i Indexed>,
    root: &SyntaxNode,
    name: &str,
) -> Option<&'i canonical::Type> {
    if let Some(declaring) = declaring {
        let canonical = declaring.canonical.as_ref()?;
        if let Some((Ok(inferred), _)) = declaring.types(index.compat)
            && let Some(annotation) = inferred.get(name)
        {
            return Some(&annotation.ty);
        }
        return canonical.annotations.get(name).map(|a| &*a.ty);
    }
    let header = child(root, SyntaxKind::ModuleHeader)?;
    let module = words(&child(&header, SyntaxKind::ModuleName)?);
    let interface = index.interfaces.get(&module)?;
    interface.values.get(name).map(|a| &*a.ty)
}

/// The link to the declaration at `location`, in the file of `root`:
/// its name and line.
fn link(location: &Location, root: &SyntaxNode) -> Option<Link> {
    let token = name_token(root, location.range)?;
    let mut line = 1;
    (root.text().slice(TextRange::up_to(location.range.start())))
        .for_each_chunk(|chunk| line += chunk.matches('\n').count() as u32);
    Some(Link {
        name: token.text().to_owned(),
        path: location.path.clone(),
        line,
    })
}

/// The doc comment `node` holds, as a node holds its own.
fn doc_of(node: &SyntaxNode) -> String {
    child_token(node, SyntaxKind::DocComment)
        .map(|d| d.text().to_owned())
        .unwrap_or_default()
}

/// A line comment right after `node`, on its line.
fn trailing_comment(node: &SyntaxNode) -> Option<String> {
    let mut next = node.last_token()?.next_token();
    while let Some(token) = next {
        match token.kind() {
            SyntaxKind::LineComment => return Some(strip_comment(token.text())),
            SyntaxKind::Whitespace if !token.text().contains('\n') => next = token.next_token(),
            _ => return None,
        }
    }
    None
}

/// The text of `range` in `node`.
fn text_in(node: &SyntaxNode, range: TextRange) -> String {
    let start = range.start() - node.text_range().start();
    node.text().slice(start..start + range.len()).to_string()
}

/// `formatHint`.
fn format_hint(annotation: &str, comment: &str, code: Option<&str>) -> String {
    let mut value = String::new();
    if !annotation.is_empty() {
        value.push_str(&wrap(annotation));
        if code.is_some() || !comment.is_empty() {
            value.push_str("\n\n---\n\n");
        }
    }
    if let Some(code) = code {
        value.push_str(&wrap(code));
    }
    if !comment.is_empty() {
        value.push_str(&strip_comment(comment));
    }
    value
}

fn wrap(code: &str) -> String {
    format!("\n```elm\n{code}\n```\n")
}

/// `stripComment`: a comment's text without its delimiters.
fn strip_comment(comment: &str) -> String {
    let mut text = comment;
    text = text.strip_prefix("{-|").unwrap_or(text);
    text = text.strip_prefix("{-").unwrap_or(text);
    text = text.strip_suffix("-}").unwrap_or(text);
    text = text.strip_prefix("-- ").unwrap_or(text);
    text.trim().to_owned()
}

/// A type as elm-language-server's renderer writes it.
pub(crate) fn render(names: &Names, ty: &canonical::Type) -> String {
    use canonical::Type as T;
    let complex = |ty: &T| match ty {
        T::Lambda(..) => true,
        T::Type { args, .. } => !args.is_empty(),
        T::Alias { args, .. } => !args.is_empty(),
        T::Var(_) | T::Record(..) | T::Unit | T::Tuple(..) => false,
    };
    let argument_of = |ty: &T| argument(render(names, ty), complex(ty));
    match ty {
        T::Lambda(..) => {
            let mut parts = Vec::new();
            let mut ty = ty;
            while let T::Lambda(arg, result) = ty {
                parts.push(&**arg);
                ty = result;
            }
            parts.push(ty);
            let parts = parts
                .iter()
                .map(|p| argument(render(names, p), matches!(p, T::Lambda(..))));
            parts.collect::<Vec<_>>().join(" -> ")
        }
        T::Var(name) => name.clone(),
        T::Type { home, name, args } => named(names, home, name, args.iter().map(argument_of)),
        T::Alias {
            home, name, args, ..
        } => named(names, home, name, args.iter().map(|(_, a)| argument_of(a))),
        T::Record(fields, extension) => {
            let mut fields: Vec<_> = fields.iter().collect();
            fields.sort_by_key(|(_, f)| f.order);
            if let Some(written) = &names.written {
                fields.sort_by_cached_key(|(name, _)| {
                    written.find(&format!("{name} :")).unwrap_or(usize::MAX)
                });
            }
            let fields = fields
                .iter()
                .map(|(name, f)| format!("{name} : {}", render(names, &f.ty)));
            record(extension.as_deref(), fields)
        }
        T::Unit => "()".to_owned(),
        T::Tuple(a, b, c) => tuple([a, b].into_iter().chain(c).map(|t| render(names, t))),
    }
}

/// An inferred type as elm-language-server's renderer writes it.
pub(crate) fn render_error(names: &Names, ty: &error::Type) -> String {
    use error::Type as E;
    let complex = |ty: &E| match ty {
        E::Lambda(..) => true,
        E::Type(_, _, args) => !args.is_empty(),
        E::Alias(_, _, args, _) => !args.is_empty(),
        E::Infinite
        | E::Error
        | E::FlexVar(_)
        | E::FlexSuper(..)
        | E::RigidVar(_)
        | E::RigidSuper(..)
        | E::Record(..)
        | E::Unit
        | E::Tuple(..) => false,
    };
    let argument_of = |ty: &E| argument(render_error(names, ty), complex(ty));
    match ty {
        E::Lambda(a, b, rest) => {
            let parts = [&**a, &**b].into_iter().chain(rest);
            let parts = parts.map(|p| argument(render_error(names, p), matches!(p, E::Lambda(..))));
            parts.collect::<Vec<_>>().join(" -> ")
        }
        E::Infinite | E::Error => "unknown".to_owned(),
        E::FlexVar(name) | E::RigidVar(name) | E::FlexSuper(_, name) | E::RigidSuper(_, name) => {
            name.clone()
        }
        E::Type(home, name, args) => named(names, home, name, args.iter().map(argument_of)),
        E::Alias(home, name, args, _) => {
            named(names, home, name, args.iter().map(|(_, a)| argument_of(a)))
        }
        E::Record(fields, extension) => {
            let base = match extension {
                error::Extension::Closed => None,
                error::Extension::FlexOpen(e) | error::Extension::RigidOpen(e) => Some(e.as_str()),
            };
            let fields = fields
                .iter()
                .map(|(name, f)| format!("{name} : {}", render_error(names, f)));
            record(base, fields)
        }
        E::Unit => "()".to_owned(),
        E::Tuple(a, b, c) => tuple([a, b].into_iter().chain(c).map(|t| render_error(names, t))),
    }
}

/// A type written as an argument: in parentheses where it's `complex`.
fn argument(written: String, complex: bool) -> String {
    if complex {
        format!("({written})")
    } else {
        written
    }
}

/// A record type of `fields`, extending `base` where given.
fn record(base: Option<&str>, fields: impl Iterator<Item = String>) -> String {
    let base = base.map(|e| format!("{e} | ")).unwrap_or_default();
    format!("{{ {base}{} }}", fields.collect::<Vec<_>>().join(", "))
}

fn tuple(parts: impl Iterator<Item = String>) -> String {
    format!("( {} )", parts.collect::<Vec<_>>().join(", "))
}

/// A named type with its arguments, qualified as the module writes it.
fn named(names: &Names, home: &Home, name: &str, args: impl Iterator<Item = String>) -> String {
    if &*home.module == "WebGL" && name == "Shader" {
        return "shader".to_owned();
    }
    let mut written = names.qualify(home, name);
    for arg in args {
        written.push(' ');
        written.push_str(&arg);
    }
    written
}

/// How elm-language-server writes a type's name in the module it's shown
/// in (`getQualifierForName`): bare where an import exposes a name of
/// that spelling, or where the type's module isn't imported; else
/// qualified with the import's alias, or its module.
#[derive(Clone)]
pub(crate) struct Names {
    // Shared, as a completion writes many types with them.
    exposed: Rc<HashSet<String>>,
    imports: Rc<Vec<(String, Option<String>)>>,
    /// An annotation as written, whose records' fields come in its
    /// order: an interface's types have lost it, and elm-language-server
    /// keeps the annotation's.
    written: Option<String>,
}

impl Names {
    pub(crate) fn of(index: &Index, indexed: &Indexed) -> Names {
        use nomi_compiler::hir::{Exposed, Exposing};
        use nomi_compiler::interface::Union;
        let imports = with_defaults(&index.package, &indexed.hir.imports);
        let mut exposed = HashSet::new();
        for import in &imports {
            let interface = index.interfaces.get(&import.name.text);
            let constructors = |union: &str| -> Vec<String> {
                (interface.and_then(|i| i.unions.get(union)))
                    .map(|u| u.whole().ctors.iter().map(|c| c.name.clone()).collect())
                    .unwrap_or_default()
            };
            match &import.exposing {
                None => {}
                Some(Exposing::Open) => {
                    if let Some(interface) = interface {
                        exposed.extend(interface.values.keys().cloned());
                        exposed.extend(interface.aliases.keys().cloned());
                        for (name, union) in &interface.unions {
                            exposed.insert(name.clone());
                            if let Union::Open(_) = union {
                                exposed.extend(constructors(name));
                            }
                        }
                    }
                }
                Some(Exposing::Explicit(list)) => {
                    for item in list {
                        match item {
                            Exposed::Lower(name) | Exposed::Operator(name) => {
                                exposed.insert(name.text.clone());
                            }
                            Exposed::Upper(name, open) => {
                                exposed.insert(name.text.clone());
                                if open.is_some() {
                                    exposed.extend(constructors(&name.text));
                                }
                            }
                        }
                    }
                }
            }
        }
        let imports: Vec<(String, Option<String>)> = (imports.iter())
            .map(|i| {
                (
                    i.name.text.clone(),
                    i.alias.as_ref().map(|a| a.text.clone()),
                )
            })
            .collect();
        Names {
            exposed: Rc::new(exposed),
            imports: Rc::new(imports),
            written: None,
        }
    }

    /// Every name bare, as elm-language-server writes a type it renders
    /// with no module to write it in, such as a record field's.
    pub(crate) fn bare() -> Names {
        Names {
            exposed: Rc::default(),
            imports: Rc::default(),
            written: None,
        }
    }

    /// These names, writing records' fields in the order `annotation`
    /// writes them.
    pub(crate) fn written_as(mut self, annotation: Option<String>) -> Names {
        self.written = annotation;
        self
    }

    fn qualify(&self, home: &Home, name: &str) -> String {
        if self.exposed.contains(name) || (&*home.module == "List" && name == "List") {
            return name.to_owned();
        }
        match self
            .imports
            .iter()
            .find(|(module, _)| *module == *home.module)
        {
            Some((module, alias)) => format!("{}.{name}", alias.as_ref().unwrap_or(module)),
            None => name.to_owned(),
        }
    }
}

/// The localizer of `module`, as its type errors would name types.
pub(crate) fn localizer(index: &Index, indexed: &Indexed) -> Localizer {
    let imports = with_defaults(&index.package, &indexed.hir.imports);
    Localizer::from_module(index.compat, indexed.hir.name(), &imports)
}

/// The type inferred for the expression or pattern at `range` in
/// `root`. elm's region for a call ends with its last argument's, inside
/// any parentheses around it, so where no region is `range`, the longest
/// that starts with it and leaves only closing parentheses out.
pub(crate) fn type_of<'t>(
    root: &SyntaxNode,
    types: &'t ExprTypes,
    range: TextRange,
) -> Option<&'t error::Type> {
    types_at(root, types, range).next_back().or_else(|| {
        (types.iter())
            .filter(|(r, _)| r.start() == range.start() && r.end() < range.end())
            .filter(|(r, _)| closing(root, TextRange::new(r.end(), range.end())))
            .max_by_key(|(r, _)| r.end())
            .map(|(_, ty)| ty)
    })
}

/// The types `types` records for the expression or pattern at `range` in
/// `root`, in order: those whose region, past its trivia, is `range`.
pub(crate) fn types_at<'t>(
    root: &SyntaxNode,
    types: &'t ExprTypes,
    range: TextRange,
) -> impl DoubleEndedIterator<Item = &'t error::Type> {
    (types.iter())
        .filter(move |(r, _)| r.contains_range(range) && trimmed(root, *r) == Some(range))
        .map(|(_, ty)| ty)
}

/// Whether `range` of `root` holds nothing but closing parentheses and
/// space.
fn closing(root: &SyntaxNode, range: TextRange) -> bool {
    root.text_range().contains_range(range)
        && (root.text().slice(range))
            .try_for_each_chunk(|chunk| {
                let closes = chunk.chars().all(|c| c == ')' || c.is_whitespace());
                if closes { Ok(()) } else { Err(()) }
            })
            .is_ok()
}

/// `range` from its first token that isn't trivia.
pub(crate) fn trimmed(root: &SyntaxNode, range: TextRange) -> Option<TextRange> {
    let range = range.intersect(root.text_range())?;
    let token = skip_trivia(root.token_at_offset(range.start()).right_biased()?)?;
    let start = token.text_range().start();
    (start <= range.end()).then(|| TextRange::new(start, range.end()))
}

#[cfg(test)]
mod tests {
    use nomi_base::{TextRange, TextSize};
    use nomi_syntax::parse_module;

    use super::name_token;

    #[test]
    fn a_module_name_is_its_name_past_a_comment_above_it() {
        let text = "module\n    -- the app\n    Main exposing (main)\n";
        let root = parse_module(text).syntax();
        let range = TextRange::new(TextSize::new(0), TextSize::of(text));
        assert_eq!(name_token(&root, range).unwrap().text(), "Main");
    }
}
