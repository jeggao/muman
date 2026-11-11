//! The network layer of muman, on [`muman_core`] and [`muman_media`]:
//! sources listed and fetched by yt-dlp and aria2c, and looked up on
//! LRCLIB, MusicBrainz, AcoustID and the Cover Art Archive.

// Tests make and change the files they check; shipped code changes one
// only where an `#[expect]` says why.
#![cfg_attr(test, expect(clippy::disallowed_methods))]
// Not in tests, as each test target sees every dev-dependency.
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

// The lower layers' modules, named as they were in one crate.
#[cfg(test)]
use muman_core::testing;
use muman_core::{
    atomic, fingerprint, given, http, progress, relpath, runner, sites, source, store, torrent, ui,
    units,
};
use muman_media::{clean, ffmpeg, info, picture, probe, record, similar};

pub mod acoustid;
pub mod aria2;
pub mod coverart;
pub mod download;
pub mod lrclib;
pub mod music;
pub mod musicbrainz;
pub mod plugins;
pub mod ytdlp_log;
