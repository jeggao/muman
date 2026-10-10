//! Files of tags a user hands `add`, read into the tags songs are given
//! by hand.
//!
//! A tag file says what a song is called and who made it; it is not
//! kept as a source, but matched to its song once and written into the
//! song's `tags`, which beat every source's. These are read:
//!
//! | Format | Recognized by |
//! |---|---|
//! | Vorbis comments, `NAME=value` a line, as `metaflac --export-tags-to` writes | `.txt`, `.tags` or `.vc` whose lines are mostly `NAME=value`, one a known field |
//! | ffmpeg's metadata file: its global section | `.ffmeta`, `.ffmetadata`, or a first line `;FFMETADATA1` |
//! | JSON: beets' `export`, a MusicBrainz recording, a record muman keeps | `.json`, an object, an array or one object a line |
//! | A cue sheet | `.cue`, read by [`crate::cue`] |
//!
//! Some names describe another file rather than the song, and a hand
//! value would outlive it: gains and peaks (a hand gain beats the
//! measured one, but these were measured on someone else's file), the
//! encoder, embedded pictures and lyrics. Those are skipped ([`SKIPPED`]).
//! Any other name is kept, as hand tags keep any Vorbis name.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::tags::{self, Field};

/// Tag names and their values, in the order a file gave them.
pub type Tags = Vec<(String, Vec<String>)>;

/// The names a tag file may hold but a song is never given by hand: of
/// the file's loudness, pictures, lyrics, encoding or container.
pub const SKIPPED: [&str; 17] = [
    "METADATA_BLOCK_PICTURE",
    "COVERART",
    "COVERARTMIME",
    "ENCODER",
    "ENCODED_BY",
    "ENCODEDBY",
    "ENCODERSETTINGS",
    "ENCODING",
    "VENDOR",
    "WAVEFORMATEXTENSIBLE_CHANNEL_MASK",
    "MAJOR_BRAND",
    "MINOR_VERSION",
    "COMPATIBLE_BRANDS",
    "CREATION_TIME",
    "HANDLER_NAME",
    "ITUNES_CDDB_1",
    "ACOUSTID_FINGERPRINT",
];

/// The beginnings of names skipped as [`SKIPPED`] is: gains, and the
/// tags iTunes and `MP3Gain` keep of their own.
const SKIPPED_PREFIXES: [&str; 4] = ["REPLAYGAIN_", "R128_", "MP3GAIN_", "ITUN"];

/// Whether a song is never given the tag `key` by hand.
fn skipped(key: &str) -> bool {
    SKIPPED.contains(&key)
        || SKIPPED_PREFIXES.iter().any(|p| key.starts_with(p))
        // LYRICS, UNSYNCEDLYRICS, `LYRICS-ENG` as ffmpeg names ID3's.
        || key.contains("LYRICS")
        || Field::named(key).is_some_and(Field::is_loudness)
}

/// The longest value taken, as the tags of a source are capped.
const MOST_BYTES: usize = 4096;

/// One song's part of a tag file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Track {
    pub tags: Tags,
    pub number: Option<u32>,
    pub length_ms: Option<i64>,
    /// The audio file a cue sheet names for it.
    pub file: Option<String>,
}

/// What a tag file holds: tags of the whole album, and of each song.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sheet {
    pub album: Tags,
    pub tracks: Vec<Track>,
    /// Names left out, and why.
    pub skipped: Vec<String>,
}

impl Sheet {
    /// The tags track `n` is given: the album's, then its own over them.
    #[must_use]
    pub fn tags_of(&self, n: usize) -> Tags {
        let mut out = self.album.clone();
        for (name, values) in &self.tracks[n].tags {
            match out.iter_mut().find(|(k, _)| k == name) {
                Some((_, v)) => v.clone_from(values),
                None => out.push((name.clone(), values.clone())),
            }
        }
        out
    }

    pub(crate) fn add(&mut self, tags: &mut Tags, name: &str, value: &str) {
        let key = tags::vorbis_key(name.trim());
        let cleaned: String = value
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        let mut value = cleaned.trim();
        if value.len() > MOST_BYTES {
            let mut end = MOST_BYTES;
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            value = &value[..end];
        }
        if key.is_empty() || value.is_empty() {
            return;
        }
        if skipped(&key) {
            self.skip(format!("{key}, which describes another file"));
            return;
        }
        let mut put = |key: &str, value: String| match tags.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) if !v.contains(&value) => v.push(value),
            Some(_) => {}
            None => tags.push((key.to_string(), vec![value])),
        };
        match key.as_str() {
            "TRACKNUMBER" | "DISCNUMBER" => {
                let (n, total) = value.split_once('/').unwrap_or((value, ""));
                let total_key = if key == "TRACKNUMBER" {
                    "TRACKTOTAL"
                } else {
                    "DISCTOTAL"
                };
                if let Some(n) = number(n) {
                    put(&key, n.to_string());
                }
                if let Some(t) = number(total) {
                    put(total_key, t.to_string());
                }
            }
            "DATE" => match date(value) {
                Some(d) => put(&key, d),
                None => self.skip(format!("DATE {value:?}, which reads as no date")),
            },
            "TRACKTOTAL" | "DISCTOTAL" => match number(value) {
                Some(n) => put(&key, n.to_string()),
                None => self.skip(format!("{key} {value:?}, which is no count")),
            },
            _ => put(&key, value.to_string()),
        }
    }

    fn skip(&mut self, why: String) {
        if !self.skipped.contains(&why) {
            self.skipped.push(why);
        }
    }
}

fn number(s: &str) -> Option<u32> {
    s.trim().parse().ok().filter(|&n| n > 0)
}

/// A date as Vorbis comments write it: a year, a month or a day, of a
/// real calendar; a time after it, as iTunes writes one, is cut.
fn date(s: &str) -> Option<String> {
    let s = s.trim();
    let s = match s.split_once('T') {
        Some((day, _)) if day.len() == 10 => day,
        _ => s,
    };
    let iso = if s.len() == 8 {
        tags::iso_date(s)?
    } else {
        s.to_string()
    };
    let parts: Vec<&str> = iso.split('-').collect();
    let digits = |p: &str, n: usize| p.len() == n && p.bytes().all(|b| b.is_ascii_digit());
    let within = |p: &str, most: u32| p.parse::<u32>().is_ok_and(|v| (1..=most).contains(&v));
    let fine = match parts[..] {
        [y] => digits(y, 4),
        [y, m] => digits(y, 4) && digits(m, 2) && within(m, 12),
        [y, m, d] => digits(y, 4) && digits(m, 2) && digits(d, 2) && within(m, 12) && within(d, 31),
        _ => false,
    };
    (fine && parts[0] != "0000").then_some(iso)
}

/// What kind of tag file `path`, holding `text`, is, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Vorbis,
    FfMetadata,
    Json,
    Cue,
}

const FFMETADATA: &str = ";FFMETADATA1";

/// The tag format `path` is written in, by its extension and, for text,
/// by what its lines hold.
#[must_use]
pub fn format_of(path: &Path, text: &str) -> Option<Format> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if text.trim_start_matches('\u{feff}').starts_with(FFMETADATA) {
        return Some(Format::FfMetadata);
    }
    match ext.as_str() {
        "cue" => Some(Format::Cue),
        "json" | "jsonl" => Some(Format::Json),
        "ffmeta" | "ffmetadata" => Some(Format::FfMetadata),
        "txt" | "tags" | "vc" if is_vorbis(text) => Some(Format::Vorbis),
        _ => None,
    }
}

/// Whether `text` is Vorbis comments: it opens with `NAME=value`, and
/// either four in five of its lines are so, or two name fields muman
/// knows. A value may run over lines of its own, as metaflac writes a
/// comment's newlines, so the second test lets a long one through.
fn is_vorbis(text: &str) -> bool {
    let lines: Vec<&str> = text
        .trim_start_matches('\u{feff}')
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    let named: Vec<&str> = lines.iter().filter_map(|l| vorbis_name(l)).collect();
    let known = named.iter().filter(|n| Field::named(n).is_some()).count();
    lines.first().is_some_and(|l| vorbis_name(l).is_some())
        && known > 0
        && (named.len() * 5 >= lines.len() * 4 || known >= 2)
}

/// The name a `NAME=value` line sets: printable ASCII but `=`.
fn vorbis_name(line: &str) -> Option<&str> {
    let (name, _) = line.split_once('=')?;
    (!name.is_empty()
        && name
            .bytes()
            .all(|b| (0x20..=0x7d).contains(&b) && b != b'='))
    .then_some(name)
}

/// The tags `text`, read from `path`, holds.
pub fn read(path: &Path, text: &str) -> Result<Sheet> {
    let read = match format_of(path, text) {
        Some(Format::Vorbis) => Ok(vorbis(text)),
        Some(Format::FfMetadata) => Ok(ffmetadata(text)),
        Some(Format::Json) => json(text),
        Some(Format::Cue) => crate::cue::read(text),
        None => bail!("holds no tags muman reads"),
    };
    let sheet = read.with_context(|| format!("reading {}", path.display()))?;
    if sheet.tracks.is_empty()
        || (sheet.album.is_empty() && sheet.tracks.iter().all(|t| t.tags.is_empty()))
    {
        bail!("{} holds no tag a song is given by hand", path.display());
    }
    Ok(sheet)
}

fn vorbis(text: &str) -> Sheet {
    let mut sheet = Sheet::default();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        match (vorbis_name(line), pairs.last_mut()) {
            (Some(name), _) => pairs.push((name.to_string(), line[name.len() + 1..].to_string())),
            // metaflac writes a value's newlines as they are.
            (None, Some((_, value))) => {
                value.push('\n');
                value.push_str(line);
            }
            (None, None) => {}
        }
    }
    let mut tags = Tags::new();
    for (name, value) in pairs {
        sheet.add(&mut tags, &name, &value);
    }
    sheet.tracks.push(Track {
        tags,
        ..Track::default()
    });
    sheet
}

fn ffmetadata(text: &str) -> Sheet {
    let mut sheet = Sheet::default();
    let mut tags = Tags::new();
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines().peekable();
    if lines.peek().is_some_and(|l| l.starts_with(FFMETADATA)) {
        lines.next();
    }
    while let Some(line) = lines.next() {
        if line.starts_with('[') {
            break;
        }
        if line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        let mut entry = line.to_string();
        // An odd run of backslashes ends in one escaping the newline.
        while entry.chars().rev().take_while(|c| *c == '\\').count() % 2 == 1 {
            entry.pop();
            entry.push('\n');
            entry.push_str(lines.next().unwrap_or_default());
        }
        let (name, value) = split_escaped(&entry);
        if let Some(value) = value {
            sheet.add(&mut tags, &name, &value);
        }
    }
    sheet.tracks.push(Track {
        tags,
        ..Track::default()
    });
    sheet
}

/// An ffmetadata line's key and value, each unescaped; no value when it
/// holds no unescaped `=`.
fn split_escaped(line: &str) -> (String, Option<String>) {
    let mut key = String::new();
    let mut value: Option<String> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        let c = if c == '\\' {
            chars.next().unwrap_or('\\')
        } else if c == '=' && value.is_none() {
            value = Some(String::new());
            continue;
        } else {
            c
        };
        match &mut value {
            Some(v) => v.push(c),
            None => key.push(c),
        }
    }
    (key, value)
}

fn json(text: &str) -> Result<Sheet> {
    let objects: Vec<Value> = match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(items)) => items,
        Ok(v @ Value::Object(_)) => vec![v],
        Ok(_) => bail!("holds no object of tags"),
        Err(_) => text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()
            .context("reading its JSON")?,
    };
    let mut sheet = Sheet::default();
    for o in &objects {
        let track = object(&mut sheet, o)?;
        sheet.tracks.push(track);
    }
    Ok(sheet)
}

/// beets' field names and the Vorbis names they set.
const BEETS: [(&str, Field); 18] = [
    ("title", Field::Title),
    ("artist", Field::Artist),
    ("artists", Field::Artist),
    ("album", Field::Album),
    ("albumartist", Field::AlbumArtist),
    ("albumartists", Field::AlbumArtist),
    ("track", Field::Track),
    ("tracktotal", Field::TrackTotal),
    ("disc", Field::Disc),
    ("disctotal", Field::DiscTotal),
    ("genre", Field::Genre),
    ("isrc", Field::Isrc),
    ("country", Field::ReleaseCountry),
    ("mb_trackid", Field::MusicBrainzTrackId),
    ("mb_releasetrackid", Field::MusicBrainzReleaseTrackId),
    ("mb_albumid", Field::MusicBrainzAlbumId),
    ("mb_releasegroupid", Field::MusicBrainzReleaseGroupId),
    ("mb_artistid", Field::MusicBrainzArtistId),
];

fn object(sheet: &mut Sheet, o: &Value) -> Result<Track> {
    let Some(map) = o.as_object() else {
        bail!("holds a value that is no object of tags");
    };
    if map.contains_key("media") || map.contains_key("release-group") {
        bail!("holds a MusicBrainz release, which names no one recording");
    }
    if map.contains_key("artist-credit") {
        let record = crate::musicbrainz::parse_recording(&o.to_string(), None)?;
        return Ok(recorded(sheet, &record));
    }
    if map.contains_key("artists")
        && map.contains_key("id")
        && map.contains_key("title")
        && let Ok(record) = serde_json::from_value::<crate::musicbrainz::Record>(o.clone())
    {
        return Ok(recorded(sheet, &record));
    }
    let mut tags = Tags::new();
    let text = |v: &Value| match v {
        Value::String(s) => vec![s.clone()],
        Value::Number(n) if n.as_f64() != Some(0.0) => vec![n.to_string()],
        Value::Array(a) => a
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    };
    for (name, field) in BEETS {
        for value in map.get(name).map(text).unwrap_or_default() {
            sheet.add(&mut tags, field.vorbis(), &value);
        }
    }
    for value in map.get("composer").map(text).unwrap_or_default() {
        sheet.add(&mut tags, "COMPOSER", &value);
    }
    let part = |k: &str| map.get(k).and_then(Value::as_u64).filter(|&n| n > 0);
    if let Some(year) = part("year") {
        let date = match (part("month"), part("day")) {
            (Some(month), Some(day)) => format!("{year:04}-{month:02}-{day:02}"),
            (Some(month), None) => format!("{year:04}-{month:02}"),
            _ => format!("{year:04}"),
        };
        sheet.add(&mut tags, "DATE", &date);
    }
    #[allow(clippy::cast_possible_truncation)]
    let length_ms = map
        .get("length")
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| (s * 1000.0).round() as i64);
    let number = map
        .get("track")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok());
    Ok(Track {
        tags,
        number: number.filter(|&n| n > 0),
        length_ms,
        file: None,
    })
}

fn recorded(sheet: &mut Sheet, record: &crate::musicbrainz::Record) -> Track {
    let mut tags = Tags::new();
    for (field, offer) in tags::from_record(record) {
        for value in &offer.values {
            sheet.add(&mut tags, field.vorbis(), value);
        }
    }
    Track {
        tags,
        number: None,
        length_ms: record.length_ms.and_then(|ms| i64::try_from(ms).ok()),
        file: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values<'a>(tags: &'a Tags, name: &str) -> Vec<&'a str> {
        tags.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    #[test]
    fn vorbis_comments_read_with_repeats_continued_lines_and_skips() {
        let text = "TITLE=Lantern Weather\nartist=Marlo Venn\nARTIST=Kiri Hoshino\n\
            TRACKNUMBER=3/12\nCOMMENT=one\ntwo\nREPLAYGAIN_TRACK_GAIN=-3.2 dB\n\
            ENCODER=some encoder\nDATE=20110304\nMOOD=calm\n";
        let path = Path::new("Lantern Weather.txt");
        assert_eq!(format_of(path, text), Some(Format::Vorbis));
        let sheet = read(path, text).unwrap();
        let tags = &sheet.tracks[0].tags;
        assert_eq!(values(tags, "ARTIST"), ["Marlo Venn", "Kiri Hoshino"]);
        assert_eq!(values(tags, "TRACKNUMBER"), ["3"]);
        assert_eq!(values(tags, "TRACKTOTAL"), ["12"]);
        assert_eq!(values(tags, "COMMENT"), ["one\ntwo"]);
        assert_eq!(values(tags, "DATE"), ["2011-03-04"]);
        assert_eq!(values(tags, "MOOD"), ["calm"]);
        assert!(values(tags, "REPLAYGAIN_TRACK_GAIN").is_empty());
        assert!(values(tags, "ENCODER").is_empty());
        assert_eq!(sheet.skipped.len(), 2, "{:?}", sheet.skipped);
    }

    #[test]
    fn plain_text_is_no_tag_file() {
        let lyrics = "Under the lantern\nweather turns = to rain\nand the moths\n";
        assert_eq!(format_of(Path::new("words.txt"), lyrics), None);
        assert_eq!(format_of(Path::new("a.txt"), "A=1\nB=2\n"), None);
        assert!(read(Path::new("words.txt"), lyrics).is_err());
    }

    #[test]
    fn ffmetadata_unescapes_and_stops_at_its_first_section() {
        let text = ";FFMETADATA1\ntitle=Copper \\= Moth\n; a comment\nartist=Marlo\\\nVenn\n\
            album=The Glass Orchards\\;\n[CHAPTER]\nTIMEBASE=1/1000\ntitle=Chapter One\n";
        let sheet = read(Path::new("tags.ffmeta"), text).unwrap();
        let tags = &sheet.tracks[0].tags;
        assert_eq!(values(tags, "TITLE"), ["Copper = Moth"]);
        assert_eq!(values(tags, "ARTIST"), ["Marlo\nVenn"]);
        assert_eq!(values(tags, "ALBUM"), ["The Glass Orchards;"]);
        assert_eq!(
            format_of(Path::new("x.txt"), text),
            Some(Format::FfMetadata)
        );
    }

    #[test]
    fn beets_json_reads_as_an_object_an_array_or_lines() {
        let one = r#"{"title": "Lantern Weather", "artists": ["Marlo Venn"], "album": "The Glass Orchards",
            "track": 3, "tracktotal": 0, "year": 2011, "month": 3, "day": 0, "length": 221.4,
            "mb_trackid": "00000000-0000-4000-8000-000000000001", "path": "/x/y.flac", "bitrate": 900000}"#;
        let sheet = read(Path::new("song.json"), one).unwrap();
        let track = &sheet.tracks[0];
        assert_eq!(values(&track.tags, "DATE"), ["2011-03"]);
        assert_eq!(values(&track.tags, "TRACKNUMBER"), ["3"]);
        assert!(values(&track.tags, "TRACKTOTAL").is_empty());
        assert_eq!(
            values(&track.tags, "MUSICBRAINZ_TRACKID"),
            ["00000000-0000-4000-8000-000000000001"]
        );
        assert_eq!(track.length_ms, Some(221_400));
        assert_eq!(track.number, Some(3));
        let lines = format!("{}\n{}\n", one.replace('\n', " "), one.replace('\n', " "));
        assert_eq!(read(Path::new("all.json"), &lines).unwrap().tracks.len(), 2);
        let array = format!("[{one}, {one}]");
        assert_eq!(read(Path::new("all.json"), &array).unwrap().tracks.len(), 2);
    }

    #[test]
    fn a_musicbrainz_recording_reads_through_its_record() {
        let ws = r#"{"id": "00000000-0000-4000-8000-000000000002", "title": "Copper Moth", "length": 200000,
            "artist-credit": [{"name": "Marlo Venn", "artist": {"id": "00000000-0000-4000-8000-000000000003", "name": "Marlo Venn"}}],
            "isrcs": ["XX0000000001"], "releases": []}"#;
        let track = &read(Path::new("rec.json"), ws).unwrap().tracks[0];
        assert_eq!(values(&track.tags, "TITLE"), ["Copper Moth"]);
        assert_eq!(values(&track.tags, "ISRC"), ["XX0000000001"]);
        assert_eq!(track.length_ms, Some(200_000));
    }

    #[test]
    fn a_file_that_sets_nothing_is_refused() {
        for empty in ["[]", "", "  \n"] {
            assert!(read(Path::new("tags.json"), empty).is_err(), "{empty:?}");
        }
        let gains = "TITLE=x\nREPLAYGAIN_TRACK_GAIN=-1 dB\n";
        assert!(read(Path::new("a.txt"), gains).is_ok());
        assert!(
            read(
                Path::new("a.txt"),
                "REPLAYGAIN_TRACK_GAIN=-1 dB\nREPLAYGAIN_TRACK_PEAK=1\n"
            )
            .is_err()
        );
        let release = r#"{"id": "00000000-0000-4000-8000-000000000004", "title": "The Glass Orchards",
            "artist-credit": [{"name": "Marlo Venn"}], "media": []}"#;
        assert!(read(Path::new("release.json"), release).is_err());
    }

    #[test]
    fn whatever_describes_another_file_is_skipped_in_any_spelling() {
        let text = "TITLE=x\nREPLAYGAIN_REFERENCE_LOUDNESS=89.0 dB\nlyrics-eng=sung words\n\
            UNSYNCED LYRICS=more\nMP3GAIN_MINMAX=1,2\nmajor_brand=M4A\ncreation_time=2011\n\
            iTunes_CDDB_1=x\nacoustid_fingerprint=AQAA\nr128_track_gain=0\n";
        let sheet = read(Path::new("a.txt"), text).unwrap();
        assert_eq!(sheet.tracks[0].tags.len(), 1, "{:?}", sheet.tracks[0].tags);
    }

    #[test]
    fn a_long_comment_runs_over_lines_and_values_are_clean() {
        let text = "TITLE=x\nARTIST=y\nCOMMENT=a\nb\nc\nd\ne\n";
        assert_eq!(format_of(Path::new("a.txt"), text), Some(Format::Vorbis));
        let sheet = read(Path::new("a.txt"), text).unwrap();
        assert_eq!(values(&sheet.tracks[0].tags, "COMMENT"), ["a\nb\nc\nd\ne"]);
        assert_eq!(
            format_of(Path::new("a.txt"), "a quiet line\nTITLE=x\nARTIST=y\n"),
            None
        );
        let long = format!("TITLE=x\u{7}{}\n", "é".repeat(5000));
        let sheet = read(Path::new("a.txt"), &long).unwrap();
        let title = &values(&sheet.tracks[0].tags, "TITLE")[0];
        assert!(title.len() <= MOST_BYTES && !title.contains('\u{7}'));
    }

    #[test]
    fn ffmetadata_continues_on_an_odd_run_of_backslashes_and_reads_without_its_header() {
        let sheet = read(Path::new("x.ffmeta"), ";FFMETADATA1\ntitle=a\\\\\\\nb\n").unwrap();
        assert_eq!(values(&sheet.tracks[0].tags, "TITLE"), ["a\\\nb"]);
        let bare = read(
            Path::new("x.ffmeta"),
            "title=Copper Moth\nartist=Marlo Venn\n",
        )
        .unwrap();
        assert_eq!(values(&bare.tracks[0].tags, "TITLE"), ["Copper Moth"]);
    }

    #[test]
    fn dates_and_counts_are_real_ones() {
        assert_eq!(date("2011-03-04T10:00:00Z"), Some("2011-03-04".into()));
        for bad in ["2011-13", "2011-00", "2011-13-45", "20111399", "0000", "11"] {
            assert_eq!(date(bad), None, "{bad}");
        }
        let sheet = read(Path::new("a.txt"), "TITLE=x\nTRACKTOTAL=abc\nDISCTOTAL=2\n").unwrap();
        assert!(values(&sheet.tracks[0].tags, "TRACKTOTAL").is_empty());
        assert_eq!(values(&sheet.tracks[0].tags, "DISCTOTAL"), ["2"]);
    }

    #[test]
    fn album_tags_give_way_to_a_tracks_own() {
        let sheet = Sheet {
            album: vec![
                ("ARTIST".into(), vec!["Marlo Venn".into()]),
                ("ALBUM".into(), vec!["Orchards".into()]),
            ],
            tracks: vec![Track {
                tags: vec![("ARTIST".into(), vec!["Kiri Hoshino".into()])],
                ..Track::default()
            }],
            skipped: Vec::new(),
        };
        assert_eq!(
            sheet.tags_of(0),
            [
                ("ARTIST".to_string(), vec!["Kiri Hoshino".to_string()]),
                ("ALBUM".into(), vec!["Orchards".into()])
            ]
        );
        assert_eq!(date("2011-3"), None);
        assert_eq!(date("2011"), Some("2011".into()));
    }
}
