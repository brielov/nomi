//! The module line, its doc comment and the imports; upstream's
//! `formatModuleHeader`, `formatModuleLine`, `formatImports` and the
//! listings they share.

use crate::block::{Block, Line};
use crate::markdown;
use crate::structure::{equals_pair, group, group_with_footer, space_sep_or_indented};
use crate::tree::{
    C1, C2, CommentedMap, Comments, CommonDeclaration, Declaration, DetailedListing, Header,
    ImportMethod, Listing, Lower, Module, Namespace, Pattern, SourceSettings, SourceTag, Symbol,
    TopLevelStructure, Upper, Value,
};

use super::{
    commented, comments, doc_comment, keyword_commented, name, parens, please_report,
    pre_commented, qualified, tail_commented,
};

fn text(text: &str) -> Block {
    Block::line(Line::text(text))
}

/// The module line, the module's docs and the imports, with a blank line
/// between each two. A module with no line gets the default one if
/// `add_default_header`, else none; upstream's `formatModuleHeader`. But
/// for a deviation (`DECISIONS.md`): where the module declares ports, the
/// default line is a port module's, as Elm reads the module, where
/// upstream's makes Elm reject it.
pub(super) fn module_header(module: &Module, add_default_header: bool) -> Vec<Block> {
    let ports = (module.body.iter()).any(|item| {
        matches!(
            item,
            TopLevelStructure::Entry(Declaration::PortAnnotation(..))
        )
    });
    let default = Header {
        source_tag: if ports {
            SourceTag::Port(Vec::new())
        } else {
            SourceTag::Normal
        },
        name: c((Vec::new(), Vec::new()), vec![Upper("Main".to_owned())]),
        settings: None,
        exports: None,
    };
    let header = match &module.header {
        Some(header) => Some(header),
        None if add_default_header => Some(&default),
        None => None,
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
    let force = matches!(
        header.exports.as_ref().map(|e| &e.value),
        Some(Listing::Explicit(_, true))
    );
    let sorted = sort_values(force, to_expose, &documented);
    module_line_block(header, sorted)
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
            _ => &[],
        })
        .map(|group| group.iter().map(|n| ref_name(n).to_owned()).collect())
        .collect()
}

fn c<C, T>(comments: C, value: T) -> crate::tree::Commented<C, T> {
    crate::tree::Commented { comments, value }
}

fn join_with_blank_lines(parts: Vec<Vec<Block>>) -> Vec<Block> {
    let mut blocks = Vec::new();
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 {
            blocks.push(Block::blank_line());
        }
        blocks.extend(part);
    }
    blocks
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
            Declaration::PortAnnotation(name, ..) => values.push(var(&name.value)),
            Declaration::Common(CommonDeclaration::Definition(pattern, ..)) => match pattern {
                Pattern::Var(name) => values.push(var(name)),
                Pattern::Record(fields) => values.extend(fields.iter().map(|f| var(&f.value))),
                _ => {}
            },
            Declaration::Datatype { name_with_args, .. } => values.push(union(
                &name_with_args.value.name,
                Listing::Open(Vec::new(), Vec::new()),
            )),
            Declaration::TypeAlias(_, name_with_args, _) => {
                values.push(union(&name_with_args.value.name, Listing::Closed));
            }
            _ => {}
        }
    }
    values
        .into_iter()
        .map(|v| c((Vec::new(), Vec::new()), v))
        .collect()
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

/// The exposed items in groups: one per `@docs` line, in its order, then
/// the rest; and the comments of the items the docs moved, which lose
/// their place. Upstream's `sortVars`.
fn sort_values(
    force_multiline: bool,
    exposing: Vec<C2<Value>>,
    documented: &[Vec<String>],
) -> (Vec<Vec<C2<Value>>>, Comments) {
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
        _ => 2,
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
            .map(|v| c((Vec::new(), Vec::new()), v.value.clone()))
    };
    let mut seen: Vec<C2<Value>> = Vec::new();
    let mut in_docs = Vec::new();
    for group in documented {
        let mut kept = Vec::new();
        for value in group.iter().filter_map(|n| allowed(n)) {
            if !seen.contains(&value) {
                seen.push(value.clone());
                kept.push(value);
            }
        }
        if !kept.is_empty() {
            in_docs.push(kept);
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
            let (pre, post) = value.comments;
            moved_comments.extend(pre);
            moved_comments.extend(post);
        } else {
            remaining.push(value);
        }
    }
    if in_docs.is_empty() && force_multiline {
        return (
            remaining.into_iter().map(|v| vec![v]).collect(),
            moved_comments,
        );
    }
    if !remaining.is_empty() {
        in_docs.push(remaining);
    }
    (in_docs, moved_comments)
}

/// `module Name exposing (…)`; upstream's `formatModuleLine`.
fn module_line_block(header: &Header, (groups, extra): (Vec<Vec<C2<Value>>>, Comments)) -> Block {
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
    let footer: Vec<Block> = comments(&extra).into_iter().collect();
    let exports = match &groups[..] {
        [] => text("(..)"),
        [group] => group_with_footer(
            false,
            "(",
            ",",
            footer,
            ")",
            false,
            group.iter().map(commented_value).collect(),
        ),
        groups => group_with_footer(
            false,
            "(",
            ",",
            footer,
            ")",
            true,
            groups
                .iter()
                .map(|values| {
                    let pre: Comments = values.iter().flat_map(|v| v.comments.0.clone()).collect();
                    let post: Comments = values.iter().flat_map(|v| v.comments.1.clone()).collect();
                    let inner = group(
                        false,
                        "",
                        ",",
                        "",
                        false,
                        values.iter().map(|v| value(&v.value)).collect(),
                    );
                    commented(&pre, inner, &post)
                })
                .collect(),
        ),
    };
    let where_clause = header.settings.as_ref().map(|settings| {
        let (pre, post) = &settings.comments;
        keyword_commented("where", pre, post, source_settings(&settings.value))
    });
    let (pre_name, post_name) = &header.name.comments;
    let name_clause = match (
        tag,
        commented(
            pre_name,
            Block::line(qualified(&header.name.value)),
            post_name,
        ),
    ) {
        (Block::SingleLine(tag), Block::SingleLine(name)) => {
            Block::line(Line::row([tag, Line::space(), name]))
        }
        (tag, name) => Block::stack1([tag, name.indent()]),
    };
    let (pre_exposing, post_exposing) = header.exports.as_ref().map_or((&[][..], &[][..]), |e| {
        (&e.comments.0[..], &e.comments.1[..])
    });
    let exposing = commented(pre_exposing, text("exposing"), post_exposing);
    space_sep_or_indented(
        space_sep_or_indented(
            name_clause,
            where_clause.into_iter().chain([exposing]).collect(),
        ),
        vec![exports],
    )
}

fn commented_value(v: &C2<Value>) -> Block {
    commented(&v.comments.0, value(&v.value), &v.comments.1)
}

/// `{ command = MyCmd, … }`; upstream's `formatSettings` over
/// `formatRecordPair`.
fn source_settings(settings: &SourceSettings) -> Block {
    let pairs = settings
        .iter()
        .map(|(key, value)| {
            let (pre, post_key) = &key.comments;
            let pair = equals_pair(
                "=",
                false,
                commented(&[], Block::line(name(&key.value.0)), post_key),
                commented(
                    &value.comments.0,
                    Block::line(name(&value.value.0)),
                    &value.comments.1,
                ),
            );
            pre_commented(pre, pair)
        })
        .collect();
    group(true, "{", ",", "}", false, pairs)
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
        .map(|((name, _), method)| import(name, method))
        .collect();
    if !imports.is_empty() {
        parts.push(imports);
    }
    join_with_blank_lines(parts)
}

/// Upstream's `formatImport`.
fn import(module: &Namespace, method: &C1<ImportMethod>) -> Block {
    let ImportMethod { alias, exposed } = &method.value;
    let name = pre_commented(&method.comments, Block::line(qualified(module)));
    // An alias that repeats the module's one-part name goes.
    let alias = alias
        .as_ref()
        .filter(|alias| !matches!(&module[..], [only] if *only == alias.value));
    let alias = alias.and_then(|alias| {
        import_clause(
            Some(Block::line(name_of(&alias.value))),
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
        let mut row = Line::text("import");
        for part in parts {
            row.push(Line::space());
            row.push(part);
        }
        Block::line(row)
    };
    match (name, alias, exposing) {
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

fn name_of(upper: &Upper) -> Line {
    name(&upper.0)
}

/// `keyword clause`, or `None` without a clause; upstream's
/// `formatImportClause`.
fn import_clause(
    clause: Option<Block>,
    keyword: &str,
    (pre, post): &(Comments, Comments),
) -> Option<Block> {
    let Some(clause) = clause else {
        if pre.is_empty() && post.is_empty() {
            return None;
        }
        return Some(please_report(
            "UNEXPECTED IMPORT",
            "import clause comments with no clause",
        ));
    };
    match (
        pre_commented(pre, text(keyword)),
        pre_commented(post, clause),
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
        Listing::Open(pre, post) => Some(parens(commented(pre, text(".."), post))),
        Listing::Explicit(values, multiline) => {
            let items = items(values);
            (!items.is_empty()).then(|| group(false, "(", ",", ")", *multiline, items))
        }
    }
}

/// Operators, then types, then values; upstream's
/// `formatDetailedListing`.
fn detailed_listing(listing: &DetailedListing) -> Vec<Block> {
    let operators = listing
        .operators
        .iter()
        .map(|(symbol, v)| commented(&v.comments.0, op(symbol), &v.comments.1));
    let types = listing.types.iter().map(|(name, v)| {
        commented(
            &v.comments.0,
            union(name, &v.value.comments, &v.value.value),
            &v.comments.1,
        )
    });
    let values = listing
        .values
        .iter()
        .map(|(name, v)| commented(&v.comments.0, var(name), &v.comments.1));
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
    Block::line(Line::text(format!("({})", symbol.0)))
}

/// A type, with `(..)` for any constructors, as Elm 0.19 exposes them.
fn union(type_name: &Upper, post: &Comments, ctors: &Listing<CommentedMap<Upper, ()>>) -> Block {
    let ctors = listing(ctors, |ctors| {
        ctors
            .iter()
            .map(|(ctor, v)| commented(&v.comments.0, Block::line(name_of(ctor)), &v.comments.1))
            .collect()
    });
    let mut line = name_of(type_name);
    if ctors.is_some() {
        line.push(Line::text("(..)"));
    }
    tail_commented(Block::line(line), post)
}

#[cfg(test)]
mod tests;
