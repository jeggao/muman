//! The files a song has beside its audio file in the library, and their
//! names: its lyrics and its cover, in each file a `[[lyrics.out]]` or a
//! `[[cover.out]]` names, and the cover file its folder holds once, where
//! the song holds it for the folder ([`crate::folder`]).
//!
//! Each is named by the song's audio path without its extension and a
//! suffix of its own, so the song's files read as one in a folder, and a
//! song moved takes each to the same suffix at its new path; a folder's
//! file is named in the folder, and keeps its name in the folder the song
//! moves to. Every name a
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

use crate::resolve::{CoverPlace, CoverRef, LyricsFile, LyricsRef, Plan};
use crate::settings::{Settings, To};
use crate::state::Written;

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

/// The files `plan` writes beside the song whose audio file is at
/// `audio`: its lyrics', its cover's, then its folder's.
#[must_use]
pub fn planned(audio: &Path, plan: &Plan) -> Vec<PathBuf> {
    let stem = audio.with_extension("");
    let lyrics = plan
        .lyrics
        .iter()
        .flat_map(LyricsRef::files)
        .map(|f| lyrics_file(&stem, &f));
    let covers = plan
        .cover
        .iter()
        .flat_map(CoverRef::files)
        .filter_map(|p| cover_file(&stem, &p));
    lyrics
        .chain(covers)
        .chain(folder_files(audio, plan))
        .collect()
}

/// The files of its folder `plan` writes, for the song whose audio file
/// is at `audio`.
#[must_use]
pub fn folder_files(audio: &Path, plan: &Plan) -> Vec<PathBuf> {
    let stem = audio.with_extension("");
    plan.folder
        .iter()
        .flat_map(CoverRef::files)
        .filter_map(|p| folder_file(&stem, &p))
        .collect()
}

/// The files beside the song at `audio` that writing it may write or
/// clear: those `plan` writes, and those `recorded` there as its last
/// build's. A file of the user's own at any other name is not the song's
/// to touch.
#[must_use]
pub fn touched(audio: &Path, plan: &Plan, recorded: Option<&Written>) -> Vec<PathBuf> {
    let mut files = planned(audio, plan);
    for path in recorded.into_iter().flat_map(Written::sidecar_paths) {
        if !files.contains(path) {
            files.push(path.clone());
        }
    }
    files
}

/// The bytes the longest tail after a song's stem takes: its audio's
/// extension, or a file beside it, as `[[lyrics.out]]` and
/// `[[cover.out]]` name them.
#[must_use]
pub fn tail(settings: &Settings) -> usize {
    let lyrics = settings
        .lyrics
        .out
        .iter()
        .filter(|o| o.to == To::File)
        .map(|o| o.suffix.len() + 1 + o.format.extension().len());
    let covers = settings
        .cover
        .of(crate::settings::Scope::Song)
        .filter(|o| o.to == To::File)
        .flat_map(|o| {
            o.format
                .extensions()
                .iter()
                .map(|e| o.suffix.len() + 1 + e.len())
        });
    lyrics
        .chain(covers)
        .fold(crate::naming::AUDIO_TAIL, usize::max)
}

/// Where `sidecar`, beside the song at `from`, goes when the song moves
/// to `to`: the same suffix after the new stem. One whose name begins
/// with the song's only as a filesystem blind to case reports it keeps
/// its extension alone; one whose name does not, a folder's file, keeps
/// its name in the song's new folder.
#[must_use]
pub fn moved(from: &Path, to: &Path, sidecar: &Path) -> PathBuf {
    use crate::relpath::folded;
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
    let mut dotted = stem.to_os_string();
    dotted.push(".");
    if !folded(Path::new(name)).starts_with(&folded(Path::new(&dotted))) {
        return to.parent().unwrap_or(Path::new("")).join(name);
    }
    let extension = sidecar
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    beside(&to.with_extension(""), &extension)
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
