//! Whole-file writes a crash cannot leave half done, and the folder lock
//! two runs take before either rewrites what the other reads.

use std::fs::{File, TryLockError};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

/// Write `bytes` to `<dir>/.<name>.part`, synced, and rename it over
/// `<dir>/<name>`, so a reader sees the old file or the new.
pub fn write(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let file = dir.join(name);
    let part = dir.join(format!(".{name}.part"));
    (|| -> std::io::Result<()> {
        let mut f = File::create(&part)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&part, &file)?;
        File::open(dir)?.sync_all()
    })()
    .with_context(|| format!("writing {}", file.display()))
}

/// An exclusive `flock` on a folder, held until dropped.
#[derive(Debug)]
pub struct Lock(#[allow(dead_code)] File);

impl Lock {
    /// Say so on stderr when another run holds it, then wait.
    pub fn folder(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let file = File::open(dir).with_context(|| format!("locking {}", dir.display()))?;
        match file.try_lock() {
            Ok(()) => return Ok(Self(file)),
            Err(TryLockError::WouldBlock) => {
                let _ = crate::ui::info(
                    &mut std::io::stderr(),
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
        assert!(!dir.path().join(".a.txt.part").exists());
    }

    #[test]
    fn a_lock_creates_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("x/y");
        let _lock = Lock::folder(&inner).unwrap();
        assert!(inner.is_dir());
    }
}
