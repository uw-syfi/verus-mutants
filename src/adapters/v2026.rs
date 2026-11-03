//! Verus output of the 2026 releases (0.2026.07.18.3a4d30b and
//! 0.2026.08.30.b432e82 print the same diagnostics and summary lines): which
//! output is a failed proof obligation, a program Verus rejected before
//! verifying, or a clean run, and where a proof failed.

use crate::model::KillLocation;

/// Verus diagnostics that report a failed proof obligation in a well-typed
/// program, with the kind label each reports. A diagnostic matches when its
/// message contains the pattern; the first match gives the label. The texts
/// are from Verus's `vir` crate (`def.rs`, `ast_to_sst.rs`, `sst_to_air.rs`).
const PROOF_DIAGNOSTICS: &[(&str, &str)] = &[
    ("postcondition not satisfied", "postcondition"),
    ("precondition not satisfied", "precondition"),
    ("requires not satisfied", "precondition"),
    // Loop invariants ("invariant not satisfied before loop", "loop invariant
    // not satisfied").
    ("invariant not satisfied", "invariant"),
    // The invariant of an `open_atomic_invariant!` or
    // `open_local_invariant!` block.
    ("Cannot show invariant holds at end of block", "invariant"),
    // Logically atomic functions: "cannot show the atomic update was
    // committed", "cannot show atomic postcondition hold at end of block".
    ("cannot show the atomic update", "atomic"),
    ("cannot show atomic", "atomic"),
    ("assertion failed", "assertion"),
    ("assertion failure", "assertion"),
    ("decreases not satisfied", "decreases"),
    ("possible division by zero", "division"),
    ("possible arithmetic underflow/overflow", "arithmetic"),
    ("possible bit shift underflow/overflow", "arithmetic"),
    ("possible array index out of bounds", "bounds"),
    ("cannot show this call will not unwind", "unwind"),
    (
        "may fail to meet its declared type invariant",
        "type-invariant",
    ),
];

fn diagnostic(message: &str) -> Option<(&'static str, &'static str)> {
    PROOF_DIAGNOSTICS
        .iter()
        .find(|(pattern, _)| message.contains(pattern))
        .copied()
}

pub fn proof_failure(output: &str) -> Option<&'static str> {
    if let Some((pattern, _)) = diagnostic(output) {
        return Some(pattern);
    }
    // Verus prints a `verification results:: N verified, M errors` line only
    // after the program type-checked, so M > 0 is a proof failure even when the
    // diagnostic is one this list does not know.
    verification_errors(output)
        .filter(|errors| *errors > 0)
        .map(|_| "verification error")
}

/// Locates the first failed proof obligation in Verus output: the diagnostic
/// class and the `--> file:line:col` line that follows its `error:` header.
pub fn kill_location(output: &str) -> Option<KillLocation> {
    let mut lines = output.lines();
    while let Some(line) = lines.next() {
        let Some(message) = line.strip_prefix("error: ") else {
            continue;
        };
        let Some((_, kind)) = diagnostic(message) else {
            continue;
        };
        let Some(arrow) = lines
            .next()
            .and_then(|next| next.trim_start().strip_prefix("--> "))
        else {
            continue;
        };
        let mut parts = arrow.rsplitn(3, ':');
        let column = parts.next()?.trim().parse().ok()?;
        let line = parts.next()?.trim().parse().ok()?;
        let file = parts.next()?.to_owned();
        return Some(KillLocation {
            kind: kind.into(),
            file,
            line,
            column,
            from_ensures: kind == "postcondition",
        });
    }
    None
}

/// The largest error count among Verus's `verification results::` summary
/// lines. A command that verifies several crates prints one line per crate,
/// and the failing crate need not come first.
fn verification_errors(output: &str) -> Option<usize> {
    output
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("verification results:: ")?;
            let (_, errors) = rest.split_once(" verified, ")?;
            errors.split_whitespace().next()?.parse().ok()
        })
        .max()
}

pub fn looks_invalid(output: &str) -> bool {
    output.contains("error[E")
        || output.contains("mismatched types")
        || output.contains("expected ") && output.contains("found ")
        || output.contains("could not compile")
        || output.contains("The verifier does not yet support")
}

pub fn has_positive_verification(output: &str) -> bool {
    output.lines().any(|line| {
        let Some(rest) = line.strip_prefix("verification results:: ") else {
            return false;
        };
        let Some((verified, errors)) = rest.split_once(" verified, ") else {
            return false;
        };
        verified.parse::<usize>().is_ok_and(|count| count > 0) && errors.starts_with("0 errors")
    })
}

#[cfg(test)]
mod tests {
    use super::{has_positive_verification, kill_location};

    #[test]
    fn kill_location_reports_file_line_and_ensures() {
        let post = "note: x\n\nerror: postcondition not satisfied\n   --> crates/c/src/mgr_pages.rs:118:13\n    |\n118 |   final(self).rows(r) == 0,\n    |   ^^^ failed this postcondition\n...\n158 |   r\n    |   - at the end of the function body\n";
        let kill = kill_location(post).unwrap();
        assert_eq!(kill.file, "crates/c/src/mgr_pages.rs");
        assert_eq!((kill.line, kill.column), (118, 13));
        assert!(kill.from_ensures);
        assert_eq!(kill.kind, "postcondition");

        let body = "error: assertion failed\n   --> src/pool.rs:677:62\n";
        let kill = kill_location(body).unwrap();
        assert!(!kill.from_ensures);
        assert_eq!((kill.kind.as_str(), kill.line), ("assertion", 677));

        // Verus 0.2026.08.30 output for a mutant inside an invariant block.
        let block = "   Compiling app v0.0.0 (/w/source/app)\n\
                     error: Cannot show invariant holds at end of block\n    \
                     --> app/src/lib.rs:623:5\n     |\n\
                     623 | /     open_atomic_invariant!(q.inv.borrow() => g => {\n\n\
                     verification results:: 64 verified, 1 errors\n";
        let kill = kill_location(block).unwrap();
        assert_eq!(
            (kill.kind.as_str(), kill.line, kill.column),
            ("invariant", 623, 5)
        );
        assert_eq!(kill.file, "app/src/lib.rs");
        for (message, kind) in [
            ("loop invariant not satisfied", "invariant"),
            ("invariant not satisfied before loop", "invariant"),
            ("cannot show the atomic update was committed", "atomic"),
            (
                "cannot show atomic postcondition hold at end of block",
                "atomic",
            ),
            ("assertion failure", "assertion"),
            (
                "value may fail to meet its declared type invariant after assignment",
                "type-invariant",
            ),
        ] {
            let output = format!("error: {message}\n  --> src/a.rs:3:4\n");
            assert_eq!(
                kill_location(&output).map(|k| k.kind),
                Some(kind.to_owned()),
                "{message}"
            );
            assert!(super::proof_failure(&output).is_some(), "{message}");
        }

        // A compile error is not a located kill.
        assert!(kill_location("error[E0308]: mismatched types\n --> src/a.rs:1:1").is_none());
    }

    #[test]
    fn verification_summary_must_be_positive() {
        assert!(has_positive_verification(
            "verification results:: 3 verified, 0 errors"
        ));
        assert!(!has_positive_verification(
            "verification results:: 0 verified, 0 errors"
        ));
    }
}
