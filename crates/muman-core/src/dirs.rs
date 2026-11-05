//! The folders a run works in: the home, with the song list, the state
//! file and the sources, and the library it writes. Neither may hold the
//! other, but for a library beside the sources in the home.

use std::path::{Path, PathBuf};

pub const MANIFEST: &str = "songs.toml";
pub const STATE: &str = "state.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    pub home: PathBuf,
    pub library: PathBuf,
    /// The folder of the user's own the library's lock is kept in, under
    /// its cache folder, to leave no file in the library; `None` where the
    /// platform names no home directory. Tests keep it beside their own
    /// library, and read no folder of the machine's.
    pub locks: Option<PathBuf>,
}

impl Dirs {
    /// [`Self::locks`], or why a library cannot be locked without one.
    pub fn lock_folder(&self) -> anyhow::Result<&Path> {
        use anyhow::Context;
        self.locks
            .as_deref()
            .context("this system names no folder of the user's own to keep the library's lock in")
    }

    /// Refuse folders that hold each other where muman would read its
    /// own songs back as new ones, or skip every file of the user's as
    /// one of the library's.
    pub fn apart(&self) -> anyhow::Result<()> {
        use crate::platform::{same_path, within};
        if same_path(&self.library, &self.home) {
            anyhow::bail!(
                "the library and the home are one folder, {}, where muman would read the \
                 songs it writes back as new ones; choose a library outside the home",
                self.home.display()
            );
        }
        if within(&self.library, &self.home.join("sources")) {
            anyhow::bail!(
                "the library {} is in the home's sources, where muman would read the songs it \
                 writes back as new ones; choose a folder outside them",
                self.library.display()
            );
        }
        if within(&self.home, &self.library) {
            anyhow::bail!(
                "the home {} is in the library {}, where muman would take your files for the \
                 library's and read none of them; choose a home outside the library",
                self.home.display(),
                self.library.display()
            );
        }
        Ok(())
    }

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

    /// Torrents' metainfo and their files, each torrent's in a folder of
    /// its hash.
    #[must_use]
    pub fn torrent(&self) -> PathBuf {
        self.sources(crate::source::TORRENT)
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
