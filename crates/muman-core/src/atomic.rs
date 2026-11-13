//! Whole-file writes a crash cannot leave half done, the folder lock two
//! runs take before either rewrites what the other reads, and renames
//! and deletes that wait out a file another program holds open.
//!
//! Each change of a file passes a point a test can stop the run at, as
//! a crash would (`kill::kill_at`), so the tests kill every command at
//! every step and check what recovery makes of it.
//!
//! A write names its file by a [`Name`], one plain part of a path checked
//! when the name is made, so a name built from what a server answered
//! cannot write outside its folder.
//!
//! What holds the song list's text, which may hold logins and tokens, is
//! its owner's alone on Unix: a new home and the run history's folders
//! are made `0700` ([`private_dir`]), as is a run's scratch folder, and a
//! run's record and a config file handed to a program `0600`
//! ([`write_private`], [`private_file`]): `ps` shows any user a program's
//! command line, but no file in a folder of another's. A folder that
//! exists is left as it is. Windows has no modes: a folder made takes its
//! parent's permissions, which for the default home and the temporary
//! folder are the user's own profile's.
//!
//! A write stopped before its rename, by a kill or a Ctrl-C, leaves
//! its part, `.<name>.<six characters>.part`, beside the file.
//! [`Lock::folder`] sweeps the settled ones from its folder once it holds
//! it, and a run sweeps its history's run folders as it begins
//! ([`clear_parts`]). A part beside a file a link leads to outside the
//! home stays. A run's scratch folder, `.muman-scratch-<random>`, holds
//! its own lock; one a stopped run left goes as the next run makes one
//! in the same folder, or an export names that folder ([`Scratch`]).
//!
//! On Windows a file open in a player, the search indexer or a virus
//! scanner cannot be replaced or deleted for a moment: the call fails
//! with a sharing violation. [`rename`] and [`remove`] retry those for a
//! few seconds before giving up; elsewhere they fail at once.
//!
//! A sync and an undo hold the library's lock while they change the
//! library, after the home's; nothing else changes it, and a dry run
//! such as `status` changes nothing in it. The lock is kept out of the
//! library, in a folder of the user's own under their cache folder,
//! which the caller names ([`crate::dirs::Dirs::locks`]) and this makes
//! private on Unix. Not in the
//! system's temporary folder, where its
//! name, the digest of the library's path, is known to every user:
//! another could make it first as a link, so that opening it made or
//! emptied a file of their choosing, or hold it locked, so that every run
//! waited forever. In a folder of the user's own neither can happen, at
//! the cost that two users who share one library folder do not keep each
//! other out of it; each user's runs keep out the others of their own,
//! whichever home they run from. Not the runtime folder either, which some
//! sessions, as `cron`'s, do not name, so two runs of one user could each
//! take a lock of their own.
//!
//! The lock goes as its holder lets go, so the folder holds a file only
//! while a run does: the file is removed while still locked, and a run
//! that locks it checks the path still names the file it locked, else
//! opens and locks it again. A run that was waiting on the removed file
//! finds its path gone or another file there, never two runs holding the
//! lock at once. Windows deletes a file still open only once the last
//! handle closes, refusing to open it meanwhile, which a run that locks
//! it waits out as [`rename`] waits out a held file.
//!
//! On a folder muman may not write, as a home on a read-only mount, the
//! lock file an earlier run left is opened read only and locked so,
//! which every system allows, so a command that only reads, as `status`,
//! still runs there; one that writes fails at its first write.

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

/// A file name of one plain part, checked when it is made, so a write
/// through it stays in its folder whatever a server or a tag named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name(String);

impl Name {
    /// `name` as one part of a path, refused when it is empty, `.`,
    /// `..`, or holds a separator of any platform.
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let plain = Path::new(&name)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
            && !name.contains(['/', '\\'])
            && !name.is_empty();
        if !plain {
            anyhow::bail!("{name:?} is no file name");
        }
        Ok(Self(name))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Write `bytes` to a temporary file in `dir`, synced, and rename it
/// over `<dir>/<name>`, so a reader sees the old file or the new. A file
/// that is a link is written where it leads, so the link stays, and a
/// file replaced keeps its permissions.
pub fn write(dir: &Path, name: &Name, bytes: &[u8]) -> Result<()> {
    write_as(dir, name, bytes, false)
}

/// [`write()`], the file its owner's alone on Unix, whatever its mode was.
pub fn write_private(dir: &Path, name: &Name, bytes: &[u8]) -> Result<()> {
    write_as(dir, name, bytes, true)
}

// The primitive every other write goes through.
#[expect(clippy::disallowed_methods)]
fn write_as(dir: &Path, name: &Name, bytes: &[u8], private: bool) -> Result<()> {
    let name = name.as_str();
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
        if !file.exists() && !private {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o666));
        }
        let mut part = builder.tempfile_in(dir)?;
        part.write_all(bytes)?;
        if !private && let Ok(meta) = std::fs::metadata(&file) {
            part.as_file().set_permissions(meta.permissions())?;
        }
        part.as_file().sync_all()?;
        // Kept, so a test's kill leaves it as a real kill does, for
        // clear_parts.
        let path = part.into_temp_path().keep().map_err(|e| e.error)?;
        #[cfg(any(test, feature = "testing"))]
        crate::kill::change_point(&file);
        if let Err(e) = retry(|| std::fs::rename(&path, &file)) {
            drop(tempfile::TempPath::try_from_path(path));
            return Err(e);
        }
        // The rename itself is durable only once its folder is synced;
        // Windows cannot open a folder for that and journals renames.
        #[cfg(unix)]
        File::open(dir)?.sync_all()?;
        Ok(())
    })()
    .with_context(|| format!("writing {}", file.display()))
}

/// Make the folder `dir`, its owner's alone on Unix, and any missing
/// above it as any program makes one; a folder that exists is left as
/// it is.
pub fn private_dir(dir: &Path) -> Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    #[cfg(unix)]
    let made =
        std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700).create(dir);
    #[cfg(not(unix))]
    let made = std::fs::DirBuilder::new().create(dir);
    match made {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => Ok(()),
        other => other.with_context(|| format!("creating {}", dir.display())),
    }
}

/// Write `bytes` to the new file `path`, its owner's alone on Unix from
/// the moment it is made: a secret handed to a program in a file rather
/// than on its command line.
pub fn private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = File::options();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .with_context(|| format!("writing {}", path.display()))
}

/// How long a file another program holds open is waited for.
const BUSY_WAIT: Duration = Duration::from_secs(5);

/// Whether `e` is Windows refusing a file another program has open.
fn busy(e: &io::Error) -> bool {
    // ERROR_ACCESS_DENIED and ERROR_SHARING_VIOLATION, the two a held
    // file is refused with.
    cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32))
}

/// Whether `e` says a folder may not be written: a read-only filesystem,
/// or on Unix a folder of no write permission. Windows says a file being
/// deleted is denied too, which [`retry`] waits out instead.
fn unwritable(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::ReadOnlyFilesystem
        || (cfg!(unix) && e.kind() == io::ErrorKind::PermissionDenied)
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
// The primitive every other rename goes through.
#[expect(clippy::disallowed_methods)]
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(any(test, feature = "testing"))]
    crate::kill::change_point(to);
    retry(|| std::fs::rename(from, to))
}

/// `std::fs::hard_link`, a second name `to` for the file at `from`.
// The primitive every other second name goes through.
#[expect(clippy::disallowed_methods)]
pub fn link(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(any(test, feature = "testing"))]
    crate::kill::change_point(to);
    std::fs::hard_link(from, to)
}

/// `from` moved to `to` only where nothing is at `to`: a second name made
/// for it first, which no filesystem gives over a file already there,
/// then its first name removed. Where the filesystem makes no second
/// name, as FAT, `to` is looked for before the rename, so only a file
/// made between the two would be written over.
// The primitive a file placed where no other may be goes through.
#[expect(clippy::disallowed_methods)]
pub fn place_new(from: &Path, to: &Path) -> io::Result<()> {
    match link(from, to) {
        Ok(()) => remove(from),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(_) if to.symlink_metadata().is_ok() => Err(io::ErrorKind::AlreadyExists.into()),
        Err(_) => rename(from, to),
    }
}

/// `std::fs::remove_file`, waiting out a held file.
// The primitive every other removal goes through.
#[expect(clippy::disallowed_methods)]
pub fn remove(path: &Path) -> io::Result<()> {
    #[cfg(any(test, feature = "testing"))]
    crate::kill::change_point(path);
    retry(|| std::fs::remove_file(path))
}

/// An exclusive lock on a folder, held until dropped.
#[derive(Debug)]
pub struct Lock {
    file: File,
    /// The lock file, removed as the lock is let go; a lock in a folder
    /// of its own keeps its file.
    removed: Option<PathBuf>,
    /// The library a test guards that this locks.
    #[cfg(any(test, feature = "testing"))]
    library: Option<PathBuf>,
}

impl Drop for Lock {
    // Lets go of the library's lock file.
    #[expect(clippy::disallowed_methods)]
    fn drop(&mut self) {
        #[cfg(any(test, feature = "testing"))]
        if let Some(library) = &self.library {
            crate::kill::library_held(library, false);
        }
        // Before the file closes, which lets the lock go.
        if let Some(path) = &self.removed {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Make `dir`, and on Unix make it the user's alone, as it may have been
/// made by an older muman or another program.
fn private(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("making {} private", dir.display()))?;
    }
    Ok(())
}

impl Lock {
    /// Say so on stderr when another run holds it, then wait.
    pub fn folder(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let lock = Self::at(&dir.join(LOCK_FILE), dir)?;
        clear_parts(dir);
        Ok(lock)
    }

    /// The lock a command that changes the home holds from first to last,
    /// so a run's record begins where the run before it ended. Apart from
    /// [`Lock::folder`]'s, which each step takes for its reads and writes.
    pub fn runs(home: &Path) -> Result<Self> {
        private_dir(home)?;
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
    /// library at once. It is kept in `locks`, a folder of the user's own
    /// the caller names and this makes private, named by the library's
    /// resolved path, to leave no file in the library, and removed as it
    /// is let go.
    pub fn library(library: &Path, locks: &Path) -> Result<Self> {
        // Made first, so its path resolves alike on the first run and later.
        if !library.exists() {
            std::fs::create_dir_all(library)
                .with_context(|| format!("creating {}", library.display()))?;
        }
        let resolved = dunce::canonicalize(library).unwrap_or_else(|_| library.to_path_buf());
        let name = crate::hash::digest(resolved.to_string_lossy().as_bytes());
        private(locks)?;
        let path = locks.join(format!(".muman-library-{name}.lock"));
        loop {
            let mut lock = Self::at(&path, library)?;
            let held = lock.file.try_clone().and_then(same_file::Handle::from_file);
            let named = same_file::Handle::from_path(&path);
            if let (Ok(held), Ok(named)) = (held, named)
                && held == named
            {
                lock.removed = Some(path);
                #[cfg(any(test, feature = "testing"))]
                {
                    crate::kill::library_held(&resolved, true);
                    lock.library = Some(resolved);
                }
                return Ok(lock);
            }
        }
    }

    /// Lock `path`, guarding `dir`; read only where `dir` may not be
    /// written and an earlier run left the file.
    fn at(path: &Path, dir: &Path) -> Result<Self> {
        let mut opened = None;
        retry(|| {
            let writing = File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path);
            opened = Some(match writing {
                Err(e) if unwritable(&e) && path.is_file() => File::open(path)?,
                writing => writing?,
            });
            Ok(())
        })
        .with_context(|| format!("locking {}", dir.display()))?;
        let file = opened.context("opening the lock")?;
        let locked = |file| Self {
            file,
            removed: None,
            #[cfg(any(test, feature = "testing"))]
            library: None,
        };
        match file.try_lock() {
            Ok(()) => return Ok(locked(file)),
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
            .map(|()| locked(file))
            .with_context(|| format!("locking {}", dir.display()))
    }
}

/// What a run's scratch folder is named after.
const SCRATCH: &str = ".muman-scratch-";

/// How old an empty scratch folder, or a part, must be before it is
/// taken for one a stopped run left: a scratch folder just made is
/// locked a moment after, and a part just begun is written to.
const SCRATCH_SETTLED: Duration = Duration::from_secs(60);

/// How many scratch folders a run makes in turn while another run's
/// clearing removes each before its lock is held.
const SCRATCH_TRIES: usize = 8;

/// A folder for one run's scratch files, removed when dropped. It holds
/// its own lock while in use, so one a run killed partway left behind,
/// whose lock is free, is removed by the next run to make one in the
/// same folder ([`clear_scratch`]).
///
/// A folder is made, then locked, so another run clearing scratch may
/// find it in between. That run removes it holding its lock, so this
/// one, its lock taken, checks the lock file is still the one at its
/// path, as [`Lock::library`] does, and makes another if not.
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
    // Makes a run's scratch folder.
    #[expect(clippy::disallowed_methods)]
    pub fn within(parent: &Path) -> Result<Self> {
        clear_scratch(parent);
        let mut builder = tempfile::Builder::new();
        builder.prefix(SCRATCH);
        #[cfg(unix)]
        builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
        for _ in 0..SCRATCH_TRIES {
            let dir = builder
                .tempdir_in(parent)
                .with_context(|| format!("creating a scratch folder in {}", parent.display()))?;
            let path = dir.path().join(LOCK_FILE);
            let lock = match Lock::at(&path, dir.path()) {
                Ok(lock) => lock,
                Err(_) if !dir.path().is_dir() => continue,
                Err(e) => return Err(e),
            };
            let held = lock.file.try_clone().and_then(same_file::Handle::from_file);
            let named = same_file::Handle::from_path(&path);
            if let (Ok(held), Ok(named)) = (held, named)
                && held == named
            {
                return Ok(Self { _lock: lock, dir });
            }
        }
        anyhow::bail!(
            "another run kept removing the scratch folders made in {}",
            parent.display()
        )
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// Remove the files [`write()`] left in `dir` when stopped before it
/// renamed them into place, as by a kill or a Ctrl-C: `.<name>.<six
/// characters>.part`, settled as a scratch folder is, so one being
/// written now stays.
// Removes the parts a stopped write left.
#[expect(clippy::disallowed_methods)]
pub fn clear_parts(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let part = name
            .to_str()
            .and_then(|n| n.strip_prefix('.')?.strip_suffix(".part"))
            .and_then(|n| n.rsplit_once('.'))
            .is_some_and(|(of, random)| !of.is_empty() && random.len() == 6);
        let settled = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > SCRATCH_SETTLED));
        if part && settled && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Remove the scratch folders in `parent` no run holds: one whose lock
/// file is free at once, its lock held while it goes, and one empty, as
/// a run stopped between making and locking it leaves, once settled. A
/// folder so named holding files but no lock file is no run's, and
/// stays.
// Removes the scratch folders a stopped run left.
#[expect(clippy::disallowed_methods)]
pub fn clear_scratch(parent: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ours = entry.file_name().to_string_lossy().starts_with(SCRATCH);
        if !ours || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(lock) = File::options().write(true).open(path.join(LOCK_FILE)) else {
            let settled = entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > SCRATCH_SETTLED));
            if settled {
                let _ = std::fs::remove_dir(&path);
            }
            continue;
        };
        if lock.try_lock().is_ok() {
            let _ = std::fs::remove_dir_all(&path);
            drop(lock);
            // Where the lock file outlived its handle, as on older Windows.
            let _ = std::fs::remove_dir(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_file_placed_new_takes_no_file_s_place() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("a.part"), dir.path().join("a"));
        std::fs::write(&from, "new").unwrap();
        std::fs::write(&to, "theirs").unwrap();
        let e = super::place_new(&from, &to).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "theirs");
        std::fs::remove_file(&to).unwrap();
        super::place_new(&from, &to).unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "new");
        assert!(!from.exists());
    }

    use super::*;

    fn name(s: &str) -> Name {
        Name::new(s).unwrap()
    }

    #[test]
    fn a_read_only_filesystem_is_unwritable_and_a_held_file_is_not() {
        let read_only = io::Error::from(io::ErrorKind::ReadOnlyFilesystem);
        assert!(unwritable(&read_only));
        assert!(!unwritable(&io::Error::from(io::ErrorKind::NotFound)));
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(unwritable(&denied), cfg!(unix));
    }

    #[test]
    fn a_scratch_folder_left_behind_is_removed_and_one_in_use_kept() {
        let dir = tempfile::tempdir().unwrap();
        let earlier = std::time::SystemTime::now() - Duration::from_secs(3600);
        let folder = |name: &str, files: &[&str], old: bool| {
            let path = dir.path().join(format!("{SCRATCH}{name}"));
            std::fs::create_dir_all(&path).unwrap();
            for file in files {
                std::fs::write(path.join(file), "measured").unwrap();
            }
            if old {
                filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(earlier))
                    .unwrap();
            }
            path
        };
        let left = folder("left", &[LOCK_FILE, "facts-1"], false);
        let unlocked = folder("made", &[], true);
        let fresh = folder("new", &[], false);
        let mine = folder("notes", &["facts-1"], true);
        let held = Scratch::within(dir.path()).unwrap();
        filetime::set_file_mtime(held.path(), filetime::FileTime::from_system_time(earlier))
            .unwrap();
        let other = Scratch::within(dir.path()).unwrap();
        assert!(!left.exists(), "its lock free, at once");
        assert!(!unlocked.exists(), "empty and settled");
        assert!(fresh.exists(), "empty, but maybe about to be locked");
        assert!(mine.exists(), "files but no lock: no run's");
        assert!(held.path().exists() && other.path().exists());
        let gone = held.path().to_path_buf();
        drop(held);
        assert!(!gone.exists());
    }

    #[test]
    fn a_part_left_behind_is_removed_and_a_fresh_one_kept() {
        let dir = tempfile::tempdir().unwrap();
        let earlier = std::time::SystemTime::now() - Duration::from_secs(3600);
        let make = |name: &str, old: bool| {
            let path = dir.path().join(name);
            std::fs::write(&path, "half").unwrap();
            if old {
                filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(earlier))
                    .unwrap();
            }
            path
        };
        let left = make(".state.json.a1B2c3.part", true);
        let fresh = make(".songs.toml.d4E5f6.part", false);
        let other = make(".notes.part", true);
        let mine = make("state.json", true);
        drop(Lock::folder(dir.path()).unwrap());
        assert!(!left.exists());
        assert!(fresh.exists() && other.exists() && mine.exists());
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
        write(&home, &name("songs.toml"), b"two").unwrap();
        let link = std::fs::symlink_metadata(home.join("songs.toml")).unwrap();
        assert!(link.file_type().is_symlink());
        assert_eq!(std::fs::read(real.join("songs.toml")).unwrap(), b"two");
        let mode = std::fs::metadata(real.join("songs.toml"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o664);
    }

    #[cfg(unix)]
    #[test]
    fn a_private_write_folder_and_scratch_are_their_owner_s_alone() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
        private_dir(&shared).unwrap();
        assert_eq!(mode(&shared), 0o755, "a folder that exists is left");
        let home = shared.join("a").join("home");
        private_dir(&home).unwrap();
        assert_eq!(mode(&home), 0o700);
        std::fs::write(home.join("run.json"), "{}").unwrap();
        std::fs::set_permissions(
            home.join("run.json"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        write_private(&home, &name("run.json"), b"{\"a\": 1}").unwrap();
        assert_eq!(
            mode(&home.join("run.json")),
            0o600,
            "tightened when written"
        );
        write_private(&home, &name("new.json"), b"{}").unwrap();
        assert_eq!(mode(&home.join("new.json")), 0o600);
        private_file(&home.join("args.conf"), b"'--secret'").unwrap();
        assert_eq!(mode(&home.join("args.conf")), 0o600);
        assert!(private_file(&home.join("args.conf"), b"again").is_err());
        let scratch = Scratch::within(dir.path()).unwrap();
        assert_eq!(mode(scratch.path()), 0o700);
    }

    #[test]
    fn a_name_of_more_than_one_part_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["../state.json", "/tmp/x", "a/b", "..", "", "a\\b"] {
            assert!(Name::new(name).is_err(), "{name}");
        }
        assert!(write(dir.path(), &name("state.json"), b"x").is_ok());
    }

    #[test]
    fn a_write_replaces_the_file_and_leaves_no_part() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &name("a.txt"), b"one").unwrap();
        write(dir.path(), &name("a.txt"), b"two").unwrap();
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

    #[test]
    fn a_library_lock_leaves_no_file_behind_and_still_excludes() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("lib");
        let locks = || {
            std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|n| n.starts_with(".muman-library-"))
                .count()
        };
        let lock = Lock::library(&library, dir.path()).unwrap();
        assert_eq!(locks(), 1);
        drop(lock);
        assert_eq!(locks(), 0, "let go of, its file goes");
        let inside = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..10 {
                        let _lock = Lock::library(&library, dir.path()).unwrap();
                        assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0, "two hold it");
                        std::thread::sleep(Duration::from_millis(1));
                        inside.fetch_sub(1, Ordering::SeqCst);
                    }
                });
            }
        });
        assert_eq!(locks(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_lock_folder_is_the_user_s_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locks = dir.path().join("cache").join("locks");
        private(&locks).unwrap();
        std::fs::set_permissions(&locks, std::fs::Permissions::from_mode(0o777)).unwrap();
        private(&locks).unwrap();
        let mode = std::fs::metadata(&locks).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
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
        write(dir.path(), &name("song.opus"), b"new").unwrap();
        release.join().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }
}
