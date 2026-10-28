//! The files a song has beside its audio file in the library, and their
//! names: today its lyrics, a `.lrc`.
//!
//! Each is named by the song's audio path without its extension and a
//! suffix of its own, so the song's files read as one in a folder, and a
//! song moved takes each to the same suffix at its new path. Every name a
//! file beside a song is given, and every one a move gives it, comes from
//! here: `tests/architecture.rs` holds every other module to deriving
//! none. [`crate::state::Sidecar`] records one as it was written, with
//! its size, time and digest, as an audio file is recorded, so one changed
//! since is the user's and neither written over nor removed.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::resolve::Plan;

/// The suffix of a song's lyrics file.
const LYRICS: &str = ".lrc";

/// The lyrics file beside the song whose audio file is at `audio`.
#[must_use]
pub fn lyrics(audio: &Path) -> PathBuf {
    beside(&audio.with_extension(""), LYRICS)
}

/// The lyrics file of the song whose path without its extension is
/// `stem`, which may itself hold a dot.
#[must_use]
pub fn lyrics_of_stem(stem: &Path) -> PathBuf {
    beside(stem, LYRICS)
}

/// The files `plan` writes beside the song at `audio`: its lyrics, where
/// they go beside it.
#[must_use]
pub fn planned(audio: &Path, plan: &Plan) -> Vec<PathBuf> {
    plan.lyrics
        .iter()
        .filter(|l| l.placement.sidecar())
        .map(|_| lyrics(audio))
        .collect()
}

/// Every name a file beside the song at `audio` may take, whether or not
/// it has one: a file there that is not muman's keeps the song from
/// being written.
#[must_use]
pub fn names(audio: &Path) -> Vec<PathBuf> {
    vec![lyrics(audio)]
}

/// Where `sidecar`, beside the song at `from`, goes when the song moves
/// to `to`: the same suffix after the new stem. One whose name does not
/// begin with the song's, as a filesystem blind to case reports it, keeps
/// its extension alone.
#[must_use]
pub fn moved(from: &Path, to: &Path, sidecar: &Path) -> PathBuf {
    let stem = from.with_extension("");
    let suffix = match (sidecar.file_name(), stem.file_name()) {
        (Some(name), Some(stem)) => name
            .to_str()
            .zip(stem.to_str())
            .and_then(|(name, stem)| name.strip_prefix(stem))
            .filter(|rest| rest.starts_with('.'))
            .map(str::to_string),
        _ => None,
    };
    let suffix = suffix.unwrap_or_else(|| {
        sidecar
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default()
    });
    beside(&to.with_extension(""), &suffix)
}

/// `stem` with `suffix` after it.
fn beside(stem: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(stem.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_beside_a_song_is_named_by_its_stem_and_moves_with_it() {
        let song = Path::new("Marlo Venn")
            .join("Vol. 2")
            .join("03 Lantern.flac");
        let lrc = Path::new("Marlo Venn")
            .join("Vol. 2")
            .join("03 Lantern.lrc");
        assert_eq!(lyrics(&song), lrc);
        assert_eq!(
            lyrics_of_stem(&Path::new("A").join("01 Paper Comets, Vol. 2")),
            Path::new("A").join("01 Paper Comets, Vol. 2.lrc")
        );
        let to = Path::new("Lantern Weather").join("Lantern.opus");
        assert_eq!(
            moved(&song, &to, &lrc),
            Path::new("Lantern Weather").join("Lantern.lrc")
        );
        let tagged = Path::new("Marlo Venn")
            .join("Vol. 2")
            .join("03 Lantern.en.lrc");
        assert_eq!(
            moved(&song, &to, &tagged),
            Path::new("Lantern Weather").join("Lantern.en.lrc")
        );
        let cased = Path::new("Marlo Venn")
            .join("Vol. 2")
            .join("03 LANTERN.lrc");
        assert_eq!(
            moved(&song, &to, &cased),
            Path::new("Lantern Weather").join("Lantern.lrc")
        );
    }
}
