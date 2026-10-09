//! `cargo xtask corpus hir`: every corpus module elm accepts (as the
//! newest release, which accepts the most) lowers to a HIR with nothing
//! missing, whose every ID maps to a node of its kind (plan 5.1), and
//! formatting it leaves that HIR's shape unchanged (plan 5.15).

use std::path::Path;

use anyhow::{Result, ensure};
use nomi_base::Compat;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{Tool, parse_module, validate};

use super::parallel;
use super::syntax::modules;

pub fn check(root: &Path) -> Result<()> {
    let modules = modules(root)?;
    let results = parallel(&modules, |m| -> Result<Option<Vec<String>>> {
        let bytes = std::fs::read(&m.file)?;
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return Ok(None);
        };
        let parse = parse_module(text);
        let tool = Tool::Elm(m.source, Compat::LATEST);
        if !parse.errors().is_empty() || !validate(&parse, tool).is_empty() {
            return Ok(None);
        }
        let root = parse.syntax();
        let module = ast::Module::cast(root.clone()).expect("a module");
        let hir = nomi_compiler::hir::lower(&module);
        let mut problems = nomi_compiler::hir::verify(&hir, &root);
        let change = nomi_engine::format::shape_change(text, tool);
        problems.extend(change.map(|d| format!("formatting changes it {d}")));
        Ok(Some(problems))
    });
    let (mut lowered, mut failed) = (0, Vec::new());
    for (m, result) in modules.iter().zip(results) {
        if let Some(problems) = result? {
            lowered += 1;
            if !problems.is_empty() {
                failed.push(format!("{}/{}: {}", m.case, m.path, problems.join("; ")));
            }
        }
    }
    for failure in failed.iter().take(20) {
        eprintln!("{failure}");
    }
    ensure!(
        failed.is_empty(),
        "{} of {lowered} modules lower badly",
        failed.len()
    );
    println!("hir: {lowered} modules lower, every ID at its node, and format to the same shape");
    Ok(())
}
