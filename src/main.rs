mod cargo;
mod config;
mod discover;
mod materialize;
mod model;
mod oracle;
mod report;
mod runner;
mod rust;

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
    /// Run only this automatic operator. May be repeated.
    #[arg(long = "operator")]
    operators: Vec<String>,
    /// Retain at most this many mutants per package/operator pair.
    #[arg(long)]
    limit_per_operator: Option<usize>,
    /// Do not apply --limit-per-operator to this operator. May be repeated.
    #[arg(long = "exhaustive-operator")]
    exhaustive_operators: Vec<String>,
    /// Accept survivors when the killed/(killed+survived) ratio meets this value.
    #[arg(long)]
    minimum_kill_rate: Option<f64>,
    /// Require every selected mutant for this operator to be killed. May be repeated.
    #[arg(long = "require-zero-survivors-for")]
    zero_survivor_operators: Vec<String>,
    /// Mutate only files changed since the merge base with this Git revision.
    #[arg(long)]
    in_diff: Option<String>,
    /// Number of isolated mutation workers.
    #[arg(long, default_value_t = 1)]
    jobs: usize,
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
            operators: Vec::new(),
            limit_per_operator: None,
            exhaustive_operators: Vec::new(),
            minimum_kill_rate: None,
            zero_survivor_operators: Vec::new(),
            in_diff: None,
            jobs: 1,
        })
    }) {
        Command::List(args) => runner::list(&args.manifest_path, args.json),
        Command::Run(args) => runner::run(runner::RunOptions {
            manifest: &args.common.manifest_path,
            json: args.common.json,
            manual_only: args.manual_only,
            automatic_only: args.automatic_only,
            fail_fast: args.fail_fast,
            limit: args.limit,
            selected_ids: &args.mutants,
            selected_operators: &args.operators,
            limit_per_operator: args.limit_per_operator,
            exhaustive_operators: &args.exhaustive_operators,
            minimum_kill_rate: args.minimum_kill_rate,
            zero_survivor_operators: &args.zero_survivor_operators,
            in_diff: args.in_diff.as_deref(),
            jobs: args.jobs,
        }),
    }
}
