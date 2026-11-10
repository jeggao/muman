//! The engine of muman, on the crates below it: the state a run keeps,
//! each song resolved to the plan its files are made from, the library
//! written to match and its ledger, the lookups and fetches a sync
//! makes, and the history a run records for undo.

// Tests make and change the files they check; shipped code changes one
// only where an `#[expect]` says why.
#![cfg_attr(test, expect(clippy::disallowed_methods))]

// The lower layers' modules, named as they were in one crate.
use muman_config::{held, hooks, manifest, migrate, providers, settings, terms};
#[cfg(any(test, feature = "testing"))]
use muman_core::kill;
#[cfg(test)]
use muman_core::testing;
use muman_core::{
    atomic, dirs, errors, fingerprint, hash, http, lang, parallel, platform, progress, provider,
    relpath, runner, sites, source, store, tools, ui, units,
};
use muman_media::{
    align, analysis, chapters, clean, codec, cues, detect, dynamics, facts, ffmpeg, flac,
    integrity, loudness, lyrics, naming, ogg, parts, picture, quality, readings, record, similar,
    style, tags,
};
use muman_net::{acoustid, coverart, download, lrclib, music, musicbrainz, ytdlp_log};

pub mod acquire;
pub mod album;
pub mod artifact;
pub mod calibrate;
pub mod diagnose;
pub mod due;
pub mod fit;
pub mod folder;
pub mod history;
pub mod identify;
pub mod levels;
pub mod library;
pub mod limit;
pub mod lookup;
pub mod names;
pub mod outputs;
pub mod query;
pub mod reconcile;
pub mod release;
pub mod render;
pub mod resolve;
pub mod rows;
pub mod state;
pub mod vet;
