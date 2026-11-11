//! The errors a command exits with by their kind, not their text: a
//! change refused for how it was asked, which exits 5, and a command line
//! or setting that does not read, which exits 2.
//!
//! A setting is [`Invalid`] wherever it is written: in a settings table
//! of the song list, as `[library]` or `[providers.*]`, or given to
//! `export --set`. A song list that is no TOML, or a `[[song]]`,
//! `[[album]]` or `[[removed]]` that does not read, is a failure, which
//! exits 4: the list itself, not a setting of it.
//!
//! A panic in code muman runs for one song, its own or a crate's it calls,
//! is that song's error ([`caught`]): the song fails as on any other
//! error, said and recorded for `check`, and every other song goes on.
//! A crate's bug is otherwise the whole run's, which exits 101 midway and
//! leaves what it was writing behind. The panic is said once, in the song's
//! failure: the hook that prints a panic stays quiet for one caught, so no
//! raw message breaks into the progress lines. The release profile keeps
//! unwinding (`panic = "abort"` would make every panic the run's again).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::panic::AssertUnwindSafe;

/// A change refused for how it was asked: nothing matched, or it needs
/// a terminal, `-y` or `--all`.
#[derive(Debug)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// A command line that parses but gives what does not read, or a setting
/// that does not read: it exits as a command line that does not parse.
#[derive(Debug)]
pub struct Invalid(pub String);

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Invalid {}

/// `e`, from reading a setting, as an [`Invalid`] whose text is its
/// whole chain.
#[must_use]
pub fn invalid(e: anyhow::Error) -> anyhow::Error {
    if e.downcast_ref::<Invalid>().is_some() {
        e
    } else {
        Invalid(format!("{e:#}")).into()
    }
}

/// A panic [`caught`] as an error: what it said, and where.
#[derive(Debug)]
pub struct Panicked(pub String);

impl fmt::Display for Panicked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Panicked {}

thread_local! {
    /// Whether this thread runs inside [`caught`].
    static CATCHING: Cell<bool> = const { Cell::new(false) };
    /// What the last panic caught on this thread said, kept by the hook.
    static SAID: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// `f`'s result, a panic in it made a [`Panicked`] error. The panic is
/// not printed: its message and place are the error's.
///
/// `f` is taken as safe to unwind: what it changes in the caller's
/// values must hold nothing half-done when it stops partway. A song's
/// build changes its own files alone, and `render` stages each and puts
/// it in place through the ledger whole, so a build stopped anywhere
/// leaves its song as an error part way would.
pub fn caught<T>(f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let shown = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if CATCHING.get() && !stops_a_test(info.payload()) {
                let at = info
                    .location()
                    .map_or_else(String::new, |l| format!(" (a panic at {l})"));
                SAID.set(Some(format!("{}{at}", message(info.payload()))));
            } else {
                shown(info);
            }
        }));
    });
    let was = CATCHING.replace(true);
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    CATCHING.set(was);
    result.unwrap_or_else(|panic| {
        if stops_a_test(&*panic) {
            std::panic::resume_unwind(panic);
        }
        let said = SAID.take().unwrap_or_else(|| message(&*panic));
        Err(Panicked(said).into())
    })
}

/// What a panic's payload says.
fn message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic that gave no reason".to_string())
}

/// Whether a panic is a test's own, stopping a run as a kill would or
/// failing a change no lock held: never caught.
fn stops_a_test(payload: &(dyn Any + Send)) -> bool {
    #[cfg(any(test, feature = "testing"))]
    {
        crate::kill::stops_a_test(payload)
    }
    #[cfg(not(any(test, feature = "testing")))]
    {
        let _ = payload;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_is_an_error_naming_what_it_said_and_where() {
        let e = caught::<()>(|| panic!("the width ran past every name")).unwrap_err();
        let said = format!("{e:#}");
        assert!(e.is::<Panicked>(), "{said}");
        assert!(
            said.starts_with("the width ran past every name (a panic at "),
            "{said}"
        );
        assert!(said.contains("errors.rs"), "{said}");
        assert_eq!(caught(|| Ok(3)).unwrap(), 3);
    }

    #[test]
    fn a_kill_goes_through() {
        let stopped = std::panic::catch_unwind(|| {
            caught::<()>(|| std::panic::panic_any(crate::kill::Killed))
        });
        assert!(stopped.unwrap_err().is::<crate::kill::Killed>());
    }
}
