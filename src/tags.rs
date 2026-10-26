//! The tags each source offers, and how much each offer is worth: a
//! dedicated metadata field, as a file's own tags, a release's info or a
//! MusicBrainz record holds, beats a value read off a video's title or
//! channel, and a value decorated with `(Official Video)` or `【MV】`
//! is worth less than a plain one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::clean;
use crate::info::VideoInfo;

/// Names how a source's tags, and the lyrics among a file's, are read.
/// Tags read by another are read again, without measuring the source
/// again, so changing what a file or an info JSON offers means changing
/// this.
pub const METHOD: &str = "tags/10";

/// What a field describes, which decides where a song's value comes
/// from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The recording: each field from whichever source offers it best.
    Recording,
    /// What identifies the recording: each field from whichever source
    /// offers it best of those that title the recording as the song is
    /// titled, so a record of another recording lends none.
    RecordingId,
    /// The release the song is on: every field from the one source whose
    /// album ranks best, so an album never splits and its IDs never mix.
    Release,
    /// How loud the audio is: from the source the song's audio is, as
    /// another's loudness is not this audio's.
    Loudness,
    /// How loud the audio's album is: from the source the song's audio
    /// is, when its release fields come from it too.
    AlbumLoudness,
}

/// A tag muman knows: how sources offer it, how a song list names it,
/// and where a song's value comes from. Adding a field is a variant and
/// its row in [`TABLE`]; reading it from a file's tags, ranking it and
/// writing it follow. Picard's names are used where they differ,
/// so a library tagged by muman and by Picard agrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Track,
    Disc,
    Date,
    Genre,
    Isrc,
    TrackTotal,
    DiscTotal,
    ReleaseCountry,
    /// The recording's MBID, as Picard names it.
    #[serde(rename = "musicbrainz_trackid")]
    MusicBrainzTrackId,
    #[serde(rename = "musicbrainz_releasetrackid")]
    MusicBrainzReleaseTrackId,
    #[serde(rename = "musicbrainz_albumid")]
    MusicBrainzAlbumId,
    #[serde(rename = "musicbrainz_releasegroupid")]
    MusicBrainzReleaseGroupId,
    #[serde(rename = "musicbrainz_artistid")]
    MusicBrainzArtistId,
    #[serde(rename = "musicbrainz_albumartistid")]
    MusicBrainzAlbumArtistId,
    /// ReplayGain's, in dB, against 89 dB SPL.
    #[serde(rename = "replaygain_track_gain")]
    TrackGain,
    /// The highest sample, full scale 1.
    #[serde(rename = "replaygain_track_peak")]
    TrackPeak,
    #[serde(rename = "replaygain_album_gain")]
    AlbumGain,
    #[serde(rename = "replaygain_album_peak")]
    AlbumPeak,
    Bpm,
    InitialKey,
    Compilation,
    Composer,
    Lyricist,
    Conductor,
    Remixer,
    Label,
    CatalogNumber,
    Barcode,
    Asin,
    Artists,
    ArtistSort,
    AlbumArtistSort,
    TitleSort,
    AlbumSort,
    ComposerSort,
    OriginalDate,
    ReleaseType,
    ReleaseStatus,
    Media,
    Script,
    DiscSubtitle,
    Mood,
    Grouping,
    Work,
    Language,
    Copyright,
    Comment,
    #[serde(rename = "musicbrainz_workid")]
    MusicBrainzWorkId,
    #[serde(rename = "musicbrainz_discid")]
    MusicBrainzDiscId,
}

impl Field {
    /// Every field, in the order written.
    pub const ALL: [Self; 53] = [
        Self::Title,
        Self::Artist,
        Self::Album,
        Self::AlbumArtist,
        Self::Track,
        Self::Disc,
        Self::Date,
        Self::Genre,
        Self::Isrc,
        Self::TrackTotal,
        Self::DiscTotal,
        Self::ReleaseCountry,
        Self::MusicBrainzTrackId,
        Self::MusicBrainzReleaseTrackId,
        Self::MusicBrainzAlbumId,
        Self::MusicBrainzReleaseGroupId,
        Self::MusicBrainzArtistId,
        Self::MusicBrainzAlbumArtistId,
        Self::TrackGain,
        Self::TrackPeak,
        Self::AlbumGain,
        Self::AlbumPeak,
        Self::Bpm,
        Self::InitialKey,
        Self::Compilation,
        Self::Composer,
        Self::Lyricist,
        Self::Conductor,
        Self::Remixer,
        Self::Label,
        Self::CatalogNumber,
        Self::Barcode,
        Self::Asin,
        Self::Artists,
        Self::ArtistSort,
        Self::AlbumArtistSort,
        Self::TitleSort,
        Self::AlbumSort,
        Self::ComposerSort,
        Self::OriginalDate,
        Self::ReleaseType,
        Self::ReleaseStatus,
        Self::Media,
        Self::Script,
        Self::DiscSubtitle,
        Self::Mood,
        Self::Grouping,
        Self::Work,
        Self::Language,
        Self::Copyright,
        Self::Comment,
        Self::MusicBrainzWorkId,
        Self::MusicBrainzDiscId,
    ];

    /// Its row of [`TABLE`].
    #[must_use]
    pub fn spec(self) -> &'static Spec {
        &TABLE[self as usize]
    }

    /// The Vorbis comment field it is written as.
    #[must_use]
    pub fn vorbis(self) -> &'static str {
        self.spec().vorbis
    }

    /// Its other names, lower-cased: the common spellings a song list may
    /// use, and the names ffprobe reports for MP3 and MP4 files.
    fn aliases(self) -> &'static [&'static str] {
        self.spec().aliases
    }

    /// Whether it is one of ReplayGain's gains and peaks.
    #[must_use]
    pub fn is_loudness(self) -> bool {
        matches!(self.scope(), Scope::Loudness | Scope::AlbumLoudness)
    }

    #[must_use]
    pub fn scope(self) -> Scope {
        self.spec().scope
    }

    /// Whether its values are identifiers, several to a value in a file
    /// that joins them, and never cleaned.
    #[must_use]
    pub fn is_id(self) -> bool {
        self.spec().kind == Kind::Mbid
    }

    /// The fields of one scope, in the order written.
    pub fn of(scope: Scope) -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(move |f| f.scope() == scope)
    }

    /// The field a song list's or a file's tag name sets: its Vorbis name
    /// or any alias, case ignored.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        let lower = name.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|f| {
            f.vorbis().eq_ignore_ascii_case(&lower)
                || f.aliases().contains(&lower.as_str())
                || f.spec()
                    .also_vorbis
                    .iter()
                    .any(|v| v.eq_ignore_ascii_case(&lower))
        })
    }
}

/// What a field's values are: how each is read, from a source or a song
/// list, and how it is written. A value is held as the text muman writes
/// it, whatever spelling it came in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Any text but control characters, which cut a C string short or
    /// move a terminal's cursor; a line break or a tab stays.
    Text,
    /// A count from 1, as a track's total.
    Count,
    /// A place from 1, and the count it is of after a `/`: `3`, `3/12`.
    Place,
    /// A year, a month or a day of a real calendar: `YYYY`, `YYYY-MM` or
    /// `YYYY-MM-DD`; read from `YYYYMMDD`, with `/` or `.` between its
    /// parts, and with a time after it, which is cut.
    Date,
    /// An ISRC, ISO 3901's 12 letters and digits: a country, a
    /// registrant, a year and a number. The dashes it is shown with are
    /// dropped and its letters upper-cased.
    Isrc,
    /// A MusicBrainz ID: a UUID, lower-cased.
    Mbid,
    /// A ReplayGain gain, measured by muman: `-6.12 dB`.
    Gain,
    /// A number of up to two decimals, as a tempo in beats a minute.
    Decimal,
    /// Yes or no, written `1` or `0`.
    Flag,
    /// A musical key, as `Abm`, `F#`, or a DJ tool's `8A`.
    MusicalKey,
    /// A product's barcode, its digits alone, leading zeros kept: a UPC's
    /// 12, an EAN's 13 or 8, a GTIN's 14.
    Barcode,
    /// A language by its ISO 639-2 code, as `jpn`; a two-letter one, as
    /// `ja`, read as it.
    Language,
    /// A ReplayGain peak, measured by muman: full scale 1, six decimals.
    Peak,
}

impl Kind {
    /// `value` as muman writes a value of this kind, or why it is none.
    pub fn read(self, value: &str) -> Result<String, &'static str> {
        let v = value.trim();
        match self {
            Self::Text => {
                if v.chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
                {
                    Err("text with a control character in it")
                } else {
                    Ok(v.to_string())
                }
            }
            Self::Count => count(v)
                .map(|n| n.to_string())
                .ok_or("no count: give one as 12"),
            Self::Place => {
                let read = match v.split_once('/') {
                    Some((n, of)) => count(n).zip(count(of)).map(|(n, of)| format!("{n}/{of}")),
                    None => count(v).map(|n| n.to_string()),
                };
                read.ok_or("no number: give one as 3, or 3/12")
            }
            Self::Date => date(v).ok_or("no date: give YYYY-MM-DD, YYYY-MM or a year"),
            Self::Isrc => isrc(v).ok_or("no ISRC: give its 12 letters and digits"),
            Self::Mbid => mbid(v).ok_or("no MusicBrainz ID"),
            Self::Gain => gain(v).ok_or("no gain: give one as -6.12 dB"),
            Self::Peak => peak(v).ok_or("no peak: give one as 0.988312"),
            Self::Decimal => decimal(v).ok_or("no tempo: give one from 1 to 9999, as 127.5"),
            Self::Flag => flag(v).ok_or("no yes or no: give 1 or 0"),
            Self::MusicalKey => {
                (!v.is_empty() && v.chars().count() <= 16 && !v.chars().any(char::is_control))
                    .then(|| v.to_string())
                    .ok_or("no key: give one as Abm")
            }
            Self::Barcode => barcode(v).ok_or("no barcode: give its 8 to 14 digits"),
            Self::Language => language(v).ok_or("no language: give its ISO 639-2 code, as jpn"),
        }
    }
}

/// A count from 1, from digits alone.
fn count(text: &str) -> Option<u32> {
    let t = text.trim();
    (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
        .then(|| t.parse().ok())
        .flatten()
        .filter(|&n| n > 0)
}

/// A date of a real calendar as Vorbis comments write it, from any common
/// spelling: see [`Kind::Date`].
fn date(text: &str) -> Option<String> {
    let t = text.trim();
    let t = match t.split_once(['T', ' ']) {
        Some((day, _)) => day,
        None => t,
    };
    let parts: Vec<&str> = if t.len() == 8 && t.bytes().all(|b| b.is_ascii_digit()) {
        vec![&t[..4], &t[4..6], &t[6..]]
    } else {
        t.split(['-', '/', '.']).collect()
    };
    let digits = |p: &str| !p.is_empty() && p.len() <= 2 && p.bytes().all(|b| b.is_ascii_digit());
    let within = |p: &str, most: u32| p.parse::<u32>().is_ok_and(|v| (1..=most).contains(&v));
    let year = parts
        .first()
        .filter(|y| y.len() == 4 && y.bytes().all(|b| b.is_ascii_digit()) && **y != "0000")?;
    match parts[1..] {
        [] => Some((*year).to_string()),
        [m] if digits(m) && within(m, 12) => Some(format!("{year}-{m:0>2}")),
        [m, d] if digits(m) && digits(d) && within(m, 12) => {
            let leap = year
                .parse::<u32>()
                .is_ok_and(|y| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0));
            let days = match m.parse::<u32>().ok()? {
                2 if leap => 29,
                2 => 28,
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            within(d, days).then(|| format!("{year}-{m:0>2}-{d:0>2}"))
        }
        _ => None,
    }
}

fn isrc(text: &str) -> Option<String> {
    let code: String = text
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let b = code.as_bytes();
    let fine = b.len() == 12
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..5].iter().all(u8::is_ascii_alphanumeric)
        && b[5..].iter().all(u8::is_ascii_digit);
    fine.then_some(code)
}

/// A tempo to two decimals, whole from 1 to 9999 as `TBPM` and `tmpo`
/// hold it.
fn decimal(text: &str) -> Option<String> {
    let n: f64 = text.trim().parse().ok()?;
    let n = (n * 100.0).round() / 100.0;
    (n.is_finite() && (1.0..10_000.0).contains(&n.round())).then(|| format!("{n}"))
}

fn flag(text: &str) -> Option<String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some("1".into()),
        "0" | "false" | "no" => Some("0".into()),
        _ => None,
    }
}

fn barcode(text: &str) -> Option<String> {
    let digits: String = text.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    (matches!(digits.len(), 8 | 12 | 13 | 14) && digits.bytes().all(|b| b.is_ascii_digit()))
        .then_some(digits)
}

fn language(text: &str) -> Option<String> {
    let code = text.trim().to_ascii_lowercase();
    match code.len() {
        3 if code.bytes().all(|b| b.is_ascii_lowercase()) => Some(code),
        2 => crate::lyrics::iso639_2_of(&code).map(str::to_string),
        _ => None,
    }
}

/// Whether `key` is written as `ID3v2`'s `TKEY` holds one: a note `A` to
/// `G`, then `b` or `#`, then `m` for minor; or `o` for no key.
#[must_use]
pub fn id3_key(key: &str) -> bool {
    if key == "o" {
        return true;
    }
    let Some((note, rest)) = key.as_bytes().split_first() else {
        return false;
    };
    let rest = rest
        .strip_prefix(b"b")
        .or_else(|| rest.strip_prefix(b"#"))
        .unwrap_or(rest);
    (b'A'..=b'G').contains(note) && (rest.is_empty() || rest == b"m")
}

fn mbid(text: &str) -> Option<String> {
    let id = text.trim().to_ascii_lowercase();
    let fine = id.len() == 36
        && id.char_indices().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        });
    fine.then_some(id)
}

/// Where a field is written in an `ID3v2` tag, as Picard writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Id3 {
    /// A text frame of its own, as `TIT2`; several values null-separated.
    Text(&'static str),
    /// A time frame, as `TDRC`.
    Time(&'static str),
    /// A user text frame, `TXXX`, of this description.
    User(&'static str),
    /// The unique file identifier MusicBrainz owns.
    Ufid,
    /// A place's frame, `TRCK` or `TPOS`, holding `3/12`.
    Place(&'static str),
    /// Within another field's frame: a total in its place's.
    Within,
    /// A text frame holding a whole number, as `TBPM`.
    Whole(&'static str),
    /// `TKEY`, in the notation it holds; a key written otherwise is a user
    /// text frame of its Vorbis name.
    Key,
    /// A comment frame, `COMM`, of no description.
    Comment,
}

/// Where a field is written in MP4's iTunes atoms, as Picard writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mp4 {
    /// An atom of its own holding text, as `©nam`.
    Text(&'static str),
    /// A pair of numbers, `trkn` or `disk`: the place and its total.
    Place(&'static str),
    /// iTunes' freeform atom of this name.
    Free(&'static str),
    /// Within another field's atom: a total in its place's.
    Within,
    /// An atom holding a whole number, as `tmpo`.
    Whole(&'static str),
    /// An atom holding yes or no, as `cpil`.
    Flag(&'static str),
}

/// The `MusicBrainz` owner of the unique file identifier it writes.
pub const UFID_OWNER: &str = "http://musicbrainz.org";

/// One row of [`TABLE`]: all muman knows of a field.
#[derive(Debug)]
pub struct Spec {
    pub field: Field,
    /// The Vorbis comment it is written as, Picard's name for it.
    pub vorbis: &'static str,
    /// Other Vorbis comments it is written as too, where readers differ
    /// on its name: a key is `KEY` to Picard, `INITIALKEY` to DJ tools.
    pub also_vorbis: &'static [&'static str],
    aliases: &'static [&'static str],
    pub kind: Kind,
    pub scope: Scope,
    pub id3: Id3,
    pub mp4: Mp4,
    /// Its APEv2 key; `None` for a total, written in its place's.
    pub ape: Option<&'static str>,
}

/// Every field muman knows, in [`Field::ALL`]'s order.
pub static TABLE: [Spec; 53] = {
    use Field as F;
    use Kind as K;
    use Scope as S;
    const fn mbid(field: Field, vorbis: &'static str, picard: &'static str, scope: Scope) -> Spec {
        Spec {
            field,
            vorbis,
            also_vorbis: &[],
            aliases: &[],
            kind: Kind::Mbid,
            scope,
            id3: Id3::User(picard),
            mp4: Mp4::Free(picard),
            ape: Some(vorbis),
        }
    }
    const fn loudness(field: Field, vorbis: &'static str, kind: Kind, scope: Scope) -> Spec {
        Spec {
            field,
            vorbis,
            also_vorbis: &[],
            aliases: &[],
            kind,
            scope,
            id3: Id3::User(vorbis),
            mp4: Mp4::Free(vorbis),
            ape: Some(vorbis),
        }
    }
    [
        Spec {
            field: F::Title,
            vorbis: "TITLE",
            also_vorbis: &[],
            aliases: &[],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TIT2"),
            mp4: Mp4::Text("©nam"),
            ape: Some("Title"),
        },
        Spec {
            field: F::Artist,
            vorbis: "ARTIST",
            also_vorbis: &[],
            aliases: &[],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TPE1"),
            mp4: Mp4::Text("©ART"),
            ape: Some("Artist"),
        },
        Spec {
            field: F::Album,
            vorbis: "ALBUM",
            also_vorbis: &[],
            aliases: &[],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::Text("TALB"),
            mp4: Mp4::Text("©alb"),
            ape: Some("Album"),
        },
        Spec {
            field: F::AlbumArtist,
            vorbis: "ALBUMARTIST",
            also_vorbis: &[],
            aliases: &["album_artist", "album artist"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::Text("TPE2"),
            mp4: Mp4::Text("aART"),
            ape: Some("Album Artist"),
        },
        Spec {
            field: F::Track,
            vorbis: "TRACKNUMBER",
            also_vorbis: &[],
            aliases: &["track", "track_number"],
            kind: K::Place,
            scope: S::Release,
            id3: Id3::Place("TRCK"),
            mp4: Mp4::Place("trkn"),
            ape: Some("Track"),
        },
        Spec {
            field: F::Disc,
            vorbis: "DISCNUMBER",
            also_vorbis: &[],
            aliases: &["disc", "disc_number"],
            kind: K::Place,
            scope: S::Release,
            id3: Id3::Place("TPOS"),
            mp4: Mp4::Place("disk"),
            ape: Some("Disc"),
        },
        Spec {
            field: F::Date,
            vorbis: "DATE",
            also_vorbis: &[],
            aliases: &["year"],
            kind: K::Date,
            scope: S::Release,
            id3: Id3::Time("TDRC"),
            mp4: Mp4::Text("©day"),
            ape: Some("Year"),
        },
        Spec {
            field: F::Genre,
            vorbis: "GENRE",
            also_vorbis: &[],
            aliases: &[],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TCON"),
            mp4: Mp4::Text("©gen"),
            ape: Some("Genre"),
        },
        Spec {
            field: F::Isrc,
            vorbis: "ISRC",
            also_vorbis: &[],
            aliases: &["tsrc"],
            kind: K::Isrc,
            scope: S::RecordingId,
            id3: Id3::Text("TSRC"),
            mp4: Mp4::Free("ISRC"),
            ape: Some("ISRC"),
        },
        Spec {
            field: F::TrackTotal,
            vorbis: "TRACKTOTAL",
            also_vorbis: &[],
            aliases: &["track_total", "totaltracks"],
            kind: K::Count,
            scope: S::Release,
            id3: Id3::Within,
            mp4: Mp4::Within,
            ape: None,
        },
        Spec {
            field: F::DiscTotal,
            vorbis: "DISCTOTAL",
            also_vorbis: &[],
            aliases: &["disc_total", "totaldiscs"],
            kind: K::Count,
            scope: S::Release,
            id3: Id3::Within,
            mp4: Mp4::Within,
            ape: None,
        },
        Spec {
            field: F::ReleaseCountry,
            vorbis: "RELEASECOUNTRY",
            also_vorbis: &[],
            aliases: &["release_country", "musicbrainz album release country"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::User("MusicBrainz Album Release Country"),
            mp4: Mp4::Free("MusicBrainz Album Release Country"),
            ape: Some("RELEASECOUNTRY"),
        },
        Spec {
            field: F::MusicBrainzTrackId,
            vorbis: "MUSICBRAINZ_TRACKID",
            also_vorbis: &[],
            aliases: &["musicbrainz track id"],
            kind: K::Mbid,
            scope: S::RecordingId,
            id3: Id3::Ufid,
            mp4: Mp4::Free("MusicBrainz Track Id"),
            ape: Some("MUSICBRAINZ_TRACKID"),
        },
        Spec {
            aliases: &["musicbrainz release track id"],
            ..mbid(
                F::MusicBrainzReleaseTrackId,
                "MUSICBRAINZ_RELEASETRACKID",
                "MusicBrainz Release Track Id",
                S::Release,
            )
        },
        Spec {
            aliases: &["musicbrainz album id"],
            ..mbid(
                F::MusicBrainzAlbumId,
                "MUSICBRAINZ_ALBUMID",
                "MusicBrainz Album Id",
                S::Release,
            )
        },
        Spec {
            aliases: &["musicbrainz release group id"],
            ..mbid(
                F::MusicBrainzReleaseGroupId,
                "MUSICBRAINZ_RELEASEGROUPID",
                "MusicBrainz Release Group Id",
                S::Release,
            )
        },
        Spec {
            aliases: &["musicbrainz artist id"],
            ..mbid(
                F::MusicBrainzArtistId,
                "MUSICBRAINZ_ARTISTID",
                "MusicBrainz Artist Id",
                S::RecordingId,
            )
        },
        Spec {
            aliases: &["musicbrainz album artist id"],
            ..mbid(
                F::MusicBrainzAlbumArtistId,
                "MUSICBRAINZ_ALBUMARTISTID",
                "MusicBrainz Album Artist Id",
                S::Release,
            )
        },
        loudness(F::TrackGain, "REPLAYGAIN_TRACK_GAIN", K::Gain, S::Loudness),
        loudness(F::TrackPeak, "REPLAYGAIN_TRACK_PEAK", K::Peak, S::Loudness),
        loudness(
            F::AlbumGain,
            "REPLAYGAIN_ALBUM_GAIN",
            K::Gain,
            S::AlbumLoudness,
        ),
        loudness(
            F::AlbumPeak,
            "REPLAYGAIN_ALBUM_PEAK",
            K::Peak,
            S::AlbumLoudness,
        ),
        Spec {
            field: F::Bpm,
            vorbis: "BPM",
            also_vorbis: &[],
            aliases: &["tbpm", "tmpo", "tempo"],
            kind: K::Decimal,
            scope: S::Recording,
            id3: Id3::Whole("TBPM"),
            mp4: Mp4::Whole("tmpo"),
            ape: Some("BPM"),
        },
        Spec {
            field: F::InitialKey,
            vorbis: "KEY",
            also_vorbis: &["INITIALKEY"],
            aliases: &["initialkey", "initial_key", "tkey"],
            kind: K::MusicalKey,
            scope: S::Recording,
            id3: Id3::Key,
            mp4: Mp4::Free("initialkey"),
            ape: Some("KEY"),
        },
        Spec {
            field: F::Compilation,
            vorbis: "COMPILATION",
            also_vorbis: &[],
            aliases: &["tcmp", "cpil", "comp"],
            kind: K::Flag,
            scope: S::Release,
            id3: Id3::Text("TCMP"),
            mp4: Mp4::Flag("cpil"),
            ape: Some("Compilation"),
        },
        Spec {
            field: F::Composer,
            vorbis: "COMPOSER",
            also_vorbis: &[],
            aliases: &["tcom"],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TCOM"),
            mp4: Mp4::Text("©wrt"),
            ape: Some("Composer"),
        },
        Spec {
            field: F::Lyricist,
            vorbis: "LYRICIST",
            also_vorbis: &[],
            aliases: &["text"],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TEXT"),
            mp4: Mp4::Free("LYRICIST"),
            ape: Some("Lyricist"),
        },
        Spec {
            field: F::Conductor,
            vorbis: "CONDUCTOR",
            also_vorbis: &[],
            aliases: &["tpe3"],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TPE3"),
            mp4: Mp4::Free("CONDUCTOR"),
            ape: Some("Conductor"),
        },
        Spec {
            field: F::Remixer,
            vorbis: "REMIXER",
            also_vorbis: &[],
            aliases: &["tpe4", "mixartist"],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TPE4"),
            mp4: Mp4::Free("REMIXER"),
            ape: Some("MixArtist"),
        },
        Spec {
            field: F::Label,
            vorbis: "LABEL",
            also_vorbis: &[],
            aliases: &["publisher", "organization", "tpub"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::Text("TPUB"),
            mp4: Mp4::Free("LABEL"),
            ape: Some("Label"),
        },
        Spec {
            field: F::CatalogNumber,
            vorbis: "CATALOGNUMBER",
            also_vorbis: &[],
            aliases: &["catalog_number", "catalognum"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::User("CATALOGNUMBER"),
            mp4: Mp4::Free("CATALOGNUMBER"),
            ape: Some("CatalogNumber"),
        },
        Spec {
            field: F::Barcode,
            vorbis: "BARCODE",
            also_vorbis: &[],
            aliases: &["upc", "ean"],
            kind: K::Barcode,
            scope: S::Release,
            id3: Id3::User("BARCODE"),
            mp4: Mp4::Free("BARCODE"),
            ape: Some("Barcode"),
        },
        Spec {
            field: F::Asin,
            vorbis: "ASIN",
            also_vorbis: &[],
            aliases: &[],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::User("ASIN"),
            mp4: Mp4::Free("ASIN"),
            ape: Some("ASIN"),
        },
        Spec {
            field: F::Artists,
            vorbis: "ARTISTS",
            also_vorbis: &[],
            aliases: &[],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::User("ARTISTS"),
            mp4: Mp4::Free("ARTISTS"),
            ape: Some("Artists"),
        },
        Spec {
            field: F::ArtistSort,
            vorbis: "ARTISTSORT",
            also_vorbis: &[],
            aliases: &["artist_sort", "tsop", "soar"],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TSOP"),
            mp4: Mp4::Text("soar"),
            ape: Some("ArtistSort"),
        },
        Spec {
            field: F::AlbumArtistSort,
            vorbis: "ALBUMARTISTSORT",
            also_vorbis: &[],
            aliases: &["album_artist_sort", "tso2", "soaa"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::Text("TSO2"),
            mp4: Mp4::Text("soaa"),
            ape: Some("AlbumArtistSort"),
        },
        Spec {
            field: F::TitleSort,
            vorbis: "TITLESORT",
            also_vorbis: &[],
            aliases: &["title_sort", "tsot", "sonm"],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TSOT"),
            mp4: Mp4::Text("sonm"),
            ape: Some("TitleSort"),
        },
        Spec {
            field: F::AlbumSort,
            vorbis: "ALBUMSORT",
            also_vorbis: &[],
            aliases: &["album_sort", "tsoa", "soal"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::Text("TSOA"),
            mp4: Mp4::Text("soal"),
            ape: Some("AlbumSort"),
        },
        Spec {
            field: F::ComposerSort,
            vorbis: "COMPOSERSORT",
            also_vorbis: &[],
            aliases: &["composer_sort", "tsoc", "soco"],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TSOC"),
            mp4: Mp4::Text("soco"),
            ape: Some("ComposerSort"),
        },
        Spec {
            field: F::OriginalDate,
            vorbis: "ORIGINALDATE",
            also_vorbis: &[],
            aliases: &["original_date", "tdor"],
            kind: K::Date,
            scope: S::Release,
            id3: Id3::Time("TDOR"),
            mp4: Mp4::Free("ORIGINALDATE"),
            ape: Some("ORIGINALDATE"),
        },
        Spec {
            field: F::ReleaseType,
            vorbis: "RELEASETYPE",
            also_vorbis: &[],
            aliases: &["release_type", "musicbrainz album type"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::User("MusicBrainz Album Type"),
            mp4: Mp4::Free("MusicBrainz Album Type"),
            ape: Some("MUSICBRAINZ_ALBUMTYPE"),
        },
        Spec {
            field: F::ReleaseStatus,
            vorbis: "RELEASESTATUS",
            also_vorbis: &[],
            aliases: &["release_status", "musicbrainz album status"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::User("MusicBrainz Album Status"),
            mp4: Mp4::Free("MusicBrainz Album Status"),
            ape: Some("MUSICBRAINZ_ALBUMSTATUS"),
        },
        Spec {
            field: F::Media,
            vorbis: "MEDIA",
            also_vorbis: &[],
            aliases: &["tmed"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::Text("TMED"),
            mp4: Mp4::Free("MEDIA"),
            ape: Some("Media"),
        },
        Spec {
            field: F::Script,
            vorbis: "SCRIPT",
            also_vorbis: &[],
            aliases: &[],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::User("SCRIPT"),
            mp4: Mp4::Free("SCRIPT"),
            ape: Some("Script"),
        },
        Spec {
            field: F::DiscSubtitle,
            vorbis: "DISCSUBTITLE",
            also_vorbis: &[],
            aliases: &["disc_subtitle", "tsst"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::Text("TSST"),
            mp4: Mp4::Free("DISCSUBTITLE"),
            ape: Some("DiscSubtitle"),
        },
        Spec {
            field: F::Mood,
            vorbis: "MOOD",
            also_vorbis: &[],
            aliases: &["tmoo"],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TMOO"),
            mp4: Mp4::Free("MOOD"),
            ape: Some("Mood"),
        },
        Spec {
            field: F::Grouping,
            vorbis: "GROUPING",
            also_vorbis: &[],
            aliases: &["tit1", "grp1", "©grp"],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Text("TIT1"),
            mp4: Mp4::Text("©grp"),
            ape: Some("Grouping"),
        },
        Spec {
            field: F::Work,
            vorbis: "WORK",
            also_vorbis: &[],
            aliases: &["©wrk"],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::User("WORK"),
            mp4: Mp4::Text("©wrk"),
            ape: Some("Work"),
        },
        Spec {
            field: F::Language,
            vorbis: "LANGUAGE",
            also_vorbis: &[],
            aliases: &["tlan"],
            kind: K::Language,
            scope: S::Recording,
            id3: Id3::Text("TLAN"),
            mp4: Mp4::Free("LANGUAGE"),
            ape: Some("Language"),
        },
        Spec {
            field: F::Copyright,
            vorbis: "COPYRIGHT",
            also_vorbis: &[],
            aliases: &["tcop", "cprt"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::Text("TCOP"),
            mp4: Mp4::Text("cprt"),
            ape: Some("Copyright"),
        },
        Spec {
            field: F::Comment,
            vorbis: "COMMENT",
            also_vorbis: &[],
            aliases: &["comm", "description"],
            kind: K::Text,
            scope: S::Recording,
            id3: Id3::Comment,
            mp4: Mp4::Text("©cmt"),
            ape: Some("Comment"),
        },
        Spec {
            field: F::MusicBrainzWorkId,
            vorbis: "MUSICBRAINZ_WORKID",
            also_vorbis: &[],
            aliases: &["musicbrainz work id"],
            kind: K::Mbid,
            scope: S::RecordingId,
            id3: Id3::User("MusicBrainz Work Id"),
            mp4: Mp4::Free("MusicBrainz Work Id"),
            ape: Some("MUSICBRAINZ_WORKID"),
        },
        Spec {
            field: F::MusicBrainzDiscId,
            vorbis: "MUSICBRAINZ_DISCID",
            also_vorbis: &[],
            aliases: &["musicbrainz disc id"],
            kind: K::Text,
            scope: S::Release,
            id3: Id3::User("MusicBrainz Disc Id"),
            mp4: Mp4::Free("MusicBrainz Disc Id"),
            ape: Some("MUSICBRAINZ_DISCID"),
        },
    ]
};

/// An MP4 atom's four bytes from its name as Picard spells it: `©` is the
/// byte 0xA9 in every one.
#[must_use]
pub fn fourcc(name: &str) -> [u8; 4] {
    let mut out = [0; 4];
    for (at, c) in out.iter_mut().zip(name.chars()) {
        *at = if c == '©' {
            0xA9
        } else {
            u8::try_from(c).unwrap_or(b'?')
        };
    }
    out
}

/// The Vorbis comment field a song list's tag name sets: a known field
/// under its own spelling, any other upper-cased.
#[must_use]
pub fn vorbis_key(name: &str) -> String {
    Field::named(name).map_or_else(|| name.to_ascii_uppercase(), |f| f.vorbis().to_string())
}

/// `value` of the tag `name` as a song list holds it when set by hand,
/// read as its field's [`Kind`] says, or what is wrong with it: a name a
/// Vorbis comment cannot carry (printable ASCII but `=`, as APEv2 keys
/// are ASCII too), a number that is none, a date that is none, or a gain
/// muman measures and writes over. An empty value sets nothing, so it is
/// never wrong.
pub fn by_hand(name: &str, value: &str) -> Result<String, String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| (0x20..=0x7D).contains(&b) && b != b'=')
    {
        return Err(format!(
            "{name:?} is no tag name: give one of ASCII letters, digits and marks but ="
        ));
    }
    if value.trim().is_empty() {
        return Ok(String::new());
    }
    let key = vorbis_key(name);
    if key.starts_with("REPLAYGAIN_") || key.starts_with("R128_") {
        return Err(format!(
            "{name} is measured by muman and written over on every sync; set [loudness] in \
             the song list instead"
        ));
    }
    let kind = Field::named(name).map_or(Kind::Text, |f| f.spec().kind);
    kind.read(value)
        .map_err(|why| format!("{name} = {value:?} is {why}"))
}

/// What is wrong with `value` as `name`'s, set by hand, as [`by_hand`]
/// reads it.
#[must_use]
pub fn bad_value(name: &str, value: &str) -> Option<String> {
    by_hand(name, value).err()
}

/// One source's value for a field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// Several for a field set more than once, as one artist each.
    pub values: Vec<String>,
    /// From a field kept for it, rather than read off a title or a
    /// channel name.
    pub structured: bool,
}

pub type Offers = BTreeMap<Field, Offer>;

/// The longest value a source may offer, in bytes: a name runs to a few
/// hundred, and a value past this is no name but a server's garbage,
/// which would fail every write of the song.
const MAX_VALUE: usize = 4096;

/// Offer `values` for `field`. Every source's values pass here, so a
/// value reads alike whatever offers it, as its field's [`Kind`] reads
/// it: a count with no leading zeros, a date as `YYYY-MM-DD`, and one
/// that reads as none, as a count of 0, dropped. A control character, which cuts a C string
/// short or moves a terminal's cursor, and one reordering the text are
/// dropped, a line break or tab spaced.
fn offer(offers: &mut Offers, field: Field, values: Vec<String>, structured: bool) {
    let kind = field.spec().kind;
    let values: Vec<String> = values
        .into_iter()
        .filter(|v| v.len() <= MAX_VALUE)
        .map(|v| {
            let v: String = v
                .chars()
                .filter(|c| !crate::naming::reorders(*c))
                .filter_map(|c| match c {
                    c if c.is_whitespace() && c.is_control() => Some(' '),
                    c if c.is_control() => None,
                    c => Some(c),
                })
                .collect();
            kind.read(&v).unwrap_or_default()
        })
        .filter(|v| !v.is_empty())
        .fold(Vec::new(), |mut seen, v| {
            if !seen.contains(&v) {
                seen.push(v);
            }
            seen
        });
    if !values.is_empty() && !offers.contains_key(&field) {
        offers.insert(field, Offer { values, structured });
    }
}

fn one(value: Option<&String>) -> Vec<String> {
    value.cloned().into_iter().collect()
}

/// What yt-dlp's info JSON offers. A release names its track, artists
/// and album in fields of their own; an upload has only its title and
/// its channel.
#[must_use]
pub fn from_info(info: &VideoInfo) -> Offers {
    let mut o = Offers::new();
    offer(&mut o, Field::Title, one(info.track.as_ref()), true);
    offer(&mut o, Field::Title, one(info.title.as_ref()), false);
    offer(
        &mut o,
        Field::Artist,
        info.artists.clone().unwrap_or_default(),
        true,
    );
    offer(&mut o, Field::Artist, one(info.artist.as_ref()), true);
    offer(
        &mut o,
        Field::Artist,
        info.creators.clone().unwrap_or_default(),
        true,
    );
    offer(&mut o, Field::Artist, one(info.creator.as_ref()), true);
    offer(
        &mut o,
        Field::Artist,
        info.channel_name().into_iter().collect(),
        false,
    );
    offer(&mut o, Field::Album, one(info.album.as_ref()), true);
    offer(
        &mut o,
        Field::AlbumArtist,
        info.album_artists
            .clone()
            // Every artist on the album: its artist only when it names one.
            .filter(|all| all.len() == 1)
            .unwrap_or_default(),
        true,
    );
    offer(
        &mut o,
        Field::AlbumArtist,
        one(info.album_artist.as_ref()),
        true,
    );
    offer(
        &mut o,
        Field::Track,
        info.track_number
            .map(|n| n.to_string())
            .into_iter()
            .collect(),
        true,
    );
    offer(
        &mut o,
        Field::Disc,
        info.disc_number
            .map(|n| n.to_string())
            .into_iter()
            .collect(),
        true,
    );
    offer(
        &mut o,
        Field::Date,
        info.release_date
            .as_deref()
            .and_then(iso_date)
            .into_iter()
            .collect(),
        true,
    );
    offer(
        &mut o,
        Field::Date,
        info.release_year
            .map(|y| y.to_string())
            .into_iter()
            .collect(),
        true,
    );
    offer(
        &mut o,
        Field::Date,
        info.upload_date
            .as_deref()
            .and_then(iso_date)
            .into_iter()
            .collect(),
        false,
    );
    offer(
        &mut o,
        Field::Genre,
        info.genres.clone().unwrap_or_default(),
        true,
    );
    offer(&mut o, Field::Genre, one(info.genre.as_ref()), true);
    o
}

/// What a MusicBrainz record offers: the recording's title, artists,
/// ISRCs, IDs and first release's date, and its release's album, album
/// artist, track, disc, date, track total, country, IDs, status, types
/// and medium, and the labels, catalogue numbers, barcode, ASIN and
/// script a lookup of the release answers.
#[must_use]
pub fn from_record(record: &crate::musicbrainz::Record) -> Offers {
    let one = |v: Option<&String>| v.cloned().into_iter().collect::<Vec<_>>();
    let number = |n: Option<u32>| n.map(|n| n.to_string()).into_iter().collect();
    let mut o = Offers::new();
    offer(&mut o, Field::Title, vec![record.title.clone()], true);
    offer(&mut o, Field::Artist, record.artists.clone(), true);
    offer(&mut o, Field::Isrc, record.isrcs.clone(), true);
    offer(
        &mut o,
        Field::MusicBrainzTrackId,
        vec![record.id.to_string()],
        true,
    );
    offer(
        &mut o,
        Field::MusicBrainzArtistId,
        record.artist_ids.clone(),
        true,
    );
    if let Some(r) = &record.release {
        offer(&mut o, Field::Album, vec![r.title.clone()], true);
        offer(&mut o, Field::AlbumArtist, r.artists.clone(), true);
        offer(&mut o, Field::Track, number(r.track), true);
        offer(&mut o, Field::Disc, number(r.disc), true);
        offer(&mut o, Field::Date, one(r.date.as_ref()), true);
        offer(&mut o, Field::TrackTotal, number(r.tracks), true);
        offer(&mut o, Field::ReleaseCountry, one(r.country.as_ref()), true);
        offer(&mut o, Field::MusicBrainzAlbumId, vec![r.id.clone()], true);
        offer(
            &mut o,
            Field::MusicBrainzReleaseGroupId,
            one(r.group_id.as_ref()),
            true,
        );
        offer(
            &mut o,
            Field::MusicBrainzReleaseTrackId,
            one(r.track_id.as_ref()),
            true,
        );
        offer(
            &mut o,
            Field::MusicBrainzAlbumArtistId,
            r.artist_ids.clone(),
            true,
        );
        // Picard writes a release's status and types in lower case.
        let lower = |v: &String| v.to_lowercase();
        offer(
            &mut o,
            Field::ReleaseStatus,
            r.status.iter().map(lower).collect(),
            true,
        );
        offer(
            &mut o,
            Field::ReleaseType,
            r.types.iter().map(lower).collect(),
            true,
        );
        offer(&mut o, Field::Media, one(r.media.as_ref()), true);
        let d = &r.details;
        offer(&mut o, Field::Label, d.labels.clone(), true);
        offer(
            &mut o,
            Field::CatalogNumber,
            d.catalog_numbers.clone(),
            true,
        );
        offer(&mut o, Field::Barcode, one(d.barcode.as_ref()), true);
        offer(&mut o, Field::Asin, one(d.asin.as_ref()), true);
        offer(&mut o, Field::Script, one(d.script.as_ref()), true);
    }
    // The release group's first release, as Picard writes it, else the
    // recording's, for a record kept before its release told it.
    let group_date = record
        .release
        .as_ref()
        .and_then(|r| r.details.original_date.as_ref());
    offer(
        &mut o,
        Field::OriginalDate,
        one(group_date.or(record.original_date.as_ref())),
        true,
    );
    o
}

/// What a file's own tags offer, every one structured: someone set it.
/// A file's tags by their Vorbis comment names, each with every value
/// it holds, as `lofty` reads them: the format's own frames and atoms
/// named as muman writes them ([`crate::render`]), and a comment given
/// twice, as two `TITLE`s, kept as two values where ffprobe joins them
/// with `;`. The file's main tag first; another, as an `ID3v1` beside an
/// `ID3v2`, adds only names the first lacks. `None` for a file `lofty`
/// does not read, as Matroska.
#[must_use]
pub fn read_file(path: &std::path::Path) -> Option<BTreeMap<String, Vec<String>>> {
    use lofty::file::TaggedFileExt;
    use lofty::tag::{ItemValue, TagType};

    let file = read_tagged(path).ok()?;
    let mut tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let primary = file.primary_tag().map(lofty::tag::Tag::tag_type);
    let mut ordered: Vec<&lofty::tag::Tag> = file.tags().iter().collect();
    ordered.sort_by_key(|t| Some(t.tag_type()) != primary);
    for tag in ordered {
        let mut own: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for item in tag.items() {
            let (Some(name), ItemValue::Text(text) | ItemValue::Locator(text)) =
                (item.key().map_key(TagType::VorbisComments), item.value())
            else {
                continue;
            };
            own.entry(name.to_ascii_lowercase())
                .or_default()
                .push(text.clone());
        }
        for (name, values) in own {
            tags.entry(name).or_insert(values);
        }
    }
    tags.extend(native(path, file.file_type()));
    Some(tags)
}

/// The lyrics a file's own tags hold, as `lofty` reads them: `USLT` in
/// `ID3v2`, `LYRICS` or else `UNSYNCEDLYRICS` in a Vorbis comment, `©lyr`
/// in MP4 and `Lyrics` in APEv2; with the ISO 639-2 language a `USLT`
/// frame names, unless it names none, as `XXX` or `und`.
#[must_use]
pub fn lyrics_in(path: &std::path::Path) -> Option<(String, Option<String>)> {
    use lofty::file::TaggedFileExt;
    use lofty::tag::{ItemKey, ItemValue};

    let file = read_tagged(path).ok()?;
    let primary = file.primary_tag().map(lofty::tag::Tag::tag_type);
    let mut ordered: Vec<&lofty::tag::Tag> = file.tags().iter().collect();
    ordered.sort_by_key(|t| Some(t.tag_type()) != primary);
    [ItemKey::Lyrics, ItemKey::UnsyncLyrics]
        .iter()
        .find_map(|key| {
            ordered
                .iter()
                .flat_map(|t| t.items())
                .filter(|i| i.key() == *key)
                .find_map(|i| match i.value() {
                    ItemValue::Text(text) if !text.trim().is_empty() => Some((text, i.lang())),
                    _ => None,
                })
        })
        .map(|(text, lang)| {
            let lang = String::from_utf8_lossy(lang).to_ascii_lowercase();
            let named = lang.len() == 3
                && lang.bytes().all(|b| b.is_ascii_lowercase())
                && !matches!(lang.as_str(), "xxx" | "und" | "zxx" | "mis" | "mul");
            (text.clone(), named.then_some(lang))
        })
}

/// The fields a file's own frames, atoms or items hold, read as
/// [`TABLE`] names where each is written, by its Vorbis name: what
/// `lofty`'s generic tag reads not, or not whole, as a `TXXX` or a
/// freeform atom under Picard's capitals, a Vorbis comment its map has
/// no key for, as `ASIN`, or an `ID3v2.3` genre given by its `ID3v1`
/// number. An `ID3v2.3` artist is read whole, `/` and all, as Picard
/// reads it: the slash that would part two names is in one's too.
fn native(path: &std::path::Path, kind: lofty::file::FileType) -> BTreeMap<String, Vec<String>> {
    use lofty::file::{AudioFile, FileType};

    let options = lofty::config::ParseOptions::new().read_properties(false);
    let Ok(mut file) = std::fs::File::open(path) else {
        return BTreeMap::new();
    };
    let comments = match kind {
        FileType::Flac => lofty::flac::FlacFile::read_from(&mut file, options)
            .ok()
            .and_then(|f| f.vorbis_comments().map(vorbis_fields)),
        FileType::Vorbis => lofty::ogg::VorbisFile::read_from(&mut file, options)
            .ok()
            .map(|f| vorbis_fields(f.vorbis_comments())),
        FileType::Opus => lofty::ogg::OpusFile::read_from(&mut file, options)
            .ok()
            .map(|f| vorbis_fields(f.vorbis_comments())),
        FileType::Speex => lofty::ogg::SpeexFile::read_from(&mut file, options)
            .ok()
            .map(|f| vorbis_fields(f.vorbis_comments())),
        _ => None,
    };
    if let Some(comments) = comments {
        return comments;
    }
    let read = match kind {
        FileType::Mpeg => lofty::mpeg::MpegFile::read_from(&mut file, options)
            .ok()
            .and_then(|f| f.id3v2().map(id3_fields)),
        FileType::Mp4 => lofty::mp4::Mp4File::read_from(&mut file, options)
            .ok()
            .and_then(|f| f.ilst().map(ilst_fields)),
        FileType::WavPack => lofty::wavpack::WavPackFile::read_from(&mut file, options)
            .ok()
            .and_then(|f| f.ape().map(ape_fields)),
        _ => None,
    };
    read.unwrap_or_default()
        .into_iter()
        .filter(|(_, values)| !values.is_empty())
        .map(|(spec, values)| (spec.vorbis.to_ascii_lowercase(), values))
        .collect()
}

type Read = Vec<(&'static Spec, Vec<String>)>;

/// Every field of [`TABLE`] Vorbis comments hold, under any of its names.
fn vorbis_fields(comments: &lofty::ogg::tag::VorbisComments) -> BTreeMap<String, Vec<String>> {
    let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, value) in comments.items() {
        if (Field::named(name).is_some() || r128(name).is_some()) && !value.trim().is_empty() {
            fields
                .entry(name.to_ascii_lowercase())
                .or_default()
                .push(value.to_string());
        }
    }
    fields
}

/// The genres of `TCON`: an `ID3v1` number, `(17)` in 2.3 or `17` in
/// 2.4, as its name, and a name given after its number once.
fn genres(tag: &lofty::id3::v2::Id3v2Tag) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for genre in tag.genres().into_iter().flatten() {
        let genre = genre.trim();
        if !genre.is_empty() && !out.iter().any(|g| g == genre) {
            out.push(genre.to_string());
        }
    }
    out
}

fn id3_fields(tag: &lofty::id3::v2::Id3v2Tag) -> Read {
    use lofty::id3::v2::{Frame, FrameId};

    let split = |text: &str| -> Vec<String> {
        text.split('\0')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect()
    };
    let text = |id: &str| {
        FrameId::new(id)
            .ok()
            .and_then(|id| tag.get_text(&id))
            .map(split)
    };
    let frames: Vec<&Frame<'_>> = tag.into_iter().collect();
    TABLE
        .iter()
        .map(|spec| {
            let values = match spec.id3 {
                Id3::Text("TCON") => genres(tag),
                // iTunes writes a grouping into `GRP1`, its `TIT1` then the work.
                Id3::Text("TIT1") => text("GRP1").or_else(|| text("TIT1")).unwrap_or_default(),
                Id3::Text(id) | Id3::Place(id) => text(id).unwrap_or_default(),
                Id3::Time(id) => frames
                    .iter()
                    .find_map(|f| match f {
                        Frame::Timestamp(t) if f.id_str() == id => Some(t.timestamp.to_string()),
                        Frame::Text(t) if f.id_str() == id => Some(t.value.to_string()),
                        _ => None,
                    })
                    .into_iter()
                    .collect(),
                Id3::User(name) => frames
                    .iter()
                    .find_map(|f| match f {
                        Frame::UserText(t) if t.description.eq_ignore_ascii_case(name) => {
                            Some(split(&t.content))
                        }
                        _ => None,
                    })
                    .unwrap_or_default(),
                Id3::Ufid => frames
                    .iter()
                    .find_map(|f| match f {
                        Frame::UniqueFileIdentifier(u) if u.owner == UFID_OWNER => {
                            String::from_utf8(u.identifier.to_vec()).ok()
                        }
                        _ => None,
                    })
                    .into_iter()
                    .collect(),
                Id3::Within => Vec::new(),
                Id3::Whole(id) => FrameId::new(id)
                    .ok()
                    .and_then(|id| tag.get_text(&id))
                    .map(split)
                    .unwrap_or_default(),
                Id3::Key => FrameId::new("TKEY")
                    .ok()
                    .and_then(|id| tag.get_text(&id))
                    .map(split)
                    .or_else(|| {
                        frames.iter().find_map(|f| match f {
                            Frame::UserText(t)
                                if t.description.eq_ignore_ascii_case("INITIALKEY") =>
                            {
                                Some(split(&t.content))
                            }
                            _ => None,
                        })
                    })
                    .unwrap_or_default(),
                Id3::Comment => frames
                    .iter()
                    .find_map(|f| match f {
                        Frame::Comment(c) if c.description.is_empty() => {
                            Some(c.content.to_string())
                        }
                        _ => None,
                    })
                    .into_iter()
                    .collect(),
            };
            (spec, values)
        })
        .collect()
}

fn ilst_fields(ilst: &lofty::mp4::Ilst) -> Read {
    use lofty::mp4::{AtomData, AtomIdent};
    use lofty::tag::Accessor;

    let texts = |atom: &lofty::mp4::Atom<'_>| -> Vec<String> {
        atom.data()
            .filter_map(|d| match d {
                AtomData::UTF8(t) | AtomData::UTF16(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    };
    // A writer may repeat an atom a value, rather than give one several.
    let all = |is: &dyn Fn(&AtomIdent<'_>) -> bool| -> Vec<String> {
        ilst.into_iter()
            .filter(|a| is(a.ident()))
            .flat_map(texts)
            .collect()
    };
    TABLE
        .iter()
        .map(|spec| {
            let values = match spec.mp4 {
                Mp4::Text(name) => all(&|i| *i == AtomIdent::Fourcc(fourcc(name))),
                Mp4::Free(name) => all(&|i| {
                    matches!(i, AtomIdent::Freeform { name: n, .. } if n.eq_ignore_ascii_case(name))
                }),
                Mp4::Whole(name) => ilst
                    .get(&AtomIdent::Fourcc(fourcc(name)))
                    .into_iter()
                    .flat_map(lofty::mp4::Atom::data)
                    .filter_map(|d| match d {
                        AtomData::SignedInteger(n) => Some(n.to_string()),
                        AtomData::UnsignedInteger(n) => Some(n.to_string()),
                        AtomData::Unknown { data, .. } if !data.is_empty() && data.len() <= 4 => {
                            let mut be = [0; 4];
                            be[4 - data.len()..].copy_from_slice(data);
                            Some(u32::from_be_bytes(be).to_string())
                        }
                        _ => None,
                    })
                    .collect(),
                Mp4::Flag(name) => ilst
                    .get(&AtomIdent::Fourcc(fourcc(name)))
                    .into_iter()
                    .flat_map(lofty::mp4::Atom::data)
                    .filter_map(|d| match d {
                        AtomData::Bool(b) => Some(if *b { "1" } else { "0" }.to_string()),
                        AtomData::Unknown { data, .. } => {
                            data.first().map(|b| if *b == 0 { "0" } else { "1" }.to_string())
                        }
                        _ => None,
                    })
                    .collect(),
                Mp4::Place(_) | Mp4::Within => {
                    let n = match spec.field {
                        Field::Track => ilst.track(),
                        Field::Disc => ilst.disk(),
                        Field::TrackTotal => ilst.track_total(),
                        _ => ilst.disk_total(),
                    };
                    n.filter(|&n| n > 0).map(|n| n.to_string()).into_iter().collect()
                }
            };
            (spec, values)
        })
        .collect()
}

fn ape_fields(ape: &lofty::ape::ApeTag) -> Read {
    TABLE
        .iter()
        .filter_map(|spec| {
            let item = ape.get(spec.ape?)?;
            let values = item.text_values()?.map(str::to_string).collect();
            Some((spec, values))
        })
        .collect()
}

/// The file at `path` read by `lofty` for its tags alone. Its audio's
/// properties are not read: `lofty` refuses some a file may hold, as an
/// Opus stream of mapping family 255, and a file's tags never need them.
pub(crate) fn read_tagged(
    path: &std::path::Path,
) -> Result<lofty::file::TaggedFile, lofty::error::FileParseError> {
    let mut probe = lofty::probe::Probe::open(path)?;
    // A file being written, as `song.opus.part`, is known by its bytes.
    if probe.file_type().is_none() {
        probe = probe.guess_file_type()?;
    }
    probe
        .options(lofty::config::ParseOptions::new().read_properties(false))
        .read()
}

/// A track or disc written `3/12` offers its total too, unless the file
/// names the total in a tag of its own.
#[must_use]
pub fn from_container(tags: &BTreeMap<String, Vec<String>>) -> Offers {
    let mut o = Offers::new();
    let mut totals = Vec::new();
    for (key, values) in tags {
        if let Some(field) = r128(key) {
            let gains = values.iter().filter_map(|v| r128_gain(v)).collect();
            offer(&mut o, field, gains, true);
            continue;
        }
        let Some(field) = Field::named(key) else {
            continue;
        };
        let values = values
            .iter()
            .flat_map(|value| container_values(field, value, &mut totals))
            .collect();
        offer(&mut o, field, values, true);
    }
    for (field, total) in totals {
        offer(&mut o, field, vec![total], true);
    }
    o
}

/// What one value of a file's `field` offers, a total it names put in
/// `totals`.
fn container_values(field: Field, value: &str, totals: &mut Vec<(Field, String)>) -> Vec<String> {
    {
        match field {
            Field::Date => value
                .trim()
                .split('T')
                .next()
                .map(|d| iso_date(d).unwrap_or_else(|| d.to_string()))
                .into_iter()
                .collect(),
            Field::Track | Field::Disc => {
                let (n, total) = value.split_once('/').unwrap_or((value, ""));
                let of = if field == Field::Track {
                    Field::TrackTotal
                } else {
                    Field::DiscTotal
                };
                totals.push((of, total.to_string()));
                vec![n.to_string()]
            }
            Field::TrackTotal | Field::DiscTotal => vec![value.to_string()],
            Field::TrackGain | Field::AlbumGain => gain(value).into_iter().collect(),
            Field::TrackPeak | Field::AlbumPeak => peak(value).into_iter().collect(),
            f if f.is_id() => value.split([';', '/']).map(str::to_string).collect(),
            _ => vec![value.to_string()],
        }
    }
}

/// The gain field an Opus file's R128 tag stands for.
fn r128(key: &str) -> Option<Field> {
    match key.to_ascii_lowercase().as_str() {
        "r128_track_gain" => Some(Field::TrackGain),
        "r128_album_gain" => Some(Field::AlbumGain),
        _ => None,
    }
}

/// How far R128's reference, -23 LUFS, lies below ReplayGain's.
pub const R128_BELOW_REPLAYGAIN_DB: f64 = 5.0;

/// An R128 gain, in 1/256 dB against -23 LUFS, as a ReplayGain one.
fn r128_gain(value: &str) -> Option<String> {
    let steps: i16 = value.trim().parse().ok()?;
    Some(gain_text(
        f64::from(steps) / 256.0 + R128_BELOW_REPLAYGAIN_DB,
    ))
}

/// A ReplayGain gain as muman writes it, `-6.12 dB`, from any spelling.
fn gain(value: &str) -> Option<String> {
    let number = value
        .trim()
        .trim_end_matches(|c: char| c.is_alphabetic() || c.is_whitespace());
    number
        .parse::<f64>()
        .ok()
        .filter(|db| db.is_finite() && db.abs() < 100.0)
        .map(gain_text)
}

fn gain_text(db: f64) -> String {
    format!("{db:.2} dB")
}

fn peak(value: &str) -> Option<String> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|p| p.is_finite() && *p >= 0.0)
        .map(|p| format!("{p:.6}"))
}

/// `YYYYMMDD` as `YYYY-MM-DD`, the Vorbis comment convention; a date
/// already written so passes through.
#[must_use]
pub fn iso_date(date: &str) -> Option<String> {
    let d = date.trim();
    if d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit()) {
        return Some(format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..]));
    }
    let shaped = d.len() == 10
        && d.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    shaped.then(|| d.to_string())
}

/// How much packaging a value carries: brackets naming the video, as
/// `(Official Video)`, any `【…】`, a `|` or ` / ` splitting title from
/// artist, and emoji.
#[must_use]
pub fn decorations(value: &str) -> usize {
    let mut count = 0;
    let lower = value.to_lowercase();
    for (open, close) in [('(', ')'), ('[', ']'), ('（', '）')] {
        let mut rest = lower.as_str();
        while let Some(start) = rest.find(open) {
            let inner = &rest[start + open.len_utf8()..];
            let Some(end) = inner.find(close) else { break };
            if clean::is_packaging(&inner[..end]) {
                count += 1;
            }
            rest = &inner[end + close.len_utf8()..];
        }
    }
    count += lower.matches('【').count();
    count += [" | ", "｜", " / ", " ⧸ ", "／"]
        .iter()
        .filter(|s| lower.contains(*s))
        .count();
    count += value
        .chars()
        .filter(|c| matches!(u32::from(*c), 0x1F300..=0x1FAFF | 0x2600..=0x27BF))
        .count();
    count
}

/// A value reduced to its letters and digits, case-folded, for asking
/// whether two sources agree.
#[must_use]
pub fn normalized(values: &[String]) -> String {
    values
        .iter()
        .flat_map(|v| v.chars())
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::info;

    fn info(json: &str) -> VideoInfo {
        info::parse(json.as_bytes()).unwrap()
    }

    fn each(tags: BTreeMap<String, String>) -> BTreeMap<String, Vec<String>> {
        tags.into_iter().map(|(k, v)| (k, vec![v])).collect()
    }

    #[test]
    fn a_release_offers_structured_fields() {
        let o = from_info(&info(
            r#"{"title": "Song (Official Video)", "track": "Song", "artists": ["A", "B", "A"],
                "album": "Record", "release_date": "20240102", "channel": "A - Topic"}"#,
        ));
        assert_eq!(
            o[&Field::Title],
            Offer {
                values: vec!["Song".into()],
                structured: true
            }
        );
        assert_eq!(o[&Field::Artist].values, ["A", "B"]);
        assert_eq!(o[&Field::Album].values, ["Record"]);
        assert_eq!(o[&Field::Date].values, ["2024-01-02"]);
    }

    #[test]
    fn every_artist_on_an_album_is_not_its_artist() {
        let o = from_info(&info(
            r#"{"title": "Song", "album": "Record", "album_artists": ["Marlo Venn", "Kiri Hoshino"]}"#,
        ));
        assert!(!o.contains_key(&Field::AlbumArtist));
        let o = from_info(&info(
            r#"{"title": "Song", "album": "Record", "album_artists": ["Marlo Venn"]}"#,
        ));
        assert_eq!(o[&Field::AlbumArtist].values, ["Marlo Venn"]);
    }

    #[test]
    fn an_upload_offers_its_title_and_channel_as_derived() {
        let o = from_info(&info(
            r#"{"title": "Song / Kiri", "uploader": "Lu5ivo", "upload_date": "20260527"}"#,
        ));
        assert!(!o[&Field::Title].structured);
        assert_eq!(
            o[&Field::Artist],
            Offer {
                values: vec!["Lu5ivo".into()],
                structured: false
            }
        );
        assert!(!o.contains_key(&Field::Album));
        assert!(!o[&Field::Date].structured);
    }

    #[test]
    fn a_track_or_disc_numbered_0_offers_none() {
        let o = from_info(&info(
            r#"{"title": "Song", "track_number": 0, "disc_number": 0}"#,
        ));
        assert!(!o.contains_key(&Field::Track));
        assert!(!o.contains_key(&Field::Disc));
    }

    #[test]
    fn a_file_s_tags_are_structured_and_normalized() {
        let tags = BTreeMap::from([
            ("title".to_string(), "Song".to_string()),
            ("album_artist".to_string(), "A".to_string()),
            ("track".to_string(), "03/12".to_string()),
            ("date".to_string(), "2001-05-06T00:00:00".to_string()),
            ("encoder".to_string(), "x".to_string()),
        ]);
        let o = from_container(&each(tags));
        assert_eq!(o[&Field::Track].values, ["3"]);
        assert_eq!(o[&Field::Date].values, ["2001-05-06"]);
        assert_eq!(o[&Field::AlbumArtist].values, ["A"]);
        assert_eq!(o[&Field::TrackTotal].values, ["12"]);
        assert!(o.values().all(|v| v.structured));
        assert_eq!(o.len(), 5);
    }

    #[test]
    fn a_file_s_musicbrainz_ids_and_totals_are_read_by_any_name() {
        let tags = BTreeMap::from([
            ("TSRC".to_string(), "XX0000000001".to_string()),
            (
                "MusicBrainz Artist Id".to_string(),
                "00000000-0000-0000-0000-00000000000a;00000000-0000-0000-0000-00000000000b"
                    .to_string(),
            ),
            ("disc".to_string(), "1/2".to_string()),
            ("TOTALDISCS".to_string(), "3".to_string()),
        ]);
        let o = from_container(&each(tags));
        assert_eq!(o[&Field::Isrc].values, ["XX0000000001"]);
        assert_eq!(o[&Field::MusicBrainzArtistId].values.len(), 2);
        assert_eq!(o[&Field::Disc].values, ["1"]);
        assert_eq!(o[&Field::DiscTotal].values, ["3"], "a tag of its own wins");
    }

    #[test]
    fn a_value_is_read_as_its_kind_spells_it() {
        let read = |kind: Kind, v| kind.read(v).ok();
        for (v, want) in [
            ("2019", "2019"),
            ("2019-7", "2019-07"),
            ("2019/07/04", "2019-07-04"),
            ("20190704", "2019-07-04"),
            ("2019.07.04", "2019-07-04"),
            ("2019-07-04T10:00:00Z", "2019-07-04"),
            ("2019-07-04 10:00", "2019-07-04"),
        ] {
            assert_eq!(read(Kind::Date, v).as_deref(), Some(want), "{v}");
        }
        for bad in [
            "July 2019",
            "2019-13",
            "2019-07-32",
            "2019-02-29",
            "2019-04-31",
            "0000",
            "19",
            "2019-07-04-01",
        ] {
            assert_eq!(read(Kind::Date, bad), None, "{bad}");
        }
        assert_eq!(
            read(Kind::Date, "2020-02-29").as_deref(),
            Some("2020-02-29")
        );
        assert_eq!(read(Kind::Date, "1900-02-29"), None);
        assert_eq!(read(Kind::Decimal, "0.004"), None, "TBPM would hold 0");
        assert_eq!(read(Kind::Decimal, "9999.5"), None, "TBPM would hold 10000");
        assert_eq!(read(Kind::Decimal, "0.5").as_deref(), Some("0.5"));
        assert_eq!(read(Kind::Place, "03/12").as_deref(), Some("3/12"));
        assert_eq!(read(Kind::Place, "0"), None);
        assert_eq!(read(Kind::Count, "3/12"), None);
        assert_eq!(
            read(Kind::Isrc, "xx-zz1-24-00001").as_deref(),
            Some("XXZZ12400001")
        );
        assert_eq!(read(Kind::Isrc, "XXZZ124000"), None);
        assert_eq!(
            read(Kind::Mbid, "00000000-0000-0000-0000-00000000000A").as_deref(),
            Some("00000000-0000-0000-0000-00000000000a")
        );
        assert_eq!(read(Kind::Mbid, "not-an-id"), None);
        assert_eq!(read(Kind::Text, "a\u{0}b"), None);
        assert_eq!(
            read(Kind::Text, "two\nlines").as_deref(),
            Some("two\nlines")
        );
    }

    #[test]
    fn a_source_s_value_its_field_cannot_hold_is_not_offered() {
        let tags = BTreeMap::from([
            ("date".to_string(), "2019/07/04".to_string()),
            ("isrc".to_string(), "xx-zz1-24-00001".to_string()),
            ("musicbrainz track id".to_string(), "none".to_string()),
        ]);
        let o = from_container(&each(tags));
        assert_eq!(o[&Field::Date].values, ["2019-07-04"]);
        assert_eq!(o[&Field::Isrc].values, ["XXZZ12400001"]);
        assert!(!o.contains_key(&Field::MusicBrainzTrackId));
        let o = from_container(&each(BTreeMap::from([(
            "date".to_string(),
            "July 2019".to_string(),
        )])));
        assert!(!o.contains_key(&Field::Date));
    }

    #[test]
    fn extended_kinds_read_as_their_formats_hold_them() {
        let read = |kind: Kind, v| kind.read(v).ok();
        assert_eq!(read(Kind::Decimal, "127.50").as_deref(), Some("127.5"));
        assert_eq!(read(Kind::Decimal, "128").as_deref(), Some("128"));
        assert_eq!(read(Kind::Decimal, "fast"), None);
        assert_eq!(read(Kind::Flag, "true").as_deref(), Some("1"));
        assert_eq!(read(Kind::Flag, "maybe"), None);
        assert_eq!(
            read(Kind::Barcode, "0 12345 67890 5").as_deref(),
            Some("012345678905")
        );
        assert_eq!(read(Kind::Barcode, "12345"), None);
        assert_eq!(read(Kind::Language, "ja").as_deref(), Some("jpn"));
        assert_eq!(read(Kind::Language, "SWE").as_deref(), Some("swe"));
        assert!(id3_key("Abm") && id3_key("F#") && id3_key("C") && id3_key("o"));
        assert!(!id3_key("8A") && !id3_key("H") && !id3_key("A minor") && !id3_key("Am#"));
        assert_eq!(Field::named("initialkey"), Some(Field::InitialKey));
        assert_eq!(Field::named("INITIALKEY"), Some(Field::InitialKey));
        assert_eq!(Field::named("publisher"), Some(Field::Label));
    }

    #[test]
    fn the_table_holds_every_field_in_its_order() {
        for (n, field) in Field::ALL.into_iter().enumerate() {
            assert_eq!(TABLE[n].field, field);
        }
        assert_eq!(Field::named("Album Artist"), Some(Field::AlbumArtist));
    }

    #[test]
    fn every_field_has_one_name_and_a_scope() {
        for f in Field::ALL {
            assert_eq!(Field::named(f.vorbis()), Some(f));
            for alias in f.aliases() {
                assert_eq!(Field::named(alias), Some(f), "{alias}");
            }
        }
        assert_eq!(
            Field::of(Scope::Recording).count()
                + Field::of(Scope::RecordingId).count()
                + Field::of(Scope::Release).count()
                + Field::of(Scope::Loudness).count()
                + Field::of(Scope::AlbumLoudness).count(),
            Field::ALL.len()
        );
    }

    #[test]
    fn an_identifier_comes_with_what_it_identifies() {
        for f in Field::ALL
            .iter()
            .filter(|f| f.is_id() || **f == Field::Isrc)
        {
            assert_ne!(f.scope(), Scope::Recording, "{f:?} picked on its own");
        }
    }

    #[test]
    fn an_offer_drops_controls_reordering_and_garbage() {
        let record = crate::musicbrainz::Record {
            id: crate::musicbrainz::Mbid::parse("00000000-0000-0000-0000-000000000001").unwrap(),
            title: "Esc\0ape\nTitle\u{202E}\u{1b}[2J".into(),
            artists: vec!["x".repeat(MAX_VALUE + 1), "Artist".into()],
            artist_ids: Vec::new(),
            isrcs: Vec::new(),
            length_ms: None,
            release: None,
            original_date: None,
        };
        let o = from_record(&record);
        assert_eq!(o[&Field::Title].values, ["Escape Title[2J"]);
        assert_eq!(o[&Field::Artist].values, ["Artist"]);
    }

    #[test]
    fn a_musicbrainz_record_offers_its_release_whole() {
        let record = crate::musicbrainz::Record {
            id: crate::musicbrainz::Mbid::parse("00000000-0000-0000-0000-000000000001").unwrap(),
            title: "Song".into(),
            artists: vec!["A".into(), "B".into()],
            artist_ids: vec!["00000000-0000-0000-0000-0000000000a1".into()],
            isrcs: vec!["XX0000000001".into()],
            length_ms: Some(200_000),
            release: Some(crate::musicbrainz::Release {
                id: "00000000-0000-0000-0000-00000000000a".into(),
                title: "Record".into(),
                artists: vec!["A".into()],
                artist_ids: vec!["00000000-0000-0000-0000-0000000000a1".into()],
                group_id: Some("00000000-0000-0000-0000-00000000000b".into()),
                date: Some("2004".into()),
                country: Some("XW".into()),
                track: Some(3),
                tracks: Some(12),
                track_id: None,
                disc: Some(1),
                status: Some("Official".into()),
                types: vec!["Album".into(), "Compilation".into()],
                media: Some("CD".into()),
                details: crate::musicbrainz::Details {
                    labels: vec!["Harbor Lights Records".into()],
                    catalog_numbers: vec!["HLR-0042".into()],
                    barcode: Some("0012345678905".into()),
                    asin: None,
                    script: Some("Latn".into()),
                    original_date: None,
                },
            }),
            original_date: Some("1999-02-01".into()),
        };
        let o = from_record(&record);
        assert_eq!(o[&Field::OriginalDate].values, ["1999-02-01"]);
        let mut grouped = record.clone();
        if let Some(r) = grouped.release.as_mut() {
            r.details.original_date = Some("2001".into());
        }
        assert_eq!(
            from_record(&grouped)[&Field::OriginalDate].values,
            ["2001"],
            "the release group's, as Picard's"
        );
        assert_eq!(o[&Field::ReleaseStatus].values, ["official"]);
        assert_eq!(o[&Field::ReleaseType].values, ["album", "compilation"]);
        assert_eq!(o[&Field::Media].values, ["CD"]);
        assert_eq!(o[&Field::Label].values, ["Harbor Lights Records"]);
        assert_eq!(o[&Field::CatalogNumber].values, ["HLR-0042"]);
        assert_eq!(o[&Field::Barcode].values, ["0012345678905"]);
        assert_eq!(o[&Field::Script].values, ["Latn"]);
        assert_eq!(o[&Field::Artist].values, ["A", "B"]);
        assert_eq!(o[&Field::Date].values, ["2004"]);
        assert_eq!(o[&Field::Track].values, ["3"]);
        assert_eq!(o[&Field::TrackTotal].values, ["12"]);
        assert_eq!(
            o[&Field::MusicBrainzTrackId].values,
            ["00000000-0000-0000-0000-000000000001"]
        );
        assert!(!o.contains_key(&Field::MusicBrainzReleaseTrackId));
        assert!(o.values().all(|v| v.structured));
        assert_eq!(o.len(), 23);
    }

    #[test]
    fn packaging_counts_and_a_featured_artist_does_not() {
        assert_eq!(decorations("Song (feat. Kiri)"), 0);
        assert_eq!(decorations("Song (Official Music Video)"), 1);
        assert_eq!(decorations("【MV】Song / Singer"), 2);
        assert_eq!(decorations("Song [4K] 🎵"), 2);
        assert_eq!(decorations("Glass/Orchard"), 0);
    }

    #[test]
    fn names_map_to_vorbis_fields() {
        assert_eq!(vorbis_key("album_artist"), "ALBUMARTIST");
        assert_eq!(vorbis_key("year"), "DATE");
        assert_eq!(vorbis_key("comment"), "COMMENT");
    }

    #[test]
    fn agreement_ignores_case_and_punctuation() {
        assert_eq!(
            normalized(&["Lu5ivo!".into()]),
            normalized(&["lu5ivo".into()])
        );
    }

    #[test]
    fn a_files_tags_read_alike_whatever_its_format_and_repeated_ones_apart() {
        use lofty::config::WriteOptions;
        use lofty::file::TaggedFileExt;
        use lofty::tag::{Accessor, ItemKey, ItemValue, Tag, TagExt, TagItem, TagType};

        let dir = tempfile::tempdir().unwrap();
        let flac = dir.path().join("a.flac");
        std::fs::write(&flac, crate::testing::SILENCE_FLAC).unwrap();
        let mut vorbis = Tag::new(TagType::VorbisComments);
        for title in ["Lantern Weather", "Lantern Weather (Live)"] {
            vorbis.push(TagItem::new(
                ItemKey::TrackTitle,
                ItemValue::Text(title.into()),
            ));
        }
        vorbis.set_track(3);
        vorbis.save_to_path(&flac, WriteOptions::default()).unwrap();
        let o = from_container(&read_file(&flac).unwrap());
        assert_eq!(
            o[&Field::Title].values,
            ["Lantern Weather", "Lantern Weather (Live)"]
        );
        assert_eq!(o[&Field::Track].values, ["3"]);

        let mp3 = dir.path().join("a.mp3");
        std::fs::write(&mp3, crate::testing::SILENCE_MP3).unwrap();
        let mut id3 = Tag::new(TagType::Id3v2);
        id3.set_title("Paper Comets".into());
        id3.set_artist("Marlo Venn".into());
        id3.set_track(7);
        id3.set_track_total(12);
        id3.save_to_path(&mp3, WriteOptions::default()).unwrap();
        assert!(lofty::read_from_path(&mp3).unwrap().primary_tag().is_some());
        let o = from_container(&read_file(&mp3).unwrap());
        assert_eq!(o[&Field::Title].values, ["Paper Comets"]);
        assert_eq!(o[&Field::Artist].values, ["Marlo Venn"]);
        assert_eq!(o[&Field::Track].values, ["7"]);
        assert_eq!(o[&Field::TrackTotal].values, ["12"]);
        assert!(read_file(&dir.path().join("none.flac")).is_none());
    }

    #[test]
    fn an_mp3_s_grouping_is_its_grp1_else_its_tit1() {
        use lofty::config::WriteOptions;
        use lofty::id3::v2::{Frame, FrameId, Id3v2Tag, TextInformationFrame};
        use lofty::tag::TagExt;

        let dir = tempfile::tempdir().unwrap();
        let mp3 = dir.path().join("a.mp3");
        let text = |id: &str, value: &str| {
            Frame::Text(TextInformationFrame::new(
                FrameId::new(id.to_string()).unwrap(),
                lofty::TextEncoding::UTF8,
                value.to_string(),
            ))
        };
        for (frames, grouping) in [
            (vec![("TIT1", "Night Songs")], "Night Songs"),
            (
                vec![("TIT1", "Harbor Suite"), ("GRP1", "Night Songs")],
                "Night Songs",
            ),
        ] {
            std::fs::write(&mp3, crate::testing::SILENCE_MP3).unwrap();
            let mut id3 = Id3v2Tag::new();
            for (id, value) in frames {
                id3.insert(text(id, value));
            }
            id3.save_to_path(&mp3, WriteOptions::default()).unwrap();
            let o = from_container(&read_file(&mp3).unwrap());
            assert_eq!(o[&Field::Grouping].values, [grouping]);
            assert!(!o.contains_key(&Field::Work));
        }
    }

    #[test]
    fn a_file_s_own_lyrics_are_read_with_the_language_a_uslt_frame_names() {
        use lofty::config::WriteOptions;
        use lofty::tag::{ItemKey, ItemValue, Tag, TagExt, TagItem, TagType};

        let lrc = "[00:01.00]first invented line\n[00:03.00]second invented line\n";
        let dir = tempfile::tempdir().unwrap();
        let formats = [
            (
                "a.flac",
                crate::testing::SILENCE_FLAC,
                TagType::VorbisComments,
                ItemKey::Lyrics,
            ),
            (
                "a.opus",
                crate::testing::SILENCE_OPUS,
                TagType::VorbisComments,
                ItemKey::UnsyncLyrics,
            ),
            (
                "a.m4a",
                crate::testing::SILENCE_M4A,
                TagType::Mp4Ilst,
                ItemKey::Lyrics,
            ),
            (
                "a.wv",
                crate::testing::SILENCE_WV,
                TagType::Ape,
                ItemKey::Lyrics,
            ),
            (
                "a.mp3",
                crate::testing::SILENCE_MP3,
                TagType::Id3v2,
                ItemKey::UnsyncLyrics,
            ),
            (
                "b.mp3",
                crate::testing::SILENCE_MP3,
                TagType::Id3v2,
                ItemKey::UnsyncLyrics,
            ),
        ];
        for (name, bytes, kind, key) in formats {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(lyrics_in(&path), None, "{name}");
            let mut tag = Tag::new(kind);
            let mut item = TagItem::new(key, ItemValue::Text(lrc.into()));
            item.set_lang(if name == "a.mp3" { *b"jpn" } else { *b"XXX" });
            tag.push(item);
            tag.save_to_path(&path, WriteOptions::default()).unwrap();
            let language = (name == "a.mp3").then(|| "jpn".to_string());
            assert_eq!(
                lyrics_in(&path),
                Some((lrc.to_string(), language)),
                "{name}"
            );
        }
    }

    #[test]
    fn an_m4a_reads_several_values_from_one_atom_or_from_repeated_ones() {
        use lofty::config::{ParseOptions, WriteOptions};
        use lofty::file::AudioFile;
        use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst, Mp4File};
        use lofty::tag::{ItemKey, ItemValue, Tag, TagExt, TagItem, TagType};

        const ARTIST: AtomIdent<'static> = AtomIdent::Fourcc(*b"\xa9ART");
        let artists = ["Odile Marsh", "Bram Feld"];
        let dir = tempfile::tempdir().unwrap();
        let one = dir.path().join("one.m4a");
        std::fs::write(&one, crate::testing::SILENCE_M4A).unwrap();
        let mut ilst = Ilst::new();
        let data = artists.map(|a| AtomData::UTF8(a.into())).to_vec();
        ilst.insert(Atom::from_collection(ARTIST, data).unwrap());
        ilst.save_to_path(&one, WriteOptions::default()).unwrap();
        let repeated = dir.path().join("repeated.m4a");
        std::fs::write(&repeated, crate::testing::SILENCE_M4A).unwrap();
        let mut generic = Tag::new(TagType::Mp4Ilst);
        for artist in artists {
            generic.push(TagItem::new(
                ItemKey::TrackArtist,
                ItemValue::Text(artist.into()),
            ));
        }
        Ilst::from(generic)
            .save_to_path(&repeated, WriteOptions::default())
            .unwrap();
        for (path, atoms) in [(&one, 1), (&repeated, 2)] {
            let mut file = std::fs::File::open(path).unwrap();
            let m4a = Mp4File::read_from(&mut file, ParseOptions::new()).unwrap();
            let held = m4a.ilst().unwrap().into_iter();
            assert_eq!(held.filter(|a| *a.ident() == ARTIST).count(), atoms);
            let o = from_container(&read_file(path).unwrap());
            assert_eq!(o[&Field::Artist].values, artists, "{}", path.display());
        }
    }

    #[test]
    fn loudness_is_read_from_replaygain_and_r128_alike() {
        let tags = each(BTreeMap::from([
            ("replaygain_track_gain".to_string(), "-6.1 dB".to_string()),
            ("replaygain_track_peak".to_string(), "0.98".to_string()),
            ("r128_album_gain".to_string(), "-1024".to_string()),
            ("replaygain_album_peak".to_string(), "loud".to_string()),
        ]));
        let o = from_container(&tags);
        assert_eq!(o[&Field::TrackGain].values, ["-6.10 dB"]);
        assert_eq!(o[&Field::TrackPeak].values, ["0.980000"]);
        assert_eq!(o[&Field::AlbumGain].values, ["1.00 dB"], "-4 dB under R128");
        assert!(!o.contains_key(&Field::AlbumPeak));
    }
}
