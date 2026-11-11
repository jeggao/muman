//! The hooks a test stops or watches a run by, which the code that
//! changes files calls at each change: [`kill_at`] stops a run at a
//! chosen change, as a kill would, and [`guard_library`] fails a change
//! of a library's file while no lock holds it. They are compiled only for
//! tests, or with the `testing` feature, for the tests of a crate built
//! on this one; a build for use has neither, and the calls to them are
//! gone.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What a run killed by [`kill_at`] panics with.
#[derive(Debug)]
pub struct Killed;

/// One test's kill: the change of a file under `root` it stops at, and
/// how many it has passed.
#[derive(Debug)]
struct Armed {
    root: PathBuf,
    at: usize,
    seen: usize,
}

/// The kills armed, a test's each, by the folder its files are under.
static ARMED: Mutex<Vec<Armed>> = Mutex::new(Vec::new());

/// Where a kill was armed; dropped, it is disarmed, and says how many
/// changes of files under its folder it saw.
#[derive(Debug)]
pub struct Kill {
    root: PathBuf,
}

impl Kill {
    /// How many changes it saw so far.
    #[must_use]
    pub fn seen(&self) -> usize {
        ARMED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|a| a.root == self.root)
            .map_or(0, |a| a.seen)
    }
}

impl Drop for Kill {
    fn drop(&mut self) {
        ARMED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|a| a.root != self.root);
    }
}

/// A library a test guards, and how many [`crate::atomic::Lock::library`]
/// hold it now.
#[derive(Debug)]
struct Guarded {
    root: PathBuf,
    held: usize,
}

/// The libraries guarded, a test's each.
static GUARDED: Mutex<Vec<Guarded>> = Mutex::new(Vec::new());

thread_local! {
    /// Whether a guard failed a change on this thread.
    static TRIPPED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a panic with `payload` is one of a hook here, a kill or a
/// guard failing a change: a test's own, which
/// [`crate::errors::caught`] lets through.
pub fn stops_a_test(payload: &(dyn std::any::Any + Send)) -> bool {
    payload.is::<Killed>() || TRIPPED.get()
}

/// `path` resolved as [`change_point`] resolves the files it is given,
/// though it may not be made yet.
fn resolved(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| {
        path.parent()
            .and_then(|p| dunce::canonicalize(p).ok())
            .map_or_else(
                || path.to_path_buf(),
                |p| p.join(path.file_name().unwrap_or_default()),
            )
    })
}

/// Where a library is guarded; dropped, it is not.
#[derive(Debug)]
pub struct Guard {
    root: PathBuf,
}

impl Drop for Guard {
    fn drop(&mut self) {
        GUARDED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|g| g.root != self.root);
    }
}

/// Fail a change of a file in `library` made while no
/// [`crate::atomic::Lock::library`] holds it, at the change.
#[must_use]
pub fn guard_library(library: &Path) -> Guard {
    let root = resolved(library);
    GUARDED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(Guarded {
            root: root.clone(),
            held: 0,
        });
    Guard { root }
}

/// The library lock on `library` taken, or let go.
pub fn library_held(library: &Path, taken: bool) {
    let root = resolved(library);
    let mut guarded = GUARDED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(g) = guarded.iter_mut().find(|g| g.root == root) {
        g.held = if taken {
            g.held + 1
        } else {
            g.held.saturating_sub(1)
        };
    }
}

/// Stop the run at the `at`th change of a file under `root`, before it is
/// made, as a kill would: a panic of [`Killed`], which no message is
/// printed for. `usize::MAX` stops at none, to count them.
#[must_use]
pub fn kill_at(root: &Path, at: usize) -> Kill {
    static QUIET: std::sync::Once = std::sync::Once::new();
    QUIET.call_once(|| {
        let shown = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if info.payload().downcast_ref::<Killed>().is_none() {
                shown(info);
            }
        }));
    });
    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    ARMED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(Armed {
            root: root.clone(),
            at,
            seen: 0,
        });
    Kill { root }
}

/// A change of the file at `path` about to be made: the kill armed for
/// its folder, if any, stops it here.
pub fn change_point(path: &Path) {
    let path = path
        .parent()
        .and_then(|p| dunce::canonicalize(p).ok())
        .map_or_else(
            || path.to_path_buf(),
            |p| p.join(path.file_name().unwrap_or_default()),
        );
    let unlocked = GUARDED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|g| g.held == 0 && path.starts_with(&g.root));
    if unlocked {
        TRIPPED.set(true);
        panic!("{} changed while no lock held its library", path.display());
    }
    let killed = {
        let mut armed = ARMED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        armed
            .iter_mut()
            .find(|a| path.starts_with(&a.root))
            .is_some_and(|a| {
                a.seen += 1;
                a.seen == a.at
            })
    };
    if killed {
        std::panic::panic_any(Killed);
    }
}
