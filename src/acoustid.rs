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
//! [`crate::musicbrainz::MAX_GAP_S`] of the song's, or that state none,
//! rank by:
//!
//! 1. Named as the song is, title then first artist, when it has them.
//! 1. The most sources, the prints submitted with it: a recording's
//!    original over a duplicate MusicBrainz has not merged yet.
//! 1. On a release group with no secondary type, as MusicBrainz's search
//!    ranks: a compilation's or a live album's last.
//! 1. The result's score, then the ID.
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
use crate::music;
use crate::musicbrainz::{MAX_GAP_S, Mbid, first_artist, is_mbid};

/// muman's application key, which AcoustID asks every client to send.
pub const KEY: &str = "EHCiWFI8Ce";
/// The least score a result is taken at, as beets takes them: below it, a
/// print shares too little with the song's to name its recording.
pub const MIN_SCORE: f64 = 0.5;
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
}

#[derive(Debug, Deserialize)]
struct Group {
    #[serde(default)]
    secondarytypes: Vec<String>,
}

impl Recording {
    fn fits(&self, seconds: f64) -> bool {
        is_mbid(&self.id)
            && self
                .duration
                .is_none_or(|d| (d - seconds).abs() <= MAX_GAP_S)
    }

    /// Its rank, smaller first: see the module docs.
    fn rank<'a>(&'a self, q: &Query, score: f64) -> impl Ord + 'a {
        let title = q
            .title
            .as_deref()
            .and_then(|t| music::names_match(&self.title, t));
        let credited = self
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let artist = q
            .artist
            .as_deref()
            .and_then(|a| music::names_match(&credited, first_artist(a)));
        (
            std::cmp::Reverse(title),
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
#[allow(clippy::cast_possible_truncation)]
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
    /// The MBID of the song's recording, or `None` when no result fits.
    pub fn find(&self, q: &Query) -> Result<Option<Mbid>> {
        // Clamped to 0 first, so no sign is lost.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let seconds = q.seconds.round().max(0.0) as u64;
        let url = format!(
            "{}/v2/lookup?client={}&format=json&meta=recordings+releasegroups+sources&duration={seconds}&fingerprint={}",
            self.base.trim_end_matches('/'),
            music::percent_encode(self.key),
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
            Ok(None) => return Ok(None),
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
        let best = answer
            .results
            .iter()
            .filter(|r| r.score >= MIN_SCORE)
            .flat_map(|r| r.recordings.iter().map(move |rec| (rec, r.score)))
            .filter(|(rec, _)| rec.fits(q.seconds))
            .min_by_key(|(rec, score)| rec.rank(q, *score));
        Ok(best.and_then(|(rec, _)| Mbid::parse(&rec.id)))
    }
}

#[cfg(test)]
mod tests;
