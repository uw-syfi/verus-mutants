use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use walkdir::WalkDir;

use crate::cargo;
use crate::config;
use crate::discover;
use crate::materialize;
use crate::model::{Mutant, MutantResult, Outcome, RunSummary};
use crate::oracle;
use crate::report;

pub fn list(manifest: &Path, json: bool) -> Result<()> {
    let loaded = config::load(manifest)?;
    let mutants = discover_all(&loaded, false, false)?;
    report::print_list(&mutants, json)
}

pub struct RunOptions<'a> {
    pub manifest: &'a Path,
    pub json: bool,
    pub manual_only: bool,
    pub automatic_only: bool,
    pub fail_fast: bool,
    pub limit: Option<usize>,
    pub selected_ids: &'a [String],
    pub selected_operators: &'a [String],
    pub limit_per_operator: Option<usize>,
}

pub fn run(options: RunOptions<'_>) -> Result<()> {
    let RunOptions {
        manifest,
        json,
        manual_only,
        automatic_only,
        fail_fast,
        limit,
        selected_ids,
        selected_operators,
        limit_per_operator,
    } = options;
    let loaded = config::load(manifest)?;
    let root = &loaded.root;
    let mut mutants = discover_all(&loaded, manual_only, automatic_only)?;
    if !selected_ids.is_empty() {
        let selected: BTreeSet<_> = selected_ids.iter().collect();
        mutants.retain(|mutant| selected.contains(&mutant.id));
        let found: BTreeSet<_> = mutants.iter().map(|mutant| &mutant.id).collect();
        let missing: Vec<_> = selected.difference(&found).collect();
        anyhow::ensure!(
            missing.is_empty(),
            "selected mutant IDs were not found: {missing:?}"
        );
    }
    if !selected_operators.is_empty() {
        let selected: BTreeSet<_> = selected_operators.iter().collect();
        mutants.retain(|mutant| selected.contains(&mutant.operator));
        let found: BTreeSet<_> = mutants.iter().map(|mutant| &mutant.operator).collect();
        let missing: Vec<_> = selected.difference(&found).collect();
        anyhow::ensure!(
            missing.is_empty(),
            "selected automatic operators were not found: {missing:?}"
        );
    }
    if let Some(per_operator) = limit_per_operator {
        let mut retained = BTreeMap::new();
        mutants.retain(|mutant| {
            let count = retained
                .entry((mutant.package.clone(), mutant.operator.clone()))
                .or_insert(0usize);
            if *count >= per_operator {
                false
            } else {
                *count += 1;
                true
            }
        });
    }
    if let Some(limit) = limit {
        mutants.truncate(limit);
    }
    anyhow::ensure!(!mutants.is_empty(), "no mutants discovered");
    let digest = source_digest(root)?;
    let output_root = root.join("target/verus-mutants");
    let log_root = output_root.join("logs");
    fs::create_dir_all(&log_root)?;
    let sandbox = TempDir::new().context("creating campaign sandbox")?;
    let source = sandbox.path().join("source");
    let target = sandbox.path().join("target");
    materialize::copy_project(root, &source)?;

    let commands = unique_commands(&mutants, &loaded.config.verification)?;
    for (key, baseline) in &commands {
        let command = &baseline.command;
        eprintln!("baseline: {}", command.join(" "));
        let output = oracle::baseline(
            &source,
            &target,
            command,
            loaded.config.verification.timeout_seconds,
            &baseline.kind,
            baseline.required_test_count,
        )?;
        fs::write(log_root.join(format!("baseline-{key}.log")), output)?;
    }

    let mut results = Vec::new();
    for mutant in mutants {
        eprintln!("mutant {}: {}", mutant.id, mutant.operator);
        let mutated_path = source.join(&mutant.file);
        let original_source = fs::read(&mutated_path)
            .with_context(|| format!("reading {} before mutation", mutated_path.display()))?;
        materialize::apply(&source, &mutant)?;
        let execution = oracle::execute(&source, &target, &mutant, &loaded.config.verification);
        fs::write(&mutated_path, original_source)
            .with_context(|| format!("restoring {} after mutation", mutated_path.display()))?;
        let execution = execution?;
        let log = PathBuf::from(format!("target/verus-mutants/logs/{}.log", mutant.id));
        fs::write(root.join(&log), &execution.output)?;
        eprintln!(
            "  {:?} ({:.2}s)",
            execution.outcome, execution.elapsed_seconds
        );
        let stop = matches!(
            execution.outcome,
            Outcome::Survived | Outcome::Timeout | Outcome::InfrastructureFailure
        );
        results.push(MutantResult {
            mutant,
            outcome: execution.outcome,
            command: execution.command,
            returncode: execution.returncode,
            elapsed_seconds: execution.elapsed_seconds,
            diagnostic: execution.diagnostic,
            log,
        });
        if fail_fast && stop {
            break;
        }
    }
    let summary = RunSummary {
        schema_version: 1,
        source_digest: digest,
        results,
    };
    report::publish(root, &summary, json)?;
    if summary
        .results
        .iter()
        .any(|result| result.outcome == Outcome::InfrastructureFailure)
    {
        anyhow::bail!("one or more mutation oracles failed for infrastructure reasons");
    }
    if summary
        .results
        .iter()
        .any(|result| matches!(result.outcome, Outcome::Survived | Outcome::Timeout))
    {
        anyhow::bail!("one or more mutants survived or timed out");
    }
    Ok(())
}

fn discover_all(
    loaded: &config::LoadedConfig,
    manual_only: bool,
    automatic_only: bool,
) -> Result<Vec<Mutant>> {
    let mut mutants = Vec::new();
    let needs_packages = !manual_only
        || loaded
            .config
            .manual_mutants
            .iter()
            .any(|mutant| mutant.package.is_none());
    let packages = if needs_packages {
        cargo::workspace_packages(&loaded.root)?
    } else {
        Vec::new()
    };
    if !manual_only {
        let verified: Vec<_> = packages
            .iter()
            .filter(|package| package.is_verus)
            .cloned()
            .collect();
        anyhow::ensure!(
            !verified.is_empty(),
            "found no Verus workspace packages; expected package.metadata.verus.verify = true or a verus! macro"
        );
        mutants.extend(discover::automatic_mutants(
            &loaded.root,
            &verified,
            &loaded.config,
        )?);
    }
    if !automatic_only {
        for configured in loaded.config.manual_mutants.clone() {
            mutants.push(configured.into_mutant(&loaded.root, &packages)?);
        }
    }
    mutants.sort_by(|a, b| a.id.cmp(&b.id));
    let mut ids = BTreeSet::new();
    for mutant in &mutants {
        anyhow::ensure!(ids.insert(&mutant.id), "duplicate mutant id {}", mutant.id);
    }
    Ok(mutants)
}

struct Baseline {
    command: Vec<String>,
    kind: crate::model::OracleKind,
    required_test_count: Option<usize>,
}

fn unique_commands(
    mutants: &[Mutant],
    verification: &config::VerificationConfig,
) -> Result<BTreeMap<String, Baseline>> {
    let mut commands = BTreeMap::new();
    for mutant in mutants {
        let command = oracle::command_for(mutant, verification)?;
        let encoded = serde_json::to_vec(&(
            &command,
            &mutant.oracle.kind,
            mutant.oracle.required_test_count,
        ))?;
        let key = hex::encode(Sha256::digest(encoded));
        commands.entry(key[..12].to_string()).or_insert(Baseline {
            command,
            kind: mutant.oracle.kind.clone(),
            required_test_count: mutant.oracle.required_test_count,
        });
    }
    Ok(commands)
}

fn source_digest(root: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !digest_excluded(root, entry.path()))
    {
        let entry = entry?;
        let relative = entry.path().strip_prefix(root)?;
        if !entry.file_type().is_file() {
            continue;
        }
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(entry.path())?);
        hasher.update([0]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn digest_excluded(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        relative.components().any(|part| {
            let part = part.as_os_str();
            part == ".git" || part == "target" || part == ".verus-mutants"
        })
    })
}

#[cfg(test)]
mod tests {
    use super::discover_all;
    use crate::config;
    use crate::model::{Campaign, OracleKind};
    use std::fs;

    fn project() -> tempfile::TempDir {
        let project = tempfile::tempdir().unwrap();
        fs::create_dir(project.path().join("src")).unwrap();
        fs::write(
            project.path().join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            project.path().join("src/lib.rs"),
            "verus! { fn checked(x: i32) -> bool { if x > 0 { true } else { false } } }\n",
        )
        .unwrap();
        project
    }

    #[test]
    fn zero_config_discovers_exec_mutants() {
        let project = project();
        let loaded = config::load(project.path()).unwrap();
        let mutants = discover_all(&loaded, false, false).unwrap();

        assert!(!mutants.is_empty());
        assert!(mutants.iter().all(|mutant| {
            mutant.campaign == Campaign::Exec
                && mutant.package == "fixture"
                && mutant.oracle.kind == OracleKind::Verus
        }));
        assert_eq!(
            loaded.config.verification.command,
            ["cargo", "verus", "build", "-p", "{package}"]
        );
    }

    #[test]
    fn manual_mutant_infers_package_and_verus_oracle() {
        let project = project();
        fs::write(
            project.path().join(".verus-mutants.toml"),
            r#"
[[manual_mutant]]
id = "M-TRUE"
file = "src/lib.rs"
replace = "x > 0"
with = "true"
"#,
        )
        .unwrap();
        let loaded = config::load(project.path()).unwrap();
        let mutants = discover_all(&loaded, true, false).unwrap();

        assert_eq!(mutants.len(), 1);
        assert_eq!(mutants[0].package, "fixture");
        assert_eq!(mutants[0].oracle.kind, OracleKind::Verus);
    }
}
