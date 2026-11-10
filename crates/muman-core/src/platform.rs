//! Where muman keeps its files by default, by each platform's convention.
//!
//! | | Home: song list, state, sources | Library |
//! |---|---|---|
//! | Linux | `$XDG_DATA_HOME/muman` | `$XDG_MUSIC_DIR/muman` |
//! | macOS | `~/Library/Application Support/muman` | `~/Music/muman` |
//! | Windows | `%LOCALAPPDATA%\muman\data` | `%USERPROFILE%\Music\muman` |
//!
//! On Linux and the other systems that follow XDG, the music folder is
//! `XDG_MUSIC_DIR` from the environment when it is set to an absolute
//! path, as the XDG user-dirs convention lets a session override the
//! file; else the one `user-dirs.dirs` names; else `~/Music`, as on a
//! server without that file. A relative or empty value is ignored, as the
//! XDG base-directory specification asks of its own variables. macOS and
//! Windows have no such variable, and their music folder is the system's.
//!
//! The home is in the local, not the roaming, application data on
//! Windows: its sources run to gigabytes, which a roaming profile would
//! copy at every sign-in. The library's lock is kept under the cache
//! folder, in its `locks` folder, as [`crate::atomic`] says why.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

/// The folder name under each platform's data and music folders.
const NAME: &str = "muman";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    pub home: PathBuf,
    pub library: PathBuf,
    /// Where files muman can make again are kept: the yt-dlp plugins.
    pub cache: Option<PathBuf>,
}

/// The platform's folders; `None` when it names no home directory.
#[must_use]
pub fn defaults() -> Option<Defaults> {
    let project = directories::ProjectDirs::from("", "", NAME)?;
    let user = directories::UserDirs::new()?;
    let from_env = if cfg!(any(target_os = "macos", windows)) {
        None
    } else {
        std::env::var_os("XDG_MUSIC_DIR")
    };
    let music = music_folder(from_env, user.audio_dir(), user.home_dir());
    Some(Defaults {
        home: project.data_local_dir().to_path_buf(),
        library: music.join(NAME),
        cache: Some(project.cache_dir().to_path_buf()),
    })
}

/// The music folder: `from_env`, the XDG variable, when it is absolute;
/// else the one the system names; else `Music` in the home.
fn music_folder(from_env: Option<OsString>, named: Option<&Path>, home: &Path) -> PathBuf {
    from_env
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| named.map(Path::to_path_buf))
        .unwrap_or_else(|| home.join("Music"))
}

/// When a file arrived where it is: the later of its modification time
/// and the time it was created there. A copy by Explorer, Finder or
/// `cp -p` keeps the source's modification time, so that alone would
/// make a file still being copied in look long settled. Unix has no
/// creation time everywhere, but the status-change time moves on any
/// copy or rename.
#[must_use]
pub fn arrived(meta: &std::fs::Metadata) -> Option<std::time::SystemTime> {
    let modified = meta.modified().ok();
    #[cfg(unix)]
    let placed = {
        use std::os::unix::fs::MetadataExt;
        u64::try_from(meta.ctime()).ok().map(|secs| {
            std::time::UNIX_EPOCH
                + std::time::Duration::new(secs, u32::try_from(meta.ctime_nsec()).unwrap_or(0))
        })
    };
    #[cfg(not(unix))]
    let placed = meta.created().ok();
    modified.max(placed)
}

/// `path` resolved through links as far as it exists, and the rest as
/// written: a folder not made yet under a linked one, as macOS's `/var`
/// is, resolves through that link as the folder made would.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        if let Ok(real) = dunce::canonicalize(at) {
            return rest.iter().rev().fold(real, |p, name| p.join(name));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                at = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// `path`, root and drive kept, as a filesystem blind to case and
/// composition tells it apart: in NFC and upper case, as
/// [`crate::relpath::folded`] says why.
fn folded(path: &Path) -> String {
    path.to_string_lossy()
        .nfc()
        .collect::<String>()
        .to_uppercase()
}

/// Whether `inner` is `outer` or a folder within it, resolved and
/// compared as [`same_path`] compares.
#[must_use]
pub fn within(inner: &Path, outer: &Path) -> bool {
    let (inner, outer) = (resolved(inner), resolved(outer));
    if cfg!(any(windows, target_os = "macos")) {
        PathBuf::from(folded(&inner)).starts_with(folded(&outer))
    } else {
        inner.starts_with(outer)
    }
}

/// Whether two paths name the same folder: resolved through links and
/// `.`/`..`, and without regard to case where the platform's filesystems
/// ignore it, so `D:\Music` and `d:\music\` agree.
#[must_use]
pub fn same_path(a: &Path, b: &Path) -> bool {
    let (a, b) = (resolved(a), resolved(b));
    if cfg!(any(windows, target_os = "macos")) {
        folded(&a) == folded(&b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_music_folder_in_the_environment_comes_before_the_named_one() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let named = home.join("Tunes");
        let set = dir.path().join("elsewhere");
        let pick = |from_env: Option<&Path>, named: Option<&Path>| {
            music_folder(from_env.map(OsString::from), named, &home)
        };
        assert_eq!(pick(Some(&set), Some(&named)), set);
        assert_eq!(pick(None, Some(&named)), named);
        assert_eq!(pick(Some(Path::new("Tunes")), Some(&named)), named);
        assert_eq!(pick(Some(Path::new("")), None), home.join("Music"));
        assert_eq!(pick(None, None), home.join("Music"));
    }

    #[test]
    fn a_folder_is_within_itself_and_its_parents_only() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().join("home").join("sources");
        std::fs::create_dir_all(&outer).unwrap();
        assert!(within(&outer, &outer));
        assert!(within(&outer.join("manual").join("not yet made"), &outer));
        assert!(!within(&dir.path().join("home"), &outer));
        assert!(!within(&dir.path().join("homes"), &dir.path().join("home")));
    }

    #[test]
    fn a_file_with_an_old_modification_time_arrived_now() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.flac");
        std::fs::write(&file, b"x").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600 * 24 * 365);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let meta = std::fs::metadata(&file).unwrap();
        let arrived = arrived(&meta).unwrap();
        assert!(arrived.elapsed().unwrap() < std::time::Duration::from_secs(60));
    }

    #[test]
    fn a_folder_reached_two_ways_is_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        std::fs::create_dir_all(dir.path().join("x")).unwrap();
        std::fs::create_dir(&lib).unwrap();
        assert!(same_path(
            &lib,
            &dir.path().join("x").join("..").join("lib")
        ));
        assert!(!same_path(&lib, dir.path()));
    }

    #[test]
    fn a_path_folds_as_a_case_blind_filesystem_does() {
        assert_eq!(folded(Path::new("Οδός")), folded(Path::new("οδόσ")));
        assert_eq!(folded(Path::new("Cafe\u{301}")), folded(Path::new("CAFÉ")));
        assert_ne!(folded(Path::new("Song")), folded(Path::new("Songs")));
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn case_does_not_tell_folders_apart_where_the_filesystem_ignores_it() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("Lib");
        std::fs::create_dir(&lib).unwrap();
        assert!(same_path(&lib, &dir.path().join("lib")));
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_not_made_yet_resolves_through_a_link_above_it() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(real.join("home")).unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join("link")).unwrap();
        let ahead = dir.path().join("link").join("home").join("not yet made");
        assert!(within(&ahead, &real.join("home")));
        assert!(same_path(&ahead, &real.join("home").join("not yet made")));
    }
}
