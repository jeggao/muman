//! What muman writes and reads when the song list says nothing: a new
//! home's song list, every setting at its default, which the built-in
//! site table is read from as well.

/// A new home's song list: the settings, each written out at its
/// default, as the reference the docs point to.
pub const NEW: &str = include_str!("manifest/new.toml");
