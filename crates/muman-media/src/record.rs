//! What muman keeps of a MusicBrainz recording and the release it
//! picked, as `<recording id>.json` in the store: the record
//! `musicbrainz.rs` makes from a search or a lookup, and the tags, facts
//! and tag files read. A record's ID names its file, so an [`Mbid`] is
//! checked where it comes in and never names another path.
//!
//! A recording as MusicBrainz answers it is read here too, with its
//! releases ranked as the MusicBrainz provider's docs say, so a tag file
//! that holds one is read as a lookup would read it
//! (`parse_recording`). Which recording a search's answer fits is the
//! provider's to say.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};

use crate::source::is_mbid;

/// A MusicBrainz ID, 32 hex digits in groups of 8, 4, 4, 4 and 12. It
/// names a file in the store, so it is checked where it comes in, from a
/// server or a tag, and a value of this type never names another path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Mbid(String);

impl Mbid {
    /// `s` as an MBID, if it has the shape of one.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        is_mbid(s).then(|| Self(s.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Mbid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Mbid {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        Self::parse(&s).ok_or_else(|| format!("{s:?} is no MusicBrainz ID"))
    }
}

impl From<Mbid> for String {
    fn from(id: Mbid) -> Self {
        id.0
    }
}

/// What muman keeps of a recording and the release it picked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The recording's MBID.
    pub id: Mbid,
    pub title: String,
    /// Each credited artist, as the recording credits them.
    pub artists: Vec<String>,
    /// The artists joined as the recording credits them, by its join
    /// phrases, as `Marlo Venn & The Glass Orchards`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub credit: String,
    /// The MBID of each credited artist.
    #[serde(default)]
    pub artist_ids: Vec<String>,
    /// What tells it from recordings of the same name, as `mono mix`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default)]
    pub isrcs: Vec<String>,
    #[serde(default)]
    pub length_ms: Option<u64>,
    #[serde(default)]
    pub release: Option<Release>,
    /// The other releases of `release`'s group that hold the recording:
    /// the album's other editions, without what only a lookup of a release
    /// answers but the group's first release.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub editions: Vec<Release>,
    /// The date of the recording's first release, `YYYY-MM-DD` or less.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_date: Option<String>,
    /// The languages its works' lyrics are in, as ISO 639-3: `zxx` for a
    /// work with none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// The release's MBID.
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    /// The artists joined as the release credits them, by its join phrases.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub credit: String,
    #[serde(default)]
    pub artist_ids: Vec<String>,
    /// The release group's MBID: the album, whichever its edition.
    #[serde(default)]
    pub group_id: Option<String>,
    /// `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
    #[serde(default)]
    pub date: Option<String>,
    /// Where it was released, as ISO 3166-1, or `XW` for the world.
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub track: Option<u32>,
    /// The tracks on the recording's disc.
    #[serde(default)]
    pub tracks: Option<u32>,
    /// The MBID of the recording's track on this release.
    #[serde(default)]
    pub track_id: Option<String>,
    #[serde(default)]
    pub disc: Option<u32>,
    /// `Official`, `Promotion`, `Bootleg` or `Pseudo-Release`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// The release group's primary type and its secondary ones, as
    /// `Album`, `Compilation`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    /// The format of the medium holding the recording, as `CD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    #[serde(default, flatten)]
    pub details: Details,
}

/// What only a lookup of the release itself answers: its labels and
/// catalogue numbers, barcode, ASIN and script.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Details {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub catalog_numbers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub barcode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asin: Option<String>,
    /// ISO 15924, as `Latn`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// The first release of its release group, as Picard's original date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_date: Option<String>,
}

/// Where a record is kept in the store.
#[must_use]
pub fn path_of(folder: &Path, id: &Mbid) -> PathBuf {
    folder.join(format!("{id}.json"))
}

/// Keep a record in `folder`.
pub fn keep(folder: &Path, record: &Record) -> Result<PathBuf> {
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let json = serde_json::to_vec_pretty(record).context("writing a MusicBrainz record")?;
    let name = crate::atomic::Name::new(format!("{}.json", record.id))?;
    crate::atomic::write(folder, &name, &json)?;
    Ok(path_of(folder, &record.id))
}

/// Read a kept record.
pub fn read(path: &Path) -> Result<Record> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))
}

/// A JSON `null` read as the type's default.
pub fn or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// A recording as a search or a lookup answers it.
#[derive(Debug, Deserialize)]
pub struct Recording {
    pub id: String,
    #[serde(default, deserialize_with = "or_default")]
    pub title: String,
    /// What tells it from recordings of the same name: a mix, a live date.
    #[serde(default, deserialize_with = "or_default")]
    pub disambiguation: String,
    #[serde(default)]
    pub length: Option<u64>,
    #[serde(default, deserialize_with = "or_default")]
    pub video: bool,
    #[serde(default, deserialize_with = "or_default")]
    pub score: u32,
    #[serde(default, rename = "artist-credit", deserialize_with = "or_default")]
    pub credit: Vec<Credit>,
    #[serde(default, deserialize_with = "or_default")]
    pub isrcs: Vec<Isrc>,
    #[serde(default, deserialize_with = "or_default")]
    pub releases: Vec<Found>,
    #[serde(default, rename = "first-release-date")]
    pub first_release_date: Option<String>,
    /// Its works, which a lookup asking for `work-rels` lists.
    #[serde(default, deserialize_with = "or_default")]
    pub relations: Vec<Relation>,
}

/// A recording's relation, as a lookup lists it; only a work's are read.
#[derive(Debug, Deserialize)]
pub struct Relation {
    #[serde(default)]
    pub work: Option<Work>,
}

#[derive(Debug, Deserialize)]
pub struct Work {
    #[serde(default, deserialize_with = "or_default")]
    pub languages: Vec<String>,
}

/// The lyrics languages of `relations`' works, each once.
#[must_use]
pub fn languages(relations: &[Relation]) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    for language in relations
        .iter()
        .filter_map(|r| r.work.as_ref())
        .flat_map(|w| &w.languages)
    {
        if !all.contains(language) {
            all.push(language.clone());
        }
    }
    all
}

/// An ISRC: a search lists each as `{"id": …}`, a lookup as text.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Isrc {
    Text(String),
    Object { id: String },
}

impl Isrc {
    fn code(&self) -> &str {
        match self {
            Self::Text(code) | Self::Object { id: code } => code,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Credit {
    /// The name as credited here, which may differ from the artist's own.
    #[serde(default, deserialize_with = "or_default")]
    pub name: String,
    /// What joins it to the next name, as ` & ` or ` feat. `.
    #[serde(default, deserialize_with = "or_default")]
    pub joinphrase: String,
    #[serde(default)]
    pub artist: Option<Artist>,
}

#[derive(Debug, Deserialize)]
pub struct Artist {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct Found {
    pub id: String,
    #[serde(default, deserialize_with = "or_default")]
    pub title: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default, rename = "release-group")]
    pub group: Option<Group>,
    #[serde(default, rename = "artist-credit", deserialize_with = "or_default")]
    pub credit: Vec<Credit>,
    #[serde(default, deserialize_with = "or_default")]
    pub media: Vec<Medium>,
}

#[derive(Debug, Deserialize)]
pub struct Group {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "primary-type")]
    pub primary: Option<String>,
    #[serde(default, rename = "secondary-types", deserialize_with = "or_default")]
    pub secondary: Vec<String>,
}

/// A medium holding the recording; a search lists its track as `track`,
/// a lookup as `tracks`.
#[derive(Debug, Deserialize)]
pub struct Medium {
    #[serde(default)]
    pub position: Option<u32>,
    /// How many tracks come before the recording's on the medium.
    #[serde(default, rename = "track-offset")]
    pub offset: Option<u32>,
    #[serde(default, rename = "track-count")]
    pub count: Option<u32>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default, alias = "tracks", deserialize_with = "or_default")]
    pub track: Vec<Track>,
}

#[derive(Debug, Deserialize)]
pub struct Track {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub position: Option<u32>,
}

#[must_use]
pub fn names(credit: &[Credit]) -> Vec<String> {
    credit
        .iter()
        .map(|c| c.name.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

/// The names of `credit` joined by its join phrases, when it credits more
/// than one; empty otherwise, the one name being [`names`]'.
#[must_use]
pub fn credited(credit: &[Credit]) -> String {
    if names(credit).len() < 2 {
        return String::new();
    }
    let joined: String = credit
        .iter()
        .map(|c| [c.name.trim(), c.joinphrase.as_str()].concat())
        .collect();
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[must_use]
pub fn artist_ids(credit: &[Credit]) -> Vec<String> {
    credit
        .iter()
        .filter_map(|c| Some(c.artist.as_ref()?.id.clone()))
        .collect()
}

/// Whether two names hold one another, by [`crate::similar::names_match`].
#[must_use]
pub fn same(a: &str, b: &str) -> bool {
    crate::similar::names_match(a, b).is_some()
}

/// A release's rank, smaller first, before its date: see the module docs.
pub type Class = (bool, bool, bool, u8);

impl Found {
    #[must_use]
    pub fn class(&self, album: Option<&str>) -> Class {
        let group = self.group.as_ref();
        let primary = match group.and_then(|g| g.primary.as_deref()) {
            Some("Album") => 0,
            Some("EP") => 1,
            Some("Single") => 2,
            _ => 3,
        };
        (
            album.is_some_and(|a| !same(&self.title, a)),
            group.is_some_and(|g| !g.secondary.is_empty()),
            self.status.as_deref() != Some("Official"),
            primary,
        )
    }

    /// Its date, or none when it states none.
    pub fn date(&self) -> Option<&str> {
        self.date
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
    }

    pub fn release(&self) -> Release {
        let medium = self.media.first();
        let track = medium.and_then(|m| m.track.first());
        let text = |s: Option<&String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        Release {
            id: self.id.clone(),
            title: self.title.trim().to_string(),
            artists: names(&self.credit),
            credit: credited(&self.credit),
            artist_ids: artist_ids(&self.credit),
            group_id: text(self.group.as_ref().and_then(|g| g.id.as_ref())),
            date: self.date().map(str::to_string),
            country: text(self.country.as_ref()),
            track: track
                .and_then(|t| t.position)
                .or_else(|| medium?.offset.map(|o| o + 1)),
            tracks: medium.and_then(|m| m.count),
            track_id: text(track.and_then(|t| t.id.as_ref())),
            disc: medium.and_then(|m| m.position),
            status: text(self.status.as_ref()),
            types: self
                .group
                .iter()
                .flat_map(|g| g.primary.iter().chain(&g.secondary))
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
            media: text(medium.and_then(|m| m.format.as_ref())),
            details: Details::default(),
        }
    }
}

/// An earlier year first, then the fuller date, as `1975-11-21` before
/// `1975`, then the earlier; one stated before none.
#[must_use]
pub fn date_key(date: Option<&str>) -> (bool, &str, std::cmp::Reverse<usize>, &str) {
    let d = date.unwrap_or_default();
    (
        date.is_none(),
        d.get(..4).unwrap_or(d),
        std::cmp::Reverse(d.len()),
        d,
    )
}

impl Recording {
    /// The other releases of `best`'s group, each once.
    fn editions(&self, best: &Found) -> Vec<Release> {
        let group = |f: &Found| f.group.as_ref().and_then(|g| g.id.clone());
        let mut seen = vec![best.id.as_str()];
        let mut editions = Vec::new();
        for f in &self.releases {
            if group(f).is_some() && group(f) == group(best) && !seen.contains(&f.id.as_str()) {
                seen.push(&f.id);
                editions.push(f.release());
            }
        }
        editions
    }

    /// Its best release for a song on `album`.
    #[must_use]
    pub fn best(&self, album: Option<&str>) -> Option<&Found> {
        self.releases.iter().min_by(|a, b| {
            a.class(album)
                .cmp(&b.class(album))
                .then_with(|| date_key(a.date()).cmp(&date_key(b.date())))
                .then_with(|| a.id.cmp(&b.id))
        })
    }

    /// What is kept of it; `None` when its ID is no MBID.
    pub fn record(&self, album: Option<&str>) -> Option<Record> {
        let best = self.best(album);
        Some(Record {
            id: Mbid::parse(&self.id)?,
            title: self.title.trim().to_string(),
            artists: names(&self.credit),
            credit: credited(&self.credit),
            artist_ids: artist_ids(&self.credit),
            comment: self.disambiguation.trim().to_string(),
            isrcs: self.isrcs.iter().map(|i| i.code().to_string()).collect(),
            length_ms: self.length,
            release: best.map(Found::release),
            editions: best.map(|b| self.editions(b)).unwrap_or_default(),
            original_date: self
                .first_release_date
                .as_ref()
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty()),
            languages: languages(&self.relations),
        })
    }
}

/// The recording a `/ws/2/recording/<id>?fmt=json` answer holds, with
/// the release it is best known from, or the one named `album`.
pub fn parse_recording(json: &str, album: Option<&str>) -> Result<Record> {
    let found: Recording = serde_json::from_str(json)?;
    match found.record(album) {
        Some(record) => Ok(record),
        None => anyhow::bail!("{:?} is no recording ID", found.id),
    }
}
