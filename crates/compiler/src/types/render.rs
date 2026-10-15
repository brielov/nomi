//! Types laid out for reports (`Reporting/Render/Type.hs`): functions,
//! applications, tuples and records, each parenthesized as its place
//! needs, breaking across lines where they don't fit.

use nomi_base::diagnostic::doc::{Doc, cat, sep, text, vcat};

use super::localizer::Localizer;
use crate::canonical;

/// `Context`: where a type is, which decides its parentheses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    None,
    /// An argument or result of a function.
    Func,
    /// An argument of a type.
    App,
}

/// `lambda`.
pub fn lambda(context: Context, arg1: Doc, arg2: Doc, args: Vec<Doc>) -> Doc {
    let mut parts = vec![arg1];
    parts.extend(
        std::iter::once(arg2)
            .chain(args)
            .map(|a| text("->").space(a)),
    );
    let doc = sep(parts).align();
    match context {
        Context::None => doc,
        Context::Func | Context::App => cat([text("("), doc, text(")")]),
    }
}

/// `apply`.
pub fn apply(context: Context, name: Doc, args: Vec<Doc>) -> Doc {
    if args.is_empty() {
        return name;
    }
    let doc = sep(std::iter::once(name).chain(args)).hang(4);
    match context {
        Context::App => cat([text("("), doc, text(")")]),
        Context::Func | Context::None => doc,
    }
}

/// `tuple`.
pub fn tuple(a: Doc, b: Doc, cs: Vec<Doc>) -> Doc {
    let entries = (std::iter::once(a)
        .chain(std::iter::once(b))
        .chain(cs)
        .enumerate())
    .map(|(i, d)| text(if i == 0 { "(" } else { "," }).space(d));
    sep([cat(entries), text(")")]).align()
}

/// `entryToDoc`.
fn entry((name, ty): (Doc, Doc)) -> Doc {
    sep([name.space(text(":")), ty]).hang(4)
}

/// Each doc after `first` for the first, `rest` for the others.
fn led(first: &str, rest: &str, docs: Vec<Doc>) -> Vec<Doc> {
    (docs.into_iter().enumerate())
        .map(|(i, d)| text(if i == 0 { first } else { rest }).space(d))
        .collect()
}

/// `record`.
pub fn record(entries: Vec<(Doc, Doc)>, ext: Option<Doc>) -> Doc {
    let fields: Vec<Doc> = entries.into_iter().map(entry).collect();
    match (fields.is_empty(), ext) {
        (true, None) => text("{}"),
        (false, None) => sep([cat(led("{", ",", fields)), text("}")]).align(),
        (_, Some(ext)) => sep([
            sep([text("{").space(ext), cat(led("|", ",", fields))]).hang(4),
            text("}"),
        ])
        .align(),
    }
}

/// `vrecordSnippet`: one field, then the others, then `...`.
pub(crate) fn vrecord_snippet(first: (Doc, Doc), entries: Vec<(Doc, Doc)>) -> Doc {
    let field = text("{").space(entry(first));
    let fields = (entries
        .into_iter()
        .map(entry)
        .chain(std::iter::once(text("..."))))
    .map(|d| text(",").space(d));
    vcat(
        std::iter::once(field)
            .chain(fields)
            .chain(std::iter::once(text("}"))),
    )
}

/// `vrecord`: a record a field a line.
pub(crate) fn vrecord(entries: Vec<(Doc, Doc)>, ext: Option<Doc>) -> Doc {
    let fields: Vec<Doc> = entries.into_iter().map(entry).collect();
    match (fields.is_empty(), ext) {
        (true, None) => text("{}"),
        (false, None) => vcat(
            led("{", ",", fields)
                .into_iter()
                .chain(std::iter::once(text("}"))),
        ),
        (_, Some(ext)) => vcat([
            vcat([text("{").space(ext), cat(led("|", ",", fields))]).hang(4),
            text("}"),
        ]),
    }
}

/// `canToDoc`: a canonical type, as annotations show.
pub fn can_to_doc(localizer: &Localizer, context: Context, ty: &canonical::Type) -> Doc {
    use canonical::Type as T;
    nomi_base::grow(|| match ty {
        T::Lambda(arg1, result) => {
            let mut rest = vec![];
            let mut at = &**result;
            while let T::Lambda(a, r) = at {
                rest.push(&**a);
                at = r;
            }
            rest.push(at);
            let arg2 = rest.remove(0);
            lambda(
                context,
                can_to_doc(localizer, Context::Func, arg1),
                can_to_doc(localizer, Context::Func, arg2),
                rest.into_iter()
                    .map(|t| can_to_doc(localizer, Context::Func, t))
                    .collect(),
            )
        }
        T::Var(name) => text(name.clone()),
        T::Type { home, name, args } => apply(
            context,
            localizer.to_doc(home, name),
            args.iter()
                .map(|a| can_to_doc(localizer, Context::App, a))
                .collect(),
        ),
        T::Record(fields, ext) => {
            // `fieldsToList`: by their place as written.
            let mut fields: Vec<(&String, &canonical::FieldType)> = fields.iter().collect();
            fields.sort_by_key(|(_, f)| f.order);
            let entries = (fields.into_iter())
                .map(|(n, f)| (text(n.clone()), can_to_doc(localizer, Context::None, &f.ty)))
                .collect();
            record(entries, ext.as_ref().map(|e| text(e.clone())))
        }
        T::Unit => text("()"),
        T::Tuple(a, b, c) => tuple(
            can_to_doc(localizer, Context::None, a),
            can_to_doc(localizer, Context::None, b),
            c.iter()
                .map(|c| can_to_doc(localizer, Context::None, c))
                .collect(),
        ),
        T::Alias {
            home, name, args, ..
        } => apply(
            context,
            localizer.to_doc(home, name),
            args.iter()
                .map(|(_, a)| can_to_doc(localizer, Context::App, a))
                .collect(),
        ),
    })
}
