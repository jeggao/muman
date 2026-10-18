//! `cargo xtask commits`: the commit rules of `AGENTS.md` over the
//! commits a change adds, or over one message, as a pull request's title
//! and description are before a squash merge writes them to main.
//!
//! A subject is `<scope>: <imperative summary>`, lowercase, no period,
//! at most 72 characters; a ` (#123)` GitHub appends on a squash merge
//! is not counted. A body follows a blank line, wrapped at 72, and is
//! required: a change with nothing to say about why is rare, and the
//! bodies that were missing were the largest changes'. A body made of a
//! branch's commit subjects, `* scope: summary` lines, is refused: a
//! squash merge writes the description, so the description is written
//! as one body. Trailers (`Key: value` at the end) and lines holding a
//! URL may run long. Merge commits and commits by bots are not checked:
//! Dependabot's bodies are its own.

use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use regex::Regex;

const MAX_SUBJECT: usize = 72;
const MAX_BODY_LINE: usize = 72;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub rule: &'static str,
    pub message: String,
}

/// A commit, or a message standing for one.
#[derive(Debug, Clone)]
pub struct Commit {
    /// A short hash, or what the message is: `pull request`.
    pub name: String,
    pub author: String,
    pub message: String,
}

/// Check the commits `diff_base..HEAD` adds, HEAD alone without a base,
/// or the message in `message_file`. Returns how many problems were
/// printed.
pub fn run(
    root: &Path,
    diff_base: Option<&str>,
    message_file: Option<&Path>,
    annotations: bool,
) -> Result<usize> {
    let commits = match message_file {
        Some(file) => vec![Commit {
            name: "pull request".into(),
            author: String::new(),
            message: std::fs::read_to_string(file)
                .with_context(|| format!("reading {}", file.display()))?,
        }],
        None => commits(root, diff_base)?,
    };
    let mut total = 0;
    for commit in &commits {
        for p in check(commit) {
            total += 1;
            if annotations {
                println!(
                    "::error title=commits/{}::{}: {}",
                    p.rule,
                    commit.name,
                    p.message.replace('\n', "%0A")
                );
            } else {
                println!("{}: {}: {}", commit.name, p.rule, p.message);
            }
        }
    }
    if !annotations {
        println!("commits: {total} problem(s) in {} commit(s)", commits.len());
    }
    Ok(total)
}

/// The commits of `base..HEAD`, merges left out, or HEAD alone.
fn commits(root: &Path, base: Option<&str>) -> Result<Vec<Commit>> {
    let range = base.map_or_else(|| "HEAD".to_string(), |b| format!("{b}..HEAD"));
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(root)
        .args(["log", "--no-merges", "--format=%h%x00%an%x00%B%x1e"]);
    if base.is_none() {
        cmd.arg("-1");
    }
    let out = cmd
        .arg(&range)
        .output()
        .with_context(|| format!("git log {range}"))?;
    if !out.status.success() {
        bail!(
            "git log {range}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\x1e')
        .filter(|r| !r.trim().is_empty())
        .filter_map(|record| {
            let mut parts = record.trim_start_matches('\n').splitn(3, '\0');
            Some(Commit {
                name: parts.next()?.to_string(),
                author: parts.next()?.to_string(),
                message: parts.next()?.to_string(),
            })
        })
        .collect())
}

#[must_use]
pub fn check(commit: &Commit) -> Vec<Problem> {
    static SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" \(#\d+\)$").unwrap());
    static FORM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[a-z0-9][a-z0-9_./-]*(, [a-z0-9][a-z0-9_./-]*)*: [^A-Z\s]\S*( \S.*)?$")
            .unwrap()
    });
    static TRAILER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z-]*: \S").unwrap());
    static SQUASHED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\* [a-z0-9][a-z0-9_./-]*(, [a-z0-9_./-]+)*: ").unwrap());
    if commit.author.contains("[bot]") {
        return Vec::new();
    }
    let problem = |rule, message: String| Problem { rule, message };
    let mut out = Vec::new();
    let text = commit.message.replace("\r\n", "\n");
    let mut lines = text.lines();
    let subject = lines.next().unwrap_or_default().trim_end();
    let subject = SUFFIX.replace(subject, "");
    let len = subject.chars().count();
    if len > MAX_SUBJECT {
        out.push(problem(
            "subject-length",
            format!("the subject is {len} characters; the most is {MAX_SUBJECT}"),
        ));
    }
    if subject.ends_with('.') {
        out.push(problem(
            "subject-period",
            "the subject ends in a period".into(),
        ));
    }
    if !FORM.is_match(&subject) {
        out.push(problem(
            "subject-form",
            format!("`{subject}` does not read `<scope>: <imperative summary>`, lowercase"),
        ));
    }
    let rest: Vec<&str> = lines.collect();
    let body: Vec<&str> = rest
        .iter()
        .copied()
        .skip_while(|l| l.trim().is_empty())
        .collect();
    if !rest.is_empty() && !rest[0].trim().is_empty() {
        out.push(problem(
            "body-blank-line",
            "a blank line separates the subject from the body".into(),
        ));
    }
    let mut body_end = body.len();
    while body_end > 0
        && (body[body_end - 1].trim().is_empty() || TRAILER.is_match(body[body_end - 1]))
    {
        body_end -= 1;
    }
    if body_end == 0 {
        out.push(problem(
            "body-missing",
            "a body says why before what, then `-` bullets led by the file or area".into(),
        ));
    }
    for line in &body[..body_end] {
        let count = line.chars().count();
        if count > MAX_BODY_LINE && !line.contains("://") {
            out.push(problem(
                "body-wrap",
                format!("a body line is {count} characters; wrap at {MAX_BODY_LINE}: `{line}`"),
            ));
        }
        if SQUASHED.is_match(line) {
            out.push(problem(
                "body-squashed-subjects",
                format!("`{line}` lists a branch's commits; write the body as one"),
            ));
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(message: &str) -> Vec<&'static str> {
        check(&Commit {
            name: "abc1234".into(),
            author: "Someone".into(),
            message: message.into(),
        })
        .into_iter()
        .map(|p| p.rule)
        .collect()
    }

    #[test]
    fn a_well_formed_message_passes_with_its_squash_suffix_and_trailers() {
        let msg = "resolve: prefer the fuller source on a tie (#21)\n\n\
                   Album offers that rank alike were told apart by order alone.\n\n\
                   - resolve.rs: best_album ranks tied offers by their fields\n\n\
                   Co-Authored-By: Someone <someone@example.com>\n\
                   Claude-Session: https://example.com/session/a-long-identifier-that-runs-past-the-limit\n";
        assert_eq!(rules(msg), [] as [&str; 0]);
    }

    #[test]
    fn the_subject_has_one_shape() {
        assert_eq!(
            rules("Follow each format's standard\n\nA body.\n"),
            ["subject-form"]
        );
        assert_eq!(
            rules("docs: Install from crates.io\n\nWhy.\n"),
            ["subject-form"]
        );
        assert_eq!(
            rules("docs: install from crates.io.\n\nWhy.\n"),
            ["subject-period"]
        );
        assert_eq!(
            rules("loudness, attach: normalize every song\n\nWhy.\n"),
            [] as [&str; 0]
        );
        assert_eq!(rules("release: muman 0.4.1\n\nWhy.\n"), [] as [&str; 0]);
        let long = format!("tools: {}\n\nWhy.\n", "x".repeat(70));
        assert_eq!(rules(&long), ["subject-length"]);
    }

    #[test]
    fn a_body_is_required_blank_separated_and_wrapped() {
        assert_eq!(rules("docs: fix a link\n"), ["body-missing"]);
        assert_eq!(
            rules("docs: fix a link\n\nCo-Authored-By: Someone <s@example.com>\n"),
            ["body-missing"]
        );
        assert_eq!(rules("docs: fix a link\nWhy.\n"), ["body-blank-line"]);
        let wide = format!("docs: fix a link\n\n{}\n", "w".repeat(73));
        assert_eq!(rules(&wide), ["body-wrap"]);
        let url = format!(
            "docs: fix a link\n\nSee https://example.com/{}\n",
            "w".repeat(73)
        );
        assert_eq!(rules(&url), [] as [&str; 0]);
    }

    #[test]
    fn a_body_of_squashed_subjects_is_refused_once() {
        let msg = "sync: judge damaged audio\n\n\
                   * progress: show nothing of a step with nothing to do\n\n\
                   A step begun with no items printed a plain line.\n\n\
                   * settings: add a missing key without a hint\n";
        assert_eq!(rules(msg), ["body-squashed-subjects"]);
    }

    #[test]
    fn bots_are_not_checked() {
        let bot = Commit {
            name: "abc1234".into(),
            author: "dependabot[bot]".into(),
            message: "build(deps): bump x\n".into(),
        };
        assert_eq!(check(&bot), [] as [Problem; 0]);
    }
}
