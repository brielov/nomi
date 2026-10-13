//! The module header and imports, after upstream's `Parse/Module.hs`.

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::{TextRange, TextSize};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxToken};

use super::cursor::Cursor;
use super::{VALID, c, commas, significant, start, token};
use crate::tree::{
    C1, C2, Comment, CommentedMap, Comments, DetailedListing, Header, ImportMethod, Listing, Lower,
    Namespace, SourceSettings, SourceTag, Symbol, Upper, Value,
};

fn namespace(name: &ast::ModuleName) -> Namespace {
    name.syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::UpperIdent)
        .map(|t| Upper(t.text().to_owned()))
        .collect()
}

/// `moduleDecl`: the 0.16 form when `where` ends the header, else 0.17's.
pub(super) fn header(cx: &mut Cursor, node: &ast::ModuleHeader) -> Header {
    let module_kw = node.module_token().expect(VALID);
    let source_tag = if node.port_token().is_some() {
        SourceTag::Port(cx.take(start(&module_kw)))
    } else if node.effect_token().is_some() {
        SourceTag::Effect(cx.take(start(&module_kw)))
    } else {
        SourceTag::Normal
    };
    let name = node.module_name().expect(VALID);
    let pre_name = cx.take(cx.range(name.syntax()).start());
    let names = namespace(&name);

    if let Some(where_kw) = node.where_token() {
        let list = node.exposing_list();
        let post_name = cx.take(
            list.as_ref()
                .map_or_else(|| start(&where_kw), |l| cx.range(l.syntax()).start()),
        );
        let exports = match list {
            Some(list) => detailed_listing(cx, &list),
            None => Listing::Open(Vec::new(), Vec::new()),
        };
        let pre_where = cx.take(start(&where_kw));
        return Header {
            source_tag,
            name: c((pre_name, post_name), names),
            settings: None,
            exports: Some(c((pre_where, Vec::new()), exports)),
        };
    }

    Header {
        source_tag,
        name: c((pre_name, Vec::new()), names),
        settings: node.effect_fields().map(|f| settings(cx, &f)),
        exports: node.exposing_list().map(|l| exposing(cx, &l)),
    }
}

/// `where { command = MyCmd }`.
fn settings(cx: &mut Cursor, fields: &ast::EffectFields) -> C2<SourceSettings> {
    let where_kw = fields.where_token().expect(VALID);
    let l_brace = fields.l_brace_token().expect(VALID);
    let r_brace = fields.r_brace_token().expect(VALID);
    let pre = cx.take(start(&where_kw));
    let post = cx.take(start(&l_brace));
    let items = fields
        .effect_fields()
        .map(|f| (cx.range(f.syntax()), f))
        .collect();
    let settings = comma_list(
        cx,
        items,
        &commas(fields.syntax()),
        Some(start(&r_brace)),
        |cx, field| {
            let key = field.lower_ident_token().expect(VALID);
            let eq = field.eq_token().expect(VALID);
            let value = field.upper_ident_token().expect(VALID);
            let pre_eq = cx.take(start(&eq));
            let post_eq = cx.take(start(&value));
            (
                Lower(key.text().to_owned()),
                pre_eq,
                post_eq,
                Upper(value.text().to_owned()),
            )
        },
    )
    .into_iter()
    .map(|item| {
        let (pre, post) = item.comments;
        let (key, pre_eq, post_eq, value) = item.value;
        (c((pre, pre_eq), key), c((post_eq, post), value))
    })
    .collect();
    c((pre, post), settings)
}

/// An import's module name, and the rest with the comments before the name.
pub(super) fn import(cx: &mut Cursor, node: &ast::Import) -> (Namespace, C1<ImportMethod>) {
    let name = node.module_name().expect(VALID);
    let pre_name = cx.take(cx.range(name.syntax()).start());
    let alias = node.import_alias().map(|alias| {
        let as_kw = alias.as_token().expect(VALID);
        let name = alias.upper_ident_token().expect(VALID);
        let pre = cx.take(start(&as_kw));
        let post = cx.take(start(&name));
        c((pre, post), Upper(name.text().to_owned()))
    });
    let exposed = match node.exposing_list() {
        Some(list) => exposing(cx, &list),
        None => c((Vec::new(), Vec::new()), Listing::Closed),
    };
    (
        namespace(&name),
        c(pre_name, ImportMethod { alias, exposed }),
    )
}

/// `exposing (…)`, or a listing without `exposing`, which upstream reads
/// with no comments before it.
fn exposing(cx: &mut Cursor, list: &ast::ExposingList) -> C2<Listing<DetailedListing>> {
    let first = list
        .l_paren_token()
        .or_else(|| token(list.syntax(), SyntaxKind::DotDot));
    let (pre, post) = match list.exposing_token() {
        Some(kw) => {
            let pre = cx.take(start(&kw));
            let after = first.map_or_else(|| item_ranges(cx, list)[0].start(), |t| start(&t));
            (pre, cx.take(after))
        }
        None => (Vec::new(), cx.take(cx.range(list.syntax()).start())),
    };
    c((pre, post), detailed_listing(cx, list))
}

fn item_ranges(cx: &Cursor, list: &ast::ExposingList) -> Vec<TextRange> {
    list.items().map(|i| cx.range(i.syntax())).collect()
}

/// The listing in `list`, in parentheses or not.
fn detailed_listing(cx: &mut Cursor, list: &ast::ExposingList) -> Listing<DetailedListing> {
    let dot_dot = token(list.syntax(), SyntaxKind::DotDot);
    let items = |cx: &mut Cursor, end| {
        let items = list.items().map(|i| (cx.range(i.syntax()), i)).collect();
        to_detailed_listing(comma_list(cx, items, &commas(list.syntax()), end, value))
    };
    match (list.l_paren_token(), list.r_paren_token()) {
        (Some(l_paren), Some(r_paren)) => listing(cx, &l_paren, &r_paren, dot_dot, items),
        _ if dot_dot.is_some() => Listing::Open(Vec::new(), Vec::new()),
        _ => Listing::Explicit(items(cx, None), false),
    }
}

/// `(..)` or `(a, b)`, multiline when a newline outside block comments
/// falls between the parentheses.
fn listing<T>(
    cx: &mut Cursor,
    l_paren: &SyntaxToken,
    r_paren: &SyntaxToken,
    dot_dot: Option<SyntaxToken>,
    items: impl FnOnce(&mut Cursor, Option<TextSize>) -> T,
) -> Listing<T> {
    let end = start(r_paren);
    let multiline = cx.multiline(TextRange::new(l_paren.text_range().end(), end));
    match dot_dot {
        Some(dot_dot) => {
            let pre = cx.take(start(&dot_dot));
            Listing::Open(pre, cx.take(end))
        }
        None => Listing::Explicit(items(cx, Some(end)), multiline),
    }
}

/// The starts of the commas among `node`'s children.
/// Comma-separated items, each with the comments before it and those
/// after, which start with the `--` comment ending its line; upstream's
/// `commaSep1'`. The last item's comments run to `end`, if the list has
/// one.
fn comma_list<N, T>(
    cx: &mut Cursor,
    items: Vec<(TextRange, N)>,
    commas: &[TextSize],
    end: Option<TextSize>,
    mut lower: impl FnMut(&mut Cursor, N) -> T,
) -> Vec<C2<T>> {
    let count = items.len();
    items
        .into_iter()
        .enumerate()
        .map(|(i, (range, node))| {
            let pre = cx.take(range.start());
            let value = lower(cx, node);
            let mut post: Comments = cx.eol(range).map(Comment::Line).into_iter().collect();
            match commas.get(i) {
                Some(&comma) if i + 1 < count => post.extend(cx.take(comma)),
                _ => post.extend(end.map(|end| cx.take(end)).unwrap_or_default()),
            }
            c((pre, post), value)
        })
        .collect()
}

/// One item of an `exposing` list.
fn value(cx: &mut Cursor, item: ast::Exposed) -> Value {
    match item {
        ast::Exposed::ExposedValue(v) => {
            Value::Var(Lower(v.lower_ident_token().expect(VALID).text().to_owned()))
        }
        ast::Exposed::ExposedOperator(o) => {
            // `-` lexes apart from other operators.
            let op = o.op().expect(VALID);
            let symbol = significant(op.syntax()).next().expect(VALID);
            Value::Op(Symbol(symbol.text().to_owned()))
        }
        ast::Exposed::ExposedType(t) => {
            let name = Upper(t.upper_ident_token().expect(VALID).text().to_owned());
            let Some(variants) = t.exposed_variants() else {
                return Value::Union(c(Vec::new(), name), Listing::Closed);
            };
            let l_paren = variants.l_paren_token().expect(VALID);
            let r_paren = variants.r_paren_token().expect(VALID);
            let inner = cx.take(start(&l_paren));
            let dot_dot = token(variants.syntax(), SyntaxKind::DotDot);
            let ctors = listing(cx, &l_paren, &r_paren, dot_dot, |cx, end| {
                let items = variants
                    .syntax()
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .filter(|t| t.kind() == SyntaxKind::UpperIdent)
                    .map(|t| (t.text_range(), Upper(t.text().to_owned())))
                    .collect();
                let ctors = comma_list(cx, items, &commas(variants.syntax()), end, |_, n| n);
                let mut set = BTreeMap::new();
                for ctor in ctors {
                    insert_with(&mut set, ctor.value, c(ctor.comments, ()), merge_c2_unit);
                }
                set
            });
            Value::Union(c(inner, name), ctors)
        }
    }
}

/// Upstream's `toDetailedListing`: a value or operator listed twice keeps
/// only its last comments, and a type listed twice merges.
fn to_detailed_listing(items: Vec<C2<Value>>) -> DetailedListing {
    let mut listing = DetailedListing::default();
    for item in items {
        let comments = item.comments;
        match item.value {
            Value::Var(name) => {
                listing.values.insert(name, c(comments, ()));
            }
            Value::Op(name) => {
                listing.operators.insert(name, c(comments, ()));
            }
            Value::Union(name, ctors) => insert_with(
                &mut listing.types,
                name.value,
                c(comments, c(name.comments, ctors)),
                |new, old| merge_c2(new, old, merge_type),
            ),
        }
    }
    listing
}

/// Upstream's `insertWith`: `merge(new, old)` when `key` is there.
pub(super) fn insert_with<K: Ord, V>(
    map: &mut BTreeMap<K, V>,
    key: K,
    new: V,
    merge: impl FnOnce(V, V) -> V,
) {
    let value = match map.remove(&key) {
        Some(old) => merge(new, old),
        None => new,
    };
    map.insert(key, value);
}

fn merge_c2<T>(left: C2<T>, right: C2<T>, merge: impl FnOnce(T, T) -> T) -> C2<T> {
    let ((mut pre, mut post), (pre2, post2)) = (left.comments, right.comments);
    pre.extend(pre2);
    post.extend(post2);
    c((pre, post), merge(left.value, right.value))
}

fn merge_c2_unit(left: C2<()>, right: C2<()>) -> C2<()> {
    merge_c2(left, right, |(), ()| ())
}

fn merge_c1<T>(left: C1<T>, right: C1<T>, merge: impl FnOnce(T, T) -> T) -> C1<T> {
    let mut comments = left.comments;
    comments.extend(right.comments);
    c(comments, merge(left.value, right.value))
}

/// Upstream's `mergeCommentedMap`: both maps, merging where both have a
/// key, left first.
fn merge_map<K: Ord, V>(
    left: CommentedMap<K, V>,
    right: CommentedMap<K, V>,
    merge: impl Fn(V, V) -> V,
) -> CommentedMap<K, V> {
    let mut map = left;
    for (key, value) in right {
        match map.remove(&key) {
            Some(left) => map.insert(key, merge_c2(left, value, &merge)),
            None => map.insert(key, value),
        };
    }
    map
}

/// Upstream's `mergeListing`: `(..)` wins over a list of names.
fn merge_listing<T>(
    left: Listing<T>,
    right: Listing<T>,
    merge: impl FnOnce(T, T) -> T,
) -> Listing<T> {
    match (left, right) {
        (Listing::Closed, other) | (other, Listing::Closed) => other,
        (Listing::Open(mut pre, mut post), Listing::Open(pre2, post2)) => {
            pre.extend(pre2);
            post.extend(post2);
            Listing::Open(pre, post)
        }
        (open @ Listing::Open(..), _) | (_, open @ Listing::Open(..)) => open,
        (Listing::Explicit(a, multiline), Listing::Explicit(b, multiline2)) => {
            Listing::Explicit(merge(a, b), multiline || multiline2)
        }
    }
}

type Ctors = C1<Listing<CommentedMap<Upper, ()>>>;

fn merge_type(left: Ctors, right: Ctors) -> Ctors {
    merge_c1(left, right, |a, b| {
        merge_listing(a, b, |a, b| merge_map(a, b, |(), ()| ()))
    })
}

fn merge_detailed_listing(left: DetailedListing, right: DetailedListing) -> DetailedListing {
    DetailedListing {
        values: merge_map(left.values, right.values, |(), ()| ()),
        operators: merge_map(left.operators, right.operators, |(), ()| ()),
        types: merge_map(left.types, right.types, merge_type),
    }
}

/// Merges an import of `name` into the earlier ones, as upstream does,
/// but for a deviation (`DECISIONS.md`): where the merged import's alias
/// would lose a qualifier in `used`, the import stays apart, in the next
/// group of `name`'s imports, which upstream's map keyed by name lacks.
pub(super) fn insert_import(
    map: &mut BTreeMap<(Namespace, usize), C1<ImportMethod>>,
    name: &Namespace,
    new: C1<ImportMethod>,
    used: &BTreeSet<String>,
) {
    let full = (name.iter())
        .map(|part| part.0.as_str())
        .collect::<Vec<_>>()
        .join(".");
    let qualifier =
        |alias: Option<&C2<Upper>>| alias.map_or_else(|| full.clone(), |a| a.value.0.clone());
    let mut group = 0;
    loop {
        let key = (name.clone(), group);
        let Some(old) = map.get(&key) else {
            map.insert(key, new);
            return;
        };
        let (old_alias, new_alias) = (old.value.alias.as_ref(), new.value.alias.as_ref());
        let kept = qualifier(new_alias.or(old_alias));
        let lost = [qualifier(old_alias), qualifier(new_alias)]
            .into_iter()
            .any(|q| q != kept && used.contains(&q));
        if !lost {
            insert_with(map, key, new, merge_import);
            return;
        }
        group += 1;
    }
}

/// Two imports of one module: the later one's comments first, its alias
/// if it has one, and both exposing lists.
pub(super) fn merge_import(new: C1<ImportMethod>, old: C1<ImportMethod>) -> C1<ImportMethod> {
    merge_c1(new, old, |new, old| ImportMethod {
        alias: new.alias.or(old.alias),
        exposed: merge_c2(new.exposed, old.exposed, |a, b| {
            merge_listing(a, b, merge_detailed_listing)
        }),
    })
}
