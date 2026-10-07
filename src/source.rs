//! A source's key, as the song list names it: `<domain>:<id>` for what
//! yt-dlp fetched, `manual:<path>` for a file in the manual folder, and
//! `<provider>:<id>` for a record a lookup keeps.
//!
//! A fetched source is named by the site it came from and the ID yt-dlp
//! gives it there, so `archive.org:<id>` reads as the address it is. A
//! site's domain is one per extractor where [`SITES`] names it, so a
//! video found at `music.youtube.com`, `m.youtube.com` or `youtu.be` is
//! the one `youtube.com:<id>`, and a track at `<artist>.bandcamp.com`
//! is `bandcamp.com:<id>`; every other site is the domain of its page,
//! as [`page`] picks it, without `www.`. A YouTube playlist, which an
//! album may be made from, is `youtube.com:playlist/<id>`.
//!
//! Keys were named by yt-dlp's extractor before, as `youtube:<id>` and
//! `archiveorg:<id>`. Such a key of an extractor [`SITES`] names reads
//! as its domain's, wherever it is read, and is written so; one of any
//! other extractor reads as it was, a source like any other, until a
//! sync renames it by the page it recorded
//! ([`crate::reconcile`]). [`SourceKey::spelled_before`] gives the old
//! name back, for what an earlier muman keyed by it.
//!
//! An extractor added to [`SITES`] names what it fetches by the domain
//! given there, where what it fetched before is named by each page's:
//! add one only with a rename of those keys, or a source fetched again
//! is listed twice.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// The site of YouTube's videos and playlists, YouTube Music's too.
pub const YOUTUBE: &str = "youtube.com";

/// What a YouTube playlist's ID follows in its key.
const PLAYLIST: &str = "playlist/";

/// yt-dlp's extractors, by the lower-cased key it prints, whose sources
/// are named by one domain whichever address they were found at.
pub const SITES: [(&str, &str); 13] = [
    ("youtube", YOUTUBE),
    ("youtubetab", YOUTUBE),
    ("archiveorg", "archive.org"),
    ("soundcloud", "soundcloud.com"),
    ("bandcamp", "bandcamp.com"),
    ("vimeo", "vimeo.com"),
    ("dailymotion", "dailymotion.com"),
    ("mixcloud", "mixcloud.com"),
    ("audiomack", "audiomack.com"),
    ("niconico", "nicovideo.jp"),
    ("bilibili", "bilibili.com"),
    ("twitter", "x.com"),
    ("tiktok", "tiktok.com"),
];

/// The schemes that are no site: a manual file, and the records lookups
/// keep.
const NOT_SITES: [&str; 4] = [
    "manual",
    crate::provider::LRCLIB,
    crate::provider::MUSICBRAINZ,
    crate::provider::COVERART,
];

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum SourceKey {
    /// What yt-dlp fetched, by its site and ID; or a record a lookup
    /// keeps, by its provider's name and ID.
    Remote { site: String, id: String },
    /// A file under the manual folder, by its path there.
    Manual(ManualKey),
}

/// A manual file's path in the manual folder, part by part in NFC: the
/// key it is listed under, never the name it is opened by. A file named
/// in NFD, as a Mac writes names, has the composed key, and only
/// [`crate::store::Store::locate`] knows its name on disk; Linux and
/// NTFS open a name by the bytes it was written with.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ManualKey(PathBuf);

impl ManualKey {
    /// The key as a path, to compare and to show: not to open.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl From<&Path> for ManualKey {
    fn from(rel: &Path) -> Self {
        Self(crate::relpath::normalized(rel))
    }
}

impl From<PathBuf> for ManualKey {
    fn from(rel: PathBuf) -> Self {
        Self::from(rel.as_path())
    }
}

impl From<&PathBuf> for ManualKey {
    fn from(rel: &PathBuf) -> Self {
        Self::from(rel.as_path())
    }
}

impl From<&str> for ManualKey {
    fn from(portable: &str) -> Self {
        Self(crate::relpath::from_portable(portable))
    }
}

impl SourceKey {
    /// A YouTube video, by its ID.
    #[must_use]
    pub fn youtube(id: &str) -> Self {
        Self::Remote {
            site: YOUTUBE.to_string(),
            id: id.to_string(),
        }
    }

    /// A YouTube playlist, as an album is.
    #[must_use]
    pub fn playlist(id: &str) -> Self {
        Self::Remote {
            site: YOUTUBE.to_string(),
            id: format!("{PLAYLIST}{id}"),
        }
    }

    /// What yt-dlp fetched, by the extractor key it printed, the page
    /// [`page`] picks and the ID: the extractor's site where [`SITES`]
    /// names one, else the page's, else the extractor as keys were once
    /// named.
    #[must_use]
    pub fn fetched(extractor: &str, page: Option<&str>, id: &str) -> Option<Self> {
        let extractor = extractor.to_ascii_lowercase();
        let site = site_of(&extractor)
            .map(str::to_string)
            .or_else(|| page.and_then(domain_of))
            .unwrap_or(extractor);
        Self::parse(&format!("{site}:{id}")).ok()
    }

    pub fn parse(text: &str) -> Result<Self> {
        let Some((scheme, rest)) = text.split_once(':') else {
            bail!("`{text}` names no source: expected `youtube.com:<id>` or `manual:<path>`");
        };
        if scheme == "manual" {
            if !is_plain_relative(Path::new(rest)) {
                bail!("`{text}`: a manual source is a path inside the manual folder");
            }
            return Ok(Self::Manual(rest.into()));
        }
        let scheme = scheme.to_ascii_lowercase();
        let (site, id) = match scheme.as_str() {
            "youtubetab" => (YOUTUBE.to_string(), format!("{PLAYLIST}{rest}")),
            old => (
                site_of(old).map_or_else(|| old.to_string(), str::to_string),
                rest.to_string(),
            ),
        };
        let valid = if site == YOUTUBE {
            id.strip_prefix(PLAYLIST)
                .map_or_else(|| is_id(&id), is_token)
        } else {
            is_site(&site) && is_token(&id)
        };
        if !valid {
            bail!("`{text}` is not a source key");
        }
        Ok(Self::Remote { site, id })
    }

    /// The ID a remote source's file is named with.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Remote { id, .. } => Some(id),
            Self::Manual(_) => None,
        }
    }

    /// The site a source was fetched from, if it was.
    #[must_use]
    pub fn site(&self) -> Option<&str> {
        match self {
            Self::Remote { site, .. } if !NOT_SITES.contains(&site.as_str()) => Some(site),
            _ => None,
        }
    }

    /// Whether this is a YouTube video.
    #[must_use]
    pub fn is_youtube(&self) -> bool {
        matches!(self, Self::Remote { site, id } if site == YOUTUBE && !id.starts_with(PLAYLIST))
    }

    /// Whether this is what yt-dlp fetched under a name of its extractor
    /// no table knows the site of: one a sync renames.
    #[must_use]
    pub fn is_old(&self) -> bool {
        self.site().is_some_and(|s| !s.contains('.'))
    }

    /// What to hand yt-dlp to fetch it again; only YouTube is known.
    #[must_use]
    pub fn url(&self) -> Option<String> {
        match self {
            Self::Remote { site, id } if site == YOUTUBE => Some(match id.strip_prefix(PLAYLIST) {
                Some(list) => format!("https://www.youtube.com/playlist?list={list}"),
                None => watch_url(id),
            }),
            _ => None,
        }
    }

    /// yt-dlp's `--download-archive` line for it, `<extractor> <id>`,
    /// where the extractor is known.
    #[must_use]
    pub fn archive_line(&self) -> Option<String> {
        let (site, id) = match self {
            Self::Remote { site, id } if self.site().is_some() && !id.starts_with(PLAYLIST) => {
                (site, id)
            }
            _ => return None,
        };
        let extractor = SITES
            .iter()
            .find(|(_, s)| s == site)
            .map_or(site.as_str(), |(e, _)| e);
        (!extractor.contains('.')).then(|| format!("{extractor} {id}\n"))
    }

    /// How an earlier muman named this key, where it named it otherwise:
    /// by yt-dlp's extractor, as `youtube:<id>`.
    #[must_use]
    pub fn spelled_before(&self) -> Option<Self> {
        let Self::Remote { site, id } = self else {
            return None;
        };
        let (extractor, id) = match id.strip_prefix(PLAYLIST) {
            Some(list) if site == YOUTUBE => ("youtubetab", list),
            _ => (SITES.iter().find(|(_, s)| s == site)?.0, id.as_str()),
        };
        Some(Self::Remote {
            site: extractor.to_string(),
            id: id.to_string(),
        })
    }

    /// A short name for messages and collision suffixes.
    #[must_use]
    pub fn short(&self) -> String {
        match self {
            Self::Remote { id, .. } => id.strip_prefix(PLAYLIST).unwrap_or(id).to_string(),
            Self::Manual(key) => key
                .path()
                .file_stem()
                .map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
        }
    }
}

/// The page a source names its site by and is fetched again from: the
/// one yt-dlp's extractor gives it, `webpage_url`, which a site's own
/// extractor writes the same however the video was reached; but for
/// yt-dlp's generic extractor, which takes a file from any address and
/// gives the mirror a redirect ended at, the address it was given,
/// `original_url`.
#[must_use]
pub fn page<'a>(
    extractor: &str,
    webpage: Option<&'a str>,
    original: Option<&'a str>,
) -> Option<&'a str> {
    if extractor.eq_ignore_ascii_case("generic") {
        original.or(webpage)
    } else {
        webpage
    }
}

/// The site [`SITES`] names for an extractor.
fn site_of(extractor: &str) -> Option<&'static str> {
    SITES.iter().find(|(e, _)| *e == extractor).map(|(_, s)| *s)
}

/// The site a page's address names, as yt-dlp's `webpage_url_domain`
/// does: its host, lower-cased, without `www.`.
#[must_use]
pub fn domain_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next()?;
    domain_of_host(host)
}

/// A host as a key names its site, if it can be one.
fn domain_of_host(host: &str) -> Option<String> {
    let host = host.trim().to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    (host.contains('.') && is_site(host)).then(|| host.to_string())
}

/// Letters, digits, `-`, `_` and `.`: a domain, or an extractor's name
/// as keys were once named.
fn is_site(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && !s.ends_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

impl fmt::Display for SourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Remote { site, id } => write!(f, "{site}:{id}"),
            Self::Manual(key) => write!(f, "manual:{}", crate::relpath::to_portable(key.path())),
        }
    }
}

impl From<SourceKey> for String {
    fn from(key: SourceKey) -> Self {
        key.to_string()
    }
}

impl TryFrom<String> for SourceKey {
    type Error = anyhow::Error;

    fn try_from(text: String) -> Result<Self> {
        Self::parse(&text)
    }
}

#[must_use]
pub fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

/// Whether `s` has the shape of a YouTube video ID.
#[must_use]
pub fn is_id(s: &str) -> bool {
    s.len() == 11 && is_token(s)
}

/// Letters, digits, `-` and `_`: what an extractor's ID is made of.
fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The ID a downloaded file is named with, `<title> [<id>].<ext>`.
#[must_use]
pub fn id_of(file: &Path) -> Option<&str> {
    let stem = file.file_stem()?.to_str()?.strip_suffix(']')?;
    let id = &stem[stem.rfind('[')? + 1..];
    is_token(id).then_some(id)
}

/// A path that stays inside the folder it is joined to.
#[must_use]
pub fn is_plain_relative(path: &Path) -> bool {
    path.components().next().is_some()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_through_text() {
        for text in [
            "youtube.com:vid00000001",
            "youtube.com:playlist/OLAK5uy_x",
            "archive.org:item-0001_a",
            "lrclib:7",
            "manual:A/b c.flac",
        ] {
            assert_eq!(SourceKey::parse(text).unwrap().to_string(), text);
        }
    }

    #[test]
    fn a_key_named_by_its_extractor_reads_as_its_site() {
        for (old, now) in [
            ("youtube:vid00000001", "youtube.com:vid00000001"),
            ("YouTube:vid00000001", "youtube.com:vid00000001"),
            ("youtubetab:OLAK5uy_x", "youtube.com:playlist/OLAK5uy_x"),
            ("archiveorg:item0001", "archive.org:item0001"),
            ("soundcloud:12345", "soundcloud.com:12345"),
        ] {
            let key = SourceKey::parse(old).unwrap();
            assert_eq!(key.to_string(), now);
            assert!(!key.is_old());
            let (scheme, id) = old.split_once(':').unwrap();
            let before = key.spelled_before().unwrap().to_string();
            assert_eq!(before, format!("{}:{id}", scheme.to_ascii_lowercase()));
        }
        let unknown = SourceKey::parse("funkwhale:abc").unwrap();
        assert_eq!(unknown.to_string(), "funkwhale:abc");
        assert!(unknown.is_old(), "renamed by a sync");
        assert!(!SourceKey::parse("lrclib:7").unwrap().is_old());
    }

    #[test]
    fn a_fetched_source_is_named_by_one_site_per_extractor_else_its_page() {
        let at = |extractor, page, id| SourceKey::fetched(extractor, page, id).unwrap().to_string();
        assert_eq!(
            at(
                "Youtube",
                Some("https://music.youtube.com/watch?v=vid00000001"),
                "vid00000001"
            ),
            "youtube.com:vid00000001"
        );
        assert_eq!(
            at(
                "Bandcamp",
                Some("https://artist.bandcamp.com/track/t"),
                "123"
            ),
            "bandcamp.com:123"
        );
        assert_eq!(
            at("Funkwhale", Some("https://www.Tunes.example/t/abc"), "abc"),
            "tunes.example:abc"
        );
        assert_eq!(at("Funkwhale", None, "abc"), "funkwhale:abc");
        let (mirror, given) = (
            Some("https://mirror7.files.example/0/a.mp3"),
            Some("https://files.example/download/a.mp3"),
        );
        assert_eq!(page("Generic", mirror, given), given);
        assert_eq!(page("Funkwhale", mirror, given), mirror);
        assert!(SourceKey::fetched("Youtube", None, "short").is_none());
    }

    #[test]
    fn a_page_names_its_site_without_www() {
        assert_eq!(
            domain_of("https://www.Example.org:8080/a?b#c").as_deref(),
            Some("example.org")
        );
        assert_eq!(
            domain_of("https://user@tunes.example/x").as_deref(),
            Some("tunes.example")
        );
        assert_eq!(domain_of("https://localhost/x"), None);
    }

    #[test]
    fn a_manual_path_stays_inside_its_folder() {
        assert!(SourceKey::parse("manual:../x.flac").is_err());
        assert!(SourceKey::parse("manual:/x.flac").is_err());
        assert!(SourceKey::parse("manual:").is_err());
    }

    #[test]
    fn a_youtube_key_needs_a_video_or_playlist_id() {
        assert!(SourceKey::parse("youtube.com:short").is_err());
        assert!(SourceKey::parse("youtube:short").is_err());
        assert!(SourceKey::parse("nothing").is_err());
        assert!(SourceKey::parse("youtube.com:-dashid_123").is_ok());
        assert!(SourceKey::parse("youtube.com:playlist/").is_err());
        assert!(SourceKey::parse("archive.org:a/b").is_err());
    }

    #[test]
    fn only_youtube_can_be_fetched_again() {
        assert_eq!(
            SourceKey::youtube("vid00000001").url().as_deref(),
            Some("https://www.youtube.com/watch?v=vid00000001")
        );
        assert!(
            SourceKey::parse("soundcloud.com:1")
                .unwrap()
                .url()
                .is_none()
        );
        assert_eq!(
            SourceKey::playlist("OLAK5uy_x").url().as_deref(),
            Some("https://www.youtube.com/playlist?list=OLAK5uy_x")
        );
    }

    #[test]
    fn yt_dlp_archives_a_source_by_its_extractor_where_known() {
        let line = |k: &str| SourceKey::parse(k).unwrap().archive_line();
        assert_eq!(
            line("youtube.com:vid00000001").as_deref(),
            Some("youtube vid00000001\n")
        );
        assert_eq!(
            line("archive.org:item0001").as_deref(),
            Some("archiveorg item0001\n")
        );
        assert_eq!(line("funkwhale:abc").as_deref(), Some("funkwhale abc\n"));
        assert_eq!(line("tunes.example:abc"), None);
        assert_eq!(line("youtube.com:playlist/OLAK5uy_x"), None);
        assert_eq!(line("lrclib:7"), None);
    }

    #[test]
    fn a_file_is_named_with_its_id() {
        let p = Path::new("/o/chan/Song [x] ⧸ B [vid00000001].mkv");
        assert_eq!(id_of(p), Some("vid00000001"));
        assert_eq!(id_of(Path::new("/o/chan/Song.mkv")), None);
        assert_eq!(id_of(Path::new("/o/chan/Song [a b].mkv")), None);
    }

    #[test]
    fn keys_serialize_as_strings() {
        let key = SourceKey::youtube("vid00000001");
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, "\"youtube.com:vid00000001\"");
        assert_eq!(serde_json::from_str::<SourceKey>(&json).unwrap(), key);
        assert_eq!(
            serde_json::from_str::<SourceKey>("\"youtube:vid00000001\"").unwrap(),
            key
        );
    }
}
