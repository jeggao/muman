//! Cue sheets, read for the tags of the album and each track they name.
//!
//! A sheet's commands before its first `TRACK` describe the album, those
//! after it the track: `TITLE`, `PERFORMER` and `SONGWRITER`, `ISRC`,
//! `CATALOG`, and the `REM` comments rippers write (`DATE`, `GENRE`,
//! `DISCNUMBER`, `TOTALDISCS`). A track's length is the distance from its
//! `INDEX 01` to the next track's in the same `FILE`, in frames of 1/75
//! s; the last track's in each file is unknown, the sheet holding no
//! file's length. `INDEX 00`, the pregap, belongs to the track before.

use anyhow::{Result, bail};

use crate::tagfile::{Sheet, Tags, Track};

const FRAMES_PER_SECOND: i64 = 75;

/// A line's words, a quoted one whole.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.trim().chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            out.push(chars.by_ref().take_while(|&c| c != '"').collect());
        } else {
            let mut word = String::new();
            while let Some(&c) = chars.peek().filter(|c| !c.is_whitespace()) {
                word.push(c);
                chars.next();
            }
            out.push(word);
        }
    }
    out
}

/// `mm:ss:ff` in frames.
fn frames(at: &str) -> Option<i64> {
    let parts: Vec<i64> = at
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    match parts[..] {
        [m, s, f] if (0..60).contains(&s) && (0..FRAMES_PER_SECOND).contains(&f) => {
            Some((m * 60 + s) * FRAMES_PER_SECOND + f)
        }
        _ => None,
    }
}

/// The tags the cue sheet `text` gives its album and tracks.
pub fn read(text: &str) -> Result<Sheet> {
    let mut sheet = Sheet::default();
    let mut album = Tags::new();
    let mut tracks: Vec<Track> = Vec::new();
    let mut starts: Vec<Option<(usize, i64)>> = Vec::new();
    let mut file: Option<String> = None;
    let mut files = 0;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let w = words(line);
        let Some(command) = w.first().map(|c| c.to_ascii_uppercase()) else {
            continue;
        };
        let arg = w.get(1).map_or("", String::as_str);
        let (tags, into_track) = match tracks.last_mut() {
            Some(t) => (&mut t.tags, true),
            None => (&mut album, false),
        };
        match command.as_str() {
            "FILE" => {
                file = Some(arg.to_string());
                files += 1;
            }
            "TRACK" => {
                let number = arg.parse().ok().filter(|&n: &u32| n > 0);
                let mut t = Track {
                    number,
                    file: file.clone(),
                    ..Track::default()
                };
                if let Some(n) = number {
                    sheet.add(&mut t.tags, "TRACKNUMBER", &n.to_string());
                }
                tracks.push(t);
                starts.push(None);
            }
            "INDEX" if into_track && arg == "01" => {
                let at = w.get(2).and_then(|a| frames(a));
                if let (Some(at), Some(start)) = (at, starts.last_mut()) {
                    *start = Some((files, at));
                }
            }
            "TITLE" => sheet.add(tags, if into_track { "TITLE" } else { "ALBUM" }, arg),
            "PERFORMER" => sheet.add(tags, if into_track { "ARTIST" } else { "ALBUMARTIST" }, arg),
            "SONGWRITER" => sheet.add(tags, "COMPOSER", arg),
            "ISRC" if into_track => sheet.add(tags, "ISRC", arg),
            "CATALOG" => sheet.add(tags, "BARCODE", arg),
            "REM" => {
                let value = w.get(2..).map(|v| v.join(" ")).unwrap_or_default();
                let upper = arg.to_ascii_uppercase();
                let name = match upper.as_str() {
                    "DATE" => "DATE",
                    "GENRE" => "GENRE",
                    "DISCNUMBER" => "DISCNUMBER",
                    "TOTALDISCS" => "DISCTOTAL",
                    "COMPOSER" => "COMPOSER",
                    other if other.starts_with("REPLAYGAIN_") => other,
                    _ => continue,
                };
                sheet.add(tags, name, &value);
            }
            _ => {}
        }
    }
    if tracks.is_empty() {
        bail!("names no track");
    }
    for n in 0..tracks.len() {
        if let (Some((f, at)), Some(Some((g, next)))) = (starts[n], starts.get(n + 1))
            && f == *g
            && *next > at
        {
            tracks[n].length_ms = Some((next - at) * 1000 / FRAMES_PER_SECOND);
        }
    }
    let performer: Vec<String> = album
        .iter()
        .find(|(k, _)| k == "ALBUMARTIST")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let total = tracks.len().to_string();
    for t in &mut tracks {
        if !performer.is_empty() && !t.tags.iter().any(|(k, _)| k == "ARTIST") {
            t.tags.push(("ARTIST".to_string(), performer.clone()));
        }
    }
    sheet.add(&mut album, "TRACKTOTAL", &total);
    sheet.album = album;
    sheet.tracks = tracks;
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: &str = "\u{feff}REM GENRE \"Chamber Pop\"\nREM DATE 2011\nREM COMMENT \"ripped\"\n\
        REM REPLAYGAIN_ALBUM_GAIN -4.10 dB\nCATALOG 0000000000017\nPERFORMER \"Marlo Venn\"\n\
        TITLE \"The Glass Orchards\"\nFILE \"01 Lantern Weather.flac\" WAVE\n  TRACK 01 AUDIO\n\
        \x20   TITLE \"Lantern Weather\"\n    ISRC XX0000000001\n    INDEX 01 00:00:00\n\
        FILE \"02 Copper Moth.flac\" WAVE\n  TRACK 02 AUDIO\n    TITLE \"Copper Moth\"\n\
        \x20   PERFORMER \"Marlo Venn & Kiri Hoshino\"\n    INDEX 00 00:00:00\n    INDEX 01 00:02:00\n";

    fn values<'a>(tags: &'a Tags, name: &str) -> Vec<&'a str> {
        tags.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    #[test]
    fn a_sheet_gives_its_album_and_each_track_their_tags() {
        let sheet = read(SHEET).unwrap();
        assert_eq!(values(&sheet.album, "ALBUM"), ["The Glass Orchards"]);
        assert_eq!(values(&sheet.album, "ALBUMARTIST"), ["Marlo Venn"]);
        assert_eq!(values(&sheet.album, "GENRE"), ["Chamber Pop"]);
        assert_eq!(values(&sheet.album, "DATE"), ["2011"]);
        assert_eq!(values(&sheet.album, "BARCODE"), ["0000000000017"]);
        assert_eq!(values(&sheet.album, "TRACKTOTAL"), ["2"]);
        assert!(values(&sheet.album, "COMMENT").is_empty());
        assert_eq!(sheet.skipped.len(), 1);
        let [one, two] = &sheet.tracks[..] else {
            panic!()
        };
        assert_eq!(one.file.as_deref(), Some("01 Lantern Weather.flac"));
        assert_eq!(values(&one.tags, "ARTIST"), ["Marlo Venn"]);
        assert_eq!(values(&one.tags, "ISRC"), ["XX0000000001"]);
        assert_eq!(values(&two.tags, "ARTIST"), ["Marlo Venn & Kiri Hoshino"]);
        assert_eq!(values(&two.tags, "TRACKNUMBER"), ["2"]);
        assert_eq!((one.length_ms, two.length_ms), (None, None));
    }

    #[test]
    fn lengths_come_from_indexes_in_one_file() {
        let image = "FILE \"disc.flac\" WAVE\nTRACK 01 AUDIO\nTITLE \"A\"\nINDEX 01 00:00:00\n\
            TRACK 02 AUDIO\nTITLE \"B\"\nINDEX 00 03:40:10\nINDEX 01 03:41:37\nTRACK 03 AUDIO\nTITLE \"C\"\n\
            TRACK 04 AUDIO\nINDEX 01 09:00:00\n";
        let sheet = read(image).unwrap();
        let lengths: Vec<Option<i64>> = sheet.tracks.iter().map(|t| t.length_ms).collect();
        assert_eq!(lengths, [Some(221_493), None, None, None]);
        assert_eq!(frames("01:02:74"), Some(4724));
        assert_eq!(frames("01:02:75"), None);
        assert!(read("REM nothing\n").is_err());
    }
}
