//! Cue sheets, read for the tags of the album and each track they name.
//!
//! A sheet's commands before its first `TRACK` describe the album, those
//! after it the track: `TITLE`, `PERFORMER` and `SONGWRITER`, `ISRC`,
//! `CATALOG`, and the `REM` comments rippers write (`DATE`, `GENRE`,
//! `DISCNUMBER`, `TOTALDISCS`). A track's length is the distance from its
//! `INDEX 01` to the next track's in the same `FILE`, in frames of 1/75
//! s; the last track's in each file is unknown, the sheet holding no
//! file's length. `INDEX 00`, the pregap, belongs to the track before,
//! and a track is in the file its `INDEX 01` is in: EAC's sheets of one
//! file a track put a track's pregap in the file before. A data track, as
//! `MODE1/2352` on an enhanced CD, is no song. A sheet's paths are read
//! with `/` or `\` between folders, as Windows rippers write them, and a
//! name left unquoted is the rest of its line.

use anyhow::{Result, bail};

use crate::tagfile::{Sheet, Tags, Track};

const FRAMES_PER_SECOND: i64 = 75;

/// A line's words, a quoted one whole, `\"` in it a quote.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.trim().chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut word = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' if chars.peek() == Some(&'"') => word.extend(chars.next()),
                    c => word.push(c),
                }
            }
            out.push(word);
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
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let parts: Vec<i64> = at
        .split(':')
        .map(|p| digits(p).then(|| p.parse().ok()).flatten())
        .collect::<Option<_>>()?;
    match parts[..] {
        [m, s, f] if (0..60).contains(&s) && (0..FRAMES_PER_SECOND).contains(&f) => m
            .checked_mul(60)?
            .checked_add(s)?
            .checked_mul(FRAMES_PER_SECOND)?
            .checked_add(f),
        _ => None,
    }
}

/// Each track's length, from its start to the next track's in the same
/// file, `starts` holding each track's file and start in frames.
fn measure(tracks: &mut [Track], starts: &[Option<(usize, i64)>]) {
    for n in 0..tracks.len() {
        if let (Some((f, at)), Some(Some((g, next)))) = (starts[n], starts.get(n + 1))
            && f == *g
            && *next > at
        {
            tracks[n].length_ms = (next - at)
                .checked_mul(1000)
                .map(|ms| ms / FRAMES_PER_SECOND);
        }
    }
}

/// The name a `FILE` line gives, with `/` between its folders: its type is
/// the line's last word, its name the rest.
fn file_name(w: &[String]) -> String {
    let name = match w.get(1..).unwrap_or_default() {
        [name] => name.clone(),
        [name @ .., _] => name.join(" "),
        [] => String::new(),
    };
    name.replace('\\', "/")
}

/// The tag a `REM` comment rippers write sets, by its first word.
fn rem_tag(word: &str) -> Option<String> {
    let upper = word.to_ascii_uppercase();
    let name = match upper.as_str() {
        "DATE" | "GENRE" | "DISCNUMBER" | "COMPOSER" => upper.as_str(),
        "TOTALDISCS" => "DISCTOTAL",
        other if other.starts_with("REPLAYGAIN_") => other,
        _ => return None,
    };
    Some(name.to_string())
}

/// The tags the cue sheet `text` gives its album and tracks.
pub fn read(text: &str) -> Result<Sheet> {
    let mut sheet = Sheet::default();
    let mut album = Tags::new();
    let mut tracks: Vec<Track> = Vec::new();
    let mut starts: Vec<Option<(usize, i64)>> = Vec::new();
    let mut file: Option<String> = None;
    let mut files = 0;
    let mut data: Vec<bool> = Vec::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let w = words(line);
        let Some(command) = w.first().map(|c| c.to_ascii_uppercase()) else {
            continue;
        };
        let arg = w.get(1).map_or("", String::as_str);
        let text = w.get(1..).map(|v| v.join(" ")).unwrap_or_default();
        let (tags, into_track) = match tracks.last_mut() {
            Some(t) => (&mut t.tags, true),
            None => (&mut album, false),
        };
        match command.as_str() {
            "FILE" => {
                file = Some(file_name(&w));
                files += 1;
            }
            "TRACK" => {
                let number = arg.parse().ok().filter(|&n: &u32| n > 0);
                let audio = w
                    .get(2)
                    .is_none_or(|kind| kind.eq_ignore_ascii_case("AUDIO"));
                let mut t = Track {
                    number,
                    file: file.clone(),
                    ..Track::default()
                };
                if let Some(n) = number.filter(|_| audio) {
                    sheet.add(&mut t.tags, "TRACKNUMBER", &n.to_string());
                }
                tracks.push(t);
                starts.push(None);
                data.push(!audio);
            }
            "INDEX" if into_track && arg.parse::<u32>() == Ok(1) => {
                let at = w.get(2).and_then(|a| frames(a));
                if let (Some(at), Some(start)) = (at, starts.last_mut()) {
                    *start = Some((files, at));
                }
                if let Some(t) = tracks.last_mut() {
                    t.file.clone_from(&file);
                }
            }
            "TITLE" => sheet.set(tags, if into_track { "TITLE" } else { "ALBUM" }, &text),
            "PERFORMER" => sheet.set(
                tags,
                if into_track { "ARTIST" } else { "ALBUMARTIST" },
                &text,
            ),
            "SONGWRITER" => sheet.set(tags, "COMPOSER", &text),
            "ISRC" if into_track => sheet.set(tags, "ISRC", arg),
            "CATALOG" => sheet.set(tags, "BARCODE", arg),
            "REM" => {
                if let Some(name) = rem_tag(arg) {
                    let value = w.get(2..).map(|v| v.join(" ")).unwrap_or_default();
                    sheet.add(tags, &name, &value);
                }
            }
            _ => {}
        }
    }
    if tracks.is_empty() {
        bail!("names no track");
    }
    measure(&mut tracks, &starts);
    let mut kept = data.iter().map(|d| !d);
    tracks.retain(|_| kept.next().unwrap_or(true));
    if tracks.is_empty() {
        bail!("names no audio track");
    }
    let performer: Vec<String> = album
        .iter()
        .find(|(k, _)| k == "ALBUMARTIST")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let numbered = tracks.iter().filter(|t| t.number.is_some()).count();
    for t in &mut tracks {
        if !performer.is_empty() && !t.tags.iter().any(|(k, _)| k == "ARTIST") {
            t.tags.push(("ARTIST".to_string(), performer.clone()));
        }
    }
    if numbered > 0 {
        sheet.add(&mut album, "TRACKTOTAL", &numbered.to_string());
    }
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
        assert_eq!(values(&sheet.album, "COMMENT"), [] as [&str; 0]);
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
        assert_eq!(frames("999999999999999999:00:00"), None);
        assert_eq!(frames("-1:00:00"), None);
        assert_eq!(frames("+1:00:00"), None);
        let far = "FILE \"d.flac\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n\
            TRACK 02 AUDIO\nINDEX 01 1000000000000000:00:00\n";
        assert_eq!(read(far).unwrap().tracks[0].length_ms, None);
        assert!(read("REM nothing\n").is_err());
    }

    #[test]
    fn escaped_quotes_short_indexes_and_repeated_commands_read_as_meant() {
        let sheet = read(
            "PERFORMER \"Marlo\"\nPERFORMER \"Marlo Venn\"\nFILE \"d.flac\" WAVE\n\
             TRACK 01 AUDIO\nTITLE \"The \\\"Lantern\\\" Weather\"\nINDEX 1 00:00:00\n\
             TRACK 02 AUDIO\nINDEX 01 01:00:00\nTRACK x AUDIO\n",
        )
        .unwrap();
        assert_eq!(values(&sheet.album, "ALBUMARTIST"), ["Marlo Venn"]);
        assert_eq!(
            values(&sheet.tracks[0].tags, "TITLE"),
            ["The \"Lantern\" Weather"]
        );
        assert_eq!(sheet.tracks[0].length_ms, Some(60_000));
        assert_eq!(values(&sheet.album, "TRACKTOTAL"), ["2"]);
    }

    #[test]
    fn a_track_is_in_the_file_its_start_is_in_and_a_data_track_is_none() {
        let sheet = read(
            "FILE \"CD1\\01 Lantern Weather.flac\" WAVE\n  TRACK 01 AUDIO\n    TITLE Lantern \
             Weather\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    TITLE \"Copper Moth\"\n    \
             INDEX 00 03:00:00\nFILE \"CD1\\02 Copper Moth.flac\" WAVE\n    INDEX 01 00:00:00\n  \
             TRACK 03 MODE1/2352\n    INDEX 01 04:00:00\n",
        )
        .unwrap();
        assert_eq!(sheet.tracks.len(), 2, "the data track is no song");
        assert_eq!(
            sheet.tracks[0].file.as_deref(),
            Some("CD1/01 Lantern Weather.flac")
        );
        assert_eq!(
            sheet.tracks[1].file.as_deref(),
            Some("CD1/02 Copper Moth.flac")
        );
        assert_eq!(values(&sheet.tracks[0].tags, "TITLE"), ["Lantern Weather"]);
        assert_eq!(values(&sheet.album, "TRACKTOTAL"), ["2"]);
    }
}
