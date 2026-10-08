use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Campaign {
    Exec,
    Rust,
    Manual,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OracleKind {
    Verus,
    Test,
    Command,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OracleSpec {
    pub kind: OracleKind,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub expected_pattern: Option<String>,
    #[serde(default)]
    pub invalid_pattern: Option<String>,
    #[serde(default)]
    pub required_test_count: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Mutant {
    pub id: String,
    pub campaign: Campaign,
    pub operator: String,
    pub package: String,
    pub file: PathBuf,
    pub function: Option<String>,
    pub start: usize,
    pub end: usize,
    pub original: String,
    pub replacement: String,
    pub expected_occurrences: usize,
    pub oracle: OracleSpec,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    KilledByProof,
    KilledByTest,
    KilledByPolicy,
    Survived,
    Invalid,
    Timeout,
    InfrastructureFailure,
}

/// Where Verus reported the failed obligation that killed a mutant.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct KillLocation {
    /// Diagnostic class: `postcondition`, `precondition`, `assertion`,
    /// `invariant`, `decreases`, `arithmetic`, `division`, `bounds`, or `other`.
    pub kind: String,
    /// File named by the first proof error, as printed by Verus.
    pub file: String,
    pub line: usize,
    pub column: usize,
    /// True when the failed obligation is an `ensures` clause (the error span
    /// is the postcondition), false for an in-body assert, invariant,
    /// precondition at a call, or arithmetic check.
    pub from_ensures: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MutantResult {
    pub mutant: Mutant,
    pub outcome: Outcome,
    pub command: Vec<String>,
    pub returncode: Option<i32>,
    pub elapsed_seconds: f64,
    pub diagnostic: Option<String>,
    /// Present for killed-by-proof results whose log has a located error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill: Option<KillLocation>,
    pub log: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunSummary {
    pub schema_version: u32,
    pub source_digest: String,
    pub results: Vec<MutantResult>,
}
