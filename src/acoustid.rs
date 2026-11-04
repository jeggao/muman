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
//! `[providers.acoustid] key` sets. A result fits when its score is at
//! least [`MIN_SCORE`]; of its recordings, those whose length is within
//! [`LOOSE_GAP_S`] of the song's, or that state none but were submitted
//! with at least [`MIN_UNTIMED_SOURCES`] prints, rank by:
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
//! [`Client::find`] answers every recording that fits, best first: the
//! lookup takes the first whose MusicBrainz comment does not name the
//! other mix than the song's audio is ([`crate::musicbrainz::other_mix`]),
//! as a mono rip whose print matches the stereo recording too.
//!
//! AcoustID allows three requests a second from a client; every request
//! waits its turn on one [`Throttle`], and a refusal holds every later
//! request back 2 s, doubling, as [`crate::http::Service`] does for every
//! service. An error AcoustID answers, such as an unknown key, is told by
//! its message, never by the request, whose print runs to a few kilobytes.

use std::time::Duration;

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::fingerprint::Print;
use crate::http::{HttpTransport, Service, Throttle, TransportError};
use crate::musicbrainz::{MAX_GAP_S, Mbid, first_artist, is_mbid};

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
    secondarytypes: Vec<String>,
}

impl Recording {
    fn fits(&self, seconds: f64) -> bool {
        is_mbid(&self.id)
            && match self.duration {
                Some(d) => (d - seconds).abs() <= LOOSE_GAP_S,
                None => self.sources >= MIN_UNTIMED_SOURCES,
            }
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
    /// The MBIDs of the recordings that fit the song, best first, each
    /// once; none when no result fits.
    pub fn find(&self, q: &Query) -> Result<Vec<Mbid>> {
        // Clamped to 0 first, so no sign is lost.
        let seconds = q.seconds.round().max(0.0) as u64;
        let url = format!(
            "{}/v2/lookup?client={}&format=json&meta=recordings+releasegroups+sources&duration={seconds}&fingerprint={}",
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
            .filter(|(rec, _)| rec.fits(q.seconds))
            .collect();
        fitting.sort_by_cached_key(|(rec, score)| rec.rank(q, *score));
        let mut ids: Vec<Mbid> = Vec::new();
        for id in fitting.iter().filter_map(|(rec, _)| Mbid::parse(&rec.id)) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests;
