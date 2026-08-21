use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use proc_macro2::{LineColumn, Span};
use sha2::{Digest, Sha256};
use verus_syn::spanned::Spanned;
use verus_syn::visit::{self, Visit};
use verus_syn::{
    Assert, AssertForall, Assume, BinOp, Expr, ExprBinary, ExprClosure, ExprIf, ExprUnary,
    ExprWhile, FnMode, ImplItemFn, ItemFn, ItemMacro, RevealHide, UnOp,
};
use walkdir::WalkDir;

use crate::cargo::WorkspacePackage;
use crate::config::{Config, OperatorsConfig};
use crate::model::{Campaign, Mutant, OracleKind, OracleSpec};

pub fn automatic_mutants(
    root: &Path,
    packages: &[WorkspacePackage],
    config: &Config,
) -> Result<Vec<Mutant>> {
    let excludes = build_globs(&config.project.exclude_globs)?;
    let excluded_functions = build_globs(&config.project.exclude_functions)?;
    let mut mutants = Vec::new();
    let mut visited = BTreeSet::new();
    for package in packages {
        if !config.project.include_packages.is_empty()
            && !config.project.include_packages.contains(&package.name)
        {
            continue;
        }
        for source_root in &package.source_roots {
            for entry in WalkDir::new(source_root).follow_links(false) {
                let entry = entry?;
                let path = entry.path();
                if !entry.file_type().is_file() || path.extension().is_none_or(|ext| ext != "rs") {
                    continue;
                }
                let absolute = path.canonicalize()?;
                if !visited.insert((package.name.clone(), absolute.clone())) {
                    continue;
                }
                let relative = absolute
                    .strip_prefix(root)
                    .with_context(|| {
                        format!(
                            "{} is outside project root {}",
                            absolute.display(),
                            root.display()
                        )
                    })?
                    .to_path_buf();
                if excludes.is_match(&relative) {
                    continue;
                }
                discover_file(
                    root,
                    package,
                    &relative,
                    &config.operators,
                    &excluded_functions,
                    &mut mutants,
                )?;
            }
        }
    }
    mutants.sort_by(|a, b| a.id.cmp(&b.id));
    mutants.dedup_by(|a, b| a.id == b.id);
    Ok(mutants)
}

fn build_globs(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let normalized = if pattern.ends_with('/') {
            format!("{}**", pattern)
        } else {
            pattern.clone()
        };
        builder.add(
            Glob::new(&normalized).with_context(|| format!("invalid exclude glob {pattern}"))?,
        );
    }
    Ok(builder.build()?)
}

fn discover_file(
    root: &Path,
    package: &WorkspacePackage,
    relative: &Path,
    operators: &OperatorsConfig,
    excluded_functions: &GlobSet,
    mutants: &mut Vec<Mutant>,
) -> Result<()> {
    let source = fs::read_to_string(root.join(relative))?;
    let syntax = match verus_syn::parse_file(&source) {
        Ok(syntax) => syntax,
        Err(error) => {
            eprintln!("warning: skipping {}: {error}", relative.display());
            return Ok(());
        }
    };
    let mut macros = VerusMacroVisitor {
        root,
        package,
        relative,
        source: &source,
        operators,
        excluded_functions,
        mutants,
    };
    macros.visit_file(&syntax);
    Ok(())
}

struct VerusMacroVisitor<'a> {
    root: &'a Path,
    package: &'a WorkspacePackage,
    relative: &'a Path,
    source: &'a str,
    operators: &'a OperatorsConfig,
    excluded_functions: &'a GlobSet,
    mutants: &'a mut Vec<Mutant>,
}

impl<'ast> Visit<'ast> for VerusMacroVisitor<'_> {
    fn visit_item_macro(&mut self, node: &'ast ItemMacro) {
        let is_verus = node
            .mac
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "verus");
        if is_verus {
            match verus_syn::parse2::<verus_syn::File>(node.mac.tokens.clone()) {
                Ok(file) => {
                    let mut visitor = ExecVisitor {
                        package: self.package,
                        relative: self.relative,
                        source: self.source,
                        operators: self.operators,
                        excluded_functions: self.excluded_functions,
                        mutants: self.mutants,
                        function: None,
                    };
                    visitor.visit_file(&file);
                }
                Err(error) => eprintln!(
                    "warning: cannot parse verus! body in {}: {error}",
                    self.root.join(self.relative).display()
                ),
            }
        } else {
            visit::visit_item_macro(self, node);
        }
    }
}

struct ExecVisitor<'a> {
    package: &'a WorkspacePackage,
    relative: &'a Path,
    source: &'a str,
    operators: &'a OperatorsConfig,
    excluded_functions: &'a GlobSet,
    mutants: &'a mut Vec<Mutant>,
    function: Option<String>,
}

impl ExecVisitor<'_> {
    fn in_exec(mode: &FnMode) -> bool {
        matches!(mode, FnMode::Default | FnMode::Exec(_))
    }

    fn add(&mut self, span: Span, operator: &str, replacement: String) {
        if self.function.is_none() {
            return;
        }
        let Some((start, end)) = byte_range(self.source, span) else {
            return;
        };
        if start >= end || end > self.source.len() {
            return;
        }
        let original = self.source[start..end].to_string();
        if original == replacement {
            return;
        }
        let function = self.function.clone();
        let identity = format!(
            "{}\0{}\0{}\0{}\0{}\0{}",
            self.package.name,
            self.relative.display(),
            function.as_deref().unwrap_or("<unknown>"),
            operator,
            original,
            start
        );
        let id = format!(
            "VM-{}",
            &hex::encode(Sha256::digest(identity.as_bytes()))[..16]
        );
        self.mutants.push(Mutant {
            id,
            campaign: Campaign::Exec,
            operator: operator.into(),
            package: self.package.name.clone(),
            file: self.relative.to_path_buf(),
            function,
            start,
            end,
            original,
            replacement,
            expected_occurrences: 1,
            oracle: OracleSpec {
                kind: OracleKind::Verus,
                package: Some(self.package.name.clone()),
                command: Vec::new(),
                expected_pattern: None,
                required_test_count: None,
            },
        });
    }

    fn add_condition(&mut self, expr: &Expr) {
        if self.operators.condition_to_true {
            self.add(expr.span(), "condition-to-true", "true".into());
        }
        if self.operators.condition_to_false {
            self.add(expr.span(), "condition-to-false", "false".into());
        }
    }
}

impl<'ast> Visit<'ast> for ExecVisitor<'_> {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        if !Self::in_exec(&node.sig.mode)
            || self.excluded_functions.is_match(node.sig.ident.to_string())
        {
            return;
        }
        let previous = self.function.replace(node.sig.ident.to_string());
        visit::visit_block(self, &node.block);
        self.function = previous;
    }

    fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
        if !Self::in_exec(&node.sig.mode)
            || self.excluded_functions.is_match(node.sig.ident.to_string())
        {
            return;
        }
        let previous = self.function.replace(node.sig.ident.to_string());
        visit::visit_block(self, &node.block);
        self.function = previous;
    }

    fn visit_expr_if(&mut self, node: &'ast ExprIf) {
        self.add_condition(&node.cond);
        visit::visit_expr_if(self, node);
    }

    fn visit_expr_while(&mut self, node: &'ast ExprWhile) {
        self.add_condition(&node.cond);
        // Invariants, decreases, and ensures are spec/proof material attached
        // to an executable loop. The exec campaign mutates only the condition
        // and executable body.
        self.visit_expr(&node.cond);
        self.visit_block(&node.body);
    }

    fn visit_expr_binary(&mut self, node: &'ast ExprBinary) {
        match &node.op {
            BinOp::And(_) | BinOp::Or(_) if self.operators.logical_clause_deletion => {
                if let Some((start, end)) = byte_range(self.source, node.left.span()) {
                    self.add(
                        node.span(),
                        "delete-right-logical-clause",
                        self.source[start..end].into(),
                    );
                }
                if let Some((start, end)) = byte_range(self.source, node.right.span()) {
                    self.add(
                        node.span(),
                        "delete-left-logical-clause",
                        self.source[start..end].into(),
                    );
                }
            }
            _ => {}
        }
        if self.operators.relational_replacement {
            let replacement = match &node.op {
                BinOp::Lt(_) => Some("<="),
                BinOp::Le(_) => Some("<"),
                BinOp::Gt(_) => Some(">="),
                BinOp::Ge(_) => Some(">"),
                BinOp::Eq(_) => Some("!="),
                BinOp::Ne(_) => Some("=="),
                _ => None,
            };
            if let Some(replacement) = replacement {
                self.add(
                    node.op.span(),
                    "replace-relational-operator",
                    replacement.into(),
                );
            }
        }
        visit::visit_expr_binary(self, node);
    }

    fn visit_expr_unary(&mut self, node: &'ast ExprUnary) {
        if matches!(
            node.op,
            UnOp::Proof(_) | UnOp::Forall(_) | UnOp::Exists(_) | UnOp::Choose(_)
        ) {
            return;
        }
        visit::visit_expr_unary(self, node);
    }

    fn visit_expr_closure(&mut self, node: &'ast ExprClosure) {
        if node.proof_fn.is_none() {
            self.visit_expr(&node.body);
        }
    }

    fn visit_assert(&mut self, _node: &'ast Assert) {}

    fn visit_assert_forall(&mut self, _node: &'ast AssertForall) {}

    fn visit_assume(&mut self, _node: &'ast Assume) {}

    fn visit_reveal_hide(&mut self, _node: &'ast RevealHide) {}
}

fn byte_range(source: &str, span: Span) -> Option<(usize, usize)> {
    let start = byte_offset(source, span.start())?;
    let end = byte_offset(source, span.end())?;
    (start <= end).then_some((start, end))
}

fn byte_offset(source: &str, location: LineColumn) -> Option<usize> {
    if location.line == 0 {
        return None;
    }
    let line_start = if location.line == 1 {
        0
    } else {
        source.match_indices('\n').nth(location.line - 2)?.0 + 1
    };
    let offset = line_start.checked_add(location.column)?;
    source.is_char_boundary(offset).then_some(offset)
}

#[cfg(test)]
mod tests {
    use super::{build_globs, byte_offset, discover_file};
    use crate::cargo::WorkspacePackage;
    use crate::config::OperatorsConfig;
    use proc_macro2::LineColumn;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn line_columns_map_to_bytes() {
        let source = "one\ntwo\nthree";
        assert_eq!(
            byte_offset(source, LineColumn { line: 1, column: 1 }),
            Some(1)
        );
        assert_eq!(
            byte_offset(source, LineColumn { line: 2, column: 2 }),
            Some(6)
        );
    }

    #[test]
    fn automatic_discovery_mutates_exec_bodies_not_specs_or_proofs() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(
            directory.path().join("src/lib.rs"),
            r#"
use vstd::prelude::*;
verus! {
spec fn model(x: int) -> bool { x < 10 }
proof fn lemma(x: int) { assert(x < 20); }
fn ignored(x: i32) -> bool { x < 100 }
fn check(x: i32) -> bool
    requires x < 30,
{
    if x < 5 { x == 0 } else { false }
}
}
"#,
        )
        .unwrap();
        let package = WorkspacePackage {
            name: "fixture".into(),
            root: directory.path().to_path_buf(),
            source_roots: vec![directory.path().join("src")],
            is_verus: true,
        };
        let mut mutants = Vec::new();
        discover_file(
            directory.path(),
            &package,
            &PathBuf::from("src/lib.rs"),
            &OperatorsConfig::default(),
            &build_globs(&[]).unwrap(),
            &mut mutants,
        )
        .unwrap();
        assert!(!mutants.is_empty());
        assert!(mutants
            .iter()
            .all(|mutant| matches!(mutant.function.as_deref(), Some("check" | "ignored"))));
        assert!(mutants.iter().all(|mutant| mutant.start > 100));

        let mut excluded = Vec::new();
        discover_file(
            directory.path(),
            &package,
            &PathBuf::from("src/lib.rs"),
            &OperatorsConfig::default(),
            &build_globs(&["check".into(), "ignored".into()]).unwrap(),
            &mut excluded,
        )
        .unwrap();
        assert!(excluded.is_empty());
    }
}
