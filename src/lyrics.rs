//! Which language a subtitle is in, what a line of lyrics is, and LRC
//! moved to another cut of the recording. [`crate::clean::lyrics`] cleans
//! lyrics by what the lines are read as here.
//!
//! A preference is a language tag (`ja`, `en`, `pt-BR`, `zh-Hant`),
//! matched as [`crate::lang`] matches one. The stream it names is found
//! by title, since yt-dlp titles each embedded stream with the display
//! name its info JSON records for a person's subtitle; a generated
//! caption has no name, so speech recognition of a song never becomes
//! its lyrics. The ISO 639-2 language tag is used only when the info
//! JSON says nothing about subtitles, and it is the weaker match:
//! yt-dlp tags Filipino `fin`, the code for Finnish.
//!
//! A line is no lyric when it is a cue or a credit. A cue is blank, only
//! music symbols, or wholly in brackets and of cue words (`[Chorus]`,
//! `(Guitar solo)`, `（間奏）`, `*GuitarSolo*`), or a short bracket naming a
//! passage played (`(gentle piano solo)`); brackets of other words hold a
//! backing vocal, which is sung. A credit is credit words, a separator
//! and a name, in the shapes `is_credit` lists, and is one only in a
//! block at the start or end of the lyrics, where credits sit: between
//! two lines sung, `Music by the water` is sung too.
//!
//! Lyrics come from LRCLIB, subtitles, tags and files anyone may write,
//! up to a body of megabytes, so each line is read in time linear in its
//! length: a time is read where it stands, never by copying the rest of
//! the line, and a closing `>` or `}` found missing once is not looked
//! for again from each later `<` or `{`.

use crate::lang::{Fit, Lang};
use crate::units;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::info::VideoInfo;
use crate::probe::Subtitle;

/// What a stream or file of lyrics is known to be written in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// The info JSON's codes whose display name a stream carries.
    Codes(Vec<String>),
    /// The stream's ISO 639-2 tag, when the info JSON lists no subtitles.
    Tag(String),
    /// A file of lyrics, which says nothing of its language; it is
    /// taken to be in the first preference.
    Unstated,
}

impl Language {
    /// The languages it may be: each code the info JSON gives the
    /// stream's display name, or the stream's tag; none for a file.
    #[must_use]
    pub fn langs(&self) -> Vec<Lang> {
        match self {
            // YouTube lists its original track again as `<code>-orig`.
            Self::Codes(codes) => codes
                .iter()
                .filter_map(|c| Lang::parse(c.strip_suffix("-orig").unwrap_or(c)))
                .collect(),
            Self::Tag(tag) => Lang::parse(tag).into_iter().collect(),
            Self::Unstated => Vec::new(),
        }
    }
}

/// What a subtitle stream is written in; `None` for a caption the site
/// generated, which carries no display name the info JSON knows.
#[must_use]
pub fn language(sub: &Subtitle, info: &VideoInfo) -> Option<Language> {
    if info.knows_subtitles() {
        let title = sub.title.as_deref()?;
        let codes: Vec<String> = info
            .subtitles
            .keys()
            .filter(|code| info.subtitle_names(code).contains(&title))
            .cloned()
            .collect();
        return (!codes.is_empty()).then_some(Language::Codes(codes));
    }
    sub.language.clone().map(Language::Tag)
}

/// The first preference `lang` meets, and how closely.
#[must_use]
pub fn preference(prefs: &[Lang], lang: &Language) -> Option<(usize, Fit)> {
    if *lang == Language::Unstated {
        return (!prefs.is_empty()).then_some((0, Fit::Exact));
    }
    let langs = lang.langs();
    prefs.iter().enumerate().find_map(|(n, p)| {
        let fit = langs.iter().filter_map(|l| p.fit(l)).min()?;
        Some((n, fit))
    })
}

/// The subtitle stream to turn into lyrics: the first preference with a
/// match wins, a stream named for it before one only tagged with it.
#[must_use]
pub fn pick<'a>(
    prefs: &[Lang],
    subtitles: &'a [Subtitle],
    info: &VideoInfo,
) -> Option<&'a Subtitle> {
    subtitles
        .iter()
        .filter_map(|s| Some((preference(prefs, &language(s, info)?)?, s)))
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, s)| s)
}

/// What a line of lyrics is, read on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Sung,
    /// Blank, only music symbols, or a bracketed cue as `[Music]`.
    Cue,
    /// Shaped as a credit, `作詞：…` or `Music by …`: one only where
    /// credits sit, before the first line sung or after the last.
    Credit,
}

/// Words that make a wholly bracketed line a cue: a section, a sound
/// that is not sung, or a repeat. A line in brackets of other words, as
/// `(Down by the river)`, is a backing vocal and sung.
const CUES: [&str; 50] = [
    "instrumental",
    "music",
    "applause",
    "laughter",
    "laughing",
    "laughs",
    "cheering",
    "cheers",
    "chorus",
    "prechorus",
    "verse",
    "intro",
    "outro",
    "bridge",
    "hook",
    "refrain",
    "solo",
    "interlude",
    "repeat",
    "repeats",
    "spoken",
    "speaking",
    "sung",
    "singing",
    "inaudible",
    "unintelligible",
    "incomprehensible",
    "humming",
    "fade",
    "fades",
    "mv",
    "音楽",
    "間奏",
    "前奏",
    "後奏",
    "拍手",
    "笑い",
    "歓声",
    "サビ",
    "繰り返し",
    "語り",
    "セリフ",
    "台詞",
    "ソロ",
    "ギターソロ",
    "イントロ",
    "アウトロ",
    "コーラス",
    "ブリッジ",
    "間奏中",
];

/// Words a cue may hold beside its cue words, as `(Guitar solo)` or
/// `[Music fades out]`, which alone make no cue.
const CUE_FILLERS: [&str; 25] = [
    "a",
    "and",
    "the",
    "of",
    "with",
    "in",
    "out",
    "pre",
    "post",
    "part",
    "end",
    "x",
    "break",
    "fading",
    "playing",
    "plays",
    "continues",
    "ends",
    "guitar",
    "piano",
    "drum",
    "drums",
    "bass",
    "crowd",
    "only",
];

/// How a short bracket of any words ends when it names a passage played,
/// not sung, as `(gentle piano solo)` or `(Irish dance break)`.
const CUE_ENDINGS: [&[&str]; 5] = [
    &["solo"],
    &["interlude"],
    &["instrumental"],
    &["dance", "break"],
    &["instrumental", "break"],
];

/// The pairs a cue or a credit may be wholly wrapped in.
pub(crate) const WRAPPERS: [(char, char); 5] = [
    ('[', ']'),
    ('(', ')'),
    ('（', '）'),
    ('【', '】'),
    ('*', '*'),
];

/// What `text` holds inside one pair of [`WRAPPERS`] around all of it.
pub(crate) fn wrapped(text: &str) -> Option<&str> {
    let t = text.trim();
    WRAPPERS.iter().find_map(|(o, c)| {
        let inner = t.strip_prefix(*o)?.strip_suffix(*c)?;
        (!inner.contains(*c)).then_some(inner)
    })
}

/// Lower-cased words, split where letters run from lower to upper case
/// too, so `GuitarSolo` is `guitar solo`.
fn cue_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut prev: Option<char> = None;
    for c in text.chars() {
        let split =
            !c.is_alphanumeric() || (c.is_uppercase() && prev.is_some_and(char::is_lowercase));
        if split && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        }
        prev = Some(c);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Whether a word is a repeat, as `x2` or `3x`.
fn is_repeat(word: &str) -> bool {
    let digits = word
        .strip_prefix('x')
        .or_else(|| word.strip_suffix('x'))
        .unwrap_or("");
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Whether the words inside a pair of brackets are a cue's: every one a
/// cue word, a filler or a number, and one at least a cue word or a
/// repeat; or at most four words ending as [`CUE_ENDINGS`] say. What
/// follows a colon names who sings, as `[Chorus: …]`.
pub(crate) fn is_cue(inner: &str) -> bool {
    let head = inner.split([':', '：']).next().unwrap_or(inner);
    let words = cue_words(&head.replace('×', "x"));
    let cue = |w: &String| CUES.contains(&w.as_str()) || is_repeat(w);
    let ends = words.len() <= 4
        && CUE_ENDINGS
            .iter()
            .any(|end| words.ends_with(&end.iter().map(|w| (*w).to_string()).collect::<Vec<_>>()));
    ends || words.iter().any(cue)
        && words.iter().all(|w| {
            cue(w) || CUE_FILLERS.contains(&w.as_str()) || w.bytes().all(|b| b.is_ascii_digit())
        })
}

/// Words that name a part in making a song or its video, as `Music` in
/// `Music by …` or `作曲` in `作曲：…`.
const CREDITS: [&str; 80] = [
    "lyrics",
    "lyric",
    "lyricist",
    "words",
    "music",
    "song",
    "songwriter",
    "songwriting",
    "written",
    "writer",
    "composer",
    "composed",
    "composition",
    "arrange",
    "arranged",
    "arranger",
    "arrangement",
    "producer",
    "produced",
    "production",
    "vocal",
    "vocals",
    "voice",
    "guitar",
    "bass",
    "drums",
    "piano",
    "keyboards",
    "illust",
    "illustration",
    "illustrated",
    "illustrator",
    "animated",
    "animator",
    "choreography",
    "director",
    "directed",
    "editing",
    "edited",
    "editor",
    "proofreading",
    "assistant",
    "art",
    "artwork",
    "movie",
    "video",
    "mv",
    "pv",
    "animation",
    "mix",
    "mixed",
    "mixing",
    "mastering",
    "mastered",
    "tuning",
    "creator",
    "title",
    "translation",
    "translated",
    "translator",
    "subtitles",
    "subtitle",
    "subs",
    "cc",
    "timing",
    "romanization",
    "作詞",
    "作词",
    "詞",
    "词",
    "作曲",
    "曲",
    "編曲",
    "编曲",
    "唄",
    "絵",
    "動画",
    "翻訳",
    "翻译",
    "字幕",
];

/// Words that join credit words, as `Music and lyrics by …`.
const JOINERS: [&str; 2] = ["and", "with"];

/// A word of `text` and where it ends.
fn words_at(text: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (start, c.is_alphanumeric()) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                out.push((text[s..i].to_lowercase(), i));
                start = None;
            }
            _ => {}
        }
    }
    out
}

fn is_credit_word(word: &str) -> bool {
    CREDITS.contains(&word)
}

/// Whether a name may follow a credit: none, one starting with no
/// lowercase letter, or one word, as a handle `someone_x` is.
fn is_name(rest: &str) -> bool {
    let rest = rest.trim();
    rest.is_empty()
        || rest.chars().next().is_some_and(|c| !c.is_lowercase())
        || !rest.contains(char::is_whitespace)
}

/// Whether `text` is shaped as a credit:
///
/// - a head of credit words, at most one other word and qualifiers in
///   brackets, a colon and a name: `Translation (English) : …`,
///   `Lyrics/music: …`, `English translation: …`;
/// - credit words, at most two other words, `by` and a name, or nothing:
///   `Composed and written by …`, `Lyrics posted by …`, `Arrangement by`;
/// - a role of [`ROLES`] alone, or before up to four capitalized words:
///   `Voice Kasane Teto`;
/// - an address: an e-mail, a URL or an `@handle`;
/// - any of these wholly in brackets: `(Subtitles: …)`.
///
/// A name starts with no lowercase letter or is one word, so `Music by
/// the water` and `Mix: the night and the morning light` are sung.
pub(crate) fn is_credit(text: &str) -> bool {
    let t = text.trim();
    if let Some(inner) = wrapped(t)
        && is_credit(inner)
    {
        return true;
    }
    is_address(t) || headed(t) || by_credit(t) || named(t)
}

fn is_address(t: &str) -> bool {
    let lower = t.to_ascii_lowercase();
    lower.contains("://")
        || lower.contains("www.")
        || t.split('@')
            .skip(1)
            .any(|after| after.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
}

/// How a credit with a colon reads: its head words, how many are credit
/// words and how many others, and what follows the colon.
fn colon(t: &str) -> Option<(Vec<String>, usize, usize, &str)> {
    let at = t.find([':', '：'])?;
    let mut depth = 0_usize;
    let head: String = t[..at]
        .chars()
        .map(|c| {
            match c {
                '(' | '（' => depth += 1,
                ')' | '）' => depth = depth.saturating_sub(1),
                _ if depth == 0 => return c,
                _ => {}
            }
            ' '
        })
        .collect();
    let words: Vec<String> = words_at(&head).into_iter().map(|(w, _)| w).collect();
    let credits = words.iter().filter(|w| is_credit_word(w)).count();
    let others = words
        .iter()
        .filter(|w| !is_credit_word(w) && !JOINERS.contains(&w.as_str()))
        .count();
    let rest = t[at..].trim_start_matches([':', '：']);
    Some((words, credits, others, rest))
}

fn headed(t: &str) -> bool {
    colon(t).is_some_and(|(words, credits, others, rest)| {
        ((credits > 0 && others <= 1 && words.len() <= 4) || is_credit_word(&words.concat()))
            && is_name(rest)
    })
}

/// Whether `text` is a credit wherever it stands: an address, or credit
/// words alone before a colon, as `Subtitles: …` or `Vocal:`, which no
/// line sung is shaped as.
pub(crate) fn is_sure_credit(text: &str) -> bool {
    let t = text.trim();
    if let Some(inner) = wrapped(t)
        && is_sure_credit(inner)
    {
        return true;
    }
    is_address(t)
        || colon(t).is_some_and(|(words, credits, others, rest)| {
            (credits > 0 && others == 0 && words.len() <= 4 || is_credit_word(&words.concat()))
                && is_name(rest)
        })
}

/// Whether `text` is a credit whose name is on the next line, as
/// `Vocal:` or `Arrangement by`.
pub(crate) fn dangles(text: &str) -> bool {
    let t = text.trim();
    is_credit(t)
        && (t.ends_with([':', '：'])
            || words_at(t)
                .last()
                .is_some_and(|(w, end)| w == "by" && t[*end..].trim().is_empty()))
}

fn by_credit(t: &str) -> bool {
    let words = words_at(t);
    let Some(by) = words.iter().position(|(w, _)| w == "by") else {
        return false;
    };
    let rest = t[words[by].1..]
        .trim_start()
        .trim_start_matches([':', '：']);
    if by == 0 {
        return false;
    }
    let before = &words[..by];
    let credits = before.iter().filter(|(w, _)| is_credit_word(w)).count();
    let others = before
        .iter()
        .filter(|(w, _)| !is_credit_word(w) && !JOINERS.contains(&w.as_str()))
        .count();
    is_credit_word(&before[0].0) && credits > 0 && others <= 2 && is_name(rest)
}

/// Credit words that never open a line sung, which may stand before a
/// name with no separator: `Voice Kasane Teto`.
const ROLES: [&str; 16] = [
    "vocal",
    "vocals",
    "voice",
    "creator",
    "illust",
    "illustration",
    "art",
    "artwork",
    "mv",
    "pv",
    "mastering",
    "tuning",
    "translation",
    "subtitles",
    "cc",
    "lyricist",
];

fn named(t: &str) -> bool {
    let words = words_at(t);
    let Some(((first, end), rest)) = words.split_first() else {
        return false;
    };
    let tail = t[*end..].trim();
    let shouted = !t.chars().any(char::is_lowercase);
    ROLES.contains(&first.as_str())
        && !shouted
        && rest.len() <= 4
        && tail
            .split_whitespace()
            .all(|w| w.chars().next().is_some_and(|c| !c.is_lowercase()))
}

/// What the line `text` is, read on its own.
pub(crate) fn kind(text: &str) -> Kind {
    let t = text.trim();
    if t.chars().all(|c| !c.is_alphanumeric()) {
        return Kind::Cue;
    }
    if wrapped(t).is_some_and(is_cue) {
        Kind::Cue
    } else if is_credit(t) {
        Kind::Credit
    } else {
        Kind::Sung
    }
}

/// Which of `kinds`, one text's lines of words in order, are no lyric:
/// the cues, and the credits in a block at its start or end. A credit's
/// shape between two lines sung is a line sung, as `Music by the water`.
pub(crate) fn unsung(kinds: &[Kind]) -> Vec<bool> {
    let first = kinds.iter().position(|k| *k == Kind::Sung);
    let last = kinds.iter().rposition(|k| *k == Kind::Sung);
    kinds
        .iter()
        .enumerate()
        .map(|(i, k)| match (k, first, last) {
            (Kind::Sung, ..) => false,
            (Kind::Credit, Some(first), Some(last)) => i < first || i > last,
            _ => true,
        })
        .collect()
}

/// The value of the first ID tag `name` in the LRC `text`, read in any
/// case and with spaces around the colon, as `[TI:]` or `[length : ]`.
fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let inner = l.trim().strip_prefix('[')?.strip_suffix(']')?;
        let (k, v) = inner.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(name) && !v.trim().is_empty()).then(|| v.trim())
    })
}

/// The length an `.lrc` states with its `[length:]` tag, in ms; none
/// when it states no positive length.
#[must_use]
pub fn stated_length(text: &str) -> Option<i64> {
    let mut seconds = 0.0;
    for part in tag(text, "length")?.split(':') {
        seconds = seconds * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    Some(units::ms_of_seconds(seconds).round() as i64)
}

/// What an `.lrc`'s ID tags say of its song.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// `[length:]`, in ms.
    pub length_ms: Option<i64>,
    /// `[offset:]`: how much earlier every line is meant, in ms.
    pub offset_ms: i64,
}

/// The ID tags of the LRC `text`: `[ti:]`, `[ar:]`, `[al:]`, `[length:]`
/// and `[offset:]`.
#[must_use]
pub fn headers(text: &str) -> Headers {
    let tag = |name| tag(text, name).map(str::to_string);
    Headers {
        title: tag("ti"),
        artist: tag("ar"),
        album: tag("al"),
        length_ms: stated_length(text),
        offset_ms: tag("offset").and_then(|o| o.parse().ok()).unwrap_or(0),
    }
}

/// `line` without the word timings an enhanced LRC line holds, as
/// `<00:12.30>`; any other `<` or `>` is sung.
pub(crate) fn without_word_times(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if time_closed_by(after, &[']', '>']).is_some() {
            let close = after.find('>').map_or(after.len(), |c| c + 1);
            rest = &after[close..];
        } else {
            out.push('<');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// The words LRC or plain lyrics sing: each timed line's text, and each
/// untimed line but the ID tags, without the lines that are no lyric.
#[must_use]
pub fn sung(text: &str) -> String {
    let mut words = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut timed = false;
        while let Some((_, after)) = rest.strip_prefix('[').and_then(lrc_time) {
            timed = true;
            rest = after;
        }
        let line = without_word_times(rest);
        let header = !timed && line.starts_with('[') && line.ends_with(']');
        if !header && !line.trim().is_empty() {
            words.push(line.trim().to_string());
        }
    }
    let kinds: Vec<Kind> = words.iter().map(|w| kind(w)).collect();
    let mut out = String::new();
    for (line, unsung) in words.iter().zip(unsung(&kinds)) {
        if !unsung {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The text of a lyrics file, whatever wrote it: UTF-8 or UTF-16 by its
/// byte-order mark, else UTF-8 when it reads as UTF-8, else the legacy
/// encoding its bytes look most like, as GBK, Shift-JIS or Windows-1252,
/// guessed as a browser guesses a page that names none. CRLF and CR
/// line endings become LF.
#[must_use]
pub fn decode(bytes: &[u8]) -> String {
    let encoding = if let Some(utf16) = utf16_unmarked(bytes) {
        utf16
    } else if is_utf8(bytes) {
        encoding_rs::UTF_8
    } else {
        let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
        detector.feed(bytes, true);
        detector.guess(None, chardetng::Utf8Detection::Deny)
    };
    let (text, _, _) = encoding.decode(bytes);
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// UTF-16 with no byte-order mark, told by its zero bytes: an ASCII
/// character has one in its pair, on the side its byte order says.
fn utf16_unmarked(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    if encoding_rs::Encoding::for_bom(bytes).is_some() || bytes.len() < 4 {
        return None;
    }
    let (pairs, _) = bytes.as_chunks::<2>();
    let total = pairs.len();
    let (high, low) = pairs.iter().fold((0, 0), |(h, l), p| {
        (h + usize::from(p[1] == 0), l + usize::from(p[0] == 0))
    });
    // UTF-8 and the legacy encodings hold no NUL; an LRC's times and
    // brackets put one in every other byte of an eighth of its pairs at
    // the least, whatever script its words are in.
    if high * 8 > total && low * 10 < total {
        Some(encoding_rs::UTF_16LE)
    } else if low * 8 > total && high * 10 < total {
        Some(encoding_rs::UTF_16BE)
    } else {
        None
    }
}

/// Whether `text` reads as text: almost none of it control characters or
/// bytes that would not decode, as a file of other data holds.
#[must_use]
pub fn is_text(text: &str) -> bool {
    let odd = text
        .chars()
        .filter(|c| *c == '\u{FFFD}' || c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        .count();
    odd * 50 <= text.chars().count()
}

/// Whether `bytes` are UTF-8, perhaps damaged: fewer bytes in error
/// than characters past ASCII read right. Text in another encoding read
/// as UTF-8 is almost all errors, so a damaged UTF-8 file loses only
/// what is damaged rather than reading as another encoding throughout.
fn is_utf8(bytes: &[u8]) -> bool {
    let (mut read, mut bad) = (0, 0);
    for chunk in bytes.utf8_chunks() {
        read += chunk.valid().chars().filter(|c| !c.is_ascii()).count();
        bad += chunk.invalid().len();
    }
    bad < read.max(1)
}

/// LRC cleaned by every rule of [`crate::clean::lyrics`], knowing
/// nothing of its song: what a source's lyrics are measured by, whatever
/// the song list switches.
#[must_use]
pub fn clean_lrc(text: &str) -> String {
    crate::clean::lyrics::all(text)
}

/// The timed lines of LRC that sing a word, as a line of no words only
/// ends the one before: how many, and the first and last time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timing {
    pub lines: usize,
    pub first_ms: i64,
    pub last_ms: i64,
}

#[must_use]
pub fn timing(text: &str) -> Option<Timing> {
    let mut times = Vec::new();
    for line in text.lines() {
        let mut rest = line;
        let mut stamps = Vec::new();
        while let Some((ms, after)) = rest.strip_prefix('[').and_then(lrc_time) {
            stamps.push(ms);
            rest = after;
        }
        if !without_word_times(rest).trim().is_empty() {
            times.extend(stamps);
        }
    }
    Some(Timing {
        lines: times.len(),
        first_ms: *times.iter().min()?,
        last_ms: *times.iter().max()?,
    })
}

/// One line of LRC: the times it starts at and what follows them, its
/// words with any word times; or a line of no time, as an ID tag.
enum Line<'a> {
    Timed { times: Vec<i64>, rest: &'a str },
    Other(&'a str),
}

/// LRC read line by line, its `[offset:]` taken out and kept apart, so
/// moving it moves every time it holds, a word's as a line's.
struct Lrc<'a> {
    lines: Vec<Line<'a>>,
    /// `[offset:]`: how much earlier every time is meant, in ms.
    offset_ms: i64,
}

impl<'a> Lrc<'a> {
    fn parse(text: &'a str) -> Self {
        let mut offset_ms = 0;
        let mut lines = Vec::new();
        for line in text.lines() {
            let mut rest = line;
            let mut times = Vec::new();
            while let Some((ms, after)) = rest.strip_prefix('[').and_then(lrc_time) {
                times.push(ms);
                rest = after;
            }
            if !times.is_empty() {
                lines.push(Line::Timed { times, rest });
            } else if let Some(offset) = id_tag(line, "offset") {
                offset_ms = offset.parse().unwrap_or(0);
            } else {
                lines.push(Line::Other(line));
            }
        }
        Self { lines, offset_ms }
    }
}

/// The value of the line `line` if it is the ID tag `name`, as `[offset:+500]`.
pub(crate) fn id_tag<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let inner = line.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (k, v) = inner.split_once(':')?;
    k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
}

/// A time as LRC writes it, between `open` and `close`, rounded to the
/// hundredth of a second it holds.
fn stamp(out: &mut String, ms: i64, open: char, close: char) {
    let cs = ms.max(0).saturating_add(5) / 10;
    let _ = write!(
        out,
        "{open}{:02}:{:02}.{:02}{close}",
        cs / 6000,
        cs / 100 % 60,
        cs % 100
    );
}

/// `words` with each word time, as `<00:12.30>`, moved by `moved`.
fn move_words(out: &mut String, words: &str, moved: &dyn Fn(i64) -> i64) {
    let mut rest = words;
    let mut unclosed = false;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let ms = time_closed_by(after, &[']', '>']).map(|(ms, _)| ms);
        let close = ms.filter(|_| !unclosed).and_then(|_| after.find('>'));
        unclosed |= ms.is_some() && close.is_none();
        if let Some((close, ms)) = close.zip(ms) {
            stamp(out, moved(ms), '<', '>');
            rest = &after[close + 1..];
        } else {
            out.push('<');
            rest = after;
        }
    }
    out.push_str(rest);
}

/// LRC timed for another cut of the recording: every time it holds, a
/// line's `[mm:ss.xx]` and a word's `<mm:ss.xx>`, moved by its own
/// `[offset:]`, which it then needs no more, then `offset_ms` earlier,
/// then shortened by `stretch_ppm` parts per million for a cut that plays
/// the recording that much longer. Of the lines that would start before
/// the track does, the one still sung as it begins starts at 0:00 and
/// those over by then are dropped; untimed lines are kept, but for a
/// `[length:]`, which no more says how long the lyrics run once moved.
#[must_use]
pub fn shift_lrc(text: &str, offset_ms: i64, stretch_ppm: i64) -> String {
    let lrc = Lrc::parse(text);
    let moving = offset_ms != 0 || stretch_ppm != 0 || lrc.offset_ms != 0;
    let moved = |ms: i64| {
        unstretch(
            ms.saturating_sub(lrc.offset_ms).saturating_sub(offset_ms),
            stretch_ppm,
        )
    };
    let all: Vec<i64> = lrc
        .lines
        .iter()
        .flat_map(|l| match l {
            Line::Timed { times, .. } => times.clone(),
            Line::Other(_) => Vec::new(),
        })
        .map(moved)
        .collect();
    let begun = all
        .iter()
        .copied()
        .filter(|ms| *ms < 0)
        .max()
        .filter(|_| !all.contains(&0));
    let mut out = String::new();
    for line in &lrc.lines {
        match line {
            Line::Timed { times, rest } => {
                let starts: Vec<i64> = times
                    .iter()
                    .map(|ms| moved(*ms))
                    .filter(|ms| *ms >= 0 || Some(*ms) == begun)
                    .collect();
                if starts.is_empty() {
                    continue;
                }
                for ms in starts {
                    stamp(&mut out, ms, '[', ']');
                }
                move_words(&mut out, rest, &moved);
            }
            Line::Other(text) if moving && id_tag(text, "length").is_some() => continue,
            Line::Other(text) => out.push_str(text),
        }
        out.push('\n');
    }
    out
}

/// `words` with each word time, as `<00:12.30>`, written as `write`
/// writes its milliseconds, or left out where it writes none; any other
/// `<` is sung.
pub(crate) fn rewrite_word_times(
    words: &str,
    write: &mut dyn FnMut(i64) -> Option<String>,
) -> String {
    let mut out = String::with_capacity(words.len());
    let mut rest = words;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some((ms, close)) = time_closed_by(after, &['>']) {
            out.extend(write(ms));
            rest = &after[close + 1..];
        } else {
            out.push('<');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// `mm:ss.xx]` or `mm:ss.xxx]` at the start of `s`, in milliseconds,
/// and what follows the bracket.
pub(crate) fn lrc_time(s: &str) -> Option<(i64, &str)> {
    let (ms, close) = time_closed_by(s, &[']'])?;
    Some((ms, &s[close + 1..]))
}

/// A time at the start of `s`, closed by the first of `closers` after it,
/// and where that closer is. A time holds only digits, `:` and `.`, so it
/// is read without looking past them: a line of a thousand `<` reads each
/// in a step, not to the line's end.
fn time_closed_by(s: &str, closers: &[char]) -> Option<(i64, usize)> {
    let close = s.find(|c: char| !(c.is_ascii_digit() || c == ':' || c == '.'))?;
    if !s[close..].starts_with(closers) {
        return None;
    }
    let tag = &s[..close];
    let (min, sec) = tag.split_once(':')?;
    // `[01:02.50]`, or `[01:02:50]` as some tools write it.
    let (whole, frac) = sec.split_once(['.', ':']).unwrap_or((sec, "0"));
    let digits = |d: &str| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit());
    if !(digits(min) && digits(whole) && digits(frac)) {
        return None;
    }
    let frac_ms = match frac.len() {
        1 => frac.parse::<i64>().ok()? * 100,
        2 => frac.parse::<i64>().ok()? * 10,
        _ => frac[..3].parse::<i64>().ok()?,
    };
    let ms = min
        .parse::<i64>()
        .ok()?
        .checked_mul(60_000)?
        .checked_add(whole.parse::<i64>().ok()?.checked_mul(1000)?)?
        .checked_add(frac_ms)?;
    Some((ms, close))
}

/// `ms` played `stretch_ppm` parts per million longer, brought back.
fn unstretch(ms: i64, stretch_ppm: i64) -> i64 {
    if stretch_ppm == 0 {
        return ms;
    }
    (ms as f64 / (1.0 + units::ratio_of_ppm(stretch_ppm as f64))).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_of_unclosed_brackets_is_read_in_one_pass() {
        let n = 200_000;
        for body in [
            "<".repeat(n),
            "<a".repeat(n / 2),
            "<1:2]".repeat(n / 5),
            "{\\".repeat(n / 2),
        ] {
            let text = format!("[00:01.00]Paper comets {body}\n");
            assert!(clean_lrc(&text).len() <= text.len());
            assert_eq!(timing(&text).map(|t| t.lines), Some(1));
            assert!(sung(&text).len() <= text.len());
            assert!(shift_lrc(&text, 500, 0).starts_with("[00:00.50]Paper comets"));
        }
        assert_eq!(
            shift_lrc("[00:02.00]a <00:03.00>b <00:04.00]c <5\n", 1000, 0),
            "[00:01.00]a <00:02.00>b <00:04.00]c <5\n",
            "a word time is closed by `>`"
        );
    }

    #[test]
    fn signs_are_sung_word_times_are_not_and_huge_times_are_no_times() {
        assert_eq!(sung("[00:01.00]we go -> home"), "we go -> home\n");
        assert_eq!(
            sung("plain words > here\nI <3 the tide\n"),
            "plain words > here\nI <3 the tide\n"
        );
        assert_eq!(
            sung("[00:01.00]<00:01.00>rain <00:01.50>falls"),
            "rain falls\n"
        );
        assert_eq!(
            sung("[9999999999999999:00.00]x"),
            "[9999999999999999:00.00]x\n"
        );
        assert_eq!(timing("[9999999999999999:00.00]x"), None);
        let _ = shift_lrc("[153722867280912:55.807]x", -1, 0);
        assert_eq!(
            decode(b"[00:01.00]a\r[00:02.00]b\r"),
            "[00:01.00]a\n[00:02.00]b\n"
        );
    }

    #[test]
    fn lyrics_files_decode_from_any_common_encoding() {
        let lrc = "[00:01.00]雨\n";
        let crlf = "[00:01.00]雨\r\n";
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(crlf.as_bytes());
        let le: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(crlf.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        let be: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain(crlf.encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        for bytes in [lrc.as_bytes(), &bom, &le, &be] {
            assert_eq!(decode(bytes), lrc);
        }
    }

    #[test]
    fn lyrics_files_in_a_legacy_encoding_decode_by_their_bytes() {
        let texts = [
            (
                encoding_rs::GBK,
                "[00:01.00]灯塔下的雨还在下\n[00:05.00]我们沿着河岸走回家\n[00:09.00]风吹过安静的街道\n",
            ),
            (
                encoding_rs::SHIFT_JIS,
                "[00:01.00]灯台の下で雨が降っている\n[00:05.00]川沿いの道を歩いて帰ろう\n[00:09.00]静かな町に風が吹く\n",
            ),
            (
                encoding_rs::WINDOWS_1252,
                "[00:01.00]Déjà la pluie sur le phare\n[00:05.00]On rentre à pied, côté rivière\n",
            ),
        ];
        for (encoding, lrc) in texts {
            let (bytes, _, lossy) = encoding.encode(lrc);
            assert!(!lossy);
            assert_eq!(decode(&bytes), lrc, "{}", encoding.name());
        }
    }

    #[test]
    fn a_time_with_a_colon_before_its_fraction_reads() {
        assert_eq!(lrc_time("01:02:50]x"), Some((62_500, "x")));
        assert_eq!(lrc_time("01:02.50]x"), Some((62_500, "x")));
    }

    #[test]
    fn utf16_without_a_mark_is_told_and_other_data_is_no_text() {
        let lrc = "[00:01.00]first line\n[00:05.00]second line\n";
        let le: Vec<u8> = lrc.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = lrc.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(decode(&le), lrc);
        assert_eq!(decode(&be), lrc);
        assert!(is_text(lrc));
        let cyrillic = "[00:07.43]Пример первой строки песни\n[00:11.00]и второй строки\n";
        let le: Vec<u8> = cyrillic.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(&le), cyrillic);
        let noise: Vec<u8> = (0..4000_u32)
            .map(|i| i.wrapping_mul(2_654_435_761).to_le_bytes()[2])
            .collect();
        assert!(!is_text(&decode(&noise)));
    }

    #[test]
    fn subtitle_markup_is_no_lyric() {
        assert_eq!(
            clean_lrc("[00:40.00]<i>Third</i> line in <font color=\"#fff\">ink</font>\n"),
            "[00:40.00]Third line in ink\n"
        );
        assert_eq!(
            clean_lrc("[00:41.00]{\\an8}<00:41.50>Lantern <00:42.00>weather\n"),
            "[00:41.00]<00:41.50>Lantern <00:42.00>weather\n"
        );
        assert_eq!(clean_lrc("[00:43.00]a < b {c}\n"), "[00:43.00]a < b {c}\n");
    }

    #[test]
    fn a_damaged_utf8_file_loses_only_its_damage() {
        let mut bytes = "[00:01.00]灯塔下的雨\n[00:05.00]Déjà vu\n"
            .as_bytes()
            .to_vec();
        bytes[13] = 0xFF;
        let text = decode(&bytes);
        assert!(text.contains("的雨") && text.contains("Déjà vu"), "{text}");
        assert!(text.contains('\u{FFFD}'));
    }
    use crate::info;

    fn sub(index: u32, language: &str, title: &str) -> Subtitle {
        Subtitle {
            index,
            language: Some(language.into()),
            title: Some(title.into()),
        }
    }

    fn video(json: &str) -> VideoInfo {
        info::parse(json.as_bytes()).unwrap()
    }

    fn prefs(p: &[&str]) -> Vec<Lang> {
        p.iter().map(|s| Lang::parse(s).unwrap()).collect()
    }

    // The streams and names of a real original.
    fn real() -> (Vec<Subtitle>, VideoInfo) {
        let subs = vec![
            sub(2, "zho", "Chinese (Traditional)"),
            sub(3, "eng", "English"),
            sub(4, "fin", "Filipino"),
            sub(5, "fra", "French"),
        ];
        let info = video(
            r#"{"subtitles": {
                "zh-Hant": [{"name": "Chinese (Traditional)"}],
                "en": [{"name": "English"}],
                "fil": [{"name": "Filipino"}],
                "fr": [{"name": "French"}]}}"#,
        );
        (subs, info)
    }

    #[test]
    fn a_missing_first_choice_falls_through_to_the_next() {
        let (subs, info) = real();
        assert_eq!(
            pick(&prefs(&["ja", "en"]), &subs, &info).map(|s| s.index),
            Some(3)
        );
    }

    #[test]
    fn the_title_finds_a_stream_its_tag_gets_wrong() {
        let (subs, info) = real();
        assert_eq!(
            pick(&prefs(&["fil"]), &subs, &info).map(|s| s.index),
            Some(4)
        );
    }

    #[test]
    fn a_base_code_covers_its_regional_variants() {
        let (subs, info) = real();
        assert_eq!(
            pick(&prefs(&["zh"]), &subs, &info).map(|s| s.index),
            Some(2)
        );
    }

    #[test]
    fn the_language_tag_is_used_when_the_info_has_no_names() {
        let subs = vec![sub(5, "jpn", "")];
        assert_eq!(
            pick(&prefs(&["ja"]), &subs, &VideoInfo::default()).map(|s| s.index),
            Some(5)
        );
    }

    #[test]
    fn a_generated_caption_is_never_the_lyrics() {
        // Only YouTube's captions exist in Japanese: the stream carries
        // the right tag but no person wrote it.
        let subs = vec![sub(2, "jpn", ""), sub(3, "eng", "English")];
        let info = video(
            r#"{"subtitles": {"en": [{"name": "English"}]},
                "automatic_captions": {"ja": [{"ext": "vtt"}]}}"#,
        );
        assert_eq!(
            pick(&prefs(&["ja", "en"]), &subs, &info).map(|s| s.index),
            Some(3)
        );
    }

    #[test]
    fn nothing_matches_no_preference() {
        let (subs, info) = real();
        assert!(pick(&prefs(&["ko"]), &subs, &info).is_none());
        assert!(pick(&[], &subs, &info).is_none());
    }

    #[test]
    fn a_stream_named_for_a_preference_beats_a_later_one() {
        let (subs, info) = real();
        assert_eq!(
            preference(&prefs(&["fr", "en"]), &language(&subs[1], &info).unwrap()),
            Some((1, Fit::Exact))
        );
        assert_eq!(
            preference(&prefs(&["ja"]), &Language::Unstated),
            Some((0, Fit::Exact)),
            "a file is taken at its word"
        );
    }

    #[test]
    fn cues_symbols_and_credits_are_no_lyrics() {
        for no in [
            "♪",
            "  ",
            "[Music]",
            "（音楽）",
            "作詞：ほしのきり",
            "Music by X",
            "Lyrics: Y",
            "【MV】",
            "[Chorus]",
            "[Verse 2: Marlo Venn]",
            "(Guitar solo)",
            "(x2)",
            "（間奏）",
            "Lyrics/music: Marlo Venn",
        ] {
            assert_eq!(sung(no), "", "{no}");
        }
        for yes in [
            "Music fills the harbor",
            "(I) miss the tide (so)",
            "灯台の夢",
            "Lyrics of a song I wrote",
            "(Down by the river, down by the shore)",
            "Music by the water plays all night",
            "Mix: the night and the morning light",
        ] {
            assert_eq!(sung(yes), format!("{yes}\n"), "{yes}");
        }
    }

    #[test]
    fn a_credit_is_one_only_before_the_first_line_sung_or_after_the_last() {
        let lrc = "[00:01.00]Lyrics: Marlo Venn\n[00:02.00]作曲：ほしのきり\n\
                   [00:05.00]The lamps along the quay\n\
                   [00:08.00](Down by the quay, down by the lamps)\n\
                   [00:11.00]Music by Ada Quill burns low\n\
                   [00:14.00][Chorus]\n[00:17.00]Last of the lamps\n\
                   [00:20.00]Mix: Tobin Ash\n";
        assert_eq!(
            clean_lrc(lrc),
            "[00:05.00]The lamps along the quay\n\
             [00:08.00](Down by the quay, down by the lamps)\n\
             [00:11.00]Music by Ada Quill burns low\n\
             [00:17.00]Last of the lamps\n"
        );
        assert_eq!(
            sung("Arranged by Tobin Ash\n\nfirst line\nArranged by Tobin Ash\nlast line\n"),
            "first line\nArranged by Tobin Ash\nlast line\n"
        );
    }

    #[test]
    fn cleaning_drops_timed_cues_and_keeps_headers() {
        assert_eq!(
            clean_lrc("[ar:x]\n[00:01.00]♪\n[00:02.00]sung\n[00:03.00][Music]\n"),
            "[ar:x]\n[00:02.00]sung\n"
        );
    }

    #[test]
    fn timing_spans_the_timed_lines() {
        assert_eq!(
            timing("[ar:x]\n[00:02.00]a\n[01:00.50]b\n"),
            Some(Timing {
                lines: 2,
                first_ms: 2000,
                last_ms: 60_500
            })
        );
        assert_eq!(timing("plain words\n"), None);
    }

    #[test]
    fn lrc_moves_earlier_by_the_offset() {
        let lrc = "[00:01.85](intro)\n[00:05.45]line\n[01:02.300][01:10.00]twice\n[ar:x]\n";
        assert_eq!(
            shift_lrc(lrc, 920, 0),
            "[00:00.93](intro)\n[00:04.53]line\n[01:01.38][01:09.08]twice\n[ar:x]\n"
        );
    }

    #[test]
    fn a_line_over_before_the_track_starts_is_dropped_and_one_still_sung_starts_it() {
        assert_eq!(
            shift_lrc("[00:00.20]gone\n[00:00.50]sung\n[00:02.00]kept\n", 1000, 0),
            "[00:00.00]sung\n[00:01.00]kept\n"
        );
        assert_eq!(
            shift_lrc("[00:00.50]gone\n[00:01.00]kept\n", 1000, 0),
            "[00:00.00]kept\n"
        );
    }

    #[test]
    fn a_later_track_moves_lines_later() {
        assert_eq!(shift_lrc("[00:59.99]x\n", -20, 0), "[01:00.01]x\n");
        // A cut 0.1% slow plays the line at 200.2 s where the track does at 200 s.
        assert_eq!(shift_lrc("[03:20.20]x\n", 0, 1000), "[03:20.00]x\n");
    }

    #[test]
    fn a_move_moves_word_times_folds_the_offset_and_rounds() {
        assert_eq!(
            shift_lrc("[00:06.00]<00:05.00>the <00:05.40>lamps go out\n", -1000, 0),
            "[00:07.00]<00:06.00>the <00:06.40>lamps go out\n"
        );
        assert_eq!(
            shift_lrc("[ar:x]\n[offset:+500]\n[00:02.00]on Kessler Row\n", 0, 0),
            "[ar:x]\n[00:01.50]on Kessler Row\n",
            "[offset:+500] sings each line 500 ms sooner"
        );
        assert_eq!(
            shift_lrc("[00:30.005]x\n[00:05.129]y\n", 0, 0),
            "[00:30.01]x\n[00:05.13]y\n"
        );
        assert_eq!(
            shift_lrc("[length:03:00]\n[00:02.00]a\n", 1000, 0),
            "[00:01.00]a\n",
            "a length no more says how long the lines run"
        );
        assert_eq!(
            shift_lrc("[length:03:00]\n[00:02.00]a\n", 0, 0),
            "[length:03:00]\n[00:02.00]a\n"
        );
        assert_eq!(
            shift_lrc("[00:01.00]a <tag> b\n", 0, 0),
            "[00:01.00]a <tag> b\n"
        );
    }

    #[test]
    fn a_timed_line_of_no_words_ends_the_one_before_and_stays() {
        let lrc = "[00:02.00]sung\n[00:08.00]\n[00:09.00]♪\n";
        assert_eq!(clean_lrc(lrc), "[00:02.00]sung\n[00:08.00]\n");
        assert_eq!(
            timing(&clean_lrc(lrc)).map(|t| (t.lines, t.last_ms)),
            Some((1, 2000))
        );
    }

    #[test]
    fn a_stated_length_is_read_in_minutes_and_seconds() {
        assert_eq!(stated_length("[ar:x]\n[length: 03:25.50]\n"), Some(205_500));
        assert_eq!(stated_length("[length:1:02:03]\n"), Some(3_723_000));
        assert_eq!(stated_length("[00:01.00]line\n"), None);
        assert_eq!(stated_length("[length:soon]\n"), None);
        assert_eq!(stated_length("[LENGTH : 2:00]\n"), Some(120_000));
        assert_eq!(stated_length("[length:-3:00]\n"), None);
        assert_eq!(stated_length("[length:0]\n"), None);
        assert_eq!(stated_length("[length:inf]\n"), None);
        assert_eq!(stated_length("[length:1e400]\n"), None);
    }
}
