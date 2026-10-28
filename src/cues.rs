//! Lyrics in each format a `[[lyrics.out]]` writes, from the LRC muman
//! keeps them as, cleaned and moved to the song: LRC itself, WebVTT and
//! SubRip cues, and plain text.
//!
//! A cue is each time a line is sung: a line holding several times, as a
//! chorus written once, is a cue at each, and the cues go in order of
//! time, lines at one time in the order the lyrics hold them, joined as
//! one cue. A cue ends where the next begins, and the last where the
//! song does, at the length the plan records, so it is the same however
//! the song was written; an empty line with a time, as LRC marks where a
//! line ends, ends the cue before it and begins none. Times are whole
//! milliseconds and lines end with LF alone, so one plan writes the same
//! bytes on every system. Lyrics whose lines carry no times make no cues,
//! and no WebVTT or SubRip file.
//!
//! WebVTT keeps a word's time, as `<00:00:12.300>`, where it falls inside
//! its cue after the word before it, and escapes `&`, `<` and `>` in what
//! is sung; SubRip holds no word times, nor does plain text.

use std::fmt::Write as _;

use crate::lyrics::{lrc_time, rewrite_word_times, without_word_times};
use crate::settings::LyricsFormat;

/// How long the last cue lasts where the song's length is not known or
/// not after it.
const LAST_CUE_MS: i64 = 4000;

/// `lrc` written as `format`, with the word times `words` keeps, the last
/// cue ending at `end_ms`; none where `format` needs times `lrc` has not.
#[must_use]
pub fn write(lrc: &str, format: LyricsFormat, words: bool, end_ms: Option<i64>) -> Option<String> {
    match format {
        LyricsFormat::Lrc if words => Some(lrc.to_string()),
        LyricsFormat::Lrc => Some(lrc.lines().map(|l| without_word_times(l) + "\n").collect()),
        LyricsFormat::Txt => Some(plain(lrc)),
        LyricsFormat::Vtt => {
            let cues = cues(lrc, end_ms)?;
            let mut out = String::from("WEBVTT\n");
            for cue in cues {
                let text = vtt_text(&cue, words);
                let _ = write!(
                    out,
                    "\n{} --> {}\n{text}\n",
                    clock(cue.start, '.'),
                    clock(cue.end, '.')
                );
            }
            Some(out)
        }
        LyricsFormat::Srt => {
            let cues = cues(lrc, end_ms)?;
            let mut out = String::new();
            for (n, cue) in cues.iter().enumerate() {
                let text = cue
                    .lines
                    .iter()
                    .map(|l| without_word_times(l).trim().to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                let _ = write!(
                    out,
                    "{}{}\n{} --> {}\n{text}\n",
                    if n == 0 { "" } else { "\n" },
                    n + 1,
                    clock(cue.start, ','),
                    clock(cue.end, ',')
                );
            }
            Some(out)
        }
    }
}

/// One cue: when it begins and ends, and the lines sung, word times kept.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cue {
    start: i64,
    end: i64,
    lines: Vec<String>,
}

/// Each line of `lrc` with a time, at each of its times, in order of
/// time, then of the line: the line's text, empty for one that only ends
/// the line before it.
fn timed(lrc: &str) -> Vec<(i64, String)> {
    let mut sung: Vec<(i64, usize, String)> = Vec::new();
    for (n, line) in lrc.lines().enumerate() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some((ms, after)) = rest.strip_prefix('[').and_then(lrc_time) {
            times.push(ms);
            rest = after;
        }
        for ms in times {
            sung.push((ms, n, rest.trim().to_string()));
        }
    }
    sung.sort_by_key(|(ms, n, _)| (*ms, *n));
    sung.into_iter().map(|(ms, _, text)| (ms, text)).collect()
}

/// The cues `lrc` makes, the last ending at `end_ms`; none for lyrics
/// with no line timed.
fn cues(lrc: &str, end_ms: Option<i64>) -> Option<Vec<Cue>> {
    let timed = timed(lrc);
    if timed.is_empty() {
        return None;
    }
    let mut starts: Vec<i64> = timed.iter().map(|(ms, _)| *ms).collect();
    starts.dedup();
    let mut cues = Vec::new();
    for (i, start) in starts.iter().enumerate() {
        let lines: Vec<String> = timed
            .iter()
            .filter(|(ms, text)| ms == start && !without_word_times(text).trim().is_empty())
            .map(|(_, text)| text.clone())
            .collect();
        let end = match starts.get(i + 1) {
            Some(next) => *next,
            None => end_ms
                .filter(|end| end > start)
                .unwrap_or(start + LAST_CUE_MS),
        };
        if !lines.is_empty() {
            cues.push(Cue {
                start: *start,
                end,
                lines,
            });
        }
    }
    Some(cues)
}

/// A cue's lines as WebVTT holds them: `&`, `<` and `>` in what is sung
/// escaped, and each word time, where `words` keeps them, inside the cue
/// and after the one before it.
fn vtt_text(cue: &Cue, words: bool) -> String {
    const MARK: char = '\u{0}';
    let mut last = cue.start;
    let lines: Vec<String> = cue
        .lines
        .iter()
        .map(|line| {
            let mut times = Vec::new();
            let marked = rewrite_word_times(&line.replace(MARK, ""), &mut |ms| {
                times.push(ms);
                Some(MARK.to_string())
            });
            let mut times = times.into_iter();
            let mut out = String::with_capacity(marked.len());
            for c in marked.chars() {
                match c {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    MARK => {
                        let ms = times
                            .next()
                            .filter(|ms| words && *ms > last && *ms < cue.end);
                        if let Some(ms) = ms {
                            last = ms;
                            let _ = write!(out, "<{}>", clock(ms, '.'));
                        }
                    }
                    c => out.push(c),
                }
            }
            out.trim().to_string()
        })
        .collect();
    lines.join("\n")
}

/// `ms` as `hh:mm:ss.mmm`, the fraction after `point`.
fn clock(ms: i64, point: char) -> String {
    let ms = ms.max(0);
    format!(
        "{:02}:{:02}:{:02}{point}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// The words of `lrc` as plain lines: a timed one at each time it is
/// sung, in order of time, and an untimed one as it stands, but for ID
/// tags; no time, no word time, no empty line.
fn plain(lrc: &str) -> String {
    let timed = timed(lrc);
    let lines: Vec<String> = if timed.is_empty() {
        lrc.lines()
            .map(str::trim)
            .filter(|l| !(l.starts_with('[') && l.ends_with(']')))
            .map(without_word_times)
            .collect()
    } else {
        timed
            .into_iter()
            .map(|(_, text)| without_word_times(&text))
            .collect()
    };
    let mut out = String::new();
    for line in lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests;
