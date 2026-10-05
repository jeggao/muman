//! The tags each source offers, and how much each offer is worth: a
//! dedicated metadata field beats a value read off a video's title or
//! channel, and a value decorated with `(Official Video)` or `【MV】`
//! is worth less than a plain one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::clean;
use crate::info::VideoInfo;

/// Names how a source's tags are read. Tags read by another are read
/// again, without measuring the source again, so changing what a file
/// or an info JSON offers means changing this.
pub const METHOD: &str = "tags/3";

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
}

impl Field {
    pub const ALL: [Self; 8] = [
        Self::Title,
        Self::Artist,
        Self::Album,
        Self::AlbumArtist,
        Self::Track,
        Self::Disc,
        Self::Date,
        Self::Genre,
    ];

    /// The fields that describe the release a song is on, which come
    /// from one source together so an album never splits.
    pub const RELEASE: [Self; 5] = [
        Self::Album,
        Self::AlbumArtist,
        Self::Track,
        Self::Disc,
        Self::Date,
    ];

    /// The Vorbis comment field it is written as.
    #[must_use]
    pub fn vorbis(self) -> &'static str {
        match self {
            Self::Title => "TITLE",
            Self::Artist => "ARTIST",
            Self::Album => "ALBUM",
            Self::AlbumArtist => "ALBUMARTIST",
            Self::Track => "TRACKNUMBER",
            Self::Disc => "DISCNUMBER",
            Self::Date => "DATE",
            Self::Genre => "GENRE",
        }
    }

    /// The field a song list's tag name sets, the common spellings
    /// included.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "title" => Self::Title,
            "artist" => Self::Artist,
            "album" => Self::Album,
            "album_artist" | "albumartist" => Self::AlbumArtist,
            "track" | "track_number" | "tracknumber" => Self::Track,
            "disc" | "disc_number" | "discnumber" => Self::Disc,
            "date" | "year" => Self::Date,
            "genre" => Self::Genre,
            _ => return None,
        })
    }
}

/// The Vorbis comment field a song list's tag name sets: a known field
/// under its own spelling, any other upper-cased.
#[must_use]
pub fn vorbis_key(name: &str) -> String {
    Field::named(name).map_or_else(|| name.to_ascii_uppercase(), |f| f.vorbis().to_string())
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

fn offer(offers: &mut Offers, field: Field, values: Vec<String>, structured: bool) {
    let values: Vec<String> = values
        .into_iter()
        .map(|v| v.trim().to_string())
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
        info.album_artists.clone().unwrap_or_default(),
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

/// What a file's own tags offer, every one structured: someone set it.
#[must_use]
pub fn from_container(tags: &BTreeMap<String, String>) -> Offers {
    let mut o = Offers::new();
    for (key, value) in tags {
        let Some(field) = Field::named(key) else {
            continue;
        };
        let value = match field {
            Field::Date => value
                .trim()
                .split('T')
                .next()
                .map(|d| iso_date(d).unwrap_or_else(|| d.to_string())),
            Field::Track | Field::Disc => value
                .split('/')
                .next()
                .map(|n| n.trim().trim_start_matches('0').to_string()),
            _ => Some(value.clone()),
        };
        offer(&mut o, field, value.into_iter().collect(), true);
    }
    o
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
    fn a_file_s_tags_are_structured_and_normalized() {
        let tags = BTreeMap::from([
            ("title".to_string(), "Song".to_string()),
            ("album_artist".to_string(), "A".to_string()),
            ("track".to_string(), "03/12".to_string()),
            ("date".to_string(), "2001-05-06T00:00:00".to_string()),
            ("comment".to_string(), "x".to_string()),
        ]);
        let o = from_container(&tags);
        assert_eq!(o[&Field::Track].values, ["3"]);
        assert_eq!(o[&Field::Date].values, ["2001-05-06"]);
        assert_eq!(o[&Field::AlbumArtist].values, ["A"]);
        assert!(o.values().all(|v| v.structured));
        assert_eq!(o.len(), 4);
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
}
