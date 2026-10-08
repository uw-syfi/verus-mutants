//! Accepted-survivor baseline: a ratchet over known surviving mutants.
//!
//! An entry names a mutant by a stable key that survives unrelated edits:
//! `(file, function, operator, replacement text)`. Line numbers, byte offsets
//! and the generated `VM-` id are not part of it. An entry may add `original`
//! to tell apart mutants that share the other four fields. Without it, an
//! entry covers every mutant with that key.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::model::{Mutant, MutantResult, Outcome};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// The survivor is believed equivalent to the original program.
    Equivalent,
    /// A known gap that someone owns and intends to close.
    Open,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub file: PathBuf,
    #[serde(default)]
    pub function: Option<String>,
    pub operator: String,
    pub replacement: String,
    #[serde(default)]
    pub original: Option<String>,
    pub status: Status,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    /// `[[mutant]]` in TOML, `"mutants": [...]` in JSON.
    #[serde(default, alias = "mutant")]
    mutants: Vec<Entry>,
}

impl Entry {
    pub fn matches(&self, mutant: &Mutant) -> bool {
        self.file == mutant.file
            && self.function == mutant.function
            && self.operator == mutant.operator
            && self.replacement == mutant.replacement
            && self
                .original
                .as_ref()
                .is_none_or(|original| original == &mutant.original)
    }

    fn describe(&self) -> String {
        format!(
            "{} {}::{} -> {:?}",
            self.operator,
            self.file.display(),
            self.function.as_deref().unwrap_or("<none>"),
            self.replacement
        )
    }
}

pub fn load(path: &Path) -> Result<Vec<Entry>> {
    let text =
        fs::read_to_string(path).with_context(|| format!("reading baseline {}", path.display()))?;
    let file: File = if path.extension().is_some_and(|ext| ext == "json") {
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
    } else {
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
    };
    let mut seen = BTreeSet::new();
    for entry in &file.mutants {
        match entry.status {
            Status::Equivalent => anyhow::ensure!(
                entry
                    .reason
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty()),
                "baseline entry {} is equivalent and needs a reason",
                entry.describe()
            ),
            Status::Open => anyhow::ensure!(
                entry
                    .owner
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty()),
                "baseline entry {} is open and needs an owner",
                entry.describe()
            ),
        }
        anyhow::ensure!(
            seen.insert((
                entry.file.clone(),
                entry.function.clone(),
                entry.operator.clone(),
                entry.replacement.clone(),
                entry.original.clone()
            )),
            "duplicate baseline entry {}",
            entry.describe()
        );
    }
    Ok(file.mutants)
}

#[derive(Debug, Default)]
pub struct Verdict {
    /// Survivors with no entry: new gaps.
    pub unlisted: Vec<MutantResult>,
    /// Entries whose mutants ran and none survived.
    pub no_longer_surviving: Vec<Entry>,
    /// Entries that match no discovered mutant.
    pub no_longer_exist: Vec<Entry>,
    pub accepted: usize,
}

impl Verdict {
    pub fn is_clean(&self) -> bool {
        self.unlisted.is_empty()
            && self.no_longer_surviving.is_empty()
            && self.no_longer_exist.is_empty()
    }
}

/// Compares a run against the baseline. `discovered` is every mutant before
/// scoping filters (`--file`, `--in-diff`, limits), so an entry outside the
/// current scope is neither stale nor missing; `results` is what ran.
pub fn evaluate(entries: &[Entry], discovered: &[Mutant], results: &[MutantResult]) -> Verdict {
    let mut verdict = Verdict::default();
    for result in results {
        if result.outcome == Outcome::Survived {
            if entries.iter().any(|entry| entry.matches(&result.mutant)) {
                verdict.accepted += 1;
            } else {
                verdict.unlisted.push(result.clone());
            }
        }
    }
    for entry in entries {
        if !discovered.iter().any(|mutant| entry.matches(mutant)) {
            verdict.no_longer_exist.push(entry.clone());
            continue;
        }
        let ran: Vec<_> = results
            .iter()
            .filter(|result| entry.matches(&result.mutant))
            .collect();
        if !ran.is_empty() && ran.iter().all(|result| result.outcome != Outcome::Survived) {
            verdict.no_longer_surviving.push(entry.clone());
        }
    }
    verdict
}

/// Renders an unlisted survivor as a ready-to-edit TOML entry.
pub fn suggest(result: &MutantResult) -> String {
    let mutant = &result.mutant;
    let mut text = String::from("[[mutant]]\n");
    text.push_str(&format!("file = {:?}\n", mutant.file.display().to_string()));
    if let Some(function) = &mutant.function {
        text.push_str(&format!("function = {function:?}\n"));
    }
    text.push_str(&format!("operator = {:?}\n", mutant.operator));
    text.push_str(&format!("replacement = {:?}\n", mutant.replacement));
    text.push_str("status = \"open\"\nowner = \"TODO\"\n");
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Campaign, OracleKind, OracleSpec};

    fn mutant(function: &str, operator: &str, replacement: &str, start: usize) -> Mutant {
        Mutant {
            id: format!("VM-{start}"),
            campaign: Campaign::Exec,
            operator: operator.into(),
            package: "p".into(),
            file: PathBuf::from("src/a.rs"),
            function: Some(function.into()),
            start,
            end: start + 1,
            original: "x".into(),
            replacement: replacement.into(),
            expected_occurrences: 1,
            detail: None,
            oracle: OracleSpec {
                kind: OracleKind::Verus,
                package: None,
                command: Vec::new(),
                expected_pattern: None,
                invalid_pattern: None,
                required_test_count: None,
            },
        }
    }

    fn result(mutant: &Mutant, outcome: Outcome) -> MutantResult {
        MutantResult {
            mutant: mutant.clone(),
            outcome,
            command: Vec::new(),
            returncode: None,
            elapsed_seconds: 0.0,
            diagnostic: None,
            kill: None,
            verified_packages: Vec::new(),
            log: PathBuf::new(),
        }
    }

    fn entry(function: &str, operator: &str, replacement: &str) -> Entry {
        Entry {
            file: PathBuf::from("src/a.rs"),
            function: Some(function.into()),
            operator: operator.into(),
            replacement: replacement.into(),
            original: None,
            status: Status::Equivalent,
            reason: Some("dead branch".into()),
            owner: None,
        }
    }

    #[test]
    fn ratchet_flags_new_survivors_and_stale_entries() {
        let kept = mutant("f", "condition-to-true", "true", 10);
        let fixed = mutant("g", "condition-to-false", "false", 20);
        let gone_elsewhere = mutant("h", "replace-integer-literal", "0", 30);
        let fresh = mutant("k", "condition-to-true", "true", 40);
        let discovered = vec![
            kept.clone(),
            fixed.clone(),
            gone_elsewhere.clone(),
            fresh.clone(),
        ];
        let entries = vec![
            entry("f", "condition-to-true", "true"),
            entry("g", "condition-to-false", "false"),
            entry("renamed", "condition-to-true", "true"),
            // Out of scope this run: exists but did not run, so not stale.
            entry("h", "replace-integer-literal", "0"),
        ];
        let results = vec![
            result(&kept, Outcome::Survived),
            result(&fixed, Outcome::KilledByProof),
            result(&fresh, Outcome::Survived),
        ];
        let verdict = evaluate(&entries, &discovered, &results);
        assert_eq!(verdict.accepted, 1);
        assert_eq!(verdict.unlisted.len(), 1);
        assert_eq!(verdict.unlisted[0].mutant.function.as_deref(), Some("k"));
        assert_eq!(verdict.no_longer_surviving.len(), 1);
        assert_eq!(
            verdict.no_longer_surviving[0].function.as_deref(),
            Some("g")
        );
        assert_eq!(verdict.no_longer_exist.len(), 1);
        assert_eq!(
            verdict.no_longer_exist[0].function.as_deref(),
            Some("renamed")
        );
        assert!(!verdict.is_clean());
    }

    #[test]
    fn key_ignores_position_and_clean_run_passes() {
        let moved = mutant("f", "condition-to-true", "true", 999);
        let entries = vec![entry("f", "condition-to-true", "true")];
        let verdict = evaluate(
            &entries,
            std::slice::from_ref(&moved),
            &[result(&moved, Outcome::Survived)],
        );
        assert!(verdict.is_clean());
        assert_eq!(verdict.accepted, 1);
    }

    #[test]
    fn loads_toml_and_json_and_validates_status() {
        let directory = tempfile::tempdir().unwrap();
        let toml_path = directory.path().join("accepted.toml");
        fs::write(
            &toml_path,
            r#"
[[mutant]]
file = "src/a.rs"
function = "f"
operator = "condition-to-true"
replacement = "true"
status = "equivalent"
reason = "branch is unreachable"

[[mutant]]
file = "src/a.rs"
function = "g"
operator = "replace-integer-literal"
replacement = "0"
status = "open"
owner = "alice"
"#,
        )
        .unwrap();
        assert_eq!(load(&toml_path).unwrap().len(), 2);

        let json_path = directory.path().join("accepted.json");
        fs::write(
            &json_path,
            r#"{"mutants":[{"file":"src/a.rs","function":"f","operator":"condition-to-true",
               "replacement":"true","status":"open","owner":"bob"}]}"#,
        )
        .unwrap();
        assert_eq!(load(&json_path).unwrap()[0].owner.as_deref(), Some("bob"));

        let bad = directory.path().join("bad.toml");
        fs::write(
            &bad,
            "[[mutant]]\nfile = \"a.rs\"\noperator = \"o\"\nreplacement = \"r\"\nstatus = \"equivalent\"\n",
        )
        .unwrap();
        assert!(load(&bad)
            .unwrap_err()
            .to_string()
            .contains("needs a reason"));
    }
}
