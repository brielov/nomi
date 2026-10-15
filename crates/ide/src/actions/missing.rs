//! What elm's reports say is missing: a `case`'s branches and a record's
//! field.

use std::fmt::Write as _;

use nomi_base::TextRange;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{
    SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, child_token, child_tokens, tokens,
};

use super::usage::{render_usage, todo_at, todo_type, with_todos};
use super::{Diagnosed, Fixing, Offer, here};
use crate::problems::Code;
use crate::tree::{code_range, covering_node, last_child_token};
use crate::{Edit, FileEdits, column};

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
fn case_branch_edit(fixing: &Fixing<'_>, diagnostic: &Diagnosed) -> Option<Edit> {
    let module = fixing.context.module;
    let text = &*module.text;
    let case = covering_node(&module.root(), diagnostic.range)?
        .ancestors()
        .find(|n| n.kind() == SyntaxKind::CaseExpr)?;
    let branches: Vec<SyntaxNode> = case
        .children()
        .filter(|c| c.kind() == SyntaxKind::CaseBranch)
        .collect();
    let (first, last) = (branches.first()?, branches.last()?);
    let indent = column(text, code_range(first)?.start());
    let pattern = first.children().find(|c| ast::Pattern::can_cast(c.kind()));
    let body = first
        .children()
        .filter(|c| !ast::Pattern::can_cast(c.kind()))
        .last();
    let body_indent = body
        .and_then(|b| code_range(&b))
        .map_or(indent + 4, |r| column(text, r.start()));
    // The first branch's upper-case names but the last, as its prefix.
    let mut uppers: Vec<String> = (pattern.iter())
        .flat_map(tokens)
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
        let _ = write!(
            inserted,
            "\n\n{}{missing} ->\n{}Debug.todo \"branch '{missing}' not implemented\"",
            " ".repeat(indent),
            " ".repeat(body_indent)
        );
    }
    Some(Edit::insert(code_range(last)?.end(), inserted))
}

pub(super) fn case_branch_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    // The diagnostic at `range`, whose message lists what's missing.
    let given = (fixing.request.given.iter()).find(|d| {
        d.range == range && (is_missing_patterns(d) || d.is(Code::IncompleteCasePattern))
    });
    let edits = given.and_then(|d| case_branch_edit(fixing, d));
    let edits = here(fixing, edits.into_iter().collect());
    vec![Offer::new("Add missing case branches", edits)]
}

pub(super) fn case_branch_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, edits: &mut Vec<Edit>) {
    edits.extend(case_branch_edit(fixing, diagnostic));
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
    let at = covering_node(root, diagnostic.range)?;
    let owner = at.ancestors().find(|n| {
        matches!(
            n.kind(),
            SyntaxKind::FieldAccessExpr | SyntaxKind::RecordUpdateExpr | SyntaxKind::Field
        )
    })?;
    match owner.kind() {
        SyntaxKind::FieldAccessExpr => {
            let name = last_child_token(&owner, SyntaxKind::LowerIdent)?;
            Some((owner, name))
        }
        SyntaxKind::Field => {
            let name =
                code_range(&owner).and_then(|_| child_token(&owner, SyntaxKind::LowerIdent))?;
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
                .find(|f| child_tokens(f, SyntaxKind::LowerIdent).any(|t| t.text() == wanted))?;
            let name = child_token(&field, SyntaxKind::LowerIdent)?;
            Some((owner, name))
        }
    }
}

/// `addMissingRecordFieldCodeAction`: the field, typed as its usage
/// expects, after the last field of the record's alias in this file.
fn record_field_edits(fixing: &Fixing<'_>, diagnostic: &Diagnosed) -> Option<FileEdits> {
    let context = &fixing.context;
    let module = context.module;
    let (owner, field) = missing_field(&module.root(), diagnostic)?;
    // The record: the access's target, or the update's.
    let target = (owner.children_with_tokens()).find_map(|e| match e {
        SyntaxElement::Node(n) => child_token(&n, SyntaxKind::LowerIdent),
        SyntaxElement::Token(t) => (t.kind() == SyntaxKind::LowerIdent).then_some(t),
    })?;
    let at = target.text_range().start();
    let alias = crate::record_fields::alias_of_name(context.index, module, at, target.text())?;
    // In this file only.
    if alias.module.path != module.path {
        return None;
    }
    let record = (alias.decl.children()).find(|c| c.kind() == SyntaxKind::RecordType)?;
    let last = record
        .children()
        .filter(|c| c.kind() == SyntaxKind::FieldType)
        .last()?;
    // The type the usage expects: the access, or the update's value.
    let expression = if owner.kind() == SyntaxKind::FieldAccessExpr {
        code_range(&owner)
    } else {
        (field.parent().and_then(|f| f.children().last())).and_then(|value| code_range(&value))
    };
    let ty = expression_type(fixing, expression?)?;
    let text = &*module.text;
    let last_range = code_range(&last)?;
    let record_range = code_range(&record).unwrap_or(last_range);
    let prefix = if text[record_range].contains('\n') {
        let indent = column(text, last_range.start()).saturating_sub(2);
        format!("\n{}", " ".repeat(indent))
    } else {
        String::new()
    };
    let edit = Edit::insert(
        last_range.end(),
        format!("{prefix}, {} : {ty}", field.text()),
    );
    Some(here(fixing, vec![edit]))
}

/// The type the expression at `range` has where it's used: the module
/// inferred with it made `Debug.todo`, and the missing values too.
fn expression_type(fixing: &Fixing<'_>, range: TextRange) -> Option<String> {
    let mut ranges: Vec<TextRange> = (fixing.program.iter())
        .filter(|d| {
            d.is(Code::MissingValue)
                && (d.range.end() <= range.start() || d.range.start() >= range.end())
        })
        .map(|d| d.range)
        .collect();
    ranges.push(range);
    ranges.sort_by_key(|r| r.start());
    let types = with_todos(fixing, &ranges)?;
    let ty = todo_type(&types, todo_at(&ranges, range))?;
    Some(render_usage(fixing, ty))
}

pub(super) fn record_field_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    let given = (fixing.request.given.iter())
        .find(|d| d.range == range && (is_missing_field(d) || d.is(Code::RecordField)));
    let Some(edits) = given.and_then(|given| record_field_edits(fixing, given)) else {
        return Vec::new();
    };
    vec![Offer::new("Create missing record field", edits)]
}

pub(super) fn record_field_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, files: &mut FileEdits) {
    if let Some(edits) = record_field_edits(fixing, diagnostic) {
        files.merge(edits);
    }
}
