//! The lowest layer of muman, which every other crate of it builds on:
//! units and paths, names every system takes, files written and removed
//! safely, the programs muman runs, the store of sources and the keys
//! that name them, and the built-in tables a song list starts from.

// Tests make and change the files they check; shipped code changes one
// only where an `#[expect]` says why.
#![cfg_attr(test, expect(clippy::disallowed_methods))]
// Not in tests, as each test target sees every dev-dependency.
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod atomic;
pub mod defaults;
pub mod dirs;
pub mod errors;
pub mod fingerprint;
pub mod given;
pub mod hash;
pub mod http;
#[cfg(any(test, feature = "testing"))]
pub mod kill;
pub mod lang;
pub mod parallel;
pub mod platform;
pub mod portable;
pub mod progress;
pub mod provider;
pub mod relpath;
pub mod runner;
pub mod sites;
pub mod source;
pub mod store;
pub mod template;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod tools;
pub mod torrent;
pub mod ui;
pub mod units;
