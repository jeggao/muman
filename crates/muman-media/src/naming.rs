//! Where a song lands in the library: its tags made safe for a path, put
//! through the `[library]` template, each folder and the file name cut
//! to fit.
//!
//! A name is made safe the same way on every system, so a library copies
//! between them: by default anything Windows, FAT or a phone refuses is
//! replaced, `/` by `⧸` and the rest by their full-width lookalikes as
//! yt-dlp does (`：？＊＂＜＞｜⧹`); control characters go, and those that
//! reorder the text around them, which can make a name read as another;
//! a value that would name nothing visible, as dots and spaces alone or
//! a zero-width space before a dot Windows drops, counts as none;
//! leading dots (hidden files) and trailing dots and spaces (which
//! Windows drops) are trimmed with every space among them, an ideographic
//! one too; and a reserved device name such as `CON` or `nul.txt` gets a
//! `_`. Limits are counted in UTF-8 bytes, which no filesystem's own
//! count exceeds, and a name is cut between graphemes, the characters a
//! reader sees, so a cut keeps a flag or an accented letter whole or
//! drops it; a grapheme longer than the room alone is cut between code
//! points, and no joiner or variation selector is left dangling at its
//! end. A cut that shifts, as a newer Unicode joins characters
//! anew, moves the song's file to its new name.
//!
//! A file name keeps room for what follows a song's stem: the audio's
//! extension, or a file beside it, as its `.lrc`, whichever is longest
//! ([`Naming::with_tail`]); a tail longer than `.flac` makes every name
//! that much shorter, so the songs whose names it cuts move.

use std::path::{Path, PathBuf};

use anyhow::Result;
use icu_properties::CodePointSetData;
use icu_properties::props::DefaultIgnorableCodePoint;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

use crate::portable::{LOOKALIKES, reorders, reserved};
use crate::template::{Fields, Template};

/// What Windows refuses in a name, besides control characters.
const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Room kept in a file name for a collision suffix and a tail as long as
/// [`AUDIO_TAIL`].
pub const SUFFIX_ROOM: usize = 24;

/// The longest audio extension, `.flac`, `.opus`, with its dot.
pub const AUDIO_TAIL: usize = ".flac".len();

/// The longest temporary suffix a file is written or moved through.
const TEMPORARY: usize = ".moving".len();

/// The fewest bytes a name is cut to, past the room for its suffix: the
/// least `[library] max_name_bytes` and `max_folder_bytes` may say, with
/// no tail longer than an audio extension ([`min_name_bytes`]).
pub const MIN_NAME_BYTES: usize = 16 + SUFFIX_ROOM;
pub const MIN_FOLDER_BYTES: usize = 16;

/// The fewest bytes `[library] max_name_bytes` may say when the longest
/// tail after a stem takes `tail` bytes.
#[must_use]
pub fn min_name_bytes(tail: usize) -> usize {
    MIN_NAME_BYTES + tail.saturating_sub(AUDIO_TAIL)
}

/// The longest ID a path is told apart by.
const ID_BYTES: usize = 64;

/// How songs are named, from the `[library]` settings.
#[derive(Debug)]
pub struct Naming {
    pub template: Template,
    /// What each character is replaced by, as [`replacements`] orders it.
    pub replace: Vec<(String, String)>,
    pub restrict: Restrict,
    /// `[library] max_name_bytes`.
    pub max_name_bytes: usize,
    /// The bytes after a stem: the longest audio extension or file beside
    /// a song.
    pub tail: usize,
    pub max_folder: usize,
    /// Characters the whole path may take after the library folder, as
    /// [`room`] counts them.
    pub room: Option<usize>,
    pub unknown_artist: String,
    pub unknown_album: String,
    pub untitled: String,
}

/// A song's tags as they come, before they are made safe.
#[derive(Debug, Clone, Default)]
pub struct Tags<'a> {
    pub title: Option<&'a str>,
    pub artists: Vec<&'a str>,
    pub album: Option<&'a str>,
    pub album_artist: Option<&'a str>,
    pub genre: Option<&'a str>,
    pub date: Option<&'a str>,
    /// The date its album was first released, as `ORIGINALDATE` holds it.
    pub original_date: Option<&'a str>,
    pub track: Option<&'a str>,
    pub disc: Option<&'a str>,
    pub composer: Option<&'a str>,
    pub bpm: Option<&'a str>,
    pub key: Option<&'a str>,
    pub id: &'a str,
}

/// How strictly names are made safe, beyond replacing `/` and control
/// characters, which every filesystem refuses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Restrict {
    /// Only what this filesystem refuses.
    None,
    /// What Windows, FAT and most phones refuse too, on every system, so
    /// a library copies anywhere.
    #[default]
    Windows,
    /// As `windows`, and everything transliterated to ASCII.
    Ascii,
}

/// What each character is replaced by in a name made safe as `restrict`
/// says, with `extra`'s replacements over the defaults; longest first, so
/// a replacement of `...` is tried before `.`.
#[must_use]
pub fn replacements(
    restrict: Restrict,
    extra: &std::collections::BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut replace: Vec<(String, String)> = match restrict {
        Restrict::None => vec![("/".into(), "⧸".into()), ("\\".into(), "⧹".into())],
        Restrict::Windows | Restrict::Ascii => LOOKALIKES
            .iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect(),
    };
    for (from, to) in extra {
        replace.retain(|(f, _)| f != from);
        replace.push((from.clone(), to.clone()));
    }
    replace.sort_by_key(|(from, _)| std::cmp::Reverse(from.chars().count()));
    replace
}

/// The characters a path may take after the library folder `library`
/// when the whole may take `max_path`.
#[must_use]
pub fn room(max_path: Option<usize>, library: &Path) -> Option<usize> {
    max_path.map(|max| max.saturating_sub(units(&library.to_string_lossy()) + 1))
}

impl Naming {
    /// This naming, keeping `tail` bytes after each stem rather than an
    /// audio extension's, for files beside a song with longer suffixes.
    #[must_use]
    pub fn with_tail(self, tail: usize) -> Self {
        Self {
            tail: tail.max(AUDIO_TAIL),
            ..self
        }
    }

    /// The room a name keeps past its stem.
    fn suffix_room(&self) -> usize {
        SUFFIX_ROOM + self.tail - AUDIO_TAIL
    }

    /// The bytes a stem's file name may take.
    fn max_name(&self) -> usize {
        self.max_name_bytes.saturating_sub(self.suffix_room())
    }

    /// `stem` told apart by `id`, and by `n` when that is not enough, as
    /// [`suffixed`] tells it apart under this naming's limits.
    #[must_use]
    pub fn suffixed(&self, stem: &Path, id: &str, n: u32) -> PathBuf {
        suffixed(
            stem,
            id,
            n,
            (self.max_name_bytes, self.tail),
            self.name_units(stem.parent()),
        )
    }

    /// The UTF-16 units `max_path` leaves a file name in `folder`, a
    /// path relative to the library folder; `None` without a `max_path`.
    #[must_use]
    pub fn name_units(&self, folder: Option<&Path>) -> Option<usize> {
        let taken = folder
            .filter(|f| !f.as_os_str().is_empty())
            .map_or(0, |f| units(&f.to_string_lossy()) + 1);
        self.room.map(|room| room.saturating_sub(taken))
    }

    /// The song's library path without its extension, relative to the
    /// library folder; never empty, never outside it.
    pub fn stem(&self, tags: &Tags<'_>) -> Result<PathBuf> {
        // Leading dots would hide a file or climb out of a folder.
        let safe = |s: Option<&str>| {
            let v = self.value(s.unwrap_or_default());
            let v = trim_dots(&v);
            // Blank when the name it makes shows nothing, as once a trailing dot
            // is trimmed from a zero-width space.
            if self.component(v, usize::MAX).chars().all(invisible) {
                String::new()
            } else {
                v.to_string()
            }
        };
        let or = |s: String, fallback: &str| {
            if s.is_empty() {
                self.value(fallback)
            } else {
                s
            }
        };
        let track = tags.track.and_then(number);
        let disc = tags.disc.and_then(number);
        let artists: Vec<String> = tags.artists.iter().map(|a| self.value(a)).collect();
        let date = safe(tags.date);
        let year = |d: &str| {
            d.chars()
                .take(4)
                .filter(char::is_ascii_digit)
                .collect::<String>()
        };
        let first = year(&safe(tags.original_date));
        let fields = Fields {
            title: or(safe(tags.title), &self.untitled),
            artist: artists.first().cloned().unwrap_or_default(),
            album: or(safe(tags.album), &self.unknown_album),
            album_artist: or(safe(tags.album_artist), &self.unknown_artist),
            genre: safe(tags.genre),
            year: year(&date),
            original_year: if first.is_empty() { year(&date) } else { first },
            date,
            // Disc 1 keeps the plain number, so an album on one disc never
            // moves when a later edition adds a second.
            disc_track: match (disc, track) {
                (Some(d), Some(n)) if d > 1 => format!("{d}-{n:02} "),
                (_, Some(n)) => format!("{n:02} "),
                (_, None) => String::new(),
            },
            track,
            disc,
            artists,
            composer: safe(tags.composer),
            bpm: safe(tags.bpm),
            key: safe(tags.key),
            id: self.value(tags.id),
        };
        let rendered = self.template.render(&fields)?;
        let mut raw: Vec<&str> = rendered.split('/').collect();
        // The file name is the template's last part even when it renders
        // empty, so a blank title never turns the album folder into it.
        let name = raw.pop().unwrap_or_default();
        let mut parts: Vec<String> = raw
            .iter()
            .map(|p| self.component(p, self.max_folder))
            .filter(|p| !p.is_empty())
            .collect();
        let Some(room) = self.room else {
            return Ok(self.finish(&parts, name, self.max_name()));
        };
        // Folders get an even share of what the path may take when
        // together they would leave the name too little.
        let suffix_room = self.suffix_room();
        let share = (room.saturating_sub(suffix_room) / (parts.len() + 1)).max(8);
        let length = |parts: &[String]| -> usize { parts.iter().map(|p| units(p) + 1).sum() };
        if length(&parts) + share + suffix_room > room {
            parts = parts
                .iter()
                .map(|p| self.component(&cut_units(p, share), self.max_folder))
                .collect();
        }
        let left = room.saturating_sub(length(&parts) + suffix_room).max(8);
        let name = cut_units(&self.component(name, self.max_name()), left);
        Ok(self.finish(&parts, &name, self.max_name()))
    }

    /// The folders and the file name as one path, in NFC, as every path
    /// muman records is: a decomposed tag must name the same file on
    /// every run.
    fn finish(&self, parts: &[String], name: &str, limit: usize) -> PathBuf {
        let name = match self.component(name, limit) {
            n if n.is_empty() => self.component(&self.untitled, limit),
            n => n,
        };
        let path: PathBuf = parts
            .iter()
            .map(String::as_str)
            .chain([name.as_str()])
            .collect();
        crate::relpath::normalized(&path)
    }

    /// Whether `c` cannot stand in a name: control characters and `/`
    /// everywhere; Windows' set under `restrict`, and on Windows itself
    /// always, since there they are separators, streams or wildcards.
    fn forbidden(&self, c: char) -> bool {
        c == '/'
            || c == '\\'
            || c.is_control()
            || ((self.restrict != Restrict::None || cfg!(windows)) && FORBIDDEN.contains(&c))
    }

    /// One tag value made safe for a path.
    pub fn value(&self, text: &str) -> String {
        let mut out: String = text.nfc().collect();
        for (from, to) in &self.replace {
            if !from.is_empty() {
                out = out.replace(from.as_str(), to);
            }
        }
        if self.restrict == Restrict::Ascii {
            out = deunicode::deunicode(&out);
        }
        out.chars()
            .filter(|c| !c.is_control() && !reorders(*c))
            .map(|c| if self.forbidden(c) { '_' } else { c })
            .collect()
    }

    /// A display name as one path component within `limit` bytes, in NFC
    /// before it is cut: composing a name can lengthen it, as a nukta
    /// letter NFC keeps apart grows from 3 bytes to 6. Trailing dots and
    /// spaces and a device name are made safe under `restrict`, and on
    /// Windows itself always: there `CON` names the console, not a file,
    /// and a trailing dot is dropped, so two songs would share a file.
    fn component(&self, name: &str, limit: usize) -> String {
        // What the template itself spells, values aside, is made safe too.
        let safe: String = name
            .nfc()
            .filter(|c| !c.is_control() && !reorders(*c))
            .map(
                |c| match LOOKALIKES.iter().find(|(f, _)| f.starts_with(c)) {
                    Some((_, to)) if self.forbidden(c) => to.chars().next().unwrap_or('_'),
                    _ => c,
                },
            )
            .collect();
        let mut out = cut(trim_dots(&safe), limit);
        if self.restrict != Restrict::None || cfg!(windows) {
            out = out
                .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
                .to_string();
            if reserved(&out) {
                let at = out.find('.').unwrap_or(out.len());
                out.insert(at, '_');
            }
        }
        out.trim_end().to_string()
    }
}

/// `text` without the dots and spaces it starts with and the spaces it
/// ends with, however they alternate, so `. .` leaves nothing rather
/// than a dot.
fn trim_dots(text: &str) -> &str {
    text.trim_start_matches(|c: char| c == '.' || c.is_whitespace())
        .trim_end()
}

/// Whether `c` shows nothing of its own: a space; one Unicode lets a
/// renderer ignore, as a zero-width or formatting character, a variation
/// selector, a soft hyphen or a Hangul filler; or the blank Braille
/// pattern, which Unicode counts as a symbol though it draws no dot.
fn invisible(c: char) -> bool {
    c.is_whitespace()
        || c == '\u{2800}'
        || CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(c)
}

/// The number in a `TRACKNUMBER` or `DISCNUMBER` such as `3` or `3/12`.
fn number(text: &str) -> Option<u32> {
    text.split('/').next()?.trim().parse().ok()
}

/// `stem` told apart by `id`, and by `n` when that is not enough:
/// `Song [id]`, `Song [id 2]`. The name is cut so the whole stays within
/// `max_name` bytes, and `max_units` UTF-16 units where a `max_path`
/// sets them, with room for `tail` bytes after the stem and a temporary
/// suffix.
#[must_use]
pub fn suffixed(
    stem: &Path,
    id: &str,
    n: u32,
    (max_name, tail): (usize, usize),
    max_units: Option<usize>,
) -> PathBuf {
    let number = if n > 1 {
        format!(" {n}")
    } else {
        String::new()
    };
    let after = tail.max(AUDIO_TAIL) + TEMPORARY;
    let room = max_name.saturating_sub(after);
    let units_room = max_units.map(|u| u.saturating_sub(after));
    // The ID cut too, so the name keeps a start of its own; a byte is at
    // most a unit.
    let id_room = room
        .min(units_room.unwrap_or(usize::MAX))
        .saturating_sub(" []".len() + number.len() + KEPT_BYTES)
        .min(ID_BYTES);
    let suffix = format!(" [{}{number}]", cut(id, id_room).trim());
    let name = stem
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut name = cut(&name, room.saturating_sub(suffix.len()));
    if let Some(units_room) = units_room {
        name = cut_units(&name, units_room.saturating_sub(units(&suffix)));
    }
    let named = format!("{}{suffix}", name.trim_end());
    stem.with_file_name(named.trim_start())
}

/// A text's length as Windows counts a path's: in UTF-16 code units, two
/// for a character past the Basic Multilingual Plane, as an emoji.
#[must_use]
pub fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `text` cut within `limit` UTF-16 units between characters as a reader
/// counts them, as [`cut`] cuts within bytes.
fn cut_units(text: &str, limit: usize) -> String {
    let mut taken = 0;
    let mut end = 0;
    for g in text.graphemes(true) {
        taken += units(g);
        if taken > limit {
            break;
        }
        end += g.len();
    }
    if end == 0 {
        let mut taken = 0;
        for c in text.chars() {
            taken += c.len_utf16();
            if taken > limit {
                break;
            }
            end += c.len_utf8();
        }
        return split(&text[..end]);
    }
    text[..end].to_string()
}

/// The least of a name a suffix telling it apart leaves.
const KEPT_BYTES: usize = 8;

/// `text` cut within `limit` bytes between characters as a reader
/// counts them, so no flag is left half drawn and no accent loses its
/// letter; between code points when even the first is longer.
fn cut(text: &str, limit: usize) -> String {
    let fitting = |ends: &mut dyn Iterator<Item = usize>| {
        ends.take_while(|&end| end <= limit).last().unwrap_or(0)
    };
    let mut end = fitting(&mut text.grapheme_indices(true).map(|(i, g)| i + g.len()));
    if end == 0 {
        end = fitting(&mut text.char_indices().map(|(i, c)| i + c.len_utf8()));
        return split(&text[..end]);
    }
    text[..end].to_string()
}

/// The start of a grapheme cut from the rest, without the joiners and
/// variation selectors at its end, which join or style a character the
/// cut took away.
fn split(start: &str) -> String {
    start
        .trim_end_matches(
            |c| matches!(c, '\u{200D}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}'),
        )
        .to_string()
}

/// The most bytes a suffix the song list gives may take.
pub const MOST_SUFFIX: usize = 24;

/// Whether `suffix`, what follows a song's stem in a file beside it, as
/// `.en` in `Song.en.lrc` or `.lrc` itself, names a file beside the song
/// on every system: empty, or a dot and up to [`MOST_SUFFIX`] bytes in
/// all of what any filesystem takes in a name. Why not, if not.
pub fn check_suffix(suffix: &str) -> Result<(), String> {
    if suffix.is_empty() {
        return Ok(());
    }
    if !suffix.starts_with('.') {
        return Err(format!("`{suffix}` must begin with a dot, as `.en`"));
    }
    plain(suffix)
}

/// Whether `name`, a file a folder of songs holds once, as `cover`, is
/// one name of its own on every system, before its extension. Why not, if
/// not.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.starts_with('.') {
        return Err(format!(
            "`{name}` must name a file, not begin with a dot or be empty"
        ));
    }
    if name.ends_with([' ', '.']) || reserved(name) {
        return Err(format!("`{name}` is a name Windows refuses"));
    }
    plain(name)
}

/// `text` of nothing a name refuses on any system, within
/// [`MOST_SUFFIX`] bytes.
fn plain(text: &str) -> Result<(), String> {
    if text.len() > MOST_SUFFIX {
        return Err(format!(
            "`{text}` is over {MOST_SUFFIX} bytes, the most a name keeps for it"
        ));
    }
    if let Some(c) = text
        .chars()
        .find(|c| c.is_control() || "/\\:*?\"<>|".contains(*c))
    {
        return Err(format!("`{text}` holds {c:?}, which a file name cannot"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(parts: &[&str]) -> PathBuf {
        parts.iter().collect()
    }

    #[test]
    fn a_name_is_cut_between_characters_as_a_reader_counts_them() {
        let flags = "🇯🇵🇫🇷";
        assert_eq!(cut(flags, 12), "🇯🇵");
        let accented = "Cafe\u{301} Cafe\u{301}";
        assert_eq!(cut(accented, 5), "Caf");
        let marked = format!("Z{}", "\u{301}".repeat(20));
        assert_eq!(cut(&marked, 5), "Z\u{301}\u{301}");
    }

    #[test]
    fn a_grapheme_cut_apart_ends_on_no_joiner() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} Family";
        assert_eq!(cut(family, 16), "\u{1F468}\u{200D}\u{1F469}");
        assert_eq!(cut_units(family, 6), "\u{1F468}\u{200D}\u{1F469}");
        assert_eq!(cut("\u{2764}\u{FE0F}\u{200D}\u{1F525}", 8), "\u{2764}");
    }

    #[test]
    fn a_name_told_apart_stays_within_max_path() {
        let id = "a-long-source-file-name-made-up-to-tell-two-songs-apart";
        let named = suffixed(
            Path::new("A/Same Title Here"),
            id,
            2,
            (200, AUDIO_TAIL),
            Some(32),
        );
        let name = named.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("Same"), "{name}");
        assert!(units(name) + ".flac.moving".len() <= 32, "{name}");
    }

    #[test]
    fn a_long_id_telling_names_apart_leaves_room_for_the_name() {
        let id = "i".repeat(80);
        for n in [1, 2, 12] {
            let named = suffixed(
                Path::new("A/Lantern Weather"),
                &id,
                n,
                (MIN_NAME_BYTES, AUDIO_TAIL),
                None,
            );
            let name = named.file_name().unwrap().to_str().unwrap();
            assert!(name.starts_with("Lantern"), "{name}");
            assert!(
                name.len() + ".flac.moving".len() <= MIN_NAME_BYTES,
                "{name}"
            );
        }
    }

    #[test]
    fn a_suffix_tells_two_songs_apart() {
        assert_eq!(
            suffixed(&path(&["A", "B", "Song"]), "id", 1, (200, AUDIO_TAIL), None),
            path(&["A", "B", "Song [id]"])
        );
        assert_eq!(
            suffixed(&path(&["A", "B", "Song"]), "id", 2, (200, AUDIO_TAIL), None),
            path(&["A", "B", "Song [id 2]"])
        );
    }

    #[test]
    fn a_suffix_or_a_folder_s_file_is_one_plain_name() {
        for good in ["", ".en", ".lrc", ".zh-Hant"] {
            assert_eq!(check_suffix(good), Ok(()), "{good}");
        }
        for bad in [
            "en",
            "./x",
            ".a/b",
            ".a\\b",
            ".a:b",
            ".\u{7}",
            &".x".repeat(13),
        ] {
            assert!(check_suffix(bad).is_err(), "{bad}");
        }
        assert_eq!(check_name("cover"), Ok(()));
        assert_eq!(check_name("folder"), Ok(()));
        for bad in ["", ".cover", "../cover", "a/b", "CON", "cover.", "cover "] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }
}
