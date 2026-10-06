//! yt-dlp's output, never passed through as it is: each line is said to
//! come from yt-dlp, its errors and warnings become muman's own, and
//! its progress goes into the run's step, counting each video it
//! finishes; on a terminal with no status line, it is one line rewritten
//! in place.

use std::io::Write;

use crate::ui::Style;

use crate::runner::Line;

/// What every progress line starts with, so it is told from the rest.
const PROGRESS: &str = "muman-progress";

/// The yt-dlp options that write progress as one parsable line each.
#[must_use]
pub fn progress_args() -> [String; 3] {
    [
        "--newline".to_string(),
        "--progress-template".to_string(),
        format!(
            "download:{PROGRESS}\t%(progress._percent_str)s\t%(progress._total_bytes_str,progress._total_bytes_estimate_str)s\t%(progress._speed_str)s\t%(info.title)s"
        ),
    ]
}

/// Relays one yt-dlp run's lines to `out`.
pub struct Relay<'a, W: Write> {
    out: &'a mut W,
    /// Whether `out` is a terminal, where a line can be rewritten.
    live: bool,
    /// The width of the line now showing, to blank what a shorter one
    /// leaves.
    shown: usize,
    /// The terminal's width in columns, which a status line must stay
    /// under or it wraps and can no longer be rewritten in place.
    columns: usize,
    /// The step the downloads count towards.
    step: Option<&'a crate::progress::Step>,
    /// The video downloading now; another's progress means it is done.
    title: Option<String>,
}

impl<'a, W: Write> Relay<'a, W> {
    pub fn new(out: &'a mut W, live: bool) -> Self {
        let columns = terminal_size::terminal_size_of(std::io::stderr())
            .map_or(80, |(w, _)| usize::from(w.0));
        Self {
            out,
            live,
            shown: 0,
            columns,
            step: None,
            title: None,
        }
    }

    /// Count each video yt-dlp finishes towards `step`, and say what it is
    /// downloading in it.
    #[must_use]
    pub fn counting(mut self, step: &'a crate::progress::Step) -> Self {
        self.step = Some(step);
        self
    }

    pub fn line(&mut self, line: Line<'_>) {
        let text = match line {
            Line::Out(t) | Line::Err(t) => t.trim_end(),
        };
        if text.is_empty() {
            return;
        }
        if let Some(rest) = text.strip_prefix("ERROR:") {
            self.settle();
            let _ = crate::ui::error(self.out, &format!("yt-dlp: {}", rest.trim()));
        } else if let Some(rest) = text.strip_prefix("WARNING:") {
            self.settle();
            let _ = crate::ui::warning(self.out, &format!("yt-dlp: {}", rest.trim()));
        } else if let Some(progress) = text.strip_prefix(PROGRESS) {
            if let Some(step) = self.step {
                let title = progress.split('\t').nth(4).map(|t| t.trim().to_string());
                if self.title.is_some() && title != self.title {
                    step.advance(1);
                }
                self.title = title;
                step.note(progress_line(progress).trim_start_matches("yt-dlp: "));
            }
            if self.live {
                self.status(&progress_line(progress));
            }
        } else if self.live {
            self.status(&format!("yt-dlp: {text}"));
        } else {
            let _ = writeln!(
                self.out,
                "{}",
                Style::Muted.paint(&format!("yt-dlp: {text}"))
            );
        }
    }

    /// Show `text` in place of the line now showing, cut to fit a line;
    /// a wide character such as a CJK one takes two columns.
    fn status(&mut self, text: &str) {
        let mut width = 0;
        let text: String = text
            .chars()
            .take_while(|c| {
                width += unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0);
                width < self.columns
            })
            .collect();
        let width = unicode_width::UnicodeWidthStr::width(text.as_str());
        let pad = " ".repeat(self.shown.saturating_sub(width));
        let _ = write!(self.out, "\r{}{pad}", Style::Muted.paint(&text));
        let _ = self.out.flush();
        self.shown = width;
    }

    /// Blank the line now showing, so the next message starts clean.
    pub fn settle(&mut self) {
        if self.shown > 0 {
            let _ = write!(self.out, "\r{}\r", " ".repeat(self.shown));
            let _ = self.out.flush();
            self.shown = 0;
        }
    }
}

impl<W: Write> std::fmt::Debug for Relay<'_, W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Relay")
            .field("live", &self.live)
            .finish_non_exhaustive()
    }
}

impl<W: Write> Drop for Relay<'_, W> {
    fn drop(&mut self) {
        self.settle();
        if let Some(step) = self.step.filter(|_| self.title.is_some()) {
            step.advance(1);
        }
    }
}

/// `yt-dlp: <title>  45.0% of 12.3MiB at 3.1MiB/s` from the template's
/// tab-separated fields.
fn progress_line(fields: &str) -> String {
    let parts: Vec<&str> = fields.split('\t').map(str::trim).collect();
    let field = |n: usize| {
        parts
            .get(n)
            .copied()
            .filter(|s| !s.is_empty() && *s != "NA" && *s != "N/A")
    };
    let mut line = format!("yt-dlp: {}", field(4).unwrap_or("downloading"));
    for (at, before) in [(1, "  "), (2, " of "), (3, " at ")] {
        if let Some(value) = field(at) {
            line.push_str(before);
            line.push_str(value);
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay(lines: &[Line<'_>], live: bool) -> String {
        let mut out = Vec::new();
        {
            let mut r = Relay::new(&mut out, live);
            for l in lines {
                r.line(*l);
            }
        }
        String::from_utf8(out).unwrap()
    }

    fn plain(s: &str) -> String {
        crate::ui::plain(s)
    }

    #[test]
    fn every_line_is_said_to_come_from_yt_dlp() {
        let text = plain(&relay(
            &[
                Line::Out("[youtube] abc: Downloading webpage"),
                Line::Err("WARNING: [youtube] slow"),
                Line::Err("ERROR: [youtube] abc: Video unavailable"),
            ],
            false,
        ));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "yt-dlp: [youtube] abc: Downloading webpage",
                "yt-dlp: [youtube] slow",
                "yt-dlp: [youtube] abc: Video unavailable",
            ]
        );
    }

    #[test]
    fn progress_is_one_rewritten_line_on_a_terminal_and_nothing_elsewhere() {
        let p1 = format!("{PROGRESS}\t  4.0%\t12.30MiB\t3.10MiB/s\tSong");
        let p2 = format!("{PROGRESS}\t100.0%\t12.30MiB\t   N/A\tSong");
        let live = plain(&relay(&[Line::Out(&p1), Line::Out(&p2)], true));
        assert!(!live.contains('\n'), "{live:?}");
        assert!(
            live.contains("\ryt-dlp: Song  4.0% of 12.30MiB at 3.10MiB/s"),
            "{live:?}"
        );
        assert!(
            live.contains("\ryt-dlp: Song  100.0% of 12.30MiB"),
            "{live:?}"
        );
        assert!(
            live.ends_with('\r'),
            "cleared once the run is over: {live:?}"
        );
        assert_eq!(relay(&[Line::Out(&p1)], false), "");
    }

    #[test]
    fn an_error_clears_the_progress_line_first() {
        let p = format!("{PROGRESS}\t50%\t1MiB\t1MiB/s\tSong");
        let text = plain(&relay(&[Line::Out(&p), Line::Err("ERROR: gone")], true));
        let at = text.find("yt-dlp: gone").unwrap();
        assert!(text[..at].ends_with('\r'), "{text:?}");
    }
}
