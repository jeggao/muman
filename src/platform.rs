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

use std::path::PathBuf;

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
