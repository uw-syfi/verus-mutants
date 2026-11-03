//! End-to-end redundancy campaign on `tests/fixtures/redundancy`, verified by
//! real Verus. Ignored by default; see `tests/common/mod.rs` for the
//! environment, then run `cargo test --test redundancy_e2e -- --ignored`.

mod common;

#[test]
#[ignore = "needs a Verus toolchain (VERUS_FIXTURE_VERIFY)"]
fn redundancy_findings_and_non_findings() {
    let project = common::fixture_project("redundancy");
    let (success, results) = common::run(
        project.path(),
        &[
            "--automatic-only",
            "--redundancy",
            "--operator",
            "drop-requires",
            "--operator",
            "drop-ensures",
            "--operator",
            "dead-refusal",
        ],
    );
    assert!(success, "redundancy findings must not fail the run");
    let outcome = |key: &str| {
        results
            .iter()
            .find(|(function, _, detail, _)| format!("{function}:{detail}") == key)
            .unwrap_or_else(|| panic!("no mutant {key}: {results:?}"))
            .3
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
