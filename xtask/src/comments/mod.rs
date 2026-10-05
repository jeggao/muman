//! `cargo xtask comments`: the mechanical rules of `COMMENTS.md` over
//! every Rust file in the repository.

mod diff;
mod parse;
mod report;
mod rules;

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Files a module summary is not asked of: entry points and test-only
/// modules, whose names say what they are.
const NO_MODULE_DOC: [&str; 3] = ["main.rs", "tests.rs", "testing.rs"];

/// Check every `.rs` file under `root`, comparing against `diff_base`
/// when given. Returns how many diagnostics were printed.
pub fn run(root: &Path, diff_base: Option<&str>, annotations: bool) -> Result<usize> {
    let format = if annotations {
        report::Format::GithubAnnotations
    } else {
        report::Format::Human
    };
    let mut files = Vec::new();
    rust_files(root, &mut files)?;
    files.sort();
    let mut out = std::io::stdout().lock();
    let mut total = 0;
    for path in &files {
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let rel = path.strip_prefix(root).unwrap_or(path);
        let opts = rules::Options {
            diff: diff_base
                .map(|base| diff::for_file(root, base, rel))
                .transpose()?,
            needs_module_doc: needs_module_doc(rel),
        };
        let diags = rules::check(&parse::parse(&source), &opts);
        report::print_file(&mut out, format, rel, &diags)?;
        total += diags.len();
    }
    if !annotations {
        writeln!(
            out,
            "comments: {total} finding(s) in {} file(s)",
            files.len()
        )?;
    }
    Ok(total)
}

fn needs_module_doc(rel: &Path) -> bool {
    let in_src = rel.components().any(|c| c.as_os_str() == "src");
    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    in_src && !NO_MODULE_DOC.contains(&name)
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || name == "target" {
            continue;
        }
        if entry.file_type()?.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_under_src_need_a_summary_but_entry_points_and_tests_do_not() {
        assert!(needs_module_doc(Path::new("src/store.rs")));
        assert!(needs_module_doc(Path::new("xtask/src/docs.rs")));
        assert!(!needs_module_doc(Path::new("src/main.rs")));
        assert!(!needs_module_doc(Path::new("src/clean/tests.rs")));
        assert!(!needs_module_doc(Path::new("tests/smoke.rs")));
    }
}
