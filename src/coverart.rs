//! Covers from the Cover Art Archive, for a song whose own pictures are
//! missing or soft (`small-cover`): the front of the album's release
//! group, kept in the store as `<id>.jpg` and listed by every song of the
//! album, ranked against the other sources' pictures by the same
//! measures as any cover.
//!
//! The album is the release group the song's MusicBrainz IDs name, its
//! `MUSICBRAINZ_RELEASEGROUPID`, else its `MUSICBRAINZ_ALBUMID`, a
//! release; without either, MusicBrainz is searched for the album's title
//! and album artist ([`crate::musicbrainz::Client::release_group`]), at
//! MusicBrainz's pace. The archive serves each front at a few sizes;
//! `front-1200` is the largest short of the original, which can be tens
//! of megabytes, and still well past what a soft cover holds. A release
//! group whose releases have no front has none; a 404 is nothing found.
//! A body that is no JPEG, PNG or WebP, as an error page served as found
//! is, fails the lookup rather than being kept as the cover.
//!
//! The songs of one album ask once between them: a run looks one album
//! up per run however many of its songs are due.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::http::{HttpTransport, Service, Throttle};
use crate::musicbrainz;

const TIMEOUT: Duration = Duration::from_secs(60);
/// The least time between two requests to the archive across a run.
const GAP: Duration = Duration::from_millis(250);
/// How long a refusal holds requests back, doubling with each in a row.
const BACKOFF: Duration = Duration::from_secs(10);

/// The spacing of a run's requests to the archive.
#[must_use]
pub fn throttle() -> Throttle {
    Throttle::new(GAP, BACKOFF)
}

/// What a lookup knows of a song's album.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Query {
    pub album: String,
    pub album_artist: String,
    pub release_group: Option<String>,
    pub release: Option<String>,
}

/// A front cover, by the ID it was found under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cover {
    pub id: String,
    pub bytes: Vec<u8>,
}

impl Cover {
    /// The extension its bytes say it has.
    #[must_use]
    pub fn extension(&self) -> &'static str {
        image_extension(&self.bytes).unwrap_or("jpg")
    }
}

/// The extension of an image by its first bytes, if it is a JPEG, PNG
/// or WebP.
fn image_extension(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\xFF\xD8\xFF") {
        Some("jpg")
    } else if b.starts_with(b"\x89PNG") {
        Some("png")
    } else if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP".as_slice()) {
        Some("webp")
    } else {
        None
    }
}

/// The archive, asked through `transport` at the pace of `throttle`,
/// and MusicBrainz for what the song's tags do not name.
pub struct Client<'a> {
    pub base: &'a str,
    pub transport: &'a (dyn HttpTransport + Sync),
    pub throttle: &'a Throttle,
    pub musicbrainz: &'a musicbrainz::Client<'a>,
}

impl std::fmt::Debug for Client<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Client<'_> {
    /// The front of `id`, a release group's or a release's.
    fn front(&self, kind: &str, id: &str) -> Result<Option<Cover>> {
        let url = format!(
            "{}/{kind}/{}/front-1200",
            self.base.trim_end_matches('/'),
            crate::music::percent_encode(id)
        );
        let agent = crate::http::user_agent();
        let service = Service {
            name: "the Cover Art Archive",
            transport: self.transport,
            throttle: self.throttle,
        };
        let Some(bytes) = service
            .get(&url, &[("User-Agent", agent.as_str())], TIMEOUT)?
            .filter(|b| !b.is_empty())
        else {
            return Ok(None);
        };
        if image_extension(&bytes).is_none() {
            bail!("{url} answered with no image");
        }
        Ok(Some(Cover {
            id: id.to_string(),
            bytes,
        }))
    }

    /// The front of a kept cover's ID again, a release group's or a
    /// release's.
    pub fn by_id(&self, id: &str) -> Result<Option<Cover>> {
        match self.front("release-group", id)? {
            Some(cover) => Ok(Some(cover)),
            None => self.front("release", id),
        }
    }

    /// The front of the song's album, by the IDs its tags name, else as a
    /// search finds the album.
    pub fn find(&self, q: &Query) -> Result<Option<Cover>> {
        if let Some(id) = &q.release_group
            && let Some(cover) = self.front("release-group", id)?
        {
            return Ok(Some(cover));
        }
        if let Some(id) = &q.release
            && let Some(cover) = self.front("release", id)?
        {
            return Ok(Some(cover));
        }
        if q.release_group.is_some() || q.release.is_some() {
            return Ok(None);
        }
        match self.musicbrainz.release_group(&q.album, &q.album_artist)? {
            Some(id) => self.front("release-group", &id),
            None => Ok(None),
        }
    }
}

/// Where a cover is kept in the store.
#[must_use]
pub fn path_of(folder: &Path, cover: &Cover) -> PathBuf {
    folder.join(format!("{}.{}", cover.id, cover.extension()))
}

/// Keep `cover` in `folder`, returning its path.
pub fn keep(folder: &Path, cover: &Cover) -> Result<PathBuf> {
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let path = path_of(folder, cover);
    let name = path.file_name().context("a cover has a name")?;
    crate::atomic::write(folder, &name.to_string_lossy(), &cover.bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lrclib::testing::Server;

    const GROUP: &str = "00000000-0000-4000-8000-0000000000aa";
    const JPEG: &[u8] = b"\xFF\xD8\xFF\xE0";

    fn find(server: &Server, q: &Query) -> Option<Cover> {
        found(server, q).unwrap()
    }

    fn found(server: &Server, q: &Query) -> Result<Option<Cover>> {
        let throttle = Throttle::none();
        let musicbrainz = musicbrainz::Client {
            base: "http://mb.test",
            transport: server,
            throttle: &throttle,
        };
        Client {
            base: "http://caa.test/",
            transport: server,
            throttle: &throttle,
            musicbrainz: &musicbrainz,
        }
        .find(q)
    }

    fn query() -> Query {
        Query {
            album: "Rooms of Salt".into(),
            album_artist: "Paper Comets".into(),
            release_group: None,
            release: None,
        }
    }

    #[test]
    fn an_album_s_id_is_asked_for_its_front_without_a_search() {
        let server =
            Server::default().answer_bytes(&format!("/release-group/{GROUP}/front-1200"), JPEG);
        let q = Query {
            release_group: Some(GROUP.into()),
            ..query()
        };
        let cover = find(&server, &q).unwrap();
        assert_eq!((cover.id.as_str(), cover.extension()), (GROUP, "jpg"));
        let asked = server.asked.lock().unwrap();
        assert_eq!(
            *asked,
            [format!("http://caa.test/release-group/{GROUP}/front-1200")]
        );
    }

    #[test]
    fn an_album_without_ids_is_found_by_its_names() {
        let search = format!(
            r#"{{"release-groups": [
                {{"id": "{GROUP}b", "title": "Rooms of Salt (Live)", "primary-type": "Album",
                  "score": 100, "artist-credit": [{{"name": "Paper Comets"}}]}},
                {{"id": "{GROUP}", "title": "Rooms of Salt", "primary-type": "Album",
                  "score": 90, "artist-credit": [{{"name": "Paper Comets"}}]}}
            ]}}"#
        );
        let server = Server::default()
            .answer("/ws/2/release-group?query=", &search)
            .answer_bytes(&format!("/release-group/{GROUP}/front-1200"), b"\x89PNG");
        let cover = find(&server, &query()).unwrap();
        assert_eq!(cover.id, GROUP, "the exact title wins");
        assert!(find(&Server::default(), &query()).is_none());
    }

    #[test]
    fn a_page_served_for_a_cover_fails_the_lookup() {
        let server = Server::default().answer(
            &format!("/release-group/{GROUP}/front-1200"),
            "<html>busy</html>",
        );
        let q = Query {
            release_group: Some(GROUP.into()),
            ..query()
        };
        let err = found(&server, &q).unwrap_err();
        assert!(format!("{err:#}").contains("no image"), "{err:#}");
    }
}
