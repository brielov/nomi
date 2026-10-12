//! What elm's reports say is missing: a `case`'s branches and a record's
//! field.

use super::*;

/// Whether `diagnostic` is elm's report of a `case` that misses patterns.
pub(super) fn is_missing_patterns(diagnostic: &Diagnosed) -> bool {
    diagnostic.message.starts_with("MISSING PATTERNS")
}

/// The patterns a report lists as missing: elm's, `#`-styled, or
/// elm-language-server's checker's.
pub(super) fn missing_patterns(message: &str) -> Vec<String> {
    let Some((_, after)) = message.split_once("Missing possibilities include:") else {
        return Vec::new();
    };
    (after.lines())
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty() && l.starts_with("  "))
        .map(|l| {
            let l = l.trim();
            let l = l
                .strip_prefix('#')
                .and_then(|l| l.strip_suffix('#'))
                .unwrap_or(l);
            l.to_owned()
        })
        .collect()
}

/// `addMissingCaseBranchesCodeAction`: a `Debug.todo` branch for each
/// missing pattern after the last branch, prefixed as the first branch's
/// constructor is qualified.
pub(super) fn case_branch_edits(fixing: &Fixing<'_>, diagnostic: &Diagnosed) -> Vec<Edit> {
    let module = fixing.context.module;
    let text = &*module.text;
    let case = match module.root.covering_element(diagnostic.range) {
        SyntaxElement::Node(node) => node,
        SyntaxElement::Token(token) => match token.parent() {
            Some(parent) => parent,
            None => return Vec::new(),
        },
    };
    let Some(case) = case.ancestors().find(|n| n.kind() == SyntaxKind::CaseExpr) else {
        return Vec::new();
    };
    let branches: Vec<SyntaxNode> = case
        .children()
        .filter(|c| c.kind() == SyntaxKind::CaseBranch)
        .collect();
    let (Some(first), Some(last)) = (branches.first(), branches.last()) else {
        return Vec::new();
    };
    let column_of = |at: TextSize| {
        let line_start = text[..usize::from(at)].rfind('\n').map_or(0, |i| i + 1);
        text[line_start..usize::from(at)].chars().count()
    };
    let Some(first_range) = code_range(first) else {
        return Vec::new();
    };
    let indent = column_of(first_range.start());
    let pattern = first.children().find(|c| crate::lint::is_pattern(c.kind()));
    let body = first
        .children()
        .filter(|c| !crate::lint::is_pattern(c.kind()))
        .last();
    let body_indent = body
        .and_then(|b| code_range(&b))
        .map_or(indent + 4, |r| column_of(r.start()));
    // The first branch's upper-case names but the last, as its prefix.
    let mut uppers: Vec<String> = (pattern.iter())
        .flat_map(|p| p.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::UpperIdent)
        .map(|t| t.text().to_owned())
        .collect();
    uppers.pop();
    let prefix = uppers.join(".");
    let mut inserted = String::new();
    for missing in missing_patterns(&diagnostic.message) {
        let missing = if prefix.is_empty() {
            missing
        } else {
            format!("{prefix}.{missing}")
        };
        inserted.push_str(&format!(
            "\n\n{}{missing} ->\n{}Debug.todo \"branch '{missing}' not implemented\"",
            " ".repeat(indent),
            " ".repeat(body_indent)
        ));
    }
    let Some(end) = code_range(last).map(|r| r.end()) else {
        return Vec::new();
    };
    vec![Edit {
        range: TextRange::empty(end),
        text: inserted,
    }]
}

pub(super) fn case_branch_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    // The diagnostic at `range`, whose message lists what's missing.
    let given = (fixing.given.iter()).find(|d| {
        d.range == range && (is_missing_patterns(d) || d.code == "incomplete_case_pattern")
    });
    let edits = given
        .map(|d| case_branch_edits(fixing, d))
        .unwrap_or_default();
    vec![(
        "Add missing case branches".to_owned(),
        here(fixing, edits),
        None,
    )]
}

pub(super) fn case_branch_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, edits: &mut Vec<Edit>) {
    edits.extend(case_branch_edits(fixing, diagnostic));
}

/// Whether `diagnostic` is elm's report of a record without a field.
pub(super) fn is_missing_field(diagnostic: &Diagnosed) -> bool {
    diagnostic.message.starts_with("TYPE MISMATCH")
        && diagnostic
            .message
            .lines()
            .next()
            .is_some_and(|l| l.contains("record does not have a `"))
}

/// The field access or update a missing-field diagnostic is about, and
/// the field's name: at the record elm points at, or at the field.
pub(super) fn missing_field(
    root: &SyntaxNode,
    diagnostic: &Diagnosed,
) -> Option<(SyntaxNode, SyntaxToken)> {
    let at = match root.covering_element(diagnostic.range) {
        SyntaxElement::Node(node) => node,
        SyntaxElement::Token(token) => token.parent()?,
    };
    let owner = at.ancestors().find(|n| {
        matches!(
            n.kind(),
            SyntaxKind::FieldAccessExpr | SyntaxKind::RecordUpdateExpr | SyntaxKind::Field
        )
    })?;
    let field_name = |node: &SyntaxNode| {
        (node.children_with_tokens())
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == SyntaxKind::LowerIdent)
            .last()
    };
    match owner.kind() {
        SyntaxKind::FieldAccessExpr => Some((owner.clone(), field_name(&owner)?)),
        SyntaxKind::Field => {
            let name = crate::symbols::code_range(&owner).and_then(|_| {
                (owner.children_with_tokens())
                    .filter_map(|e| e.into_token())
                    .find(|t| t.kind() == SyntaxKind::LowerIdent)
            })?;
            Some((owner.parent()?, name))
        }
        _ => {
            // An update: the field elm's message names.
            let wanted = diagnostic
                .message
                .split("does not have a `")
                .nth(1)?
                .split('`')
                .next()?;
            let field = (owner.children())
                .filter(|c| c.kind() == SyntaxKind::Field)
                .find(|f| {
                    (f.children_with_tokens())
                        .filter_map(|e| e.into_token())
                        .any(|t| t.kind() == SyntaxKind::LowerIdent && t.text() == wanted)
                })?;
            let name = (field.children_with_tokens())
                .filter_map(|e| e.into_token())
                .find(|t| t.kind() == SyntaxKind::LowerIdent)?;
            Some((owner, name))
        }
    }
}

/// `addMissingRecordFieldCodeAction`: the field, typed as its usage
/// expects, after the last field of the record's alias in this file.
pub(super) fn record_field_edits(fixing: &Fixing<'_>, diagnostic: &Diagnosed) -> FileEdits {
    let context = &fixing.context;
    let module = context.module;
    let Some((owner, field)) = missing_field(&module.root, diagnostic) else {
        return Vec::new();
    };
    // The record: the access's target, or the update's.
    let Some(target) = (owner.children_with_tokens())
        .filter_map(|e| match e {
            SyntaxElement::Node(n) => crate::lint::first_token(&n, SyntaxKind::LowerIdent),
            SyntaxElement::Token(t) => (t.kind() == SyntaxKind::LowerIdent).then_some(t),
        })
        .next()
    else {
        return Vec::new();
    };
    let Some(alias) = crate::record_fields::alias_declaration(
        context.index,
        module,
        target.text_range().start(),
        target.text(),
    ) else {
        return Vec::new();
    };
    // In this file only: its tree is this module's.
    if !alias
        .ancestors()
        .last()
        .is_some_and(|root| root.text() == *module.text)
    {
        return Vec::new();
    }
    let Some(record) = alias
        .children()
        .find(|c| c.kind() == SyntaxKind::RecordType)
    else {
        return Vec::new();
    };
    let Some(last) = record
        .children()
        .filter(|c| c.kind() == SyntaxKind::FieldType)
        .last()
    else {
        return Vec::new();
    };
    // The type the usage expects: the access, or the update's value.
    let typed = if owner.kind() == SyntaxKind::FieldAccessExpr {
        code_range(&owner).and_then(|range| expression_type(fixing, range))
    } else {
        field
            .parent()
            .and_then(|f| f.children().last())
            .and_then(|value| code_range(&value))
            .and_then(|range| expression_type(fixing, range))
    };
    let Some(ty) = typed else {
        return Vec::new();
    };
    let text = &*module.text;
    let Some(last_range) = code_range(&last) else {
        return Vec::new();
    };
    let line_of = |at: TextSize| text[..usize::from(at)].matches('\n').count();
    let record_range = code_range(&record).unwrap_or(last_range);
    let new_line = line_of(record_range.start()) != line_of(record_range.end());
    let line_start = text[..usize::from(last_range.start())]
        .rfind('\n')
        .map_or(0, |i| i + 1);
    let column = text[line_start..usize::from(last_range.start())]
        .chars()
        .count();
    let prefix = if new_line {
        format!("\n{}", " ".repeat(column.saturating_sub(2)))
    } else {
        String::new()
    };
    let edit = Edit {
        range: TextRange::empty(last_range.end()),
        text: format!("{prefix}, {} : {ty}", field.text()),
    };
    here(fixing, vec![edit])
}

/// The type the expression at `range` has where it's used: the module
/// inferred with it made `Debug.todo`, and the missing values too.
pub(super) fn expression_type(fixing: &Fixing<'_>, range: TextRange) -> Option<String> {
    let module = fixing.context.module;
    let insert = "(Debug.todo \"\")";
    let mut edits: Vec<(TextRange, String)> = (fixing.program.iter())
        .filter(|d| {
            d.code == "missing_value" && d.range.end() <= range.start()
                || d.code == "missing_value" && d.range.start() >= range.end()
        })
        .map(|d| (d.range, insert.to_owned()))
        .collect();
    edits.push((range, insert.to_owned()));
    edits.sort_by_key(|(r, _)| r.start());
    let (_, types) = crate::record_fields::patched_all(fixing.context.index, module, &edits)?;
    let shift: i64 = (edits.iter())
        .take_while(|(r, _)| r.start() < range.start())
        .map(|(r, i)| i.len() as i64 - i64::from(u32::from(r.len())))
        .sum();
    let start = TextSize::new((i64::from(u32::from(range.start())) + shift) as u32);
    let at = TextRange::at(start, TextSize::of(insert));
    let inner = TextRange::new(at.start() + TextSize::new(1), at.end() - TextSize::new(1));
    let ty = (types.iter())
        .rev()
        .find(|(r, _)| *r == at || *r == inner)
        .map(|(_, ty)| ty)?;
    Some(render_usage(fixing, ty))
}

pub(super) fn record_field_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    let given = (fixing.given.iter())
        .find(|d| d.range == range && (is_missing_field(d) || d.code == "record_field"));
    let Some(given) = given else {
        return Vec::new();
    };
    let edits = record_field_edits(fixing, given);
    if edits.is_empty() {
        return Vec::new();
    }
    let single = edits.iter().all(|(_, e)| e.len() == 1);
    let title = if single {
        "Create missing record field"
    } else {
        "Create missing record fields"
    };
    vec![(title.to_owned(), edits, None)]
}

pub(super) fn record_field_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, files: &mut FileEdits) {
    merge(files, record_field_edits(fixing, diagnostic));
}
