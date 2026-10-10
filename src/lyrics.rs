//! Which language a subtitle is in, and LRC cleaned of what is no
//! lyric and moved to another cut of the recording.
//!
//! A preference is a language code as yt-dlp lists subtitles (`ja`,
//! `en`, `pt-BR`). The stream it names is found by title, since yt-dlp
//! titles each embedded stream with the display name its info JSON
//! records for a person's subtitle; a generated caption has no name, so
//! speech recognition of a song never becomes its lyrics. The ISO 639-2
//! language tag is used only when the info JSON says nothing about
//! subtitles, and it is the weaker match: yt-dlp tags Filipino `fin`,
//! the code for Finnish.

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

/// The base language of a preference: `pt-BR` → `pt`.
fn base(code: &str) -> &str {
    code.split(['-', '_']).next().unwrap_or(code)
}

/// ISO 639-2 codes, bibliographic and terminologic, for the languages
/// subtitles are commonly offered in.
fn iso639_2(code: &str) -> &'static [&'static str] {
    match base(code).to_ascii_lowercase().as_str() {
        "ja" => &["jpn"],
        "en" => &["eng"],
        "zh" => &["zho", "chi"],
        "ko" => &["kor"],
        "fr" => &["fra", "fre"],
        "de" => &["deu", "ger"],
        "es" => &["spa"],
        "pt" => &["por"],
        "it" => &["ita"],
        "ru" => &["rus"],
        "id" => &["ind"],
        "vi" => &["vie"],
        "th" => &["tha"],
        "uk" => &["ukr"],
        "pl" => &["pol"],
        "nl" => &["nld", "dut"],
        "tr" => &["tur"],
        "ar" => &["ara"],
        _ => &[],
    }
}

/// Whether an info JSON code is a preference or one of its regional
/// variants (`en` covers `en-US`, `en-GB`).
fn covers(pref: &str, code: &str) -> bool {
    let (pref, code) = (pref.to_ascii_lowercase(), code.to_ascii_lowercase());
    code == pref || code.starts_with(&format!("{pref}-"))
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

/// The position of the first preference `lang` meets.
#[must_use]
pub fn preference(prefs: &[String], lang: &Language) -> Option<usize> {
    match lang {
        Language::Unstated => (!prefs.is_empty()).then_some(0),
        Language::Codes(codes) => prefs
            .iter()
            .position(|p| codes.iter().any(|c| covers(p, c))),
        Language::Tag(tag) => prefs.iter().position(|p| {
            iso639_2(p).contains(&tag.to_ascii_lowercase().as_str()) || tag.eq_ignore_ascii_case(p)
        }),
    }
}

/// The subtitle stream to turn into lyrics: the first preference with a
/// match wins, a stream named for it before one only tagged with it.
#[must_use]
pub fn pick<'a>(
    prefs: &[String],
    subtitles: &'a [Subtitle],
    info: &VideoInfo,
) -> Option<&'a Subtitle> {
    subtitles
        .iter()
        .filter_map(|s| Some((preference(prefs, &language(s, info)?)?, s)))
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, s)| s)
}

/// Whether a line of lyrics is sung: not blank, not only music symbols,
/// not a bracketed cue as `[Music]`, not a credit as `作詞：…`.
#[must_use]
pub fn is_lyric(text: &str) -> bool {
    const CREDITS: [&str; 18] = [
        "lyrics",
        "lyric",
        "music",
        "composer",
        "composed",
        "arrange",
        "arranged",
        "arrangement",
        "vocal",
        "vocals",
        "illust",
        "illustration",
        "movie",
        "mix",
        "作詞",
        "作曲",
        "編曲",
        "動画",
    ];
    let t = text.trim();
    if t.chars().all(|c| !c.is_alphanumeric()) {
        return false;
    }
    let pairs = [('[', ']'), ('(', ')'), ('（', '）'), ('【', '】')];
    if pairs.iter().any(|(o, c)| {
        t.starts_with(*o)
            && t.ends_with(*c)
            && t[o.len_utf8()..].find(*c) == Some(t.len() - o.len_utf8() - c.len_utf8())
    }) {
        return false;
    }
    let lower = t.to_lowercase();
    !CREDITS.iter().any(|word| {
        lower.strip_prefix(word).is_some_and(|rest| {
            let rest = rest.trim_start();
            rest.starts_with([':', '：', '/', '・']) || rest.starts_with("by ")
        })
    })
}

/// The length an `.lrc` states with its `[length:]` tag, in ms.
#[must_use]
pub fn stated_length(text: &str) -> Option<i64> {
    let line = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("[length:")?.strip_suffix(']'))?;
    let mut seconds = 0.0;
    for part in line.trim().split(':') {
        seconds = seconds * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    #[allow(clippy::cast_possible_truncation)]
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
    let tag = |name: &str| {
        text.lines().find_map(|l| {
            let inner = l.trim().strip_prefix('[')?.strip_suffix(']')?;
            let (k, v) = inner.split_once(':')?;
            (k.trim().eq_ignore_ascii_case(name) && !v.trim().is_empty())
                .then(|| v.trim().to_string())
        })
    };
    Headers {
        title: tag("ti"),
        artist: tag("ar"),
        album: tag("al"),
        length_ms: stated_length(text),
        offset_ms: tag("offset").and_then(|o| o.parse().ok()).unwrap_or(0),
    }
}

/// The words LRC or plain lyrics sing: each timed line's text, and each
/// untimed line but the ID tags, without the lines that are no lyric.
#[must_use]
pub fn sung(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut timed = false;
        while let Some((_, after)) = rest.strip_prefix('[').and_then(lrc_time) {
            timed = true;
            rest = after;
        }
        // Word timings, `<00:12.30>`, inside an enhanced LRC line.
        let words: String = rest
            .split('<')
            .map(|part| part.split_once('>').map_or(part, |(_, w)| w))
            .collect();
        let header = !timed && words.starts_with('[') && words.ends_with(']');
        if !header && !words.trim().is_empty() && is_lyric(&words) {
            out.push_str(words.trim());
            out.push('\n');
        }
    }
    out
}

/// The text of a lyrics file, whatever wrote it: UTF-8 or UTF-16 by its
/// byte-order mark, else UTF-8 when it reads as UTF-8, else the legacy
/// encoding its bytes look most like, as GBK, Shift-JIS or Windows-1252,
/// guessed as a browser guesses a page that names none. CRLF line
/// endings become LF.
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
    text.replace("\r\n", "\n")
}

/// UTF-16 with no byte-order mark, told by its zero bytes: text mostly in
/// Latin letters has one in every pair, on the side its byte order says.
fn utf16_unmarked(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    if encoding_rs::Encoding::for_bom(bytes).is_some() || bytes.len() < 4 {
        return None;
    }
    let (pairs, _) = bytes.as_chunks::<2>();
    let total = pairs.len();
    let (high, low) = pairs.iter().fold((0, 0), |(h, l), p| {
        (h + usize::from(p[1] == 0), l + usize::from(p[0] == 0))
    });
    if high * 2 > total && low * 10 < total {
        Some(encoding_rs::UTF_16LE)
    } else if low * 2 > total && high * 10 < total {
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

/// LRC without its timed lines that are no lyric; untimed lines, the
/// `[ar:…]` headers among them, stay.
#[must_use]
pub fn clean_lrc(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut rest = line;
        let mut timed = false;
        while let Some((_, after)) = rest.strip_prefix('[').and_then(lrc_time) {
            timed = true;
            rest = after;
        }
        if timed && !is_lyric(rest) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The timed lines of LRC: how many, and the first and last time.
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
        while let Some((ms, after)) = rest.strip_prefix('[').and_then(lrc_time) {
            times.push(ms);
            rest = after;
        }
    }
    Some(Timing {
        lines: times.len(),
        first_ms: *times.iter().min()?,
        last_ms: *times.iter().max()?,
    })
}

/// LRC timed for another cut of the recording: every `[mm:ss.xx]` tag
/// moved `offset_ms` earlier, then shortened by `stretch_ppm` parts per
/// million for a cut that plays the recording that much longer. A line
/// that would start before the track does is dropped; untimed lines are
/// kept.
#[must_use]
pub fn shift_lrc(text: &str, offset_ms: i64, stretch_ppm: i64) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut rest = line;
        let mut times = Vec::new();
        while let Some((ms, after)) = rest.strip_prefix('[').and_then(lrc_time) {
            times.push(unstretch(ms - offset_ms, stretch_ppm));
            rest = after;
        }
        if !times.is_empty() {
            times.retain(|ms| *ms >= 0);
            if times.is_empty() {
                continue;
            }
            for ms in times {
                let _ = write!(
                    out,
                    "[{:02}:{:02}.{:02}]",
                    ms / 60_000,
                    ms / 1000 % 60,
                    ms / 10 % 100
                );
            }
        }
        out.push_str(rest);
        out.push('\n');
    }
    out
}

/// `mm:ss.xx]` or `mm:ss.xxx]` at the start of `s`, in milliseconds,
/// and what follows the bracket.
/// `ms` played `stretch_ppm` parts per million longer, brought back.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn unstretch(ms: i64, stretch_ppm: i64) -> i64 {
    if stretch_ppm == 0 {
        return ms;
    }
    (ms as f64 / (1.0 + units::ratio_of_ppm(stretch_ppm as f64))).round() as i64
}

fn lrc_time(s: &str) -> Option<(i64, &str)> {
    let (tag, after) = s.split_once(']')?;
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
    let ms = min.parse::<i64>().ok()? * 60_000 + whole.parse::<i64>().ok()? * 1000 + frac_ms;
    Some((ms, after))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let noise: Vec<u8> = (0..4000_u32)
            .map(|i| i.wrapping_mul(2_654_435_761).to_le_bytes()[2])
            .collect();
        assert!(!is_text(&decode(&noise)));
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

    fn prefs(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| (*s).to_string()).collect()
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
            Some(1)
        );
        assert_eq!(
            preference(&prefs(&["ja"]), &Language::Unstated),
            Some(0),
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
        ] {
            assert!(!is_lyric(no), "{no}");
        }
        for yes in [
            "Music fills the harbor",
            "(I) miss the tide (so)",
            "灯台の夢",
            "Lyrics of a song I wrote",
        ] {
            assert!(is_lyric(yes), "{yes}");
        }
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
    fn a_line_before_the_track_starts_is_dropped() {
        assert_eq!(
            shift_lrc("[00:00.50]gone\n[00:02.00]kept\n", 1000, 0),
            "[00:01.00]kept\n"
        );
    }

    #[test]
    fn a_later_track_moves_lines_later() {
        assert_eq!(shift_lrc("[00:59.99]x\n", -20, 0), "[01:00.01]x\n");
        // A cut 0.1% slow plays the line at 200.2 s where the track does at 200 s.
        assert_eq!(shift_lrc("[03:20.20]x\n", 0, 1000), "[03:20.00]x\n");
    }

    #[test]
    fn a_stated_length_is_read_in_minutes_and_seconds() {
        assert_eq!(stated_length("[ar:x]\n[length: 03:25.50]\n"), Some(205_500));
        assert_eq!(stated_length("[length:1:02:03]\n"), Some(3_723_000));
        assert_eq!(stated_length("[00:01.00]line\n"), None);
        assert_eq!(stated_length("[length:soon]\n"), None);
    }
}
