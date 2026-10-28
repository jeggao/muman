//! The files a song has beside its audio file in the library, and their
//! names: today its lyrics, a `.lrc`.
//!
//! Each is named by the song's audio path without its extension and a
//! suffix of its own, so the song's files read as one in a folder, and a
//! song moved takes each to the same suffix at its new path. Every name a
//! file beside a song is given, and every one a move gives it, comes from
//! here: `tests/architecture.rs` holds every other module to deriving
//! none. A suffix the song list gives is checked here too
//! ([`check_suffix`]): a dot and plain text, no folder and nothing a
//! filesystem refuses, so a file beside a song stays beside it, and a name
//! never reaches past the library folder, as into the home's sources,
//! where a sync would read it as one. [`crate::state::Sidecar`] records one as it was written, with
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

/// The most bytes a suffix the song list gives may take.
pub const MOST_SUFFIX: usize = 24;

/// Whether `suffix`, what follows a song's stem in a file beside it, as
/// `.en` in `Song.en.lrc` or `.lrc` itself, names a file beside the song
/// on every system: empty, or a dot and up to [`MOST_SUFFIX`] bytes in
/// all of what any filesystem takes in a name. Why not, if not.
pub fn check_suffix(suffix: &str) -> Result<(), String> {
    if suffix.is_empty() {
        return Ok(());
    }
    if !suffix.starts_with('.') {
        return Err(format!("`{suffix}` must begin with a dot, as `.en`"));
    }
    plain(suffix)
}

/// Whether `name`, a file a folder of songs holds once, as `cover`, is
/// one name of its own on every system, before its extension. Why not, if
/// not.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.starts_with('.') {
        return Err(format!(
            "`{name}` must name a file, not begin with a dot or be empty"
        ));
    }
    if name.ends_with([' ', '.']) || crate::naming::reserved(name) {
        return Err(format!("`{name}` is a name Windows refuses"));
    }
    plain(name)
}

/// `text` of nothing a name refuses on any system, within
/// [`MOST_SUFFIX`] bytes.
fn plain(text: &str) -> Result<(), String> {
    if text.len() > MOST_SUFFIX {
        return Err(format!(
            "`{text}` is over {MOST_SUFFIX} bytes, the most a name keeps for it"
        ));
    }
    if let Some(c) = text
        .chars()
        .find(|c| c.is_control() || "/\\:*?\"<>|".contains(*c))
    {
        return Err(format!("`{text}` holds {c:?}, which a file name cannot"));
    }
    Ok(())
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
    fn a_suffix_or_a_folder_s_file_is_one_plain_name() {
        for good in ["", ".en", ".lrc", ".zh-Hant"] {
            assert_eq!(check_suffix(good), Ok(()), "{good}");
        }
        for bad in [
            "en",
            "./x",
            ".a/b",
            ".a\\b",
            ".a:b",
            ".\u{7}",
            &".x".repeat(13),
        ] {
            assert!(check_suffix(bad).is_err(), "{bad}");
        }
        assert_eq!(check_name("cover"), Ok(()));
        assert_eq!(check_name("folder"), Ok(()));
        for bad in ["", ".cover", "../cover", "a/b", "CON", "cover.", "cover "] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

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
