//! The errors a command exits with by their kind, not their text: a
//! change refused for how it was asked, which exits 5, and a command line
//! or setting that does not read, which exits 2.
//!
//! A setting is [`Invalid`] wherever it is written: in a settings table
//! of the song list, as `[library]` or `[providers.*]`, or given to
//! `export --set`. A song list that is no TOML, or a `[[song]]`,
//! `[[album]]` or `[[removed]]` that does not read, is a failure, which
//! exits 4: the list itself, not a setting of it.

use std::fmt;

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
