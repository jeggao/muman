//! Repository checks run by CI and before a commit: `cargo xtask
//! comments` and `cargo xtask docs`.

mod comments;
mod docs;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "Repository checks for muman")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// Check every Rust file against the rules of COMMENTS.md.
    Comments {
        /// Also apply the rules about what a change adds, against this
        /// git ref, such as `origin/main`.
        #[arg(long, value_name = "REF")]
        diff_base: Option<String>,

        /// Print GitHub Actions annotations instead of plain lines.
        #[arg(long)]
        github_annotations: bool,
    },
    /// Regenerate docs/cli.md, then check Markdown links and the changelog.
    Docs {
        /// Change nothing; fail when docs/cli.md is out of date.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // xtask lives one level below the repository root.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits in the repository")
        .to_path_buf();
    let found = match cli.command {
        Task::Comments {
            diff_base,
            github_annotations,
        } => comments::run(&root, diff_base.as_deref(), github_annotations),
        Task::Docs { check } => docs::run(&root, check).map(|problems| {
            for p in &problems {
                eprintln!("docs: {p}");
            }
            problems.len()
        }),
    };
    match found {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("xtask: {e:#}");
            ExitCode::from(2)
        }
    }
}
