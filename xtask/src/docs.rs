//! `cargo xtask docs`: the generated command-line reference, relative
//! links in every Markdown file, and the changelog's newest release
//! against the crate version.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use clap::CommandFactory;
use regex::Regex;

const CLI_DOC: &str = "docs/cli.md";

const CLI_HEADER: &str = "\
# Command-line reference

<!-- Generated from the clap definitions in src/cli.rs by `cargo xtask docs`; edit those, not this file. -->

";

/// Regenerate the reference, or with `check` only compare it, then run
/// the link and changelog checks. Returns the problems found.
pub fn run(root: &Path, check: bool) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    let path = root.join(CLI_DOC);
    let want = cli_reference();
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    if have.replace("\r\n", "\n") != want {
        if check {
            problems.push(format!("{CLI_DOC} is out of date; run `cargo xtask docs`"));
        } else {
            std::fs::write(&path, &want).with_context(|| format!("writing {CLI_DOC}"))?;
        }
    }
    for file in markdown_files(root)? {
        let text = std::fs::read_to_string(root.join(&file))?;
        problems.extend(broken_links(root, &file, &text));
    }
    let manifest = std::fs::read_to_string(root.join("Cargo.toml"))?;
    let changelog = std::fs::read_to_string(root.join("CHANGELOG.md"))?;
    problems.extend(changelog_problems(&manifest, &changelog));
    Ok(problems)
}

fn cli_reference() -> String {
    let options = clap_markdown::MarkdownOptions::new()
        .show_footer(false)
        .show_table_of_contents(false);
    let raw = clap_markdown::help_markdown_command_custom(&muman::cli::Cli::command(), &options);
    // clap-markdown opens with its own `#` title; ours replaces it, and
    // every heading below moves down a level.
    let body = raw
        .split_once("\n## ")
        .map_or(raw.as_str(), |(_, rest)| rest);
    let mut out = String::from(CLI_HEADER);
    out.push_str("## ");
    for line in body.split_inclusive('\n') {
        if line.starts_with('#') {
            out.push('#');
        }
        out.push_str(line);
    }
    format!("{}\n", out.trim_end())
}

/// Tracked and new Markdown files, as git lists them.
fn markdown_files(root: &Path) -> Result<Vec<PathBuf>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "*.md",
        ])
        .output()
        .context("running git ls-files")?;
    if !out.status.success() {
        bail!("git ls-files failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(PathBuf::from)
        .collect())
}

fn broken_links(root: &Path, file: &Path, text: &str) -> Vec<String> {
    static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?ms)^```.*?^```").unwrap());
    static SPAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]*`").unwrap());
    static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\]\(([^)\s]+)\)").unwrap());
    let prose = SPAN
        .replace_all(&FENCE.replace_all(text, ""), "")
        .into_owned();
    let dir = root.join(file.parent().unwrap_or(Path::new("")));
    LINK.captures_iter(&prose)
        .filter_map(|caps| {
            let target = &caps[1];
            if target.contains("://") || target.starts_with("mailto:") || target.starts_with('#') {
                return None;
            }
            let path = target.split('#').next().unwrap_or(target);
            (!dir.join(path).exists())
                .then(|| format!("{}: link to missing `{target}`", file.display()))
        })
        .collect()
}

fn changelog_problems(manifest: &str, changelog: &str) -> Vec<String> {
    static VERSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?m)^version\s*=\s*"([^"]+)""#).unwrap());
    static RELEASE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^## \[(\d+\.\d+\.\d+[^\]]*)\]").unwrap());
    let mut out = Vec::new();
    if !changelog.contains("\n## [Unreleased]") {
        out.push("CHANGELOG.md has no `## [Unreleased]` section".into());
    }
    let version = VERSION.captures(manifest).map(|c| c[1].to_string());
    let newest = RELEASE.captures(changelog).map(|c| c[1].to_string());
    if let (Some(version), Some(newest)) = (version, newest)
        && version != newest
    {
        out.push(format!(
            "Cargo.toml is version {version} but CHANGELOG.md's newest release is {newest}"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_names_every_subcommand() {
        let text = cli_reference();
        assert!(text.starts_with("# Command-line reference"));
        assert!(text.contains("## `muman`"), "{text}");
        assert!(text.contains("### `muman sync`"), "{text}");
    }

    #[test]
    fn links_inside_code_are_not_checked_and_web_links_are_skipped() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let text = "[ok](Cargo.toml) [web](https://x.example) `[no](nope)`\n```\n[no](nope)\n```\n[bad](missing.md#a)\n";
        assert_eq!(
            broken_links(root, Path::new("x.md"), text),
            ["x.md: link to missing `missing.md#a`"]
        );
    }

    #[test]
    fn the_newest_release_must_be_the_crate_version() {
        let manifest = "[package]\nname = \"muman\"\nversion = \"0.2.0\"\n";
        let ok = "# Changelog\n\n## [Unreleased]\n\n## [0.2.0] - 2026-01-01\n";
        assert!(changelog_problems(manifest, ok).is_empty());
        let stale = "# Changelog\n\n## [Unreleased]\n\n## [0.1.0] - 2026-01-01\n";
        assert_eq!(changelog_problems(manifest, stale).len(), 1);
        assert_eq!(changelog_problems(manifest, "# Changelog\n").len(), 1);
    }
}
