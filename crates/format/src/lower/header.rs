//! The module header and imports, after upstream's `Parse/Module.hs`.

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::{TextRange, TextSize};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxToken, significant_tokens};

use super::cursor::Cursor;
use super::{VALID, child_tokens, commas, lower_name, start, upper_name, upper_names};
use crate::tree::{
    Around, C1, C2, Comment, CommentedMap, Comments, DetailedListing, ForceMultiline, Header,
    ImportKey, ImportMethod, Listing, Namespace, SourceSetting, SourceTag, Symbol, Upper, Value, c,
};

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
    let before_name = cx.take(cx.range(name.syntax()).start());
    let names = upper_names(name.syntax());

    if let Some(where_kw) = node.where_token() {
        let list = node.exposing_list();
        let after_name = cx.take(
            list.as_ref()
                .map_or_else(|| start(&where_kw), |l| cx.range(l.syntax()).start()),
        );
        let exports = match list {
            Some(list) => detailed_listing(cx, &list),
            None => Listing::Open(Around::default()),
        };
        let before_where = cx.take(start(&where_kw));
        return Header {
            source_tag,
            name: c(Around::new(before_name, after_name), names),
            settings: None,
            exports: Some(c(Around::new(before_where, Vec::new()), exports)),
        };
    }

    Header {
        source_tag,
        name: c(Around::new(before_name, Vec::new()), names),
        settings: node.effect_fields().map(|f| settings(cx, &f)),
        exports: node.exposing_list().map(|l| exposing(cx, &l)),
    }
}

/// `where { command = MyCmd }`.
fn settings(cx: &mut Cursor, fields: &ast::EffectFields) -> C2<Vec<SourceSetting>> {
    let where_kw = fields.where_token().expect(VALID);
    let l_brace = fields.l_brace_token().expect(VALID);
    let r_brace = fields.r_brace_token().expect(VALID);
    let before = cx.take(start(&where_kw));
    let after = cx.take(start(&l_brace));
    let items = fields
        .effect_fields()
        .map(|f| (cx.range(f.syntax()), f))
        .collect();
    // Each setting takes the comments around it, before its key and after
    // its value.
    let settings = comma_list(
        cx,
        items,
        commas(fields.syntax()),
        Some(start(&r_brace)),
        |cx, field| {
            let key = field.lower_ident_token().expect(VALID);
            let eq = field.eq_token().expect(VALID);
            let value = field.upper_ident_token().expect(VALID);
            let before_eq = cx.take(start(&eq));
            let after_eq = cx.take(start(&value));
            SourceSetting {
                key: c(Around::new(Vec::new(), before_eq), lower_name(&key)),
                value: c(Around::new(after_eq, Vec::new()), upper_name(&value)),
            }
        },
    )
    .into_iter()
    .map(|item| {
        let mut setting = item.value;
        setting.key.comments.before = item.comments.before;
        setting.value.comments.after = item.comments.after;
        setting
    })
    .collect();
    c(Around { before, after }, settings)
}

/// An import's module name, and the rest with the comments before the name.
pub(super) fn import(cx: &mut Cursor, node: &ast::Import) -> (Namespace, C1<ImportMethod>) {
    let name = node.module_name().expect(VALID);
    let before_name = cx.take(cx.range(name.syntax()).start());
    let alias = node.import_alias().map(|alias| {
        let as_kw = alias.as_token().expect(VALID);
        let name = alias.upper_ident_token().expect(VALID);
        let before = cx.take(start(&as_kw));
        let after = cx.take(start(&name));
        c(Around { before, after }, upper_name(&name))
    });
    let exposed = match node.exposing_list() {
        Some(list) => exposing(cx, &list),
        None => C2::bare(Listing::Closed),
    };
    (
        upper_names(name.syntax()),
        c(before_name, ImportMethod { alias, exposed }),
    )
}

/// `exposing (…)`, or a listing without `exposing`, which upstream reads
/// with no comments before it.
fn exposing(cx: &mut Cursor, list: &ast::ExposingList) -> C2<Listing<DetailedListing>> {
    let first = list.l_paren_token().or_else(|| list.dot_dot_token());
    let comments = match list.exposing_token() {
        Some(kw) => {
            let before = cx.take(start(&kw));
            let after = first.map_or_else(
                || cx.range(list.items().next().expect(VALID).syntax()).start(),
                |t| start(&t),
            );
            Around::new(before, cx.take(after))
        }
        None => Around::new(Vec::new(), cx.take(cx.range(list.syntax()).start())),
    };
    c(comments, detailed_listing(cx, list))
}

/// The listing in `list`, in parentheses or not.
fn detailed_listing(cx: &mut Cursor, list: &ast::ExposingList) -> Listing<DetailedListing> {
    let dot_dot = list.dot_dot_token();
    let items = |cx: &mut Cursor, end| {
        let items = list.items().map(|i| (cx.range(i.syntax()), i)).collect();
        to_detailed_listing(comma_list(cx, items, commas(list.syntax()), end, value))
    };
    match (list.l_paren_token(), list.r_paren_token()) {
        (Some(l_paren), Some(r_paren)) => listing(cx, &l_paren, &r_paren, dot_dot, items),
        _ if dot_dot.is_some() => Listing::Open(Around::default()),
        _ => Listing::Explicit(items(cx, None), ForceMultiline(false)),
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
            let before = cx.take(start(&dot_dot));
            Listing::Open(Around::new(before, cx.take(end)))
        }
        None => Listing::Explicit(items(cx, Some(end)), ForceMultiline(multiline)),
    }
}

/// Comma-separated items, each with the comments before it and those
/// after, which start with the `--` comment ending its line; upstream's
/// `commaSep1'`. The last item's comments run to `end`, if the list has
/// one.
fn comma_list<N, T>(
    cx: &mut Cursor,
    items: Vec<(TextRange, N)>,
    mut commas: impl Iterator<Item = TextSize>,
    end: Option<TextSize>,
    mut lower: impl FnMut(&mut Cursor, N) -> T,
) -> Vec<C2<T>> {
    let count = items.len();
    items
        .into_iter()
        .enumerate()
        .map(|(i, (range, node))| {
            let before = cx.take(range.start());
            let value = lower(cx, node);
            let mut after: Comments = cx.eol(range).map(Comment::Line).into_iter().collect();
            match commas.next() {
                Some(comma) if i + 1 < count => after.extend(cx.take(comma)),
                _ => after.extend(end.map(|end| cx.take(end)).unwrap_or_default()),
            }
            c(Around { before, after }, value)
        })
        .collect()
}

/// One item of an `exposing` list.
fn value(cx: &mut Cursor, item: ast::Exposed) -> Value {
    match item {
        ast::Exposed::ExposedValue(v) => {
            Value::Var(lower_name(&v.lower_ident_token().expect(VALID)))
        }
        ast::Exposed::ExposedOperator(o) => {
            // `-` lexes apart from other operators.
            let op = o.op().expect(VALID);
            let symbol = significant_tokens(op.syntax()).next().expect(VALID);
            Value::Op(Symbol(symbol.text().to_owned()))
        }
        ast::Exposed::ExposedType(t) => {
            let name = upper_name(&t.upper_ident_token().expect(VALID));
            let Some(variants) = t.exposed_variants() else {
                return Value::Union(c(Vec::new(), name), Listing::Closed);
            };
            let l_paren = variants.l_paren_token().expect(VALID);
            let r_paren = variants.r_paren_token().expect(VALID);
            let inner = cx.take(start(&l_paren));
            let dot_dot = variants.dot_dot_token();
            let ctors = listing(cx, &l_paren, &r_paren, dot_dot, |cx, end| {
                let items = child_tokens(variants.syntax(), SyntaxKind::UpperIdent)
                    .map(|t| (t.text_range(), upper_name(&t)))
                    .collect();
                let ctors = comma_list(cx, items, commas(variants.syntax()), end, |_, n| n);
                let mut set = BTreeMap::new();
                for ctor in ctors {
                    insert_with(&mut set, ctor.value, c(ctor.comments, ()), |new, old| {
                        merge_c2(new, old, unit)
                    });
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
fn insert_with<K: Ord, V>(map: &mut BTreeMap<K, V>, key: K, new: V, merge: impl FnOnce(V, V) -> V) {
    let value = match map.remove(&key) {
        Some(old) => merge(new, old),
        None => new,
    };
    map.insert(key, value);
}

/// Two values of a set, merged.
fn unit((): (), (): ()) {}

fn merge_c2<T>(left: C2<T>, right: C2<T>, merge: impl FnOnce(T, T) -> T) -> C2<T> {
    let (mut comments, more) = (left.comments, right.comments);
    comments.before.extend(more.before);
    comments.after.extend(more.after);
    c(comments, merge(left.value, right.value))
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
        insert_with(&mut map, key, value, |new, old| merge_c2(old, new, &merge));
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
        (Listing::Open(mut comments), Listing::Open(more)) => {
            comments.before.extend(more.before);
            comments.after.extend(more.after);
            Listing::Open(comments)
        }
        (open @ Listing::Open(..), _) | (_, open @ Listing::Open(..)) => open,
        (Listing::Explicit(a, multiline), Listing::Explicit(b, multiline2)) => {
            Listing::Explicit(merge(a, b), ForceMultiline(multiline.0 || multiline2.0))
        }
    }
}

type Ctors = C1<Listing<CommentedMap<Upper, ()>>>;

fn merge_type(left: Ctors, right: Ctors) -> Ctors {
    merge_c1(left, right, |a, b| {
        merge_listing(a, b, |a, b| merge_map(a, b, unit))
    })
}

fn merge_detailed_listing(left: DetailedListing, right: DetailedListing) -> DetailedListing {
    DetailedListing {
        values: merge_map(left.values, right.values, unit),
        operators: merge_map(left.operators, right.operators, unit),
        types: merge_map(left.types, right.types, merge_type),
    }
}

/// Merges an import of `name` into the earlier ones, as upstream does,
/// but for a deviation (`DECISIONS.md`): where the merged import's alias
/// would lose a qualifier in `used`, the import stays apart, in the next
/// group of `name`'s imports, which upstream's map keyed by name lacks.
pub(super) fn insert_import(
    map: &mut BTreeMap<ImportKey, C1<ImportMethod>>,
    name: &Namespace,
    new: C1<ImportMethod>,
    used: &BTreeSet<String>,
) {
    let full = name
        .iter()
        .map(|part| part.0.as_str())
        .collect::<Vec<_>>()
        .join(".");
    let mut group = 0;
    loop {
        let key = ImportKey {
            module: name.clone(),
            group,
        };
        let Some(old) = map.get(&key) else {
            map.insert(key, new);
            return;
        };
        let (old_alias, new_alias) = (old.value.alias.as_ref(), new.value.alias.as_ref());
        let kept = qualifier(&full, new_alias.or(old_alias));
        let lost = [qualifier(&full, old_alias), qualifier(&full, new_alias)]
            .into_iter()
            .any(|q| q != kept && used.contains(q));
        if !lost {
            insert_with(map, key, new, merge_import);
            return;
        }
        group += 1;
    }
}

/// What an import's names are qualified with: its alias, or else `full`,
/// its module's name.
fn qualifier<'a>(full: &'a str, alias: Option<&'a C2<Upper>>) -> &'a str {
    alias.map_or(full, |a| a.value.0.as_str())
}

/// Two imports of one module: the later one's comments first, its alias
/// if it has one, and both exposing lists.
fn merge_import(new: C1<ImportMethod>, old: C1<ImportMethod>) -> C1<ImportMethod> {
    merge_c1(new, old, |new, old| ImportMethod {
        alias: new.alias.or(old.alias),
        exposed: merge_c2(new.exposed, old.exposed, |a, b| {
            merge_listing(a, b, merge_detailed_listing)
        }),
    })
}
