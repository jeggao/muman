//! Lyrics from an LRCLIB server: a song's record found by its title,
//! artist, album and length, and kept in the store as `<id>.lrc`, the
//! record's length stated in it, beside the record as `<id>.json`.
//!
//! `/api/get` is asked first when the album is known, then `/api/search`
//! by title and artist. A record fits when its length is within 2 s of
//! the song's audio, and its track and artist names hold the song's,
//! compared as YouTube Music's are, by letters and digits without
//! featured artists. Of those, a timed one wins, then the closest in
//! length. Measured against lrclib.net: asking for 300 s returned a 302 s
//! record, and a miss is a 404 `TrackNotFound`.
//!
//! The stated length, `[length:mm:ss.cc]` atop the `.lrc`, is what ranks
//! the record's lyrics against the chosen audio (`resolve`): a timed
//! subtitle of the same recording keeps winning, while LRCLIB's timed
//! lines win over untimed or missing ones. An instrumental record stops
//! the song looking; a record deleted from the store is fetched again by
//! its ID.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::http::{HttpTransport, TransportError};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::clean;
use crate::music;

/// The record a lookup takes must be within this of the song's length;
/// LRCLIB's own lookup allows as much.
const MAX_GAP_S: f64 = 2.0;
const TIMEOUT: Duration = Duration::from_secs(20);

/// What a lookup is told of the song.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: u64,
    #[serde(default)]
    pub track_name: Option<String>,
    #[serde(default)]
    pub artist_name: Option<String>,
    #[serde(default)]
    pub album_name: Option<String>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub plain_lyrics: Option<String>,
    #[serde(default)]
    pub synced_lyrics: Option<String>,
}

impl Record {
    fn synced(&self) -> bool {
        self.synced_lyrics
            .as_deref()
            .is_some_and(|l| !l.trim().is_empty())
    }

    fn gap(&self, seconds: f64) -> f64 {
        self.duration.map_or(f64::MAX, |d| (d - seconds).abs())
    }

    /// Whether it is the query's song by its names and length.
    fn fits(&self, q: &Query) -> bool {
        let same = |a: Option<&str>, b: &str| {
            let (a, b) = (
                music::normalize(clean::without_credits(a.unwrap_or_default())),
                music::normalize(clean::without_credits(b)),
            );
            !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
        };
        self.gap(q.seconds) <= MAX_GAP_S
            && same(self.track_name.as_deref(), &q.title)
            && same(self.artist_name.as_deref(), &q.artist)
    }

    /// The `.lrc` it is kept as: timed lines when it has them, its
    /// length stated, so a song of another length is told apart.
    #[must_use]
    pub fn lrc(&self) -> String {
        let mut text = String::new();
        if let Some(d) = self.duration {
            #[allow(clippy::cast_possible_truncation)]
            let cs = (d * 100.0).round() as i64;
            let _ = writeln!(
                text,
                "[length:{:02}:{:02}.{:02}]",
                cs / 6000,
                cs / 100 % 60,
                cs % 100
            );
        }
        let body = if self.synced() {
            self.synced_lyrics.as_deref()
        } else {
            self.plain_lyrics.as_deref()
        };
        text.push_str(body.unwrap_or_default());
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }
}

/// What a lookup found.
#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    Lyrics(Record),
    /// A record of the song that says it has no words.
    Instrumental(Record),
    Nothing,
}

/// One LRCLIB server, asked through `transport`.
pub struct Client<'a> {
    pub base: &'a str,
    pub transport: &'a (dyn HttpTransport + Sync),
}

impl std::fmt::Debug for Client<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

fn user_agent() -> String {
    format!(
        "muman/{} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_REPOSITORY")
    )
}

impl Client<'_> {
    fn get(&self, path: &str) -> Result<Option<String>> {
        let url = format!("{}{path}", self.base.trim_end_matches('/'));
        let agent = user_agent();
        match self
            .transport
            .get_json(&url, &[("User-Agent", agent.as_str())], TIMEOUT)
        {
            Ok(body) => Ok(Some(body)),
            Err(TransportError::Status { code: 404, .. }) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("{url}: {e}")),
        }
    }

    /// The record of one ID, to fetch a kept one again.
    pub fn by_id(&self, id: u64) -> Result<Option<Record>> {
        self.get(&format!("/api/get/{id}"))?
            .map(|b| serde_json::from_str(&b).context("reading LRCLIB's answer"))
            .transpose()
    }

    /// The song's record: asked for by all four when the album is known,
    /// else searched for by title and artist; of those that fit, the
    /// timed, then the closest in length.
    pub fn find(&self, q: &Query) -> Result<Found> {
        let enc = |s: &str| music::percent_encode(s);
        let mut candidates: Vec<Record> = Vec::new();
        if let Some(album) = &q.album {
            #[allow(clippy::cast_possible_truncation)]
            let path = format!(
                "/api/get?track_name={}&artist_name={}&album_name={}&duration={}",
                enc(&q.title),
                enc(&q.artist),
                enc(album),
                q.seconds.round() as i64
            );
            if let Some(body) = self.get(&path)? {
                candidates.push(serde_json::from_str(&body).context("reading LRCLIB's answer")?);
            }
        }
        if !candidates.iter().any(|r| r.fits(q)) {
            let path = format!(
                "/api/search?track_name={}&artist_name={}",
                enc(&q.title),
                enc(&q.artist)
            );
            if let Some(body) = self.get(&path)? {
                candidates.extend(
                    serde_json::from_str::<Vec<Record>>(&body)
                        .context("reading LRCLIB's answer")?,
                );
            }
        }
        let best = candidates.into_iter().filter(|r| r.fits(q)).min_by(|a, b| {
            b.synced()
                .cmp(&a.synced())
                .then(a.gap(q.seconds).total_cmp(&b.gap(q.seconds)))
        });
        Ok(match best {
            Some(r) if r.instrumental => Found::Instrumental(r),
            Some(r)
                if r.synced()
                    || r.plain_lyrics
                        .as_deref()
                        .is_some_and(|l| !l.trim().is_empty()) =>
            {
                Found::Lyrics(r)
            }
            _ => Found::Nothing,
        })
    }
}

/// Where a record is kept in the store.
#[must_use]
pub fn path_of(folder: &Path, id: u64) -> PathBuf {
    folder.join(format!("{id}.lrc"))
}

/// Keep a record in `folder`: the record, then its lyrics, so a kept
/// `.lrc` always has its record beside it.
pub fn keep(folder: &Path, record: &Record) -> Result<PathBuf> {
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let json = serde_json::to_vec_pretty(record).context("writing an LRCLIB record")?;
    crate::atomic::write(folder, &format!("{}.json", record.id), &json)?;
    crate::atomic::write(
        folder,
        &format!("{}.lrc", record.id),
        record.lrc().as_bytes(),
    )?;
    Ok(path_of(folder, record.id))
}

#[cfg(test)]
pub mod testing {
    //! An LRCLIB server answering from records by path.

    use std::sync::Mutex;
    use std::time::Duration;

    use crate::http::{HttpTransport, TransportError};

    /// Answers each GET from the first record whose path fragment the
    /// URL holds; 404 for any other.
    #[derive(Debug, Default)]
    pub struct Server {
        pub answers: Vec<(String, String)>,
        pub asked: Mutex<Vec<String>>,
    }

    impl Server {
        #[must_use]
        pub fn answer(mut self, fragment: &str, body: &str) -> Self {
            self.answers.push((fragment.into(), body.into()));
            self
        }
    }

    impl HttpTransport for Server {
        fn get_json(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            _: Duration,
        ) -> Result<String, TransportError> {
            assert!(headers.iter().any(|(k, _)| *k == "User-Agent"));
            self.asked.lock().unwrap().push(url.to_string());
            self.answers
                .iter()
                .find(|(f, _)| url.contains(f.as_str()))
                .map(|(_, b)| b.clone())
                .ok_or(TransportError::Status {
                    code: 404,
                    body: "TrackNotFound".into(),
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Server;
    use super::*;

    fn query() -> Query {
        Query {
            title: "Lantern Weather".into(),
            artist: "Paper Comets".into(),
            album: None,
            seconds: 354.0,
        }
    }

    const RECORDS: &str = r#"[
        {"id": 1, "trackName": "Lantern Weather", "artistName": "Paper Comets", "duration": 300.0,
         "plainLyrics": "far", "syncedLyrics": "[00:01.00] far"},
        {"id": 2, "trackName": "Lantern Weather (Remastered)", "artistName": "Paper Comets", "duration": 355.0,
         "plainLyrics": "plain only"},
        {"id": 3, "trackName": "Lantern weather", "artistName": "Paper Comets", "duration": 353.0,
         "plainLyrics": "timed", "syncedLyrics": "[00:00.15] Is the harbor still awake?"}
    ]"#;

    #[test]
    fn the_timed_record_of_the_same_length_is_taken() {
        let server = Server::default().answer("/api/search", RECORDS);
        let client = Client {
            base: "http://lrclib.test",
            transport: &server,
        };
        let Found::Lyrics(r) = client.find(&query()).unwrap() else {
            panic!("nothing found");
        };
        assert_eq!(r.id, 3);
        assert_eq!(
            r.lrc(),
            "[length:05:53.00]\n[00:00.15] Is the harbor still awake?\n"
        );
    }

    #[test]
    fn nothing_fits_another_song_or_length() {
        let server = Server::default().answer(
            "/api/search",
            r#"[{"id": 4, "trackName": "Second Tune", "artistName": "Paper Comets", "duration": 354.0, "plainLyrics": "x"}]"#,
        );
        let client = Client {
            base: "http://lrclib.test",
            transport: &server,
        };
        assert_eq!(client.find(&query()).unwrap(), Found::Nothing);
        let none = Server::default();
        let client = Client {
            base: "http://lrclib.test",
            transport: &none,
        };
        assert_eq!(client.find(&query()).unwrap(), Found::Nothing);
    }

    #[test]
    fn an_exact_answer_is_asked_first_with_the_album() {
        let server = Server::default().answer(
            "/api/get?",
            r#"{"id": 5, "trackName": "Lantern Weather", "artistName": "Paper Comets", "duration": 355.0, "instrumental": true}"#,
        );
        let client = Client {
            base: "http://lrclib.test/",
            transport: &server,
        };
        let q = Query {
            album: Some("Rooms of Salt".into()),
            ..query()
        };
        assert!(matches!(client.find(&q).unwrap(), Found::Instrumental(_)));
        let asked = server.asked.lock().unwrap();
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert!(asked[0].starts_with("http://lrclib.test/api/get?track_name=Lantern%20Weather"));
        assert!(asked[0].ends_with("&duration=354"));
    }
}
