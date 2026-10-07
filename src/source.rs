//! A source's key, as the song list names it: `<extractor>:<id>` for
//! what yt-dlp fetched, `manual:<path>` for a file in the manual folder.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum SourceKey {
    /// What yt-dlp fetched: its extractor, lower-cased, and the ID.
    Remote { extractor: String, id: String },
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
    #[must_use]
    pub fn youtube(id: &str) -> Self {
        Self::Remote {
            extractor: "youtube".to_string(),
            id: id.to_string(),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let Some((scheme, rest)) = text.split_once(':') else {
            bail!("`{text}` names no source: expected `youtube:<id>` or `manual:<path>`");
        };
        if scheme == "manual" {
            if !is_plain_relative(Path::new(rest)) {
                bail!("`{text}`: a manual source is a path inside the manual folder");
            }
            return Ok(Self::Manual(rest.into()));
        }
        let extractor = scheme.to_ascii_lowercase();
        if extractor.is_empty() || !is_token(rest) || (extractor == "youtube" && !is_id(rest)) {
            bail!("`{text}` is not a source key");
        }
        Ok(Self::Remote {
            extractor,
            id: rest.to_string(),
        })
    }

    /// The ID a remote source's file is named with.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Remote { id, .. } => Some(id),
            Self::Manual(_) => None,
        }
    }

    /// A YouTube playlist, as an album is, by yt-dlp's extractor for it.
    #[must_use]
    pub fn playlist(id: &str) -> Self {
        Self::Remote {
            extractor: "youtubetab".to_string(),
            id: id.to_string(),
        }
    }

    /// What to hand yt-dlp to fetch it again; only YouTube is known.
    #[must_use]
    pub fn url(&self) -> Option<String> {
        match self {
            Self::Remote { extractor, id } if extractor == "youtube" => Some(watch_url(id)),
            Self::Remote { extractor, id } if extractor == "youtubetab" => {
                Some(format!("https://www.youtube.com/playlist?list={id}"))
            }
            _ => None,
        }
    }

    /// A short name for messages and collision suffixes.
    #[must_use]
    pub fn short(&self) -> String {
        match self {
            Self::Remote { id, .. } => id.clone(),
            Self::Manual(key) => key
                .path()
                .file_stem()
                .map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
        }
    }
}

impl fmt::Display for SourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Remote { extractor, id } => write!(f, "{extractor}:{id}"),
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
            "youtube:vid00000001",
            "soundcloud:12345",
            "manual:A/b c.flac",
        ] {
            assert_eq!(SourceKey::parse(text).unwrap().to_string(), text);
        }
    }

    #[test]
    fn a_manual_path_stays_inside_its_folder() {
        assert!(SourceKey::parse("manual:../x.flac").is_err());
        assert!(SourceKey::parse("manual:/x.flac").is_err());
        assert!(SourceKey::parse("manual:").is_err());
    }

    #[test]
    fn a_youtube_key_needs_a_video_id() {
        assert!(SourceKey::parse("youtube:short").is_err());
        assert!(SourceKey::parse("nothing").is_err());
        assert!(SourceKey::parse("youtube:-dashid_123").is_ok());
    }

    #[test]
    fn only_youtube_can_be_fetched_again() {
        assert_eq!(
            SourceKey::youtube("vid00000001").url().as_deref(),
            Some("https://www.youtube.com/watch?v=vid00000001")
        );
        assert!(SourceKey::parse("soundcloud:1").unwrap().url().is_none());
        assert_eq!(
            SourceKey::playlist("OLAK5uy_x").url().as_deref(),
            Some("https://www.youtube.com/playlist?list=OLAK5uy_x")
        );
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
        assert_eq!(json, "\"youtube:vid00000001\"");
        assert_eq!(serde_json::from_str::<SourceKey>(&json).unwrap(), key);
    }
}
