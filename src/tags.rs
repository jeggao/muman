//! The tags each source offers, and how much each offer is worth: a
//! dedicated metadata field, as a file's own tags, a release's info or a
//! MusicBrainz record holds, beats a value read off a video's title or
//! channel, and a value decorated with `(Official Video)` or `【MV】`
//! is worth less than a plain one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::clean;
use crate::info::VideoInfo;

/// Names how a source's tags are read. Tags read by another are read
/// again, without measuring the source again, so changing what a file
/// or an info JSON offers means changing this.
pub const METHOD: &str = "tags/6";

/// What a field describes, which decides where a song's value comes
/// from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The recording: each field from whichever source offers it best.
    Recording,
    /// The release the song is on: every field from the one source whose
    /// album ranks best, so an album never splits and its IDs never mix.
    Release,
}

/// A tag muman knows: how sources offer it, how a song list names it,
/// and where a song's value comes from. Adding a field is a variant and
/// its row in each `match` below; reading it from a file's tags, ranking
/// it and writing it follow. Picard's names are used where they differ,
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
}

impl Field {
    /// Every field, in the order written.
    pub const ALL: [Self; 18] = [
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
            Self::Isrc => "ISRC",
            Self::TrackTotal => "TRACKTOTAL",
            Self::DiscTotal => "DISCTOTAL",
            Self::ReleaseCountry => "RELEASECOUNTRY",
            Self::MusicBrainzTrackId => "MUSICBRAINZ_TRACKID",
            Self::MusicBrainzReleaseTrackId => "MUSICBRAINZ_RELEASETRACKID",
            Self::MusicBrainzAlbumId => "MUSICBRAINZ_ALBUMID",
            Self::MusicBrainzReleaseGroupId => "MUSICBRAINZ_RELEASEGROUPID",
            Self::MusicBrainzArtistId => "MUSICBRAINZ_ARTISTID",
            Self::MusicBrainzAlbumArtistId => "MUSICBRAINZ_ALBUMARTISTID",
        }
    }

    /// Its other names, lower-cased: the common spellings a song list may
    /// use, and the names ffprobe reports for MP3 and MP4 files.
    fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::AlbumArtist => &["album_artist"],
            Self::Track => &["track", "track_number"],
            Self::Disc => &["disc", "disc_number"],
            Self::Date => &["year"],
            Self::Isrc => &["tsrc"],
            Self::TrackTotal => &["track_total", "totaltracks"],
            Self::DiscTotal => &["disc_total", "totaldiscs"],
            Self::ReleaseCountry => &["release_country", "musicbrainz album release country"],
            Self::MusicBrainzTrackId => &["musicbrainz track id"],
            Self::MusicBrainzReleaseTrackId => &["musicbrainz release track id"],
            Self::MusicBrainzAlbumId => &["musicbrainz album id"],
            Self::MusicBrainzReleaseGroupId => &["musicbrainz release group id"],
            Self::MusicBrainzArtistId => &["musicbrainz artist id"],
            Self::MusicBrainzAlbumArtistId => &["musicbrainz album artist id"],
            Self::Title | Self::Artist | Self::Album | Self::Genre => &[],
        }
    }

    #[must_use]
    pub fn scope(self) -> Scope {
        match self {
            Self::Title
            | Self::Artist
            | Self::Genre
            | Self::Isrc
            | Self::MusicBrainzTrackId
            | Self::MusicBrainzArtistId => Scope::Recording,
            Self::Album
            | Self::AlbumArtist
            | Self::Track
            | Self::Disc
            | Self::Date
            | Self::TrackTotal
            | Self::DiscTotal
            | Self::ReleaseCountry
            | Self::MusicBrainzReleaseTrackId
            | Self::MusicBrainzAlbumId
            | Self::MusicBrainzReleaseGroupId
            | Self::MusicBrainzAlbumArtistId => Scope::Release,
        }
    }

    /// Whether its values are identifiers, several to a value in a file
    /// that joins them, and never cleaned.
    #[must_use]
    pub fn is_id(self) -> bool {
        matches!(
            self,
            Self::MusicBrainzTrackId
                | Self::MusicBrainzReleaseTrackId
                | Self::MusicBrainzAlbumId
                | Self::MusicBrainzReleaseGroupId
                | Self::MusicBrainzArtistId
                | Self::MusicBrainzAlbumArtistId
        )
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
            f.vorbis().eq_ignore_ascii_case(&lower) || f.aliases().contains(&lower.as_str())
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
    // Every source's values pass here, so a count reads alike whatever
    // offers it: no leading zeros, and 0, which counts nothing, is none.
    let counts = matches!(
        field,
        Field::Track | Field::Disc | Field::TrackTotal | Field::DiscTotal
    );
    let values: Vec<String> = values
        .into_iter()
        .map(|v| {
            let v = v.trim();
            if counts && v.bytes().all(|b| b.is_ascii_digit()) {
                v.trim_start_matches('0').to_string()
            } else {
                v.to_string()
            }
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

/// What a MusicBrainz record offers: the recording's title, artists,
/// ISRCs and IDs, and its release's album, album artist, track, disc,
/// date, track total, country and IDs.
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
        vec![record.id.clone()],
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
    }
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

    let file = lofty::read_from_path(path).ok()?;
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
    Some(tags)
}

/// A track or disc written `3/12` offers its total too, unless the file
/// names the total in a tag of its own.
#[must_use]
pub fn from_container(tags: &BTreeMap<String, Vec<String>>) -> Offers {
    let mut o = Offers::new();
    let mut totals = Vec::new();
    for (key, values) in tags {
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
            f if f.is_id() => value.split([';', '/']).map(str::to_string).collect(),
            _ => vec![value.to_string()],
        }
    }
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
            ("comment".to_string(), "x".to_string()),
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
    fn every_field_has_one_name_and_a_scope() {
        for f in Field::ALL {
            assert_eq!(Field::named(f.vorbis()), Some(f));
            for alias in f.aliases() {
                assert_eq!(Field::named(alias), Some(f), "{alias}");
            }
        }
        assert_eq!(
            Field::of(Scope::Recording).count() + Field::of(Scope::Release).count(),
            Field::ALL.len()
        );
    }

    #[test]
    fn a_musicbrainz_record_offers_its_release_whole() {
        let record = crate::musicbrainz::Record {
            id: "00000000-0000-0000-0000-000000000001".into(),
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
            }),
        };
        let o = from_record(&record);
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
        assert_eq!(o.len(), 15);
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
}
