//! The module line, its doc comment and the imports; upstream's
//! `formatModuleHeader`, `formatModuleLine`, `formatImports` and the
//! listings they share.

use crate::markdown;
use crate::tree::{
    Around, C1, C2, CommentedMap, Comments, CommonDeclaration, Declaration, DetailedListing,
    ForceMultiline, Header, ImportKey, ImportMethod, Listing, Lower, Module, Pattern,
    SourceSetting, SourceTag, Symbol, TopLevelStructure, Upper, Value, c,
};

use super::block::{Block, Line};
use super::structure::{
    Delimiters, equals_pair, group, group_with_footer, interleave, intersperse,
    space_sep_or_indented,
};
use super::{
    ModuleContext, c2, commented, comments, doc_comment, keyword_commented, name, operator, parens,
    please_report, pre_commented, qualified, tail_commented, text,
};

/// The module line, the module's docs and the imports, with a blank line
/// between each two. A module with no line gets the default one as a
/// file, but none in a code block; upstream's `formatModuleHeader`.
pub(super) fn module_header(module: &Module, context: ModuleContext) -> Vec<Block> {
    let default;
    let header = match (&module.header, context) {
        (Some(header), _) => Some(header),
        (None, ModuleContext::File) => {
            default = default_header(module);
            Some(&default)
        }
        (None, ModuleContext::CodeBlock) => None,
    };
    let mut parts = Vec::new();
    if let Some(header) = header {
        parts.push(vec![module_line(module, header)]);
    }
    if let Some(docs) = &module.docs {
        parts.push(vec![doc_comment(docs)]);
    }
    let imports = imports(module);
    if !imports.is_empty() {
        parts.push(imports);
    }
    join_with_blank_lines(parts)
}

/// The header a module without one gets, `module Main`, but for a
/// deviation (`DECISIONS.md`): where the module declares ports, a port
/// module's, as Elm reads the module, where upstream's makes Elm reject
/// it.
fn default_header(module: &Module) -> Header {
    let ports = module.body.iter().any(|item| {
        matches!(
            item,
            TopLevelStructure::Entry(Declaration::PortAnnotation { .. })
        )
    });
    Header {
        source_tag: if ports {
            SourceTag::Port(Vec::new())
        } else {
            SourceTag::Normal
        },
        name: C2::bare(vec![Upper("Main".to_owned())]),
        settings: None,
        exports: None,
    }
}

/// The module line of `header`, with the values it exposes sorted into
/// the groups of the docs' `@docs` lines.
fn module_line(module: &Module, header: &Header) -> Block {
    let documented = documented(module);
    let to_expose = match &header.exports {
        Some(exports) => listing_values(&exports.value),
        None if documented.iter().all(Vec::is_empty) => defined_values(module),
        None => defined_values(module)
            .into_iter()
            .filter(|v| {
                documented
                    .iter()
                    .flatten()
                    .any(|n| n == value_name(&v.value))
            })
            .collect(),
    };
    let force = match header.exports.as_ref().map(|e| &e.value) {
        Some(Listing::Explicit(_, force)) => *force,
        Some(Listing::Open(_) | Listing::Closed) | None => ForceMultiline(false),
    };
    module_line_block(header, &sort_values(force, to_expose, &documented))
}

/// The names of each `@docs` line of the module's docs, at their top
/// level and before `formatDocComment` cleans them; upstream's
/// `documentedVars`.
fn documented(module: &Module) -> Vec<Vec<String>> {
    let Some(docs) = &module.docs else {
        return Vec::new();
    };
    docs.blocks
        .iter()
        .flat_map(|block| match block {
            markdown::Block::ElmDocs(groups) => groups.as_slice(),
            markdown::Block::Para(_)
            | markdown::Block::Header(..)
            | markdown::Block::Blockquote(_)
            | markdown::Block::List { .. }
            | markdown::Block::Code { .. }
            | markdown::Block::Html(_)
            | markdown::Block::HRule
            | markdown::Block::References(_) => &[],
        })
        .map(|group| group.iter().map(|n| ref_name(n).to_owned()).collect())
        .collect()
}

fn join_with_blank_lines(parts: Vec<Vec<Block>>) -> Vec<Block> {
    interleave(parts, || vec![Block::blank_line()])
        .flatten()
        .collect()
}

/// The name a doc comment's `@docs` term refers to; upstream's
/// `refName . textToRef`, which unwraps operators of one or two
/// characters.
fn ref_name(term: &str) -> &str {
    let mut chars = term.chars();
    match chars.next() {
        Some(c) if c.is_uppercase() || c.is_lowercase() => term,
        Some('(') if term.ends_with(')') && matches!(term.chars().count(), 3 | 4) => {
            &term[1..term.len() - 1]
        }
        _ => term,
    }
}

fn value_name(value: &Value) -> &str {
    match value {
        Value::Var(name) => &name.0,
        Value::Op(symbol) => &symbol.0,
        Value::Union(name, _) => &name.value.0,
    }
}

/// What a module without `exposing` exposes: everything it defines at the
/// top level; upstream's `definedVars`.
fn defined_values(module: &Module) -> Vec<C2<Value>> {
    let var = |name: &Lower| Value::Var(name.clone());
    let union = |name: &Upper, listing| Value::Union(c(Vec::new(), name.clone()), listing);
    let mut values = Vec::new();
    for item in &module.body {
        let TopLevelStructure::Entry(decl) = item else {
            continue;
        };
        match decl {
            Declaration::PortAnnotation { name, .. } => values.push(var(&name.value)),
            Declaration::Common(CommonDeclaration::Definition { name, .. }) => match name {
                Pattern::Var(name) => values.push(var(name)),
                Pattern::Record(fields) => values.extend(fields.iter().map(|f| var(&f.value))),
                Pattern::Anything
                | Pattern::Unit(_)
                | Pattern::Literal(_)
                | Pattern::Op(_)
                | Pattern::Data(..)
                | Pattern::Parens(_)
                | Pattern::Tuple(_)
                | Pattern::EmptyList(_)
                | Pattern::List(_)
                | Pattern::Cons { .. }
                | Pattern::EmptyRecord(_)
                | Pattern::Alias(..) => {}
            },
            Declaration::Datatype { name_with_args, .. } => values.push(union(
                &name_with_args.value.name,
                Listing::Open(Around::default()),
            )),
            Declaration::TypeAlias { name_with_args, .. } => {
                values.push(union(&name_with_args.value.name, Listing::Closed));
            }
            Declaration::Common(CommonDeclaration::TypeAnnotation { .. })
            | Declaration::PortDefinition { .. }
            | Declaration::Fixity { .. }
            | Declaration::LegacyFixity { .. } => {}
        }
    }
    values.into_iter().map(C2::bare).collect()
}

/// An `exposing` list's items, with their comments; upstream's
/// `detailedListingToSet`.
fn listing_values(listing: &Listing<DetailedListing>) -> Vec<C2<Value>> {
    let Listing::Explicit(listing, _) = listing else {
        return Vec::new();
    };
    let values = listing
        .values
        .iter()
        .map(|(name, v)| c(v.comments.clone(), Value::Var(name.clone())));
    let operators = listing
        .operators
        .iter()
        .map(|(symbol, v)| c(v.comments.clone(), Value::Op(symbol.clone())));
    let types = listing.types.iter().map(|(name, v)| {
        let ctors = &v.value;
        c(
            v.comments.clone(),
            Value::Union(c(ctors.comments.clone(), name.clone()), ctors.value.clone()),
        )
    });
    values.chain(operators).chain(types).collect()
}

/// The exposed items as the module line lists them.
struct SortedExports {
    /// One group per `@docs` line, in its order, then the rest.
    groups: Vec<Vec<C2<Value>>>,
    /// The comments of the items the docs moved, which lose their place.
    moved_comments: Comments,
}

/// The exposed items in groups; upstream's `sortVars`.
fn sort_values(
    force: ForceMultiline,
    exposing: Vec<C2<Value>>,
    documented: &[Vec<String>],
) -> SortedExports {
    let kind = |v: &Value| match v {
        Value::Op(_) => 1,
        Value::Union(..) => 2,
        Value::Var(_) => 3,
    };
    // Upstream holds them in a set before it sorts them, so a type
    // listed twice orders by its derived `Ord` on listings:
    // `Y(A)`, then `Y(..)`, then `Y`.
    let listing = |v: &Value| match v {
        Value::Union(_, Listing::Explicit(..)) => 0,
        Value::Union(_, Listing::Open(..)) => 1,
        Value::Union(_, Listing::Closed) | Value::Var(_) | Value::Op(_) => 2,
    };
    let mut exposing = exposing;
    exposing.sort_by(|a, b| {
        (kind(&a.value), value_name(&a.value), listing(&a.value)).cmp(&(
            kind(&b.value),
            value_name(&b.value),
            listing(&b.value),
        ))
    });
    exposing.dedup();

    let allowed = |name: &str| {
        exposing
            .iter()
            .rev()
            .find(|v| value_name(&v.value) == name)
            .map(|v| C2::bare(v.value.clone()))
    };
    let mut seen: Vec<C2<Value>> = Vec::new();
    let mut groups = Vec::new();
    for group in documented {
        let mut kept = Vec::new();
        for value in group.iter().filter_map(|n| allowed(n)) {
            if !seen.contains(&value) {
                seen.push(value.clone());
                kept.push(value);
            }
        }
        if !kept.is_empty() {
            groups.push(kept);
        }
    }
    let is_in_docs = |v: &C2<Value>| {
        seen.iter()
            .any(|s| value_name(&s.value) == value_name(&v.value))
    };
    let mut moved_comments = Vec::new();
    let mut remaining = Vec::new();
    for value in exposing {
        if is_in_docs(&value) {
            let Around { before, after } = value.comments;
            moved_comments.extend(before);
            moved_comments.extend(after);
        } else {
            remaining.push(value);
        }
    }
    if groups.is_empty() && force.0 {
        return SortedExports {
            groups: remaining.into_iter().map(|v| vec![v]).collect(),
            moved_comments,
        };
    }
    if !remaining.is_empty() {
        groups.push(remaining);
    }
    SortedExports {
        groups,
        moved_comments,
    }
}

/// `module Name exposing (…)`; upstream's `formatModuleLine`.
fn module_line_block(header: &Header, exposed: &SortedExports) -> Block {
    let tag = match &header.source_tag {
        SourceTag::Normal => text("module"),
        SourceTag::Port(comments) => {
            space_sep_or_indented(tail_commented(text("port"), comments), vec![text("module")])
        }
        SourceTag::Effect(comments) => space_sep_or_indented(
            tail_commented(text("effect"), comments),
            vec![text("module")],
        ),
    };
    let exports = exports(&exposed.groups, comments(&exposed.moved_comments));
    let where_clause = header.settings.as_ref().map(|settings| {
        let Around { before, after } = &settings.comments;
        keyword_commented("where", before, after, source_settings(&settings.value))
    });
    let name_clause = match (tag, c2(&header.name, |names| Block::line(qualified(names)))) {
        (Block::SingleLine(tag), Block::SingleLine(name)) => {
            Block::line(Line::row([tag, Line::space(), name]))
        }
        (tag, name) => Block::stack1([tag, name.indent()]),
    };
    let exposing = match &header.exports {
        Some(e) => commented(&e.comments.before, text("exposing"), &e.comments.after),
        None => commented(&[], text("exposing"), &[]),
    };
    space_sep_or_indented(
        space_sep_or_indented(
            name_clause,
            where_clause.into_iter().chain([exposing]).collect(),
        ),
        vec![exports],
    )
}

/// The exposing list of `groups`, with `footer` after them; one group
/// shares a line, and more stack, each on its own.
fn exports(groups: &[Vec<C2<Value>>], footer: Option<Block>) -> Block {
    let footer: Vec<Block> = footer.into_iter().collect();
    match groups {
        [] => text("(..)"),
        [group] => group_with_footer(
            Delimiters::PARENS,
            footer,
            ForceMultiline(false),
            group.iter().map(commented_value).collect(),
        ),
        groups => group_with_footer(
            Delimiters::PARENS,
            footer,
            ForceMultiline(true),
            groups
                .iter()
                .map(|values| {
                    let before: Comments = values
                        .iter()
                        .flat_map(|v| v.comments.before.clone())
                        .collect();
                    let after: Comments = values
                        .iter()
                        .flat_map(|v| v.comments.after.clone())
                        .collect();
                    let inner = group(
                        Delimiters::NONE,
                        ForceMultiline(false),
                        values.iter().map(|v| value(&v.value)).collect(),
                    );
                    commented(&before, inner, &after)
                })
                .collect(),
        ),
    }
}

fn commented_value(v: &C2<Value>) -> Block {
    commented(&v.comments.before, value(&v.value), &v.comments.after)
}

/// `{ command = MyCmd, … }`; upstream's `formatSettings` over
/// `formatRecordPair`.
fn source_settings(settings: &[SourceSetting]) -> Block {
    let pairs = settings
        .iter()
        .map(|SourceSetting { key, value }| {
            let pair = equals_pair(
                "=",
                ForceMultiline(false),
                commented(&[], Block::line(name(&key.value.0)), &key.comments.after),
                commented(
                    &value.comments.before,
                    Block::line(name(&value.value.0)),
                    &value.comments.after,
                ),
            );
            pre_commented(&key.comments.before, pair)
        })
        .collect();
    group(Delimiters::PADDED_BRACES, ForceMultiline(false), pairs)
}

/// The comments before the imports, then each import; upstream's
/// `formatImports`.
fn imports(module: &Module) -> Vec<Block> {
    let mut parts = Vec::new();
    if let Some(comments) = comments(&module.imports.comments) {
        parts.push(vec![comments]);
    }
    let imports: Vec<Block> = module
        .imports
        .value
        .iter()
        .map(|(ImportKey { module, .. }, method)| import(module, method))
        .collect();
    if !imports.is_empty() {
        parts.push(imports);
    }
    join_with_blank_lines(parts)
}

/// Upstream's `formatImport`.
fn import(module: &[Upper], method: &C1<ImportMethod>) -> Block {
    let ImportMethod { alias, exposed } = &method.value;
    let module_name = pre_commented(&method.comments, Block::line(qualified(module)));
    // An alias that repeats the module's one-part name goes.
    let alias = alias
        .as_ref()
        .filter(|alias| !matches!(module, [only] if *only == alias.value));
    let alias = alias.and_then(|alias| {
        import_clause(
            Some(Block::line(name(&alias.value.0))),
            "as",
            &alias.comments,
        )
    });
    let exposing = import_clause(
        listing(&exposed.value, detailed_listing),
        "exposing",
        &exposed.comments,
    );
    let keyword = || text("import");
    let row = |parts: Vec<Line>| {
        Block::line(intersperse(
            std::iter::once(Line::text("import")).chain(parts),
            Line::space,
        ))
    };
    match (module_name, alias, exposing) {
        (Block::SingleLine(name), alias, exposing)
            if alias.as_ref().is_none_or(Block::is_line)
                && exposing.as_ref().is_none_or(Block::is_line) =>
        {
            let clauses = alias.into_iter().chain(exposing).map(|b| {
                b.into_line()
                    .unwrap_or_else(|_| unreachable!("checked to be lines"))
            });
            row(std::iter::once(name).chain(clauses).collect())
        }
        (Block::SingleLine(name), Some(Block::SingleLine(alias)), Some(exposing)) => {
            Block::stack1([row(vec![name, alias]), exposing.indent()])
        }
        (Block::SingleLine(name), alias, Some(exposing)) => Block::stack1(
            std::iter::once(row(vec![name]))
                .chain(alias.map(Block::indent))
                .chain([exposing.indent()]),
        ),
        (name, alias, exposing) => Block::stack1(
            [keyword(), name.indent()]
                .into_iter()
                .chain(alias.map(|a| a.indent().indent()))
                .chain(exposing.map(|e| e.indent().indent())),
        ),
    }
}

/// `keyword clause`, or `None` without a clause; upstream's
/// `formatImportClause`.
fn import_clause(clause: Option<Block>, keyword: &str, comments: &Around) -> Option<Block> {
    let Some(clause) = clause else {
        if comments.is_empty() {
            return None;
        }
        return Some(please_report(
            "UNEXPECTED IMPORT",
            "import clause comments with no clause",
        ));
    };
    match (
        pre_commented(&comments.before, text(keyword)),
        pre_commented(&comments.after, clause),
    ) {
        (Block::SingleLine(keyword), Block::SingleLine(clause)) => {
            Some(Block::line(Line::row([keyword, Line::space(), clause])))
        }
        (keyword, clause) => Some(Block::stack1([keyword, clause.indent()])),
    }
}

/// Upstream's `formatListing`.
fn listing<T>(listing: &Listing<T>, items: impl Fn(&T) -> Vec<Block>) -> Option<Block> {
    match listing {
        Listing::Closed => None,
        Listing::Open(comments) => Some(parens(commented(
            &comments.before,
            text(".."),
            &comments.after,
        ))),
        Listing::Explicit(values, multiline) => {
            let items = items(values);
            (!items.is_empty()).then(|| group(Delimiters::PARENS, *multiline, items))
        }
    }
}

/// Operators, then types, then values; upstream's
/// `formatDetailedListing`.
fn detailed_listing(listing: &DetailedListing) -> Vec<Block> {
    let operators = listing
        .operators
        .iter()
        .map(|(symbol, v)| commented(&v.comments.before, op(symbol), &v.comments.after));
    let types = listing.types.iter().map(|(name, v)| {
        commented(
            &v.comments.before,
            union(name, &v.value.comments, &v.value.value),
            &v.comments.after,
        )
    });
    let values = listing
        .values
        .iter()
        .map(|(name, v)| commented(&v.comments.before, var(name), &v.comments.after));
    operators.chain(types).chain(values).collect()
}

/// Upstream's `formatVarValue`.
fn value(value: &Value) -> Block {
    match value {
        Value::Var(name) => var(name),
        Value::Op(symbol) => op(symbol),
        Value::Union(name, ctors) => union(&name.value, &name.comments, ctors),
    }
}

fn var(lower: &Lower) -> Block {
    Block::line(name(&lower.0))
}

fn op(symbol: &Symbol) -> Block {
    Block::line(operator(symbol))
}

/// A type, with `(..)` for any constructors, as Elm 0.19 exposes them.
fn union(type_name: &Upper, post: &Comments, ctors: &Listing<CommentedMap<Upper, ()>>) -> Block {
    let ctors = listing(ctors, |ctors| {
        ctors
            .iter()
            .map(|(ctor, v)| {
                commented(
                    &v.comments.before,
                    Block::line(name(&ctor.0)),
                    &v.comments.after,
                )
            })
            .collect()
    });
    let mut line = name(&type_name.0);
    if ctors.is_some() {
        line.push(Line::text("(..)"));
    }
    tail_commented(Block::line(line), post)
}

#[cfg(test)]
mod tests;
