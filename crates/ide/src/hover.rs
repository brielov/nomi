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

use std::path::{Path, PathBuf};

use nomi_base::{TextRange, TextSize};
use nomi_compiler::canonical::{self, Home};
use nomi_compiler::canonicalize::foreign::with_defaults;
use nomi_compiler::types::localizer::Localizer;
use nomi_compiler::types::{self, error};
use nomi_engine::Snapshot;
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, parse_module};

use crate::symbols::{child, code_range, token};

/// What to show for the place under the cursor: Markdown, and a link to
/// the declaration of a parameter's type, for the protocol to write as
/// a URI: its name, file and one-based line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    pub markdown: String,
    pub link: Option<(String, PathBuf, u32)>,
}

/// The hover at `offset` in the file at `path`.
pub fn hover(snapshot: &Snapshot, index: &Index, path: &Path, offset: TextSize) -> Option<Hover> {
    let module = index.module(path)?;
    let found = crate::navigation::definition(snapshot, index, path, offset)?;
    let text = snapshot.read(&found.path).ok()?;
    let root = parse_module(&text).syntax();
    let declaring = index.module(&found.path);
    let localizer = Names::of(index, module);
    let name_at = name_token(&root, found.range)?;
    let parent = name_at.parent()?;
    let typed = |range: TextRange| -> Option<String> {
        let declaring = declaring?;
        let (_, types) = declaring.types(index.compat)?;
        let ty = type_of(&declaring.root(), types, range)?;
        Some(render_error(&localizer, ty))
    };
    let markdown = match parent.kind() {
        SyntaxKind::Definition
            if parent
                .parent()
                .is_some_and(|d| d.kind() == SyntaxKind::LetValue) =>
        {
            let declaration = parent.parent()?;
            let name = name_at.text();
            let annotation = match child(&declaration, SyntaxKind::Signature) {
                Some(signature) => text_of(&signature)?,
                None => format!("{name} : {}", composed(&parent, typed)?),
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
                .map(|ty| format!("{name} : {}", render(&localizer, &ty)));
            declaration_hint(&root, &name_at, annotation)?
        }
        _ => {
            // A variable a pattern binds.
            let within = parent.ancestors().find(|a| {
                matches!(
                    a.kind(),
                    SyntaxKind::LetDestructure
                        | SyntaxKind::Definition
                        | SyntaxKind::LambdaExpr
                        | SyntaxKind::CaseBranch
                )
            })?;
            // An alias's type is its whole pattern's.
            let ty = typed(name_at.text_range()).or_else(|| {
                (parent.kind() == SyntaxKind::AliasPattern)
                    .then(|| code_range(&parent).and_then(typed))
                    .flatten()
            });
            let name = name_at.text();
            if within.kind() == SyntaxKind::LetDestructure {
                let annotation = ty.map(|ty| format!("{name} : {ty}")).unwrap_or_default();
                format_hint(&annotation, "Defined in local let scope", None)
            } else {
                // A record update's base has no type of its own to
                // elm-language-server, so nothing to link to.
                let base = (module.root().token_at_offset(offset))
                    .find(|t| t.kind() == SyntaxKind::LowerIdent)
                    .and_then(|t| t.parent())
                    .is_some_and(|p| p.kind() == SyntaxKind::RecordUpdateExpr);
                let link = (!base)
                    .then(|| crate::navigation::type_definition(snapshot, index, path, offset))
                    .flatten()
                    .and_then(|location| link(snapshot, &location));
                let markdown = match ty {
                    Some(ty) if ty != "unknown" => {
                        format_hint(&format!("{name} : {ty}"), "Local parameter", None)
                    }
                    _ => "Local parameter".to_owned(),
                };
                return Some(Hover { markdown, link });
            }
        }
    };
    // A hint with nothing to say, such as a module's without a doc
    // comment, is no hover at all.
    (!markdown.is_empty()).then_some(Hover {
        markdown,
        link: None,
    })
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
            let comment = (crate::lint::first_token(&declaration, SyntaxKind::DocComment))
                .map(|d| d.text().to_owned())
                .unwrap_or_default();
            let signature = child(&declaration, SyntaxKind::Signature);
            let annotation = annotation
                .or_else(|| signature.as_ref().and_then(text_of))
                .unwrap_or_default();
            // Its name and parameters, where anything comes before.
            let preceded =
                signature.is_some() || !comment.is_empty() || declaration.prev_sibling().is_some();
            let end = (parent.children())
                .filter(|c| crate::lint::is_pattern(c.kind()))
                .last()
                .map_or_else(|| name_at.text_range().end(), |p| p.text_range().end());
            let left = TextRange::new(name_at.text_range().start(), end);
            let code = preceded.then(|| text_in(&parent, left));
            format_hint(&annotation, &comment, code.as_deref())
        }
        SyntaxKind::TypeAliasDecl | SyntaxKind::CustomTypeDecl => {
            let comment = doc_of(&parent);
            let range = crate::symbols::declaration_range(&parent)?;
            format_hint("", &comment, Some(&text_in(root, range)))
        }
        SyntaxKind::Variant => {
            let union = parent.parent()?;
            let union_doc = doc_of(&union);
            if union_doc.is_empty() {
                let name =
                    token(&union, SyntaxKind::UpperIdent).unwrap_or_else(|| "unknown".into());
                let comment = format!("A variant on the union type `{name}`");
                format_hint("", &comment, Some(&text_of(&parent)?))
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
                match alias.and_then(|a| token(&a, SyntaxKind::UpperIdent)) {
                    Some(name) => format!("Field on the type alias `{name}`"),
                    None => "Field".to_owned(),
                },
            );
            format_hint(&text_of(&parent)?, &lines.join("\n\n"), None)
        }
        SyntaxKind::PortDecl => {
            let name = token(&parent, SyntaxKind::LowerIdent)?;
            let ty = parent.children().find(|c| crate::lint::is_type(c.kind()))?;
            format_hint(
                &format!("{name} : {}", text_of(&ty)?),
                &doc_of(&parent),
                None,
            )
        }
        _ => return None,
    })
}

/// The name a declaring node's range starts with, past its trivia.
fn name_token(root: &SyntaxNode, range: TextRange) -> Option<SyntaxToken> {
    let mut token = root.token_at_offset(range.start()).right_biased()?;
    while token.kind().is_trivia() {
        token = token.next_token()?;
    }
    // A header's name, not its `module` keyword.
    if token
        .parent()
        .is_some_and(|p| p.kind() == SyntaxKind::ModuleHeader)
    {
        let header = token.parent()?;
        let name = child(&header, SyntaxKind::ModuleName)?;
        return name.first_token();
    }
    // A declaration's name, not its `type` or `port` keyword.
    if let Some(parent) = token.parent()
        && matches!(
            parent.kind(),
            SyntaxKind::TypeAliasDecl | SyntaxKind::CustomTypeDecl | SyntaxKind::PortDecl
        )
    {
        let kind = if parent.kind() == SyntaxKind::PortDecl {
            SyntaxKind::LowerIdent
        } else {
            SyntaxKind::UpperIdent
        };
        return crate::lint::first_token(&parent, kind);
    }
    Some(token)
}

/// The type of the top-level value `name` the module of `root`
/// declares: inferred where the module is the project's, from its
/// interface where it's a dependency's.
/// A `let` value's type as elm-language-server writes it: its
/// parameters' types, then its body's, by `typed`.
fn composed(
    definition: &SyntaxNode,
    typed: impl Fn(TextRange) -> Option<String>,
) -> Option<String> {
    let mut parts = Vec::new();
    for part in definition.children() {
        let range = code_range(&part)?;
        if crate::lint::is_pattern(part.kind()) {
            let ty = typed(range)?;
            parts.push(if ty.contains("->") {
                format!("({ty})")
            } else {
                ty
            });
        } else {
            parts.push(typed(range)?);
        }
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
    let top = (definition.parent()).is_some_and(|d| d.kind() == SyntaxKind::ValueDecl);
    if top {
        let name = token(definition, SyntaxKind::LowerIdent)?;
        let ty = value_type(index, Some(module), &module.root(), &name)?;
        return Some(render(&names, &ty));
    }
    let (_, types) = module.types(index.compat)?;
    composed(definition, |range| {
        type_of(&module.root(), types, range).map(|ty| render_error(&names, ty))
    })
}

fn value_type(
    index: &Index,
    declaring: Option<&Indexed>,
    root: &SyntaxNode,
    name: &str,
) -> Option<canonical::Type> {
    if let Some(declaring) = declaring {
        let canonical = declaring.canonical.as_ref()?;
        if let Ok(inferred) = types::infer(index.compat, canonical)
            && let Some(annotation) = inferred.get(name)
        {
            return Some((*annotation.ty).clone());
        }
        return canonical.annotations.get(name).map(|a| (*a.ty).clone());
    }
    let header = child(root, SyntaxKind::ModuleHeader)?;
    let module = crate::symbols::words(&child(&header, SyntaxKind::ModuleName)?);
    let interface = index.interfaces.get(&module)?;
    interface.values.get(name).map(|a| (*a.ty).clone())
}

/// The link to the declaration at `location`: its name and line.
fn link(snapshot: &Snapshot, location: &crate::Location) -> Option<(String, PathBuf, u32)> {
    let text = snapshot.read(&location.path).ok()?;
    let root = parse_module(&text).syntax();
    let token = name_token(&root, location.range)?;
    let line = text[..usize::from(location.range.start())]
        .matches('\n')
        .count() as u32;
    Some((token.text().to_owned(), location.path.clone(), line + 1))
}

/// The doc comment `node` holds, as a node holds its own.
fn doc_of(node: &SyntaxNode) -> String {
    (crate::lint::first_token(node, SyntaxKind::DocComment))
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

/// `node`'s text without its doc comment and the trivia around it.
fn text_of(node: &SyntaxNode) -> Option<String> {
    let range = code_range(node)?;
    Some(text_in(node, range))
}

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
pub(crate) fn render(localizer: &Names, ty: &canonical::Type) -> String {
    use canonical::Type as T;
    match ty {
        T::Lambda(..) => {
            let mut parts = Vec::new();
            let mut ty = ty;
            while let T::Lambda(arg, result) = ty {
                parts.push(&**arg);
                ty = result;
            }
            parts.push(ty);
            let parts: Vec<String> = (parts.iter())
                .map(|p| match p {
                    T::Lambda(..) => format!("({})", render(localizer, p)),
                    p => render(localizer, p),
                })
                .collect();
            parts.join(" -> ")
        }
        T::Var(name) => name.clone(),
        T::Type { home, name, args } => {
            let args: Vec<(String, bool)> = (args.iter())
                .map(|a| (render(localizer, a), complex(a)))
                .collect();
            named(localizer, home, name, &args)
        }
        T::Alias {
            home, name, args, ..
        } => {
            let args: Vec<(String, bool)> = (args.iter())
                .map(|(_, a)| (render(localizer, a), complex(a)))
                .collect();
            named(localizer, home, name, &args)
        }
        T::Record(fields, extension) => {
            let mut fields: Vec<_> = fields.iter().collect();
            fields.sort_by_key(|(_, f)| f.order);
            if let Some(written) = &localizer.written {
                fields.sort_by_key(|(name, _)| {
                    written.find(&format!("{name} :")).unwrap_or(usize::MAX)
                });
            }
            let fields: Vec<String> = (fields.iter())
                .map(|(name, f)| format!("{name} : {}", render(localizer, &f.ty)))
                .collect();
            let base = extension
                .as_ref()
                .map(|e| format!("{e} | "))
                .unwrap_or_default();
            format!("{{ {base}{} }}", fields.join(", "))
        }
        T::Unit => "()".to_owned(),
        T::Tuple(a, b, c) => {
            let mut parts = vec![render(localizer, a), render(localizer, b)];
            parts.extend(c.as_ref().map(|c| render(localizer, c)));
            format!("( {} )", parts.join(", "))
        }
    }
}

/// Whether a type is written in parentheses as an argument.
fn complex(ty: &canonical::Type) -> bool {
    use canonical::Type as T;
    match ty {
        T::Lambda(..) => true,
        T::Type { args, .. } => !args.is_empty(),
        T::Alias { args, .. } => !args.is_empty(),
        _ => false,
    }
}

/// An inferred type as elm-language-server's renderer writes it.
pub(crate) fn render_error(localizer: &Names, ty: &error::Type) -> String {
    use error::Type as E;
    fn complex(ty: &error::Type) -> bool {
        match ty {
            error::Type::Lambda(..) => true,
            error::Type::Type(_, _, args) => !args.is_empty(),
            error::Type::Alias(_, _, args, _) => !args.is_empty(),
            _ => false,
        }
    }
    match ty {
        E::Lambda(a, b, rest) => {
            let parts: Vec<String> = (std::iter::once(&**a)
                .chain(std::iter::once(&**b))
                .chain(rest))
            .map(|p| match p {
                E::Lambda(..) => format!("({})", render_error(localizer, p)),
                p => render_error(localizer, p),
            })
            .collect();
            parts.join(" -> ")
        }
        E::Infinite | E::Error => "unknown".to_owned(),
        E::FlexVar(name) | E::RigidVar(name) | E::FlexSuper(_, name) | E::RigidSuper(_, name) => {
            name.clone()
        }
        E::Type(home, name, args) => {
            let args: Vec<(String, bool)> = (args.iter())
                .map(|a| (render_error(localizer, a), complex(a)))
                .collect();
            named(localizer, home, name, &args)
        }
        E::Alias(home, name, args, _) => {
            let args: Vec<(String, bool)> = (args.iter())
                .map(|(_, a)| (render_error(localizer, a), complex(a)))
                .collect();
            named(localizer, home, name, &args)
        }
        E::Record(fields, extension) => {
            let fields: Vec<String> = (fields.iter())
                .map(|(name, f)| format!("{name} : {}", render_error(localizer, f)))
                .collect();
            let base = match extension {
                error::Extension::Closed => String::new(),
                error::Extension::FlexOpen(e) | error::Extension::RigidOpen(e) => format!("{e} | "),
            };
            format!("{{ {base}{} }}", fields.join(", "))
        }
        E::Unit => "()".to_owned(),
        E::Tuple(a, b, c) => {
            let mut parts = vec![render_error(localizer, a), render_error(localizer, b)];
            parts.extend(c.as_ref().map(|c| render_error(localizer, c)));
            format!("( {} )", parts.join(", "))
        }
    }
}

/// A named type with its arguments, qualified as the module writes it.
fn named(localizer: &Names, home: &Home, name: &str, args: &[(String, bool)]) -> String {
    if &*home.module == "WebGL" && name == "Shader" {
        return "shader".to_owned();
    }
    let mut written = localizer.qualify(home, name);
    for (arg, complex) in args {
        written.push(' ');
        if *complex {
            written.push_str(&format!("({arg})"));
        } else {
            written.push_str(arg);
        }
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
    exposed: std::rc::Rc<std::collections::HashSet<String>>,
    imports: std::rc::Rc<Vec<(String, Option<String>)>>,
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
        let mut exposed = std::collections::HashSet::new();
        for import in &imports {
            let interface = index.interfaces.get(&import.name.text);
            let constructors = |union: &str| -> Vec<String> {
                (interface.and_then(|i| i.unions.get(union)))
                    .map(|u| match u {
                        Union::Open(u) | Union::Closed(u) | Union::Private(u) => {
                            u.ctors.iter().map(|c| c.name.clone()).collect()
                        }
                    })
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
            exposed: std::rc::Rc::new(exposed),
            imports: std::rc::Rc::new(imports),
            written: None,
        }
    }

    /// These names, writing records' fields in the order `annotation`
    /// writes them.
    /// Every name bare, as elm-language-server writes a type it renders
    /// with no module to write it in, such as a record field's.
    pub(crate) fn bare() -> Names {
        Names {
            exposed: std::rc::Rc::default(),
            imports: std::rc::Rc::default(),
            written: None,
        }
    }

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
    types: &'t types::ExprTypes,
    range: TextRange,
) -> Option<&'t error::Type> {
    let exact = (types.iter())
        .rev()
        .find(|(r, _)| trimmed(root, *r) == Some(range))
        .map(|(_, ty)| ty);
    exact.or_else(|| {
        let text = root.text().to_string();
        (types.iter())
            .filter(|(r, _)| r.start() == range.start() && r.end() < range.end())
            .filter(|(r, _)| {
                text.get(usize::from(r.end())..usize::from(range.end()))
                    .is_some_and(|rest| rest.chars().all(|c| c == ')' || c.is_whitespace()))
            })
            .max_by_key(|(r, _)| r.end())
            .map(|(_, ty)| ty)
    })
}

/// `range` from its first token that isn't trivia.
pub(crate) fn trimmed(root: &SyntaxNode, range: TextRange) -> Option<TextRange> {
    let range = range.intersect(root.text_range())?;
    let token = root.token_at_offset(range.start()).right_biased()?;
    let mut token = token;
    while token.kind().is_trivia() {
        token = token.next_token()?;
    }
    let start = token.text_range().start();
    (start <= range.end()).then(|| TextRange::new(start, range.end()))
}
