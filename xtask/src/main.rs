//! Repository checks run by CI and before a commit: `cargo xtask
//! comments`, `cargo xtask commits`, `cargo xtask docs` and `cargo xtask
//! layers`.

// The rules on changing files and printing are muman's; these checks
// write only their own files and print their own findings.
#![expect(clippy::disallowed_methods, clippy::print_stdout, clippy::print_stderr)]

mod comments;
mod commits;
mod docs;
mod layers;

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
    /// Check commit messages against the rules of AGENTS.md: HEAD's, or
    /// every commit since a ref, or one message in a file. Only a broken
    /// subject in a message file fails; the rest is warned about.
    Commits {
        /// Check every commit this branch adds over this git ref, such as
        /// `origin/main`.
        #[arg(long, value_name = "REF", conflicts_with = "message_file")]
        diff_base: Option<String>,

        /// Check this file as a commit message: a pull request's title, a
        /// blank line and its description.
        #[arg(long, value_name = "FILE")]
        message_file: Option<PathBuf>,

        /// Who wrote the message in the file; a bot's is not checked.
        #[arg(
            long,
            value_name = "NAME",
            default_value = "",
            requires = "message_file"
        )]
        author: String,

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
    /// Check that no module names one of a higher layer than its own,
    /// but as xtask/src/layers.rs lists.
    Layers {
        /// Also print each upward reference, each module's fan-in and the
        /// cycles.
        #[arg(long)]
        report: bool,
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
        Task::Commits {
            diff_base,
            message_file,
            author,
            github_annotations,
        } => commits::run(
            &root,
            diff_base.as_deref(),
            message_file.as_deref(),
            &author,
            github_annotations,
        ),
        Task::Layers { report } => layers::run(&root, report),
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
