//! End-to-end redundancy campaign on `tests/fixtures/redundancy`, verified by
//! real Verus. Ignored by default because it needs a Verus toolchain: set
//! `VERUS_FIXTURE_VERIFY` to an executable that runs `cargo verus build "$@"`
//! in its working directory (for example a container wrapper), then run
//! `cargo test --test redundancy_e2e -- --ignored`.

use std::fs;
use std::path::Path;
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

#[test]
#[ignore = "needs a Verus toolchain (VERUS_FIXTURE_VERIFY)"]
fn redundancy_findings_and_non_findings() {
    let verify = std::env::var("VERUS_FIXTURE_VERIFY").expect("VERUS_FIXTURE_VERIFY");
    let project = tempfile::tempdir().unwrap();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/redundancy"),
        project.path(),
    );
    fs::write(
        project.path().join(".verus-mutants.toml"),
        format!(
            "[verification]\ncommand = [{verify:?}, \"-p\", \"{{package}}\"]\ntimeout_seconds = 900\n"
        ),
    )
    .unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_cargo-verus-mutants"))
        .args(["run", "--automatic-only", "--redundancy"])
        .args(["--operator", "drop-requires", "--operator", "drop-ensures"])
        .args(["--operator", "dead-refusal"])
        .current_dir(project.path())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "redundancy findings must not fail the run"
    );
    let summary: serde_json::Value = serde_json::from_slice(
        &fs::read(project.path().join("target/verus-mutants/summary.json")).unwrap(),
    )
    .unwrap();
    let mut verdicts: Vec<(String, String)> = summary["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let mutant = &r["mutant"];
            (
                format!(
                    "{}:{}",
                    mutant["function"].as_str().unwrap(),
                    mutant["detail"].as_str().unwrap()
                ),
                r["outcome"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    verdicts.sort();
    let outcome = |key: &str| {
        verdicts
            .iter()
            .find(|(name, _)| name == key)
            .unwrap_or_else(|| panic!("no mutant {key}: {verdicts:?}"))
            .1
            .as_str()
    };
    // Findings.
    assert_eq!(outcome("double_small:requires x < 4000000000"), "redundant");
    assert_eq!(outcome("double_small:ensures r % 2 == 0"), "redundant");
    assert_eq!(outcome("scaled_twice:ensures y == 4 * x"), "redundant");
    assert_eq!(outcome("checked_dead:if x >= 100"), "redundant");
    // Non-findings: the overflow check, callers' uses, a reachable refusal.
    assert_eq!(
        outcome("double_small:requires x < 2000000000"),
        "killed-by-proof"
    );
    assert_eq!(
        outcome("double_small:ensures r == x * 2"),
        "killed-by-proof"
    );
    assert_eq!(outcome("scaled:ensures y == 2 * x"), "killed-by-proof");
    assert_eq!(outcome("checked_dead:requires x < 100"), "killed-by-proof");
    assert_eq!(outcome("checked_live:if x >= 100"), "killed-by-proof");
}
