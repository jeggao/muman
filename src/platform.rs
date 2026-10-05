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
