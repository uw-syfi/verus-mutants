//! Helpers for the end-to-end tests that run real Verus. They are ignored by
//! default. Set `VERUS_FIXTURE_VERIFY` to an executable that runs
//! `cargo verus build "$@"` in its working directory (for example a container
//! wrapper). The fixtures depend on Verus 0.2026.07.18's `vstd`; set
//! `VERUS_FIXTURE_VSTD` to another `vstd` dependency value (a TOML value such
//! as `"=0.0.0-2026-08-30-0159"`) to run them against another release.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn manifests(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            manifests(&path, found);
        } else if path.file_name().is_some_and(|name| name == "Cargo.toml") {
            found.push(path);
        }
    }
}

/// Copies `tests/fixtures/<name>` into a temporary project whose
/// `.verus-mutants.toml` verifies with `VERUS_FIXTURE_VERIFY -p {package}`.
pub fn fixture_project(name: &str) -> tempfile::TempDir {
    let verify = std::env::var("VERUS_FIXTURE_VERIFY").expect("VERUS_FIXTURE_VERIFY");
    let project = tempfile::tempdir().unwrap();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
        project.path(),
    );
    if let Ok(vstd) = std::env::var("VERUS_FIXTURE_VSTD") {
        let mut found = Vec::new();
        manifests(project.path(), &mut found);
        for manifest in found {
            let text = fs::read_to_string(&manifest).unwrap();
            let text: Vec<String> = text
                .lines()
                .map(|line| {
                    if line.starts_with("vstd = ") {
                        format!("vstd = {vstd}")
                    } else {
                        line.to_owned()
                    }
                })
                .collect();
            fs::write(&manifest, text.join("\n") + "\n").unwrap();
        }
    }
    fs::write(
        project.path().join(".verus-mutants.toml"),
        format!(
            "[verification]\ncommand = [{verify:?}, \"-p\", \"{{package}}\"]\ntimeout_seconds = 900\n"
        ),
    )
    .unwrap();
    project
}

/// Runs the tool in `project` and returns whether it succeeded and the
/// `(function, operator, original, outcome)` of every result.
pub fn run(project: &Path, args: &[&str]) -> (bool, Vec<(String, String, String, String)>) {
    let status = Command::new(env!("CARGO_BIN_EXE_cargo-verus-mutants"))
        .arg("run")
        .args(args)
        .current_dir(project)
        .status()
        .unwrap();
    let summary: serde_json::Value = serde_json::from_slice(
        &fs::read(project.join("target/verus-mutants/summary.json")).unwrap(),
    )
    .unwrap();
    let text = |value: &serde_json::Value| value.as_str().unwrap_or_default().to_owned();
    let mut results: Vec<_> = summary["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let mutant = &r["mutant"];
            let label = match mutant["detail"].as_str() {
                Some(detail) => detail.to_owned(),
                None => text(&mutant["original"]),
            };
            (
                text(&mutant["function"]),
                text(&mutant["operator"]),
                label,
                text(&r["outcome"]),
            )
        })
        .collect();
    results.sort();
    (status.success(), results)
}
