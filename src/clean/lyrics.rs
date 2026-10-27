//! What cleaning takes out of lyrics, and what it mends in them, by
//! rules in a fixed order. Each rule is a switch in the song list's
//! `[clean.lyrics.<tier>]`, its tier saying how far it may stray, as the
//! tag rules' tiers say in [`super::TIERS`].
//!
//! Measured on a library of about two thousand songs, lyrics from
//! LRCLIB came with a space after each time and at the ends of lines;
//! lyrics converted from subtitles came with markup, `\h`, zero-width
//! spaces around every line, one line written again for every frame it
//! showed, lines typed out as they grow, and credits and the song's
//! title around the lyrics. A few LRCLIB records carried a Cyrillic
//! `е` inside English words, one in about twenty-five lines, as a
//! scraper's mark.
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
//! [`crate::lyrics`], which matching reads too. Here a line may also be
//! a *heading*, the song's title or its artist restated before the
//! lyrics or after them. A heading is told only by its neighbours: a
//! first line that is the title is sung, as many songs open with their
//! title, unless the lines around it are credits, it is quoted as the
//! title, or it names the artist with it.
//!
//! What the rules leave alone matters as much as what they take:
//!
//! - emoji and kaomoji an uploader drew, `♪` beside words, all-caps and
//!   self-censored words: they are the text;
//! - two lines at one time, as a subtitle of two lines is;
//! - a line repeated 400 ms after itself or later, as a stutter is
//!   sung; a run of one line under 250 ms apart is one subtitle's frames;
//! - lines that grow from LRCLIB, where `I want you`, `I want you so
//!   bad` are sung, not typed;
//! - the ideographic space halving a CJK line, a zero-width joiner and
//!   non-joiner, and the marks that order right-to-left text in it;
//! - a letter of another script in a word that is not mostly of one
//!   script, as `極Chu` or `重音TETO`, or one that looks like no letter of
//!   the word's own script.

use std::collections::{BTreeMap, BTreeSet};

use icu_properties::CodePointMapData;
use icu_properties::props::{GeneralCategory, Script};
use unicode_security::confusable_detection::skeleton;
use unicode_security::mixed_script::is_potential_mixed_script_confusable_char;

use super::{QUOTES, Settings, normalize, without_credits};
use crate::lyrics::{self, Kind};

/// What the rules write, raised whenever a rule writes otherwise than
/// before, so every song with lyrics is written again.
pub const EDITION: u32 = 2;

/// One rule: what it does to the lyrics, knowing the song.
type Edit = fn(&mut Lyrics, &Context<'_>);

/// In the order run: markup goes before spacing, so the spaces a tag
/// leaves are tidied; a heading is read while the credits beside it
/// still say what it is; lines are put in order before frames merge.
const RULES: [(&str, Edit); 10] = [
    ("lyrics.tidy.markup", markup),
    ("lyrics.tidy.spacing", spacing),
    ("lyrics.tidy.homoglyphs", homoglyphs),
    ("lyrics.packaging.cues", cues),
    ("lyrics.packaging.title", title),
    ("lyrics.packaging.credits", credits),
    ("lyrics.packaging.headers", headers),
    ("lyrics.structure.order", order),
    ("lyrics.structure.frames", frames),
    ("lyrics.guesswork.typing", typing),
];

/// The rules that read the song's title and artists.
pub const NAMED: [&str; 1] = ["lyrics.packaging.title"];

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

/// What the rules may know of the song: its title and artists, for a
/// heading, and whether the lyrics were a subtitle stream's.
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
    Timed {
        stamps: String,
        ms: Vec<i64>,
        text: String,
    },
    /// An ID tag, a blank line or an untimed line of words.
    Other(String),
}

impl Line {
    fn text(&self) -> &str {
        match self {
            Self::Timed { text, .. } | Self::Other(text) => text,
        }
    }

    fn text_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::Timed { text, .. } => Some(text),
            Self::Other(text) => (!is_id_tag(text)).then_some(text),
        }
    }

    /// What it sings, without word times; none for an ID tag or a line
    /// of no words.
    fn words(&self) -> Option<String> {
        let words = match self {
            Self::Timed { text, .. } => lyrics::without_word_times(text),
            Self::Other(text) if is_id_tag(text) => return None,
            Self::Other(text) => text.clone(),
        };
        let words = words.trim();
        (!words.is_empty()).then(|| words.to_string())
    }

    /// Its one time, for a line that starts once.
    fn start(&self) -> Option<i64> {
        match self {
            Self::Timed { ms, .. } if ms.len() == 1 => ms.first().copied(),
            _ => None,
        }
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
                let mut ms = Vec::new();
                while let Some((t, after)) = rest.strip_prefix('[').and_then(lyrics::lrc_time) {
                    ms.push(t);
                    rest = after;
                }
                if ms.is_empty() {
                    Line::Other(line.to_string())
                } else {
                    Line::Timed {
                        stamps: line[..line.len() - rest.len()].to_string(),
                        ms,
                        text: rest.to_string(),
                    }
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

    fn timed(&self) -> bool {
        self.0.iter().any(|l| matches!(l, Line::Timed { .. }))
    }

    fn edit_texts(&mut self, mut edit: impl FnMut(&str) -> String) {
        for line in &mut self.0 {
            if let Some(text) = line.text_mut() {
                *text = edit(text);
            }
        }
    }
}

/// The keys of the ID tags an `.lrc` may open with.
const ID_TAGS: [&str; 12] = [
    "ar", "ti", "al", "au", "by", "re", "ve", "length", "offset", "la", "id", "#",
];

/// The ID tags of a player or a transcriber, not of the song.
const MAKERS: [&str; 5] = ["by", "re", "ve", "id", "#"];

fn id_key(line: &str) -> Option<String> {
    let inner = line.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (key, _) = inner.split_once(':')?;
    let key = key.trim().to_ascii_lowercase();
    ID_TAGS.contains(&key.as_str()).then_some(key)
}

fn is_id_tag(line: &str) -> bool {
    id_key(line).is_some()
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
    l.edit_texts(|text| {
        let text = unmarked(text)
            .replace("\\h", " ")
            .replace("\\N", " ")
            .replace("\\n", " ");
        html_escape::decode_html_entities(&text).into_owned()
    });
}

fn category(c: char) -> GeneralCategory {
    CodePointMapData::<GeneralCategory>::new().get(c)
}

fn script(c: char) -> Script {
    match CodePointMapData::<Script>::new().get(c) {
        // Japanese writes one word in all three.
        Script::Hiragana | Script::Katakana => Script::Han,
        s => s,
    }
}

fn is_right_to_left(c: char) -> bool {
    matches!(
        script(c),
        Script::Arabic | Script::Hebrew | Script::Syriac | Script::Thaana | Script::Nko
    )
}

/// A line's characters as text means them: invisible format characters
/// gone but the joiners, and the marks ordering right-to-left text in a
/// line that holds it; every other space a plain one, but the
/// ideographic space; runs of spaces one, none at either end.
fn tidy_line(text: &str) -> String {
    let rtl = text.chars().any(is_right_to_left);
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let kept = match category(c) {
            GeneralCategory::Format => {
                matches!(c, '\u{200C}' | '\u{200D}')
                    || rtl && matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}')
            }
            GeneralCategory::Control => c == '\t',
            _ => true,
        };
        if !kept {
            continue;
        }
        let spaced = matches!(
            category(c),
            GeneralCategory::SpaceSeparator
                | GeneralCategory::LineSeparator
                | GeneralCategory::ParagraphSeparator
        ) && c != '\u{3000}'
            || c == '\t';
        if spaced {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Each line tidied; blank lines gone from timed lyrics, where a time of
/// no words ends a line instead, and from either end of untimed ones,
/// whose stanzas stay apart by one.
fn spacing(l: &mut Lyrics, _: &Context<'_>) {
    l.edit_texts(tidy_line);
    let timed = l.timed();
    let blank = |line: &Line| matches!(line, Line::Other(t) if t.trim().is_empty());
    let mut kept: Vec<Line> = Vec::new();
    for line in l.0.drain(..) {
        let after_blank = kept.last().is_none_or(blank);
        if blank(&line) && (timed || after_blank) {
            continue;
        }
        kept.push(line);
    }
    while kept.last().is_some_and(blank) {
        kept.pop();
    }
    l.0 = kept;
}

/// The letters of one script the lyrics hold, each with its skeleton,
/// the form Unicode's confusables data gives every letter it looks like.
type Letters = BTreeMap<Script, Vec<(char, String)>>;

fn skeleton_of(c: char) -> String {
    skeleton(c.encode_utf8(&mut [0; 4])).collect()
}

/// A letter of another script in a word mostly of one, swapped for the
/// one letter of the word's script that Unicode's confusables data
/// says it looks like, of the same case: `hеad` with a Cyrillic `е` is
/// `head`. The letters swapped to are those the lyrics hold elsewhere,
/// so a Latin `e` in a Cyrillic word finds the Cyrillic `е` around it.
fn homoglyphs(l: &mut Lyrics, _: &Context<'_>) {
    let mut letters: Letters = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for line in &l.0 {
        for c in line.text().chars().filter(|c| c.is_alphabetic()) {
            if seen.insert(c) {
                letters
                    .entry(script(c))
                    .or_default()
                    .push((c, skeleton_of(c)));
            }
        }
    }
    l.edit_texts(|text| {
        let chars: Vec<char> = text.chars().collect();
        let line = scripts(&chars);
        let mut out = chars.clone();
        let mut start = 0;
        while start < chars.len() {
            let len = chars[start..]
                .iter()
                .take_while(|c| {
                    c.is_alphanumeric() || category(**c) == GeneralCategory::NonspacingMark
                })
                .count();
            mend_word(&mut out[start..start + len], &line, &letters);
            start += len.max(1);
        }
        out.into_iter().collect()
    });
}

/// How many letters of each script `chars` holds, but the scripts
/// shared by all.
fn scripts(chars: &[char]) -> BTreeMap<Script, usize> {
    let mut counts: BTreeMap<Script, usize> = BTreeMap::new();
    for c in chars.iter().filter(|c| c.is_alphabetic()) {
        *counts.entry(script(*c)).or_default() += 1;
    }
    counts.retain(|s, _| !matches!(*s, Script::Common | Script::Inherited));
    counts
}

/// The script most of `counts` is in; a tie goes to the one `line`
/// holds most of.
fn main_script(counts: &BTreeMap<Script, usize>, line: &BTreeMap<Script, usize>) -> Option<Script> {
    let most = counts.values().max()?;
    let tied: Vec<Script> = counts
        .iter()
        .filter(|(_, n)| *n == most)
        .map(|(s, _)| *s)
        .collect();
    if let [one] = tied.as_slice() {
        return Some(*one);
    }
    let in_line = |s: &Script| line.get(s).copied().unwrap_or(0);
    let best = tied.iter().map(in_line).max()?;
    let mut top = tied.iter().filter(|s| in_line(s) == best);
    match (top.next(), top.next()) {
        (Some(one), None) => Some(*one),
        _ => None,
    }
}

fn mend_word(word: &mut [char], line: &BTreeMap<Script, usize>, letters: &Letters) {
    let counts = scripts(word);
    if counts.len() < 2 {
        return;
    }
    let Some(main) = main_script(&counts, line) else {
        return;
    };
    let own = letters.get(&main).map_or(&[][..], Vec::as_slice);
    for c in word.iter_mut() {
        if !c.is_alphabetic()
            || script(*c) == main
            || !is_potential_mixed_script_confusable_char(*c)
        {
            continue;
        }
        let looks = skeleton_of(*c);
        let prototype = looks.chars().next().filter(|p| {
            looks.chars().count() == 1 && script(*p) == main && skeleton_of(*p) == looks
        });
        let alike: BTreeSet<char> = own
            .iter()
            .filter(|(_, s)| *s == looks)
            .map(|(o, _)| *o)
            .chain(prototype)
            .filter(|o| o.is_uppercase() == c.is_uppercase())
            .collect();
        if let [one] = Vec::from_iter(alike).as_slice() {
            *c = *one;
        }
    }
}

// ---- Packaging ----

/// The lines of words, with where each stands among the lyrics' lines.
fn word_lines(l: &Lyrics) -> Vec<(usize, String)> {
    l.0.iter()
        .enumerate()
        .filter_map(|(i, line)| Some((i, line.words()?)))
        .collect()
}

/// Cues dropped, timed or not; a cue opening a line sung, as `(Spoken)`
/// or `[Chorus]`, goes from its front.
fn cues(l: &mut Lyrics, _: &Context<'_>) {
    let doomed: BTreeSet<usize> = word_lines(l)
        .into_iter()
        .filter(|(_, w)| lyrics::kind(w) == Kind::Cue)
        .map(|(i, _)| i)
        .collect();
    drop_lines(l, &doomed);
    l.edit_texts(|text| {
        let t = text.trim_start();
        let opened = lyrics::WRAPPERS.iter().find_map(|(o, c)| {
            let inner = t.strip_prefix(*o)?;
            let close = inner.find(*c)?;
            let rest = &inner[close + c.len_utf8()..];
            (lyrics::is_cue(&inner[..close]) && rest.chars().any(char::is_alphanumeric))
                .then(|| rest.trim_start().to_string())
        });
        opened.unwrap_or_else(|| text.to_string())
    });
}

/// What a line of words is among the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Is {
    Sung,
    Cue,
    Credit,
    /// The song's title or its artist restated around the lyrics.
    Heading,
}

/// `text` without one pair of quotes around all of it.
fn unquoted(text: &str) -> Option<&str> {
    let t = text.trim();
    QUOTES.iter().find_map(|(o, c)| {
        let inner = t.strip_prefix(*o)?.strip_suffix(*c)?;
        (!inner.is_empty() && !inner.contains(*c)).then_some(inner)
    })
}

/// The parts a heading joins its title and artist with.
const HEADING_SEPARATORS: [&str; 8] = [" / ", "／", " - ", " – ", " — ", " | ", "｜", " by "];

/// The parts a list of artists joins its names with.
const ARTIST_SEPARATORS: [&str; 10] = [
    " feat. ",
    " feat ",
    " ft. ",
    " featuring ",
    " & ",
    ", ",
    " x ",
    " × ",
    "、",
    " and ",
];

fn split_all<'t>(text: &'t str, seps: &[&str]) -> Vec<&'t str> {
    let mut parts = vec![text];
    for sep in seps {
        parts = parts.into_iter().flat_map(|p| p.split(sep)).collect();
    }
    parts
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

impl Context<'_> {
    fn is_title(&self, text: &str) -> bool {
        let n = normalize(text);
        !n.is_empty()
            && self
                .title
                .is_some_and(|t| n == normalize(t) || n == normalize(without_credits(t)))
    }

    /// Whether `text` names the song's artists and nothing else.
    fn is_artist(&self, text: &str) -> bool {
        let names: Vec<String> = self.artists.iter().map(|a| normalize(a)).collect();
        let parts = split_all(text, &ARTIST_SEPARATORS);
        !parts.is_empty()
            && parts.iter().all(|p| {
                let n = normalize(p);
                !n.is_empty() && names.contains(&n)
            })
    }

    /// Whether `text` names the title and the artist, as `Title - Artist`.
    fn is_title_and_artist(&self, text: &str) -> bool {
        let parts = split_all(text, &HEADING_SEPARATORS);
        parts.len() >= 2
            && parts.iter().enumerate().any(|(i, p)| {
                self.is_title(p)
                    && parts
                        .iter()
                        .enumerate()
                        .any(|(j, q)| i != j && self.is_artist(q))
            })
    }
}

/// What each line of words is, by [`lyrics::kind`] and, at the start and
/// the end, by the song: a block of lines before the first line sung,
/// or after the last, is a heading block when it holds a credit, the
/// artist or the title quoted, and then each line in it that is the
/// title, quoted text or the artist is a heading. At the end only a
/// line naming both title and artist is, as a song may end on its title.
fn classify(words: &[(usize, String)], ctx: &Context<'_>) -> Vec<Is> {
    let mut kinds: Vec<Is> = words
        .iter()
        .map(|(_, w)| match lyrics::kind(w) {
            Kind::Sung => Is::Sung,
            Kind::Cue => Is::Cue,
            Kind::Credit => Is::Credit,
        })
        .collect();
    for i in 1..words.len() {
        if kinds[i] == Is::Sung && kinds[i - 1] == Is::Credit && lyrics::dangles(&words[i - 1].1) {
            kinds[i] = Is::Credit;
        }
    }
    let opening = |w: &str| {
        ctx.is_title_and_artist(w) || ctx.is_artist(w) || ctx.is_title(w) || unquoted(w).is_some()
    };
    let closing = |w: &str| ctx.is_title_and_artist(w) || ctx.is_artist(w);
    let strong = |w: &str| {
        ctx.is_title_and_artist(w)
            || ctx.is_artist(w)
            || unquoted(w).is_some_and(|q| ctx.is_title(q))
    };
    for (range, shaped) in [
        (
            (0..words.len()).collect::<Vec<_>>(),
            &opening as &dyn Fn(&str) -> bool,
        ),
        ((0..words.len()).rev().collect(), &closing),
    ] {
        let block: Vec<usize> = range
            .into_iter()
            .take_while(|&i| kinds[i] != Is::Sung || shaped(&words[i].1))
            .collect();
        let heads = block
            .iter()
            .any(|&i| kinds[i] == Is::Credit || strong(&words[i].1));
        for i in block {
            if heads && kinds[i] == Is::Sung {
                kinds[i] = Is::Heading;
            }
        }
    }
    kinds
}

/// The lines of `is` that stand before the first line sung or after the
/// last.
fn at_edges(l: &Lyrics, ctx: &Context<'_>, is: Is) -> BTreeSet<usize> {
    let words = word_lines(l);
    let kinds = classify(&words, ctx);
    let first = kinds.iter().position(|k| *k == Is::Sung);
    let last = kinds.iter().rposition(|k| *k == Is::Sung);
    words
        .iter()
        .zip(&kinds)
        .enumerate()
        .filter(|(n, (_, k))| {
            **k == is
                && match (first, last) {
                    (Some(first), Some(last)) => *n < first || *n > last,
                    _ => true,
                }
        })
        .map(|(_, ((i, _), _))| *i)
        .collect()
}

/// `l` without the lines `doomed`, and without the blank lines that
/// leaves at either end or doubled where a line went.
fn drop_lines(l: &mut Lyrics, doomed: &BTreeSet<usize>) {
    if doomed.is_empty() {
        return;
    }
    let blank = |line: &Line| matches!(line, Line::Other(t) if t.trim().is_empty());
    let lines = std::mem::take(&mut l.0);
    let mut gone = false;
    for (i, line) in lines.into_iter().enumerate() {
        if doomed.contains(&i) {
            gone = true;
            continue;
        }
        if blank(&line) && gone && l.0.last().is_none_or(blank) {
            continue;
        }
        gone = gone && blank(&line);
        l.0.push(line);
    }
    while gone && l.0.last().is_some_and(blank) {
        l.0.pop();
    }
}

/// The song's title and artist restated before the lyrics or after them.
fn title(l: &mut Lyrics, ctx: &Context<'_>) {
    let doomed = at_edges(l, ctx, Is::Heading);
    drop_lines(l, &doomed);
}

/// Credits: those before the first line sung or after the last, and
/// those no line sung is shaped as, or that run two lines or more,
/// wherever they stand, as a video credits its makers after its intro.
fn credits(l: &mut Lyrics, ctx: &Context<'_>) {
    let mut doomed = at_edges(l, ctx, Is::Credit);
    let words = word_lines(l);
    let kinds = classify(&words, ctx);
    for (n, ((i, w), k)) in words.iter().zip(&kinds).enumerate() {
        let credit = |m: usize| kinds.get(m) == Some(&Is::Credit);
        let run = n.checked_sub(1).is_some_and(credit) || credit(n + 1);
        if *k == Is::Credit && (run || lyrics::is_sure_credit(w)) {
            doomed.insert(*i);
        }
    }
    drop_lines(l, &doomed);
}

/// No ID tag in untimed lyrics, where a player shows it as a line; in
/// timed ones, none naming who timed them or with what.
fn headers(l: &mut Lyrics, _: &Context<'_>) {
    let timed = l.timed();
    l.0.retain(|line| match line {
        Line::Other(text) => match id_key(text) {
            Some(key) => timed && !MAKERS.contains(&key.as_str()),
            None => true,
        },
        Line::Timed { .. } => true,
    });
}

// ---- Structure ----

/// Timed lines in the order of their first times, each in a place a
/// timed line held, so ID tags stay where they stand.
fn order(l: &mut Lyrics, _: &Context<'_>) {
    let places: Vec<usize> = (0..l.0.len())
        .filter(|&i| matches!(l.0[i], Line::Timed { .. }))
        .collect();
    let mut timed: Vec<Line> = places.iter().map(|&i| l.0[i].clone()).collect();
    timed.sort_by_key(|line| match line {
        Line::Timed { ms, .. } => ms.iter().min().copied(),
        Line::Other(_) => None,
    });
    for (i, line) in places.into_iter().zip(timed) {
        l.0[i] = line;
    }
}

/// How soon a line may follow itself and be another frame of one
/// subtitle: frames come 33 ms apart, LRCLIB's slips 80 to 110 ms, and
/// the quickest stutter sung was 400 ms.
const FRAME_MS: i64 = 250;

/// A line of a subtitle written again in the next one, starting under
/// [`FRAME_MS`] after it, as a frame of the line at its first time. A
/// subtitle is the lines of words that start at one time together, so
/// one of two lines repeats though the other changed; a line it holds
/// twice, as layered subtitles write one, it holds once. A time of no
/// words between two frames ends nothing, nor does one at a time words
/// start at, or one after another.
fn frames(l: &mut Lyrics, _: &Context<'_>) {
    let mut doomed = BTreeSet::new();
    let mut groups: Vec<(i64, Vec<usize>, Vec<String>)> = Vec::new();
    for (i, line) in l.0.iter().enumerate() {
        match (line.start(), line.words()) {
            (Some(at), Some(words)) => match groups.last_mut() {
                Some((start, _, texts)) if *start == at && texts.contains(&words) => {
                    doomed.insert(i);
                }
                Some((start, lines, texts)) if *start == at => {
                    lines.push(i);
                    texts.push(words);
                }
                _ => groups.push((at, vec![i], vec![words])),
            },
            (None, _) if matches!(line, Line::Timed { .. }) => {
                groups.push((i64::MIN, Vec::new(), Vec::new()));
            }
            _ => {}
        }
    }
    for pair in groups.windows(2) {
        let ((before, _, was), (at, lines, texts)) = (&pair[0], &pair[1]);
        if (0..FRAME_MS).contains(&(at.saturating_sub(*before))) {
            for (i, text) in lines.iter().zip(texts) {
                if was.contains(text) {
                    doomed.insert(*i);
                }
            }
        }
    }
    let timed: Vec<usize> = (0..l.0.len())
        .filter(|&i| matches!(l.0[i], Line::Timed { .. }) && !doomed.contains(&i))
        .collect();
    let blank = |i: usize| matches!(&l.0[i], Line::Timed { .. }) && l.0[i].words().is_none();
    let starts = |i: usize| match &l.0[i] {
        Line::Timed { ms, .. } => ms.first().copied(),
        Line::Other(_) => None,
    };
    let mut ended = None;
    for (n, &i) in timed.iter().enumerate() {
        if !blank(i) {
            ended = Some(false);
            continue;
        }
        let next = timed.get(n + 1).copied();
        let before_frame = (i + 1..l.0.len())
            .find(|&j| matches!(l.0[j], Line::Timed { .. }) && !blank(j))
            .is_some_and(|j| doomed.contains(&j));
        let shared = [n.checked_sub(1).map(|m| timed[m]), next]
            .into_iter()
            .flatten()
            .any(|j| !blank(j) && starts(j) == starts(i));
        if ended == Some(true) || before_frame || shared {
            doomed.insert(i);
        } else {
            ended = Some(true);
        }
    }
    drop_lines(l, &doomed);
}

// ---- Guesswork ----

/// How long a subtitle may take to type a line out.
const TYPING_MS: i64 = 2000;

/// In a subtitle stream's lyrics, a run of three lines or more, each
/// following the one before within [`TYPING_MS`], starting with all of
/// it and going on, is one line typed out: the run is its last line, at
/// its first line's time. Two such lines are a line sung and sung again
/// longer, as `Love?` and `Love? That's useless`.
fn typing(l: &mut Lyrics, ctx: &Context<'_>) {
    if !ctx.subtitle {
        return;
    }
    let timed: Vec<usize> = (0..l.0.len())
        .filter(|&i| matches!(l.0[i], Line::Timed { .. }))
        .collect();
    let grows = |i: usize, j: usize| {
        let (Some(was), Some(now)) = (l.0[i].start(), l.0[j].start()) else {
            return false;
        };
        let (Some(line), Some(longer)) = (l.0[i].words(), l.0[j].words()) else {
            return false;
        };
        longer.len() > line.len()
            && longer.starts_with(&line)
            && (0..TYPING_MS).contains(&(now - was))
    };
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for pair in timed.windows(2) {
        let (i, j) = (pair[0], pair[1]);
        if !grows(i, j) {
            continue;
        }
        match runs.last_mut() {
            Some(run) if run.last() == Some(&i) => run.push(j),
            _ => runs.push(vec![i, j]),
        }
    }
    let mut doomed = BTreeSet::new();
    for run in runs.into_iter().filter(|r| r.len() >= 3) {
        let (first, last) = (run[0], run[run.len() - 1]);
        if let (
            Line::Timed { stamps, ms, .. },
            Line::Timed {
                stamps: s, ms: m, ..
            },
        ) = (l.0[first].clone(), &mut l.0[last])
        {
            *s = stamps;
            *m = ms;
        }
        doomed.extend(&run[..run.len() - 1]);
    }
    drop_lines(l, &doomed);
}

#[cfg(test)]
mod tests;
