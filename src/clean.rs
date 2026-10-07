//! What cleaning takes out of the tags a source offers: what a store or
//! an uploader wrapped around the song's name, read by rules in a fixed
//! order. Each rule is a switch in the song list's `[clean.<tier>]`, its
//! tier saying how far it may stray from the value it was given.
//!
//! Cleaning is what lets `Song (Album Version)` agree with `Song`, and a
//! title read off a video compete as the song's name. Measured on uploads
//! matched to their releases, under two in five titles agreed with the
//! release before cleaning, and all did after, romanization aside.
//!
//! Offers are cleaned at each resolve, never when read: the raw offers
//! stay in the state, so changing a rule or a switch needs nothing read
//! again, and a song whose tags it changes renders again. Tags set in
//! a song's or an album's `tags` are never cleaned.
//!
//! A *structured* value comes from a field kept for it: a release's track
//! and artists, a file's own tags. A *derived* one is read off a video's
//! title or its channel's name, and only a derived title is split by
//! convention. Its credit goes when one side of a ` - ` or ` / ` names
//! someone the song's sources credit; with neither side named, a slash
//! keeps its left side, a list of names is the credit, and a dash keeps
//! its right side, YouTube's `Artist - Title`. With both sides named, or
//! a right side that only qualifies, as ` - Acoustic Version`, the title
//! stays whole.
//!
//! What the rules leave alone matters as much as what they take:
//!
//! - artists are split at `;` only, since a comma belongs to names such
//!   as `Venn, Hale & Orchard`; genres split at `,`, `;` and `/`, never at
//!   `&`, as in `Rock & Roll`;
//! - a remaster note goes from a title, never from an album, whose
//!   remaster is another release;
//! - a bracket naming the video stays when it also says live, demo, mix,
//!   edit, acoustic, instrumental, cover or karaoke: that names another
//!   recording;
//! - an alias after a slash goes from a derived artist always, a channel
//!   having one owner, but from a structured one only when the two read as
//!   one person's, so a duet `Paper Comets / Marlo Venn` stays whole;
//! - the ideographic space, and symbols below U+2700 such as `☆` and `♪`,
//!   are part of a name.

use std::collections::BTreeSet;

use crate::tags::{Field, Offer, Offers};

/// The tiers, least invasive first, each with what its rules may do.
pub const TIERS: [(&str, &str); 4] = [
    ("tidy", "Spacing and punctuation only; never removes a word"),
    (
        "packaging",
        "Removes words that name the release or the video, not the recording",
    ),
    (
        "structure",
        "Splits a whole value, or swaps it for another, by its shape and the library",
    ),
    (
        "guesswork",
        "Reads a video's title by convention and known names; can misread",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Any,
    /// A value from a field kept for it.
    Structured,
    /// A value read off a video's title or channel.
    Derived,
}

/// One value in, its replacements out; `None` leaves it be.
type Edit = fn(&str, &At<'_>) -> Option<Vec<String>>;

struct Rule {
    /// `<tier>.<key>`, as the song list switches it.
    name: &'static str,
    fields: &'static [Field],
    scope: Scope,
    edit: Edit,
}

const TEXT: &[Field] = &[
    Field::Title,
    Field::Artist,
    Field::Album,
    Field::AlbumArtist,
    Field::Genre,
];
const TITLE: &[Field] = &[Field::Title];
const NAMED: &[Field] = &[Field::Title, Field::Album];
const ARTISTS: &[Field] = &[Field::Artist, Field::AlbumArtist];

/// In the order run: packaging goes before a title is split, so a
/// ` - Full Song` tail is never read as an artist.
const RULES: &[Rule] = &[
    rule("tidy.spacing", TEXT, Scope::Any, spacing),
    rule("tidy.brackets", TEXT, Scope::Any, brackets),
    rule("tidy.contractions", NAMED, Scope::Any, contractions),
    rule("packaging.album_version", TITLE, Scope::Any, album_version),
    rule("packaging.explicit", NAMED, Scope::Any, explicit),
    rule("packaging.remaster", TITLE, Scope::Any, remaster),
    rule("packaging.restated", NAMED, Scope::Any, restated),
    rule("packaging.video_frame", TITLE, Scope::Derived, video_frame),
    rule("packaging.video_tags", TITLE, Scope::Derived, video_tags),
    rule("packaging.emoji", TITLE, Scope::Derived, emoji),
    rule("packaging.pipe_tail", TITLE, Scope::Derived, pipe_tail),
    rule(
        "guesswork.title_artist",
        TITLE,
        Scope::Derived,
        title_artist,
    ),
    rule("guesswork.credits", TITLE, Scope::Derived, credits),
    rule("packaging.quotes", TITLE, Scope::Derived, quotes),
    rule(
        "structure.channel_alias",
        ARTISTS,
        Scope::Any,
        channel_alias,
    ),
    rule(
        "packaging.channel_suffix",
        &[Field::Artist],
        Scope::Derived,
        channel_suffix,
    ),
    rule(
        "structure.joined_artists",
        ARTISTS,
        Scope::Structured,
        joined_artists,
    ),
    rule(
        "structure.album_as_artist",
        &[Field::Artist],
        Scope::Structured,
        album_as_artist,
    ),
    rule(
        "structure.disc_in_album",
        &[Field::Album],
        Scope::Structured,
        disc_in_album,
    ),
    rule(
        "structure.joined_genres",
        &[Field::Genre],
        Scope::Any,
        joined_genres,
    ),
];

const fn rule(name: &'static str, fields: &'static [Field], scope: Scope, edit: Edit) -> Rule {
    Rule {
        name,
        fields,
        scope,
        edit,
    }
}

/// The rule whose disc number the album's name gave up.
pub const DISC_IN_ALBUM: &str = "structure.disc_in_album";

/// Every switch as `(tier, key)`, grouped by tier in the order of
/// `TIERS`, and within one in the order run.
pub fn switches() -> impl Iterator<Item = (&'static str, &'static str)> {
    TIERS.iter().flat_map(|(tier, _)| {
        RULES
            .iter()
            .filter_map(|r| r.name.split_once('.'))
            .filter(move |(t, _)| t == tier)
    })
}

/// Which rules the song list switched off.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    off: BTreeSet<&'static str>,
    /// Keys under `[clean]` naming no rule, as written.
    pub unknown: Vec<String>,
}

impl Settings {
    /// Turn the rule named `<tier>.<key>` on or off; whether there is one.
    pub fn set(&mut self, name: &str, on: bool) -> bool {
        let Some(rule) = RULES.iter().find(|r| r.name == name) else {
            return false;
        };
        if on {
            self.off.remove(rule.name);
        } else {
            self.off.insert(rule.name);
        }
        true
    }

    #[must_use]
    pub fn is_on(&self, name: &str) -> bool {
        !self.off.contains(name)
    }

    /// Every rule off.
    #[must_use]
    pub fn none() -> Self {
        Self {
            off: RULES.iter().map(|r| r.name).collect(),
            unknown: Vec::new(),
        }
    }
}

/// A name some source credits, as compared with part of a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    normalized: String,
    initials: Option<String>,
}

impl Name {
    fn of(s: &str) -> Self {
        Self {
            normalized: normalize(s),
            initials: initials(s),
        }
    }
}

/// The names a song's sources credit, each also without a channel's
/// suffix and split into the aliases a slash joins.
pub fn names<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<Name> {
    let mut out: Vec<Name> = Vec::new();
    for v in values {
        let mut forms = vec![v, without_channel_suffix(v).unwrap_or(v)];
        if let Some((a, b)) = alias_split(v) {
            forms.extend([a, b]);
        }
        for name in forms.into_iter().map(Name::of) {
            if name.normalized.chars().count() >= 2 && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// Every album artist with each album of theirs the library holds, by
/// the fields kept for them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Albums(BTreeSet<(String, String)>);

impl Albums {
    pub fn of<'a>(offers: impl IntoIterator<Item = &'a Offers>) -> Self {
        let mut set = BTreeSet::new();
        for o in offers {
            let (Some(artist), Some(album)) = (o.get(&Field::AlbumArtist), o.get(&Field::Album))
            else {
                continue;
            };
            if !(artist.structured && album.structured) {
                continue;
            }
            for a in &artist.values {
                for b in &album.values {
                    set.insert((normalize(a), normalize(b)));
                }
            }
        }
        Self(set)
    }

    fn has(&self, artist: &str, album: &str) -> bool {
        self.0.contains(&(artist.to_string(), album.to_string()))
    }
}

/// What a rule may look at besides the value: the song's names, the
/// library's albums, and the source's other offers.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    pub names: &'a [Name],
    pub albums: &'a Albums,
    pub offers: &'a Offers,
}

struct At<'a> {
    ctx: &'a Context<'a>,
    structured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    pub values: Vec<String>,
    /// The rules that changed a value, in the order run.
    pub rules: Vec<&'static str>,
}

/// An offer cleaned by every rule switched on for its field. A rule that
/// would leave nothing of a value leaves it as it was.
#[must_use]
pub fn clean(field: Field, offer: &Offer, ctx: &Context<'_>, settings: &Settings) -> Cleaned {
    let at = At {
        ctx,
        structured: offer.structured,
    };
    let mut values = offer.values.clone();
    let mut rules = Vec::new();
    for rule in RULES {
        let admitted = match rule.scope {
            Scope::Any => true,
            Scope::Structured => offer.structured,
            Scope::Derived => !offer.structured,
        };
        if !admitted || !rule.fields.contains(&field) || !settings.is_on(rule.name) {
            continue;
        }
        let mut next = Vec::new();
        let mut changed = false;
        for v in &values {
            let raw = (rule.edit)(v, &at).unwrap_or_default();
            let out: Vec<String> = raw
                .iter()
                .map(|o| tidy(o))
                .filter(|o| !o.is_empty())
                .collect();
            // Any rule leaves its own spacing tidy, so only a tidy rule's
            // raw output tells whether it changed something itself.
            let same = if rule.name.starts_with("tidy.") {
                raw == [v.clone()]
            } else {
                out == [tidy(v)]
            };
            if out.is_empty() || same {
                next.push(v.clone());
            } else {
                changed = true;
                next.extend(out);
            }
        }
        if changed {
            rules.push(rule.name);
            values = next.into_iter().fold(Vec::new(), |mut seen, v| {
                if !seen.contains(&v) {
                    seen.push(v);
                }
                seen
            });
        }
    }
    Cleaned { values, rules }
}

// ---- Scanning ----

const BRACKETS: [(char, char); 6] = [
    ('(', ')'),
    ('[', ']'),
    ('（', '）'),
    ('【', '】'),
    ('「', '」'),
    ('『', '』'),
];

const QUOTES: [(char, char); 6] = [
    ('"', '"'),
    ('\'', '\''),
    ('“', '”'),
    ('‘', '’'),
    ('「', '」'),
    ('『', '』'),
];

const SLASHES: [&str; 3] = [" / ", "／", " ⧸ "];
const DASHES: [&str; 3] = [" - ", " – ", " — "];

/// A bracketed stretch, by byte offsets: from its opening bracket to
/// just past its closing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
    open: char,
}

impl Span {
    fn close(self) -> char {
        BRACKETS
            .iter()
            .find(|(o, _)| *o == self.open)
            .map_or(')', |(_, c)| *c)
    }

    fn inner(self, s: &str) -> &str {
        &s[self.start + self.open.len_utf8()..self.end - self.close().len_utf8()]
    }

    /// A round or square bracket, where a qualifier is written.
    fn is_qualifier(self) -> bool {
        matches!(self.open, '(' | '[' | '（')
    }
}

/// The brackets not inside another, nesting-aware; one left open
/// encloses nothing.
fn spans(s: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut open: Vec<(usize, char)> = Vec::new();
    for (i, c) in s.char_indices() {
        if BRACKETS.iter().any(|(o, _)| *o == c) {
            open.push((i, c));
        } else if let Some((o, _)) = BRACKETS.iter().find(|(_, close)| *close == c)
            && let Some(at) = open.iter().rposition(|(_, oc)| oc == o)
        {
            let (start, _) = open[at];
            open.truncate(at);
            if open.is_empty() {
                out.push(Span {
                    start,
                    end: i + c.len_utf8(),
                    open: *o,
                });
            }
        }
    }
    out
}

fn inside(spans: &[Span], at: usize) -> bool {
    spans.iter().any(|s| s.start <= at && at < s.end)
}

/// The first of `seps` outside every bracket, and which.
fn find_top<'s>(s: &str, seps: &[&'s str]) -> Option<(usize, &'s str)> {
    let sp = spans(s);
    seps.iter()
        .flat_map(|sep| s.match_indices(sep).map(move |(i, _)| (i, *sep)))
        .filter(|(i, _)| !inside(&sp, *i))
        .min_by_key(|(i, _)| *i)
}

/// The last of `seps` outside every bracket, and which.
fn rfind_top<'s>(s: &str, seps: &[&'s str]) -> Option<(usize, &'s str)> {
    let sp = spans(s);
    seps.iter()
        .flat_map(|sep| s.match_indices(sep).map(move |(i, _)| (i, *sep)))
        .filter(|(i, _)| !inside(&sp, *i))
        .max_by_key(|(i, _)| *i)
}

/// `s` without the stretches given.
fn cut(s: &str, doomed: &[Span]) -> String {
    let mut out = String::new();
    let mut from = 0;
    for d in doomed {
        out.push_str(&s[from..d.start]);
        out.push(' ');
        from = d.end;
    }
    out.push_str(&s[from..]);
    out
}

fn is_space(c: char) -> bool {
    c.is_whitespace() && c != '\u{3000}'
}

fn collapse(s: &str) -> String {
    s.split(is_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a rule leaves, tidied: spaces collapsed, none just inside a
/// bracket, no empty bracket.
fn tidy(s: &str) -> String {
    let mut t = collapse(s);
    loop {
        let mut next = t.clone();
        for (o, c) in BRACKETS {
            next = next
                .replace(&format!("{o} "), &o.to_string())
                .replace(&format!(" {c}"), &c.to_string())
                .replace(&format!("{o}{c}"), "");
        }
        let next = collapse(&next);
        if next == t {
            return t;
        }
        t = next;
    }
}

/// Lower-cased runs of letters and digits.
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_cjk(c: char) -> bool {
    matches!(u32::from(c),
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF | 0xAC00..=0xD7AF | 0xFF66..=0xFF9F)
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(is_cjk)
}

/// Letters only from the Latin alphabets, and at least one.
fn latin_only(s: &str) -> bool {
    let mut letters = s.chars().filter(|c| c.is_alphabetic()).peekable();
    letters.peek().is_some() && letters.all(|c| u32::from(c) <= 0x24F)
}

/// The first letter of each word, splitting at spaces, punctuation and a
/// change from lower to upper case: `PaperBoatsWander` is `pbw`.
fn initials(s: &str) -> Option<String> {
    let mut letters = String::new();
    let mut prev: Option<char> = None;
    for c in s.chars() {
        let starts = c.is_alphanumeric()
            && prev.is_none_or(|p| !p.is_alphanumeric() || (p.is_lowercase() && c.is_uppercase()));
        if starts {
            letters.extend(c.to_lowercase());
        }
        prev = Some(c);
    }
    (letters.chars().count() >= 3 && letters.is_ascii()).then_some(letters)
}

/// Whether a part of a title names someone the song's sources credit:
/// it is the name, holds a name of four or more letters, or abbreviates
/// one. A name holding the part is not enough, so `Kiri` is no credit
/// for `Kiri Hoshino`.
fn names_someone(side: &str, names: &[Name]) -> bool {
    let n = normalize(side);
    if n.is_empty() {
        return false;
    }
    let trimmed = side.trim();
    let caps = (trimmed.chars().count() >= 3 && trimmed.chars().all(|c| c.is_ascii_uppercase()))
        .then(|| trimmed.to_ascii_lowercase());
    let spelled = initials(side);
    names.iter().any(|name| {
        n == name.normalized
            || (name.normalized.chars().count() >= 4 && n.contains(&name.normalized))
            || caps.is_some() && caps == name.initials
            || spelled.as_ref() == Some(&name.normalized)
    })
}

/// A list of several names: `A & B`, `A x B`, `A・B`.
fn is_name_list(s: &str) -> bool {
    s.contains(['&', '・', '、', '+', '×', ',']) || s.to_lowercase().contains(" x ")
}

// ---- Packaging words ----

/// Words that mark a bracket as a video's packaging, not the song's name.
const PACKAGING: [&str; 19] = [
    "official",
    "mv",
    "m/v",
    "pv",
    "video",
    "lyric",
    "lyrics",
    "audio",
    "hd",
    "hq",
    "4k",
    "visualizer",
    "subtitles",
    "subtitle",
    "subs",
    "sub",
    "reupload",
    "kara",
    "256k",
];

/// Phrases that do the same across words.
const PACKAGING_PHRASES: [&str; 2] = ["full ver", "full song"];

/// Words that name another recording, which no packaging word outweighs.
const KEEP: [&str; 11] = [
    "live",
    "demo",
    "remix",
    "mix",
    "edit",
    "acoustic",
    "instrumental",
    "inst",
    "cover",
    "karaoke",
    "remastered",
];

/// Whether a bracket's text names a video's packaging, as `Official
/// Video` or `Lyrics`, and no other recording.
#[must_use]
pub fn is_packaging(inner: &str) -> bool {
    let lower = inner.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '/')
        .filter(|w| !w.is_empty())
        .collect();
    let packaged = PACKAGING.iter().any(|p| words.contains(p))
        || PACKAGING_PHRASES.iter().any(|p| lower.contains(p));
    packaged && !KEEP.iter().any(|k| words.contains(k))
}

// ---- Credits ----

/// What opens a credit to featured artists, lower-case.
const MARKERS: [&str; 5] = ["feat.", "feat ", "ft.", "ft ", "featuring "];

fn starts_with_marker(s: &str) -> bool {
    let lower = s.trim_start().to_ascii_lowercase();
    MARKERS.iter().any(|m| lower.starts_with(m))
}

/// Where a credit opens outside every bracket, after a space.
fn top_marker(s: &str) -> Option<usize> {
    let sp = spans(s);
    let lower = s.to_ascii_lowercase();
    MARKERS
        .iter()
        .chain(&["w/ "])
        .flat_map(|m| lower.match_indices(m).map(|(i, _)| i))
        .filter(|&i| i > 0 && !inside(&sp, i))
        .filter(|&i| s[..i].ends_with(is_space))
        .min()
}

/// A track's name without the featured artists a release credits in it,
/// `ルララリ (feat. Kiri Hoshino)` → `ルララリ`: an upload credits them
/// in its own words and script, or not at all.
#[must_use]
pub fn without_credits(name: &str) -> &str {
    // ASCII lower-casing keeps every byte offset of the original.
    let lower = name.to_ascii_lowercase();
    let cut = [
        "(feat", "[feat", "(ft.", "[ft.", " feat.", " ft.", "(with ", "[with ",
    ]
    .iter()
    .filter_map(|marker| lower.find(marker))
    .min();
    match cut.map(|at| name[..at].trim()) {
        Some(bare) if !bare.is_empty() => bare,
        _ => name,
    }
}

/// A Latin-only bracket right after a title in another script, as
/// `'ルララリ' (RURARARI)`: the name's romanization, kept with it.
fn romanization_after(title: &str, rest: &str) -> Option<String> {
    let rest = rest.trim_start();
    let first = spans(rest).into_iter().next()?;
    let inner = first.inner(rest);
    (has_cjk(title)
        && first.start == 0
        && matches!(first.open, '(' | '（')
        && latin_only(inner)
        && !is_packaging(inner)
        && !starts_with_marker(inner))
    .then(|| rest[..first.end].to_string())
}

// ---- Rules ----

/// `r` in place of `v`, when it differs.
fn changed(v: &str, r: impl Into<String>) -> Option<Vec<String>> {
    let r = r.into();
    (r != v).then(|| vec![r])
}

fn spacing(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    let s: String = v
        .chars()
        .filter(|c| !matches!(c, '\u{200B}' | '\u{2060}' | '\u{FEFF}'))
        .map(|c| match c {
            '\u{A0}' | '\u{202F}' | '\u{2007}' => ' ',
            c => c,
        })
        .collect();
    changed(v, tidy(&s))
}

fn brackets(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    let mut s = v.to_string();
    loop {
        let doubled = spans(&s).into_iter().find_map(|outer| {
            let inner = outer.inner(&s).trim();
            let only = spans(inner);
            match only.as_slice() {
                [one] if one.start == 0 && one.end == inner.len() && one.open == outer.open => {
                    Some((outer, one.inner(inner).to_string()))
                }
                _ => None,
            }
        });
        let Some((outer, inner)) = doubled else {
            break;
        };
        s = format!(
            "{}{}{inner}{}{}",
            &s[..outer.start],
            outer.open,
            outer.close(),
            &s[outer.end..]
        );
    }
    for (o, c) in BRACKETS {
        s = s.replace(&format!("{o}{c}"), "");
    }
    changed(v, s)
}

/// The letters after an apostrophe a title-caser capitalized.
const CONTRACTIONS: [&str; 7] = ["s", "t", "d", "m", "ll", "re", "ve"];

fn contractions(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    let chars: Vec<char> = v.chars().collect();
    let mut out = chars.clone();
    for i in 1..chars.len() {
        if !matches!(chars[i], '\'' | '’') || !chars[i - 1].is_alphabetic() {
            continue;
        }
        let start = (0..i)
            .rev()
            .take_while(|&j| chars[j].is_alphabetic())
            .last()
            .unwrap_or(i);
        let before: String = chars[start..i].iter().collect();
        // An all-caps word is shouted, not title-cased.
        if before != "I" && before.chars().all(char::is_uppercase) {
            continue;
        }
        let after: String = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_alphanumeric())
            .collect();
        let lower = after.to_lowercase();
        if after != lower && CONTRACTIONS.contains(&lower.as_str()) {
            for (k, c) in lower.chars().enumerate() {
                out[i + 1 + k] = c;
            }
        }
    }
    changed(v, out.into_iter().collect::<String>())
}

/// `v` without each qualifier, bracketed or a final ` - ` tail, whose
/// words are wholly what `is` matches.
fn drop_qualifier(v: &str, is: fn(&[String]) -> bool) -> Option<Vec<String>> {
    let doomed: Vec<Span> = spans(v)
        .into_iter()
        .filter(|s| s.is_qualifier() && is(&words(s.inner(v))))
        .collect();
    let mut out = cut(v, &doomed);
    if let Some((at, sep)) = rfind_top(&out, &DASHES)
        && is(&words(&out[at + sep.len()..]))
    {
        out.truncate(at);
    }
    changed(v, out)
}

fn album_version(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    drop_qualifier(
        v,
        |w| matches!(w, [kind, version] if ["album", "lp", "original"].contains(&kind.as_str()) && version == "version"),
    )
}

fn explicit(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    drop_qualifier(v, |w| {
        matches!(w, [kind] | [kind, _] if kind == "explicit" || kind == "clean")
            && w.get(1).is_none_or(|v| v == "version")
    })
}

fn is_year(w: &str) -> bool {
    w.len() == 4
        && (w.starts_with("19") || w.starts_with("20"))
        && w.bytes().all(|b| b.is_ascii_digit())
}

fn remaster(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    drop_qualifier(v, |w| {
        w.iter()
            .filter(|w| *w == "remaster" || *w == "remastered")
            .count()
            == 1
            && w.iter().all(|w| {
                is_year(w)
                    || [
                        "remaster",
                        "remastered",
                        "digitally",
                        "digital",
                        "version",
                        "edition",
                    ]
                    .contains(&w.as_str())
            })
    })
}

/// Words a restatement may add to what an earlier qualifier said.
const RESTATING: [&str; 5] = ["remaster", "remastered", "digitally", "version", "edition"];

fn restated(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    let qualifiers: Vec<Span> = spans(v).into_iter().filter(|s| s.is_qualifier()).collect();
    let mut doomed: Vec<Span> = Vec::new();
    for (i, later) in qualifiers.iter().enumerate() {
        let said: BTreeSet<String> = words(later.inner(v))
            .into_iter()
            .filter(|w| !RESTATING.contains(&w.as_str()))
            .collect();
        if said.is_empty() {
            continue;
        }
        let restates = qualifiers[..i]
            .iter()
            .filter(|e| !doomed.contains(e))
            .any(|e| said.is_subset(&words(e.inner(v)).into_iter().collect()));
        if restates {
            doomed.push(*later);
        }
    }
    changed(v, cut(v, &doomed))
}

fn video_frame(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    let doomed: Vec<Span> = spans(v).into_iter().filter(|s| s.open == '【').collect();
    changed(v, cut(v, &doomed))
}

fn video_tags(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    let doomed: Vec<Span> = spans(v)
        .into_iter()
        .filter(|s| s.is_qualifier() && is_packaging(s.inner(v)))
        .collect();
    let mut out = cut(v, &doomed);
    if let Some((at, sep)) = rfind_top(&out, &DASHES) {
        let tail = &out[at + sep.len()..];
        if is_packaging(tail) && words(tail).len() <= 4 {
            out.truncate(at);
        }
    }
    changed(v, out)
}

fn is_emoji(c: char) -> bool {
    matches!(u32::from(c), 0x1F000..=0x1FAFF | 0x2700..=0x27BF | 0xFE0F | 0x200D)
}

fn emoji(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    changed(v, v.trim_matches(|c: char| is_emoji(c) || is_space(c)))
}

fn pipe_tail(v: &str, at: &At<'_>) -> Option<Vec<String>> {
    let (i, sep) = find_top(v, &[" | ", "｜"])?;
    let (left, right) = (&v[..i], &v[i + sep.len()..]);
    let names = at.ctx.names;
    if names_someone(left, names) && !names_someone(right, names) {
        changed(v, right)
    } else {
        changed(v, left)
    }
}

/// The text of a quoted title: a quote that opens the value or follows a
/// separator, as in `PBW - 'Song' feat. Kiri`. A straight quote closes
/// only before a space, punctuation or the end, so the apostrophe in
/// `I'm` closes nothing.
fn quoted_title(v: &str) -> Option<String> {
    for (i, c) in v.char_indices() {
        let Some(&(_, close)) = QUOTES.iter().find(|(o, _)| *o == c) else {
            continue;
        };
        let before = v[..i].trim_end();
        if !(before.is_empty() || before.ends_with(['-', '–', '—', '/', '／', '|', '｜', ':']))
        {
            continue;
        }
        let from = i + c.len_utf8();
        let Some((j, _)) = v[from..].char_indices().find(|&(j, ch)| {
            ch == close
                && j > 0
                && (matches!(close, '」' | '』' | '”')
                    || v[from + j + ch.len_utf8()..]
                        .chars()
                        .next()
                        .is_none_or(|n| {
                            is_space(n)
                                || n.is_ascii_punctuation()
                                || BRACKETS.iter().any(|(o, _)| *o == n)
                        }))
        }) else {
            continue;
        };
        let quoted = v[from..from + j].trim();
        let rest = &v[from + j + close.len_utf8()..];
        if before.is_empty() && rest.trim().is_empty() {
            return None;
        }
        return Some(match romanization_after(quoted, rest) {
            Some(r) => format!("{quoted} {r}"),
            None => quoted.to_string(),
        });
    }
    None
}

/// A part that only qualifies the song, as ` - Acoustic Version`.
fn is_qualifier_text(s: &str) -> bool {
    let w = words(s);
    !w.is_empty()
        && w.iter().all(|w| {
            KEEP.contains(&w.as_str()) || ["version", "ver", "remaster"].contains(&w.as_str())
        })
}

fn title_artist(v: &str, at: &At<'_>) -> Option<Vec<String>> {
    if let Some(title) = quoted_title(v) {
        return changed(v, title);
    }
    let (i, sep) = find_top(v, &[SLASHES.as_slice(), DASHES.as_slice()].concat())?;
    let (left, right) = (v[..i].trim(), v[i + sep.len()..].trim());
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let names = at.ctx.names;
    let credit = |s: &str| (names_someone(s, names), is_name_list(s));
    let slash = SLASHES.contains(&sep);
    let title = match (credit(left), credit(right)) {
        ((true, _), (true, _)) => return None,
        ((true, _), _) => right,
        (_, (true, _)) => left,
        _ if slash => left,
        ((false, true), (false, false)) => right,
        ((false, false), (false, true)) => left,
        _ if is_qualifier_text(right) => return None,
        // YouTube's own convention: `Artist - Title`.
        _ => right,
    };
    changed(v, title)
}

fn credits(v: &str, at: &At<'_>) -> Option<Vec<String>> {
    let doomed: Vec<Span> = spans(v)
        .into_iter()
        .filter(|s| {
            let inner = s.inner(v).trim_start().to_ascii_lowercase();
            s.is_qualifier()
                && (starts_with_marker(&inner)
                    || inner.starts_with("with ")
                    || inner.starts_with("w/"))
        })
        .collect();
    let s = cut(v, &doomed);
    let Some(i) = top_marker(&s) else {
        return changed(v, s);
    };
    let mut out = s[..i].to_string();
    // A bracket after the credit is kept unless it lists names or spells
    // the credit's own: `feat. 星野キリ(Kiri Hoshino)`.
    for after in spans(&s[i..]) {
        let text = &s[i..][after.start..after.end];
        let spaced = s[i..][..after.start].ends_with(is_space);
        let inner = after.inner(&s[i..]);
        if spaced
            && after.is_qualifier()
            && !is_name_list(inner)
            && !names_someone(inner, at.ctx.names)
        {
            out.push(' ');
            out.push_str(text);
        }
    }
    changed(v, out)
}

fn quotes(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    QUOTES.iter().find_map(|&(o, c)| {
        let inner = v.strip_prefix(o)?.strip_suffix(c)?;
        (!inner.is_empty() && !inner.contains(c)).then(|| vec![inner.to_string()])
    })
}

/// A name and its alias: `Marlo Venn / MarloV`, `くもりび/ kumoribi`. A
/// slash between letters, as in `QR/ZX`, joins one name.
fn alias_split(v: &str) -> Option<(&str, &str)> {
    let (i, sep) = find_top(v, &[" / ", "/ ", " /", "／"])?;
    let (a, b) = (v[..i].trim(), v[i + sep.len()..].trim());
    (!a.is_empty() && !b.is_empty()).then_some((a, b))
}

/// Whether two names read as one person's: one in another script, one
/// the start of the other, or one the other's initials.
fn aliases(a: &str, b: &str) -> bool {
    let (na, nb) = (normalize(a), normalize(b));
    let (short, long) = if na.len() <= nb.len() {
        (&na, &nb)
    } else {
        (&nb, &na)
    };
    (has_cjk(a) && latin_only(b))
        || (latin_only(a) && has_cjk(b))
        || (short.chars().count() >= 4 && long.starts_with(short.as_str()))
        || initials(a).is_some_and(|i| i == nb)
        || initials(b).is_some_and(|i| i == na)
}

fn channel_alias(v: &str, at: &At<'_>) -> Option<Vec<String>> {
    let (a, b) = alias_split(v)?;
    // A channel is one owner, so its slash always joins aliases.
    (!at.structured || aliases(a, b)).then(|| vec![a.to_string()])
}

/// What a channel's name adds to its owner's.
const CHANNEL_SUFFIXES: [&str; 8] = [
    "official youtube channel",
    "official channel",
    "official",
    "vevo",
    "オフィシャルチャンネル",
    "公式チャンネル",
    "オフィシャル",
    "公式",
];

fn without_channel_suffix(v: &str) -> Option<&str> {
    CHANNEL_SUFFIXES.iter().find_map(|suffix| {
        let at = v.len().checked_sub(suffix.len())?;
        let rest = v.get(..at)?;
        let ends = v.get(at..)?.eq_ignore_ascii_case(suffix);
        let apart = *suffix == "vevo" || has_cjk(suffix) || rest.ends_with(is_space);
        let rest = rest.trim_end_matches(|c: char| is_space(c) || c == '-');
        (ends && apart && !rest.is_empty()).then_some(rest)
    })
}

fn channel_suffix(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    without_channel_suffix(v).map(|s| vec![s.to_string()])
}

fn joined_artists(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    let parts: Vec<String> = v.split([';', '；']).map(str::to_string).collect();
    (parts.len() > 1).then_some(parts)
}

fn album_as_artist(v: &str, at: &At<'_>) -> Option<Vec<String>> {
    let artist = at
        .ctx
        .offers
        .get(&Field::AlbumArtist)
        .filter(|o| o.structured)?
        .values
        .first()?;
    let (album, owner) = (normalize(v), normalize(artist));
    (album != owner && at.ctx.albums.has(&owner, &album)).then(|| vec![artist.clone()])
}

/// The disc an album's name says it is, `Live (CD2)` or `Live - Disc 2`,
/// and where that begins.
#[must_use]
pub fn disc_in(album: &str) -> Option<(usize, u32)> {
    let disc = |text: &str| -> Option<u32> {
        let w = words(text);
        let (kind, number) = match w.as_slice() {
            [kind, n] | [kind, n, _, _] => (kind.as_str(), n.as_str()),
            [joined] => {
                let at = joined.find(|c: char| c.is_ascii_digit())?;
                (&joined[..at], &joined[at..])
            }
            _ => return None,
        };
        if w.len() == 4 && w[2] != "of" {
            return None;
        }
        ["cd", "disc", "disk"]
            .contains(&kind)
            .then(|| number.parse().ok())
            .flatten()
    };
    let trimmed = album.trim_end();
    if let Some(last) = spans(trimmed)
        .last()
        .filter(|s| s.is_qualifier() && s.end == trimmed.len())
        && let Some(n) = disc(last.inner(trimmed))
    {
        return Some((last.start, n));
    }
    let (at, sep) = rfind_top(trimmed, &DASHES)?;
    disc(&trimmed[at + sep.len()..]).map(|n| (at, n))
}

fn disc_in_album(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    disc_in(v).map(|(at, _)| vec![v[..at].to_string()])
}

fn joined_genres(v: &str, _: &At<'_>) -> Option<Vec<String>> {
    let parts: Vec<String> = v
        .split([',', ';', '/', '；', '、'])
        .map(str::to_string)
        .collect();
    (parts.len() > 1).then_some(parts)
}

#[cfg(test)]
mod tests;
