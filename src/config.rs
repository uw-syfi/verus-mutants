use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cargo_metadata::MetadataCommand;
use serde::Deserialize;

use crate::cargo::WorkspacePackage;
use crate::model::{Campaign, Mutant, OracleKind, OracleSpec};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub project: ProjectConfig,
    pub verification: VerificationConfig,
    pub operators: OperatorsConfig,
    pub operator_oracles: BTreeMap<String, ManualOracleConfig>,
    pub rust_mutants: RustMutantsConfig,
    #[serde(rename = "manual_mutant")]
    pub manual_mutants: Vec<ManualMutantConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    pub include_packages: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub exclude_functions: Vec<String>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            include_packages: Vec::new(),
            exclude_globs: vec!["target/".into(), "generated/".into()],
            exclude_functions: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct VerificationConfig {
    pub command: Vec<String>,
    pub baseline_command: Vec<String>,
    pub timeout_seconds: u64,
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            command: vec![
                "cargo".into(),
                "verus".into(),
                "build".into(),
                "-p".into(),
                "{package}".into(),
            ],
            baseline_command: Vec::new(),
            timeout_seconds: 240,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct OperatorsConfig {
    pub mutate_contracts: bool,
    pub mutate_spec_functions: bool,
    pub condition_to_true: bool,
    pub condition_to_false: bool,
    pub logical_clause_deletion: bool,
    pub relational_replacement: bool,
    pub boolean_literal_replacement: bool,
    pub integer_literal_replacement: bool,
    pub arithmetic_replacement: bool,
    pub statement_deletion: bool,
    pub struct_field_value_substitution: bool,
    pub match_arm_body_substitution: bool,
    pub external_body_insertion: bool,
    pub external_body_visibility_widening: bool,
}

impl Default for OperatorsConfig {
    fn default() -> Self {
        Self {
            mutate_contracts: false,
            mutate_spec_functions: false,
            condition_to_true: true,
            condition_to_false: true,
            logical_clause_deletion: true,
            relational_replacement: true,
            boolean_literal_replacement: true,
            integer_literal_replacement: true,
            arithmetic_replacement: true,
            statement_deletion: true,
            struct_field_value_substitution: true,
            match_arm_body_substitution: true,
            external_body_insertion: false,
            external_body_visibility_widening: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ManualMutantConfig {
    pub id: String,
    #[serde(default)]
    pub package: Option<String>,
    pub file: PathBuf,
    #[serde(rename = "replace")]
    pub original: String,
    #[serde(rename = "with")]
    pub replacement: String,
    #[serde(default = "one")]
    pub expected_occurrences: usize,
    #[serde(default)]
    pub oracle: Option<ManualOracleConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ManualOracleConfig {
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

impl ManualOracleConfig {
    pub fn to_spec(&self, default_package: &str) -> OracleSpec {
        OracleSpec {
            kind: self.kind.clone(),
            package: self
                .package
                .clone()
                .or_else(|| Some(default_package.to_owned())),
            command: self.command.clone(),
            expected_pattern: self.expected_pattern.clone(),
            invalid_pattern: self.invalid_pattern.clone(),
            required_test_count: self.required_test_count,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RustMutantsConfig {
    pub enabled: bool,
    pub inventory_command: Vec<String>,
    pub oracle: Option<ManualOracleConfig>,
}

fn one() -> usize {
    1
}

pub struct LoadedConfig {
    pub root: PathBuf,
    pub config: Config,
}

pub fn load(path: &Path) -> Result<LoadedConfig> {
    let (root, config_path) =
        if path.is_dir() || path.file_name().is_some_and(|n| n == "Cargo.toml") {
            let mut command = MetadataCommand::new();
            if path.is_dir() {
                command.current_dir(path);
            } else {
                command.manifest_path(path);
            }
            command.no_deps();
            let metadata = command.exec().context("discovering Cargo workspace root")?;
            let root = metadata.workspace_root.as_std_path().to_path_buf();
            let config_path = root.join(".verus-mutants.toml");
            (root, config_path)
        } else {
            let absolute = path
                .canonicalize()
                .context("canonicalizing mutation configuration")?;
            let root = absolute
                .parent()
                .context("mutation manifest has no parent")?
                .to_path_buf();
            (root, absolute)
        };
    if !config_path.is_file() {
        return Ok(LoadedConfig {
            root,
            config: Config::default(),
        });
    }
    let text = fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    let config =
        toml::from_str(&text).with_context(|| format!("parsing {}", config_path.display()))?;
    Ok(LoadedConfig { root, config })
}

impl ManualMutantConfig {
    pub fn into_mutant(self, root: &Path, packages: &[WorkspacePackage]) -> Result<Mutant> {
        let file = root.join(&self.file);
        let source = fs::read_to_string(&file)
            .with_context(|| format!("reading manual mutant target {}", file.display()))?;
        let count = source.matches(&self.original).count();
        anyhow::ensure!(
            count == self.expected_occurrences,
            "manual mutant {} expected {} occurrence(s), found {}",
            self.id,
            self.expected_occurrences,
            count
        );
        let start = source
            .find(&self.original)
            .context("validated occurrence disappeared")?;
        let package = match self.package {
            Some(package) => package,
            None => infer_package(&file, packages)?.name.clone(),
        };
        let oracle = self.oracle.unwrap_or(ManualOracleConfig {
            kind: OracleKind::Verus,
            package: None,
            command: Vec::new(),
            expected_pattern: None,
            invalid_pattern: None,
            required_test_count: None,
        });
        Ok(Mutant {
            id: self.id,
            campaign: Campaign::Manual,
            operator: "manual".into(),
            package,
            file: self.file,
            function: None,
            start,
            end: start + self.original.len(),
            original: self.original,
            replacement: self.replacement,
            expected_occurrences: self.expected_occurrences,
            oracle: OracleSpec {
                kind: oracle.kind,
                package: oracle.package,
                command: oracle.command,
                expected_pattern: oracle.expected_pattern,
                invalid_pattern: oracle.invalid_pattern,
                required_test_count: oracle.required_test_count,
            },
        })
    }
}

fn infer_package<'a>(
    file: &Path,
    packages: &'a [WorkspacePackage],
) -> Result<&'a WorkspacePackage> {
    let absolute = file
        .canonicalize()
        .with_context(|| format!("canonicalizing manual mutant target {}", file.display()))?;
    packages
        .iter()
        .filter(|package| absolute.starts_with(&package.root))
        .max_by_key(|package| package.root.components().count())
        .with_context(|| {
            format!(
                "cannot infer a Verus package for {}; set manual_mutant.package",
                file.display()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::load;
    use std::fs;

    #[test]
    fn member_directory_resolves_workspace_configuration_root() {
        let workspace = tempfile::tempdir().unwrap();
        let member = workspace.path().join("member");
        fs::create_dir_all(member.join("src")).unwrap();
        fs::write(
            workspace.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"member\"]\nresolver = \"2\"\n",
        )
        .unwrap();
        fs::write(
            member.join("Cargo.toml"),
            "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(member.join("src/lib.rs"), "verus! {}\n").unwrap();

        let loaded = load(&member).unwrap();
        assert_eq!(loaded.root, workspace.path().canonicalize().unwrap());
    }
}
