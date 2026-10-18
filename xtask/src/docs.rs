//! `cargo xtask docs`: the generated command-line reference, relative
//! links in every Markdown file, the length caps of `DOCUMENTATION.md`,
//! the layout of `AGENTS.md` against the modules under `src/`, and the
//! changelog's shape and its newest release against the crate version.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use clap::CommandFactory;
use regex::Regex;

const CLI_DOC: &str = "docs/cli.md";

/// The most non-blank lines a file may hold, as `DOCUMENTATION.md` caps
/// them; a file under `docs/` takes [`DOCS_CAP`], and the changelog has
/// none.
const LENGTH_CAPS: &[(&str, usize)] = &[
    ("AGENTS.md", 250),
    ("COMMENTS.md", 250),
    ("DOCUMENTATION.md", 250),
    ("README.md", 150),
];
const DOCS_CAP: usize = 400;

/// Keep a Changelog's headings, in the order a release lists them.
const CHANGELOG_SECTIONS: [&str; 6] = [
    "Added",
    "Changed",
    "Deprecated",
    "Removed",
    "Fixed",
    "Security",
];

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
        let text = std::fs::read_to_string(root.join(&file))
            .with_context(|| format!("reading {}", file.display()))?;
        problems.extend(broken_links(root, &file, &text));
        problems.extend(over_length(&file, &text));
    }
    let agents = std::fs::read_to_string(root.join("AGENTS.md")).context("reading AGENTS.md")?;
    problems.extend(layout_problems(&agents, &source_files(&root.join("src"))?));
    let manifest =
        std::fs::read_to_string(root.join("Cargo.toml")).context("reading Cargo.toml")?;
    let changelog =
        std::fs::read_to_string(root.join("CHANGELOG.md")).context("reading CHANGELOG.md")?;
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
    let lines: Vec<&str> = body.lines().collect();
    let mut fenced = false;
    for (i, line) in lines.iter().enumerate() {
        if fenced && line.trim().is_empty() {
            out.push_str("```\n");
            fenced = false;
        }
        if let Some(label) = line.strip_prefix("###### ") {
            // clap-markdown's "Options:" and "Subcommands:" labels.
            out.push_str(label);
        } else if line.starts_with('#') {
            out.push('#');
            out.push_str(line);
        } else if !fenced && is_help_block(line, lines.get(i + 1).copied()) {
            // An `after_help` table such as the exit codes: aligned text.
            out.push_str("```text\n");
            out.push_str(line);
            fenced = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if fenced {
        out.push_str("```\n");
    }
    format!("{}\n", out.trim_end())
}

/// Whether `line` opens a block of help text laid out in columns: a
/// label such as `Exit codes:` over indented lines.
fn is_help_block(line: &str, next: Option<&str>) -> bool {
    line.ends_with(':')
        && line.starts_with(|c: char| c.is_ascii_uppercase())
        && !line.contains("**")
        && next.is_some_and(|n| n.starts_with("  "))
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

/// The cap `file` falls under, if any.
fn length_cap(file: &Path) -> Option<usize> {
    let name = file.to_string_lossy().replace('\\', "/");
    LENGTH_CAPS
        .iter()
        .find(|(f, _)| *f == name)
        .map(|(_, cap)| *cap)
        .or_else(|| name.starts_with("docs/").then_some(DOCS_CAP))
}

fn over_length(file: &Path, text: &str) -> Option<String> {
    let cap = length_cap(file)?;
    let count = text.lines().filter(|l| !l.trim().is_empty()).count();
    (count > cap).then(|| {
        format!(
            "{}: {count} non-blank lines; the cap is {cap}, so split it",
            file.display()
        )
    })
}

/// Every `.rs` file under `dir`, by its path from there with `/`.
fn source_files(dir: &Path) -> Result<Vec<String>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = format!("{prefix}{name}");
            if entry.file_type()?.is_dir() {
                walk(&entry.path(), &format!("{path}/"), out)?;
            } else if Path::new(&name).extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out)?;
    out.sort();
    Ok(out)
}

/// The modules the layout block of `AGENTS.md` does not name. A file is
/// named by its file name alone, so `*/tests.rs` covers every module's
/// tests.
fn layout_problems(agents: &str, sources: &[String]) -> Vec<String> {
    let Some(block) = agents
        .split_once("\n## Layout\n")
        .and_then(|(_, rest)| rest.split_once("```text\n"))
        .and_then(|(_, rest)| rest.split_once("\n```"))
        .map(|(block, _)| block)
    else {
        return vec!["AGENTS.md has no ```text block under `## Layout`".into()];
    };
    sources
        .iter()
        .filter(|path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            !block.contains(name)
        })
        .map(|path| format!("AGENTS.md's layout does not name `src/{path}`"))
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
    out.extend(changelog_shape(changelog));
    out
}

/// Each release's `###` headings are Keep a Changelog's, once each and
/// in its order, and each bullet opens with a bold lead phrase.
fn changelog_shape(changelog: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut release = String::new();
    let mut last: Option<usize> = None;
    for (idx, line) in changelog.lines().enumerate() {
        let at = idx + 1;
        if let Some(name) = line.strip_prefix("## ") {
            release = name.to_string();
            last = None;
        } else if let Some(heading) = line.strip_prefix("### ") {
            match CHANGELOG_SECTIONS.iter().position(|s| *s == heading) {
                None => out.push(format!(
                    "CHANGELOG.md:{at}: `### {heading}` is not a Keep a Changelog heading"
                )),
                Some(i) if last.is_some_and(|l| l == i) => {
                    out.push(format!(
                        "CHANGELOG.md:{at}: `### {heading}` repeats in {release}"
                    ));
                }
                Some(i) if last.is_some_and(|l| l > i) => out.push(format!(
                    "CHANGELOG.md:{at}: `### {heading}` is out of order in {release}"
                )),
                Some(i) => last = Some(i),
            }
        } else if line.starts_with("- ") && !line.starts_with("- **") {
            out.push(format!(
                "CHANGELOG.md:{at}: a bullet opens with a bold lead phrase"
            ));
        }
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
        assert!(!text.contains("#######"), "{text}");
        assert!(text.contains("```text\nExit codes:"), "{text}");
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
    fn a_release_lists_each_heading_once_in_order_with_bold_bullets() {
        let ok = "## [Unreleased]\n\n### Added\n\n- **A.** x\n\n### Fixed\n\n- **B.** y\n";
        assert_eq!(changelog_shape(ok), [] as [std::string::String; 0]);
        let twice = "## [Unreleased]\n\n### Changed\n\n- **A.** x\n\n### Changed\n\n- **B.** y\n";
        assert_eq!(
            changelog_shape(twice).len(),
            1,
            "{:?}",
            changelog_shape(twice)
        );
        let order = "## [0.1.0]\n\n### Fixed\n\n- **A.** x\n\n### Added\n\n- **B.** y\n";
        assert!(changelog_shape(order)[0].contains("out of order"));
        assert!(changelog_shape("## [0.1.0]\n\n### Fixes\n")[0].contains("not a Keep"));
        assert!(changelog_shape("## [0.1.0]\n\n### Fixed\n\n- plain\n")[0].contains("bold"));
    }

    #[test]
    fn a_module_the_layout_leaves_out_is_named() {
        let agents = "# A\n\n## Layout\n\n```text\nsrc/\n  a.rs   one\n  */tests.rs  tests\n```\n";
        let sources = ["a.rs".to_string(), "a/tests.rs".into(), "b.rs".into()];
        assert_eq!(
            layout_problems(agents, &sources),
            ["AGENTS.md's layout does not name `src/b.rs`"]
        );
        assert_eq!(layout_problems("# A\n", &sources).len(), 1);
    }

    #[test]
    fn caps_fall_on_the_named_files_and_docs_but_not_the_changelog() {
        assert_eq!(length_cap(Path::new("README.md")), Some(150));
        assert_eq!(length_cap(Path::new("docs/guide.md")), Some(400));
        assert_eq!(length_cap(Path::new("CHANGELOG.md")), None);
        let text = "a\n\nb\nc\n";
        assert!(over_length(Path::new("README.md"), text).is_none());
        let long = "x\n".repeat(151);
        assert!(over_length(Path::new("README.md"), &long).is_some());
    }

    #[test]
    fn the_newest_release_must_be_the_crate_version() {
        let manifest = "[package]\nname = \"muman\"\nversion = \"0.2.0\"\n";
        let ok = "# Changelog\n\n## [Unreleased]\n\n## [0.2.0] - 2026-01-01\n";
        assert_eq!(
            changelog_problems(manifest, ok),
            [] as [std::string::String; 0]
        );
        let stale = "# Changelog\n\n## [Unreleased]\n\n## [0.1.0] - 2026-01-01\n";
        assert_eq!(changelog_problems(manifest, stale).len(), 1);
        assert_eq!(changelog_problems(manifest, "# Changelog\n").len(), 1);
    }
}
