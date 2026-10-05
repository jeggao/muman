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
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

/// The lock file in a locked folder; a regular file, since Windows
/// cannot open a folder to lock it.
pub const LOCK_FILE: &str = ".lock";

/// Write `bytes` to a temporary file in `dir`, synced, and rename it
/// over `<dir>/<name>`, so a reader sees the old file or the new.
pub fn write(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let file = dir.join(name);
    (|| -> io::Result<()> {
        let mut part = tempfile::Builder::new()
            .prefix(&format!(".{name}."))
            .suffix(".part")
            .tempfile_in(dir)?;
        part.write_all(bytes)?;
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

impl Lock {
    /// Say so on stderr when another run holds it, then wait.
    pub fn folder(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(LOCK_FILE);
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
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

#[cfg(test)]
mod tests {
    use super::*;

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
