//! Tags from MusicBrainz: a song's recording found by its title, artist
//! and length, with the release it is best known from, and kept in the
//! store as `<recording id>.json`. A record offers the recording's
//! title, artists, ISRCs, MusicBrainz IDs, first release's date and its
//! works' lyrics languages, the language a song is sung in, and the
//! release's album, album artist, track, disc, date, track total,
//! country, IDs, status, types and medium, every one structured, ranked
//! against the other sources' offers as any tag is
//! ([`muman_media::tags::from_record`]). A search answers none of the
//! release's labels, catalogue numbers, barcode, ASIN or script, nor
//! its release group's first release, the original date as Picard
//! writes it, so the release is looked up once more for them, once a
//! run for each release, so an album's songs ask once; that lookup
//! failing fails the record's, which is made again rather than kept
//! without them. A search answers no works either, so the recording it
//! finds is looked up once more for them, a request more for each song;
//! a lookup by ID asks for them with the rest. A record kept before has
//! these from when it is next fetched again. A recording's credits, its
//! performers, producer and the like, are not asked for.
//!
//! The lookups of a recording and of its release ask too for what a
//! song's names and genres are to be chosen by, kept on the record as
//! [`crate::record`] says, which no pick reads: the recording's, by
//! [`RECORDING_INC`], its genres and its credited artists with their
//! aliases and genres; the release's, by [`RELEASE_INC`], its genres,
//! its release group's, and its credited artists'. With
//! `artist-credits`, `aliases` and `genres` answer them for every artist
//! a credit names, as well as for the entity looked up, so no request is
//! added. A lookup by ID answers at most 25 releases, each crediting its
//! artists with their aliases again: measured on musicbrainz.org, a
//! recording on 16 releases answered 37 kB without them and 266 kB with
//! them, still one request.
//!
//! A record keeps the artists credited together joined as the credit
//! joins them, by its join phrases, as `Marlo Venn & The Glass Orchards`,
//! for the recording and for its release; the recording's comment, which
//! names a mix; and the other releases of its release's group that hold
//! it, the album's other editions, so the songs of one album can be put
//! on one of them (`album`). An edition keeps what a search
//! answers of it and the group's first release, not what only a lookup
//! of the release answers: that is one request a release, and an album
//! has hundreds.
//!
//! `/ws/2/recording` is searched for the title and the first artist as
//! phrases, and a length within 3 s of the song's audio. A recording fits
//! when it is no video, it has that length, and its title and credited
//! artists, or their credit as joined, hold the song's, compared as
//! LRCLIB's are. Each recording's
//! releases rank by:
//!
//! 1. The song's album, when it has one.
//! 1. No secondary type: a compilation, a live album or a soundtrack
//!    ranks after a single.
//! 1. Official, over a promotion or a bootleg.
//! 1. An album, then an EP, then a single.
//!
//! Of the recordings that fit, one titled exactly the song's title wins
//! over one whose title only holds it, as `Purple Rain` holds `Rain`;
//! then one whose comment does not name the other mix than the song's
//! audio is, by the word `mono` or `stereo` ([`other_mix`]): a mono and
//! a stereo rip of one album fingerprint and time alike, and MusicBrainz
//! tells their recordings apart as `original mono album mix` and
//! `original stereo album mix`; then the one whose best release ranks
//! first, then the one on the most releases, then the closest in length;
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
//! A recording's ID names its file in the store, so every ID that comes
//! in, from a search, a lookup by ID, AcoustID or a song's tags, becomes
//! an [`Mbid`] where it is read or is dropped there: a recording or a
//! release group without one is no candidate, and an answer by ID whose
//! ID is no MBID is an error.
//!
//! MusicBrainz allows one request a second from an address, on average,
//! and refuses every request with a 503 while a client goes faster. Every
//! request waits its turn on one [`Throttle`] for the whole run, a second
//! apart whatever the provider's `concurrency`; a refusal holds every
//! later request back 2 s, doubling, and is asked again, as
//! [`crate::http::Service`] does for every service. Each
//! request names muman by [`crate::http::user_agent`], as MusicBrainz
//! requires of every client, and asks for JSON.

use crate::units;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::http::{HttpTransport, Service, Throttle};
use crate::lrclib::Query;

/// The record a lookup takes must be within this of the song's length: a
/// margin for the silence a video or a rip adds around the same master.
pub const MAX_GAP_S: f64 = 3.0;
const TIMEOUT: Duration = Duration::from_secs(20);
/// Recordings one search lists, the most MusicBrainz allows: a phrase
/// search scores a famous song's many recordings alike, its original
/// among the last as often as the first.
const LIMIT: usize = 100;
/// What a lookup of a recording asks for beyond itself, whatever else it
/// asks: its works, its genres, and its artists with their aliases and
/// genres.
const RECORDING_INC: &str = "work-rels+artist-credits+aliases+genres";
/// What a lookup of a release asks for: its labels, its release group,
/// their genres, and its artists with their aliases and genres.
const RELEASE_INC: &str = "labels+release-groups+artist-credits+aliases+genres";

/// The spacing musicbrainz.org asks of a client: a second between
/// requests, and 2 s held back after a refusal, doubling.
#[must_use]
pub fn throttle() -> Throttle {
    Throttle::new(Duration::from_secs(1), Duration::from_secs(2))
}

/// The details of each release looked up in a run, so the songs of one
/// album ask for its release once.
pub type Releases = std::sync::Mutex<std::collections::BTreeMap<String, Option<Details>>>;

/// A release as [`RELEASE_INC`] answers it.
#[derive(Debug, Deserialize)]
struct Looked {
    #[serde(default, rename = "release-group")]
    group: Option<FirstReleased>,
    #[serde(default, rename = "artist-credit", deserialize_with = "or_default")]
    credit: Vec<Credit>,
    #[serde(default, deserialize_with = "or_default")]
    genres: Vec<Genre>,
    #[serde(default)]
    barcode: Option<String>,
    #[serde(default)]
    asin: Option<String>,
    #[serde(default, rename = "text-representation")]
    text: Option<Representation>,
    #[serde(default, rename = "label-info", deserialize_with = "or_default")]
    labels: Vec<LabelInfo>,
}

#[derive(Debug, Deserialize)]
struct FirstReleased {
    #[serde(default, rename = "first-release-date")]
    date: Option<String>,
    #[serde(default, deserialize_with = "or_default")]
    genres: Vec<Genre>,
}

#[derive(Debug, Deserialize)]
struct Representation {
    #[serde(default)]
    script: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LabelInfo {
    #[serde(default, rename = "catalog-number")]
    catalog: Option<String>,
    #[serde(default)]
    label: Option<Named>,
}

#[derive(Debug, Deserialize)]
struct Named {
    #[serde(default, deserialize_with = "or_default")]
    name: String,
}

impl Looked {
    fn details(self) -> Details {
        let text = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let (first, group_genres) = self
            .group
            .map_or((None, Vec::new()), |g| (g.date, genres(&g.genres)));
        let mut details = Details {
            barcode: text(self.barcode),
            asin: text(self.asin),
            script: text(self.text.and_then(|t| t.script)),
            original_date: text(first),
            credited_artists: credited_artists(&self.credit),
            genres: genres(&self.genres),
            group_genres,
            ..Details::default()
        };
        for info in self.labels {
            if let Some(name) = text(info.label.map(|l| l.name))
                && !details.labels.contains(&name)
            {
                details.labels.push(name);
            }
            if let Some(number) = text(info.catalog)
                && !details.catalog_numbers.contains(&number)
            {
                details.catalog_numbers.push(number);
            }
        }
        details
    }
}

#[derive(Debug, Deserialize)]
struct Search {
    #[serde(default, deserialize_with = "or_default")]
    recordings: Vec<Recording>,
}

/// The artist a search asks for: the first of the song's, which are
/// joined with `, `.
pub(crate) fn first_artist(artist: &str) -> &str {
    artist.split(", ").next().unwrap_or(artist).trim()
}

/// Whether a recording's `comment` names a mix the song's audio is not,
/// as `original mono album mix` for audio in stereo: `mono` says whether
/// it is mono, where known.
#[must_use]
pub fn other_mix(comment: &str, mono: Option<bool>) -> bool {
    let lower = comment.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let says = |w: &str| words.contains(&w);
    let named = match (says("mono"), says("stereo")) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    };
    named.is_some_and(|m| mono.is_some_and(|mono| mono != m))
}

/// The gap to the song's length in seconds, when the recording has one.
fn gap(r: &Recording, seconds: f64) -> Option<f64> {
    r.length
        .map(|ms| (units::seconds_of_ms(ms as f64) - seconds).abs())
}

fn fits(r: &Recording, q: &Query) -> bool {
    let artist = first_artist(&q.artist);
    is_mbid(&r.id)
        && !r.video
        && gap(r, q.seconds).is_some_and(|g| g <= MAX_GAP_S)
        && same(&r.title, &q.title)
        && (same(&names(&r.credit).join(" "), artist) || same(&credited(&r.credit), artist))
}

/// Its rank among the recordings that fit, smaller first: titled the
/// song's title exactly, then without a comment naming the other mix
/// than the song's audio is, then by its best release, a recording on
/// none last, then on the most releases, the gap in whole seconds, the
/// date, without a comment, the search's score.
fn rank<'a>(r: &'a Recording, q: &Query) -> impl Ord + 'a {
    let album = q.album.as_deref();
    let best = r.best(album);
    let class = best.map(|f| f.class(album));
    (
        std::cmp::Reverse(crate::similar::names_match(&r.title, &q.title)),
        other_mix(&r.disambiguation, q.mono),
        class.is_none(),
        class,
        std::cmp::Reverse(r.releases.len()),
        gap(r, q.seconds).map_or(i64::MAX, whole_seconds),
        date_key(best.and_then(Found::date)),
        !r.disambiguation.trim().is_empty(),
        std::cmp::Reverse(r.score),
        r.id.as_str(),
    )
}

/// One MusicBrainz server, asked through `transport` at the pace of
/// `throttle`.
pub struct Client<'a> {
    pub base: &'a str,
    pub transport: &'a (dyn HttpTransport + Sync),
    pub throttle: &'a Throttle,
    pub releases: &'a Releases,
}

impl std::fmt::Debug for Client<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Client<'_> {
    /// GET `path` on its turn; `None` for a 404.
    fn get(&self, path: &str) -> Result<Option<String>> {
        let url = format!("{}{path}", self.base.trim_end_matches('/'));
        let agent = crate::http::user_agent();
        let headers = [
            ("User-Agent", agent.as_str()),
            ("Accept", "application/json"),
        ];
        Service {
            name: "MusicBrainz",
            transport: self.transport,
            throttle: self.throttle,
        }
        .get_text(&url, &headers, TIMEOUT)
    }

    /// The record of one recording, to fetch a kept one again, its
    /// release picked as a search picks it for a song on `album`.
    pub fn by_id(&self, id: &Mbid, album: Option<&str>) -> Result<Option<Record>> {
        let path = format!(
            "/ws/2/recording/{}?inc={RECORDING_INC}+releases+release-groups+media+isrcs&fmt=json",
            crate::http::percent_encode(id.as_str())
        );
        let Some(body) = self.get(&path)? else {
            return Ok(None);
        };
        let mut record = parse_recording(&body, album).context("reading MusicBrainz's answer")?;
        self.detail(&mut record)?;
        Ok(Some(record))
    }

    /// `record`'s release given the details only a lookup of it answers,
    /// once a run for each release. A release MusicBrainz no longer has
    /// has none; a lookup that fails fails the record's, so it is made
    /// again rather than kept without them.
    fn detail(&self, record: &mut Record) -> Result<()> {
        let Some(release) = record.release.as_mut() else {
            return Ok(());
        };
        let held = self
            .releases
            .lock()
            .ok()
            .and_then(|r| r.get(&release.id).cloned());
        let details = if let Some(details) = held {
            details
        } else {
            let path = format!(
                "/ws/2/release/{}?inc={RELEASE_INC}&fmt=json",
                crate::http::percent_encode(&release.id)
            );
            let looked = match self.get(&path)? {
                Some(body) => Some(
                    serde_json::from_str::<Looked>(&body)
                        .context("reading MusicBrainz's answer for a release")?
                        .details(),
                ),
                None => None,
            };
            if let Ok(mut known) = self.releases.lock() {
                known.insert(release.id.clone(), looked.clone());
            }
            looked
        };
        release.details = details.unwrap_or_default();
        let first = release.details.original_date.clone();
        for edition in &mut record.editions {
            edition.details.original_date.clone_from(&first);
        }
        Ok(())
    }

    /// The song's recording and the release it is best known from, or
    /// `None` when no recording fits.
    pub fn find(&self, q: &Query) -> Result<Option<Record>> {
        let ms = |s: f64| whole_seconds(units::ms_of_seconds(s)).max(0);
        let query = format!(
            "recording:{} AND artist:{} AND dur:[{} TO {}]",
            phrase(&q.title),
            phrase(first_artist(&q.artist)),
            ms(q.seconds - MAX_GAP_S),
            ms(q.seconds + MAX_GAP_S)
        );
        let path = format!(
            "/ws/2/recording?query={}&limit={LIMIT}&fmt=json",
            crate::http::percent_encode(&query)
        );
        let Some(body) = self.get(&path)? else {
            return Ok(None);
        };
        let found: Search = serde_json::from_str(&body).context("reading MusicBrainz's answer")?;
        let album = q.album.as_deref();
        let best = found
            .recordings
            .iter()
            .filter(|r| fits(r, q))
            .min_by_key(|r| rank(r, q));
        let mut record = best.and_then(|r| r.record(album));
        if let Some(record) = record.as_mut() {
            self.detail(record)?;
            self.recording(record)?;
        }
        Ok(record)
    }

    /// `record` given what a search does not answer of its recording: its
    /// works' lyrics languages, its genres and its artists' aliases. A
    /// lookup that fails fails the record's, as [`Self::detail`]'s.
    fn recording(&self, record: &mut Record) -> Result<()> {
        let path = format!(
            "/ws/2/recording/{}?inc={RECORDING_INC}&fmt=json",
            crate::http::percent_encode(record.id.as_str())
        );
        if let Some(body) = self.get(&path)? {
            let looked: Recording = serde_json::from_str(&body)
                .context("reading MusicBrainz's answer for a recording")?;
            record.languages = languages(&looked.relations);
            record.credited_artists = credited_artists(&looked.credit);
            record.genres = genres(&looked.genres);
        }
        Ok(())
    }
}

/// Release groups a search lists.
#[derive(Debug, Deserialize)]
struct Groups {
    #[serde(default, rename = "release-groups", deserialize_with = "or_default")]
    groups: Vec<Grouped>,
}

/// A release group as a search lists it.
#[derive(Debug, Deserialize)]
struct Grouped {
    id: String,
    #[serde(default, deserialize_with = "or_default")]
    title: String,
    #[serde(default, rename = "primary-type")]
    primary: Option<String>,
    #[serde(default, rename = "secondary-types", deserialize_with = "or_default")]
    secondary: Vec<String>,
    #[serde(default, rename = "artist-credit", deserialize_with = "or_default")]
    credit: Vec<Credit>,
    #[serde(default)]
    score: u32,
}

impl Client<'_> {
    /// The release group of an album by its title and its artist: of
    /// those whose names hold both, one titled exactly the album, then an
    /// album with no secondary type, then the best scored.
    pub fn release_group(&self, album: &str, artist: &str) -> Result<Option<Mbid>> {
        let query = format!(
            "releasegroup:{} AND artist:{}",
            phrase(album),
            phrase(first_artist(artist))
        );
        let path = format!(
            "/ws/2/release-group?query={}&limit=25&fmt=json",
            crate::http::percent_encode(&query)
        );
        let Some(body) = self.get(&path)? else {
            return Ok(None);
        };
        let found: Groups = serde_json::from_str(&body).context("reading MusicBrainz's answer")?;
        let best = found
            .groups
            .iter()
            .filter(|g| is_mbid(&g.id))
            .filter(|g| same(&names(&g.credit).join(" "), first_artist(artist)))
            .filter_map(|g| Some((crate::similar::names_match(&g.title, album)?, g)))
            .min_by_key(|(exact, g)| {
                (
                    std::cmp::Reverse(*exact),
                    g.primary.as_deref() != Some("Album"),
                    !g.secondary.is_empty(),
                    std::cmp::Reverse(g.score),
                )
            });
        Ok(best.and_then(|(_, g)| Mbid::parse(&g.id)))
    }
}

fn whole_seconds(s: f64) -> i64 {
    s.round() as i64
}

/// `text` as a quoted Lucene phrase.
fn phrase(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

use crate::record::{
    Credit, Found, Genre, Recording, credited, credited_artists, date_key, genres, languages,
    names, or_default, parse_recording, same,
};
pub use crate::record::{Details, Mbid, Record, Release, keep, path_of, read};
pub use crate::source::is_mbid;

#[cfg(test)]
mod tests;
