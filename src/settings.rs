//! What `songs.toml` sets beyond its songs: how the library is laid out
//! and named, what bitrate encodes, how yt-dlp fetches, and how much
//! history `undo` keeps.
//!
//! These live in the song list rather than in a per-user file so that
//! one list renders one library, the same on every machine. The tables
//! are `[library]`, `[audio]`, `[ytdlp]` and `[history]`; every key is
//! optional, and `new.toml`, the file a new home starts from, lists each
//! with its default. A key these tables do not know is an error naming
//! it, so a misspelled setting is never silently ignored. Which machine
//! runs what is no setting: tools and folders come from flags and
//! `MUMAN_*` variables.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use toml_edit::DocumentMut;

/// The tables read here; every other top-level key belongs to the song
/// list proper.
pub const TABLES: [&str; 4] = ["library", "audio", "ytdlp", "history"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub library: Library,
    pub audio: Audio,
    pub ytdlp: Ytdlp,
    pub history: History,
}

/// The library's place and each song's path in it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Library {
    /// The library folder: absolute, `~/…`, or relative to the home.
    /// `--library` and `MUMAN_LIBRARY` override it on one machine.
    pub path: Option<String>,
    /// Each song's path without its extension, as a MiniJinja template;
    /// `/` separates folders. See [`crate::template`].
    pub template: String,
    /// Characters replaced in every tag value before it is put in a
    /// path, merged over the defaults.
    pub replace: BTreeMap<String, String>,
    pub restrict: Restrict,
    /// The longest file name kept, in UTF-8 bytes, which bounds it on
    /// every filesystem: no character takes more UTF-16 units than bytes.
    pub max_name_bytes: usize,
    pub max_folder_bytes: usize,
    /// The longest whole path, library folder included, in characters;
    /// the title is cut to fit. Windows programs and players without
    /// long-path support stop at 260.
    pub max_path: Option<usize>,
    pub unknown_artist: String,
    pub unknown_album: String,
    pub untitled: String,
    pub lyrics: LyricsPlacement,
    /// Set the library folder's time to now after a run that changed it,
    /// for players that rescan only when it is newer than their last scan.
    pub touch_root: bool,
}

/// `<album artist>/<album>/<NN title>`, `<D-NN title>` from a second
/// disc on: a single on no album is named for its title.
pub const DEFAULT_TEMPLATE: &str = "{{ album_artist }}/{{ album }}/{{ disc_track }}{{ title }}";

impl Default for Library {
    fn default() -> Self {
        Self {
            path: None,
            template: DEFAULT_TEMPLATE.to_string(),
            replace: BTreeMap::new(),
            restrict: Restrict::default(),
            max_name_bytes: 200,
            max_folder_bytes: 120,
            max_path: None,
            unknown_artist: "Unknown Artist".into(),
            unknown_album: "Unknown Album".into(),
            untitled: "Untitled".into(),
            lyrics: LyricsPlacement::default(),
            touch_root: false,
        }
    }
}

impl Library {
    /// The folder `path` names, relative to `home` when relative.
    #[must_use]
    pub fn folder(&self, home: &Path) -> Option<PathBuf> {
        let path = self.path.as_deref()?.trim();
        let expanded = match path.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
                directories::UserDirs::new()?
                    .home_dir()
                    .join(rest.trim_start_matches(['/', '\\']))
            }
            _ => PathBuf::from(path),
        };
        Some(if expanded.is_absolute() {
            expanded
        } else {
            home.join(expanded)
        })
    }
}

/// How strictly names are made safe, beyond replacing `/` and control
/// characters, which every filesystem refuses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Restrict {
    /// Only what this filesystem refuses.
    None,
    /// What Windows, FAT and most phones refuse too, on every system, so
    /// a library copies anywhere.
    #[default]
    Windows,
    /// As `windows`, and everything transliterated to ASCII.
    Ascii,
}

/// Where lyrics are written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsPlacement {
    /// A `.lrc` file beside the song, which most players read.
    #[default]
    Sidecar,
    /// The `LYRICS` tag inside the song.
    Embedded,
    Both,
}

impl LyricsPlacement {
    #[must_use]
    pub fn sidecar(self) -> bool {
        matches!(self, Self::Sidecar | Self::Both)
    }

    #[must_use]
    pub fn embedded(self) -> bool {
        matches!(self, Self::Embedded | Self::Both)
    }
}

/// How audio that is not copied is encoded.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Audio {
    /// Opus bitrate for mono and stereo, in kbit/s; at or above what
    /// YouTube serves, so encoding loses nothing audible.
    pub opus_kbps: u32,
    /// Opus bitrate for more than two channels.
    pub opus_surround_kbps: u32,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            opus_kbps: 160,
            opus_surround_kbps: 256,
        }
    }
}

/// How yt-dlp fetches a source.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ytdlp {
    /// yt-dlp's `--format`: what is kept of a video.
    pub format: String,
    /// yt-dlp's `--sub-langs`: the subtitles kept for lyrics.
    pub sub_langs: String,
    pub concurrent_fragments: u32,
    /// Load muman's two postprocessors, which keep generated captions
    /// out and take a YouTube Music track's square album art.
    pub plugins: bool,
    /// More arguments, such as `["--cookies-from-browser", "firefox"]`.
    pub args: Vec<String>,
    /// Days an unfinished download is kept to resume.
    pub partial_days: u64,
}

impl Default for Ytdlp {
    fn default() -> Self {
        Self {
            format: "bv*+ba/b".into(),
            sub_langs: "all,-live_chat".into(),
            concurrent_fragments: 4,
            plugins: true,
            args: Vec::new(),
            partial_days: 14,
        }
    }
}

/// What `undo` can put back.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct History {
    /// The changing runs kept.
    pub runs: usize,
    /// The most the kept runs' replaced files may take, in MiB.
    pub max_mib: u64,
}

impl Default for History {
    fn default() -> Self {
        Self {
            runs: 3,
            max_mib: 2048,
        }
    }
}

/// The library folder the song list in `home` names, if it names one.
pub fn library_folder(home: &Path) -> Result<Option<PathBuf>> {
    let file = home.join(crate::dirs::MANIFEST);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    let doc: DocumentMut = text
        .parse()
        .with_context(|| format!("{} is not valid TOML", file.display()))?;
    Ok(read(&doc)?.library.folder(home))
}

/// The settings `doc` sets, the defaults where it sets none.
pub fn read(doc: &DocumentMut) -> Result<Settings> {
    let mut only = DocumentMut::new();
    for table in TABLES {
        if let Some(item) = doc.get(table) {
            only.insert(table, item.clone());
        }
    }
    toml::from_str(&only.to_string()).context("reading the settings")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(text: &str) -> Result<Settings> {
        read(&text.parse().unwrap())
    }

    #[test]
    fn nothing_set_is_every_default() {
        assert_eq!(settings("version = 1\n").unwrap(), Settings::default());
    }

    #[test]
    fn a_table_sets_only_what_it_names() {
        let s = settings(
            "[library]\ntemplate = \"{{ artist }}/{{ title }}\"\nlyrics = \"both\"\n\
             [audio]\nopus_kbps = 192\n",
        )
        .unwrap();
        assert_eq!(s.library.template, "{{ artist }}/{{ title }}");
        assert_eq!(s.library.lyrics, LyricsPlacement::Both);
        assert_eq!(s.library.untitled, "Untitled");
        assert_eq!(s.audio.opus_kbps, 192);
        assert_eq!(s.audio.opus_surround_kbps, 256);
    }

    #[test]
    fn a_misspelled_key_is_named() {
        let e = settings("[audio]\nopus_kpbs = 192\n").unwrap_err();
        assert!(format!("{e:#}").contains("opus_kpbs"), "{e:#}");
        assert!(settings("[library]\nrestrict = \"dos\"\n").is_err());
    }

    #[test]
    fn a_relative_library_is_under_the_home() {
        let library = Library {
            path: Some("music".into()),
            ..Library::default()
        };
        let home = Path::new("home");
        assert_eq!(library.folder(home), Some(home.join("music")));
        assert_eq!(Library::default().folder(home), None);
    }
}
