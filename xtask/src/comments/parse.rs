//! Each line of a Rust file classified by its comments. A small lexer
//! skips string, raw string, byte string and char literals, tells a
//! lifetime from a char, and nests block comments, so nothing inside a
//! literal passes for a comment.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Blank,
    Code,
    /// Only a plain `//` or `/* */` comment.
    Comment,
    /// Only rustdoc: `///`, `//!`, `/** */` or `/*! */`.
    Doc,
    CodeWithComment,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub raw: String,
    pub kind: LineKind,
    pub comment: Option<Comment>,
}

#[derive(Debug, Clone)]
pub struct Comment {
    /// Where the comment's part of this line sits in `Line::raw`.
    pub byte_range: Range<usize>,
    pub text: String,
    pub doc: bool,
}

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub lines: Vec<Line>,
}

#[must_use]
pub fn parse(source: &str) -> ParsedFile {
    let comments = lex(source);
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut next = comments.iter().peekable();
    for raw in source.split_inclusive('\n') {
        let line = raw.trim_end_matches('\n').trim_end_matches('\r');
        let end = start + line.len();
        while next.peek().is_some_and(|(r, _)| r.end <= start) {
            next.next();
        }
        let comment = next
            .peek()
            .filter(|(r, _)| r.start < end.max(start + 1) && r.end > start)
            .map(|(r, doc)| {
                let range = r.start.max(start) - start..r.end.min(end) - start;
                Comment {
                    text: line[range.clone()].to_string(),
                    byte_range: range,
                    doc: *doc,
                }
            });
        let kind = classify(line, comment.as_ref());
        lines.push(Line {
            raw: line.to_string(),
            kind,
            comment,
        });
        start += raw.len();
    }
    ParsedFile { lines }
}

/// Every comment's byte range in `source`, and whether it is rustdoc.
fn lex(source: &str) -> Vec<(Range<usize>, bool)> {
    let b = source.as_bytes();
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match (b[i], at(i + 1)) {
            (b'/', b'/') => {
                let end = source[i..].find('\n').map_or(b.len(), |n| i + n);
                let doc = (at(i + 2) == b'/' && at(i + 3) != b'/') || at(i + 2) == b'!';
                out.push((i..end, doc));
                i = end;
            }
            (b'/', b'*') => {
                let start = i;
                let mut depth = 0usize;
                while i < b.len() {
                    if at(i) == b'/' && at(i + 1) == b'*' {
                        depth += 1;
                        i += 2;
                    } else if at(i) == b'*' && at(i + 1) == b'/' {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                let doc = (at(start + 2) == b'*' && !matches!(at(start + 3), b'*' | b'/'))
                    || at(start + 2) == b'!';
                out.push((start..i.min(b.len()), doc));
            }
            (b'"', _) => i = skip_string(b, i + 1),
            (b'r' | b'b' | b'c', _) if !ident_byte(at(i.wrapping_sub(1))) || i == 0 => {
                let mut j = i + 1;
                if b[i] != b'r' && at(j) == b'r' {
                    j += 1;
                }
                let raw = b[i] == b'r' || at(i + 1) == b'r';
                let hashes = b[j..].iter().take_while(|&&c| c == b'#').count();
                if at(j + hashes) == b'"' && (raw || hashes == 0) {
                    i = if raw {
                        skip_raw(b, j + hashes + 1, hashes)
                    } else {
                        skip_string(b, j + 1)
                    };
                } else if b[i] == b'b' && at(i + 1) == b'\'' {
                    i = skip_char(b, i + 1);
                } else {
                    i += 1;
                }
            }
            (b'\'', _) => i = skip_char(b, i),
            _ => i += 1,
        }
    }
    out
}

fn ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// Past the closing quote of a string whose body starts at `i`.
fn skip_string(b: &[u8], mut i: usize) -> usize {
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    b.len()
}

/// Past `"` and `hashes` `#`s closing a raw string whose body starts at `i`.
fn skip_raw(b: &[u8], mut i: usize, hashes: usize) -> usize {
    while i < b.len() {
        if b[i] == b'"'
            && b[i + 1..]
                .iter()
                .take(hashes)
                .filter(|&&c| c == b'#')
                .count()
                == hashes
        {
            return i + 1 + hashes;
        }
        i += 1;
    }
    b.len()
}

/// Past a char literal at `i`, or just past the quote of a lifetime or
/// label: `'a'` and `'\n'` are chars, `'a` is not.
fn skip_char(b: &[u8], i: usize) -> usize {
    let at = |j: usize| b.get(j).copied().unwrap_or(0);
    if at(i + 1) == b'\\' {
        let mut j = i + 2;
        while j < b.len() && b[j] != b'\'' {
            j += 1;
        }
        return j + 1;
    }
    // One char, which may be several bytes, then the closing quote.
    let width = std::str::from_utf8(&b[i + 1..(i + 5).min(b.len())])
        .or_else(|e| std::str::from_utf8(&b[i + 1..i + 1 + e.valid_up_to()]))
        .ok()
        .and_then(|s| s.chars().next())
        .map_or(1, char::len_utf8);
    if at(i + 1 + width) == b'\'' {
        i + 2 + width
    } else {
        i + 1
    }
}

fn classify(line: &str, comment: Option<&Comment>) -> LineKind {
    if line.trim().is_empty() {
        return LineKind::Blank;
    }
    let Some(c) = comment else {
        return LineKind::Code;
    };
    let outside = format!(
        "{}{}",
        &line[..c.byte_range.start],
        &line[c.byte_range.end..]
    );
    match (outside.trim().is_empty(), c.doc) {
        (true, true) => LineKind::Doc,
        (true, false) => LineKind::Comment,
        (false, _) => LineKind::CodeWithComment,
    }
}

impl ParsedFile {
    #[must_use]
    pub fn non_blank_count(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| l.kind != LineKind::Blank)
            .count()
    }

    /// Plain comment lines; rustdoc is documentation, counted apart.
    #[must_use]
    pub fn pure_comment_count(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| l.kind == LineKind::Comment)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<LineKind> {
        parse(src).lines.into_iter().map(|l| l.kind).collect()
    }

    #[test]
    fn plain_doc_and_inline_comments_are_told_apart() {
        assert_eq!(
            kinds("//! module\n/// item\n// why\nlet x = 1; // inline\n\nfn f() {}\n"),
            [
                LineKind::Doc,
                LineKind::Doc,
                LineKind::Comment,
                LineKind::CodeWithComment,
                LineKind::Blank,
                LineKind::Code,
            ]
        );
    }

    #[test]
    fn slashes_in_strings_and_raw_strings_are_not_comments() {
        let parsed = parse(
            "let u = \"https://example.com\";\nlet r = r#\"// not\"#;\nlet s = \"a\n// still a string\n\";\n",
        );
        assert!(parsed.lines.iter().all(|l| l.comment.is_none()));
    }

    #[test]
    fn a_lifetime_does_not_open_a_char_literal() {
        let parsed = parse("fn f<'a>(x: &'a str) {} // why\n");
        assert_eq!(parsed.lines[0].kind, LineKind::CodeWithComment);
        assert_eq!(parsed.lines[0].comment.as_ref().unwrap().text, "// why");
    }

    #[test]
    fn a_block_comment_marks_every_line_it_spans() {
        assert_eq!(
            kinds("/* one\n   two */\nlet x = 1;\n"),
            [LineKind::Comment, LineKind::Comment, LineKind::Code]
        );
    }

    #[test]
    fn chars_byte_strings_and_nested_blocks_are_lexed() {
        let parsed = parse(
            "let q = '\"'; // a\nlet s = b\"//\"; let c = '/';\n/* outer /* inner */ still */ x();\nlet e = '\\''; // b\n",
        );
        assert_eq!(parsed.lines[0].comment.as_ref().unwrap().text, "// a");
        assert!(parsed.lines[1].comment.is_none());
        assert_eq!(parsed.lines[2].kind, LineKind::CodeWithComment);
        assert_eq!(
            parsed.lines[2].comment.as_ref().unwrap().text,
            "/* outer /* inner */ still */"
        );
        assert_eq!(parsed.lines[3].comment.as_ref().unwrap().text, "// b");
    }

    #[test]
    fn four_slashes_and_empty_blocks_are_not_rustdoc() {
        assert_eq!(
            kinds("//// rule\n/**/\n/*** x */\n/** doc */\n"),
            [
                LineKind::Comment,
                LineKind::Comment,
                LineKind::Comment,
                LineKind::Doc
            ]
        );
    }

    #[test]
    fn crlf_is_stripped() {
        let parsed = parse("// a\r\nlet x = 1;\r\n");
        assert_eq!(parsed.lines[0].raw, "// a");
        assert_eq!(parsed.lines[1].raw, "let x = 1;");
    }

    #[test]
    fn density_counts_plain_comments_only() {
        let parsed = parse("/// doc\n// plain\nlet x = 1; // inline\n");
        assert_eq!(parsed.pure_comment_count(), 1);
        assert_eq!(parsed.non_blank_count(), 3);
    }
}
