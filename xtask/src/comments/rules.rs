//! The mechanical rules of `COMMENTS.md`. Each returns the diagnostics
//! for one file; a `// allow-<rule>: <reason>` comment on or above a
//! line silences that rule there.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use super::diff::DiffInfo;
use super::parse::{LineKind, ParsedFile, parse};

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub rule: &'static str,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// rustfmt's own width, so a comment never outruns the code beside it.
const MAX_LINE_LEN: usize = 100;
const MAX_CONSECUTIVE_COMMENTS: usize = 3;
const MAX_SECTION_HEADERS: usize = 8;
const MAX_COMMENT_DENSITY: f64 = 0.30;
/// Below this a single comment swings the ratio past any cap.
const MIN_NON_BLANK_FOR_DENSITY: usize = 10;
const DENSITY_DELTA_PP: f64 = 5.0;

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub diff: Option<DiffInfo>,
    /// Whether the file is a module that must open with a `//!` summary.
    pub needs_module_doc: bool,
}

#[must_use]
pub fn check(file: &ParsedFile, opts: &Options) -> Vec<Diagnostic> {
    let suppressed = suppressions(file);
    let mut out = Vec::new();
    out.extend(todo_format(file));
    out.extend(ai_narration(file));
    out.extend(section_header_style(file));
    out.extend(consecutive_comments(file));
    out.extend(line_length(file));
    out.extend(commented_out_code(file));
    out.extend(comment_density(file));
    out.extend(section_header_count(file));
    if opts.needs_module_doc {
        out.extend(module_doc(file));
    }
    if let Some(diff) = &opts.diff {
        out.extend(no_comments_on_unchanged_code(file, diff));
        out.extend(density_delta(file, diff));
    }
    out.retain(|d| {
        !suppressed
            .get(&d.line)
            .is_some_and(|rules| rules.contains(d.rule) || rules.contains("*"))
    });
    out.sort_by_key(|d| (d.line, d.column, d.rule));
    out
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

static ALLOW: LazyLock<Regex> =
    LazyLock::new(|| regex(r"^//[/!]?\s*allow-([A-Za-z0-9-]+|\*)\s*:\s*\S"));
static HEADER: LazyLock<Regex> = LazyLock::new(|| regex(r"^//\s+----\s+\S.*\S\s+----\s*$"));

/// Line numbers (1-based) to the rules silenced on them: the marker's
/// own line and the next non-blank one.
fn suppressions(file: &ParsedFile) -> HashMap<usize, HashSet<String>> {
    let mut out: HashMap<usize, HashSet<String>> = HashMap::new();
    for (idx, line) in file.lines.iter().enumerate() {
        let Some(caps) = line
            .comment
            .as_ref()
            .and_then(|c| ALLOW.captures(c.text.trim_start()))
        else {
            continue;
        };
        let rule = caps[1].to_string();
        let target = file
            .lines
            .iter()
            .enumerate()
            .skip(idx + 1)
            .find(|(_, l)| l.kind != LineKind::Blank)
            .map_or(idx, |(j, _)| j);
        out.entry(target + 1).or_default().insert(rule.clone());
        out.entry(idx + 1).or_default().insert(rule);
    }
    out
}

fn diag(rule: &'static str, line: usize, column: usize, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        rule,
        line,
        column,
        message: message.into(),
    }
}

/// Each line's comment with its index, for rules that read the text.
fn comments(file: &ParsedFile) -> impl Iterator<Item = (usize, &super::parse::Comment)> {
    file.lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| l.comment.as_ref().map(|c| (i, c)))
}

fn line_length(file: &ParsedFile) -> Vec<Diagnostic> {
    file.lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.comment.is_some())
        // A Markdown table row in rustdoc cannot be wrapped.
        .filter(|(_, l)| {
            !(l.kind == LineKind::Doc
                && l.comment
                    .as_ref()
                    .is_some_and(|c| c.text[3..].trim_start().starts_with('|')))
        })
        .filter_map(|(idx, line)| {
            let len = line.raw.chars().count();
            (len > MAX_LINE_LEN).then(|| {
                diag(
                    "line-length",
                    idx + 1,
                    MAX_LINE_LEN + 1,
                    format!("comment line is {len} characters; wrap it at {MAX_LINE_LEN}"),
                )
            })
        })
        .collect()
}

fn todo_format(file: &ParsedFile) -> Vec<Diagnostic> {
    static OK: LazyLock<Regex> = LazyLock::new(|| regex(r"^//[/!]?\s*TODO:\s+\S"));
    static MENTION: LazyLock<Regex> = LazyLock::new(|| regex(r"\bTODO\b"));
    comments(file)
        .filter(|(_, c)| MENTION.is_match(&c.text) && !OK.is_match(c.text.trim_start()))
        .map(|(idx, c)| {
            diag(
                "todo-format",
                idx + 1,
                c.byte_range.start + 1,
                "a TODO must read `// TODO: ` and one actionable sentence",
            )
        })
        .collect()
}

fn ai_narration(file: &ParsedFile) -> Vec<Diagnostic> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        regex(
            r"(?i)^//[/!]?\s*(added by\b|changed from\b|updated to\b|new:|removed:|claude:|copilot:|claude-generated\b|gpt-generated\b|ai-generated\b)",
        )
    });
    comments(file)
        .filter_map(|(idx, c)| {
            let m = RE.find(c.text.trim_start())?;
            Some(diag(
                "ai-narration",
                idx + 1,
                c.byte_range.start + 1,
                format!(
                    "`{}` narrates an edit; a comment says why the code is so",
                    m.as_str().trim()
                ),
            ))
        })
        .collect()
}

fn section_header_style(file: &ParsedFile) -> Vec<Diagnostic> {
    static DECORATIVE: LazyLock<Regex> = LazyLock::new(|| regex(r"-{3,}|={3,}"));
    comments(file)
        .filter(|(idx, c)| {
            file.lines[*idx].kind == LineKind::Comment
                && DECORATIVE.is_match(&c.text)
                && !HEADER.is_match(c.text.trim_start())
        })
        .map(|(idx, c)| {
            diag(
                "section-header-style",
                idx + 1,
                c.byte_range.start + 1,
                "a section header reads `// ---- Title ----`",
            )
        })
        .collect()
}

fn consecutive_comments(file: &ParsedFile) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut run = 0usize;
    for (idx, line) in file.lines.iter().enumerate() {
        if line.kind == LineKind::Comment {
            run += 1;
            if run == MAX_CONSECUTIVE_COMMENTS + 1 {
                out.push(diag(
                    "consecutive-comments",
                    idx + 1,
                    1,
                    format!(
                        "more than {MAX_CONSECUTIVE_COMMENTS} comment lines in a row; \
                         move the rationale into the item's or module's rustdoc"
                    ),
                ));
            }
        } else {
            run = 0;
        }
    }
    out
}

fn commented_out_code(file: &ParsedFile) -> Vec<Diagnostic> {
    static CODE: LazyLock<Regex> = LazyLock::new(|| {
        regex(
            r"(?x)^//\s*(
                (let|const|static)\s+(mut\s+)?\w+\s*(:[^=]+)?=.*;\s*$
              | (pub(\([^)]*\))?\s+)?(fn|struct|enum|impl|mod|trait|use)\s+[\w:<{]
              | \#!?\[\w
              | [\w.:]+(::<[^>]*>)?\([^)]*\)\??;\s*$
              | [\w.\[\]]+\s*[+\-*/]?=\s*[^=].*;\s*$
              | \}\s*$
            )",
        )
    });
    comments(file)
        .filter(|(idx, c)| {
            file.lines[*idx].kind == LineKind::Comment
                && CODE.is_match(c.text.trim_start())
                && !ALLOW.is_match(c.text.trim_start())
        })
        .map(|(idx, c)| {
            diag(
                "commented-out-code",
                idx + 1,
                c.byte_range.start + 1,
                "this looks like commented-out code; delete it, or explain it with \
                 `// allow-commented-out-code: <reason>` above",
            )
        })
        .collect()
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn comment_density(file: &ParsedFile) -> Vec<Diagnostic> {
    let non_blank = file.non_blank_count();
    if non_blank < MIN_NON_BLANK_FOR_DENSITY {
        return Vec::new();
    }
    let pure = file.pure_comment_count();
    let ratio = pure as f64 / non_blank as f64;
    if ratio <= MAX_COMMENT_DENSITY {
        return Vec::new();
    }
    vec![diag(
        "comment-density",
        1,
        1,
        format!(
            "{}% of non-blank lines are `//` comments ({pure}/{non_blank}); the cap is {}%",
            (ratio * 100.0).round() as u32,
            (MAX_COMMENT_DENSITY * 100.0).round() as u32
        ),
    )]
}

fn section_header_count(file: &ParsedFile) -> Vec<Diagnostic> {
    let count = comments(file)
        .filter(|(_, c)| HEADER.is_match(c.text.trim_start()))
        .count();
    if count <= MAX_SECTION_HEADERS {
        return Vec::new();
    }
    vec![diag(
        "section-header-count",
        1,
        1,
        format!("{count} section headers; the cap is {MAX_SECTION_HEADERS}, so split the module"),
    )]
}

fn module_doc(file: &ParsedFile) -> Vec<Diagnostic> {
    let first = file.lines.iter().find(|l| l.kind != LineKind::Blank);
    let opens_with_summary = first.is_some_and(|l| {
        l.kind == LineKind::Doc
            && l.comment
                .as_ref()
                .is_some_and(|c| c.text.starts_with("//!"))
    });
    if opens_with_summary {
        return Vec::new();
    }
    vec![diag(
        "module-doc",
        1,
        1,
        "a module opens with a `//!` summary of what it is for",
    )]
}

fn no_comments_on_unchanged_code(file: &ParsedFile, diff: &DiffInfo) -> Vec<Diagnostic> {
    let Some(changed) = diff.changed_lines.as_ref() else {
        return Vec::new();
    };
    let base = diff.base_file.as_deref().map(parse);
    let mut out = Vec::new();
    for (idx, line) in file.lines.iter().enumerate() {
        let line_no = idx + 1;
        let Some(comment) = &line.comment else {
            continue;
        };
        if !changed.contains(&line_no)
            || neighbor_changed(file, idx, false, changed)
            || neighbor_changed(file, idx, true, changed)
            || base.as_ref().is_some_and(|b| base_comment_near(b, line_no))
        {
            continue;
        }
        out.push(diag(
            "no-comments-on-unchanged-code",
            line_no,
            comment.byte_range.start + 1,
            "a comment added beside code this change leaves alone",
        ));
    }
    out
}

/// Whether the base had a comment within two lines: then this one was
/// edited in place, as after a rename, rather than added.
fn base_comment_near(base: &ParsedFile, line_no: usize) -> bool {
    (line_no.saturating_sub(2)..=line_no + 2)
        .any(|n| n >= 1 && base.lines.get(n - 1).is_some_and(|l| l.comment.is_some()))
}

/// Whether the nearest code line before (or after) `from` changed;
/// comment-only and blank lines are skipped.
fn neighbor_changed(
    file: &ParsedFile,
    from: usize,
    forward: bool,
    changed: &HashSet<usize>,
) -> bool {
    let is_code = |j: &usize| {
        matches!(
            file.lines[*j].kind,
            LineKind::Code | LineKind::CodeWithComment
        )
    };
    let found = if forward {
        (from + 1..file.lines.len()).find(is_code)
    } else {
        (0..from).rev().find(is_code)
    };
    found.is_some_and(|j| changed.contains(&(j + 1)))
}

#[allow(clippy::cast_precision_loss)]
fn density_delta(file: &ParsedFile, diff: &DiffInfo) -> Vec<Diagnostic> {
    let Some(base_file) = diff.base_file.as_ref() else {
        return Vec::new();
    };
    let base = parse(base_file);
    let (now_lines, base_lines) = (file.non_blank_count(), base.non_blank_count());
    if now_lines < MIN_NON_BLANK_FOR_DENSITY || base_lines < MIN_NON_BLANK_FOR_DENSITY {
        return Vec::new();
    }
    let now = file.pure_comment_count() as f64 / now_lines as f64;
    let was = base.pure_comment_count() as f64 / base_lines as f64;
    let delta = (now - was) * 100.0;
    if delta <= DENSITY_DELTA_PP {
        return Vec::new();
    }
    vec![diag(
        "density-delta",
        1,
        1,
        format!(
            "`//` comment density rose {delta:.1} points ({:.0}% to {:.0}%); the cap is \
             {DENSITY_DELTA_PP} per change",
            was * 100.0,
            now * 100.0
        ),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fired(src: &str, rule: &str) -> Vec<usize> {
        let opts = Options {
            needs_module_doc: true,
            ..Options::default()
        };
        check(&parse(src), &opts)
            .into_iter()
            .filter(|d| d.rule == rule)
            .map(|d| d.line)
            .collect()
    }

    #[test]
    fn a_todo_needs_its_colon_and_sentence() {
        assert_eq!(fired("//! m\n// TODO fix\n", "todo-format"), [2]);
        assert_eq!(
            fired("//! m\n// TODO: Fix the thing.\n", "todo-format"),
            [] as [usize; 0]
        );
    }

    #[test]
    fn narration_is_refused() {
        assert_eq!(fired("//! m\n// Added by Claude\n", "ai-narration"), [2]);
    }

    #[test]
    fn four_plain_lines_in_a_row_are_too_many_but_rustdoc_is_not() {
        let plain = "//! m\n// a\n// b\n// c\n// d\nfn f() {}\n";
        assert_eq!(fired(plain, "consecutive-comments"), [5]);
        let doc = "//! m\n/// a\n/// b\n/// c\n/// d\nfn f() {}\n";
        assert_eq!(fired(doc, "consecutive-comments"), [] as [usize; 0]);
    }

    #[test]
    fn section_headers_have_one_shape() {
        assert_eq!(
            fired("//! m\n// ===== X =====\n", "section-header-style"),
            [2]
        );
        assert_eq!(
            fired("//! m\n// ---- Title ----\n", "section-header-style"),
            [] as [usize; 0]
        );
        assert!(
            fired("//! m\n/// | a |\n/// |---|\n", "section-header-style").is_empty(),
            "a rustdoc table is no header"
        );
    }

    #[test]
    fn commented_out_code_is_caught_and_prose_is_not() {
        for code in [
            "// let x = 1;",
            "// fn main() {",
            "// foo(bar);",
            "// x += 1;",
            "// }",
            "// #[test]",
        ] {
            assert_eq!(
                fired(&format!("//! m\n{code}\n"), "commented-out-code"),
                [2],
                "{code}"
            );
        }
        for prose in [
            "// Let the caller decide.",
            "// The cap is 3 (see above).",
            "// Use the lock; it is cheap.",
        ] {
            assert!(
                fired(&format!("//! m\n{prose}\n"), "commented-out-code").is_empty(),
                "{prose}"
            );
        }
    }

    #[test]
    fn an_allow_marker_silences_the_next_line() {
        let src = "//! m\n// allow-commented-out-code: kept as the reference shape\n// foo(bar);\n";
        assert_eq!(fired(src, "commented-out-code"), [] as [usize; 0]);
    }

    #[test]
    fn a_module_opens_with_its_summary() {
        assert_eq!(fired("use std::io;\n", "module-doc"), [1]);
        assert_eq!(
            fired("\n//! What this is.\nuse std::io;\n", "module-doc"),
            [] as [usize; 0]
        );
    }

    #[test]
    fn density_counts_plain_comments_against_the_cap() {
        let dense = format!("//! m\n{}", "// why\nlet x = 1;\n".repeat(6));
        assert_eq!(fired(&dense, "comment-density"), [1]);
        let documented = format!("//! m\n{}", "/// what\nlet x = 1;\n".repeat(6));
        assert_eq!(fired(&documented, "comment-density"), [] as [usize; 0]);
    }

    #[test]
    fn a_long_comment_line_is_flagged_but_a_rustdoc_table_row_is_not() {
        let src = format!("//! m\n// {}\n", "word ".repeat(25));
        assert_eq!(fired(&src, "line-length"), [2]);
        let table = format!("//! | a | {} |\n", "word ".repeat(25));
        assert!(fired(&table, "line-length").is_empty());
    }

    #[test]
    fn a_new_comment_on_unchanged_code_is_flagged() {
        let src = "//! m\n\nuse x;\nfn a() {}\n// why\nfn b() {}\n";
        let diff = DiffInfo {
            changed_lines: Some(HashSet::from([5])),
            base_file: Some("//! m\n\nuse x;\nfn a() {}\nfn b() {}\n".into()),
        };
        let opts = Options {
            diff: Some(diff),
            needs_module_doc: true,
        };
        let rules: Vec<_> = check(&parse(src), &opts).iter().map(|d| d.rule).collect();
        assert_eq!(rules, ["no-comments-on-unchanged-code"]);
    }
}
