//! The folders a run works in: the state root, with the song list, the
//! state file and the sources, and the library it writes.

use std::path::{Path, PathBuf};

pub const MANIFEST: &str = "songs.toml";
pub const STATE: &str = "state.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    pub home: PathBuf,
    pub library: PathBuf,
}

impl Dirs {
    #[must_use]
    pub fn manifest(&self) -> PathBuf {
        self.home.join(MANIFEST)
    }

    #[must_use]
    pub fn state(&self) -> PathBuf {
        self.home.join(STATE)
    }

    #[must_use]
    pub fn ytdlp(&self) -> PathBuf {
        self.home.join("sources").join("yt-dlp")
    }

    /// A folder of sources, by its name in `sources`.
    #[must_use]
    pub fn sources(&self, folder: &str) -> PathBuf {
        self.home.join("sources").join(folder)
    }

    #[must_use]
    pub fn lrclib(&self) -> PathBuf {
        self.kept(crate::provider::LRCLIB)
    }

    #[must_use]
    pub fn musicbrainz(&self) -> PathBuf {
        self.kept(crate::provider::MUSICBRAINZ)
    }

    /// The folder records of a kept scheme lie in.
    #[must_use]
    pub fn kept(&self, extractor: &str) -> PathBuf {
        crate::store::kept(extractor).map_or_else(|| self.sources(extractor), |k| k.dir(self))
    }

    #[must_use]
    pub fn manual(&self) -> PathBuf {
        self.home.join("sources").join("manual")
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.home
    }
}
