use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::model::{Mutant, Outcome, RunSummary};

pub fn print_list(mutants: &[Mutant], json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(mutants)?);
    } else {
        for mutant in mutants {
            println!(
                "{}  {:<31} {}::{}  {}",
                mutant.id,
                mutant.operator,
                mutant.package,
                mutant.function.as_deref().unwrap_or("<manual>"),
                mutant.file.display()
            );
        }
        println!("{} mutants", mutants.len());
    }
    Ok(())
}

pub fn publish(root: &Path, summary: &RunSummary, json_stdout: bool) -> Result<()> {
    let output = root.join("target/verus-mutants");
    fs::create_dir_all(&output)?;
    let final_path = output.join("summary.json");
    let temporary = output.join(".summary.json.tmp");
    let mut file = fs::File::create(&temporary)?;
    serde_json::to_writer_pretty(&mut file, summary)?;
    writeln!(file)?;
    file.sync_all()?;
    fs::rename(&temporary, &final_path).context("atomically publishing mutation summary")?;
    if json_stdout {
        println!("{}", serde_json::to_string_pretty(summary)?);
    } else {
        let count = |outcome| {
            summary
                .results
                .iter()
                .filter(|result| result.outcome == outcome)
                .count()
        };
        println!("\n{} mutants", summary.results.len());
        println!("  {} killed by Verus", count(Outcome::KilledByProof));
        let kills: Vec<_> = summary
            .results
            .iter()
            .filter_map(|result| result.kill.as_ref())
            .collect();
        if !kills.is_empty() {
            let ensures = kills.iter().filter(|kill| kill.from_ensures).count();
            println!(
                "    ({ensures} at an ensures clause, {} in-body)",
                kills.len() - ensures
            );
        }
        println!("  {} killed by tests", count(Outcome::KilledByTest));
        println!("  {} killed by policy", count(Outcome::KilledByPolicy));
        println!("  {} survived", count(Outcome::Survived));
        println!(
            "  {} redundant (findings, not failures)",
            count(Outcome::Redundant)
        );
        println!("  {} invalid", count(Outcome::Invalid));
        println!("  {} timed out", count(Outcome::Timeout));
        println!(
            "  {} infrastructure failures",
            count(Outcome::InfrastructureFailure)
        );
        let killed = count(Outcome::KilledByProof)
            + count(Outcome::KilledByTest)
            + count(Outcome::KilledByPolicy);
        let survived = count(Outcome::Survived);
        if killed + survived > 0 {
            println!(
                "  {:.1}% kill rate (invalid mutants excluded)",
                100.0 * killed as f64 / (killed + survived) as f64
            );
        }
        print_section("Survivors", summary, Outcome::Survived);
        print_section("Redundant", summary, Outcome::Redundant);
        println!("summary: {}", final_path.display());
    }
    Ok(())
}

/// One line per result with this outcome, naming the function, the clause or
/// branch (for redundancy operators), the file, and the packages verified.
fn print_section(title: &str, summary: &RunSummary, outcome: Outcome) {
    let rows: Vec<_> = summary
        .results
        .iter()
        .filter(|result| result.outcome == outcome)
        .collect();
    if rows.is_empty() {
        return;
    }
    println!("\n{title} ({}):", rows.len());
    for result in rows {
        let mutant = &result.mutant;
        let subject = mutant
            .detail
            .clone()
            .unwrap_or_else(|| format!("{} -> {}", mutant.original, mutant.replacement));
        println!(
            "  {} {}::{} [{}] {}",
            mutant.operator,
            mutant.file.display(),
            mutant.function.as_deref().unwrap_or("<manual>"),
            mutant.id,
            subject
        );
        if !result.verified_packages.is_empty() {
            println!("    re-verified: {}", result.verified_packages.join(", "));
        }
    }
}
