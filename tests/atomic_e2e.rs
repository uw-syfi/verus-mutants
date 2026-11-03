//! End-to-end campaign on `tests/fixtures/atomic`: mutants inside an
//! `open_atomic_invariant!` block are generated, well-formed and judged by real
//! Verus. Ignored by default; see `tests/common/mod.rs` for the environment,
//! then run `cargo test --test atomic_e2e -- --ignored`.

mod common;

#[test]
#[ignore = "needs a Verus toolchain (VERUS_FIXTURE_VERIFY)"]
fn invariant_block_mutants_are_verified() {
    let project = common::fixture_project("atomic");
    let (_, results) = common::run(project.path(), &["--automatic-only"]);
    let got: Vec<(&str, &str, &str)> = results
        .iter()
        .map(|(_, operator, original, outcome)| {
            (operator.as_str(), original.as_str(), outcome.as_str())
        })
        .collect();
    let k = "killed-by-proof";
    assert_eq!(
        got,
        [
            ("condition-to-false", "x % 2 == 0", k),
            ("condition-to-true", "x % 2 == 0", k),
            // The invariant still holds when nothing is stored.
            (
                "delete-executable-statement",
                "self.cell.store(Tracked(&mut perm), y);",
                "survived",
            ),
            // `x - 1` is even for odd `x` too.
            ("replace-arithmetic-operator", "+", "survived"),
            ("replace-integer-literal", "0", k),
            ("replace-integer-literal", "1", k),
            ("replace-integer-literal", "2", k),
            ("replace-relational-operator", "==", k),
        ]
    );
}
