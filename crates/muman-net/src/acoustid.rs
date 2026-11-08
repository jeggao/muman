//! A song's MusicBrainz recording found by its fingerprint on AcoustID,
//! which maps Chromaprint prints to the recordings they were submitted
//! with. The `acoustid` provider keeps no record of its own: the recording
//! it names is fetched from MusicBrainz by its ID and kept as any
//! `musicbrainz:` record is ([`crate::musicbrainz`]), so a song tagged
//! wrongly or not at all still finds its tags.
//!
//! `/v2/lookup` is asked with the print of the song's audio as
//! [`crate::fingerprint::Print::encoded`] writes it, the audio's length in
//! whole seconds, and muman's application key, or the one
//! `[providers.acoustid] key` sets, for the `meta` values [`META`] names,
//! as AcoustID's web service documents them. A result fits when its
//! score is at least [`MIN_SCORE`]; of its recordings, those whose length
//! is within [`LOOSE_GAP_S`] of the song's, or that state none but were
//! submitted with at least [`MIN_UNTIMED_SOURCES`] prints, rank by:
//!
//! 1. Titled as the song is, when it has a title.
//! 1. Its length: within [`crate::musicbrainz::MAX_GAP_S`] of the song's,
//!    then within [`LOOSE_GAP_S`], then stating none.
//! 1. Crediting the song's first artist, when it has one.
//! 1. The most sources, the prints submitted with it: a recording's
//!    original over a duplicate MusicBrainz has not merged yet.
//! 1. On a release group with no secondary type, as MusicBrainz's search
//!    ranks: a compilation's or a live album's last.
//! 1. The result's score, then the ID.
//!
//! A recording that states no length is one nothing checks against the
//! song, and AcoustID links prints to such stubs: measured on a library of
//! about two thousand songs, a stub release whose thirteen recordings
//! state no length took every song of its album it had prints of, the
//! real recordings standing 3 to 7 s off a 96 kHz rip's longer tail, and a
//! 29 s excerpt of a live track was named the studio recording by the one
//! print submitted with it. So a stated length within a looser margin
//! ranks first, and a stub one person's print alone names is no match.
//!
//! A recording whose title and first artist both disagree with the
//! song's, as [`crate::similar::titles_agree`] and
//! [`crate::similar::credits_agree`] read them, never fits: a print
//! submitted with another song names that song, whatever its score, and
//! a cover act's name holding the artist's is not the artist's. A title
//! or artist no rule compares with the song's, as kanji with Latin
//! letters, disagrees with nothing.
//!
//! [`Client::find`] answers the recordings that fit, best first, at most
//! [`KEPT`]: the lookup takes the first whose MusicBrainz comment does not
//! name the other mix than the song's audio is
//! ([`crate::musicbrainz::other_mix`]), as a mono rip whose print matches
//! the stereo recording too, and keeps them all with its row in the state
//! ([`Candidate`]), so an album's release can be chosen among every
//! recording its songs' prints name, not only the one taken; no pick
//! reads them. Each keeps its title, artists, length, score and prints,
//! and the releases it is on, each with its group, title, first date and
//! country, the group's types, and the disc and track it is on, at most
//! [`KEPT_RELEASES`]: a famous recording is on hundreds, nearly all
//! compilations, so those titled as the song's album come first, then
//! those whose group has no secondary type, then the earliest.
//!
//! AcoustID allows three requests a second from a client; every request
//! waits its turn on one [`Throttle`], and a refusal holds every later
//! request back 2 s, doubling, as [`crate::http::Service`] does for every
//! service. An error AcoustID answers, such as an unknown key, is told by
//! its message, never by the request, whose print runs to a few kilobytes.

use std::time::Duration;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::fingerprint::Print;
use crate::http::{HttpTransport, Service, Throttle, TransportError};
use crate::musicbrainz::{MAX_GAP_S, Mbid, first_artist, is_mbid};
use crate::record::date_key;

/// muman's application key, which AcoustID asks every client to send.
pub const KEY: &str = "EHCiWFI8Ce";
/// The least score a result is taken at, as beets takes them: below it, a
/// print shares too little with the song's to name its recording.
pub const MIN_SCORE: f64 = 0.5;
/// How far a recording's stated length may stand from the song's at
/// most: a 96 kHz rip of an album measured 4.8 s longer than its release.
pub const LOOSE_GAP_S: f64 = 10.0;
/// The prints a recording that states no length must have been submitted
/// with: one names a stub as surely as a mistake.
pub const MIN_UNTIMED_SOURCES: u32 = 2;
/// The recordings a lookup keeps at most, best first.
pub const KEPT: usize = 8;
/// The releases kept of each recording at most, as many as MusicBrainz
/// lists of a recording looked up by its ID.
pub const KEPT_RELEASES: usize = 25;
/// What a lookup asks AcoustID to answer of each recording: its release
/// groups, their releases, the tracks it is on, and the prints submitted
/// with it.
const META: &str = "recordings+releasegroups+releases+tracks+sources";
const TIMEOUT: Duration = Duration::from_secs(20);

/// The spacing AcoustID asks of a client: three requests a second, and
/// 2 s held back after a refusal, doubling.
#[must_use]
pub fn throttle() -> Throttle {
    Throttle::new(Duration::from_millis(334), Duration::from_secs(2))
}

/// What a lookup asks: the song's print and length, and the names it
/// prefers a recording by.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub print: Print,
    pub seconds: f64,
    pub title: Option<String>,
    /// The song's artists, joined with `, `.
    pub artist: Option<String>,
    /// The album the MusicBrainz release is picked for.
    pub album: Option<String>,
    /// Whether the song's audio is mono, where known.
    pub mono: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct Answer {
    #[serde(default)]
    status: String,
    #[serde(default)]
    results: Vec<Found>,
    #[serde(default)]
    error: Option<Failure>,
}

#[derive(Debug, Deserialize)]
struct Failure {
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct Found {
    #[serde(default)]
    score: f64,
    #[serde(default)]
    recordings: Vec<Recording>,
}

#[derive(Debug, Deserialize)]
struct Recording {
    id: String,
    #[serde(default)]
    title: String,
    /// Seconds.
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    artists: Vec<Artist>,
    #[serde(default)]
    releasegroups: Vec<Group>,
    /// Prints submitted with it.
    #[serde(default)]
    sources: u32,
}

#[derive(Debug, Deserialize)]
struct Artist {
    #[serde(default)]
    name: String,
    #[serde(default)]
    joinphrase: String,
}

#[derive(Debug, Deserialize)]
struct Group {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "type")]
    primary: Option<String>,
    #[serde(default)]
    secondarytypes: Vec<String>,
    #[serde(default)]
    releases: Vec<Released>,
}

/// A release as `meta=releases+tracks` lists it, its first release event
/// on it.
#[derive(Debug, Deserialize)]
struct Released {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    date: Option<Date>,
    #[serde(default)]
    medium_count: Option<u32>,
    /// The mediums holding the recording, each with its tracks of it.
    #[serde(default)]
    mediums: Vec<Medium>,
}

#[derive(Debug, Deserialize)]
struct Date {
    #[serde(default)]
    year: Option<u32>,
    #[serde(default)]
    month: Option<u32>,
    #[serde(default)]
    day: Option<u32>,
}

impl Date {
    /// `YYYY`, `YYYY-MM` or `YYYY-MM-DD`, as a MusicBrainz date is written.
    fn text(&self) -> Option<String> {
        let year = self.year.filter(|y| *y > 0)?;
        Some(
            match (self.month.filter(|m| *m > 0), self.day.filter(|d| *d > 0)) {
                (Some(m), Some(d)) => format!("{year:04}-{m:02}-{d:02}"),
                (Some(m), None) => format!("{year:04}-{m:02}"),
                _ => format!("{year:04}"),
            },
        )
    }
}

#[derive(Debug, Deserialize)]
struct Medium {
    #[serde(default)]
    position: Option<u32>,
    #[serde(default)]
    track_count: Option<u32>,
    #[serde(default)]
    tracks: Vec<Track>,
}

#[derive(Debug, Deserialize)]
struct Track {
    #[serde(default)]
    position: Option<u32>,
}

/// A recording AcoustID named for a song's print, as its lookup keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: Mbid,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub length_ms: Option<u64>,
    /// The result's score, in thousandths.
    #[serde(default)]
    pub score: u32,
    /// Prints submitted with it.
    #[serde(default)]
    pub sources: u32,
    /// The releases it is on, at most [`KEPT_RELEASES`], best first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub releases: Vec<CandidateRelease>,
}

/// A release a [`Candidate`] is on, and where on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateRelease {
    /// The release's MBID.
    pub id: String,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub title: String,
    /// `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// The release group's primary type and its secondary ones, as
    /// `Album`, `Compilation`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disc: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<u32>,
    /// The tracks on the recording's disc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracks: Option<u32>,
}

impl Recording {
    fn fits(&self, seconds: f64) -> bool {
        is_mbid(&self.id)
            && match self.duration {
                Some(d) => (d - seconds).abs() <= LOOSE_GAP_S,
                None => self.sources >= MIN_UNTIMED_SOURCES,
            }
    }

    /// Whether its title and first artist both disagree with the song's.
    fn another_song(&self, q: &Query) -> bool {
        let (Some(title), Some(artist)) = (q.title.as_deref(), q.artist.as_deref()) else {
            return false;
        };
        let names: Vec<&str> = self.artists.iter().map(|a| a.name.as_str()).collect();
        let titled = crate::similar::titles_agree(&[self.title.as_str()], &[vec![title]], false);
        titled == Some(false)
            && crate::similar::credits_agree(&names, &[first_artist(artist)]) == Some(false)
    }

    /// How sure its length is, smaller first: within [`MAX_GAP_S`] of
    /// `seconds`, then further, then stating none.
    fn timed(&self, seconds: f64) -> u8 {
        match self.duration {
            Some(d) if (d - seconds).abs() <= MAX_GAP_S => 0,
            Some(_) => 1,
            None => 2,
        }
    }

    /// Its rank, smaller first: see the module docs.
    fn rank<'a>(&'a self, q: &Query, score: f64) -> impl Ord + 'a {
        let title = q
            .title
            .as_deref()
            .and_then(|t| crate::similar::names_match(&self.title, t));
        let names = self
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let joined: String = self
            .artists
            .iter()
            .map(|a| [a.name.as_str(), a.joinphrase.as_str()].concat())
            .collect();
        let artist = q.artist.as_deref().and_then(|a| {
            crate::similar::names_match(&names, first_artist(a))
                .or_else(|| crate::similar::names_match(&joined, first_artist(a)))
        });
        (
            std::cmp::Reverse(title),
            self.timed(q.seconds),
            std::cmp::Reverse(artist.is_some()),
            std::cmp::Reverse(self.sources),
            !self
                .releasegroups
                .iter()
                .any(|g| g.secondarytypes.is_empty()),
            std::cmp::Reverse(thousandths(score)),
            self.id.as_str(),
        )
    }
}

/// A score, which AcoustID gives from 0 to 1, as a whole number to order.
fn thousandths(score: f64) -> i64 {
    (score * 1000.0).round() as i64
}

impl Recording {
    /// What a lookup keeps of it, given its result's `score`; `None` when
    /// its ID is no MBID.
    fn candidate(&self, q: &Query, score: f64) -> Option<Candidate> {
        let text = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        let mut releases: Vec<(bool, bool, CandidateRelease)> = Vec::new();
        for group in &self.releasegroups {
            let types: Vec<String> = group
                .primary
                .iter()
                .chain(&group.secondarytypes)
                .filter_map(|t| text(t))
                .collect();
            for r in group.releases.iter().filter(|r| is_mbid(&r.id)) {
                let medium = r
                    .mediums
                    .iter()
                    .filter(|m| !m.tracks.is_empty())
                    .min_by_key(|m| m.position);
                let release = CandidateRelease {
                    id: r.id.clone(),
                    group_id: Some(group.id.clone()).filter(|g| is_mbid(g)),
                    title: r.title.trim().to_string(),
                    date: r.date.as_ref().and_then(Date::text),
                    country: r.country.as_deref().and_then(text),
                    types: types.clone(),
                    disc: medium.and_then(|m| m.position),
                    discs: r.medium_count,
                    track: medium.and_then(|m| m.tracks.iter().filter_map(|t| t.position).min()),
                    tracks: medium.and_then(|m| m.track_count),
                };
                let elsewhere = q
                    .album
                    .as_deref()
                    .is_some_and(|a| crate::similar::names_match(&release.title, a).is_none());
                if !releases.iter().any(|(_, _, k)| k.id == release.id) {
                    releases.push((elsewhere, !group.secondarytypes.is_empty(), release));
                }
            }
        }
        releases.sort_by(|(a_off, a_sec, a), (b_off, b_sec, b)| {
            (a_off, a_sec)
                .cmp(&(b_off, b_sec))
                .then_with(|| date_key(a.date.as_deref()).cmp(&date_key(b.date.as_deref())))
                .then_with(|| a.id.cmp(&b.id))
        });
        Some(Candidate {
            id: Mbid::parse(&self.id)?,
            title: self.title.trim().to_string(),
            artists: self.artists.iter().filter_map(|a| text(&a.name)).collect(),
            length_ms: self
                .duration
                .filter(|d| *d >= 0.0)
                .map(|d| crate::units::ms_of_seconds(d).round() as u64),
            score: thousandths(score).clamp(0, 1000) as u32,
            sources: self.sources,
            releases: releases
                .into_iter()
                .take(KEPT_RELEASES)
                .map(|(_, _, r)| r)
                .collect(),
        })
    }
}

/// One AcoustID server, asked through `transport` at the pace of
/// `throttle` with the application key `key`.
pub struct Client<'a> {
    pub base: &'a str,
    pub key: &'a str,
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

/// The message of an error AcoustID answered, from its body.
fn message(body: &str) -> Option<String> {
    let answer: Answer = serde_json::from_str(body).ok()?;
    Some(answer.error?.message).filter(|m| !m.trim().is_empty())
}

impl Client<'_> {
    /// The recordings that fit the song, best first, each once and at
    /// most [`KEPT`]; none when no result fits.
    pub fn find(&self, q: &Query) -> Result<Vec<Candidate>> {
        // Clamped to 0 first, so no sign is lost.
        let seconds = q.seconds.round().max(0.0) as u64;
        let url = format!(
            "{}/v2/lookup?client={}&format=json&meta={META}&duration={seconds}&fingerprint={}",
            self.base.trim_end_matches('/'),
            crate::http::percent_encode(self.key),
            q.print.encoded()
        );
        let agent = crate::http::user_agent();
        let service = Service {
            name: "AcoustID",
            transport: self.transport,
            throttle: self.throttle,
        };
        let body = match service.get_text(&url, &[("User-Agent", agent.as_str())], TIMEOUT) {
            Ok(Some(body)) => body,
            Ok(None) => return Ok(Vec::new()),
            Err(e) => match e.downcast_ref::<TransportError>() {
                Some(TransportError::Status { code, body, .. }) => match message(body) {
                    Some(m) => bail!("AcoustID: {m}"),
                    None => bail!("AcoustID: status {code}"),
                },
                Some(t) => bail!("AcoustID: {t}"),
                None => return Err(e),
            },
        };
        let answer: Answer = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("reading AcoustID's answer: {e}"))?;
        if answer.status != "ok" {
            let m = answer.error.map(|e| e.message).unwrap_or_default();
            bail!(
                "AcoustID: {}",
                if m.is_empty() { &answer.status } else { &m }
            );
        }
        let mut fitting: Vec<(&Recording, f64)> = answer
            .results
            .iter()
            .filter(|r| r.score >= MIN_SCORE)
            .flat_map(|r| r.recordings.iter().map(move |rec| (rec, r.score)))
            .filter(|(rec, _)| rec.fits(q.seconds) && !rec.another_song(q))
            .collect();
        fitting.sort_by_cached_key(|(rec, score)| rec.rank(q, *score));
        let mut found: Vec<Candidate> = Vec::new();
        for (rec, score) in fitting {
            if found.len() < KEPT
                && !found.iter().any(|c| c.id.as_str() == rec.id)
                && let Some(candidate) = rec.candidate(q, score)
            {
                found.push(candidate);
            }
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests;
