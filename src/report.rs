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
        println!("  {} killed by tests", count(Outcome::KilledByTest));
        println!("  {} killed by policy", count(Outcome::KilledByPolicy));
        println!("  {} survived", count(Outcome::Survived));
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
        println!("summary: {}", final_path.display());
    }
    Ok(())
}
