//! Diagnostics as `path:line:col: rule: message`, or as GitHub Actions
//! annotations that land on the line in a pull request.

use std::io::Write;
use std::path::Path;

use super::rules::Diagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    GithubAnnotations,
}

pub fn print_file(
    out: &mut impl Write,
    format: Format,
    path: &Path,
    diags: &[Diagnostic],
) -> std::io::Result<()> {
    // Forward slashes on every platform, so annotations match the repo path.
    let path = path.to_string_lossy().replace('\\', "/");
    for d in diags {
        match format {
            Format::Human => writeln!(
                out,
                "{path}:{}:{}: {}: {}",
                d.line, d.column, d.rule, d.message
            )?,
            Format::GithubAnnotations => writeln!(
                out,
                "::error file={path},line={},col={},title=comments/{}::{}",
                d.line,
                d.column,
                d.rule,
                escape(&d.message)
            )?,
        }
    }
    Ok(())
}

fn escape(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_annotation_escapes_percent_first() {
        assert_eq!(escape("100%\n"), "100%25%0A");
    }

    #[test]
    fn a_windows_path_reads_with_forward_slashes() {
        let mut out = Vec::new();
        let d = Diagnostic {
            rule: "line-length",
            line: 3,
            column: 101,
            message: "long".into(),
        };
        print_file(&mut out, Format::Human, Path::new(r"src\a.rs"), &[d]).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "src/a.rs:3:101: line-length: long\n"
        );
    }
}
