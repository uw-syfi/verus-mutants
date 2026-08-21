mod cargo;
mod config;
mod discover;
mod materialize;
mod model;
mod oracle;
mod report;
mod runner;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "cargo-verus-mutants", bin_name = "cargo verus-mutants")]
#[command(about = "Mutation analysis for Verus Cargo projects")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Discover mutants without executing their oracles.
    List(CommonArgs),
    /// Execute mutants. This is the default command.
    Run(RunArgs),
}

#[derive(Debug, clap::Args)]
struct CommonArgs {
    /// Workspace directory, Cargo.toml, or optional .verus-mutants.toml.
    #[arg(long, default_value = ".")]
    manifest_path: PathBuf,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct RunArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Run only manually configured mutants.
    #[arg(long, conflicts_with = "automatic_only")]
    manual_only: bool,
    /// Run only automatically discovered exec mutants.
    #[arg(long, conflicts_with = "manual_only")]
    automatic_only: bool,
    /// Stop after the first survivor or infrastructure failure.
    #[arg(long)]
    fail_fast: bool,
    /// Run at most this many mutants after deterministic sorting.
    #[arg(long)]
    limit: Option<usize>,
    /// Run only this mutant ID. May be repeated.
    #[arg(long = "mutant")]
    mutants: Vec<String>,
}

fn main() -> Result<()> {
    // Cargo invokes subcommands as `cargo-verus-mutants verus-mutants ...`.
    let mut args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == "verus-mutants") {
        args.remove(1);
    }
    let cli = Cli::parse_from(args);
    match cli.command.unwrap_or_else(|| {
        Command::Run(RunArgs {
            common: CommonArgs {
                manifest_path: PathBuf::from("."),
                json: false,
            },
            manual_only: false,
            automatic_only: false,
            fail_fast: false,
            limit: None,
            mutants: Vec::new(),
        })
    }) {
        Command::List(args) => runner::list(&args.manifest_path, args.json),
        Command::Run(args) => runner::run(
            &args.common.manifest_path,
            args.common.json,
            args.manual_only,
            args.automatic_only,
            args.fail_fast,
            args.limit,
            &args.mutants,
        ),
    }
}
