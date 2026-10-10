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
/// words by [`holds`].
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
const SUFFIXES: [&str; 3] = [" - ", " – ", " — "];

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

#[allow(clippy::cast_precision_loss)]
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

    /// The numbers it names, but for those in a decoration that names a
    /// version, as a remaster's year.
    fn numbers(&self) -> BTreeSet<String> {
        let numbered = |s: &str| -> Vec<String> {
            s.split(|c: char| !c.is_ascii_digit())
                .filter(|n| !n.is_empty())
                .map(|n| n.trim_start_matches('0').to_string())
                .collect()
        };
        let mut out: BTreeSet<String> = numbered(&self.core).into_iter().collect();
        for (d, words) in self.decorations.iter().zip(&self.decoration_words) {
            let versioned = VERSIONS
                .iter()
                .flat_map(|(_, e)| e.iter())
                .chain(&SOFT_VERSIONS);
            if !versioned.into_iter().any(|e| holds(words, e)) {
                out.extend(numbered(d));
            }
        }
        out
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
    let mut core = String::new();
    let mut decorations = Vec::new();
    let mut rest = s;
    while let Some((at, open, close)) = rest
        .char_indices()
        .find_map(|(i, c)| opener(c).map(|close| (i, c, close)))
    {
        let inner_at = at + open.len_utf8();
        let Some(len) = closing(&rest[inner_at..], open, close) else {
            break;
        };
        core.push_str(&rest[..at]);
        let inner = rest[inner_at..inner_at + len].trim();
        if !inner.is_empty() && !clean::is_packaging(inner) {
            decorations.push(inner.to_string());
        }
        rest = &rest[inner_at + len + close.len_utf8()..];
    }
    core.push_str(rest);
    let mut core = core.trim().to_string();
    if let Some((at, len)) = SUFFIXES
        .iter()
        .filter_map(|s| core.find(s).map(|at| (at, s.len())))
        .min()
        .filter(|&(at, _)| at > 0)
    {
        let suffix = core[at + len..].trim().to_string();
        if !suffix.is_empty() && !clean::is_packaging(&suffix) {
            decorations.push(suffix);
        }
        core.truncate(at);
    }
    if core.trim().is_empty() && !decorations.is_empty() {
        core = decorations.remove(0);
    }
    (core.trim().to_string(), decorations)
}

/// Where the bracket `close` that matches an `open` before `inner`
/// stands in it, past any pair nested inside.
fn closing(inner: &str, open: char, close: char) -> Option<usize> {
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

fn opener(c: char) -> Option<char> {
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

static GUESSES: LazyLock<[Regex; 4]> = LazyLock::new(|| {
    [
        r"^(?P<track>\d+)\.?\s*[-–—]\s*(?P<a>.+?)\s+[-–—]\s+(?P<b>.+)$",
        r"^(?P<a>.+?)\s+[-–—]\s+(?P<b>.+)$",
        r"^(?P<track>\d+)\.?[\s_-]+(?P<b>.+)$",
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

#[cfg(test)]
mod tests;
