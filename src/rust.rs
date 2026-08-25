use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::RustMutantsConfig;
use crate::model::{Campaign, Mutant};

#[derive(Deserialize)]
struct CargoMutant {
    file: PathBuf,
    function: Option<CargoFunction>,
    genre: String,
    name: String,
    package: String,
    replacement: String,
    span: CargoSpan,
}

#[derive(Deserialize)]
struct CargoFunction {
    function_name: String,
}

#[derive(Deserialize)]
struct CargoSpan {
    start: CargoPosition,
    end: CargoPosition,
}

#[derive(Deserialize)]
struct CargoPosition {
    line: usize,
    column: usize,
}

pub fn automatic_mutants(root: &Path, config: &RustMutantsConfig) -> Result<Vec<Mutant>> {
    if !config.enabled {
        return Ok(Vec::new());
    }
    anyhow::ensure!(
        !config.inventory_command.is_empty(),
        "rust_mutants.inventory_command is required when Rust mutation discovery is enabled"
    );
    let oracle = config
        .oracle
        .as_ref()
        .context("rust_mutants.oracle is required when Rust mutation discovery is enabled")?;
    let output = Command::new(&config.inventory_command[0])
        .args(&config.inventory_command[1..])
        .current_dir(root)
        .output()
        .with_context(|| {
            format!(
                "running Rust mutation inventory: {}",
                config.inventory_command.join(" ")
            )
        })?;
    anyhow::ensure!(
        output.status.success(),
        "Rust mutation inventory failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inventory: Vec<CargoMutant> =
        serde_json::from_slice(&output.stdout).context("parsing cargo-mutants JSON inventory")?;
    inventory
        .into_iter()
        .map(|candidate| into_mutant(root, candidate, oracle))
        .collect()
}

fn into_mutant(
    root: &Path,
    candidate: CargoMutant,
    oracle: &crate::config::ManualOracleConfig,
) -> Result<Mutant> {
    let source = fs::read_to_string(root.join(&candidate.file))
        .with_context(|| format!("reading Rust mutant target {}", candidate.file.display()))?;
    let start = byte_offset(&source, &candidate.span.start)
        .with_context(|| format!("invalid start span for {}", candidate.name))?;
    let end = byte_offset(&source, &candidate.span.end)
        .with_context(|| format!("invalid end span for {}", candidate.name))?;
    anyhow::ensure!(
        start < end,
        "empty Rust mutation span for {}",
        candidate.name
    );
    let original = source[start..end].to_owned();
    let identity = format!(
        "rust\0{}\0{}\0{}\0{}\0{}",
        candidate.package,
        candidate.file.display(),
        candidate.genre,
        start,
        candidate.replacement
    );
    Ok(Mutant {
        id: format!(
            "VMR-{}",
            &hex::encode(Sha256::digest(identity.as_bytes()))[..16]
        ),
        campaign: Campaign::Rust,
        operator: format!("rust-{}", candidate.genre.to_ascii_lowercase()),
        package: candidate.package.clone(),
        file: candidate.file,
        function: candidate.function.map(|function| function.function_name),
        start,
        end,
        original,
        replacement: candidate.replacement,
        expected_occurrences: 1,
        oracle: oracle.to_spec(&candidate.package),
    })
}

fn byte_offset(source: &str, position: &CargoPosition) -> Option<usize> {
    if position.line == 0 || position.column == 0 {
        return None;
    }
    let line_start = if position.line == 1 {
        0
    } else {
        source.match_indices('\n').nth(position.line - 2)?.0 + 1
    };
    let offset = line_start.checked_add(position.column - 1)?;
    source.is_char_boundary(offset).then_some(offset)
}

#[cfg(test)]
mod tests {
    use super::{byte_offset, CargoPosition};

    #[test]
    fn cargo_mutants_positions_are_one_based() {
        let source = "one\n  two\n";
        assert_eq!(
            byte_offset(source, &CargoPosition { line: 2, column: 3 }),
            Some(6)
        );
    }
}
