//! What cleaning takes out of lyrics, by rules in a fixed order. Each
//! rule is a switch in the song list's `[clean.lyrics.<tier>]`, its tier
//! saying how far it may stray, as the tag rules' tiers say in
//! [`super::TIERS`].
//!
//! Lyrics are cleaned as a song is written, from the text their source
//! holds, never when read: the source stays as it was, so a changed
//! switch needs nothing read again. A song's plan names the rules
//! switched off and [`EDITION`], so a switch flipped, or a rule that
//! writes otherwise, writes every song with lyrics again; a test keeps
//! [`EDITION`] in step with what the rules write. Before the plan named
//! them, a library kept lyrics as the muman that wrote them cleaned
//! them, however its rules changed after.
//!
//! What a line is, sung, a cue or a credit, is read by
//! [`crate::lyrics`], which matching reads too.

use std::collections::BTreeSet;

use super::Settings;
use crate::lyrics::{self, Kind};

/// What the rules write, raised whenever a rule writes otherwise than
/// before, so every song with lyrics is written again.
pub const EDITION: u32 = 1;

/// One rule: what it does to the lyrics, knowing the song.
type Edit = fn(&mut Lyrics, &Context<'_>);

/// In the order run: markup goes first, so a cue in `<i>` is read as one.
const RULES: [(&str, Edit); 3] = [
    ("lyrics.tidy.markup", markup),
    ("lyrics.packaging.cues", cues),
    ("lyrics.packaging.credits", credits),
];

/// Every rule's name, `lyrics.<tier>.<key>`, in the order run.
pub fn names() -> impl Iterator<Item = &'static str> {
    RULES.iter().map(|(name, _)| *name)
}

/// Every switch as `(tier, key)`, grouped by tier in the order of
/// [`super::TIERS`], and within one in the order run.
pub fn switches() -> impl Iterator<Item = (&'static str, &'static str)> {
    super::TIERS.iter().flat_map(|(tier, _)| {
        names()
            .filter_map(|n| n.strip_prefix("lyrics.")?.split_once('.'))
            .filter(move |(t, _)| t == tier)
    })
}

/// The rules `settings` switches off, as a plan names them.
#[must_use]
pub fn off(settings: &Settings) -> Vec<String> {
    names()
        .filter(|n| !settings.is_on(n))
        .map(str::to_string)
        .collect()
}

/// What the rules may know of the song: its title and artists, and
/// whether the lyrics were a subtitle stream's.
#[derive(Debug, Clone, Default)]
pub struct Context<'a> {
    pub title: Option<&'a str>,
    pub artists: Vec<&'a str>,
    pub subtitle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    pub text: String,
    /// The rules that changed the lyrics, in the order run.
    pub rules: Vec<&'static str>,
}

/// `text` cleaned by every rule but those named in `off`. CRLF and a
/// missing last newline come out as LF, every line ended.
#[must_use]
pub fn clean(text: &str, ctx: &Context<'_>, off: &[String]) -> Cleaned {
    let mut lyrics = Lyrics::parse(text);
    let mut rules = Vec::new();
    for (name, edit) in RULES {
        if off.iter().any(|o| o == name) {
            continue;
        }
        let was = lyrics.clone();
        edit(&mut lyrics, ctx);
        if lyrics != was {
            rules.push(name);
        }
    }
    Cleaned {
        text: lyrics.write(),
        rules,
    }
}

/// `text` cleaned by every rule, knowing nothing of its song.
#[must_use]
pub fn all(text: &str) -> String {
    clean(text, &Context::default(), &[]).text
}

// ---- Lines ----

#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// The times it starts at, as written, and what follows them: its
    /// words, with any word times.
    Timed { stamps: String, text: String },
    /// An ID tag, a blank line or an untimed line of words.
    Other(String),
}

impl Line {
    fn text(&self) -> &str {
        match self {
            Self::Timed { text, .. } | Self::Other(text) => text,
        }
    }

    /// What a timed line sings, without word times; none for any other
    /// line, or one of no words, which ends the line before.
    fn words(&self) -> Option<String> {
        let Self::Timed { text, .. } = self else {
            return None;
        };
        let words = lyrics::without_word_times(text);
        let words = words.trim();
        (!words.is_empty()).then(|| words.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Lyrics(Vec<Line>);

impl Lyrics {
    fn parse(text: &str) -> Self {
        let lines = text
            .lines()
            .map(|line| {
                let mut rest = line;
                let mut timed = false;
                while let Some((_, after)) = rest.strip_prefix('[').and_then(lyrics::lrc_time) {
                    timed = true;
                    rest = after;
                }
                if timed {
                    Line::Timed {
                        stamps: line[..line.len() - rest.len()].to_string(),
                        text: rest.to_string(),
                    }
                } else {
                    Line::Other(line.to_string())
                }
            })
            .collect();
        Self(lines)
    }

    fn write(&self) -> String {
        let mut out = String::new();
        for line in &self.0 {
            if let Line::Timed { stamps, .. } = line {
                out.push_str(stamps);
            }
            out.push_str(line.text());
            out.push('\n');
        }
        out
    }

    fn drop_lines(&mut self, doomed: &BTreeSet<usize>) {
        let mut i = 0;
        self.0.retain(|_| {
            i += 1;
            !doomed.contains(&(i - 1))
        });
    }
}

// ---- Tidy ----

/// `line` without the markup subtitles carry into LRC: an HTML-like tag,
/// as `<i>` or `</font>`, and an ASS override, as `{\an8}`. A word time,
/// `<00:12.34>`, is no markup. A closing `>` or `}` found missing once is
/// not looked for again, so a line of a thousand `<a` reads in one pass.
fn unmarked(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    let mut unclosed = Vec::new();
    while let Some(at) = rest.find(['<', '{']) {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let close = if tail.starts_with('<') { '>' } else { '}' };
        let markup = match tail[1..].chars().next() {
            Some(c) if close == '>' => c.is_ascii_alphabetic() || c == '/',
            Some(c) => c == '\\',
            None => false,
        };
        let end = (markup && !unclosed.contains(&close))
            .then(|| tail.find(close))
            .flatten();
        if markup && end.is_none() {
            unclosed.push(close);
        }
        if let Some(end) = end {
            rest = &tail[end + 1..];
        } else {
            out.push_str(&tail[..1]);
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

fn markup(l: &mut Lyrics, _: &Context<'_>) {
    for line in &mut l.0 {
        match line {
            Line::Timed { text, .. } | Line::Other(text) => *text = unmarked(text),
        }
    }
}

// ---- Packaging ----

/// The timed lines of words, with where each stands among the lines, and
/// what each is.
fn kinds(l: &Lyrics) -> Vec<(usize, Kind)> {
    l.0.iter()
        .enumerate()
        .filter_map(|(i, line)| Some((i, lyrics::kind(&line.words()?))))
        .collect()
}

/// Timed lines that are cues.
fn cues(l: &mut Lyrics, _: &Context<'_>) {
    let doomed = kinds(l)
        .into_iter()
        .filter(|(_, k)| *k == Kind::Cue)
        .map(|(i, _)| i)
        .collect();
    l.drop_lines(&doomed);
}

/// Timed credits before the first line sung or after the last.
fn credits(l: &mut Lyrics, _: &Context<'_>) {
    let found = kinds(l);
    let all: Vec<Kind> = found.iter().map(|(_, k)| *k).collect();
    let doomed = found
        .iter()
        .zip(lyrics::unsung(&all))
        .filter(|((_, k), unsung)| *k == Kind::Credit && *unsung)
        .map(|((i, _), _)| *i)
        .collect();
    l.drop_lines(&doomed);
}

#[cfg(test)]
mod tests;
