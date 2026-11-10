//! The names of the files a song has beside its audio file in the
//! library: its lyrics and its cover, in each file a `[[lyrics.out]]` or a
//! `[[cover.out]]` names, and the cover file its folder holds once, where
//! the song holds it for the folder ([`crate::folder`]). Which files a
//! plan writes is its parts' to say (`artifact::Parts::files`).
//!
//! Each is named by the song's audio path without its extension and a
//! suffix of its own, so the song's files read as one in a folder, and a
//! song moved takes each to the same suffix at its new path; a folder's
//! file is named in the folder ([`of_folder`]), and keeps its name in the
//! folder the song moves to, unless the user changed it, when it stays
//! ([`crate::folder`]). Every name a
//! file beside a song is given, and every one a move gives it, comes from
//! here: `tests/architecture.rs` holds every other module to deriving
//! none. A suffix the song list gives is checked by its name
//! ([`crate::naming::check_suffix`]): a dot and plain text, no folder and nothing a
//! filesystem refuses, so a file beside a song stays beside it, and a name
//! never reaches past the library folder, as into the home's sources,
//! where a sync would read it as one. [`crate::state::Sidecar`] records one as it was written, with
//! its size, time and digest, as an audio file is recorded, so one changed
//! since is the user's and neither written over nor removed.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::resolve::{CoverPlace, LyricsFile};

/// The file of lyrics `file` beside the song whose path without its
/// extension is `stem`, which may itself hold a dot: the suffix the
/// `[[lyrics.out]]` names, then its format's extension.
#[must_use]
pub fn lyrics_file(stem: &Path, file: &LyricsFile) -> PathBuf {
    beside(
        stem,
        &format!("{}.{}", file.suffix, file.format.extension()),
    )
}

/// The file of the cover `place` beside the song whose path without its
/// extension is `stem`: the suffix the `[[cover.out]]` names, then its
/// format's extension; none for a place inside the song.
#[must_use]
pub fn cover_file(stem: &Path, place: &CoverPlace) -> Option<PathBuf> {
    let suffix = place.file.as_ref()?;
    Some(beside(
        stem,
        &format!("{suffix}.{}", place.format.extension()),
    ))
}

/// The file of a folder's cover `place` held by the song whose path
/// without its extension is `stem`: the name the `[[cover.out]]` gives,
/// then its format's extension, in the song's folder; none for a place
/// inside the song.
#[must_use]
pub fn folder_file(stem: &Path, place: &CoverPlace) -> Option<PathBuf> {
    let name = place.file.as_ref()?;
    let folder = stem.parent().unwrap_or(Path::new(""));
    Some(folder.join(format!("{name}.{}", place.format.extension())))
}

/// Where `sidecar`, beside the song at `from`, goes when the song moves
/// to `to`: the same suffix after the new stem. One whose name begins
/// with the song's only as a filesystem blind to case reports it keeps
/// its extension alone; one whose name does not, a folder's file, keeps
/// its name in the song's new folder.
#[must_use]
pub fn moved(from: &Path, to: &Path, sidecar: &Path) -> PathBuf {
    let stem = from.with_extension("");
    let (Some(name), Some(stem)) = (sidecar.file_name(), stem.file_name()) else {
        return beside(&to.with_extension(""), "");
    };
    let suffix = name
        .to_str()
        .zip(stem.to_str())
        .and_then(|(name, stem)| name.strip_prefix(stem))
        .filter(|rest| rest.starts_with('.'))
        .map(str::to_string);
    if let Some(suffix) = suffix {
        return beside(&to.with_extension(""), &suffix);
    }
    if of_folder(from, sidecar) {
        return to.parent().unwrap_or(Path::new("")).join(name);
    }
    let extension = sidecar
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    beside(&to.with_extension(""), &extension)
}

/// Whether `sidecar`, beside the song at `from`, is a folder's file: one
/// whose name does not begin with the song's, as a filesystem blind to
/// case reports it.
#[must_use]
pub fn of_folder(from: &Path, sidecar: &Path) -> bool {
    use crate::relpath::folded;
    let (Some(name), Some(stem)) = (
        sidecar.file_name(),
        from.with_extension("").file_name().map(ToOwned::to_owned),
    ) else {
        return false;
    };
    let mut dotted = stem;
    dotted.push(".");
    !folded(Path::new(name)).starts_with(&folded(Path::new(&dotted)))
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
    fn a_folder_s_file_is_named_in_the_folder_and_keeps_its_name() {
        let place = CoverPlace {
            file: Some("cover".into()),
            ..CoverPlace::default()
        };
        let folder = Path::new("Marlo Venn").join("Vol. 2");
        let song = folder.join("03 Lantern.flac");
        let file = folder_file(&song.with_extension(""), &place).unwrap();
        assert_eq!(file, folder.join("cover.png"));
        let to = Path::new("Vol. 2").join("03 Lantern.flac");
        assert_eq!(
            moved(&song, &to, &file),
            Path::new("Vol. 2").join("cover.png")
        );
        let lrc = folder.join("03 lantern.lrc");
        assert_eq!(
            moved(&song, &to, &lrc),
            Path::new("Vol. 2").join("03 Lantern.lrc"),
            "a song's own, spelled otherwise"
        );
    }

    #[test]
    fn a_file_beside_a_song_is_named_by_its_stem_and_moves_with_it() {
        let song = Path::new("Marlo Venn")
            .join("Vol. 2")
            .join("03 Lantern.flac");
        let lrc = Path::new("Marlo Venn")
            .join("Vol. 2")
            .join("03 Lantern.lrc");
        assert_eq!(
            lyrics_file(&song.with_extension(""), &LyricsFile::default()),
            lrc
        );
        let en = LyricsFile {
            suffix: ".en".into(),
            ..LyricsFile::default()
        };
        assert_eq!(
            lyrics_file(&Path::new("A").join("01 Paper Comets, Vol. 2"), &en),
            Path::new("A").join("01 Paper Comets, Vol. 2.en.lrc")
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
