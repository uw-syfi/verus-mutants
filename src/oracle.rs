use std::fs::{self, File};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tempfile::TempDir;
use wait_timeout::ChildExt;

use crate::config::VerificationConfig;
use crate::model::{Mutant, OracleKind, Outcome};

pub struct Execution {
    pub outcome: Outcome,
    pub command: Vec<String>,
    pub returncode: Option<i32>,
    pub elapsed_seconds: f64,
    pub output: String,
    pub diagnostic: Option<String>,
}

pub fn command_for(mutant: &Mutant, verification: &VerificationConfig) -> Result<Vec<String>> {
    let template = if mutant.oracle.command.is_empty() {
        &verification.command
    } else {
        &mutant.oracle.command
    };
    anyhow::ensure!(
        !template.is_empty(),
        "mutant {} has an empty oracle command",
        mutant.id
    );
    let package = mutant.oracle.package.as_deref().unwrap_or(&mutant.package);
    Ok(template
        .iter()
        .map(|part| part.replace("{package}", package))
        .collect())
}

pub fn execute(
    source: &std::path::Path,
    target: &std::path::Path,
    mutant: &Mutant,
    verification: &VerificationConfig,
) -> Result<Execution> {
    let command = command_for(mutant, verification)?;
    let raw = run_process(source, target, &command, verification.timeout_seconds)?;
    let (outcome, diagnostic) = classify(mutant, raw.returncode, raw.timed_out, &raw.output);
    Ok(Execution {
        outcome,
        command,
        returncode: raw.returncode,
        elapsed_seconds: raw.elapsed_seconds,
        output: raw.output,
        diagnostic,
    })
}

pub fn baseline(
    source: &std::path::Path,
    target: &std::path::Path,
    command: &[String],
    timeout_seconds: u64,
    kind: &OracleKind,
    required_test_count: Option<usize>,
) -> Result<String> {
    let raw = run_process(source, target, command, timeout_seconds)?;
    anyhow::ensure!(!raw.timed_out, "baseline timed out: {}", command.join(" "));
    anyhow::ensure!(
        raw.returncode == Some(0),
        "baseline failed: {}\n{}",
        command.join(" "),
        tail(&raw.output, 4000)
    );
    if kind == &OracleKind::Verus {
        anyhow::ensure!(
            has_positive_verification(&raw.output),
            "Verus baseline completed without a positive verification summary: {}",
            command.join(" ")
        );
    }
    if kind == &OracleKind::Test {
        let passed = passed_test_count(&raw.output);
        let required = required_test_count.unwrap_or(1);
        anyhow::ensure!(
            passed >= required,
            "test baseline passed {passed} tests, expected at least {required}"
        );
    }
    Ok(raw.output)
}

struct RawExecution {
    returncode: Option<i32>,
    timed_out: bool,
    elapsed_seconds: f64,
    output: String,
}

fn run_process(
    source: &std::path::Path,
    target: &std::path::Path,
    command: &[String],
    timeout_seconds: u64,
) -> Result<RawExecution> {
    fs::create_dir_all(target)?;
    let capture = TempDir::new().context("creating command-output directory")?;
    let output_path = capture.path().join("combined.log");
    let stdout = File::create(&output_path)?;
    let stderr = stdout.try_clone()?;
    let started = Instant::now();
    let mut child = Command::new(&command[0])
        .args(&command[1..])
        .current_dir(source)
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_TERM_COLOR", "never")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| format!("spawning {}", command.join(" ")))?;
    let timeout = Duration::from_secs(timeout_seconds);
    let status = child.wait_timeout(timeout)?;
    let (returncode, timed_out) = match status {
        Some(status) => (status.code(), false),
        None => {
            child.kill().ok();
            let status = child.wait()?;
            (status.code(), true)
        }
    };
    let output = fs::read_to_string(output_path)?;
    Ok(RawExecution {
        returncode,
        timed_out,
        elapsed_seconds: started.elapsed().as_secs_f64(),
        output,
    })
}

fn classify(
    mutant: &Mutant,
    returncode: Option<i32>,
    timed_out: bool,
    output: &str,
) -> (Outcome, Option<String>) {
    if timed_out {
        return (Outcome::Timeout, Some("oracle timed out".into()));
    }
    if returncode == Some(0) {
        return (Outcome::Survived, None);
    }
    match mutant.oracle.kind {
        OracleKind::Verus => {
            if let Some(kind) = proof_failure(output) {
                (Outcome::KilledByProof, Some(kind.into()))
            } else if looks_invalid(output) {
                (Outcome::Invalid, first_error(output))
            } else {
                (Outcome::InfrastructureFailure, first_error(output))
            }
        }
        OracleKind::Test => {
            if output.contains("running 0 tests") {
                (
                    Outcome::Invalid,
                    Some("test oracle selected zero tests".into()),
                )
            } else if output.contains("test result: FAILED") {
                (Outcome::KilledByTest, first_error(output))
            } else {
                (Outcome::InfrastructureFailure, first_error(output))
            }
        }
        OracleKind::Command => {
            if mutant
                .oracle
                .expected_pattern
                .as_ref()
                .is_some_and(|pattern| output.contains(pattern))
            {
                (
                    Outcome::KilledByPolicy,
                    mutant.oracle.expected_pattern.clone(),
                )
            } else {
                (Outcome::InfrastructureFailure, first_error(output))
            }
        }
    }
}

fn proof_failure(output: &str) -> Option<&'static str> {
    [
        "postcondition not satisfied",
        "precondition not satisfied",
        "invariant not satisfied",
        "assertion failed",
        "decreases not satisfied",
        "constructed value may fail to meet its declared type invariant",
    ]
    .into_iter()
    .find(|pattern| output.contains(pattern))
}

fn looks_invalid(output: &str) -> bool {
    output.contains("error[E")
        || output.contains("mismatched types")
        || output.contains("expected ") && output.contains("found ")
        || output.contains("could not compile")
        || output.contains("The verifier does not yet support")
}

fn has_positive_verification(output: &str) -> bool {
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

fn passed_test_count(output: &str) -> usize {
    output
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("test result: ok. ")?;
            rest.split_once(" passed")?.0.parse::<usize>().ok()
        })
        .sum()
}

fn first_error(output: &str) -> Option<String> {
    output
        .lines()
        .find(|line| line.trim_start().starts_with("error"))
        .map(str::to_owned)
}

fn tail(text: &str, bytes: usize) -> &str {
    if text.len() <= bytes {
        text
    } else {
        let mut start = text.len() - bytes;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        &text[start..]
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, has_positive_verification, passed_test_count};
    use crate::model::{Campaign, Mutant, OracleKind, OracleSpec, Outcome};
    use std::path::PathBuf;

    fn mutant(kind: OracleKind) -> Mutant {
        Mutant {
            id: "M".into(),
            campaign: Campaign::Exec,
            operator: "test".into(),
            package: "p".into(),
            file: PathBuf::from("src/lib.rs"),
            function: None,
            start: 0,
            end: 1,
            original: "a".into(),
            replacement: "b".into(),
            expected_occurrences: 1,
            oracle: OracleSpec {
                kind,
                package: None,
                command: Vec::new(),
                expected_pattern: None,
                required_test_count: None,
            },
        }
    }

    #[test]
    fn proof_failures_are_kills_but_compile_failures_are_invalid() {
        let proof = classify(
            &mutant(OracleKind::Verus),
            Some(101),
            false,
            "error: postcondition not satisfied",
        );
        assert_eq!(proof.0, Outcome::KilledByProof);
        let compile = classify(
            &mutant(OracleKind::Verus),
            Some(101),
            false,
            "error[E0308]: mismatched types",
        );
        assert_eq!(compile.0, Outcome::Invalid);
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

    #[test]
    fn test_count_sums_across_cargo_targets() {
        let output = "test result: ok. 1 passed; 0 failed\n\
                      test result: ok. 0 passed; 0 failed\n";
        assert_eq!(passed_test_count(output), 1);
    }
}
