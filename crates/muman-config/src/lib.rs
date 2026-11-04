//! The config layer of muman, on the crates below it: the song list,
//! `songs.toml`, with its songs, settings and providers' tables, the
//! editions an older list is brought up from, its hooks, what each
//! source held when its song was built, and the queries that pick songs.

// Tests make and change the files they check; shipped code changes one
// only where an `#[expect]` says why.
#![cfg_attr(test, expect(clippy::disallowed_methods))]

// The lower layers' modules, named as they were in one crate.
#[cfg(test)]
use muman_core::testing;
use muman_core::{
    atomic, defaults, dirs, errors, hash, http, lang, provider, relpath, runner, sites, source,
    store, template, tools, torrent, ui, units,
};
use muman_media::{clean, codec, cues, facts, info, loudness, naming, quality, tags};
use muman_net::{acoustid, aria2, download};

pub mod held;
pub mod hooks;
pub mod manifest;
pub mod migrate;
pub mod providers;
pub mod settings;
pub mod terms;
