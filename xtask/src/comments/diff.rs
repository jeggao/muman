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
}

/// Build per-file diff info by shelling out to `git`.
pub fn for_file(repo_root: &Path, base: &str, path: &Path) -> Result<DiffInfo> {
    let rel = path
        .strip_prefix(repo_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let changed = changed_lines(repo_root, base, &rel)?;
    let base_file = base_blob(repo_root, base, &rel)?;
    Ok(DiffInfo {
        changed_lines: Some(changed),
        base_file,
    })
}

fn changed_lines(repo_root: &Path, base: &str, rel: &str) -> Result<HashSet<usize>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("diff")
        .arg("--no-color")
        .arg("--unified=0")
        .arg(format!("{base}...HEAD"))
        .arg("--")
        .arg(rel)
        .output()
        .with_context(|| format!("git diff {base} -- {rel}"))?;
    // If the file is untracked or git fails, fall back to all-changed (empty
    // set); we treat the file as fully new in the worktree below.
    if !output.status.success() {
        return Ok(HashSet::new());
    }
    Ok(parse_unified_diff(&String::from_utf8_lossy(&output.stdout)))
}

/// Parse `git diff --unified=0` output and return the set of line numbers in
/// the *new* (post-image) file that were added or modified. Pure removals
/// have no line in the new file and produce no entries.
#[must_use]
pub fn parse_unified_diff(diff: &str) -> HashSet<usize> {
    let mut out = HashSet::new();
    for line in diff.lines() {
        // Hunk header form: `@@ -<oldStart>[,<oldCount>] +<newStart>[,<newCount>] @@`.
        let Some(rest) = line.strip_prefix("@@ ") else {
            continue;
        };
        let Some(end) = rest.find(" @@") else {
            continue;
        };
        let header = &rest[..end];
        let Some(plus) = header.split_whitespace().find(|t| t.starts_with('+')) else {
            continue;
        };
        let plus = &plus[1..];
        let (start_s, count_s) = match plus.split_once(',') {
            Some((a, b)) => (a, b),
            None => (plus, "1"),
        };
        let Ok(start) = start_s.parse::<usize>() else {
            continue;
        };
        let Ok(count) = count_s.parse::<usize>() else {
            continue;
        };
        if count == 0 {
            continue;
        }
        for n in start..start + count {
            out.insert(n);
        }
    }
    out
}

fn base_blob(repo_root: &Path, base: &str, rel: &str) -> Result<Option<String>> {
    let merge_base = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("merge-base")
        .arg(base)
        .arg("HEAD")
        .output()
        .with_context(|| format!("git merge-base {base} HEAD"))?;
    let revision = if merge_base.status.success() {
        String::from_utf8_lossy(&merge_base.stdout)
            .trim()
            .to_string()
    } else {
        base.to_string()
    };
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("show")
        .arg(format!("{revision}:{rel}"))
        .output()
        .with_context(|| format!("git show {revision}:{rel}"))?;
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
