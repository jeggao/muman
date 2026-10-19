//! Which lines a change touched, and the file as it was, from git.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

/// Diff context for a single file: which lines changed in the working copy
/// relative to the diff base, and the file content at the base (for
/// density-delta).
#[derive(Debug, Clone, Default)]
pub struct DiffInfo {
    pub changed_lines: Option<HashSet<usize>>,
    pub base_file: Option<String>,
    /// The change's hunks, which place a line of the new file in the base.
    pub hunks: Vec<Hunk>,
}

/// One hunk of a `--unified=0` diff: the base's lines it replaces and the
/// new file's lines it puts in their place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
}

impl DiffInfo {
    /// The base's lines a line of the new file stands where: those its
    /// hunk replaced, or the one it was before lines were added or taken
    /// out above it. Comparing line numbers alone would look at whatever
    /// the shift brought there.
    #[must_use]
    pub fn base_span(&self, line: usize) -> (usize, usize) {
        let mut shift: isize = 0;
        for h in &self.hunks {
            if line < h.new_start {
                break;
            }
            if line < h.new_start + h.new_count {
                return (h.old_start, h.old_start + h.old_count.max(1) - 1);
            }
            shift += h.old_count.cast_signed() - h.new_count.cast_signed();
        }
        let at = (line.cast_signed() + shift).max(1).cast_unsigned();
        (at, at)
    }
}

/// Build per-file diff info by shelling out to `git`: the file as the
/// working copy holds it, which is what the rules read, against the
/// commit the branch left `base` at. Diffing a commit instead would number
/// lines the working copy has since moved.
pub fn for_file(repo_root: &Path, base: &str, path: &Path) -> Result<DiffInfo> {
    let rel = path
        .strip_prefix(repo_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let from = merge_base(repo_root, base)?;
    let hunks = hunks(repo_root, &from, &rel)?;
    let base_file = base_blob(repo_root, &from, &rel)?;
    Ok(DiffInfo {
        changed_lines: Some(changed_of(&hunks)),
        base_file,
        hunks,
    })
}

/// The commit `HEAD` branched from `base` at, or `base` itself when git
/// finds none.
fn merge_base(repo_root: &Path, base: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("merge-base")
        .arg(base)
        .arg("HEAD")
        .output()
        .with_context(|| format!("git merge-base {base} HEAD"))?;
    Ok(if output.status.success() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        base.to_string()
    })
}

/// The hunks that make the working copy's `rel` of the one at `from`.
fn hunks(repo_root: &Path, from: &str, rel: &str) -> Result<Vec<Hunk>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("diff")
        .arg("--no-color")
        .arg("--unified=0")
        .arg(from)
        .arg("--")
        .arg(rel)
        .output()
        .with_context(|| format!("git diff {from} -- {rel}"))?;
    // If the file is untracked or git fails, fall back to all-changed (empty
    // set); we treat the file as fully new in the worktree below.
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_hunks(&String::from_utf8_lossy(&output.stdout)))
}

/// Parse `git diff --unified=0` output and return the set of line numbers in
/// the *new* (post-image) file that were added or modified. Pure removals
/// have no line in the new file and produce no entries.
#[cfg(test)]
#[must_use]
pub fn parse_unified_diff(diff: &str) -> HashSet<usize> {
    changed_of(&parse_hunks(diff))
}

/// The lines of the new file the hunks add or modify.
fn changed_of(hunks: &[Hunk]) -> HashSet<usize> {
    hunks
        .iter()
        .flat_map(|h| h.new_start..h.new_start + h.new_count)
        .collect()
}

/// The hunks of `git diff --unified=0` output, in order.
#[must_use]
pub fn parse_hunks(diff: &str) -> Vec<Hunk> {
    // Hunk header form: `@@ -<oldStart>[,<oldCount>] +<newStart>[,<newCount>] @@`.
    let range = |token: &str| -> Option<(usize, usize)> {
        let (start, count) = token.split_once(',').unwrap_or((token, "1"));
        Some((start.parse().ok()?, count.parse().ok()?))
    };
    diff.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("@@ ")?;
            let header = &rest[..rest.find(" @@")?];
            let mut tokens = header.split_whitespace();
            let (old_start, old_count) = range(tokens.find_map(|t| t.strip_prefix('-'))?)?;
            let (new_start, new_count) = range(tokens.find_map(|t| t.strip_prefix('+'))?)?;
            Some(Hunk {
                old_start,
                old_count,
                new_start,
                new_count,
            })
        })
        .collect()
}

fn base_blob(repo_root: &Path, from: &str, rel: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("show")
        .arg(format!("{from}:{rel}"))
        .output()
        .with_context(|| format!("git show {from}:{rel}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_added_lines() {
        let diff = "\
@@ -10,0 +11,2 @@ context
+added one
+added two
@@ -20,1 +22,1 @@ ctx
-old
+new
";
        let mut got: Vec<usize> = parse_unified_diff(diff).into_iter().collect();
        got.sort_unstable();
        assert_eq!(got, vec![11, 12, 22]);
    }

    #[test]
    fn a_line_is_placed_in_the_base_past_what_was_added_above_it() {
        let diff = "\
@@ -3,0 +4,5 @@
+five
+lines
+added
+above
+it
@@ -10,2 +15,1 @@
-two
-old
+one new
";
        let info = DiffInfo {
            hunks: parse_hunks(diff),
            ..DiffInfo::default()
        };
        assert_eq!(info.base_span(2), (2, 2), "above every hunk");
        assert_eq!(info.base_span(5), (3, 3), "added after base line 3");
        assert_eq!(info.base_span(12), (7, 7), "shifted down by five");
        assert_eq!(info.base_span(15), (10, 11), "replacing two");
        assert_eq!(
            info.base_span(20),
            (16, 16),
            "five added, one more taken out"
        );
    }

    /// Run git in `dir` with an identity of its own, on any platform's
    /// line endings alike.
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=Marlo Venn",
                "-c",
                "user.email=marlo@example.invalid",
                "-c",
                "core.autocrlf=false",
            ])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn lines_are_numbered_as_the_working_copy_holds_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("a.rs");
        git(root, &["init", "-q"]);
        std::fs::write(&file, "fn a() {}\n// why\nfn b() {}\n").unwrap();
        git(root, &["add", "a.rs"]);
        git(root, &["commit", "-q", "-m", "base"]);
        git(root, &["tag", "base"]);
        std::fs::write(&file, "fn a() {}\n// why\nfn b() {}\nfn c() {}\n").unwrap();
        git(root, &["commit", "-q", "-am", "head"]);
        std::fs::write(
            &file,
            "fn x() {}\nfn y() {}\nfn a() {}\n// why\nfn b() {}\nfn c() {}\n",
        )
        .unwrap();
        let info = for_file(root, "base", Path::new("a.rs")).unwrap();
        let mut changed: Vec<usize> = info.changed_lines.clone().unwrap().into_iter().collect();
        changed.sort_unstable();
        assert_eq!(changed, [1, 2, 6], "the comment, moved down, is not one");
        assert_eq!(info.base_span(4), (2, 2));
    }

    #[test]
    fn ignores_pure_removals() {
        let diff = "@@ -5,2 +4,0 @@\n-removed one\n-removed two\n";
        assert!(parse_unified_diff(diff).is_empty());
    }

    #[test]
    fn empty_diff_yields_empty_set() {
        assert!(parse_unified_diff("").is_empty());
    }

    #[test]
    fn handles_count_default_of_one() {
        let diff = "@@ -10 +11 @@\n-old\n+new\n";
        let got: HashSet<usize> = parse_unified_diff(diff);
        assert_eq!(got, [11usize].into_iter().collect());
    }

    #[test]
    fn handles_multiple_hunks() {
        let diff = "\
@@ -1,1 +1,1 @@
-a
+a'
@@ -10,0 +11,3 @@
+x
+y
+z
@@ -50,1 +60,1 @@
-old
+new
";
        let mut got: Vec<usize> = parse_unified_diff(diff).into_iter().collect();
        got.sort_unstable();
        assert_eq!(got, vec![1, 11, 12, 13, 60]);
    }

    #[test]
    fn ignores_diff_header_lines() {
        let diff = "\
diff --git a/foo.nix b/foo.nix
index abc..def 100644
--- a/foo.nix
+++ b/foo.nix
@@ -1 +1 @@
-old
+new
";
        let got: HashSet<usize> = parse_unified_diff(diff);
        assert_eq!(got, [1usize].into_iter().collect());
    }

    #[test]
    fn malformed_hunk_header_skipped() {
        let diff = "@@ junk @@\n+a\n@@ -1,1 +5,1 @@\n+real\n";
        let got: HashSet<usize> = parse_unified_diff(diff);
        assert_eq!(got, [5usize].into_iter().collect());
    }

    #[test]
    fn hunk_header_without_closing_marker_skipped() {
        let diff = "@@ -1,1 +5,1\n+a\n";
        assert!(parse_unified_diff(diff).is_empty());
    }

    #[test]
    fn hunk_header_with_non_numeric_start_skipped() {
        let diff = "@@ -1,1 +abc,1 @@\n+a\n";
        assert!(parse_unified_diff(diff).is_empty());
    }

    #[test]
    fn hunk_header_with_non_numeric_count_skipped() {
        let diff = "@@ -1,1 +5,xx @@\n+a\n";
        assert!(parse_unified_diff(diff).is_empty());
    }

    #[test]
    fn hunk_with_section_heading_after_closing_marker() {
        let diff = "@@ -1,2 +1,2 @@ fn outer()\n-a\n+a'\n-b\n+b'\n";
        let mut got: Vec<usize> = parse_unified_diff(diff).into_iter().collect();
        got.sort_unstable();
        assert_eq!(got, vec![1, 2]);
    }

    #[test]
    fn count_zero_with_plus_form_is_skipped() {
        let diff = "@@ -1,2 +5,0 @@\n-a\n-b\n";
        assert!(parse_unified_diff(diff).is_empty());
    }
}
