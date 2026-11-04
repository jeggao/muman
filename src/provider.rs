//! The providers sources come from, by name: what each is called in
//! the song list and in a source's key, what it finds and yields, and
//! whether muman keeps its records. Each provider's settings are
//! `providers.rs`'s.

use std::fmt;

use anyhow::{Context, Result};

use crate::source::SourceKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Provider {
    /// Files of the user's own, and torrents' files: what their own tags
    /// say; never looked up.
    Manual,
    /// A YouTube video that is not a release.
    YouTube,
    /// A YouTube Music release, a track with its album.
    YouTubeMusic,
    /// Lyrics from lrclib.net.
    Lrclib,
    /// Tags from musicbrainz.org.
    MusicBrainz,
    /// MusicBrainz recordings found by fingerprint on acoustid.org.
    AcoustId,
    /// Covers from the Cover Art Archive.
    CoverArt,
}

impl Provider {
    pub const ALL: [Self; 7] = [
        Self::Manual,
        Self::YouTube,
        Self::YouTubeMusic,
        Self::Lrclib,
        Self::MusicBrainz,
        Self::AcoustId,
        Self::CoverArt,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::YouTube => "youtube",
            Self::YouTubeMusic => "youtube-music",
            Self::Lrclib => "lrclib",
            Self::MusicBrainz => "musicbrainz",
            Self::AcoustId => "acoustid",
            Self::CoverArt => "coverart",
        }
    }

    pub fn named(name: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.name() == name)
            .with_context(|| {
                format!(
                    "`{name}` is no provider: {}",
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }

    /// Whether a song can look a source of this provider up.
    #[must_use]
    pub fn findable(self) -> bool {
        self != Self::Manual
    }

    /// The provider of the sources it finds.
    #[must_use]
    pub fn yields(self) -> Self {
        match self {
            Self::AcoustId => Self::MusicBrainz,
            p => p,
        }
    }

    /// Whether `[providers.*] url` names its server.
    pub(crate) fn served(self) -> bool {
        self.kept() || self == Self::AcoustId
    }

    /// The keys its `[providers.*]` table takes.
    #[must_use]
    pub fn keys(self) -> Vec<&'static str> {
        let mut keys = vec!["enabled", "concurrency", "recheck", "per_run"];
        if self.findable() {
            keys.extend(["until", "skip"]);
        }
        if self.served() {
            keys.push("url");
        }
        if self == Self::AcoustId {
            keys.push("key");
        }
        keys
    }

    /// Whether its sources are records muman keeps from a lookup, which
    /// `sync` fetches again by ID when gone.
    #[must_use]
    pub fn kept(self) -> bool {
        crate::store::KEPT.iter().any(|k| k.provider == self)
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The extractor name LRCLIB's keys carry: `lrclib:<id>`.
pub const LRCLIB: &str = "lrclib";
/// The extractor name MusicBrainz's keys carry: `musicbrainz:<recording id>`.
pub const MUSICBRAINZ: &str = "musicbrainz";
/// The extractor name the Cover Art Archive's keys carry:
/// `coverart:<release group or release id>`.
pub const COVERART: &str = "coverart";

/// Whether `key` is a record muman keeps from a lookup, as
/// [`Provider::kept`] says.
#[must_use]
pub fn is_kept(key: &SourceKey) -> bool {
    matches!(key, SourceKey::Remote { site, .. } if crate::store::kept(site).is_some())
}
