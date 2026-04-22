use clap::Parser;
use ginkgo_dashboard_lib::datagen::{run, GenConfig};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "datagen", about = "Synthetic GINKGO benchmark data generator")]
struct Cli {
    /// Output directory for generated files.
    #[arg(long, default_value = "fixtures")]
    out: PathBuf,

    /// Number of matrices to generate.
    #[arg(long, default_value_t = 200)]
    matrices: usize,

    /// Number of commit-series files (0 = single-file mode).
    /// Incompatible with --sha.
    #[arg(long, default_value_t = 0)]
    commits: usize,

    /// RNG seed for deterministic generation.
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// Log-normal multiplicative noise sigma applied to benchmark times.
    #[arg(long, default_value_t = 0.08)]
    noise_stddev: f64,

    /// Real git SHA to name the output file after. Enables single-commit mode.
    /// When provided, output is `<out>/bench_<sha7>.json` + manifest upsert.
    /// Incompatible with --commits > 0.
    #[arg(long)]
    sha: Option<String>,

    /// Commit author (for manifest, in "Name <email>" form).
    #[arg(long)]
    author: Option<String>,

    /// Commit date (ISO 8601).
    #[arg(long)]
    date: Option<String>,

    /// Commit subject/message line.
    #[arg(long)]
    message: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // --sha and --commits > 0 are mutually exclusive.
    if cli.sha.is_some() && cli.commits > 0 {
        return Err(
            "--sha and --commits are mutually exclusive: use --sha for single-commit mode \
             or --commits N for commit-series mode, not both"
                .into(),
        );
    }

    let config = GenConfig {
        matrices: cli.matrices,
        commits: cli.commits,
        seed: cli.seed,
        noise_stddev: cli.noise_stddev,
        out_dir: cli.out.clone(),
        commit_sha: cli.sha,
        commit_author: cli.author,
        commit_date: cli.date,
        commit_message: cli.message,
    };

    let n = run(&config)?;

    if let Some(sha) = &config.commit_sha {
        let short: String = sha.chars().take(7).collect();
        println!(
            "Wrote bench_{}.json + commits.json to {}",
            short,
            cli.out.display()
        );
    } else {
        println!(
            "Wrote {} benchmark file(s) + solver.json to {}",
            n,
            cli.out.display()
        );
    }

    Ok(())
}
