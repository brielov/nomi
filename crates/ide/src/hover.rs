//! Hover (plan 6.16): the type of the expression under the cursor, the
//! innermost one, from inference's per-expression types (plan 6.8); on a
//! top-level declaration's name, its annotation, inferred where it has
//! none. Types read as type errors show them, named as the module's
//! imports let it write them.

use std::path::Path;

use nomi_base::{TextRange, TextSize};
use nomi_compiler::canonicalize::foreign::with_defaults;
use nomi_compiler::canonicalize::module;
use nomi_compiler::hir::Def;
use nomi_compiler::types::localizer::Localizer;
use nomi_compiler::types::render::{self, Context};
use nomi_compiler::types::{self, error};
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::SyntaxNode;

/// What to show for the place under the cursor: Markdown, and the range
/// it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    pub range: TextRange,
    pub markdown: String,
}

/// The hover at `offset` in the file at `path`.
pub fn hover(index: &Index, path: &Path, offset: TextSize) -> Option<Hover> {
    let module = index.module(path)?;
    let canonical = module.canonical.as_ref()?;
    let localizer = localizer(index, module);
    let (annotations, types) = types::infer_types(index.compat, canonical);
    // A top-level declaration's name: its annotation.
    for &id in &module.hir.values {
        if let Def::Define { name, .. } = &module.hir.defs[id]
            && name.range.contains_inclusive(offset)
        {
            let annotation = match &annotations {
                Ok(annotations) => annotations.get(&name.text)?.clone(),
                Err(_) => canonical.annotations.get(&name.text)?.clone(),
            };
            let ty = render::can_to_doc(&localizer, Context::None, &annotation.ty).plain();
            return Some(Hover {
                range: name.range,
                markdown: code(&format!("{} : {ty}", name.text)),
            });
        }
    }
    // The innermost expression around the cursor.
    let (range, ty) = (types.iter())
        .filter_map(|(range, ty)| Some((trimmed(&module.root, *range)?, ty)))
        .filter(|(range, _)| range.contains_inclusive(offset))
        .min_by_key(|(range, _)| range.len())?;
    let ty = error::to_doc(&localizer, Context::None, ty).plain();
    Some(Hover {
        range,
        markdown: code(&ty),
    })
}

fn code(text: &str) -> String {
    format!("```elm\n{text}\n```")
}

/// The localizer of `module`, as its type errors would name types.
pub(crate) fn localizer(index: &Index, indexed: &Indexed) -> Localizer {
    let imports = with_defaults(&index.package, &indexed.hir.imports);
    Localizer::from_module(index.compat, module::name(&indexed.hir), &imports)
}

/// `range` from its first token that isn't trivia.
fn trimmed(root: &SyntaxNode, range: TextRange) -> Option<TextRange> {
    let range = range.intersect(root.text_range())?;
    let token = root.token_at_offset(range.start()).right_biased()?;
    let mut token = token;
    while token.kind().is_trivia() {
        token = token.next_token()?;
    }
    let start = token.text_range().start();
    (start <= range.end()).then(|| TextRange::new(start, range.end()))
}
