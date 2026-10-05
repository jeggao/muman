//! Where muman keeps its files by default, by each platform's convention.
//!
//! | | Home: song list, state, sources | Library |
//! |---|---|---|
//! | Linux | `$XDG_DATA_HOME/muman` | `$XDG_MUSIC_DIR/muman` |
//! | macOS | `~/Library/Application Support/muman` | `~/Music/muman` |
//! | Windows | `%LOCALAPPDATA%\muman\data` | `%USERPROFILE%\Music\muman` |
//!
//! The home is in the local, not the roaming, application data on
//! Windows: its sources run to gigabytes, which a roaming profile would
//! copy at every sign-in.

use std::path::{Path, PathBuf};

/// The folder name under each platform's data and music folders.
const NAME: &str = "muman";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    pub home: PathBuf,
    pub library: PathBuf,
}

/// The platform's folders; `None` when it names no home directory.
#[must_use]
pub fn defaults() -> Option<Defaults> {
    let project = directories::ProjectDirs::from("", "", NAME)?;
    let user = directories::UserDirs::new()?;
    // Linux without `user-dirs.dirs`, as on a server, names no music folder.
    let music = user
        .audio_dir()
        .map_or_else(|| user.home_dir().join("Music"), PathBuf::from);
    Some(Defaults {
        home: project.data_local_dir().to_path_buf(),
        library: music.join(NAME),
    })
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

/// Whether two paths name the same folder: resolved through links and
/// `.`/`..`, and without regard to case where the platform's filesystems
/// ignore it, so `D:\Music` and `d:\music\` agree.
#[must_use]
pub fn same_path(a: &Path, b: &Path) -> bool {
    let resolve = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (a, b) = (resolve(a), resolve(b));
    if cfg!(any(windows, target_os = "macos")) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn case_does_not_tell_folders_apart_where_the_filesystem_ignores_it() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("Lib");
        std::fs::create_dir(&lib).unwrap();
        assert!(same_path(&lib, &dir.path().join("lib")));
    }
}
