//! The errors a command exits with by their kind, not their text: a
//! change refused for how it was asked, which exits 5.

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
