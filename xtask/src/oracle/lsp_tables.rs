//! `cargo xtask oracle lsp-tables`: elm-language-server's completion data,
//! written to `crates/ide/src/completion_data.rs`: the
//! package ranking that orders its auto-imports (`ranking.ts`), and its
//! snippets, keywords and the type it completes without a declaration
//! (`completionProvider.ts`, `elmUtils.ts`).
//!
//! The snippet methods are calls of `createSnippet` with literal
//! arguments, so Node evaluates their bodies against a stub that records
//! each call; the ranking is a literal table.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

const SOURCE: &str = "reference/elm-language-server/src";
const OUTPUT: &str = "crates/ide/src/completion_data.rs";

pub(crate) fn run(root: &Path) -> Result<()> {
    let source = root.join(SOURCE);
    let ranking = ranking(&fs::read_to_string(
        source.join("common/providers/ranking.ts"),
    )?)?;
    let provider = fs::read_to_string(source.join("common/providers/completionProvider.ts"))?;
    let inline = snippets(&body(
        &provider,
        "private createSnippetsInline(): CompletionItem[] {",
    )?)?;
    let start = snippets(&body(
        &provider,
        "private createSnippetsStartOfLine(): CompletionItem[] {",
    )?)?;
    let utils = fs::read_to_string(source.join("compiler/utils/elmUtils.ts"))?;
    let list = empty_list_markdown(&utils)?;

    let mut out = String::from(
        "//! elm-language-server 2.10.0's completion data, written by `cargo xtask oracle\n\
         //! lsp-tables` from its `ranking.ts`, `completionProvider.ts` and `elmUtils.ts`\n\
         //! (MIT, THIRD-PARTY-NOTICES.md). Regenerate it; don't edit it.\n\n",
    );
    out.push_str("/// Each ranked package and its rank, by name.\n");
    out.push_str("pub(crate) const RANKING: &[(&str, &str)] = &[\n");
    for (name, rank) in &ranking {
        out.push_str(&format!("    ({name:?}, {rank:?}),\n"));
    }
    out.push_str("];\n\n");
    for (name, doc, table) in [
        (
            "SNIPPETS_INLINE",
            "The snippets anywhere in an expression",
            &inline,
        ),
        (
            "SNIPPETS_START_OF_LINE",
            "The snippets at a line's start",
            &start,
        ),
    ] {
        out.push_str(&format!(
            "/// {doc}: label, text, documentation, and whether it's a keyword.\n"
        ));
        out.push_str(&format!(
            "pub(crate) const {name}: &[(&str, &str, Option<&str>, bool)] = &[\n"
        ));
        for (label, text, documentation, keyword) in table {
            out.push_str(&format!(
                "    ({label:?}, {text:?}, {documentation:?}, {keyword}),\n"
            ));
        }
        out.push_str("];\n\n");
    }
    out.push_str("/// `getEmptyTypes`: the documentation of `List`, which no module declares.\n");
    out.push_str(&format!(
        "pub(crate) const LIST_MARKDOWN: &str = {list:?};\n"
    ));
    fs::write(root.join(OUTPUT), out)?;
    eprintln!(
        "  {} ranked packages, {} + {} snippets",
        ranking.len(),
        inline.len(),
        start.len()
    );
    Ok(())
}

/// `RANKING_LIST`'s entries, sorted by name for a binary search.
fn ranking(text: &str) -> Result<Vec<(String, String)>> {
    let mut found = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('"') else {
            continue;
        };
        let Some((name, rest)) = rest.split_once("\": \"") else {
            continue;
        };
        let rank = rest.trim_end_matches(',').trim_end_matches('"');
        found.push((name.to_owned(), rank.to_owned()));
    }
    if found.is_empty() {
        bail!("no ranking found");
    }
    found.sort();
    Ok(found)
}

/// A snippet: its label, text, documentation, and whether it's a keyword.
type Snippet = (String, String, Option<String>, bool);

/// The body of the method that `signature` opens.
fn body(text: &str, signature: &str) -> Result<String> {
    let start = text.find(signature).context("a snippet method")? + signature.len();
    let mut depth = 1;
    for (i, c) in text[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(text[start..start + i].to_owned());
                }
            }
            _ => {}
        }
    }
    bail!("an unclosed method")
}

/// The `createSnippet` calls of a method's body, run in Node.
fn snippets(body: &str) -> Result<Vec<Snippet>> {
    let script = format!(
        "const CompletionItemKind = {{ Keyword: 14, Snippet: 15 }};\n\
         const self = {{ createSnippet: (label, text, documentation, kind) => ({{\n\
           label, text: Array.isArray(text) ? text.join('\\n') : text,\n\
           documentation: documentation ?? null, keyword: kind === 14 }}) }};\n\
         const found = (function () {{ {body} }}).call(self);\n\
         process.stdout.write(JSON.stringify(found));\n"
    );
    let output = Command::new("node")
        .args(["-e", &script])
        .output()
        .context("running node")?;
    if !output.status.success() {
        bail!("node: {}", String::from_utf8_lossy(&output.stderr));
    }
    let found: Vec<Value> = serde_json::from_slice(&output.stdout)?;
    Ok(found
        .into_iter()
        .map(|s| {
            (
                s["label"].as_str().unwrap_or_default().to_owned(),
                s["text"].as_str().unwrap_or_default().to_owned(),
                s["documentation"].as_str().map(str::to_owned),
                s["keyword"].as_bool().unwrap_or(false),
            )
        })
        .collect())
}

/// The Markdown of `getEmptyTypes`' one type, as its template string
/// holds it.
fn empty_list_markdown(text: &str) -> Result<String> {
    let start = text.find("markdown: `").context("getEmptyTypes")? + "markdown: `".len();
    let mut markdown = String::new();
    let mut chars = text[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            '`' => return Ok(markdown),
            '\\' => markdown.extend(chars.next()),
            c => markdown.push(c),
        }
    }
    bail!("an unclosed template string")
}
