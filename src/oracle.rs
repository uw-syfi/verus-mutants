use std::fs::{self, File};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tempfile::TempDir;
use wait_timeout::ChildExt;

use crate::config::VerificationConfig;
use crate::model::{KillLocation, Mutant, OracleKind, Outcome};

pub struct Execution {
    pub outcome: Outcome,
    pub command: Vec<String>,
    pub returncode: Option<i32>,
    pub elapsed_seconds: f64,
    pub output: String,
    pub diagnostic: Option<String>,
    pub kill: Option<KillLocation>,
}

/// Expands `{package}`, `{module}`, `{function}` and `{worker}` in a command
/// template for one mutant on one worker.
///
/// `{module}` is the Verus module path of the mutated file, derived from its
/// path below `src/` (`src/a/b.rs` and `src/a/b/mod.rs` give `a::b`). For
/// `lib.rs` and `main.rs` it is the package's crate name. `{function}` is the
/// mutated function, empty for manual mutants.
pub fn expand(template: &[String], mutant: &Mutant, worker: usize) -> Vec<String> {
    let package = mutant.oracle.package.as_deref().unwrap_or(&mutant.package);
    let module = module_path(mutant);
    let function = mutant.function.as_deref().unwrap_or("");
    let worker = worker.to_string();
    template
        .iter()
        .map(|part| {
            part.replace("{package}", package)
                .replace("{module}", &module)
                .replace("{function}", function)
                .replace("{worker}", &worker)
        })
        .collect()
}

fn module_path(mutant: &Mutant) -> String {
    let components: Vec<_> = mutant
        .file
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let below_src = match components.iter().rposition(|part| part == "src") {
        Some(index) => &components[index + 1..],
        None => &components[..],
    };
    let mut parts: Vec<String> = below_src.to_vec();
    if let Some(last) = parts.last_mut() {
        *last = last.strip_suffix(".rs").unwrap_or(last).to_owned();
    }
    if matches!(parts.last().map(String::as_str), Some("mod")) {
        parts.pop();
    }
    if parts.len() == 1 && matches!(parts[0].as_str(), "lib" | "main") {
        return mutant.package.replace('-', "_");
    }
    parts.join("::")
}

pub fn command_for(
    mutant: &Mutant,
    verification: &VerificationConfig,
    worker: usize,
) -> Result<Vec<String>> {
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
    Ok(expand(template, mutant, worker))
}

pub fn execute(
    source: &std::path::Path,
    target: &std::path::Path,
    mutant: &Mutant,
    verification: &VerificationConfig,
    worker: usize,
) -> Result<Execution> {
    let command = command_for(mutant, verification, worker)?;
    let raw = run_process(source, target, &command, verification.timeout_seconds)?;
    let (outcome, diagnostic) = classify(mutant, raw.returncode, raw.timed_out, &raw.output);
    let kill = (outcome == Outcome::KilledByProof)
        .then(|| kill_location(&raw.output))
        .flatten();
    Ok(Execution {
        outcome,
        kill,
        command,
        returncode: raw.returncode,
        elapsed_seconds: raw.elapsed_seconds,
        output: raw.output,
        diagnostic,
    })
}

/// Command template for redundancy operators.
pub fn redundancy_template(verification: &VerificationConfig) -> &[String] {
    if !verification.redundancy_command.is_empty() {
        &verification.redundancy_command
    } else if !verification.baseline_command.is_empty() {
        &verification.baseline_command
    } else {
        &verification.command
    }
}

pub fn redundancy_command_for(
    mutant: &Mutant,
    verification: &VerificationConfig,
    worker: usize,
    package: &str,
) -> Vec<String> {
    let mut scoped = mutant.clone();
    scoped.oracle.package = Some(package.to_owned());
    expand(redundancy_template(verification), &scoped, worker)
}

/// Verifies the mutated tree package by package (the defining crate first,
/// then its dependents), stopping at the first package that does not verify.
/// The mutant survives only if every package verifies, and the result lists the
/// packages that ran.
pub fn execute_redundancy(
    source: &std::path::Path,
    target: &std::path::Path,
    mutant: &Mutant,
    verification: &VerificationConfig,
    worker: usize,
    packages: &[String],
) -> Result<(Execution, Vec<String>)> {
    let mut output = String::new();
    let mut elapsed = 0.0;
    let mut ran = Vec::new();
    let mut first_command = Vec::new();
    for package in packages {
        let command = redundancy_command_for(mutant, verification, worker, package);
        let raw = run_process(source, target, &command, verification.timeout_seconds)?;
        elapsed += raw.elapsed_seconds;
        output.push_str(&format!(
            "==== {} ====\n{}\n",
            command.join(" "),
            raw.output
        ));
        ran.push(package.clone());
        if first_command.is_empty() {
            first_command = command;
        }
        let (outcome, diagnostic) = classify(mutant, raw.returncode, raw.timed_out, &raw.output);
        if outcome != Outcome::Survived {
            let kill = (outcome == Outcome::KilledByProof)
                .then(|| kill_location(&raw.output))
                .flatten();
            return Ok((
                Execution {
                    outcome,
                    kill,
                    command: first_command,
                    returncode: raw.returncode,
                    elapsed_seconds: elapsed,
                    output,
                    diagnostic,
                },
                ran,
            ));
        }
    }
    Ok((
        Execution {
            outcome: Outcome::Redundant,
            kill: None,
            command: first_command,
            returncode: Some(0),
            elapsed_seconds: elapsed,
            output,
            diagnostic: None,
        },
        ran,
    ))
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
            // A verification command may compose Verus with executable
            // positive witnesses. Their failure is a semantic kill, not an
            // infrastructure error.
            if output.contains("test result: FAILED") {
                (Outcome::KilledByTest, first_error(output))
            } else if let Some(kind) = proof_failure(output) {
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
                .invalid_pattern
                .as_ref()
                .is_some_and(|pattern| output.contains(pattern))
            {
                (Outcome::Invalid, mutant.oracle.invalid_pattern.clone())
            } else if mutant
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

/// Verus diagnostics that report a failed proof obligation in a well-typed
/// program. Order matters only for the reported label.
const PROOF_DIAGNOSTICS: &[&str] = &[
    "postcondition not satisfied",
    "precondition not satisfied",
    "requires not satisfied",
    "invariant not satisfied",
    "assertion failed",
    "decreases not satisfied",
    "possible division by zero",
    "possible arithmetic underflow/overflow",
    "possible bit shift underflow/overflow",
    "possible array index out of bounds",
    "constructed value may fail to meet its declared type invariant",
];

/// Kind label for each entry of `PROOF_DIAGNOSTICS`, in the same order.
const DIAGNOSTIC_KINDS: &[&str] = &[
    "postcondition",
    "precondition",
    "precondition",
    "invariant",
    "assertion",
    "decreases",
    "division",
    "arithmetic",
    "arithmetic",
    "bounds",
    "other",
];

fn proof_failure(output: &str) -> Option<&'static str> {
    if let Some(pattern) = PROOF_DIAGNOSTICS
        .iter()
        .find(|pattern| output.contains(**pattern))
    {
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
        let Some(kind) = PROOF_DIAGNOSTICS
            .iter()
            .position(|pattern| message.starts_with(pattern))
            .map(|index| DIAGNOSTIC_KINDS[index])
        else {
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

/// The error count from Verus's `verification results::` summary line.
fn verification_errors(output: &str) -> Option<usize> {
    output.lines().find_map(|line| {
        let rest = line.strip_prefix("verification results:: ")?;
        let (_, errors) = rest.split_once(" verified, ")?;
        errors.split_whitespace().next()?.parse().ok()
    })
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
    use super::{classify, expand, has_positive_verification, kill_location, passed_test_count};
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
            detail: None,
            oracle: OracleSpec {
                kind,
                package: None,
                command: Vec::new(),
                expected_pattern: None,
                invalid_pattern: None,
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
        let test = classify(
            &mutant(OracleKind::Verus),
            Some(101),
            false,
            "test result: FAILED. 1 passed; 1 failed",
        );
        assert_eq!(test.0, Outcome::KilledByTest);
        let compile = classify(
            &mutant(OracleKind::Verus),
            Some(101),
            false,
            "error[E0308]: mismatched types",
        );
        assert_eq!(compile.0, Outcome::Invalid);
    }

    #[test]
    fn arithmetic_and_requires_diagnostics_are_proof_kills() {
        for text in [
            "error: possible division by zero\n --> src/lib.rs:5:9",
            "error: requires not satisfied\n --> src/lib.rs:5:9",
            "error: possible arithmetic underflow/overflow",
            "error: possible array index out of bounds",
        ] {
            let result = classify(&mutant(OracleKind::Verus), Some(101), false, text);
            assert_eq!(result.0, Outcome::KilledByProof, "{text}");
        }
        // An unrecognized diagnostic is still a kill once the verification
        // summary reports errors, because type checking already succeeded.
        let result = classify(
            &mutant(OracleKind::Verus),
            Some(101),
            false,
            "error: some future diagnostic\nverification results:: 3 verified, 1 errors",
        );
        assert_eq!(result.0, Outcome::KilledByProof);
        let result = classify(
            &mutant(OracleKind::Verus),
            Some(101),
            false,
            "error[E0425]: cannot find value `g` in this scope",
        );
        assert_eq!(result.0, Outcome::Invalid);
    }

    #[test]
    fn command_placeholders_expand_per_mutant_and_worker() {
        let template: Vec<String> = [
            "env",
            "VOL=target-{worker}",
            "verify",
            "-p",
            "{package}",
            "--verify-module",
            "{module}",
            "--fn={function}",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let mut m = mutant(OracleKind::Verus);
        m.package = "my-crate".into();
        m.function = Some("run".into());
        for (file, module) in [
            ("crates/x/src/mgr_rc.rs", "mgr_rc"),
            ("crates/x/src/a/b.rs", "a::b"),
            ("crates/x/src/a/b/mod.rs", "a::b"),
            ("crates/x/src/lib.rs", "my_crate"),
        ] {
            m.file = PathBuf::from(file);
            let command = expand(&template, &m, 3);
            assert_eq!(
                command,
                [
                    "env",
                    "VOL=target-3",
                    "verify",
                    "-p",
                    "my-crate",
                    "--verify-module",
                    module,
                    "--fn=run"
                ],
                "{file}"
            );
        }
    }

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

    #[test]
    fn test_count_sums_across_cargo_targets() {
        let output = "test result: ok. 1 passed; 0 failed\n\
                      test result: ok. 0 passed; 0 failed\n";
        assert_eq!(passed_test_count(output), 1);
    }

    fn redundancy_fixture() -> (Mutant, crate::config::VerificationConfig) {
        let verification = crate::config::VerificationConfig {
            // Verifies every package except `bad`.
            command: vec![
                "sh".into(),
                "-c".into(),
                "if [ {package} = bad ]; then echo 'error: postcondition not satisfied'; exit 1; fi; \
                 echo 'verification results:: 1 verified, 0 errors'"
                    .into(),
            ],
            ..crate::config::VerificationConfig::default()
        };
        (mutant(OracleKind::Verus), verification)
    }

    #[test]
    fn redundancy_survives_only_if_every_package_verifies() {
        let (mutant, verification) = redundancy_fixture();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let all = ["a".to_string(), "b".to_string()];
        let (execution, ran) =
            super::execute_redundancy(dir.path(), &target, &mutant, &verification, 0, &all)
                .unwrap();
        assert_eq!(execution.outcome, Outcome::Redundant);
        assert_eq!(ran, all);
        let breaking = ["a".to_string(), "bad".to_string(), "never".to_string()];
        let (execution, ran) =
            super::execute_redundancy(dir.path(), &target, &mutant, &verification, 0, &breaking)
                .unwrap();
        assert_eq!(execution.outcome, Outcome::KilledByProof);
        assert_eq!(ran, ["a", "bad"], "stops at the first failing package");
    }
}
