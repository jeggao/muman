//! The media layer of muman, on [`muman_core`]: each source probed,
//! decoded and measured, its tags, chapters, lyrics and pictures read
//! and cleaned, songs' names made safe, and what a song's files are
//! made with chosen and written.

// Tests make and change the files they check; shipped code changes one
// only where an `#[expect]` says why.
#![cfg_attr(test, expect(clippy::disallowed_methods))]
// Not in tests, as each test target sees every dev-dependency.
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

// The lowest layer's modules, named as they were in one crate.
#[cfg(test)]
use muman_core::testing;
use muman_core::{
    atomic, fingerprint, hash, lang, portable, relpath, runner, sites, source, store, template,
    units,
};

pub mod align;
pub mod analysis;
pub mod chapters;
pub mod clean;
pub mod codec;
pub mod cue;
pub mod cues;
pub mod detect;
pub mod dynamics;
pub mod facts;
pub mod ffmpeg;
pub mod flac;
pub mod genres;
pub mod info;
pub mod integrity;
pub mod loudness;
pub mod lyrics;
pub mod naming;
pub mod ogg;
pub mod overrides;
pub mod parts;
pub mod picture;
pub mod probe;
pub mod quality;
pub mod readings;
pub mod record;
pub mod similar;
pub mod style;
pub mod tagfile;
pub mod tags;
pub mod treble;
