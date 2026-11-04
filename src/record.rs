//! What muman keeps of a MusicBrainz recording and the release it
//! picked, as `<recording id>.json` in the store: the record
//! `musicbrainz.rs` makes from a search or a lookup, and the tags, facts
//! and tag files read. A record's ID names its file, so an [`Mbid`] is
//! checked where it comes in and never names another path.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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
