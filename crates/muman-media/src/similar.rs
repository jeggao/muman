//! How alike two names or two texts are, in any script.
//!
//! The tools this follows were built for Latin script and fail outside
//! it: Picard splits on `\W+`, which cuts Devanagari, Tamil and Thai
//! words apart at their vowel signs; beets transliterates with
//! `unidecode`, which reads Japanese kanji as Mandarin; and stripping
//! every combining mark erases Indic vowels, Thai tones, Khmer coeng and
//! kana voicing. So text is folded per script, broken into words per
//! script, and measured in grapheme clusters, and a pair no rule can
//! bridge is left uncompared rather than counted as a mismatch.
//!
//! [`fold`] makes the two sides of a comparison alike, in this order:
//!
//! 1. NFKC, which folds full-width ASCII and half-width katakana as
//!    Lucene's `CJKWidthFilter` does; then every default-ignorable
//!    character (ZWJ, ZWNJ, bidi marks, variation selectors) goes.
//! 1. Full Unicode case folding (ICU4X), so `ß` is `ss` and `ς` is `σ`,
//!    and the letters that fold to no base of their own mapped by hand:
//!    `ı đ ð ł ø æ œ þ` to `i d d l o ae oe th`.
//! 1. Combining marks stripped after NFD only on Latin, Greek, Hebrew
//!    and Arabic letters, where they are diacritics, niqqud and harakat.
//!    Cyrillic keeps them, so `й` is not `и`, but `ё` is `е`; every
//!    other script keeps them, since there they are vowels and tones.
//! 1. One letter for the forms a script writes alike, as its search
//!    analyzers fold them:
//!
//! | Script | Folded | After |
//! |---|---|---|
//! | Arabic | `ٱ` to `ا` (other alefs lose their hamza in step 3), `ة` to `ه`, `ى` to `ي`, tatweel removed | Lucene's `ArabicNormalizer` |
//! | Persian | `ی ے` to `ي`, `ک` to `ك`, `ە ہ` to `ه` | Lucene's `PersianNormalizer` |
//! | Hebrew | `ך ם ן ף ץ` to `כ מ נ פ צ` | No analyzer folds them; NFKC leaves them |
//! | Katakana | to hiragana | ICU's `Katakana-Hiragana` |
//! | Han | Traditional to Simplified, one to one | Unihan `kSimplifiedVariant` |
//! | Any decimal digit | to ASCII | NFKC folds only full-width ones |
//!
//! Han folds one way only: one Simplified character can stand for
//! several Traditional ones, so the table (`similar/han.txt`, 6,447
//! pairs) keeps only the entries naming one, with chains followed to
//! their end. A digit's value is its distance from the start of its run
//! of decimal digits, modulo 10: Unicode keeps every such run in blocks
//! of ten, from 0 to 9.
//!
//! [`tokens`] are the words ICU4X's dictionary word breaker finds: UAX
//! #29 words where a script writes spaces, and dictionary words for
//! Chinese, Japanese, Thai, Lao, Khmer and Burmese, which write none. The
//! dictionary is chosen over ICU4X's LSTM models, which compute in
//! floating point and so could break a text differently on another
//! machine. Kana are folded after breaking, since the dictionary knows a
//! loanword by its katakana.
//!
//! [`compare`] measures two [`Name`]s, each split into a core and its
//! decorations (brackets, a ` - ` suffix, text between wave dashes),
//! with brackets naming packaging dropped. The distance is the lesser
//! of two, as beets and Picard each take one:
//!
//! - edits over grapheme clusters, so a Devanagari conjunct or a Thai
//!   syllable's marks count once, with a Hangul syllable taken as its
//!   jamo; a decoration's edits count a fifth, as beets weighs brackets;
//! - one less the Monge–Elkan similarity of the two sides' words, both
//!   ways round, for reordered names; each word stands for one word of
//!   the other side at most, so a word said three times is not said once.
//!
//! Two names whose main scripts differ are compared only when one is
//! Latin and the other an alphabet [`deunicode`] romanizes letter by
//! letter (Cyrillic, Greek, Armenian, Georgian), at a small cost. Any
//! other pair, kanji and romaji, Hangul and Latin, is uncompared:
//! MusicBrainz bridges such names only with aliases entered by hand. So
//! is a name in kanji alone against one in kana alone, which may be the
//! name and its reading.
//!
//! A version word ([`VERSIONS`]) in one name's decorations but not the
//! other's says the two may be other recordings: live, instrumental,
//! karaoke, acoustic, remix, cover and demo are conflicts; remastered,
//! edit, mix, mono and version only cost a little. Each is matched as
//! whole words, in Chinese, Japanese and Thai as words the dictionary
//! finds, run together; a kana entry, written in hiragana, matches only
//! katakana, as the loanword is written, so デモ is a demo and でも
//! ("but") is not, nor is the ライブ inside ドライブ ("drive").
//!
//! Whether a source names the song's recording at all ([`agree`],
//! [`titles_agree`], [`credits_agree`]) is read in three bands, so a near miss counts
//! neither way: alike within [`ALIKE`], another's past [`APART`] or
//! numbered otherwise, and unknown between them or where no rule
//! compares the two. Only the cores are measured, and their numbers: a
//! take, a date or a version in a decoration, as `(take 2, 3/14/1999)`
//! against `[Live, Alternate version]`, tells two releases of one song
//! apart more often than two songs, and MusicBrainz keeps `live` out of
//! a title altogether, so how far a version counts is the caller's
//! ([`versions_in`]). A spelling, as `Harbor` and `Harbour`, stands
//! under a tenth apart; `Song One` and `Song Four`, a title and a longer
//! one that holds it, or one cut short, stand a third to a half apart,
//! which proves nothing either way; two titles with no word in common
//! stand nearly a whole apart. Credits agree only on a whole name,
//! folded, after splitting each credit where `&`, `and`, a comma or
//! `feat.` joins two names: holding a name is not naming it, so a
//! tribute act's `Marlo Venn Tribute Band` never credits `Marlo Venn`.

use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

use icu_casemap::CaseMapper;
use icu_properties::props::{
    DefaultIgnorableCodePoint, GeneralCategory, GeneralCategoryGroup, Script,
};
use icu_properties::{CodePointMapData, CodePointSetData};
use icu_segmenter::options::WordBreakInvariantOptions;
use icu_segmenter::{WordSegmenter, WordSegmenterBorrowed};
use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

use crate::clean;

/// What a decoration's edits weigh against the core's.
const DECORATION: f64 = 0.2;

/// What comparing across scripts through a romanization costs.
const ROMANIZED: f64 = 0.05;

/// What a soft version word on one side only costs.
const SOFT: f64 = 0.05;

/// Words saying a name is another recording than one without them,
/// with the conflict they name: written folded, each matched as whole
/// words, as the module's documentation says.
pub const VERSIONS: [(&str, &[&str]); 7] = [
    (
        "live version",
        &[
            "live",
            "en vivo",
            "ao vivo",
            "en directo",
            "en direct",
            "らいぶ",
            "现场",
            "라이브",
        ],
    ),
    (
        "instrumental",
        &[
            "instrumental",
            "inst",
            "off vocal",
            "おふぼーかる",
            "伴奏",
            "인스트",
        ],
    ),
    ("karaoke", &["karaoke", "からおけ", "卡拉ok"]),
    (
        "acoustic version",
        &[
            "acoustic",
            "acustico",
            "akustik",
            "あこーすてぃっく",
            "어쿠스틱",
        ],
    ),
    ("remix", &["remix", "りみっくす", "리믹스"]),
    ("cover", &["cover", "かばー", "翻唱"]),
    ("demo", &["demo", "でも"]),
];

/// Words saying a name is another master or cut of one recording.
pub const SOFT_VERSIONS: [&str; 8] = [
    "remastered",
    "remaster",
    "りますたー",
    "edit",
    "mix",
    "mono",
    "version",
    "ver",
];

const OPENS: [(char, char); 10] = [
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('【', '】'),
    ('「', '」'),
    ('『', '』'),
    ('〈', '〉'),
    ('《', '》'),
    ('〔', '〕'),
    ('〖', '〗'),
];

/// What opens a name's suffix, as `Lantern Weather - Live at the Pier`.
pub(crate) const SUFFIXES: [&str; 7] = [" - ", " – ", " — ", " ‐ ", " ‒ ", " ― ", " − "];

/// Wave dashes, which set off a subtitle in Japanese: `Title 〜Night〜`.
const WAVES: [char; 2] = ['~', '〜'];

static SEGMENTER: LazyLock<WordSegmenterBorrowed<'static>> =
    LazyLock::new(|| WordSegmenter::new_dictionary(WordBreakInvariantOptions::default()));

static HAN: LazyLock<HashMap<char, char>> = LazyLock::new(|| {
    include_str!("similar/han.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut c = l.chars();
            Some((c.next()?, c.next()?))
        })
        .collect()
});

fn script(c: char) -> Script {
    CodePointMapData::<Script>::new().get(c)
}

fn category(c: char) -> GeneralCategory {
    CodePointMapData::<GeneralCategory>::new().get(c)
}

fn is_mark(c: char) -> bool {
    GeneralCategoryGroup::Mark.contains(category(c))
}

/// Whether `c` counts in a name: a letter, a digit or a mark on one.
fn counts(c: char) -> bool {
    let gc = category(c);
    GeneralCategoryGroup::Letter.contains(gc)
        || GeneralCategoryGroup::Number.contains(gc)
        || GeneralCategoryGroup::Mark.contains(gc)
}

/// The ASCII digit any decimal digit stands for.
fn digit(c: char) -> Option<char> {
    if category(c) != GeneralCategory::DecimalNumber {
        return None;
    }
    let mut start = u32::from(c);
    while let Some(before) = start.checked_sub(1).and_then(char::from_u32) {
        if category(before) != GeneralCategory::DecimalNumber {
            break;
        }
        start -= 1;
    }
    let value = (u32::from(c) - start) % 10;
    char::from_digit(value, 10)
}

/// Whether the marks on letters of `s` are diacritics, to be dropped.
fn strips_marks(s: Script) -> bool {
    [Script::Latin, Script::Greek, Script::Hebrew, Script::Arabic].contains(&s)
}

/// The letters case folding leaves without a base of their own.
fn by_hand(c: char) -> Option<&'static str> {
    Some(match c {
        'ı' => "i",
        'đ' | 'ð' => "d",
        'ł' => "l",
        'ø' => "o",
        'æ' => "ae",
        'œ' => "oe",
        'þ' => "th",
        _ => return None,
    })
}

/// One letter for the forms a script writes alike.
fn alike(c: char) -> Option<char> {
    Some(match c {
        'ٱ' => 'ا',
        'ة' | 'ە' | 'ہ' => 'ه',
        'ى' | 'ی' | 'ے' => 'ي',
        'ک' => 'ك',
        'ך' => 'כ',
        'ם' => 'מ',
        'ן' => 'נ',
        'ף' => 'פ',
        'ץ' => 'צ',
        _ => return digit(c).or_else(|| HAN.get(&c).copied()),
    })
}

fn kana(c: char) -> char {
    match u32::from(c) {
        0x30A1..=0x30F6 | 0x30FD..=0x30FE => char::from_u32(u32::from(c) - 0x60).unwrap_or(c),
        _ => c,
    }
}

/// `s` folded but for its kana, which the word breaker reads by script.
fn prepare(s: &str) -> String {
    let wide: String = s
        .nfkc()
        .filter(|&c| !CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(c))
        .collect::<String>()
        .replace('&', " and ");
    // ё is е with a mark Cyrillic keeps, so it is mapped before marks part.
    let cased = CaseMapper::new().fold_string(&wide).replace('ё', "е");
    let mut out = String::with_capacity(cased.len());
    // The script the last letter written is in, which owns the marks after
    // it; a digit or a space owns none.
    let mut base = Script::Common;
    for c in cased.nfd() {
        if is_mark(c) {
            if !strips_marks(base) {
                out.push(c);
            }
            continue;
        }
        if c == '\u{0640}' {
            continue;
        }
        if let Some(letters) = by_hand(c) {
            out.push_str(letters);
            base = Script::Latin;
        } else if counts(c) {
            let folded = alike(c).unwrap_or(c);
            out.push(folded);
            base = script(folded);
        } else {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            base = Script::Common;
        }
    }
    out.nfc().collect::<String>().trim().to_string()
}

/// `s` as it is compared: see the module's documentation.
#[must_use]
pub fn fold(s: &str) -> String {
    let folded: String = prepare(s).chars().map(kana).collect();
    without_article(&folded).to_string()
}

fn without_article(mut s: &str) -> &str {
    while let Some(rest) = s.strip_prefix("the ").or_else(|| s.strip_suffix(" the")) {
        s = rest;
    }
    s
}

/// The words of `s`, folded.
#[must_use]
pub fn tokens(s: &str) -> Vec<String> {
    words_of(s)
        .into_iter()
        .map(|w| w.chars().map(kana).collect())
        .collect()
}

/// The words of `s`, folded but for kana, which tell a loanword in
/// katakana from a word of the language in hiragana.
/// The words of `s`, folded but for kana, as a version's entries are
/// matched against.
pub(crate) fn words(s: &str) -> Vec<String> {
    words_of(s)
}

/// Whether `words` hold `entry`, a version's entry, as whole words.
pub(crate) fn holds_words(words: &[String], entry: &str) -> bool {
    holds(words, entry)
}

fn words_of(s: &str) -> Vec<String> {
    let prepared = prepare(s);
    let mut words = Vec::new();
    let mut last = 0;
    // A segment counts by what it holds: ICU4X 2.3 calls the first word
    // of a Devanagari or Tamil text no word at all.
    for at in SEGMENTER.segment_str(&prepared) {
        let word = &prepared[last..at];
        if word.chars().any(counts) {
            words.push(word.to_string());
        }
        last = at;
    }
    words
}

/// The units an edit counts: grapheme clusters, a Hangul syllable as
/// its jamo.
fn units(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for g in s.graphemes(true) {
        let mut chars = g.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if ('\u{AC00}'..='\u{D7A3}').contains(&c) => {
                out.extend(c.to_string().nfd().map(String::from));
            }
            _ => out.push(g.to_string()),
        }
    }
    out
}

/// The Levenshtein distance between two runs of units.
fn edits(a: &[String], b: &[String]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(x != y));
            diagonal = above;
        }
    }
    row[b.len()]
}

/// One less the share of `a` and `b`'s units an edit leaves alone.
fn edit_distance(a: &str, b: &str) -> f64 {
    let (a, b) = (units(a), units(b));
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 0.0;
    }
    ratio(edits(&a, &b), longest)
}

fn ratio(part: usize, whole: usize) -> f64 {
    part as f64 / whole as f64
}

/// How alike the words of `a` are to the closest of `b`'s, averaged.
///
/// Each word of `b` is the closest of one word of `a` at most, and the
/// sum is over the longer list, so a word said twice is not said once.
fn monge_elkan(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut free = vec![true; b.len()];
    let mut total = 0.0;
    for w in a {
        let best = b
            .iter()
            .enumerate()
            .filter(|(n, _)| free[*n])
            .map(|(n, v)| (n, 1.0 - edit_distance(w, v)))
            .max_by(|x, y| x.1.total_cmp(&y.1).then(y.0.cmp(&x.0)));
        if let Some((n, sim)) = best {
            free[n] = false;
            total += sim;
        }
    }
    total / ratio(a.len().max(b.len()), 1)
}

/// The families of scripts a name can be written in: Chinese and
/// Japanese mix Han and kana in one name, so they are one.
fn family(s: Script) -> Script {
    match s {
        Script::Hiragana | Script::Katakana => Script::Han,
        other => other,
    }
}

/// The script most of `s`'s letters are in.
fn main_script(s: &str) -> Option<Script> {
    let mut counts: Vec<(Script, usize)> = Vec::new();
    for c in s.chars() {
        let sc = family(script(c));
        if matches!(sc, Script::Common | Script::Inherited) {
            continue;
        }
        match counts.iter_mut().find(|(s, _)| *s == sc) {
            Some((_, n)) => *n += 1,
            None => counts.push((sc, 1)),
        }
    }
    // The first script seen wins a tie, so the answer never depends on
    // how scripts are numbered.
    let most = counts.iter().map(|&(_, n)| n).max()?;
    counts.iter().find(|&&(_, n)| n == most).map(|&(s, _)| s)
}

/// Whether `deunicode` romanizes `s` letter by letter, as it does the
/// alphabets of Europe and the Caucasus.
fn romanizes(s: Script) -> bool {
    [
        Script::Cyrillic,
        Script::Greek,
        Script::Armenian,
        Script::Georgian,
    ]
    .contains(&s)
}

/// A name split for comparing: its core, and what decorates it.
#[derive(Debug, Clone, PartialEq)]
pub struct Name {
    pub core: String,
    pub decorations: Vec<String>,
    words: Vec<String>,
    /// Each decoration's words, kana as written.
    decoration_words: Vec<Vec<String>>,
    script: Option<Script>,
    /// Whether its core holds Han, and kana.
    han: bool,
    kana: bool,
}

impl Name {
    #[must_use]
    pub fn new(raw: &str) -> Name {
        let wide: String = raw.nfkc().collect();
        let (core, decorations) = split(clean::without_credits(&wide));
        let words = std::iter::once(&core)
            .chain(&decorations)
            .flat_map(|s| tokens(s))
            .collect();
        let in_script = |sc: Script| core.chars().any(|c| script(c) == sc);
        Name {
            script: main_script(&core),
            han: in_script(Script::Han),
            kana: in_script(Script::Hiragana) || in_script(Script::Katakana),
            core: fold(&core),
            decoration_words: decorations.iter().map(|d| words_of(d)).collect(),
            decorations: decorations.iter().map(|d| fold(d)).collect(),
            words,
        }
    }

    /// Whether the name has letters or digits to compare.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.core.is_empty()
    }

    /// The version words in its decorations: each conflict named, and
    /// whether a soft one is there.
    #[must_use]
    pub fn versions(&self) -> (BTreeSet<&'static str>, bool) {
        let mut hard = BTreeSet::new();
        let mut soft = false;
        for words in &self.decoration_words {
            for (kind, entries) in VERSIONS {
                if entries.iter().any(|e| holds(words, e)) {
                    hard.insert(kind);
                }
            }
            soft |= SOFT_VERSIONS.iter().any(|e| holds(words, e));
        }
        (hard, soft)
    }

    /// The numbers it names, in digits or as [`roman`] numerals, but for
    /// those in a decoration that names a version, as a remaster's year.
    fn numbers(&self) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = numbered(&self.core, &tokens(&self.core))
            .into_iter()
            .collect();
        for (d, words) in self.decorations.iter().zip(&self.decoration_words) {
            let versioned = VERSIONS
                .iter()
                .flat_map(|(_, e)| e.iter())
                .chain(&SOFT_VERSIONS);
            if !versioned.into_iter().any(|e| holds(words, e)) {
                out.extend(numbered(d, words));
            }
        }
        out
    }

    /// The numbers its decorations give it as one of a series, as `(Part
    /// 2)` or `[Pt. II]`.
    fn series(&self) -> BTreeSet<String> {
        self.decorations
            .iter()
            .zip(&self.decoration_words)
            .filter(|(_, words)| words.iter().any(|w| SERIES.contains(&w.as_str())))
            .flat_map(|(d, words)| numbered(d, words))
            .collect()
    }
}

/// Whether `words`, as [`words_of`] gives them, hold `entry` as whole
/// words. An entry in a script written without spaces is matched by
/// words the dictionary found, run together, and in kana by katakana,
/// as a loanword is written: so デモ is a demo and でも ("but") is not,
/// nor is the ライブ in ドライブ ("drive").
fn holds(words: &[String], entry: &str) -> bool {
    let spaced = entry
        .chars()
        .all(|c| !matches!(family(script(c)), Script::Han | Script::Thai));
    if spaced {
        let wanted: Vec<&str> = entry.split(' ').collect();
        return words
            .windows(wanted.len())
            .any(|w| w.iter().map(String::as_str).eq(wanted.iter().copied()));
    }
    let written: String = entry
        .chars()
        .map(|c| match u32::from(c) {
            0x3041..=0x3096 => char::from_u32(u32::from(c) + 0x60).unwrap_or(c),
            _ => c,
        })
        .collect();
    (0..words.len()).any(|start| {
        let mut run = String::new();
        words[start..].iter().take(4).any(|w| {
            run.push_str(w);
            run == written
        })
    })
}

/// `s` split into its core and its decorations.
fn split(s: &str) -> (String, Vec<String>) {
    let title = crate::parts::title(s);
    let decorations = (title.decorations.into_iter())
        .filter(|d| d.kind != crate::parts::Kind::Packaging)
        .map(|d| d.text)
        .collect();
    (title.core, decorations)
}

/// Where the bracket `close` that matches an `open` before `inner`
/// stands in it, past any pair nested inside.
pub(crate) fn closing(inner: &str, open: char, close: char) -> Option<usize> {
    if open == close {
        return inner.find(close);
    }
    let mut depth = 0usize;
    for (at, c) in inner.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            if depth == 0 {
                return Some(at);
            }
            depth -= 1;
        }
    }
    None
}

pub(crate) fn opener(c: char) -> Option<char> {
    OPENS
        .iter()
        .find(|(o, _)| *o == c)
        .map(|&(_, close)| close)
        .or_else(|| WAVES.contains(&c).then_some(c))
}

/// What comparing two names found.
#[derive(Debug, Clone, PartialEq)]
pub struct Compared {
    /// From 0, alike, to 1.
    pub distance: f64,
    /// Why the two may be other recordings, as `live version`.
    pub conflicts: Vec<&'static str>,
}

/// How far apart `a` and `b` are, or `None` when they cannot be told:
/// either is empty, or they are written in scripts no rule bridges.
#[must_use]
pub fn compare(a: &Name, b: &Name) -> Option<Compared> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    // A name in kanji and one in kana alone may be one name and its
    // reading, which no rule tells.
    if (a.han && !a.kana && b.kana && !b.han) || (b.han && !b.kana && a.kana && !a.han) {
        return None;
    }
    let mut cost = 0.0;
    let romanized;
    let (a, b) = match (a.script, b.script) {
        (Some(x), Some(y)) if x != y => {
            let latin = |s| s == Script::Latin;
            let (latin_side, other) = if latin(x) { (a, b) } else { (b, a) };
            if !(latin(latin_side.script?) && romanizes(other.script?)) {
                return None;
            }
            romanized = Name::new(&deunicode::deunicode(&raw_of(other)));
            cost = ROMANIZED;
            (latin_side, &romanized)
        }
        _ => (a, b),
    };
    let decorations = |n: &Name| n.decorations.join(" ");
    let (core_a, core_b) = (units(&a.core), units(&b.core));
    let (dec_a, dec_b) = (units(&decorations(a)), units(&decorations(b)));
    let weight = ratio(core_a.len().max(core_b.len()), 1)
        + DECORATION * ratio(dec_a.len().max(dec_b.len()), 1);
    let by_edits =
        (ratio(edits(&core_a, &core_b), 1) + DECORATION * ratio(edits(&dec_a, &dec_b), 1)) / weight;
    let by_words = 1.0
        - f64::midpoint(
            monge_elkan(&a.words, &b.words),
            monge_elkan(&b.words, &a.words),
        );
    let (hard_a, soft_a) = a.versions();
    let (hard_b, soft_b) = b.versions();
    if soft_a != soft_b {
        cost += SOFT;
    }
    let mut conflicts: Vec<&'static str> = hard_a.symmetric_difference(&hard_b).copied().collect();
    if a.numbers() != b.numbers() {
        conflicts.push("another number");
    }
    Some(Compared {
        distance: (by_edits.min(by_words) + cost).min(1.0),
        conflicts,
    })
}

/// The distance, in thousandths, two names are alike within, as the
/// vetting of a source and an AcoustID match read them: a spelling, a
/// soft version word or a decoration apart.
pub const ALIKE: i64 = 200;

/// The distance, in thousandths, past which two names are another's:
/// between [`ALIKE`] and this, they neither agree nor disagree.
pub const APART: i64 = 500;

/// How many words a title cut short keeps at least, to agree with the
/// title it is the start of: a tag field's limit or a file name's cuts a
/// long title, and three words are seldom the start of another song's.
pub const CUT_WORDS: usize = 3;

/// Words that, after a name's start, make it another of a series rather
/// than the name cut short, as `Night Ferry, Volume 2` is another album
/// than `Night Ferry`.
const SERIES: [&str; 8] = ["volume", "vol", "part", "pt", "disc", "cd", "book", "act"];

/// The Roman numerals a title numbers a part with, `I` to `XX`.
const ROMAN: [&str; 20] = [
    "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv", "xv",
    "xvi", "xvii", "xviii", "xix", "xx",
];

/// The numbers `text`, whose words are `words`, names: each run of
/// digits, and each [`ROMAN`] numeral but `I` and `V`, which count only
/// after a word naming one of a series, as `Part I`, since alone they are
/// a word or a letter.
fn numbered(text: &str, words: &[String]) -> Vec<String> {
    let digits = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|n| !n.is_empty())
        .map(|n| n.trim_start_matches('0').to_string());
    let roman = words.iter().enumerate().filter_map(|(at, w)| {
        let n = ROMAN.iter().position(|r| r == w)? + 1;
        let after_series = at
            .checked_sub(1)
            .is_some_and(|p| SERIES.contains(&words[p].as_str()));
        (!matches!(n, 1 | 5) || after_series).then(|| n.to_string())
    });
    digits.chain(roman).collect()
}

/// Whether `a` and `b` name one thing, as vetting asks it, by their
/// cores: `Some(true)` within [`ALIKE`], or when one is the other cut
/// short after [`CUT_WORDS`] words or more apart by spaces, unless what
/// the other adds numbers it or names it one of a series ([`SERIES`]);
/// `Some(false)` past [`APART`], numbered otherwise, or numbered as
/// another part of a series in a decoration, as `(Part 1)` against
/// `(Part 2)`; `None` between the two, or where no rule compares them,
/// as kanji and kana.
#[must_use]
pub fn agree(a: &Name, b: &Name) -> Option<bool> {
    let (sa, sb) = (a.series(), b.series());
    if !sa.is_empty() && !sb.is_empty() && sa != sb {
        return Some(false);
    }
    let (wa, wb) = (tokens(&a.core), tokens(&b.core));
    let (short, long, spaced) = if wa.len() <= wb.len() {
        (&wa, &wb, &a.core)
    } else {
        (&wb, &wa, &b.core)
    };
    let numbered = |w: &String| {
        w.chars().any(char::is_numeric)
            || SERIES.contains(&w.as_str())
            || ROMAN.contains(&w.as_str())
    };
    if short.len() >= CUT_WORDS
        && spaced.split_whitespace().count() >= CUT_WORDS
        && long.starts_with(short)
        && !long[short.len()..].iter().any(numbered)
    {
        return Some(true);
    }
    let cores = compare(&Name::new(&a.core), &Name::new(&b.core))?;
    let thousandths = (cores.distance * 1000.0).round() as i64;
    if !cores.conflicts.is_empty() || thousandths > APART {
        Some(false)
    } else {
        (thousandths <= ALIKE).then_some(true)
    }
}

/// The versions [`VERSIONS`] names anywhere in `text`, its core too, as
/// a title cut off inside its bracket holds them.
#[must_use]
pub fn versions_in(text: &str) -> BTreeSet<&'static str> {
    let words = words_of(text);
    VERSIONS
        .iter()
        .filter(|(_, entries)| entries.iter().any(|e| holds(&words, e)))
        .map(|(kind, _)| *kind)
        .collect()
}

/// What joins two names in one credit, as `Marlo Venn & Kiri Hoshino`
/// or `Marlo Venn feat. Kiri Hoshino`.
pub(crate) static JOINERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*[&＆,、;/×]\s*|\s+(?:feat\.?|ft\.?|featuring|vs\.?|with|and|x|-)\s+")
        .expect("a valid pattern")
});

/// The names one credit holds as written, split where [`JOINERS`] join
/// them: one for a credit naming one artist.
#[must_use]
pub fn names_in(credit: &str) -> Vec<&str> {
    crate::parts::credit(credit).names
}

/// The names one credit holds, folded: the whole, and each name what
/// joins credits parts in it, each also without its apostrophes, as
/// `Marlo's Lanterns` is written `Marlos Lanterns`, and without its
/// spaces, as `星野 キリ` is written `星野キリ`.
#[must_use]
pub fn credited(credit: &str) -> BTreeSet<String> {
    std::iter::once(credit)
        .chain(crate::parts::credit(credit).names)
        .flat_map(|n| {
            let bare = fold(&n.replace(['\'', '’', 'ʼ'], ""));
            [fold(n), bare.replace(' ', ""), bare]
        })
        .filter(|n| !n.is_empty())
        .collect()
}

/// Whether two credits, each a list of values, name someone in common:
/// `Some(true)` where a name one holds, folded, is one the other holds,
/// never by one name holding the other, so `Marlo Venn Tribute Band` is
/// not `Marlo Venn`; `Some(false)` where none is and some two values
/// compare at all; `None` where none do, as names in kanji and in Latin
/// letters.
#[must_use]
pub fn credits_agree(a: &[&str], b: &[&str]) -> Option<bool> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let folded =
        |side: &[&str]| -> BTreeSet<String> { side.iter().flat_map(|v| credited(v)).collect() };
    if !folded(a).is_disjoint(&folded(b)) {
        return Some(true);
    }
    let split = |side: &[&str]| -> Vec<Name> { side.iter().map(|v| Name::new(v)).collect() };
    let (a, b) = (split(a), split(b));
    a.iter()
        .any(|x| b.iter().any(|y| compare(x, y).is_some()))
        .then_some(false)
}

/// Whether `mine`, a source's title or album, names what one of
/// `theirs`, each another source's values, names, read every way files
/// write a title, as [`agree`] reads each pair: `Some(true)` where one
/// pair agrees; `Some(false)` where every pair disagrees, so a reading
/// that proves nothing, or one no rule compares, says nothing. `sides`
/// reads `mine` at ` / `, ` - `, ` | ` and `: ` too, as an album's name,
/// where a title read so could agree by a movement alone. A version
/// `mine` names that a source never names makes the two disagree; one
/// only the source names does not, since MusicBrainz keeps `live` out of
/// its titles.
#[must_use]
pub fn titles_agree(mine: &[&str], theirs: &[Vec<&str>], sides: bool) -> Option<bool> {
    let versions = |values: &[&str]| -> BTreeSet<&'static str> {
        values.iter().flat_map(|v| versions_in(v)).collect()
    };
    let named_versions = versions(mine);
    let mine: Vec<Name> = readings(mine, sides).iter().map(|m| Name::new(m)).collect();
    let (mut compared, mut apart, mut versioned) = (false, true, false);
    for source in theirs {
        let alike = named_versions.is_subset(&versions(source));
        for t in readings(source, true).iter().map(|t| Name::new(t)) {
            for m in &mine {
                compared = true;
                match agree(m, &t) {
                    Some(true) if alike => return Some(true),
                    Some(true) => versioned = true,
                    Some(false) => {}
                    None => apart = false,
                }
            }
        }
    }
    (versioned || compared && apart).then_some(false)
}

/// What joins a credit, a part or a context to a title in one value, as
/// `Marlo Venn / Night Ferry` or `1st Set: Night Ferry`.
const SIDES: [&str; 4] = [" / ", " - ", " | ", ": "];

/// What opens a title's decoration to [`readings`], past those [`OPENS`]
/// holds: a bracket a title cut off left open, or one written `<…>`.
const READ_OPENS: [char; 8] = ['(', '[', '<', '＜', '（', '【', '〈', '「'];

/// What closes one of [`READ_OPENS`].
const READ_CLOSES: [char; 8] = [')', ']', '>', '＞', '）', '】', '〉', '」'];

/// Each way `values`, one source's, may name a song: each value; all of
/// them joined, as a work and its movement given as two; each with its
/// brackets and [`SIDES`] read as spaces, as `Soliloquy (Night Ferry)`
/// names `Night Ferry: Soliloquy`; what comes before its first bracket,
/// even one cut off unclosed, or before a ` -` opening `-Kiri Version-`;
/// a disc closing an album
/// ([`without_disc`]); and with `sides`, each side of a [`SIDES`], as a
/// file that names its performer before the title.
fn readings(values: &[&str], sides: bool) -> Vec<String> {
    let mut all: Vec<String> = values.iter().map(|v| (*v).to_string()).collect();
    if values.len() > 1 {
        all.push(values.join(" "));
    }
    for v in values {
        if sides {
            for side in SIDES.iter().filter(|s| v.contains(**s)) {
                all.extend(v.split(side).map(str::to_string));
            }
        }
        let flat = SIDES.iter().fold(
            v.replace(READ_OPENS, " ").replace(READ_CLOSES, " "),
            |f, s| f.replace(s, " "),
        );
        all.push(flat);
        for at in [v.find(READ_OPENS), v.find(" -")].into_iter().flatten() {
            all.push(v[..at].to_string());
        }
        all.extend(without_disc(v).map(str::to_string));
    }
    all.retain(|r| !r.trim().is_empty());
    all.sort();
    all.dedup();
    all
}

/// The names `title` gives beside the song's, as `Marlo Venn / Night
/// Ferry` names its performer.
pub fn performers(title: &str) -> impl Iterator<Item = &str> {
    SIDES[..3]
        .iter()
        .filter(|s| title.contains(**s))
        .flat_map(|s| title.split(s))
}

/// `album` without a disc closing it unbracketed, as `Live CD1` or
/// `Live - Disc 2`, which cleaning leaves.
fn without_disc(album: &str) -> Option<&str> {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let kinds = ["cd", "disc", "disk"];
    let (rest, last) = album.trim_end().rsplit_once(' ')?;
    let last = last.to_ascii_lowercase();
    let rest = if kinds
        .iter()
        .any(|k| last.strip_prefix(k).is_some_and(digits))
    {
        rest
    } else if digits(&last) {
        let (before, kind) = rest.trim_end().rsplit_once(' ')?;
        if !kinds.contains(&kind.to_ascii_lowercase().as_str()) {
            return None;
        }
        before
    } else {
        return None;
    };
    Some(rest.trim_end_matches([' ', '-', '–']))
}

/// A name put back together, for romanizing.
fn raw_of(n: &Name) -> String {
    std::iter::once(n.core.clone())
        .chain(n.decorations.iter().map(|d| format!("({d})")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a file's name may say of its song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guess {
    pub track: Option<u32>,
    pub artist: Option<String>,
    pub title: String,
}

static GUESSES: LazyLock<[Regex; 5]> = LazyLock::new(|| {
    [
        r"^(?P<track>\d+)\.?\s*[-‐‒–—―−]\s*(?P<a>.+?)\s+[-‐‒–—―−]\s+(?P<b>.+)$",
        r"^(?P<a>.+?)\s+[-‐‒–—―−]\s+(?P<b>.+)$",
        r"^(?P<track>\d+)\.?[\s_-]+(?P<b>.+)$",
        r"^(?P<track>\d+)\.(?P<b>\D.*)$",
        r"^(?P<b>.+?)\s+by\s+(?P<a>.+)$",
    ]
    .map(|p| Regex::new(p).expect("a valid pattern"))
});

/// Every reading of the file name `stem`, the likeliest first, as
/// beets' `fromfilename` reads them; a dash between two names is read
/// both ways round.
#[must_use]
pub fn guesses(stem: &str) -> Vec<Guess> {
    let wide: String = stem.nfkc().collect();
    let stem = if wide.contains(' ') {
        wide
    } else {
        wide.replace('_', " ")
    };
    let stem = stem.trim();
    let mut out = Vec::new();
    for (n, re) in GUESSES.iter().enumerate() {
        let Some(c) = re.captures(stem) else {
            continue;
        };
        let track = c.name("track").and_then(|t| {
            t.as_str()
                .chars()
                .map(|c| digit(c).unwrap_or(c))
                .collect::<String>()
                .parse()
                .ok()
        });
        let a = c.name("a").map(|m| m.as_str().trim().to_string());
        let b = c
            .name("b")
            .map_or_else(String::new, |m| m.as_str().trim().to_string());
        out.push(Guess {
            track,
            artist: a.clone(),
            title: b.clone(),
        });
        if let (Some(a), true) = (a, n < 2) {
            out.push(Guess {
                track,
                artist: Some(b),
                title: a,
            });
        }
    }
    out.push(Guess {
        track: None,
        artist: None,
        title: stem.to_string(),
    });
    out.dedup();
    out
}

/// The runs of `k` words of `words`, or all of them as one when fewer.
#[must_use]
pub fn shingles(words: &[String], k: usize) -> BTreeSet<String> {
    let k = k.max(1);
    if words.len() < k {
        return std::iter::once(words.join("\u{1f}"))
            .filter(|s| !s.is_empty())
            .collect();
    }
    words.windows(k).map(|w| w.join("\u{1f}")).collect()
}

/// The share of the smaller set the other holds.
#[must_use]
pub fn containment(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let smaller = a.len().min(b.len());
    if smaller == 0 {
        return 0.0;
    }
    ratio(a.intersection(b).count(), smaller)
}

/// The script most of `text`'s letters are in, by family: Han and kana
/// are one.
#[must_use]
pub fn script_of(text: &str) -> Option<Script> {
    main_script(text)
}

/// The column each row of `cost` is given, so the sum is the least: the
/// Hungarian method, for no more rows than columns. Ties go to the
/// earlier column, so the answer is the same on every run.
#[must_use]
pub fn assign(cost: &[Vec<i64>]) -> Vec<usize> {
    let rows = cost.len();
    let cols = cost.first().map_or(0, Vec::len);
    assert!(rows <= cols, "no more rows than columns");
    let inf = i64::MAX / 4;
    let (mut row_potential, mut col_potential) = (vec![0; rows + 1], vec![0; cols + 1]);
    let (mut row_of, mut way) = (vec![0usize; cols + 1], vec![0usize; cols + 1]);
    for i in 1..=rows {
        row_of[0] = i;
        let mut col = 0;
        let mut slack = vec![inf; cols + 1];
        let mut used = vec![false; cols + 1];
        loop {
            used[col] = true;
            let row = row_of[col];
            let mut delta = inf;
            let mut next = 0;
            for j in 1..=cols {
                if !used[j] {
                    let cur = cost[row - 1][j - 1] - row_potential[row] - col_potential[j];
                    if cur < slack[j] {
                        slack[j] = cur;
                        way[j] = col;
                    }
                    if slack[j] < delta {
                        delta = slack[j];
                        next = j;
                    }
                }
            }
            for j in 0..=cols {
                if used[j] {
                    row_potential[row_of[j]] += delta;
                    col_potential[j] -= delta;
                } else {
                    slack[j] -= delta;
                }
            }
            col = next;
            if row_of[col] == 0 {
                break;
            }
        }
        loop {
            let next = way[col];
            row_of[col] = row_of[next];
            col = next;
            if col == 0 {
                break;
            }
        }
    }
    let mut out = vec![0; rows];
    for j in 1..=cols {
        if row_of[j] != 0 {
            out[row_of[j] - 1] = j - 1;
        }
    }
    out
}

/// Letters and digits only, lower-cased and in NFKC, so punctuation,
/// spacing, case and full-width forms never decide a match.
pub fn normalize(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkc()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// How closely one name holds another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Exactness {
    /// One holds the other, as `Rain` and `Rain (Live)` do.
    Holds,
    Exact,
}

/// Whether two names are one, by their letters and digits without
/// featured artists, or one holds the other; `None` when neither.
#[must_use]
pub fn names_match(a: &str, b: &str) -> Option<Exactness> {
    let (a, b) = (
        normalize(clean::without_credits(a)),
        normalize(clean::without_credits(b)),
    );
    if a.is_empty() || b.is_empty() {
        None
    } else if a == b {
        Some(Exactness::Exact)
    } else {
        (a.contains(&b) || b.contains(&a)).then_some(Exactness::Holds)
    }
}

#[cfg(test)]
mod tests;
