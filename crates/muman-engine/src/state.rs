//! What muman keeps for itself in `state.db`: the library files
//! it wrote and the plan each was written from, which is what lets it
//! delete only its own files and rebuild only what changed, and caches
//! of what it measured, which are safe to lose. A `state.json` an
//! earlier muman kept is read once and brought into the store,
//! then kept as `state.json.bak`; a stub of a newer format takes its
//! place, so an earlier muman refuses the home rather than take every
//! library file for the user's. Moving the `.bak` back, the store aside,
//! goes back to the earlier muman, losing what ran since.
//!
//! | Table | Safe to lose |
//! |---|---|
//! | `outputs` | No: without it nothing is deleted, and no file is written over until `sync --force` |
//! | `replaced` | Mostly: a playlist added again fetches those uploads |
//! | `library`, `alignments`, `facts`, `mixes` | Yes: measured or set again |
//! | `sizes` | Yes: songs are rendered again to measure them |
//! | `failures`, `lookups` | Yes: each is tried or made again at once |
//! | `damaged` | Yes: a song built from a damaged download then waits for `sync --accept` to take the whole copy fetched in its place |
//! | `trusted` | Yes: the song list's commands and servers are asked about again (`trust`) |
//!
//! Each file beside an output, as its lyrics, is recorded as the output
//! is, by its size, time and digest ([`Sidecar`]), so one changed since is
//! the user's alike; version 2 named it by its place alone, which keeps
//! one it recorded so muman's.
//!
//! An output with no plan is one muman owns but cannot vouch for, as each
//! a run records just before writing it, new or written over: a crash
//! between the two leaves a file the next run writes again or deletes,
//! rather than one no run would ever delete or one taken for changed by
//! someone else. A run keeps each file it wrote as it goes, so a crash
//! loses at most the last few. An output whose size and time differ from those
//! recorded was changed by something else, a tagger or a player: it is
//! not written over, and once no song makes it, it is left in place and
//! dropped from `outputs`, no longer muman's. A file at a song's path
//! that no output names, the user's own, is not written over either
//! unless forced, and then is kept for `undo` first. When the library
//! folder moves, each output whose file is at its place in the new one
//! as written, by its size and time or its bytes, stays muman's, so a
//! library moved or copied is not taken for the user's; the files in the
//! old one are left alone.
//!
//! A source that could not be read is not read again until its revision,
//! each of its files' size and modification time, changes; one that could
//! not be fetched again waits an hour, doubling with each failure up to a
//! week. A failure ends when the step succeeds: a step that runs without
//! the run's lock, as fetching and lookups do, keeps only what it
//! measured since it loaded the state ([`State::since`]) through
//! [`State::keep_measures`], which merges that into the store's caches
//! as they are now by one rule, in one transaction that writes them
//! alone. So a failure one step cleared is cleared on disk too, a fact,
//! failure or comparison another process recorded meanwhile stays rather
//! than being written over by the copy the step loaded, and what the
//! library holds is written only by the run holding its lock: a fact
//! naming a source's parts anew is kept as a renaming, which every load
//! applies to the kept plans until a whole save writes them renamed.
//!
//! A cache entry an earlier version wrote in another shape is dropped and
//! made again; only `outputs` failing to parse refuses the file, since
//! losing it would lose which library files are muman's.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::align;
use crate::analysis::Analysis;
use crate::atomic;
use crate::codec::Layout;
use crate::facts::Facts;
use crate::provider::Provider;
use crate::resolve::Plan;
use crate::source::SourceKey;

mod db;
mod json;

/// How long a write waits for another process's transaction.
const BUSY_WAIT: Duration = Duration::from_secs(60);

/// What `state.json` holds once the state is in `state.db`: a format
/// newer than any muman before the store reads, so one refuses the home
/// rather than read it as holding no state.
const STUB: &[u8] = b"{\"version\": 4, \"moved_to\": \"state.db\"}\n";

/// Where `state.json` is kept once brought over.
const BACKUP: &str = "state.json.bak";

/// The store being made from `state.json`, before it is `state.db`.
const IMPORTING: &str = "state.db.import";

/// Whether `home` holds a state store.
#[must_use]
pub fn exists(home: &Path) -> bool {
    home.join(db::FILE).exists()
}

/// Refuse a home whose song list is gone while the library holds songs
/// muman wrote: read as listing nothing, it would remove every one, and a
/// list saved from it would list only what a command added. A song list
/// that lists no song removes them.
pub fn present(home: &Path) -> Result<()> {
    let file = home.join(crate::dirs::MANIFEST);
    if file.exists() {
        return Ok(());
    }
    let written = State::load(home).map_or(0, |s| s.outputs.len());
    if written == 0 {
        return Ok(());
    }
    Err(crate::errors::Refused(format!(
        "{} is missing while the library holds {written} song(s) muman wrote: put it back, \
         or `muman undo` the run that removed it. A song list that lists no song removes them",
        file.display()
    ))
    .into())
}

/// When the store was last written, as its files' times say.
#[must_use]
pub fn written_at(home: &Path) -> Option<SystemTime> {
    [db::FILE.to_string(), format!("{}-wal", db::FILE)]
        .iter()
        .filter_map(|name| {
            std::fs::metadata(home.join(name))
                .and_then(|m| m.modified())
                .ok()
        })
        .max()
}

/// Bring `home`'s `state.json`, as any earlier muman wrote it, into
/// `state.db`, once: the store is made beside it under another name and
/// linked into place, so of two processes bringing it over the first
/// alone makes it, and then the file is kept as `state.json.bak`, a stub
/// in its place. A run stopped between the two finds the store and the
/// file, and sets the file aside. Returns the state when it brought it
/// over now, holding what an earlier version named a key, which only
/// this process's run reads.
fn bring_over(home: &Path) -> Result<Option<State>> {
    let file = home.join(crate::dirs::STATE);
    let text = match std::fs::read(&file) {
        Ok(text) if text != STUB => text,
        Ok(_) => return Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Set aside by a run stopped before its stub.
            if exists(home) && home.join(BACKUP).exists() {
                stub(home)?;
            }
            return Ok(None);
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    if exists(home) {
        clear_import(home)?;
        set_aside(home)?;
        return Ok(None);
    }
    let mut state = json::read(&file, &text)?;
    let importing = home.join(IMPORTING);
    clear_import(home)?;
    let mut loaded = db::Loaded::default();
    db::Db::open_file(&importing, BUSY_WAIT)?.save_diff(&state, &mut loaded)?;
    state.held = Held::new(loaded);
    let store = home.join(db::FILE);
    match atomic::link(&importing, &store) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            clear_import(home)?;
            return Ok(None);
        }
        // A filesystem without hard links.
        Err(_) if !store.exists() => atomic::rename(&importing, &store)
            .with_context(|| format!("making {}", store.display()))?,
        _ => {}
    }
    clear_import(home)?;
    set_aside(home)?;
    Ok(Some(state))
}

/// Remove a store being made from `state.json`, or its other name once
/// it is `state.db`.
fn clear_import(home: &Path) -> Result<()> {
    for end in ["", "-wal", "-shm", "-journal"] {
        let path = home.join(format!("{IMPORTING}{end}"));
        match atomic::remove(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(e).with_context(|| format!("removing {}", path.display()));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Keep `state.json` as `state.json.bak`, a stub in its place.
fn set_aside(home: &Path) -> Result<()> {
    let file = home.join(crate::dirs::STATE);
    atomic::rename(&file, &home.join(BACKUP))
        .with_context(|| format!("setting {} aside", file.display()))?;
    stub(home)
}

/// Write the stub in `state.json`'s place, where there is none.
fn stub(home: &Path) -> Result<()> {
    if home.join(crate::dirs::STATE).exists() {
        return Ok(());
    }
    atomic::write(home, &atomic::Name::new(crate::dirs::STATE)?, STUB)
}

/// The format written: 3 records each file beside an output as it was
/// written, which version 2 named by its place alone, so a muman that
/// knows only that place refuses it rather than take a file it does not
/// name for the user's; 2 keys what yt-dlp fetched by its site, where 1
/// keyed it by yt-dlp's extractor.
pub const VERSION: u32 = 3;

/// The first format to key what yt-dlp fetched by its site.
const SITES: u32 = 2;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    /// The library folder the outputs are relative to.
    #[serde(default)]
    pub library: Option<PathBuf>,
    /// Every audio file written, by its path in the library.
    #[serde(default, with = "crate::relpath::portable_keys")]
    pub outputs: BTreeMap<PathBuf, Written>,
    /// Uploads a release was kept in place of, so listing them again
    /// fetches nothing.
    #[serde(default, deserialize_with = "json::lenient_map")]
    pub replaced: BTreeMap<SourceKey, SourceKey>,
    /// Comparisons of two sources' audio, found by the audio they
    /// compared, whichever sources hold it now: one of each pair of audio,
    /// and one of each pair of sources, at their newest audio.
    #[serde(default, deserialize_with = "json::lenient_list")]
    pub alignments: Vec<Aligned>,
    #[serde(default, deserialize_with = "json::lenient_map")]
    pub facts: BTreeMap<SourceKey, Facts>,
    /// What a source's audio holds once mixed into another layout, as a
    /// song is written mixed, found by the audio, whichever source holds
    /// it now: one of each audio in a layout, and one of each source's,
    /// at its newest audio.
    #[serde(
        default,
        deserialize_with = "json::lenient_list",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub mixes: Vec<Mix>,
    /// What could not be done for a source, so it is not tried again at
    /// once.
    #[serde(default, deserialize_with = "json::lenient_map")]
    pub failures: BTreeMap<SourceKey, Failure>,
    /// Each lookup a song made from one of its sources, and what it found.
    #[serde(default, deserialize_with = "json::lenient_list")]
    pub lookups: Vec<Looked>,
    /// What each plan fitting asked about renders to, by
    /// [`crate::limit::plan_key`].
    #[serde(default, deserialize_with = "json::lenient_map")]
    pub sizes: BTreeMap<String, Measured>,
    /// The manual files the song list listed when the library was last
    /// brought in line with it: one the list has not listed since, still
    /// in the manual folder, was taken out of it by hand.
    #[serde(default, deserialize_with = "json::lenient_list")]
    pub listed: Vec<SourceKey>,
    /// The audio a fetched source held, as the song list records it, when
    /// it was found damaged and asked for again: a whole copy fetched in
    /// its place is taken without `sync --accept`.
    #[serde(
        default,
        deserialize_with = "json::lenient_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub damaged: BTreeMap<SourceKey, String>,
    /// What the user trusted of the song list's commands and servers: a
    /// digest of each by its name (`trust`).
    #[serde(
        default,
        deserialize_with = "json::lenient_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub trusted: BTreeMap<String, String>,
    /// Sources whose failures this process cleared, for a merge to clear
    /// on disk.
    #[serde(skip)]
    pub(crate) cleared: BTreeSet<SourceKey>,
    /// Each source [`State::rebase`] named anew this process, by its
    /// facts before and after.
    #[serde(skip)]
    pub(crate) rebased: Vec<(SourceKey, Facts, Facts)>,
    /// How the file named each source key it names otherwise now, so a
    /// plan's real size is found under the name it was kept by.
    #[serde(skip)]
    pub(crate) respelled: BTreeMap<SourceKey, SourceKey>,
    /// The rows of the store this was read from, so a save writes only
    /// what differs.
    #[serde(skip)]
    pub(crate) held: Held,
}

/// What a state read from the store holds of its rows ([`db::Loaded`]);
/// none for a state made in code, whose save deletes no row. Two states
/// are equal whatever they hold.
#[derive(Default)]
pub(crate) struct Held(Mutex<db::Loaded>);

impl Held {
    fn new(loaded: db::Loaded) -> Self {
        Self(Mutex::new(loaded))
    }

    fn get(&self) -> db::Loaded {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn with<T>(&self, f: impl FnOnce(&mut db::Loaded) -> T) -> T {
        f(&mut self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    fn set(&self, loaded: db::Loaded) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = loaded;
    }
}

impl Clone for Held {
    fn clone(&self) -> Self {
        Self::new(self.get())
    }
}

impl PartialEq for Held {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Held")
    }
}

/// A lookup of `find` made from the source `from`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Looked {
    pub from: SourceKey,
    /// The provider looked up, by name.
    pub find: String,
    /// `lookup::method` of that provider when it was made.
    pub method: String,
    /// Seconds since the Unix epoch.
    pub at: u64,
    pub outcome: Outcome,
    /// How many times in a row it found again, unchanged, a record that
    /// falls short of what the song asks.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unchanged: u32,
}

// serde's `skip_serializing_if` passes the field by reference.
#[expect(clippy::trivially_copy_pass_by_ref)]
fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Found(SourceKey),
    Nothing,
    /// A record of the song that has no words.
    Instrumental,
    /// The user asked for no lookup, as `add --no-match` does.
    Declined,
    Failed {
        count: u32,
        error: String,
    },
}

impl Looked {
    #[must_use]
    pub fn now(from: SourceKey, find: Provider, outcome: Outcome) -> Self {
        Self {
            from,
            find: find.name().to_string(),
            method: crate::lookup::method(find).to_string(),
            at: now_secs(),
            outcome,
            unchanged: 0,
        }
    }

    /// When a lookup that found a record the song lists, which falls short
    /// of what the song asks, is due again, in seconds since the Unix
    /// epoch: after `recheck`, twice as long for each time it found the
    /// record again unchanged, up to [`SHORT_MAX_WAIT_SECS`].
    #[must_use]
    pub fn short_again_at(&self, recheck: std::time::Duration) -> u64 {
        let wait = recheck
            .as_secs()
            .saturating_mul(1 << self.unchanged.min(16))
            .min(SHORT_MAX_WAIT_SECS.max(recheck.as_secs()));
        self.at.saturating_add(wait)
    }

    /// Whether it is due again, by [`Self::again_at`].
    #[must_use]
    pub fn due(
        &self,
        now: u64,
        recheck: std::time::Duration,
        listed: &dyn Fn(&SourceKey) -> bool,
    ) -> bool {
        self.again_at(recheck, listed).is_some_and(|at| now >= at)
    }

    /// When it is due again, in seconds since the Unix epoch, or never:
    /// one that found nothing or an instrumental after `recheck`, one that
    /// failed after an hour doubling with each failure; one made another
    /// way at once, unless what it found the song still lists, by
    /// `listed`.
    #[must_use]
    pub fn again_at(
        &self,
        recheck: std::time::Duration,
        listed: &dyn Fn(&SourceKey) -> bool,
    ) -> Option<u64> {
        let find = Provider::named(&self.find).ok();
        if find.is_none_or(|p| crate::lookup::method(p) != self.method) {
            return (!matches!(&self.outcome, Outcome::Found(k) if listed(k))).then_some(0);
        }
        match &self.outcome {
            Outcome::Found(_) | Outcome::Declined => None,
            Outcome::Nothing | Outcome::Instrumental => {
                Some(self.at.saturating_add(recheck.as_secs()))
            }
            Outcome::Failed { count, .. } => Some(
                self.at.saturating_add(
                    3600_u64
                        .saturating_mul(1 << count.saturating_sub(1).min(16))
                        .min(MAX_WAIT_SECS),
                ),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "json::StoredWritten")]
pub struct Written {
    /// The song's sources when it was written, which is how a song
    /// whose build failed is known to keep this file.
    pub sources: Vec<SourceKey>,
    /// The files written beside it, as [`crate::outputs`] names them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sidecars: Vec<Sidecar>,
    /// What it was made from; none for a file an earlier version wrote,
    /// or one a run was about to write.
    #[serde(default)]
    pub plan: Option<Plan>,
    /// Its size and modification time when written, by which a file
    /// changed since is told from one muman wrote.
    #[serde(default)]
    pub stamp: Option<String>,
    /// A digest of its bytes when written, by which a copy of it with new
    /// times, as a backup put back, is still the file muman wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

impl Written {
    /// The paths of the files written beside it.
    pub fn sidecar_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.sidecars.iter().map(|s| &s.path)
    }

    /// Its file at `path` and those beside it.
    pub fn files<'a>(&'a self, path: &'a Path) -> impl Iterator<Item = &'a Path> {
        std::iter::once(path).chain(self.sidecars.iter().map(|s| s.path.as_path()))
    }
}

/// A file written beside an output, as its lyrics are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    #[serde(with = "crate::relpath::portable")]
    pub path: PathBuf,
    /// Its size and modification time when written, as an output's; none
    /// for one an earlier version recorded, muman's by its place alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<String>,
    /// A digest of its bytes when written, as an output's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

impl Sidecar {
    /// One at `path`, not yet vouched for.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            stamp: None,
            digest: None,
        }
    }

    /// One at `path` in `library`, vouched for as it is now.
    #[must_use]
    pub fn vouched(library: &Path, path: PathBuf) -> Self {
        let file = library.join(&path);
        Self {
            stamp: crate::store::stamp_text(&file),
            digest: crate::hash::digest_file(&file),
            path,
        }
    }
}

/// What a plan renders to, as rendered once: its audio file and each file
/// beside it, in bytes, before any hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "json::StoredMeasured")]
pub struct Measured {
    /// The plan's audio source, which keeps the entry while it is listed.
    pub source: SourceKey,
    pub audio: u64,
    /// Each file beside it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beside: Vec<u64>,
}

impl Measured {
    /// What it takes in whole blocks of `block` bytes, file by file.
    #[must_use]
    pub fn blocks(&self, block: u64) -> u64 {
        std::iter::once(self.audio)
            .chain(self.beside.iter().copied())
            .map(|bytes| crate::limit::blocks(bytes, block))
            .sum()
    }
}

/// A step that failed for a source: what it was, how often, when last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub step: Step,
    /// The revision it failed at, where the step reads the file.
    pub rev: Option<String>,
    pub error: String,
    pub count: u32,
    /// Seconds since the Unix epoch.
    pub last: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Step {
    /// Reading and measuring the file.
    Measure,
    /// Fetching it again after it went missing.
    Fetch,
    /// Fetching it again, its copy found damaged.
    Refetch,
    /// Mending a torrent's damaged file, which cannot be done: it holds
    /// every byte its torrent names, so each peer's copy is damaged
    /// alike. It is not fetched, `--retry` or not; the file changing, and
    /// measured again, starts over.
    Mend,
}

/// The longest a failed fetch waits before it is tried again.
const MAX_WAIT_SECS: u64 = 7 * 24 * 3600;

/// The longest a record that falls short waits to be asked about again:
/// long enough that one never improved costs a few requests a year, short
/// enough that one fixed upstream reaches the library within a season.
pub const SHORT_MAX_WAIT_SECS: u64 = 90 * 24 * 3600;

/// Copies of a download fetched again, each found damaged or failing,
/// after which muman stops asking until `sync --retry`.
pub const REFETCH_TRIES: u32 = 5;

impl Failure {
    /// Whether the step is due again: a measure when the file changed, a
    /// fetch once an hour has passed, doubling with each failure; a
    /// damaged download's first fetch again at once, the next as a failed
    /// fetch's, and none after [`REFETCH_TRIES`]; a mend never.
    #[must_use]
    pub fn due(&self, rev: Option<&str>, now: u64) -> bool {
        let wait = |failed: u32| {
            3600_u64
                .saturating_mul(1 << failed.saturating_sub(1).min(16))
                .min(MAX_WAIT_SECS)
        };
        match self.step {
            Step::Measure => self.rev.as_deref() != rev,
            Step::Fetch => now.saturating_sub(self.last) >= wait(self.count),
            Step::Refetch => {
                self.count <= 1
                    || self.count <= REFETCH_TRIES
                        && now.saturating_sub(self.last) >= wait(self.count - 1)
            }
            Step::Mend => false,
        }
    }
}

/// Seconds since the Unix epoch.
#[must_use]
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// What `source`'s audio, by its revision `audio`, holds mixed into
/// `layout`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mix {
    pub source: SourceKey,
    pub audio: String,
    pub layout: Layout,
    /// The [`crate::codec::MIXING`] it was mixed by.
    #[serde(default = "first_mixing")]
    pub filter: u32,
    pub analysis: Analysis,
}

/// The mixing of a mix recorded before mixes named theirs.
fn first_mixing() -> u32 {
    1
}

/// One comparison of two sources' audio, `a` against `b`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aligned {
    pub a: SourceKey,
    pub b: SourceKey,
    pub method: String,
    /// The revisions of `a` and `b` it was made on.
    pub revs: (String, String),
    /// How much later `a` plays `b`'s first moment.
    pub offset_ms: i64,
    /// How much longer `a` plays the recording than `b`, in parts per
    /// million.
    #[serde(default)]
    pub stretch_ppm: i64,
    pub score: f64,
    /// The share of `a`'s windows that agree with the offset.
    pub coverage: f64,
    /// How long each side's audio is.
    pub a_ms: i64,
    pub b_ms: i64,
    /// How long `a` plays sound outside the stretch it shares with `b`,
    /// past an edge where `b` ends of its own.
    pub a_extra_ms: i64,
    /// How long `b` plays the song on past an edge where `a` is cut off.
    #[serde(default)]
    pub a_lacks_ms: i64,
    /// How long `a` plays the song on past an edge where `b` is cut off.
    #[serde(default)]
    pub b_lacks_ms: i64,
}

impl Aligned {
    #[must_use]
    pub fn alignment(&self) -> align::Alignment {
        align::Alignment {
            offset_ms: self.offset_ms,
            stretch_ppm: self.stretch_ppm,
            score: self.score,
            coverage: self.coverage,
        }
    }

    #[must_use]
    pub fn fits(&self) -> bool {
        self.alignment().fits()
    }

    /// How much of `a` is sound that is not the recording it shares
    /// with `b`, as a video's intro or outro is. Silence padding counts
    /// for nothing, and so do the few windows inside that disagree in
    /// any pair's quiet passages, and so does what `a` plays past where
    /// `b` is cut off, which is the song going on.
    #[must_use]
    pub fn unmatched_ms(&self) -> i64 {
        self.a_extra_ms
    }
}

/// The caches of a state, apart from what the library holds: what a step
/// without the run's lock measures and keeps ([`State::keep_measures`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Caches {
    pub facts: BTreeMap<SourceKey, Facts>,
    pub failures: BTreeMap<SourceKey, Failure>,
    /// The failures cleared, which a merge clears on disk too.
    pub cleared: BTreeSet<SourceKey>,
    pub alignments: Vec<Aligned>,
    pub mixes: Vec<Mix>,
    pub sizes: BTreeMap<String, Measured>,
}

impl Caches {
    /// These as they are once `delta`, a [`State::since`] of them, is
    /// kept.
    pub fn absorb(&mut self, delta: &Self) {
        self.facts
            .extend(delta.facts.iter().map(|(k, v)| (k.clone(), v.clone())));
        for key in &delta.cleared {
            self.failures.remove(key);
        }
        self.failures
            .extend(delta.failures.iter().map(|(k, v)| (k.clone(), v.clone())));
        self.alignments.extend(delta.alignments.iter().cloned());
        self.mixes.extend(delta.mixes.iter().cloned());
        self.sizes
            .extend(delta.sizes.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
}

impl State {
    /// The state in `home`; an empty one when there is none yet. A
    /// `state.json` an earlier muman left is brought into `state.db`
    /// first.
    pub fn load(home: &Path) -> Result<Self> {
        if let Some(imported) = bring_over(home)? {
            return Ok(imported);
        }
        if !exists(home) {
            return Ok(Self {
                version: VERSION,
                ..Self::default()
            });
        }
        let (mut state, loaded) = db::Db::open(home, BUSY_WAIT)?.snapshot()?;
        state.held = Held::new(loaded);
        Ok(state)
    }

    /// Write what differs from what this was read from: a run that
    /// changed nothing leaves the home untouched.
    pub fn save(&self, home: &Path) -> Result<()> {
        bring_over(home)?;
        std::fs::create_dir_all(home).with_context(|| format!("creating {}", home.display()))?;
        let mut loaded = self.held.get();
        db::Db::open(home, BUSY_WAIT)?.save_diff(self, &mut loaded)?;
        self.held.set(loaded);
        stub(home)
    }

    /// Save the output at `path` alone, in a commit of its own: a song
    /// written is recorded as it is, at a cost that does not grow with
    /// the state, as a whole save's does.
    pub fn save_output(&self, home: &Path, path: &Path) -> Result<()> {
        let mut db = db::Db::open(home, BUSY_WAIT)?;
        self.held.with(|loaded| db.save_output(self, path, loaded))
    }

    /// Write the state in `home` to `out` as JSON, in the shape
    /// `state.json` held it, changing nothing: a `state.json` an earlier
    /// muman left is read, not brought over.
    pub fn dump(home: &Path, out: &mut impl std::io::Write) -> Result<()> {
        let file = home.join(crate::dirs::STATE);
        let state = match std::fs::read(&file) {
            Ok(text) if !exists(home) && text != STUB => json::read(&file, &text)?,
            _ if exists(home) => db::Db::open(home, BUSY_WAIT)?.snapshot()?.0,
            _ => Self {
                version: VERSION,
                ..Self::default()
            },
        };
        let text = serde_json::to_string_pretty(&state).context("writing the state")?;
        Ok(crate::ui::line(out, &text)?)
    }

    /// Record `trusted` as [`Self::trusted`] under the folder's lock,
    /// every other row left as the store holds it.
    pub fn keep_trusted(home: &Path, trusted: &BTreeMap<String, String>) -> Result<()> {
        let _lock = atomic::Lock::folder(home)?;
        let mut state = Self::load(home)?;
        state.trusted.clone_from(trusted);
        state.save(home)
    }

    /// Name each key `renames` names anew, wherever this names it: a
    /// source an extractor's name keyed, renamed by its site.
    pub fn rekey(&mut self, renames: &BTreeMap<SourceKey, SourceKey>) -> Result<()> {
        fn walk(v: &mut serde_json::Value, to: &BTreeMap<String, String>) {
            match v {
                serde_json::Value::String(s) => {
                    if let Some(new) = to.get(s.as_str()) {
                        s.clone_from(new);
                    }
                }
                serde_json::Value::Array(items) => items.iter_mut().for_each(|i| walk(i, to)),
                serde_json::Value::Object(map) => {
                    let old = std::mem::take(map);
                    for (k, mut item) in old {
                        walk(&mut item, to);
                        map.insert(to.get(&k).cloned().unwrap_or(k), item);
                    }
                }
                _ => {}
            }
        }
        if renames.is_empty() {
            return Ok(());
        }
        let to: BTreeMap<String, String> = renames
            .iter()
            .map(|(old, new)| (old.to_string(), new.to_string()))
            .collect();
        let mut value = serde_json::to_value(&*self).context("renaming sources in the state")?;
        walk(&mut value, &to);
        let mut now: Self =
            serde_json::from_value(value).context("renaming sources in the state")?;
        now.cleared = std::mem::take(&mut self.cleared);
        now.rebased = std::mem::take(&mut self.rebased);
        now.respelled = std::mem::take(&mut self.respelled);
        now.held = std::mem::take(&mut self.held);
        for (old, new) in renames {
            now.respelled.insert(new.clone(), old.clone());
        }
        *self = now;
        Ok(())
    }

    /// The comparison of `a` against `b` at these revisions, if made.
    #[must_use]
    pub fn alignment(&self, revs: &(String, String)) -> Option<&Aligned> {
        self.alignments
            .iter()
            .find(|x| x.method == align::METHOD && &x.revs == revs)
    }

    /// What the audio `audio` holds mixed into `layout`, whichever source
    /// holds it.
    #[must_use]
    pub fn mix(&self, audio: &str, layout: Layout) -> Option<&Analysis> {
        self.mixes
            .iter()
            .find(|m| m.audio == audio && m.layout == layout && m.filter == crate::codec::MIXING)
            .map(|m| &m.analysis)
    }

    pub fn record_mix(&mut self, mix: Mix) {
        self.mixes
            .retain(|m| m.layout != mix.layout || (m.audio != mix.audio && m.source != mix.source));
        self.mixes.push(mix);
    }

    /// Record that `step` failed for `key` now.
    pub fn record_failure(
        &mut self,
        key: &SourceKey,
        step: Step,
        rev: Option<String>,
        error: String,
    ) {
        let count = self
            .failures
            .get(key)
            .filter(|f| f.step == step)
            .map_or(0, |f| f.count);
        self.failures.insert(
            key.clone(),
            Failure {
                step,
                rev,
                error,
                count: count + 1,
                last: now_secs(),
            },
        );
    }

    /// Forget that a step failed for `key`, as its success shows.
    pub fn clear_failure(&mut self, key: &SourceKey) {
        if self.failures.remove(key).is_some() {
            self.cleared.insert(key.clone());
        }
    }

    /// Name `key`'s parts in every kept plan and comparison as `new`
    /// names them, where `old`, made of the same files, named them
    /// otherwise, as facts made before muman hashed sources did: facts
    /// made again of files that did not change write no song again.
    pub fn rebase(&mut self, key: &SourceKey, old: &Facts, new: &Facts) {
        if old.rev != new.rev {
            return;
        }
        for written in self.outputs.values_mut() {
            if let Some(plan) = &mut written.plan {
                *plan = plan.renamed(key, old, new);
            }
        }
        let (was, now) = (old.audio_rev(), new.audio_rev());
        for a in &mut self.alignments {
            if a.revs.0 == was {
                a.revs.0.clone_from(&now);
            }
            if a.revs.1 == was {
                a.revs.1.clone_from(&now);
            }
        }
        self.rebased.push((key.clone(), old.clone(), new.clone()));
    }

    /// `plan` as it was named before this process's [`State::rebase`].
    #[must_use]
    pub fn as_before(&self, plan: &Plan) -> Plan {
        self.rebased
            .iter()
            .rev()
            .fold(plan.clone(), |p, (key, old, new)| p.renamed(key, new, old))
            .respelled(&|k| self.respelled.get(k).cloned())
    }

    /// Take every cache `measured` holds over these: a fact replaces the
    /// one kept, a failure `measured` cleared is cleared and one it
    /// recorded replaces the one kept, and comparisons and sizes are
    /// recorded by their keys. Returns the renamings a fact made, as
    /// [`State::rebase`] makes them.
    pub(crate) fn merge_measured(&mut self, measured: &Caches) -> Vec<(SourceKey, Facts, Facts)> {
        let before = self.rebased.len();
        for (key, facts) in &measured.facts {
            if let Some(old) = self.facts.get(key).filter(|f| *f != facts).cloned() {
                self.rebase(key, &old, facts);
            }
            self.facts.insert(key.clone(), facts.clone());
        }
        for key in &measured.cleared {
            self.failures.remove(key);
        }
        for (key, failure) in &measured.failures {
            self.failures.insert(key.clone(), failure.clone());
        }
        for aligned in &measured.alignments {
            self.record_alignment(aligned.clone());
        }
        for mix in &measured.mixes {
            self.record_mix(mix.clone());
        }
        self.sizes
            .extend(measured.sizes.iter().map(|(k, v)| (k.clone(), v.clone())));
        self.rebased.split_off(before)
    }

    /// The lookup of `find` made from `from`.
    #[must_use]
    pub fn looked(&self, from: &SourceKey, find: Provider) -> Option<&Looked> {
        self.lookups
            .iter()
            .find(|l| &l.from == from && l.find == find.name())
    }

    /// Record lookups and the uploads releases replaced in the state file
    /// as it is now, under its lock: a failure counts on from the last.
    pub fn keep_lookups(
        home: &Path,
        looked: &[Looked],
        replaced: &[(SourceKey, SourceKey)],
    ) -> Result<()> {
        if looked.is_empty() && replaced.is_empty() {
            return Ok(());
        }
        let _lock = atomic::Lock::folder(home)?;
        let mut state = Self::load(home)?;
        for new in looked {
            let mut new = new.clone();
            let before = state
                .lookups
                .iter()
                .position(|l| l.from == new.from && l.find == new.find);
            if let Some(at) = before {
                let old = state.lookups.remove(at);
                if let (Outcome::Failed { count, .. }, Outcome::Failed { count: c, .. }) =
                    (&old.outcome, &mut new.outcome)
                {
                    *c = count + 1;
                }
            }
            state.lookups.push(new);
        }
        state.replaced.extend(replaced.iter().cloned());
        state.save(home)
    }

    /// Merge `measured` into the state file as it is now, under its lock,
    /// by [`Self::keep_caches`].
    pub fn keep_measures(home: &Path, measured: &Caches) -> Result<()> {
        let lock = atomic::Lock::folder(home)?;
        Self::keep_caches(home, measured, &lock)
    }

    /// Keep what this measured since `base` by [`Self::keep_measures`],
    /// and move `base` on past it, so the next keep holds only what is
    /// measured after.
    pub fn keep_since(&self, home: &Path, base: &mut Caches) -> Result<()> {
        let delta = self.since(base);
        Self::keep_measures(home, &delta)?;
        base.absorb(&delta);
        Ok(())
    }

    /// The caches this holds, for [`Self::since`].
    #[must_use]
    pub fn caches(&self) -> Caches {
        Caches {
            facts: self.facts.clone(),
            failures: self.failures.clone(),
            cleared: self.cleared.clone(),
            alignments: self.alignments.clone(),
            mixes: self.mixes.clone(),
            sizes: self.sizes.clone(),
        }
    }

    /// What this holds of the caches that `base`, its caches as they
    /// were, does not: each entry added or changed since, and each
    /// failure gone since, as cleared.
    #[must_use]
    pub fn since(&self, base: &Caches) -> Caches {
        fn changed<K: Ord + Clone, V: PartialEq + Clone>(
            now: &BTreeMap<K, V>,
            was: &BTreeMap<K, V>,
        ) -> BTreeMap<K, V> {
            now.iter()
                .filter(|(k, v)| was.get(*k) != Some(*v))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        }
        fn added<T: PartialEq + Clone>(now: &[T], was: &[T]) -> Vec<T> {
            now.iter().filter(|x| !was.contains(x)).cloned().collect()
        }
        Caches {
            facts: changed(&self.facts, &base.facts),
            failures: changed(&self.failures, &base.failures),
            cleared: (base.failures.keys())
                .filter(|k| !self.failures.contains_key(*k))
                .cloned()
                .collect(),
            alignments: added(&self.alignments, &base.alignments),
            mixes: added(&self.mixes, &base.mixes),
            sizes: changed(&self.sizes, &base.sizes),
        }
    }

    /// Keep every cache `measured` holds in the store as it is now, for a
    /// caller already holding its lock, by `merge_measured`, in
    /// one transaction that writes the caches alone: what the library
    /// holds is left to the run that holds its lock, which writes the
    /// names a fact gives its source's parts anew into the plans.
    pub fn keep_caches(home: &Path, measured: &Caches, _lock: &atomic::Lock) -> Result<()> {
        bring_over(home)?;
        std::fs::create_dir_all(home).with_context(|| format!("creating {}", home.display()))?;
        db::Db::open(home, BUSY_WAIT)?.keep(measured)?;
        stub(home)
    }

    pub fn record_alignment(&mut self, aligned: Aligned) {
        self.alignments
            .retain(|x| x.revs != aligned.revs && !(x.a == aligned.a && x.b == aligned.b));
        self.alignments.push(aligned);
    }
}

/// Make `home` one an earlier muman left, its state `json` in
/// `state.json` and no store.
#[cfg(any(test, feature = "testing"))]
// Writes the files an earlier muman left, for a test.
#[expect(clippy::disallowed_methods)]
pub fn put_old_state(home: &Path, json: &[u8]) {
    for name in ["state.db", "state.db-wal", "state.db-shm", "state.json.bak"] {
        let _ = std::fs::remove_file(home.join(name));
    }
    std::fs::write(home.join(crate::dirs::STATE), json).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dirs::STATE;

    #[test]
    fn a_damaged_download_is_fetched_again_at_once_then_less_often_then_not() {
        let failure = |count: u32| Failure {
            step: Step::Refetch,
            rev: None,
            error: String::new(),
            count,
            last: 1_000_000,
        };
        let hour = 3600;
        assert!(failure(1).due(None, 1_000_000), "the first at once");
        assert!(!failure(2).due(None, 1_000_000 + hour - 1));
        assert!(failure(2).due(None, 1_000_000 + hour));
        assert!(!failure(3).due(None, 1_000_000 + hour));
        assert!(failure(3).due(None, 1_000_000 + 2 * hour));
        assert!(failure(REFETCH_TRIES).due(None, 1_000_000 + 30 * 24 * hour));
        assert!(
            !failure(REFETCH_TRIES + 1).due(None, 1_000_000 + 30 * 24 * hour),
            "given up on until `--retry`"
        );
    }

    #[test]
    fn an_instrumental_is_asked_again_and_a_listed_find_survives_a_new_method() {
        let day = 24 * 3600;
        let looked = |outcome: Outcome, method: &str| Looked {
            from: SourceKey::youtube("aaaaaaaaaaa"),
            find: "lrclib".into(),
            method: method.into(),
            at: 0,
            outcome,
            unchanged: 0,
        };
        let method = crate::lookup::method(Provider::Lrclib);
        let none = |_: &SourceKey| false;
        let instrumental = looked(Outcome::Instrumental, method);
        assert!(!instrumental.due(day, std::time::Duration::from_secs(7 * day), &none));
        assert!(instrumental.due(7 * day, std::time::Duration::from_secs(7 * day), &none));
        let record = SourceKey::parse("lrclib:7").unwrap();
        let old = looked(Outcome::Found(record.clone()), "lrclib/1");
        assert!(
            !old.due(0, std::time::Duration::from_secs(7 * day), &|k| *k
                == record),
            "the song lists what it found"
        );
        assert!(
            old.due(0, std::time::Duration::from_secs(7 * day), &none),
            "found, never joined: asked again"
        );
        assert!(!looked(Outcome::Found(record), method).due(
            100 * day,
            std::time::Duration::from_secs(7 * day),
            &none
        ));
    }

    #[test]
    fn a_merge_clears_what_a_step_cleared_and_keeps_what_another_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (
            SourceKey::youtube("aaaaaaaaaaa"),
            SourceKey::youtube("bbbbbbbbbbb"),
            SourceKey::youtube("ccccccccccc"),
        );
        let mut disk = State::default();
        disk.record_failure(&a, Step::Fetch, None, "gone".into());
        disk.record_failure(&b, Step::Measure, Some("1".into()), "unreadable".into());
        disk.save(dir.path()).unwrap();
        let mut measured = State::load(dir.path()).unwrap();
        let mut base = measured.caches();
        measured.clear_failure(&a);
        let mut meanwhile = State::load(dir.path()).unwrap();
        meanwhile.record_failure(&c, Step::Fetch, None, "gone too".into());
        meanwhile.save(dir.path()).unwrap();
        measured.keep_since(dir.path(), &mut base).unwrap();
        let after = State::load(dir.path()).unwrap();
        assert_eq!(
            after.failures.keys().collect::<Vec<_>>(),
            [&b, &c],
            "a's success clears it; b and c stay"
        );
    }

    #[test]
    fn a_step_keeps_what_it_measured_and_not_the_state_it_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let (k, j, l) = (
            SourceKey::youtube("aaaaaaaaaaa"),
            SourceKey::youtube("bbbbbbbbbbb"),
            SourceKey::youtube("ccccccccccc"),
        );
        let mut disk = State::default();
        disk.facts
            .insert(k.clone(), Facts::unreadable("1:1".into()));
        disk.save(dir.path()).unwrap();
        let mut step = State::load(dir.path()).unwrap();
        let mut base = step.caches();
        assert_eq!(step.since(&base), Caches::default(), "nothing measured yet");

        let mut meanwhile = State::load(dir.path()).unwrap();
        meanwhile
            .facts
            .insert(k.clone(), Facts::unreadable("2:2".into()));
        meanwhile.record_failure(&j, Step::Fetch, None, "gone".into());
        meanwhile.record_alignment(aligned(0, 1.0, 1000, 1000));
        meanwhile.save(dir.path()).unwrap();

        step.facts
            .insert(l.clone(), Facts::unreadable("3:3".into()));
        step.keep_since(dir.path(), &mut base).unwrap();
        let after = State::load(dir.path()).unwrap();
        assert_eq!(after.facts[&k].rev, "2:2", "measured meanwhile, not loaded");
        assert_eq!(after.facts[&l].rev, "3:3");
        assert!(after.failures.contains_key(&j));
        assert_eq!(after.alignments.len(), 1);

        step.facts
            .insert(l.clone(), Facts::unreadable("4:4".into()));
        let mut later = State::load(dir.path()).unwrap();
        later.clear_failure(&j);
        later.save(dir.path()).unwrap();
        step.keep_since(dir.path(), &mut base).unwrap();
        let after = State::load(dir.path()).unwrap();
        assert_eq!(after.facts[&l].rev, "4:4");
        assert!(
            !after.failures.contains_key(&j),
            "a failure the step never saw is not put back"
        );
    }

    #[test]
    fn facts_hashed_elsewhere_name_the_kept_plans_by_their_digests() {
        let dir = tempfile::tempdir().unwrap();
        let key = SourceKey::youtube("aaaaaaaaaaa");
        let mut old = Facts::unreadable("10:20".into());
        old.audio = Some(crate::facts::AudioFacts {
            index: 1,
            codec: "opus".into(),
            channels: 2,
            ..crate::facts::AudioFacts::default()
        });
        let plan = crate::resolve::Plan {
            folder: None,
            version: 1,
            format: crate::resolve::Format::Copy {
                codec: crate::codec::Codec::Opus,
            },
            audio: crate::resolve::AudioRef {
                key: key.clone(),
                rev: "10:20".into(),
                index: 1,
            },
            cover: None,
            lyrics: None,
            tags: Vec::new(),
            loudness: None,
            tagging: crate::resolve::TAGGING,
            chapters: Vec::new(),
        };
        let mut disk = State::default();
        disk.facts.insert(key.clone(), old.clone());
        disk.outputs.insert(
            "song.opus".into(),
            Written {
                sources: vec![key.clone()],
                sidecars: Vec::new(),
                plan: Some(plan),
                stamp: None,
                digest: None,
            },
        );
        disk.save(dir.path()).unwrap();
        let mut measured = State::load(dir.path()).unwrap();
        let mut base = measured.caches();
        let mut new = old;
        new.audio.as_mut().unwrap().digest = Some("0123456789abcdef0123456789abcdef".into());
        measured.facts.insert(key, new);
        measured.keep_since(dir.path(), &mut base).unwrap();
        let after = State::load(dir.path()).unwrap();
        let plan = after
            .outputs
            .values()
            .next()
            .unwrap()
            .plan
            .as_ref()
            .unwrap();
        assert_eq!(plan.audio.rev, "0123456789abcdef0123456789abcdef");

        // The step wrote its caches alone: the kept row still names the
        // old rev, renamed on every load until a whole save writes it.
        let row = |name: &str| {
            let db = rusqlite::Connection::open(dir.path().join(db::FILE)).unwrap();
            let count: i64 = db
                .query_row(&format!("SELECT count(*) FROM {name}"), [], |r| r.get(0))
                .unwrap();
            let output: String = db
                .query_row("SELECT row FROM outputs", [], |r| r.get(0))
                .unwrap();
            (count, output.contains("0123456789abcdef"))
        };
        assert_eq!(row("renamings"), (1, false));
        after.save(dir.path()).unwrap();
        assert_eq!(row("renamings"), (0, true), "written renamed, and cleared");
        let again = State::load(dir.path()).unwrap();
        assert_eq!(again.outputs, after.outputs);
    }

    #[test]
    fn a_state_keyed_by_extractor_reads_by_site_and_names_its_plans_as_it_did() {
        let dir = tempfile::tempdir().unwrap();
        let (key, record) = (
            SourceKey::youtube("aaaaaaaaaaa"),
            SourceKey::parse("lrclib:7").unwrap(),
        );
        let plan = |audio: SourceKey| crate::resolve::Plan {
            folder: None,
            version: 1,
            format: crate::resolve::Format::Copy {
                codec: crate::codec::Codec::Opus,
            },
            audio: crate::resolve::AudioRef {
                key: audio,
                rev: "10:20".into(),
                index: 1,
            },
            cover: None,
            lyrics: None,
            tags: Vec::new(),
            loudness: None,
            tagging: crate::resolve::TAGGING,
            chapters: Vec::new(),
        };
        let old = SourceKey::Remote {
            site: "youtube".into(),
            id: "aaaaaaaaaaa".into(),
        };
        let mut disk = State::default();
        disk.facts
            .insert(key.clone(), Facts::unreadable("10:20".into()));
        disk.facts
            .insert(record.clone(), Facts::unreadable("1:2".into()));
        let text = serde_json::to_string(&disk)
            .unwrap()
            .replace("\"youtube.com:", "\"youtube:")
            .replacen("\"version\":0", "\"version\":1", 1);
        std::fs::write(dir.path().join(STATE), text).unwrap();
        let state = State::load(dir.path()).unwrap();
        assert_eq!(state.version, VERSION);
        assert!(state.facts.contains_key(&key) && state.facts.contains_key(&record));
        let tools = "ffmpeg version fake";
        assert_eq!(
            crate::limit::plan_key(tools, &state.as_before(&plan(key.clone()))),
            crate::limit::plan_key(tools, &plan(old)),
            "a size the plan was kept by is found"
        );
        disk.version = VERSION;
        disk.save(dir.path()).unwrap();
        let now = State::load(dir.path()).unwrap();
        assert_eq!(
            now.as_before(&plan(key.clone())),
            plan(key),
            "named so already"
        );
    }

    #[test]
    fn a_source_rekeyed_is_named_anew_everywhere_and_its_plans_as_before() {
        let (old, new) = (
            SourceKey::parse("funkwhale:abc").unwrap(),
            SourceKey::parse("tunes.example:abc").unwrap(),
        );
        let mut state = State::default();
        state
            .facts
            .insert(old.clone(), Facts::unreadable("1:2".into()));
        state.listed.push(old.clone());
        state.outputs.insert(
            "song.opus".into(),
            Written {
                sources: vec![old.clone()],
                sidecars: Vec::new(),
                plan: None,
                stamp: None,
                digest: None,
            },
        );
        state.rekey(&[(old.clone(), new.clone())].into()).unwrap();
        assert!(state.facts.contains_key(&new) && !state.facts.contains_key(&old));
        assert_eq!(state.listed, std::slice::from_ref(&new));
        assert_eq!(
            state.outputs[Path::new("song.opus")].sources,
            std::slice::from_ref(&new)
        );
        assert_eq!(state.respelled.get(&new), Some(&old));
    }

    #[test]
    fn respelling_a_plan_names_every_key_the_state_renames() {
        let (old, new) = (
            SourceKey::parse("funkwhale:abc").unwrap(),
            SourceKey::parse("tunes.example:abc").unwrap(),
        );
        let cover = |key: &SourceKey| crate::resolve::CoverRef {
            key: key.clone(),
            rev: "1:2".into(),
            at: crate::facts::CoverAt::Attachment { ordinal: 1 },
            mimetype: "image/jpeg".into(),
            crop: None,
            made: None,
            out: None,
        };
        let plan = |key: &SourceKey| crate::resolve::Plan {
            version: 1,
            format: crate::resolve::Format::Copy {
                codec: crate::codec::Codec::Opus,
            },
            audio: crate::resolve::AudioRef {
                key: key.clone(),
                rev: "1:2".into(),
                index: 1,
            },
            cover: Some(cover(key)),
            lyrics: None,
            tags: Vec::new(),
            loudness: None,
            tagging: crate::resolve::TAGGING,
            chapters: Vec::new(),
            folder: Some(cover(key)),
        };
        let mut state = State::default();
        state.outputs.insert(
            "song.opus".into(),
            Written {
                sources: vec![old.clone()],
                sidecars: Vec::new(),
                plan: Some(plan(&old)),
                stamp: None,
                digest: None,
            },
        );
        state.rekey(&[(old.clone(), new.clone())].into()).unwrap();
        let kept = state.outputs[Path::new("song.opus")].plan.clone().unwrap();
        assert_eq!(kept, plan(&new));
        assert_eq!(state.as_before(&kept), plan(&old));
    }

    #[test]
    fn a_mix_is_found_only_as_the_filter_it_was_mixed_by_makes_it() {
        let key = SourceKey::youtube("aaaaaaaaaaa");
        let layout = Layout::STEREO;
        let mut state = State::default();
        let saved = serde_json::json!({
            "source": key.to_string(), "audio": "1:2", "layout": "stereo", "analysis": {},
        });
        state.mixes.push(serde_json::from_value(saved).unwrap());
        assert_eq!(
            state.mixes[0].filter, 1,
            "recorded before mixes named theirs"
        );
        assert!(state.mix("1:2", layout).is_some());

        state.mixes[0].filter = crate::codec::MIXING + 1;
        assert!(state.mix("1:2", layout).is_none(), "another filter's");
        state.record_mix(Mix {
            source: key.clone(),
            audio: "1:2".into(),
            layout,
            filter: crate::codec::MIXING,
            analysis: Analysis::default(),
        });
        assert_eq!(state.mixes.len(), 1, "the old filter's goes");
        assert!(state.mix("1:2", layout).is_some());
    }

    fn aligned(offset_ms: i64, coverage: f64, a_ms: i64, b_ms: i64) -> Aligned {
        let start = offset_ms.max(0);
        let end = a_ms.min(b_ms + offset_ms);
        Aligned {
            a: SourceKey::youtube("aaaaaaaaaaa"),
            b: SourceKey::youtube("bbbbbbbbbbb"),
            method: align::METHOD.into(),
            revs: ("1".into(), "2".into()),
            offset_ms,
            stretch_ppm: 0,
            score: 0.99,
            coverage,
            a_ms,
            b_ms,
            a_extra_ms: a_ms - (end - start).max(0),
            a_lacks_ms: 0,
            b_lacks_ms: 0,
        }
    }

    #[test]
    fn a_video_s_intro_is_unmatched_and_the_release_is_not() {
        let video = aligned(20_000, 1.0, 200_000, 180_000);
        assert_eq!(video.unmatched_ms(), 20_000);
        let release = aligned(-20_000, 1.0, 180_000, 200_000);
        assert_eq!(release.unmatched_ms(), 0);
    }

    /// A state an earlier muman kept in `home`'s `state.json`.
    fn old_home(home: &Path) -> State {
        let mut old = State {
            version: VERSION,
            ..State::default()
        };
        old.replaced.insert(
            SourceKey::youtube("uuuuuuuuuuu"),
            SourceKey::youtube("ttttttttttt"),
        );
        old.listed
            .push(SourceKey::Manual("Lantern Weather.flac".into()));
        json::write(&old, home).unwrap();
        old
    }

    #[test]
    fn a_state_json_is_brought_over_once_kept_aside_and_stubbed() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let old = old_home(home);
        let text = std::fs::read(home.join(STATE)).unwrap();
        let state = State::load(home).unwrap();
        assert_eq!(
            (&state.replaced, &state.listed),
            (&old.replaced, &old.listed)
        );
        assert!(exists(home));
        assert_eq!(std::fs::read(home.join(BACKUP)).unwrap(), text);
        assert_eq!(std::fs::read(home.join(STATE)).unwrap(), STUB);
        let e = json::read(&home.join(STATE), STUB).err().unwrap();
        assert!(
            format!("{e:#}").contains("newer than this muman's 3"),
            "a muman of the json state refuses the home: {e:#}"
        );
        let again = State::load(home).unwrap();
        assert_eq!(
            (&again.replaced, &again.listed),
            (&old.replaced, &old.listed)
        );
        assert_eq!(
            std::fs::read(home.join(BACKUP)).unwrap(),
            text,
            "brought over once"
        );
    }

    #[test]
    fn a_state_json_brought_over_and_stopped_anywhere_ends_in_the_store() {
        let dir = tempfile::tempdir().unwrap();
        old_home(dir.path());
        let counted = crate::kill::kill_at(dir.path(), usize::MAX);
        State::load(dir.path()).unwrap();
        let steps = counted.seen();
        drop(counted);
        assert!(steps > 3, "{steps}");
        for at in 1..=steps {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            let old = old_home(home);
            let kill = crate::kill::kill_at(home, at);
            let stopped = std::panic::catch_unwind(|| State::load(home));
            drop(kill);
            assert!(stopped.is_err(), "stopped at step {at} of {steps}");
            let state = State::load(home).unwrap();
            assert_eq!(
                (&state.replaced, &state.listed),
                (&old.replaced, &old.listed),
                "step {at}"
            );
            assert!(exists(home) && home.join(BACKUP).exists(), "step {at}");
            assert_eq!(std::fs::read(home.join(STATE)).unwrap(), STUB, "step {at}");
            assert!(!home.join(IMPORTING).exists(), "step {at}");
        }
    }

    #[test]
    fn a_missing_state_is_empty_and_a_saved_one_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = State::load(dir.path()).unwrap();
        assert!(state.outputs.is_empty());
        state.replaced.insert(
            SourceKey::youtube("uuuuuuuuuuu"),
            SourceKey::youtube("ttttttttttt"),
        );
        state.record_alignment(aligned(0, 1.0, 1, 1));
        state.record_alignment(aligned(5, 1.0, 1, 1));
        state.save(dir.path()).unwrap();
        let file = dir.path().join(db::FILE);
        let then = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1 << 30);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(then)
            .unwrap();
        state.save(dir.path()).unwrap();
        assert_eq!(
            std::fs::metadata(&file).unwrap().modified().unwrap(),
            then,
            "the same state is not written again"
        );
        let back = State::load(dir.path()).unwrap();
        assert_eq!(back.replaced, state.replaced);
        assert_eq!(
            back.alignments.len(),
            1,
            "a comparison made again replaces the old"
        );
        let revs = ("1".to_string(), "2".to_string());
        assert_eq!(back.alignment(&revs).unwrap().offset_ms, 5);
        assert!(back.alignment(&("1".into(), "3".into())).is_none());
    }

    #[test]
    fn a_comparison_or_a_mix_is_found_by_its_audio_whichever_source_holds_it() {
        let mut state = State::default();
        state.record_alignment(aligned(5, 1.0, 1, 1));
        let copy = Aligned {
            a: SourceKey::Manual("Lantern Weather.flac".into()),
            ..aligned(5, 1.0, 1, 1)
        };
        assert_eq!(
            state.alignment(&copy.revs).unwrap().offset_ms,
            5,
            "another source holding the same audio"
        );
        state.record_alignment(Aligned {
            revs: ("1".into(), "3".into()),
            ..aligned(9, 1.0, 1, 1)
        });
        assert_eq!(
            state.alignments.len(),
            1,
            "the pair's comparison at new audio replaces the old"
        );
        state.record_alignment(Aligned {
            revs: ("1".into(), "3".into()),
            ..copy
        });
        assert_eq!(state.alignments.len(), 1, "one comparison of that audio");

        let mix = |source: SourceKey, audio: &str| Mix {
            source,
            audio: audio.into(),
            layout: Layout::STEREO,
            filter: crate::codec::MIXING,
            analysis: Analysis::default(),
        };
        state.record_mix(mix(SourceKey::youtube("aaaaaaaaaaa"), "1:2"));
        assert!(state.mix("1:2", Layout::STEREO).is_some());
        state.record_mix(mix(SourceKey::Manual("Lantern Weather.flac".into()), "1:2"));
        assert_eq!(state.mixes.len(), 1, "one mix of that audio");
        state.record_mix(mix(SourceKey::Manual("Lantern Weather.flac".into()), "1:3"));
        assert_eq!(
            state.mixes.len(),
            1,
            "the source's mix of new audio replaces it"
        );
        assert!(state.mix("1:2", Layout::STEREO).is_none());
    }

    #[test]
    fn cache_entries_of_another_shape_are_dropped_and_outputs_kept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(STATE),
            r#"{"version": 1,
                "outputs": {"A/B/C.opus": {"sources": ["youtube.com:aaaaaaaaaaa"]}},
                "alignments": [{"a": "youtube.com:aaaaaaaaaaa", "old": true}],
                "facts": {"youtube.com:aaaaaaaaaaa": {"rev": "1"}}}"#,
        )
        .unwrap();
        let state = State::load(dir.path()).unwrap();
        assert_eq!(state.outputs.len(), 1);
        assert!(state.alignments.is_empty() && state.facts.is_empty());
    }

    #[test]
    fn lyrics_an_earlier_version_named_by_their_place_are_a_file_beside_their_song() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(STATE),
            r#"{"version": 2,
                "outputs": {"A/x.opus": {"sources": [], "lyrics": "A/x.lrc", "stamp": "1:1"}}}"#,
        )
        .unwrap();
        let state = State::load(dir.path()).unwrap();
        assert_eq!(
            state.outputs[Path::new("A/x.opus")].sidecars,
            [Sidecar::at("A/x.lrc".into())],
            "muman's by its place, as before"
        );
        state.save(dir.path()).unwrap();
        assert!(
            std::fs::read_to_string(dir.path().join(BACKUP))
                .unwrap()
                .contains("\"lyrics\""),
            "kept as it was"
        );
        assert_eq!(std::fs::read(dir.path().join(STATE)).unwrap(), STUB);
        assert_eq!(State::load(dir.path()).unwrap().outputs, state.outputs);
    }

    #[test]
    fn a_broken_or_newer_state_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(STATE), "{").unwrap();
        assert!(format!("{:#}", State::load(dir.path()).unwrap_err()).contains("move it aside"));
        std::fs::write(dir.path().join(STATE), "{\"version\": 9}").unwrap();
        assert!(format!("{:#}", State::load(dir.path()).unwrap_err()).contains("newer"));
        std::fs::write(
            dir.path().join(STATE),
            r#"{"version": 1, "outputs": {"../../elsewhere.opus": {"sources": []}}}"#,
        )
        .unwrap();
        let e = format!("{:#}", State::load(dir.path()).unwrap_err());
        assert!(e.contains(STATE) && e.contains("outside its folder"), "{e}");
    }
}
