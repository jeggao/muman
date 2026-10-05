//! Tags from MusicBrainz: a song's recording found by its title, artist
//! and length, with the release it is best known from, and kept in the
//! store as `<recording id>.json`. A record offers the recording's title,
//! artists, ISRCs and MusicBrainz IDs, and the release's album, album
//! artist, track, disc, date, track total, country and IDs, every one
//! structured, ranked against the other sources' offers as any tag is
//! ([`crate::tags::from_record`]).
//!
//! `/ws/2/recording` is searched for the title and the first artist as
//! phrases, and a length within 3 s of the song's audio. A recording fits
//! when it is no video, it has that length, and its title and credited
//! artists hold the song's, compared as LRCLIB's are. Each recording's
//! releases rank by:
//!
//! 1. The song's album, when it has one.
//! 1. No secondary type: a compilation, a live album or a soundtrack
//!    ranks after a single.
//! 1. Official, over a promotion or a bootleg.
//! 1. An album, then an EP, then a single.
//!
//! Of the recordings that fit, the one whose best release ranks first
//! wins, then the one on the most releases, then the closest in length;
//! of its releases, the best ranked, then the earliest. Measured against
//! musicbrainz.org, a famous song's title and artist matched 225
//! recordings, every one scored 100 and the first 25 live bootlegs; the
//! length left 19, among them a 5.1 mix, a karaoke track and an
//! anniversary disc's recording, each with an official album too, and the
//! original's 353 releases against at most 6 of the others told it apart.
//! Another song's length left 70, still all scored alike, the original
//! 51st, so a search lists the most MusicBrainz allows, 100, and the
//! order it answers in decides nothing. A disambiguation comment ranks
//! last of all: it names a live date or a karaoke track, but the original
//! mix too. A record deleted from the store is fetched again by its ID,
//! its release picked again the same way.
//!
//! MusicBrainz allows one request a second from an address, on average,
//! and refuses every request with a 503 while a client goes faster. Every
//! request waits its turn on one [`Throttle`] for the whole run, a second
//! apart whatever the provider's `concurrency`; a 503 holds every later
//! request back 2 s, doubling, and is asked again up to three times. Each
//! request names muman by [`crate::http::user_agent`], as MusicBrainz
//! requires of every client, and asks for JSON.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Deserializer, Serialize};

use crate::clean;
use crate::http::{HttpTransport, Throttle, TransportError};
use crate::lrclib::Query;
use crate::music;

/// The record a lookup takes must be within this of the song's length: a
/// margin for the silence a video or a rip adds around the same master.
const MAX_GAP_S: f64 = 3.0;
const TIMEOUT: Duration = Duration::from_secs(20);
/// Recordings one search lists, the most MusicBrainz allows: a phrase
/// search scores a famous song's many recordings alike, its original
/// among the last as often as the first.
const LIMIT: usize = 100;
/// Times a request refused for going too fast is asked again.
const RETRIES: u32 = 3;

/// The spacing musicbrainz.org asks of a client: a second between
/// requests, and 2 s held back after a refusal, doubling.
#[must_use]
pub fn throttle() -> Throttle {
    Throttle::new(Duration::from_secs(1), Duration::from_secs(2))
}

/// What muman keeps of a recording and the release it picked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The recording's MBID.
    pub id: String,
    pub title: String,
    /// Each credited artist, as the recording credits them.
    pub artists: Vec<String>,
    /// The MBID of each credited artist.
    #[serde(default)]
    pub artist_ids: Vec<String>,
    #[serde(default)]
    pub isrcs: Vec<String>,
    #[serde(default)]
    pub length_ms: Option<u64>,
    #[serde(default)]
    pub release: Option<Release>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// The release's MBID.
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
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
}

/// A JSON `null` read as the type's default.
fn or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Deserialize)]
struct Search {
    #[serde(default, deserialize_with = "or_default")]
    recordings: Vec<Recording>,
}

/// A recording as a search or a lookup answers it.
#[derive(Debug, Deserialize)]
struct Recording {
    id: String,
    #[serde(default, deserialize_with = "or_default")]
    title: String,
    /// What tells it from recordings of the same name: a mix, a live date.
    #[serde(default, deserialize_with = "or_default")]
    disambiguation: String,
    #[serde(default)]
    length: Option<u64>,
    #[serde(default, deserialize_with = "or_default")]
    video: bool,
    #[serde(default, deserialize_with = "or_default")]
    score: u32,
    #[serde(default, rename = "artist-credit", deserialize_with = "or_default")]
    credit: Vec<Credit>,
    #[serde(default, deserialize_with = "or_default")]
    isrcs: Vec<Isrc>,
    #[serde(default, deserialize_with = "or_default")]
    releases: Vec<Found>,
}

/// An ISRC: a search lists each as `{"id": …}`, a lookup as text.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Isrc {
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
struct Credit {
    /// The name as credited here, which may differ from the artist's own.
    #[serde(default, deserialize_with = "or_default")]
    name: String,
    #[serde(default)]
    artist: Option<Artist>,
}

#[derive(Debug, Deserialize)]
struct Artist {
    id: String,
}

#[derive(Debug, Deserialize)]
struct Found {
    id: String,
    #[serde(default, deserialize_with = "or_default")]
    title: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default, rename = "release-group")]
    group: Option<Group>,
    #[serde(default, rename = "artist-credit", deserialize_with = "or_default")]
    credit: Vec<Credit>,
    #[serde(default, deserialize_with = "or_default")]
    media: Vec<Medium>,
}

#[derive(Debug, Deserialize)]
struct Group {
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "primary-type")]
    primary: Option<String>,
    #[serde(default, rename = "secondary-types", deserialize_with = "or_default")]
    secondary: Vec<String>,
}

/// A medium holding the recording; a search lists its track as `track`,
/// a lookup as `tracks`.
#[derive(Debug, Deserialize)]
struct Medium {
    #[serde(default)]
    position: Option<u32>,
    /// How many tracks come before the recording's on the medium.
    #[serde(default, rename = "track-offset")]
    offset: Option<u32>,
    #[serde(default, rename = "track-count")]
    count: Option<u32>,
    #[serde(default, alias = "tracks", deserialize_with = "or_default")]
    track: Vec<Track>,
}

#[derive(Debug, Deserialize)]
struct Track {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    position: Option<u32>,
}

fn names(credit: &[Credit]) -> Vec<String> {
    credit
        .iter()
        .map(|c| c.name.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

fn artist_ids(credit: &[Credit]) -> Vec<String> {
    credit
        .iter()
        .filter_map(|c| Some(c.artist.as_ref()?.id.clone()))
        .collect()
}

/// Whether two names hold one another, by letters and digits, without
/// featured artists.
fn same(a: &str, b: &str) -> bool {
    let (a, b) = (
        music::normalize(clean::without_credits(a)),
        music::normalize(clean::without_credits(b)),
    );
    !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
}

/// The artist a search asks for: the first of the song's, which are
/// joined with `, `.
fn first_artist(artist: &str) -> &str {
    artist.split(", ").next().unwrap_or(artist).trim()
}

/// A release's rank, smaller first, before its date: see the module docs.
type Class = (bool, bool, bool, u8);

impl Found {
    fn class(&self, album: Option<&str>) -> Class {
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
    fn date(&self) -> Option<&str> {
        self.date
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
    }

    fn release(&self) -> Release {
        let medium = self.media.first();
        let track = medium.and_then(|m| m.track.first());
        let text = |s: Option<&String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        Release {
            id: self.id.clone(),
            title: self.title.trim().to_string(),
            artists: names(&self.credit),
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
        }
    }
}

/// An earlier year first, then the fuller date, as `1975-11-21` before
/// `1975`, then the earlier; one stated before none.
fn date_key(date: Option<&str>) -> (bool, &str, std::cmp::Reverse<usize>, &str) {
    let d = date.unwrap_or_default();
    (
        date.is_none(),
        d.get(..4).unwrap_or(d),
        std::cmp::Reverse(d.len()),
        d,
    )
}

impl Recording {
    /// The gap to the song's length in seconds, when the recording has one.
    fn gap(&self, seconds: f64) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.length.map(|ms| (ms as f64 / 1000.0 - seconds).abs())
    }

    fn fits(&self, q: &Query) -> bool {
        let credited = names(&self.credit).join(" ");
        !self.video
            && self.gap(q.seconds).is_some_and(|g| g <= MAX_GAP_S)
            && same(&self.title, &q.title)
            && same(&credited, first_artist(&q.artist))
    }

    /// Its best release for a song on `album`.
    fn best(&self, album: Option<&str>) -> Option<&Found> {
        self.releases.iter().min_by(|a, b| {
            a.class(album)
                .cmp(&b.class(album))
                .then_with(|| date_key(a.date()).cmp(&date_key(b.date())))
                .then_with(|| a.id.cmp(&b.id))
        })
    }

    /// Its rank among the recordings that fit, smaller first: by its best
    /// release, a recording on none last, then on the most releases, the
    /// gap in whole seconds, the date, without a comment, the search's
    /// score.
    fn rank(&self, album: Option<&str>, seconds: f64) -> impl Ord + '_ {
        let best = self.best(album);
        let class = best.map(|f| f.class(album));
        (
            class.is_none(),
            class,
            std::cmp::Reverse(self.releases.len()),
            self.gap(seconds).map_or(i64::MAX, whole_seconds),
            date_key(best.and_then(Found::date)),
            !self.disambiguation.trim().is_empty(),
            std::cmp::Reverse(self.score),
            self.id.as_str(),
        )
    }

    fn record(&self, album: Option<&str>) -> Record {
        Record {
            id: self.id.clone(),
            title: self.title.trim().to_string(),
            artists: names(&self.credit),
            artist_ids: artist_ids(&self.credit),
            isrcs: self.isrcs.iter().map(|i| i.code().to_string()).collect(),
            length_ms: self.length,
            release: self.best(album).map(Found::release),
        }
    }
}

/// One MusicBrainz server, asked through `transport` at the pace of
/// `throttle`.
pub struct Client<'a> {
    pub base: &'a str,
    pub transport: &'a (dyn HttpTransport + Sync),
    pub throttle: &'a Throttle,
}

impl std::fmt::Debug for Client<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Client<'_> {
    /// GET `path` on its turn, asking again after a refusal for going too
    /// fast; `None` for a 404.
    fn get(&self, path: &str) -> Result<Option<String>> {
        let url = format!("{}{path}", self.base.trim_end_matches('/'));
        let agent = crate::http::user_agent();
        let headers = [
            ("User-Agent", agent.as_str()),
            ("Accept", "application/json"),
        ];
        let mut refused = 0;
        loop {
            self.throttle.wait();
            match self.transport.get_json(&url, &headers, TIMEOUT) {
                Ok(body) => return Ok(Some(body)),
                Err(TransportError::Status { code: 404, .. }) => return Ok(None),
                Err(TransportError::Status { code: 503, .. }) if refused < RETRIES => {
                    refused += 1;
                    self.throttle.refused(refused);
                }
                Err(e) => return Err(anyhow!("{url}: {e}")),
            }
        }
    }

    /// The record of one recording, to fetch a kept one again.
    pub fn by_id(&self, id: &str) -> Result<Option<Record>> {
        let path = format!(
            "/ws/2/recording/{}?inc=artist-credits+releases+release-groups+media+isrcs&fmt=json",
            music::percent_encode(id)
        );
        self.get(&path)?
            .map(|b| {
                serde_json::from_str::<Recording>(&b)
                    .map(|r| r.record(None))
                    .context("reading MusicBrainz's answer")
            })
            .transpose()
    }

    /// The song's recording and the release it is best known from, or
    /// `None` when no recording fits.
    pub fn find(&self, q: &Query) -> Result<Option<Record>> {
        let ms = |s: f64| whole_seconds(s * 1000.0).max(0);
        let query = format!(
            "recording:{} AND artist:{} AND dur:[{} TO {}]",
            phrase(&q.title),
            phrase(first_artist(&q.artist)),
            ms(q.seconds - MAX_GAP_S),
            ms(q.seconds + MAX_GAP_S)
        );
        let path = format!(
            "/ws/2/recording?query={}&limit={LIMIT}&fmt=json",
            music::percent_encode(&query)
        );
        let Some(body) = self.get(&path)? else {
            return Ok(None);
        };
        let found: Search = serde_json::from_str(&body).context("reading MusicBrainz's answer")?;
        let album = q.album.as_deref();
        let best = found
            .recordings
            .iter()
            .filter(|r| r.fits(q))
            .min_by_key(|r| r.rank(album, q.seconds));
        Ok(best.map(|r| r.record(album)))
    }
}

#[allow(clippy::cast_possible_truncation)]
fn whole_seconds(s: f64) -> i64 {
    s.round() as i64
}

/// `text` as a quoted Lucene phrase.
fn phrase(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Whether `s` has the shape of an MBID: 32 hex digits in groups of 8, 4,
/// 4, 4 and 12.
#[must_use]
pub fn is_mbid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

/// Where a record is kept in the store.
#[must_use]
pub fn path_of(folder: &Path, id: &str) -> PathBuf {
    folder.join(format!("{id}.json"))
}

/// Keep a record in `folder`.
pub fn keep(folder: &Path, record: &Record) -> Result<PathBuf> {
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let json = serde_json::to_vec_pretty(record).context("writing a MusicBrainz record")?;
    crate::atomic::write(folder, &format!("{}.json", record.id), &json)?;
    Ok(path_of(folder, &record.id))
}

/// Read a kept record.
pub fn read(path: &Path) -> Result<Record> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))
}

#[cfg(test)]
mod tests;
