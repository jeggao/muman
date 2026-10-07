//! Whole-file writes a crash cannot leave half done, the folder lock two
//! runs take before either rewrites what the other reads, and renames
//! and deletes that wait out a file another program holds open.
//!
//! On Windows a file open in a player, the search indexer or a virus
//! scanner cannot be replaced or deleted for a moment: the call fails
//! with a sharing violation. [`rename`] and [`remove`] retry those for a
//! few seconds before giving up; elsewhere they fail at once.

use std::fs::{File, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

/// The lock file in a locked folder; a regular file, since Windows
/// cannot open a folder to lock it.
pub const LOCK_FILE: &str = ".lock";

/// The lock file a command that changes the home holds throughout.
pub const RUNS_LOCK_FILE: &str = ".run.lock";

/// Write `bytes` to a temporary file in `dir`, synced, and rename it
/// over `<dir>/<name>`, so a reader sees the old file or the new. A file
/// that is a link is written where it leads, so the link stays, and a
/// file replaced keeps its permissions.
pub fn write(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    // A name of one part, so nothing a name holds writes outside `dir`.
    let plain = Path::new(name)
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
        && !name.contains(['/', '\\'])
        && !name.is_empty();
    if !plain {
        anyhow::bail!(
            "refusing to write {name:?}, which is no file name, in {}",
            dir.display()
        );
    }
    let named = dir.join(name);
    let file = match std::fs::symlink_metadata(&named) {
        Ok(m) if m.file_type().is_symlink() => dunce::canonicalize(&named).unwrap_or(named),
        _ => named,
    };
    let (dir, name) = match (file.parent(), file.file_name()) {
        (Some(d), Some(n)) => (d.to_path_buf(), n.to_string_lossy().into_owned()),
        _ => (dir.to_path_buf(), name.to_string()),
    };
    let dir = dir.as_path();
    (|| -> io::Result<()> {
        let prefix = format!(".{name}.");
        let mut builder = tempfile::Builder::new();
        builder.prefix(&prefix).suffix(".part");
        // A file made anew is made as any program makes one, through the
        // umask, not private as a temporary file is.
        #[cfg(unix)]
        if !file.exists() {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o666));
        }
        let mut part = builder.tempfile_in(dir)?;
        part.write_all(bytes)?;
        if let Ok(meta) = std::fs::metadata(&file) {
            part.as_file().set_permissions(meta.permissions())?;
        }
        part.as_file().sync_all()?;
        let path = part.into_temp_path();
        rename(&path, &file)?;
        // The rename itself is durable only once its folder is synced;
        // Windows cannot open a folder for that and journals renames.
        #[cfg(unix)]
        File::open(dir)?.sync_all()?;
        Ok(())
    })()
    .with_context(|| format!("writing {}", file.display()))
}

/// How long a file another program holds open is waited for.
const BUSY_WAIT: Duration = Duration::from_secs(5);

/// Whether `e` is Windows refusing a file another program has open.
fn busy(e: &io::Error) -> bool {
    // ERROR_ACCESS_DENIED and ERROR_SHARING_VIOLATION, the two a held
    // file is refused with.
    cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32))
}

fn retry(mut op: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut wait = Duration::from_millis(50);
    let mut waited = Duration::ZERO;
    loop {
        match op() {
            Err(e) if busy(&e) && waited < BUSY_WAIT => {
                std::thread::sleep(wait);
                waited += wait;
                wait = (wait * 2).min(Duration::from_secs(1));
            }
            other => return other,
        }
    }
}

/// `std::fs::rename`, replacing `to`, waiting out a held file.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    retry(|| std::fs::rename(from, to))
}

/// `std::fs::remove_file`, waiting out a held file.
pub fn remove(path: &Path) -> io::Result<()> {
    retry(|| std::fs::remove_file(path))
}

/// An exclusive lock on a folder, held until dropped.
#[derive(Debug)]
pub struct Lock(#[allow(dead_code)] File);

/// The folder library locks are kept in: the system's temporary folder,
/// but beside the library in tests, so each test's lock goes with its
/// folder rather than stay behind by the thousand.
fn library_locks(library: &Path) -> PathBuf {
    match library.parent() {
        Some(parent) if cfg!(test) => parent.to_path_buf(),
        _ => std::env::temp_dir(),
    }
}

impl Lock {
    /// Say so on stderr when another run holds it, then wait.
    pub fn folder(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        Self::at(&dir.join(LOCK_FILE), dir)
    }

    /// The lock a command that changes the home holds from first to last,
    /// so a run's record begins where the run before it ended. Apart from
    /// [`Lock::folder`]'s, which each step takes for its reads and writes.
    pub fn runs(home: &Path) -> Result<Self> {
        std::fs::create_dir_all(home).with_context(|| format!("creating {}", home.display()))?;
        Self::at(&home.join(RUNS_LOCK_FILE), home)
    }

    /// Whether a command that changes `home` holds it now, by
    /// [`Lock::runs`]; taking nothing.
    #[must_use]
    pub fn busy(home: &Path) -> bool {
        File::options()
            .write(true)
            .open(home.join(RUNS_LOCK_FILE))
            .is_ok_and(|f| matches!(f.try_lock(), Err(TryLockError::WouldBlock)))
    }

    /// A lock on the library folder, so two homes never write one
    /// library at once. It is kept in the system's temporary folder, named
    /// by the library's resolved path, to leave no file in the library.
    pub fn library(library: &Path) -> Result<Self> {
        // Made first, so its path resolves alike on the first run and later.
        if !library.exists() {
            std::fs::create_dir_all(library)
                .with_context(|| format!("creating {}", library.display()))?;
        }
        let resolved = dunce::canonicalize(library).unwrap_or_else(|_| library.to_path_buf());
        let name = crate::facts::digest(resolved.to_string_lossy().as_bytes());
        let path = library_locks(&resolved).join(format!(".muman-library-{name}.lock"));
        Self::at(&path, library)
    }

    /// Lock `path`, guarding `dir`.
    fn at(path: &Path, dir: &Path) -> Result<Self> {
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .with_context(|| format!("locking {}", dir.display()))?;
        match file.try_lock() {
            Ok(()) => return Ok(Self(file)),
            Err(TryLockError::WouldBlock) => {
                let _ = crate::ui::info(
                    &mut anstream::stderr(),
                    "Waiting for another muman run to finish",
                );
            }
            Err(TryLockError::Error(e)) => {
                return Err(e).with_context(|| format!("locking {}", dir.display()));
            }
        }
        file.lock()
            .map(|()| Self(file))
            .with_context(|| format!("locking {}", dir.display()))
    }
}

/// What a run's scratch folder is named after.
const SCRATCH: &str = ".muman-scratch-";

/// How old a scratch folder whose lock is free must be before it is
/// taken for a run's that ended without removing it: a folder just made
/// is locked a moment after.
const SCRATCH_SETTLED: Duration = Duration::from_secs(60);

/// A folder for one run's scratch files, removed when dropped. It holds
/// its own lock while in use, so one a run killed partway left behind,
/// whose lock is free, is removed by the next run to make one there.
#[derive(Debug)]
pub struct Scratch {
    // Released before the folder goes: Windows removes no folder holding
    // an open file.
    _lock: Lock,
    dir: tempfile::TempDir,
}

impl Scratch {
    /// A scratch folder in the system's temporary folder.
    pub fn new() -> Result<Self> {
        Self::within(&std::env::temp_dir())
    }

    /// A scratch folder within `parent`.
    pub fn within(parent: &Path) -> Result<Self> {
        clear_scratch(parent);
        let dir = tempfile::Builder::new()
            .prefix(SCRATCH)
            .tempdir_in(parent)
            .with_context(|| format!("creating a scratch folder in {}", parent.display()))?;
        let lock = Lock::folder(dir.path())?;
        Ok(Self { _lock: lock, dir })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// Remove the scratch folders in `parent` no run holds.
fn clear_scratch(parent: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ours = entry.file_name().to_string_lossy().starts_with(SCRATCH);
        let settled = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > SCRATCH_SETTLED));
        if !ours || !settled || !path.is_dir() {
            continue;
        }
        let free = File::options()
            .write(true)
            .open(path.join(LOCK_FILE))
            .map_or(true, |f| f.try_lock().is_ok());
        if free {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scratch_folder_left_behind_is_removed_and_one_in_use_kept() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join(format!("{SCRATCH}left"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("facts-1"), "measured").unwrap();
        let earlier = std::time::SystemTime::now() - Duration::from_secs(3600);
        filetime::set_file_mtime(&old, filetime::FileTime::from_system_time(earlier)).unwrap();
        let held = Scratch::within(dir.path()).unwrap();
        filetime::set_file_mtime(held.path(), filetime::FileTime::from_system_time(earlier))
            .unwrap();
        let other = Scratch::within(dir.path()).unwrap();
        assert!(!old.exists());
        assert!(held.path().exists() && other.path().exists());
        let gone = held.path().to_path_buf();
        drop(held);
        assert!(!gone.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_write_through_a_link_keeps_the_link_and_the_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("kept");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("songs.toml"), "one").unwrap();
        std::fs::set_permissions(
            real.join("songs.toml"),
            std::fs::Permissions::from_mode(0o664),
        )
        .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::os::unix::fs::symlink(real.join("songs.toml"), home.join("songs.toml")).unwrap();
        write(&home, "songs.toml", b"two").unwrap();
        let link = std::fs::symlink_metadata(home.join("songs.toml")).unwrap();
        assert!(link.file_type().is_symlink());
        assert_eq!(std::fs::read(real.join("songs.toml")).unwrap(), b"two");
        let mode = std::fs::metadata(real.join("songs.toml"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o664);
    }

    #[test]
    fn a_name_of_more_than_one_part_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["../state.json", "/tmp/x", "a/b", "..", "", "a\\b"] {
            assert!(write(dir.path(), name, b"x").is_err(), "{name}");
        }
        assert!(write(dir.path(), "state.json", b"x").is_ok());
    }

    #[test]
    fn a_write_replaces_the_file_and_leaves_no_part() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", b"one").unwrap();
        write(dir.path(), "a.txt", b"two").unwrap();
        assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["a.txt"]);
    }

    #[test]
    fn a_lock_creates_its_folder_and_excludes_a_second() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("x").join("y");
        let lock = Lock::folder(&inner).unwrap();
        assert!(inner.is_dir());
        let other = File::options()
            .write(true)
            .open(inner.join(LOCK_FILE))
            .unwrap();
        assert!(matches!(other.try_lock(), Err(TryLockError::WouldBlock)));
        drop(lock);
        assert!(other.try_lock().is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn a_held_file_is_waited_for_then_replaced() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("song.opus");
        std::fs::write(&target, b"old").unwrap();
        // Shared for reading only, as a player opens a file it plays.
        let held = File::options()
            .read(true)
            .share_mode(1)
            .open(&target)
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        });
        write(dir.path(), "song.opus", b"new").unwrap();
        release.join().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }
}
