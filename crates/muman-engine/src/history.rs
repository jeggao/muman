//! Each run that changes the song list or the library, kept so `undo`
//! can put both back: the song list before and after, the outputs it
//! changed as they were, the library files it replaced or removed, as
//! many as a budget of bytes allows, and the files it moved. A file not
//! kept is written again from its sources instead.
//!
//! Every library write lands as a new file renamed into place, so a hard
//! link to the old one keeps its bytes; a file is linked where the
//! filesystem allows, else copied, while the `[history]` budget lasts.
//! The budget is of every kept run together, and each run that records
//! holds the history to it: the files of the oldest runs go first, as the
//! run needs room and again as it ends, so a budget lowered since binds
//! the runs kept before it too. The budget and the count are those of the
//! song list as the run begins, a hand edit since the last run included,
//! which is the run's to apply. A run whose files went keeps its record:
//! `undo` puts its song list back and has the sync write those files
//! again from their sources, as for any file not kept. Past the budget, a
//! run that rewrites the whole library keeps part of it and loses
//! nothing. The history is safe to lose: it only makes `undo` possible,
//! and a history that cannot be written, as a folder made read-only,
//! costs a run only that, with a warning.
//!
//! The record is written ahead of what it records: before the run moves,
//! writes over or removes a library file, which a sync does only through
//! [`crate::library::Ledger`], or writes a new one, and again as it goes,
//! so a run stopped partway, by a crash or Ctrl-C, is still listed, as
//! interrupted, and can be undone. Such a run may have saved the song list
//! after its last record, stopped between a save and its note, so `undo`
//! takes back the song list as it stands, saying so where it differs from
//! that record: a hand edit since is told apart from the run's own save
//! by nothing on disk. The next run that changes the home,
//! holding the runs lock that proves the stopped one gone, goes on in
//! its record rather than begin its own: the song list before and the
//! files kept are the stopped run's, so one undo takes back both, and
//! what it made is not kept as though it had been there before. A
//! folder with no record, left by a run stopped before it wrote one,
//! holds nothing `undo` could use and is let go of with the runs past
//! the count. A file such a run moved before
//! the state could say so is followed by the next run, by the moves the
//! record lists, so it stays muman's. `undo` goes in the reverse of the
//! run's order: files put back where the run wrote over them, then moves
//! reversed, as a song retagged where it moved to needs, then the files
//! no song kept that the run removed to free a path a move landed on, put
//! back where the moves back left it free: kept apart from the files it
//! wrote over, as a song moved there and retagged is kept at that path
//! too. Then each file
//! the run made, an output or one beside it, taken out: the sync after
//! cannot, as the song list put back may still name its path, and a file
//! no record names is the user's to it. A file the user
//! had changed since muman wrote it, which only a forced run writes over,
//! is noted so as it is kept and put back with the stamp it no longer
//! matches, still the user's to the sync after.
//!
//! A run's song list before is the one the last kept run left, so a hand
//! edit is undone with the run that applied it. A command that changes
//! the home holds a lock of its own from first to last, `edit` from the
//! moment its editor closes, so a run waiting on another begins where that
//! one ended rather than record the other's change as its own. The song
//! list a run leaves is the one it last read or wrote, as its sync
//! reports it ([`Run::saw`]), so a hand edit made while it ran, which it
//! never read, is the next run's.
//!
//! A run begins, taking the runs lock and writing its record, before the
//! song list is saved, and each save is noted in the record as it is
//! written: every save takes a [`crate::manifest::Recorder`], which only a
//! begun run, the undo whose sync saves and the list a new home starts
//! with are, so a save outside a run does not build. Every other order
//! has failed: a run begun after the save, with no earlier run
//! kept, took the changed list for its list before, and Ctrl-C while it
//! waited on another run left the change saved and no run's; a record
//! first written after the save left a kill between the two a change
//! `undo` refused as a hand edit; a mark written ahead at some saves
//! alone had a run stopped after another refused, and one stopped at a
//! marked save take back a hand edit unsaid. No question waits while the
//! runs lock is held, as every other run would wait on it: a command
//! that asks begins its run only at its save ([`Later`]), lets go of it
//! before it asks again, a save refused included, and goes on in its
//! record at the next save, as a run goes on in a stopped one's. The sync
//! after an undo saves as the undo records ([`Undone`]), under the runs
//! lock the undo holds.
//!
//! `undo` holds the library's lock while
//! it puts files back, as a sync does while it writes them, and
//! refuses when the song list changed after the run, by hand or
//! otherwise, rather than lose those changes. It marks the record
//! `undoing` before it changes anything and skips each step already
//! done, so an undo stopped partway finishes when run again. The record
//! goes only once the sync after it has brought the library in line:
//! the song list and the state put back, it is marked `undone`, so an
//! undo stopped in that sync is still the latest run, which the next
//! `undo` finishes rather than undo the run before it too, and `check`
//! names. A run begun meanwhile begins from the song list put back, and
//! one whose sync ends finishes the undo, letting go of the record. One
//! stopped before it put the song list and the state back is finished
//! by `undo` alone ([`undo_unfinished`]): a sync would leave what it put
//! back beside what it had not yet removed, so a command that changes
//! the library or the list refuses until then. A purge
//! cannot be undone in full: a fetched source is fetched again by the
//! next sync, and a file of the user's own is in the desktop's trash,
//! which muman cannot take it back from on every platform. The run
//! records each it trashed, and `undo` refuses until the user puts them
//! back, rather than list their songs again with nothing to make them:
//! where each was, or where a run since followed it, as the list put back
//! names it there.
//! What `add` copied into the sources is recorded with a digest of it,
//! and `undo` takes each copy out with its song, else the next sync
//! would list it again as a file dropped in: a copy as made is deleted,
//! the file it came from untouched, and one changed since goes to the
//! trash. A copy gone from its path is looked for where the user may
//! have moved it within the manual folder, by that digest, among the
//! files the song list put back names nowhere, as a sync follows a moved
//! file by its bytes, and taken out the same: left, it would be listed
//! again too. Two files holding it are both left, and named, as which is
//! the copy cannot be told.
//!
//! A run that follows files the user moved in `sources/manual` records
//! the moves ahead of the save that writes them ([`Run::followed`]), and
//! `undo` puts the song list back with them made, and the outputs too, all
//! at once, so a swap stays one: the user's file stays where the user
//! moved it and the state keeps its record there, so the list put back
//! names each file where it is, and no sync after has to find it again by
//! what it holds, which two alike files would leave in doubt. The run
//! before is moved on to the list the undo leaves with those moves added
//! to its own, so its undo after puts back its list with them made too.
//! Once the sync after the undo saves the list, the list saved is the
//! undo's list before, and no move is left to make.
//!
//! A record holds the song list's text, and with it any login or token
//! the song list holds, so on Unix the history's folders are made `0700`
//! and a record is written `0600` every time, one an older muman left
//! readable to all made private as an undo rewrites it
//! ([`crate::atomic`]). A folder that exists is left as it is, as the
//! home is, and no record is rewritten only to make it private: what the
//! owner made of their home's permissions is theirs to change.
//!
//! `run.json` carries a `version`; a record without one is of version 1.
//! Version 2 keeps each output the run changed as it was, none for one it
//! added, noted as the ledger moves, writes over or removes a file, as a
//! sync records ahead the songs it is about to write, and as it saves
//! outputs it changed otherwise; read, the outputs as the run began are
//! the outputs now with those put back. Version 1 kept every output, which
//! grew with the library; its records are undone as before, and an
//! earlier muman cannot read version 2's. Version 3 keeps the moves the
//! run followed; an earlier muman reads it as version 2, and puts back the
//! keys before, as an undo of a version 2 record does, for the sync after
//! to follow the files again.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::atomic;
use crate::dirs::{Dirs, MANIFEST};
use crate::source::SourceKey;
use crate::state::{State, Written};
use crate::store;

const RECORD: &str = "run.json";
const VERSION: u32 = 3;
/// The folder of a run's that keeps the files it freed paths of.
const FREED: &str = "freed";

fn version_one() -> u32 {
    1
}

fn complete() -> bool {
    true
}

/// A library file a run moved, as a new template moves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Moved {
    #[serde(with = "crate::relpath::portable")]
    from: PathBuf,
    #[serde(with = "crate::relpath::portable")]
    to: PathBuf,
}

/// What one run changed, as `undo` reads it.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Record {
    #[serde(default = "version_one")]
    version: u32,
    /// Whether the run ended; a record written ahead says no.
    #[serde(default = "complete")]
    complete: bool,
    /// Set by an `undo` under way, so a second one finishes it.
    #[serde(default)]
    undoing: bool,
    /// Set once that undo put the song list and the state back, leaving
    /// only its sync to do.
    #[serde(default)]
    undone: bool,
    songs_before: Option<String>,
    songs_after: Option<String>,
    /// The outputs as the run began, whole, as version 1 kept them.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "crate::relpath::portable_keys"
    )]
    outputs: BTreeMap<PathBuf, Written>,
    /// Each output the run changed as it was before, none for one it
    /// added: version 2's.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "crate::relpath::portable_keys"
    )]
    before: BTreeMap<PathBuf, Option<Written>>,
    /// The outputs as the run began, whole: version 1's, or version 2's
    /// made from the outputs now and those it changed.
    #[serde(skip)]
    was: BTreeMap<PathBuf, Written>,
    /// Library files the run replaced or removed.
    #[serde(with = "crate::relpath::portable_keys")]
    touched: BTreeSet<PathBuf>,
    /// Those kept under the run's `files`.
    #[serde(with = "crate::relpath::portable_keys")]
    kept: BTreeSet<PathBuf>,
    /// Those of them that were muman's files changed since it wrote
    /// them, as a forced run writes over a file retagged by hand: put
    /// back, they are still changed since written.
    #[serde(default, with = "crate::relpath::portable_keys")]
    changed: BTreeSet<PathBuf>,
    /// The bytes those take.
    #[serde(default)]
    bytes: u64,
    /// Library files the run moved, in the order it moved them.
    #[serde(default)]
    moves: Vec<Moved>,
    /// Library files no song kept that the run removed before its moves,
    /// to free their paths for a move, each kept under the run's `freed`
    /// while the budget lasts: `undo` puts them back once it has moved the
    /// files back, where a file the run kept under `files` goes before.
    #[serde(
        default,
        skip_serializing_if = "BTreeSet::is_empty",
        with = "crate::relpath::portable_keys"
    )]
    freed: BTreeSet<PathBuf>,
    /// Files of the user's own the run moved to the desktop's trash,
    /// under the home.
    #[serde(default, with = "crate::relpath::portable_keys")]
    trashed: BTreeSet<PathBuf>,
    /// Files `add` copied into the sources, under the home, with a digest
    /// of each as copied.
    #[serde(default, with = "crate::relpath::portable_keys")]
    copied: BTreeMap<PathBuf, String>,
    /// Each key of the song list before that the run followed a file of
    /// the user's from, by the key it followed it to: version 3's.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    followed: BTreeMap<SourceKey, SourceKey>,
}

/// One run, recording as it goes; written ahead by [`Run::checkpoint`]
/// and at the end by [`Run::finish`] only when the run changed anything.
// Each flag is one fact of the run's own: what it knows, wrote, reached
// and began.
#[expect(clippy::struct_excessive_bools)]
#[derive(Debug)]
pub struct Run {
    home: PathBuf,
    dir: PathBuf,
    record: Record,
    outputs_known: bool,
    /// The outputs the run changed or is about to.
    changing: BTreeSet<PathBuf>,
    /// The other kept runs, oldest first, with the bytes each keeps.
    others: Vec<(PathBuf, u64)>,
    /// The song list's `[history]` as the run began, a hand edit since
    /// the last run included.
    limits: crate::settings::History,
    /// Whether the history could not be written: the run goes on, and
    /// cannot be undone.
    lost: bool,
    /// The song list as the run last read or wrote it.
    seen: Seen,
    /// Whether the run saved the song list.
    saved: bool,
    /// An undo stopped in its sync, which this run's sync finishes.
    finishes: Option<PathBuf>,
    /// Whether the run's sync reached its end.
    synced: bool,
    /// The outputs a run stopped partway made, which this run continues:
    /// none was there before it, so none is kept to put back.
    made_before: BTreeSet<PathBuf>,
    /// The song list as the run found it, a hand edit since the last run
    /// included.
    found: Option<String>,
    /// The library the run kept files of, once it kept one.
    library: Option<PathBuf>,
    /// Whether the run began its own record, not going on in a stopped
    /// run's.
    fresh: bool,
    /// Held while the run lasts, so no other begins meanwhile.
    _runs: atomic::Lock,
}

/// The run of a command that asks before it changes the song list,
/// begun only once a question is answered, as it saves: no other run
/// waits on an answer, and a change declined begins none. The song list
/// as the command found it is kept from the start, so the run still knows
/// what it was before where no earlier run says. What the command read is
/// checked again as it saves, a song changed meanwhile refusing the save.
///
/// A command that asks again after a save, as `review` asks about the
/// next song, lets go of its run first ([`Later::let_go`]); its record
/// stays as the save left it, as a stopped run's, and the next save goes
/// on in it, so one `undo` takes back every answer.
#[derive(Debug)]
pub struct Later {
    home: PathBuf,
    found: Option<String>,
    run: Option<Run>,
    /// Whether a run let go of had saved, its record left to go on in.
    left: bool,
}

impl Later {
    /// A run to begin later, the song list read whole now.
    pub fn new(home: &Path) -> Result<Self> {
        let found = {
            let _lock = atomic::Lock::folder(home)?;
            songs_text(home)?
        };
        Ok(Self {
            home: home.to_path_buf(),
            found,
            run: None,
            left: false,
        })
    }

    /// The run, begun now if it was not, going on in the one let go of:
    /// called just before the command saves, which it is passed to, so a
    /// run under way is waited for while nothing is changed yet.
    pub fn begin(&mut self) -> Result<&mut Run> {
        match &mut self.run {
            Some(run) => Ok(run),
            none => Ok(none.insert(Run::begin_from(&self.home, self.found.as_deref())?)),
        }
    }

    /// Let go of the run before asking again, so it holds no other run
    /// back meanwhile. One that saved nothing, as for a save refused, goes
    /// with the record it wrote ahead; one that saved keeps its record as
    /// it stands, which the next [`Self::begin`] goes on in, as any run
    /// goes on in one stopped partway. A run begun elsewhere meanwhile
    /// goes on in it first and ends it: what was saved is then that run's,
    /// undone with it, and the next save here begins anew.
    pub fn let_go(&mut self) {
        if let Some(run) = self.run.take() {
            self.left |= run.saved;
            run.discard();
        }
    }

    /// The run, if one began: the one let go of gone on in, so the command
    /// ends it, its change synced.
    pub fn begun(mut self) -> Result<Option<Run>> {
        if self.run.is_none() && self.left {
            self.begin()?;
        }
        Ok(self.run)
    }
}

/// The song list as a run last read or wrote it, `None` for a home
/// without one, once it has.
#[derive(Debug, Clone, Default)]
enum Seen {
    #[default]
    Not,
    As(Option<String>),
}

fn songs_text(home: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(home.join(MANIFEST)) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("reading the song list"),
    }
}

fn history(home: &Path) -> PathBuf {
    home.join(crate::dirs::HISTORY)
}

/// Whether the history keeps any run of `home`.
pub(crate) fn kept_any(home: &Path) -> bool {
    runs(home).is_ok_and(|r| !r.is_empty())
}

/// The runs kept, oldest first.
fn runs(home: &Path) -> Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = match std::fs::read_dir(history(home)) {
        Ok(entries) => entries
            .filter_map(|e| Some(e.ok()?.path()))
            .filter(|p| p.join(RECORD).is_file())
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).context("reading the run history"),
    };
    found.sort();
    Ok(found)
}

/// Folders in the history with no record: runs stopped before writing
/// one.
fn strays(home: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(history(home))
        .into_iter()
        .flatten()
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.is_dir() && !p.join(RECORD).is_file())
        .collect()
}

/// What a run's record says of itself, read without its outputs, which
/// run to megabytes on a large library.
#[derive(Debug, Default, Deserialize)]
struct Head {
    #[serde(default = "complete")]
    complete: bool,
    #[serde(default)]
    undoing: bool,
    #[serde(default)]
    undone: bool,
    #[serde(default)]
    songs_before: Option<String>,
    #[serde(default)]
    songs_after: Option<String>,
    #[serde(default)]
    bytes: u64,
}

impl Head {
    /// The song list the run left, when it says: as it ended, or as an
    /// undo of it put it back.
    fn left(self) -> Option<String> {
        if self.undone {
            Some(
                self.songs_before
                    .unwrap_or_else(|| crate::manifest::NEW.to_string()),
            )
        } else if self.complete && !self.undoing {
            self.songs_after
        } else {
            None
        }
    }
}

/// The runs kept in `home` but `mine`, oldest first, with the bytes each
/// keeps.
fn others(home: &Path, mine: Option<&PathBuf>) -> Vec<(PathBuf, u64)> {
    (runs(home).unwrap_or_default().into_iter())
        .filter(|dir| Some(dir) != mine)
        .map(|dir| {
            let bytes = held(&dir);
            (dir, bytes)
        })
        .collect()
}

fn head(dir: &Path) -> Option<Head> {
    serde_json::from_slice(&std::fs::read(dir.join(RECORD)).ok()?).ok()
}

/// The bytes a run's record says it keeps; for a record that does not
/// say, what its files take.
fn held(dir: &Path) -> u64 {
    let said = head(dir).map_or(0, |h| h.bytes);
    if said > 0 {
        return said;
    }
    [dir.join("files"), dir.join(FREED)]
        .iter()
        .flat_map(|d| crate::store::walk(d, usize::MAX).unwrap_or_default())
        .filter_map(|f| std::fs::metadata(f).ok())
        .map(|m| m.len())
        .sum()
}

/// What the stopped run `record` holds made of each output it recorded,
/// by the outputs as `home`'s state holds them now: the output, if new,
/// and each file beside it its song lists now but did not then. A run
/// going on with it keeps none of them, as it would keep a file it found.
/// A file the stopped run moved, or was about to, is not one it made: it
/// holds what was there before the run, where it went or where it still
/// is, which undoing it moves back, so a run going on keeps it before
/// writing over or removing it.
fn made_by(record: &Record, home: &Path) -> BTreeSet<PathBuf> {
    if record.before.is_empty() {
        return BTreeSet::new();
    }
    let moved: BTreeSet<&PathBuf> = (record.moves.iter())
        .flat_map(|m| [&m.from, &m.to])
        .collect();
    let now = State::load(home).map(|s| s.outputs).unwrap_or_default();
    let files = |rel: &Path, w: Option<&Written>| -> BTreeSet<PathBuf> {
        w.map(|w| w.files(rel).map(Path::to_path_buf).collect())
            .unwrap_or_default()
    };
    (record.before.iter())
        .flat_map(|(rel, was)| {
            let then = files(rel, was.as_ref());
            let mut made = files(rel, now.get(rel));
            if was.is_none() {
                made.insert(rel.clone());
            }
            made.retain(|f| !then.contains(f) && !moved.contains(f));
            made
        })
        .collect()
}

impl Run {
    /// Begin recording, with the song list as it is before anything.
    pub fn begin(home: &Path) -> Result<Self> {
        Self::begin_from(home, None)
    }

    /// [`Self::begin`] for a command that read the song list as `found`
    /// before it changed it: its song list before, where no run kept says.
    pub fn begin_from(home: &Path, found: Option<&str>) -> Result<Self> {
        // A clock coarser than a nanosecond can repeat a reading; the
        // count keeps two runs of one process apart regardless.
        static STARTED: AtomicU32 = AtomicU32::new(0);
        // Before anything is read: a run waiting on another begins with
        // the song list that one leaves.
        let runs_lock = atomic::Lock::runs(home)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let id = format!(
            "{:012}-{:09}-{}-{}",
            now.as_secs(),
            now.subsec_nanos(),
            std::process::id(),
            STARTED.fetch_add(1, Ordering::Relaxed)
        );
        // As the last run left it, when one is kept: a hand edit since is
        // part of what this run applies, and undone with it.
        let kept = runs(home).unwrap_or_default();
        for dir in kept.iter().chain(&strays(home)) {
            atomic::clear_parts(dir);
        }
        let last = Some(kept)
            .and_then(|mut r| r.pop())
            .and_then(|dir| Some((head(&dir)?, dir)));
        let finishes = last
            .as_ref()
            .filter(|(h, _)| h.undone)
            .map(|(_, dir)| dir.clone());
        // As they stand, for a run that ends before it reaches the
        // library: an empty record would read as every song added.
        let outputs = State::load(home).map(|s| s.outputs).unwrap_or_default();
        // A run stopped partway, as by a kill, is no run's but this one's
        // to finish, the runs lock held: it goes on in its record, so one
        // undo takes back both, its song list and its files with them.
        let resumed = last
            .as_ref()
            .filter(|(h, _)| !h.complete && !h.undoing && !h.undone)
            .and_then(|(_, dir)| {
                let text = std::fs::read(dir.join(RECORD)).ok()?;
                Some((dir.clone(), Record::read(&text, &outputs).ok()?))
            });
        let now = {
            // Read whole, not halfway through another run's save.
            let _lock = atomic::Lock::folder(home)?;
            songs_text(home)
        };
        let left = match &resumed {
            Some((_, record)) => Some(record.songs_before.clone()),
            None => last.and_then(|(h, _)| h.left()).map(Some),
        };
        let (songs_before, now) = match (left, now) {
            (Some(left), now) => (left, now.ok().flatten()),
            (None, now) => {
                let now = now?;
                (found.map(str::to_string).or_else(|| now.clone()), now)
            }
        };
        // A song list that does not read keeps the default history; the
        // run itself reports why it does not read.
        let limits = now
            .as_deref()
            .and_then(|t| t.parse().ok())
            .and_then(|doc| crate::settings::read(&doc).ok())
            .unwrap_or_default()
            .history;
        let found = now.clone();
        let mine = resumed.as_ref().map(|(dir, _)| dir.clone());
        let others = others(home, mine.as_ref());
        let (dir, record, outputs_known) = match resumed {
            Some((dir, mut record)) => {
                record.version = VERSION;
                record.songs_before = songs_before;
                (dir, record, true)
            }
            None => (
                history(home).join(id),
                Record {
                    version: VERSION,
                    complete: false,
                    songs_before,
                    was: outputs,
                    ..Record::default()
                },
                false,
            ),
        };
        let changing: BTreeSet<PathBuf> = record.before.keys().cloned().collect();
        let made_before = made_by(&record, home);
        let mut run = Self {
            home: home.to_path_buf(),
            dir,
            record,
            outputs_known,
            changing,
            others,
            limits,
            lost: false,
            seen: Seen::Not,
            saved: false,
            finishes,
            synced: false,
            made_before,
            found,
            library: None,
            fresh: mine.is_none(),
            _runs: runs_lock,
        };
        // Ahead of all it changes, so stopped anywhere it is undone.
        run.checkpoint(home)?;
        Ok(run)
    }

    /// Let go of the run unfinished: the record it wrote ahead goes if it
    /// saved nothing, unless it went on in a stopped run's; else it stays
    /// as it stands, for a run to go on in.
    // Removes the run's own folder, written ahead and never used.
    #[expect(clippy::disallowed_methods)]
    fn discard(self) {
        if self.fresh && !self.saved {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Note the song list as the run read or wrote it: the list the run
    /// leaves, so a hand edit made while it ran, which it never read, is
    /// the next run's.
    pub fn saw(&mut self, manifest: &crate::manifest::Manifest) {
        self.seen = Seen::As(manifest.text().map(str::to_string));
    }

    /// Note that the run is about to follow the user's files `moved`, by
    /// the key each moved to, in the song list: `undo` puts the song list
    /// back with them followed, as the state and the files stay.
    pub fn followed(&mut self, moved: &BTreeMap<SourceKey, SourceKey>) {
        self.record.followed = then(&self.record.followed, moved);
    }

    /// Note that the run's sync reached its end, the library in line with
    /// the song list: an undo stopped in its own sync is finished.
    pub fn synced(&mut self) {
        self.synced = true;
    }

    /// The song list the run leaves: as it last saw it, else as it is.
    fn songs_left(&self, home: &Path) -> Result<Option<String>> {
        match &self.seen {
            Seen::As(text) => Ok(text.clone()),
            Seen::Not => songs_text(home),
        }
    }

    /// The outputs as they were before the run wrote or removed any.
    pub fn outputs_before(&mut self, outputs: &BTreeMap<PathBuf, Written>) {
        if !self.outputs_known {
            self.record.was.clone_from(outputs);
            self.outputs_known = true;
        }
    }

    /// Note that the run is about to change the output at `rel`, before
    /// the state that changes it is saved.
    pub fn changing(&mut self, rel: &Path) {
        self.changing.insert(rel.to_path_buf());
    }

    /// Note each output `outputs`, about to be saved, holds otherwise than
    /// the run began with, and write the record ahead when one was not
    /// noted yet.
    pub fn outputs_after(
        &mut self,
        home: &Path,
        outputs: &BTreeMap<PathBuf, Written>,
    ) -> Result<()> {
        let was = &self.record.was;
        let unnoted: Vec<PathBuf> = was
            .keys()
            .chain(outputs.keys())
            .filter(|rel| was.get(*rel) != outputs.get(*rel) && !self.changing.contains(*rel))
            .cloned()
            .collect();
        if unnoted.is_empty() {
            return Ok(());
        }
        self.changing.extend(unnoted);
        self.checkpoint(home)
    }

    /// Keep a library file the run is about to replace or remove: linked
    /// where the filesystem allows, else copied while the budget lasts.
    pub fn keep(&mut self, library: &Path, rel: &Path) -> Result<()> {
        if self.lost {
            return Ok(());
        }
        if let Err(e) = self.try_keep(library, rel, false) {
            self.lose(&e);
        }
        Ok(())
    }

    /// Keep a library file no song keeps that the run is about to remove
    /// before its moves, to free its path for one, as [`Run::keep`] keeps
    /// a file but apart: a move may land where it was, and a file written
    /// over there after be kept too.
    pub fn keep_freed(&mut self, library: &Path, rel: &Path) -> Result<()> {
        if self.lost {
            return Ok(());
        }
        if let Err(e) = self.try_keep(library, rel, true) {
            self.lose(&e);
        }
        Ok(())
    }

    /// Say once that the history cannot be written, and write no more of it.
    fn lose(&mut self, e: &anyhow::Error) {
        if !self.lost {
            let _ = crate::ui::warning(
                &mut anstream::stderr(),
                &format!("The run history cannot be written, so this run cannot be undone: {e:#}"),
            );
        }
        self.lost = true;
    }

    // Keeps a file for undo before the run changes it.
    #[expect(clippy::disallowed_methods)]
    fn try_keep(&mut self, library: &Path, rel: &Path, freeing: bool) -> Result<()> {
        let from = library.join(rel);
        let Ok(meta) = std::fs::metadata(&from) else {
            return Ok(());
        };
        self.changing.insert(rel.to_path_buf());
        if self.made_before.contains(rel) {
            return Ok(());
        }
        let first = if freeing {
            self.record.freed.insert(rel.to_path_buf())
        } else {
            self.record.touched.insert(rel.to_path_buf())
        };
        if !first {
            return Ok(());
        }
        let budget = self.limits.max_size.0;
        // A file past the whole budget is written again instead, and costs
        // no other run its record.
        if meta.len() > budget {
            return Ok(());
        }
        make_room(&mut self.others, self.record.bytes + meta.len(), budget)?;
        if self.record.bytes + meta.len() > budget {
            return Ok(());
        }
        let to = self
            .dir
            .join(if freeing { FREED } else { "files" })
            .join(rel);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        self.library = Some(library.to_path_buf());
        // A stopped run this one goes on with may have kept it already,
        // its record not yet saying so; a copy onto that link would empty
        // the library's file too.
        match std::fs::remove_file(&to) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Ok(()),
            _ => {}
        }
        let kept = std::fs::hard_link(&from, &to)
            .or_else(|_| std::fs::copy(&from, &to).map(drop))
            .is_ok();
        if kept && freeing {
            self.record.bytes += meta.len();
        } else if kept {
            self.record.bytes += meta.len();
            self.record.kept.insert(rel.to_path_buf());
            // A copy with new times, its bytes muman's, is no change.
            let outputs = &self.record.was;
            let changed = crate::reconcile::changed_since_written(library, outputs, rel)
                && outputs.get(rel).is_some_and(|w| {
                    w.digest.is_none() || crate::hash::digest_file(&from) != w.digest
                });
            if changed {
                self.record.changed.insert(rel.to_path_buf());
            }
        }
        Ok(())
    }

    /// Let go of each file kept that the run never replaced: a link to
    /// the library's file still, which a tagger writing in place would
    /// change with it, and which `undo` has no need of.
    // Lets go of a link the run made in its own folder.
    #[expect(clippy::disallowed_methods)]
    fn let_go_of_unreplaced(&mut self) {
        let Some(library) = self.library.clone() else {
            return;
        };
        let unreplaced: Vec<PathBuf> = (self.record.kept.iter())
            .filter(|rel| {
                let kept = self.dir.join("files").join(rel);
                same_file::is_same_file(library.join(rel), &kept).unwrap_or(false)
            })
            .cloned()
            .collect();
        for rel in unreplaced {
            let kept = self.dir.join("files").join(&rel);
            let bytes = std::fs::metadata(&kept).map_or(0, |m| m.len());
            if std::fs::remove_file(&kept).is_ok() {
                self.record.bytes = self.record.bytes.saturating_sub(bytes);
                self.record.kept.remove(&rel);
                self.record.touched.remove(&rel);
                self.record.changed.remove(&rel);
            }
        }
    }

    /// Forget the moves of a stopped run this one goes on in that it never
    /// made, as [`follow_moves`] finds them in `library`: `undo` would
    /// name each where nothing went, and this run records again those it
    /// makes.
    pub fn forget_unmade(&mut self, library: &Path) {
        if self.fresh {
            return;
        }
        let made = (self.record.moves.iter())
            .take_while(|m| library.join(&m.to).exists())
            .count();
        self.record.moves.truncate(made);
    }

    /// Record that the run is moving a library file, before it does.
    pub fn moving(&mut self, from: &Path, to: &Path) {
        self.changing.extend([from.to_path_buf(), to.to_path_buf()]);
        self.record.moves.push(Moved {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        });
    }

    /// Record that the run moves `file`, under `home`, to the desktop's
    /// trash, which muman cannot take it back from on every platform;
    /// [`Run::checkpoint`] before the move, so a run stopped after it
    /// still asks for it back.
    pub fn trashed(&mut self, home: &Path, file: &Path) {
        if let Ok(rel) = file.strip_prefix(home) {
            self.record.trashed.insert(rel.to_path_buf());
        }
    }

    /// Record that the run copied `file`, under `home`, into the
    /// sources, so `undo` takes the copy out with the song.
    pub fn copied(&mut self, home: &Path, file: &Path) {
        if let (Ok(rel), Some(digest)) = (file.strip_prefix(home), crate::hash::digest_file(file)) {
            self.record.copied.insert(rel.to_path_buf(), digest);
        }
    }

    /// Record that the run is about to copy into `file`, a name no file
    /// holds, so a run stopped before [`Run::copied`] notes it still
    /// takes the copy out: [`Run::checkpoint`] before the copy.
    pub fn copying(&mut self, home: &Path, file: &Path) {
        if let Ok(rel) = file.strip_prefix(home) {
            self.record.copied.insert(rel.to_path_buf(), String::new());
        }
    }

    /// Forget a copy the run took out again itself.
    pub fn not_copied(&mut self, home: &Path, file: &Path) {
        if let Ok(rel) = file.strip_prefix(home) {
            self.record.copied.remove(rel);
        }
    }

    /// Forget a move the run could not make.
    pub fn not_moved(&mut self, from: &Path, to: &Path) {
        self.record
            .moves
            .retain(|m| !(m.from == from && m.to == to));
    }

    /// Write the record as it stands, ahead of the library changes it
    /// lists, so a run stopped partway can still be undone.
    pub fn checkpoint(&mut self, home: &Path) -> Result<()> {
        self.record.songs_after = self.songs_left(home)?;
        if !self.lost
            && let Err(e) = self.write()
        {
            self.lose(&e);
        }
        Ok(())
    }

    fn write(&mut self) -> Result<()> {
        self.record.before = self
            .changing
            .iter()
            .map(|rel| (rel.clone(), self.record.was.get(rel).cloned()))
            .collect();
        if let Some(history) = self.dir.parent() {
            atomic::private_dir(history)?;
        }
        atomic::private_dir(&self.dir)?;
        let text = serde_json::to_vec(&self.record).context("writing the run record")?;
        atomic::write_private(&self.dir, &atomic::Name::new(RECORD)?, &text)
    }

    /// Write the record when the run changed the song list or the
    /// library, and let go of runs beyond the `[history]` count and of
    /// folders without a record. The record keeps each output the state
    /// now holds otherwise than the run began with, whatever noted it. A
    /// run that only wrote new songs still changed the library; one that
    /// left the same outputs, if newly vouched for, did not.
    // Lets go of a run's folder once nothing needs it.
    #[expect(clippy::disallowed_methods)]
    pub fn finish(mut self, home: &Path) -> Result<()> {
        let _lock = atomic::Lock::folder(home)?;
        if let Some(undone) = self.finishes.as_ref().filter(|_| self.synced) {
            let _ = std::fs::remove_dir_all(undone);
        }
        self.record.songs_after = self.songs_left(home)?;
        self.record.complete = true;
        if self.lost {
            return Ok(());
        }
        self.let_go_of_unreplaced();
        let mut same_outputs = false;
        if let Ok(now) = State::load(home) {
            let was = &self.record.was;
            same_outputs = was.keys().eq(now.outputs.keys());
            let changed: Vec<PathBuf> = was
                .keys()
                .chain(now.outputs.keys())
                .filter(|rel| was.get(*rel) != now.outputs.get(*rel))
                .cloned()
                .collect();
            self.changing.extend(changed);
        }
        // A run that never read the song list, as one stopped on a list that
        // does not read, and left it as it found it applied no hand edit:
        // that is the next run's.
        let applied = matches!(self.seen, Seen::As(_)) || self.record.songs_after != self.found;
        if self.record.touched.is_empty()
            && self.record.freed.is_empty()
            && self.record.moves.is_empty()
            && self.record.copied.is_empty()
            && (self.record.songs_after == self.record.songs_before || !applied)
            && same_outputs
        {
            let _ = std::fs::remove_dir_all(&self.dir);
            return Ok(());
        }
        if let Err(e) = self.write() {
            self.lose(&e);
            return Ok(());
        }
        for stray in strays(home) {
            let _ = std::fs::remove_dir_all(stray);
        }
        let all = runs(home).unwrap_or_default();
        let past = all.len().saturating_sub(self.limits.runs);
        for old in all.iter().take(past) {
            let _ = std::fs::remove_dir_all(old);
        }
        let mut others: Vec<(PathBuf, u64)> = all
            .into_iter()
            .skip(past)
            .filter(|dir| *dir != self.dir)
            .map(|dir| {
                let bytes = held(&dir);
                (dir, bytes)
            })
            .collect();
        let _ = make_room(&mut others, self.record.bytes, self.limits.max_size.0);
        Ok(())
    }
}

/// A run's save of the song list, begun and written ahead before it: the
/// list noted as saved, and the record written again, so it never lags
/// the list the run wrote.
impl crate::manifest::Recorder for Run {
    fn saved(&mut self, manifest: &crate::manifest::Manifest) -> Result<()> {
        self.saw(manifest);
        self.saved = true;
        let home = self.home.clone();
        self.checkpoint(&home)
    }
}

/// What a sync records its song-list saves in.
#[derive(Debug)]
pub enum Recording<'a> {
    /// The run the sync is part of.
    Run(&'a mut Run),
    /// The undo whose sync it is, under the runs lock the undo holds.
    Undo(&'a mut Undone),
    /// None, for a dry run, which saves nothing: a save is refused.
    DryRun,
    /// None, for a test's sync.
    #[cfg(any(test, feature = "testing"))]
    Test,
}

impl Recording<'_> {
    /// The run, when the sync is part of one.
    pub fn run(&mut self) -> Option<&mut Run> {
        match self {
            Self::Run(run) => Some(run),
            _ => None,
        }
    }

    /// The same, borrowed for a step of the sync.
    pub fn reborrow(&mut self) -> Recording<'_> {
        match self {
            Self::Run(run) => Recording::Run(run),
            Self::Undo(undone) => Recording::Undo(undone),
            Self::DryRun => Self::DryRun,
            #[cfg(any(test, feature = "testing"))]
            Self::Test => Self::Test,
        }
    }
}

impl crate::manifest::Recorder for Recording<'_> {
    fn ahead(&self) -> Result<()> {
        match self {
            Self::DryRun => anyhow::bail!("a dry run saves no song list"),
            _ => Ok(()),
        }
    }

    fn saved(&mut self, manifest: &crate::manifest::Manifest) -> Result<()> {
        match self {
            Self::Run(run) => run.saved(manifest),
            Self::Undo(undone) => undone.saved(manifest),
            Self::DryRun => Ok(()),
            #[cfg(any(test, feature = "testing"))]
            Self::Test => Ok(()),
        }
    }
}

/// Let go of the files the oldest of `others` keep, oldest first, until
/// they take no more than `budget` with the `mine` bytes of the run
/// recording; each entry's bytes become none as its files go.
fn make_room(others: &mut [(PathBuf, u64)], mine: u64, budget: u64) -> Result<()> {
    let mut total = mine + others.iter().map(|(_, b)| b).sum::<u64>();
    for (dir, bytes) in others.iter_mut() {
        if total <= budget {
            break;
        }
        if *bytes > 0 {
            let_go(dir)?;
            total -= *bytes;
            *bytes = 0;
        }
    }
    Ok(())
}

/// Let go of the files a run keeps and keep its record, which then lists
/// none kept, so `undo` has them written again from their sources; a run
/// whose record does not read goes whole.
// Lets go of a run's folder.
#[expect(clippy::disallowed_methods)]
fn let_go(dir: &Path) -> Result<()> {
    let record = std::fs::read(dir.join(RECORD))
        .ok()
        .and_then(|t| serde_json::from_slice::<Record>(&t).ok());
    let gone = match record {
        Some(mut record) => {
            record.kept.clear();
            record.bytes = 0;
            let text = serde_json::to_vec(&record).context("writing the run record")?;
            atomic::write_private(dir, &atomic::Name::new(RECORD)?, &text)?;
            vec![dir.join("files"), dir.join(FREED)]
        }
        None => vec![dir.to_path_buf()],
    };
    for gone in gone {
        match std::fs::remove_dir_all(&gone) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(e).with_context(|| format!("removing {}", gone.display()));
            }
            _ => {}
        }
    }
    Ok(())
}

/// The moves `first` and then `second` make, as one set of moves made at
/// once: each key to where `second` takes the key `first` took it to.
fn then(
    first: &BTreeMap<SourceKey, SourceKey>,
    second: &BTreeMap<SourceKey, SourceKey>,
) -> BTreeMap<SourceKey, SourceKey> {
    let mut both: BTreeMap<SourceKey, SourceKey> = (first.iter())
        .map(|(from, to)| (from.clone(), second.get(to).unwrap_or(to).clone()))
        .collect();
    for (from, to) in second {
        // A file `first` took to `from` is followed above; one at a key it
        // took a file from, and none to, arrived since, named by no key.
        if !first.contains_key(from) && !first.values().any(|t| t == from) {
            both.insert(from.clone(), to.clone());
        }
    }
    both.retain(|from, to| from != to);
    both
}

impl Record {
    /// The song list undoing the run puts back: the one before it, a new
    /// home's for none, each key it followed a file of the user's from
    /// written as the key it followed it to, as the state and the file
    /// stay where the run left them.
    fn put_back(&self) -> String {
        let before = self.songs_before.as_deref().unwrap_or(crate::manifest::NEW);
        crate::manifest::moved_in(before, &self.followed)
    }

    /// The record `text` holds, its outputs as the run began made whole
    /// from `now`, the outputs as they are, where it keeps only those the
    /// run changed.
    fn read(text: &[u8], now: &BTreeMap<PathBuf, Written>) -> serde_json::Result<Self> {
        let mut record: Self = serde_json::from_slice(text)?;
        record.was = if record.version < 2 {
            record.outputs.clone()
        } else {
            let mut was = now.clone();
            for (rel, before) in &record.before {
                match before {
                    Some(w) => was.insert(rel.clone(), w.clone()),
                    None => was.remove(rel),
                };
            }
            was
        };
        Ok(record)
    }
}

/// Whether the song list is as the last run that ended left it; `None`
/// when no such run is kept to tell.
#[must_use]
pub fn songs_as_last_left(home: &Path) -> Option<bool> {
    let after = runs(home).ok()?.pop().and_then(|dir| head(&dir))?.left()?;
    Some(songs_text(home).ok()?.as_deref() == Some(after.as_str()))
}

/// Whether the latest run kept is an undo stopped partway, which the
/// next `undo` finishes.
#[must_use]
pub fn undo_stopped(home: &Path) -> bool {
    runs(home)
        .ok()
        .and_then(|mut r| r.pop())
        .and_then(|dir| head(&dir))
        .is_some_and(|h| h.undoing)
}

/// Whether the latest run kept is an undo stopped before it put the song
/// list and the state back: only `undo` finishes it, as a sync would
/// leave what it put back beside what it had not yet removed. One
/// stopped later, in its sync, the next run's sync finishes.
#[must_use]
pub fn undo_unfinished(home: &Path) -> bool {
    runs(home)
        .ok()
        .and_then(|mut r| r.pop())
        .and_then(|dir| head(&dir))
        .is_some_and(|h| h.undoing && !h.undone)
}

/// Whether [`follow_moves`] may move a file in the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moves {
    Follow,
    Foresee,
}

/// The outputs a run stopped partway left where they were: each file it
/// moved before the state could say so follows to where it went, with
/// its lyrics if they went too, so the next run knows it as muman's.
/// Returns how many followed.
///
/// The run made its moves in the order the record lists them, so those
/// made are the ones before the first whose file is not where it went,
/// a move of case alone stopped between its two renames finished, or
/// taken as made under [`Moves::Foresee`];
/// one into a path an earlier move left is a chain, as a song moving into
/// another's old place. They are followed all at once, from the outputs
/// as the run began, and only when the state still says so: the paths
/// they left that no move went into are still listed.
///
/// A file beside a song that stayed where it was moves after the song
/// under [`Moves::Follow`], but a folder's file the record lists no move
/// of, which the run left in its folder ([`crate::folder`]); under
/// [`Moves::Foresee`], as a dry run
/// reads, it stays and is recorded where it is, so nothing in the
/// library changes.
// Moves the files beside a song a stopped run left behind it.
#[expect(clippy::disallowed_methods)]
pub fn follow_moves(home: &Path, library: &Path, state: &mut State, moves: Moves) -> usize {
    let Some(dir) = runs(home).ok().and_then(|mut r| r.pop()) else {
        return 0;
    };
    let Some(record) = std::fs::read(dir.join(RECORD))
        .ok()
        .and_then(|t| Record::read(&t, &state.outputs).ok())
    else {
        return 0;
    };
    if record.complete || record.undoing {
        return 0;
    }
    let made: Vec<&Moved> = record
        .moves
        .iter()
        .take_while(|m| made(library, m, moves))
        .collect();
    // A song's move, as the outputs stood when the run began; the rest
    // are its lyrics'.
    let songs: Vec<&Moved> = made
        .iter()
        .copied()
        .filter(|m| record.was.contains_key(&m.from))
        .collect();
    let into: BTreeSet<&PathBuf> = songs.iter().map(|m| &m.to).collect();
    let left: Vec<&PathBuf> = songs
        .iter()
        .map(|m| &m.from)
        .filter(|from| !into.contains(from))
        .collect();
    if left.is_empty() || !left.iter().all(|from| state.outputs.contains_key(*from)) {
        return 0;
    }
    let went: BTreeMap<&PathBuf, &PathBuf> = made.iter().map(|m| (&m.from, &m.to)).collect();
    let listed = |rel: &Path| record.moves.iter().any(|m| m.from == rel);
    let before = state.outputs.clone();
    let mut followed = BTreeMap::new();
    for m in &songs {
        let Some(mut written) = before.get(&m.from).cloned() else {
            continue;
        };
        for sidecar in &mut written.sidecars {
            if let Some(to) = went.get(&sidecar.path) {
                sidecar.path = (*to).clone();
            } else if moves == Moves::Follow
                && library.join(&sidecar.path).exists()
                && (!crate::outputs::of_folder(&m.from, &sidecar.path) || listed(&sidecar.path))
            {
                // Stopped between the audio and the files beside it: they follow now.
                let to = crate::outputs::moved(&m.from, &m.to, &sidecar.path);
                if !library.join(&to).exists()
                    && atomic::rename(&library.join(&sidecar.path), &library.join(&to)).is_ok()
                {
                    sidecar.path = to;
                }
            }
        }
        followed.insert(m.to.clone(), written);
    }
    for m in &songs {
        state.outputs.remove(&m.from);
    }
    let n = followed.len();
    state.outputs.extend(followed);
    n
}

/// Whether the move `m` a stopped run records was made in `library`: its
/// file where it went, or, stopped between the two renames of a move of
/// case alone, at the name between them, which the move is finished from
/// under [`Moves::Follow`].
fn made(library: &Path, m: &Moved, moves: Moves) -> bool {
    let half = match moves {
        Moves::Follow => crate::library::finish_move(library, &m.from, &m.to),
        Moves::Foresee => crate::library::half_moved(library, &m.from, &m.to),
    };
    half || library.join(&m.to).exists()
}

/// The paths the latest run, when it was stopped partway, records moving
/// files to: a `.moving` beside one is its own
/// ([`crate::library::Leftovers`]).
#[must_use]
pub fn moving(home: &Path) -> Vec<PathBuf> {
    let Some(dir) = runs(home).ok().and_then(|mut r| r.pop()) else {
        return Vec::new();
    };
    let Some(record) = std::fs::read(dir.join(RECORD))
        .ok()
        .and_then(|t| serde_json::from_slice::<Record>(&t).ok())
    else {
        return Vec::new();
    };
    if record.complete {
        return Vec::new();
    }
    record.moves.into_iter().map(|m| m.to).collect()
}

/// Undoing the latest run, as planned before asking: which run, and
/// what it would do in words.
#[derive(Debug)]
pub struct UndoPlan {
    dir: PathBuf,
    record: Record,
    lines: Vec<String>,
}

impl UndoPlan {
    /// What undoing would do, a line each.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The song list undoing puts back; `None` when the home had none,
    /// and a new home's is put back.
    #[must_use]
    pub fn songs_before(&self) -> Option<&str> {
        self.record.songs_before.as_deref()
    }

    /// Whether `other` undoes the same run in the same way: its record
    /// and what it would do, not only its folder.
    #[must_use]
    pub fn same_run(&self, other: &Self) -> bool {
        let record = |p: &Self| serde_json::to_vec(&p.record).ok();
        self.dir == other.dir && self.lines == other.lines && record(self) == record(other)
    }
}

/// Refuse until every file of the user's the run trashed is back: its
/// song would be listed again with no file to make it from. One put back
/// elsewhere in the manual folder, and followed there by a run since, is
/// back where the list put back names it ([`Record::put_back`]).
fn all_trashed_back(dirs: &Dirs, record: &Record) -> Result<()> {
    let home = &dirs.home;
    let followed = |rel: &Path| {
        let manual = dirs.manual();
        let from = home.join(rel);
        let key = SourceKey::Manual(from.strip_prefix(&manual).ok()?.into());
        let SourceKey::Manual(to) = record.followed.get(&key)? else {
            return None;
        };
        crate::relpath::on_disk(&manual, to.path())
    };
    let gone: Vec<String> = record
        .trashed
        .iter()
        .filter(|rel| crate::relpath::on_disk(home, rel).is_none() && followed(rel).is_none())
        .map(|rel| format!("  {}", home.join(rel).display()))
        .collect();
    if !gone.is_empty() {
        return Err(crate::errors::Refused(format!(
            "The last run moved these to the trash; put them back there, then undo \
             again:\n{}",
            gone.join("\n")
        ))
        .into());
    }
    Ok(())
}

/// What undoing the latest run would do, refused as `undo` itself would
/// be: when the song list changed since, or the run wrote another
/// library folder.
/// Refuse to undo `record` in another library than the one it wrote. A
/// run stopped before its sync, as an add after its copies, wrote none.
fn refuse_another_library(record: &Record, state: &State, dirs: &Dirs) -> Result<()> {
    let wrote_library =
        !(record.touched.is_empty() && record.moves.is_empty() && record.before.is_empty());
    // A home's first run stopped before the state named its library wrote
    // none other than the one the state has yet to name.
    let named = match state.library.as_deref() {
        Some(l) => crate::platform::same_path(l, &dirs.library),
        None => !record.complete,
    };
    if wrote_library && !named {
        return Err(
            crate::errors::Refused("The last run wrote another library folder".into()).into(),
        );
    }
    Ok(())
}

/// Refuse to undo `record`'s run where it would write over a file not
/// as the run left it, or move a song back onto a file in its way.
fn refuse_writing_over(record: &Record, state: &State, dirs: &Dirs) -> Result<()> {
    // What undoing would write over must be muman's as the run left it,
    // as the state stamps it: a file a stopped run was writing it stamps
    // not, and an undo resumed put some files back already.
    if !record.undoing {
        let mut state = state.clone();
        crate::reconcile::vouch_for_copies(&dirs.library, &mut state);
        let beside: BTreeMap<&PathBuf, &crate::state::Sidecar> = state
            .outputs
            .values()
            .flat_map(|w| &w.sidecars)
            .map(|s| (&s.path, s))
            .collect();
        for rel in record.touched.iter().chain(&record.freed) {
            if !dirs.library.join(rel).exists() {
                continue;
            }
            let changed = "changed since muman wrote it";
            let why = if state.outputs.contains_key(rel) {
                crate::reconcile::edited_since_written(&dirs.library, &state.outputs, rel)
                    .then_some(changed)
            } else if !record.complete {
                None
            } else if let Some(s) = beside.get(rel) {
                (!crate::reconcile::sidecar_as_written(&dirs.library, s)).then_some(changed)
            } else {
                Some("is not muman's")
            };
            let Some(why) = why else {
                continue;
            };
            return Err(crate::errors::Refused(format!(
                "{} {why}; undoing would write over it. Move it aside first",
                crate::relpath::show(rel)
            ))
            .into());
        }
        // Each move goes back, last first, to where it came from, which a
        // move after it freed, or a file the run kept puts back; any other
        // file there would be moved over.
        for (n, m) in record.moves.iter().enumerate() {
            let freed = record.moves[n + 1..].iter().any(|later| later.to == m.from);
            if dirs.library.join(&m.from).exists()
                && dirs.library.join(&m.to).exists()
                && !freed
                && !record.kept.contains(&m.from)
            {
                return Err(crate::errors::Refused(format!(
                    "{} is taken; undoing would move {} back onto it. Move it aside first",
                    crate::relpath::show(&m.from),
                    crate::relpath::show(&m.to)
                ))
                .into());
            }
        }
    }
    Ok(())
}

pub fn plan(dirs: &Dirs) -> Result<UndoPlan> {
    let home = &dirs.home;
    let state = State::load(home)?;
    let (dir, record) = latest(home, &state.outputs)?;
    let songs = songs_text(home)?;
    // An undo of a home's first run leaves a song list listing nothing.
    let before = Some(record.put_back());
    // A run stopped partway may have saved the song list after its last
    // record: what it holds now is taken back, said so below.
    let stopped = !record.complete && !record.undoing;
    if songs != record.songs_after && !(record.undoing && songs == before) && !stopped {
        return Err(crate::errors::Refused(format!(
            "The song list changed after the last run; undoing it would lose those changes. \
             Edit {} by hand instead",
            home.join(MANIFEST).display()
        ))
        .into());
    }
    refuse_another_library(&record, &state, dirs)?;
    refuse_writing_over(&record, &state, dirs)?;
    all_trashed_back(dirs, &record)?;
    let mut lines = Vec::new();
    if record.undoing {
        lines.push("Finish the undo stopped partway".to_string());
    }
    if !record.complete {
        lines.push("The run was stopped before it ended".to_string());
    }
    if stopped {
        // Written as the run stopped, before the state could stamp it: what
        // it holds cannot be told from an edit of yours made since.
        for rel in record.touched.iter().filter(|rel| {
            dirs.library.join(rel).exists()
                && state.outputs.get(*rel).is_some_and(|w| w.stamp.is_none())
        }) {
            lines.push(format!(
                "{}: written as the run stopped, so not told from an edit since; \
                 put back as it was before the run",
                crate::relpath::show(rel)
            ));
        }
    }
    if stopped && songs != record.songs_after {
        lines.push(
            "The song list differs from the stopped run's last record: what it saved since, \
             or was edited by hand since, is taken back too"
                .to_string(),
        );
    }
    if (record.songs_before != record.songs_after || songs != record.songs_after) && !record.undone
    {
        lines.push("Put the song list back as it was".to_string());
    }
    // What an undo stopped partway put back already is not said again.
    let back = |rel: &PathBuf| record.undoing && !dir.join("files").join(rel).exists();
    // A file the run moved and then wrote over is said by its move back.
    let moved_to = |rel: &PathBuf| record.moves.iter().any(|m| m.to == *rel);
    for rel in (record.touched.iter()).filter(|rel| !back(rel) && !moved_to(rel)) {
        let how = if record.kept.contains(rel) {
            "put back"
        } else {
            "written again from its sources"
        };
        lines.push(format!("{}: {how}", crate::relpath::show(rel)));
    }
    let moved_back = |m: &Moved| {
        record.undoing && dirs.library.join(&m.from).exists() && !dirs.library.join(&m.to).exists()
    };
    let mut said = BTreeSet::new();
    for m in (record.moves.iter()).filter(|m| !moved_back(m) && said.insert((&m.from, &m.to))) {
        let then = if record.touched.contains(&m.to) && !record.kept.contains(&m.to) {
            ", then written again from its sources"
        } else {
            ""
        };
        lines.push(format!(
            "{}: moved back from {}{then}",
            crate::relpath::show(&m.from),
            crate::relpath::show(&m.to)
        ));
    }
    let freed_back = |rel: &PathBuf| record.undoing && !dir.join(FREED).join(rel).exists();
    for rel in record.freed.iter().filter(|rel| !freed_back(rel)) {
        let how = if dir.join(FREED).join(rel).exists() {
            "put back, once the files moved there are back"
        } else {
            "written again from its sources"
        };
        lines.push(format!("{}: {how}", crate::relpath::show(rel)));
    }
    lines.extend(copies_said(dirs, &record));
    // What `undo` removes, the files beside a song, as its lyrics, too.
    let added = made_by_run(&record, &state.outputs, &dirs.library);
    if !added.is_empty() {
        let files = if added.len() == 1 { "file" } else { "files" };
        lines.push(format!("Remove {} {files} the run added:", added.len()));
        for rel in &added {
            lines.push(format!("  {}", crate::relpath::show(rel)));
        }
    }
    Ok(UndoPlan { dir, record, lines })
}

fn latest(home: &Path, now: &BTreeMap<PathBuf, Written>) -> Result<(PathBuf, Record)> {
    let Some(dir) = runs(home)?.pop() else {
        return Err(crate::errors::Refused("There is no run to undo".into()).into());
    };
    let text = std::fs::read(dir.join(RECORD))
        .with_context(|| format!("reading {}", dir.join(RECORD).display()))?;
    let record = Record::read(&text, now)
        .with_context(|| format!("{} cannot be read", dir.join(RECORD).display()))?;
    Ok((dir, record))
}

/// Library files a step changed before the sync after it, which that
/// sync's `changed` hook counts with its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Changed {
    pub written: usize,
    pub moved: usize,
    pub removed: usize,
}

/// An undo whose song list and state are back, its record kept until
/// the sync after it ends: [`Undone::finish`] lets go of it.
#[derive(Debug)]
#[must_use]
pub struct Undone {
    dir: PathBuf,
    changed: Changed,
}

impl Undone {
    /// The library files the undo put back, moved back and removed.
    #[must_use]
    pub fn changed(&self) -> Changed {
        self.changed
    }

    /// Let go of the run's record, the sync after the undo ended. That
    /// sync may write into the song list what it reads anew, as a
    /// source's changed tags, and the run before, now the last, is moved
    /// on to the list it left: the runs lock held from the undo to here,
    /// no hand changed it meanwhile, so the next `undo` goes on.
    // Lets go of a run's folder once nothing needs it.
    #[expect(clippy::disallowed_methods)]
    pub fn finish(self) -> Result<()> {
        let (before, followed) = (put_back_as(&self.dir, None)?).unwrap_or_default();
        std::fs::remove_dir_all(&self.dir)
            .with_context(|| format!("removing {}", self.dir.display()))?;
        let Some(home) = self.dir.parent().and_then(Path::parent) else {
            return Ok(());
        };
        let Some(last) = runs(home)?.pop() else {
            return Ok(());
        };
        let now = songs_text(home)?.unwrap_or_else(|| crate::manifest::NEW.to_string());
        let left = head(&last).filter(|h| h.complete && !h.undoing && !h.undone);
        if (now == before && followed.is_empty())
            || left.and_then(|h| h.songs_after) != Some(before)
        {
            return Ok(());
        }
        move_on(&last, &now, &followed)
    }
}

/// A save of the song list by the sync after the undo, as following a
/// file the user moved: the list the undo leaves, written into its record
/// as the list put back, and the run before, which left the list the undo
/// put back, moved on to it, as [`Undone::finish`] moves it. Stopped
/// before that, the next run begins from the list saved rather than take
/// it for a hand edit, and an `undo` run again to finish this one finds
/// the list as it left it.
impl crate::manifest::Recorder for Undone {
    fn saved(&mut self, manifest: &crate::manifest::Manifest) -> Result<()> {
        let now = manifest.text().unwrap_or(crate::manifest::NEW);
        let Some((was, followed)) = put_back_as(&self.dir, Some(now))? else {
            return Ok(());
        };
        let Some(home) = self.dir.parent().and_then(Path::parent) else {
            return Ok(());
        };
        let before = runs(home)?
            .into_iter()
            .take_while(|dir| *dir != self.dir)
            .last();
        let left = (before.as_deref())
            .and_then(head)
            .filter(|h| h.complete && !h.undoing && !h.undone)
            .and_then(|h| h.songs_after);
        match before {
            Some(before) if left.as_ref() == Some(&was) => move_on(&before, now, &followed),
            _ => Ok(()),
        }
    }
}

/// The song list before the run the undone record in `dir` keeps, as
/// the run before it left it, a new home's for none, and the user's files
/// that run followed, which the list the undo put back follows. Given
/// `now`, the record is given `now` as its list before and none followed,
/// as `now` names each file where it is. `None` when the record does not
/// read.
fn put_back_as(
    dir: &Path,
    now: Option<&str>,
) -> Result<Option<(String, BTreeMap<SourceKey, SourceKey>)>> {
    let Ok(text) = std::fs::read(dir.join(RECORD)) else {
        return Ok(None);
    };
    let Ok(mut record) = serde_json::from_slice::<serde_json::Value>(&text) else {
        return Ok(None);
    };
    let was = (record["songs_before"].as_str())
        .unwrap_or(crate::manifest::NEW)
        .to_string();
    let followed: BTreeMap<SourceKey, SourceKey> =
        serde_json::from_value(record["followed"].clone()).unwrap_or_default();
    if let Some(now) = now {
        record["songs_before"] = serde_json::Value::String(now.to_string());
        if let Some(fields) = record.as_object_mut() {
            fields.remove("followed");
        }
        let bytes = serde_json::to_vec(&record).context("writing the run record")?;
        atomic::write_private(dir, &atomic::Name::new(RECORD)?, &bytes)?;
    }
    Ok(Some((was, followed)))
}

/// Move the run kept in `dir` on to the song list `now`, the user's files
/// `followed` followed in the list its own undo puts back too.
fn move_on(dir: &Path, now: &str, followed: &BTreeMap<SourceKey, SourceKey>) -> Result<()> {
    let file = dir.join(RECORD);
    let mut record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?,
    )
    .with_context(|| format!("reading {}", file.display()))?;
    record["songs_after"] = serde_json::Value::String(now.to_string());
    if !followed.is_empty() {
        let its: BTreeMap<SourceKey, SourceKey> =
            serde_json::from_value(record["followed"].clone()).unwrap_or_default();
        record["followed"] =
            serde_json::to_value(then(&its, followed)).context("writing the run record")?;
        record["version"] = serde_json::Value::from(VERSION);
    }
    let bytes = serde_json::to_vec(&record).context("writing the run record")?;
    atomic::write_private(dir, &atomic::Name::new(RECORD)?, &bytes)
}

/// Carry out `plan`, made under the lock the caller holds: the song
/// list and the state back as they were before the run, its moved files
/// back where they were, its kept files in place, and the files it made
/// taken out, as the outputs as it began did not record them. The sync
/// that follows writes again what was not kept, then finishes the undo.
/// An undo stopped partway, in its sync too, finishes when run again.
// Puts kept files back and removes what the run added.
#[expect(clippy::disallowed_methods)]
pub fn undo<W: Write>(dirs: &Dirs, plan: UndoPlan, out: &mut W) -> Result<Undone> {
    let home = &dirs.home;
    let UndoPlan {
        dir, mut record, ..
    } = plan;
    let mut state = State::load(home)?;
    let mut changed = Changed::default();
    if !record.undoing {
        record.undoing = true;
        let text = serde_json::to_vec(&record).context("writing the run record")?;
        atomic::write_private(&dir, &atomic::Name::new(RECORD)?, &text)?;
    }
    // In the reverse of the order the run went: it moved a file before it
    // wrote over it where it moved to, as a song retagged and moved.
    for rel in &record.kept {
        let to = dirs.library.join(rel);
        let from = dir.join("files").join(rel);
        if !from.exists() {
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        atomic::rename(&from, &to)
            .or_else(|_| std::fs::copy(&from, &to).map(drop))
            .with_context(|| format!("putting back {}", to.display()))?;
        changed.written += 1;
        // A file the run moved and then wrote over is said as it moves back.
        if !record.moves.iter().any(|m| m.to == *rel) {
            crate::ui::info(out, &format!("Put back: {}", crate::relpath::show(rel)))?;
        }
    }
    if !record.complete {
        for m in &record.moves {
            let _ = crate::library::finish_move(&dirs.library, &m.from, &m.to);
        }
    }
    for m in record.moves.iter().rev() {
        let (from, to) = (dirs.library.join(&m.from), dirs.library.join(&m.to));
        if from.exists() || !to.exists() {
            continue;
        }
        if let Some(parent) = from.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        atomic::rename(&to, &from).with_context(|| format!("moving back {}", to.display()))?;
        changed.moved += 1;
        crate::ui::info(
            out,
            &format!("Moved back: {}", crate::relpath::show(&m.from)),
        )?;
    }
    // Where a move landed, so after the moves back; over a song written
    // there for a move that failed, as a file kept is put back.
    for rel in &record.freed {
        let (from, to) = (dir.join(FREED).join(rel), dirs.library.join(rel));
        if !from.exists() {
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        atomic::rename(&from, &to)
            .or_else(|_| std::fs::copy(&from, &to).map(drop))
            .with_context(|| format!("putting back {}", to.display()))?;
        changed.written += 1;
        crate::ui::info(out, &format!("Put back: {}", crate::relpath::show(rel)))?;
    }
    // A run stopped partway may have left a part beside a path it was
    // writing, which the outputs put back may no longer name.
    let moving: Vec<&PathBuf> = (record.moves.iter())
        .filter(|_| !record.complete)
        .map(|m| &m.to)
        .collect();
    crate::library::Leftovers::of(&state.outputs, moving).clear(&dirs.library);
    let made = made_by_run(&record, &state.outputs, &dirs.library);
    for rel in &made {
        let path = dirs.library.join(rel);
        atomic::remove(&path).with_context(|| format!("removing {}", path.display()))?;
        changed.removed += 1;
        crate::ui::info(out, &format!("Removed: {}", crate::relpath::show(rel)))?;
    }
    let left: Vec<PathBuf> = (record.moves.iter().map(|m| m.to.clone()))
        .chain(made)
        .collect();
    crate::library::remove_empty_folders(&dirs.library, &left);
    state.outputs = outputs_put_back(&record, &state.outputs, &dirs.library)?;
    take_out_copies(dirs, &record, out)?;
    // A home that had no song list before gets one that lists nothing, so
    // the sync after removes what the run added.
    let put_back = record.put_back();
    atomic::write(home, &atomic::Name::new(MANIFEST)?, put_back.as_bytes())?;
    state.save(home)?;
    if !record.undone {
        record.undone = true;
        let text = serde_json::to_vec(&record).context("writing the run record")?;
        atomic::write_private(&dir, &atomic::Name::new(RECORD)?, &text)?;
    }
    Ok(Undone { dir, changed })
}

/// What undoing does with each copy `add` made that is still there, a
/// line each.
fn copies_said(dirs: &Dirs, record: &Record) -> Vec<String> {
    let copies = Copies::of(dirs, record);
    let mut lines = Vec::new();
    for (rel, digest) in &record.copied {
        let Copy::At(copy) = copies.find(rel, digest) else {
            continue;
        };
        let how = if as_copied(&copy, digest) {
            "deleted"
        } else {
            "changed since, moved to the trash"
        };
        let shown = copy.strip_prefix(&dirs.home).unwrap_or(&copy);
        lines.push(format!(
            "{}: the copy the run made, {how}",
            crate::relpath::show(shown)
        ));
    }
    lines
}

/// Where a copy `add` made is now.
enum Copy {
    At(PathBuf),
    Gone,
    /// Moved, to one of these files, each holding every byte of it.
    Unsure(Vec<PathBuf>),
}

/// The manual files the song list an undo puts back names nowhere, but
/// at the path of a copy the run made, among which a copy the user moved
/// within the folder is found by its bytes; none when that song list does
/// not read.
struct Copies {
    home: PathBuf,
    manual: PathBuf,
    loose: Vec<PathBuf>,
}

impl Copies {
    fn of(dirs: &Dirs, record: &Record) -> Self {
        let before = record
            .songs_before
            .as_deref()
            .unwrap_or(crate::manifest::NEW);
        let loose = (crate::manifest::keys_in(before))
            .zip(crate::store::Store::scan(dirs).ok())
            .map(|(named, store)| {
                (store.manual_files())
                    .filter(|(k, _)| !named.contains(k))
                    .filter_map(|(k, _)| Some(store.locate(&k)?.path))
                    .filter(|p| !record.copied.keys().any(|rel| dirs.home.join(rel) == *p))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            home: dirs.home.clone(),
            manual: dirs.manual(),
            loose,
        }
    }

    /// Where the copy at `rel` under the home, of `digest` as copied, is:
    /// at its path, else the one loose file holding every byte of it.
    fn find(&self, rel: &Path, digest: &str) -> Copy {
        let at = self.home.join(rel);
        if at.exists() {
            return Copy::At(at);
        }
        if digest.is_empty() || !at.starts_with(&self.manual) {
            return Copy::Gone;
        }
        let mut found: Vec<PathBuf> = (self.loose.iter())
            .filter(|f| as_copied(f, digest))
            .cloned()
            .collect();
        match found.len() {
            0 => Copy::Gone,
            1 => Copy::At(found.remove(0)),
            _ => Copy::Unsure(found),
        }
    }
}

/// Move a file of the user's own to the desktop's trash. On macOS this
/// asks the file manager API directly rather than scripting Finder,
/// which prompts for permission and fails over SSH.
pub fn to_trash(file: &Path) -> Result<(), trash::Error> {
    // Only macOS sets the delete method; elsewhere nothing writes to it.
    #[allow(unused_mut)]
    let mut trash = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        trash.set_delete_method(DeleteMethod::NsFileManager);
    }
    trash.delete(file)
}

/// Take out each copy `add` made that is still there, and the folders
/// that leaves empty in the sources.
// Removes the copies `add` made, as undo.
#[expect(clippy::disallowed_methods)]
fn take_out_copies<W: Write>(dirs: &Dirs, record: &Record, out: &mut W) -> Result<()> {
    let copies = Copies::of(dirs, record);
    for (rel, digest) in &record.copied {
        let copy = match copies.find(rel, digest) {
            Copy::At(copy) => copy,
            Copy::Gone => {
                crate::ui::info(
                    out,
                    &format!(
                        "The copy the run made is gone already: {}",
                        dirs.home.join(rel).display()
                    ),
                )?;
                continue;
            }
            Copy::Unsure(files) => {
                let files: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
                crate::ui::warning(
                    out,
                    &format!(
                        "Left, as more than one file holds the copy the run made of {}: {}",
                        dirs.home.join(rel).display(),
                        files.join(", ")
                    ),
                )?;
                continue;
            }
        };
        // No digest: the run stopped as it copied, so what is there is
        // its copy, whole or not.
        if digest.is_empty() || as_copied(&copy, digest) {
            std::fs::remove_file(&copy).with_context(|| format!("deleting {}", copy.display()))?;
            crate::ui::info(
                out,
                &format!("Deleted the copy the run made: {}", copy.display()),
            )?;
        } else {
            to_trash(&copy).with_context(|| format!("moving {} to the trash", copy.display()))?;
            crate::ui::info(
                out,
                &format!(
                    "Moved the copy the run made, changed since, to the trash: {}",
                    copy.display()
                ),
            )?;
        }
        let sources = dirs.home.join("sources");
        for folder in copy
            .ancestors()
            .skip(1)
            .take_while(|f| f.starts_with(&sources))
        {
            if *folder == dirs.manual()
                || *folder == dirs.ytdlp()
                || *folder == dirs.torrent()
                || std::fs::remove_dir(folder).is_err()
            {
                break;
            }
        }
    }
    Ok(())
}

/// Whether `file` holds the bytes it was copied with.
fn as_copied(file: &Path, digest: &str) -> bool {
    crate::hash::digest_file(file).as_deref() == Some(digest)
}

/// The library files `record`'s run made that undoing it takes out, by
/// the outputs `now` records: each file, an output or one beside it,
/// that the outputs as the run began did not record, still as muman
/// wrote it. One the run kept is put back instead, and one changed since
/// it was written is left, the user's once its record is put back.
fn made_by_run(record: &Record, now: &BTreeMap<PathBuf, Written>, library: &Path) -> Vec<PathBuf> {
    let then: BTreeSet<&Path> = (record.was.iter())
        .flat_map(|(rel, w)| w.files(rel))
        .collect();
    // A file a move left where it went, not moved back, is no file the
    // run made: it holds what was there before the run.
    let moved: BTreeSet<&Path> = (record.moves.iter())
        .flat_map(|m| [m.from.as_path(), m.to.as_path()])
        .collect();
    let mine = |rel: &Path| {
        !then.contains(rel)
            && !moved.contains(rel)
            && !record.kept.contains(rel)
            && library.join(rel).is_file()
    };
    let mut made = Vec::new();
    for (rel, w) in now {
        if mine(rel) && !crate::reconcile::edited_since_written(library, now, rel) {
            made.push(rel.clone());
        }
        for sidecar in &w.sidecars {
            if mine(&sidecar.path) && crate::reconcile::sidecar_as_written(library, sidecar) {
                made.push(sidecar.path.clone());
            }
        }
    }
    made.sort();
    made.dedup();
    made
}

/// The outputs once `record`'s run is undone: as the run found them,
/// each file put back stamped as it lies, and each the run wrote over and
/// did not keep left for the sync after to write again. An output the run
/// added is no longer one: [`made_by_run`] took it out, or it is the
/// user's. Each names the user's files the run followed by the keys it
/// followed them to, as the song list put back does.
///
/// A file of a song the run changed that the run itself neither wrote
/// over, removed nor moved is as `now` records it: the sync after an
/// undo since may have written it, with no run of its own to say so, and
/// the record from before this run would take that file for the user's.
/// A song none of whose files the run changed, as one whose write failed,
/// is recorded whole as `now` has it, under the sources it had, so the
/// plan recorded is the one its files were written from.
fn outputs_put_back(
    record: &Record,
    now: &BTreeMap<PathBuf, Written>,
    library: &Path,
) -> Result<BTreeMap<PathBuf, Written>> {
    let mut outputs = BTreeMap::new();
    // A file moved back holds what the run wrote over it where it went,
    // unless what it held there was kept and put back.
    let written_over: BTreeSet<&PathBuf> = record
        .moves
        .iter()
        .filter(|m| record.touched.contains(&m.to) && !record.kept.contains(&m.to))
        .map(|m| &m.from)
        .collect();
    let changed_by_run = |rel: &Path| {
        record.touched.contains(rel)
            || record.freed.contains(rel)
            || (record.moves.iter()).any(|m| m.from == rel || m.to == rel)
    };
    let beside_now: BTreeMap<&PathBuf, &crate::state::Sidecar> = (now.values())
        .flat_map(|w| &w.sidecars)
        .map(|s| (&s.path, s))
        .collect();
    for (rel, before) in &record.was {
        let untouched = !before.files(rel).any(changed_by_run);
        if let Some(w) = now.get(rel).filter(|_| untouched) {
            let written = Written {
                sources: before.sources.clone(),
                ..w.clone()
            };
            outputs.insert(rel.clone(), written);
            continue;
        }
        let mut written = before.clone();
        if record.kept.contains(rel) {
            // One changed since written keeps the stamp it no longer matches.
            if !record.changed.contains(rel) {
                written.stamp = store::stamp_text(&library.join(rel));
            }
        } else if record.touched.contains(rel) || written_over.contains(rel) {
            written.plan = None;
            written.stamp = None;
        } else if let Some(w) = now.get(rel).filter(|_| !changed_by_run(rel)) {
            written.stamp.clone_from(&w.stamp);
            written.digest.clone_from(&w.digest);
        }
        for sidecar in &mut written.sidecars {
            if let Some(s) = beside_now.get(&sidecar.path)
                && !changed_by_run(&sidecar.path)
            {
                sidecar.stamp.clone_from(&s.stamp);
                sidecar.digest.clone_from(&s.digest);
            }
        }
        outputs.insert(rel.clone(), written);
    }
    let mut followed = State {
        outputs,
        ..State::default()
    };
    followed.rekey(&record.followed)?;
    Ok(followed.outputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(root: &Path) -> Dirs {
        Dirs {
            home: root.join("home"),
            library: root.join("lib"),
            locks: Some(root.to_path_buf()),
        }
    }

    fn written() -> Written {
        Written {
            sources: Vec::new(),
            sidecars: Vec::new(),
            plan: None,
            stamp: Some("1:1".into()),
            digest: None,
        }
    }

    #[test]
    fn a_run_that_changes_nothing_leaves_no_record() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        std::fs::create_dir_all(&d.home).unwrap();
        Run::begin(&d.home).unwrap().finish(&d.home).unwrap();
        assert_eq!(runs(&d.home).unwrap(), Vec::<PathBuf>::new());
    }

    #[cfg(unix)]
    #[test]
    fn a_record_of_the_song_list_s_text_is_its_owner_s_alone() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        std::fs::create_dir_all(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "args = [\"--password\", \"0001\"]").unwrap();
        let run = Run::begin(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "args = [\"--password\", \"0002\"]").unwrap();
        run.finish(&d.home).unwrap();
        let kept = runs(&d.home).unwrap();
        assert_eq!(mode(&history(&d.home)), 0o700);
        assert_eq!(mode(&kept[0]), 0o700);
        assert_eq!(mode(&kept[0].join(RECORD)), 0o600);
    }

    #[test]
    fn moves_made_one_set_after_another_are_one_set_of_moves() {
        let key = |p: &str| SourceKey::Manual(p.into());
        let moves = |pairs: &[(&str, &str)]| -> BTreeMap<SourceKey, SourceKey> {
            pairs.iter().map(|(a, b)| (key(a), key(b))).collect()
        };
        let swapped = moves(&[("a.flac", "b.flac"), ("b.flac", "a.flac")]);
        assert_eq!(
            then(
                &moves(&[("a.flac", "x/a.flac")]),
                &moves(&[("x/a.flac", "y/a.flac")])
            ),
            moves(&[("a.flac", "y/a.flac")])
        );
        assert_eq!(then(&swapped, &swapped), moves(&[]), "swapped back");
        assert_eq!(
            then(
                &moves(&[("a.flac", "x.flac")]),
                &moves(&[("a.flac", "z.flac")])
            ),
            moves(&[("a.flac", "x.flac")]),
            "a file put at a key since is none of the keys before"
        );
        assert_eq!(
            then(
                &moves(&[("a.flac", "x.flac")]),
                &moves(&[("b.flac", "c.flac")])
            ),
            moves(&[("a.flac", "x.flac"), ("b.flac", "c.flac")])
        );
    }

    #[test]
    fn undo_puts_the_song_list_back_with_the_files_the_run_followed() {
        let key = |p: &str| SourceKey::Manual(p.into());
        let record = Record {
            songs_before: Some(
                "version = 1\n[[song]]\nsources = [\"manual:a.flac\"] # mine\n\
                 [[song]]\nsources = [\"manual:b.flac\"]\n"
                    .into(),
            ),
            followed: BTreeMap::from([
                (key("a.flac"), key("b.flac")),
                (key("b.flac"), key("a.flac")),
            ]),
            ..Record::default()
        };
        assert_eq!(
            record.put_back(),
            "version = 1\n[[song]]\nsources = [\"manual:b.flac\"] # mine\n\
             [[song]]\nsources = [\"manual:a.flac\"]\n"
        );
    }

    #[test]
    fn a_record_without_a_version_is_read_as_version_1() {
        let old = br#"{"songs_before":"before","songs_after":"after",
            "outputs":{"A/x.opus":{"sources":[],"stamp":"1:1"}},"touched":[],"kept":[]}"#;
        let mut now = BTreeMap::new();
        now.insert(PathBuf::from("A/y.opus"), written());
        let record = Record::read(old, &now).unwrap();
        assert_eq!(record.version, 1);
        assert_eq!(
            record.was.keys().collect::<Vec<_>>(),
            [Path::new("A/x.opus")],
            "the outputs as the run began are those version 1 kept"
        );
    }

    #[test]
    fn undo_puts_the_song_list_and_a_kept_file_back() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        std::fs::create_dir_all(&d.home).unwrap();
        std::fs::create_dir_all(d.library.join("A")).unwrap();
        std::fs::write(d.home.join(MANIFEST), "before").unwrap();
        std::fs::write(d.library.join("A/x.opus"), "old").unwrap();
        let mut state = State {
            library: Some(d.library.clone()),
            ..State::default()
        };
        state.outputs.insert("A/x.opus".into(), written());
        state.save(&d.home).unwrap();

        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.keep(&d.library, Path::new("A/x.opus")).unwrap();
        std::fs::remove_file(d.library.join("A/x.opus")).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        state.outputs.clear();
        state.save(&d.home).unwrap();
        run.finish(&d.home).unwrap();

        assert_eq!(plan(&d).unwrap().lines().to_vec().len(), 2);
        undo(&d, plan(&d).unwrap(), &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(d.home.join(MANIFEST)).unwrap(),
            "before"
        );
        assert_eq!(
            std::fs::read_to_string(d.library.join("A/x.opus")).unwrap(),
            "old"
        );
        let back = State::load(&d.home).unwrap();
        assert!(back.outputs.contains_key(Path::new("A/x.opus")));
        assert!(runs(&d.home).unwrap().is_empty(), "an undone run is gone");
    }

    #[test]
    fn a_file_kept_that_the_run_never_replaced_is_let_go_of() {
        let dir = tempfile::tempdir().unwrap();
        let (d, state) = library(dir.path());
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.keep(&d.library, Path::new("A/x.opus")).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        run.finish(&d.home).unwrap();
        let kept = runs(&d.home).unwrap();
        assert_eq!(kept.len(), 1, "the song list's change is the run's");
        assert!(!kept[0].join("files/A/x.opus").exists());
        assert!(
            !plan(&d)
                .unwrap()
                .lines()
                .iter()
                .any(|l| l.contains("x.opus"))
        );
    }

    #[test]
    fn a_stopped_run_is_undone_saying_what_the_song_list_holds_since() {
        let dir = tempfile::tempdir().unwrap();
        let (d, state) = library(dir.path());
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        std::fs::write(d.home.join(MANIFEST), "saved by the run").unwrap();
        run.checkpoint(&d.home).unwrap();
        drop(run);
        std::fs::write(d.home.join(MANIFEST), "saved again, or by hand").unwrap();
        let planned = plan(&d).unwrap();
        let said = planned.lines().join("\n");
        assert!(said.contains("stopped before it ended"), "{said}");
        assert!(said.contains("is taken back too"), "{said}");
        assert!(said.contains("Put the song list back"), "{said}");
        undo(&d, planned, &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(d.home.join(MANIFEST)).unwrap(),
            "before"
        );
    }

    #[test]
    fn undo_is_refused_over_a_file_edited_since_the_run_wrote_it() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.keep(&d.library, Path::new("A/x.opus")).unwrap();
        std::fs::remove_file(d.library.join("A/x.opus")).unwrap();
        std::fs::write(d.library.join("A/x.opus"), "new").unwrap();
        state.outputs.insert(
            "A/x.opus".into(),
            Written {
                stamp: store::stamp_text(&d.library.join("A/x.opus")),
                digest: None,
                ..written()
            },
        );
        state.save(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        run.finish(&d.home).unwrap();
        assert!(plan(&d).is_ok());

        std::fs::write(d.library.join("A/x.opus"), "new, retagged by hand").unwrap();
        let e = plan(&d).unwrap_err();
        assert!(format!("{e:#}").contains("A/x.opus changed since"), "{e:#}");
    }

    #[test]
    fn undo_is_refused_once_the_song_list_changed_again() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        std::fs::create_dir_all(&d.home).unwrap();
        let run = Run::begin(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        run.finish(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "edited by hand").unwrap();
        let e = plan(&d).unwrap_err();
        assert!(
            e.downcast_ref::<crate::errors::Refused>().is_some(),
            "{e:#}"
        );
        assert!(format!("{e:#}").contains("changed after"), "{e:#}");
    }

    /// A library with one file muman wrote, `A/x.opus`, and its state.
    fn library(root: &Path) -> (Dirs, State) {
        let d = dirs(root);
        std::fs::create_dir_all(&d.home).unwrap();
        std::fs::create_dir_all(d.library.join("A")).unwrap();
        std::fs::write(d.home.join(MANIFEST), "before").unwrap();
        std::fs::write(d.library.join("A/x.opus"), "old").unwrap();
        let mut state = State {
            library: Some(d.library.clone()),
            ..State::default()
        };
        state.outputs.insert("A/x.opus".into(), written());
        state.save(&d.home).unwrap();
        (d, state)
    }

    #[test]
    fn a_run_that_never_reaches_the_library_leaves_no_record() {
        let dir = tempfile::tempdir().unwrap();
        let (d, _) = library(dir.path());
        Run::begin(&d.home).unwrap().finish(&d.home).unwrap();
        assert_eq!(runs(&d.home).unwrap(), Vec::<PathBuf>::new());
    }

    #[test]
    fn a_run_stopped_partway_is_listed_and_undone() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        run.keep(&d.library, Path::new("A/x.opus")).unwrap();
        run.checkpoint(&d.home).unwrap();
        // As a render lands: a new file renamed over the old.
        std::fs::write(d.library.join("A/x.part"), "half written").unwrap();
        std::fs::rename(d.library.join("A/x.part"), d.library.join("A/x.opus")).unwrap();
        state.outputs.clear();
        state.save(&d.home).unwrap();
        drop(run);

        let said = plan(&d).unwrap().lines().to_vec();
        assert_eq!(said[0], "The run was stopped before it ended", "{said:?}");
        undo(&d, plan(&d).unwrap(), &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(d.library.join("A/x.opus")).unwrap(),
            "old"
        );
        assert_eq!(
            std::fs::read_to_string(d.home.join(MANIFEST)).unwrap(),
            "before"
        );
    }

    #[test]
    fn a_move_is_undone() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.moving(Path::new("A/x.opus"), Path::new("B/x.opus"));
        run.checkpoint(&d.home).unwrap();
        std::fs::create_dir_all(d.library.join("B")).unwrap();
        std::fs::rename(d.library.join("A/x.opus"), d.library.join("B/x.opus")).unwrap();
        let w = state.outputs.remove(Path::new("A/x.opus")).unwrap();
        state.outputs.insert("B/x.opus".into(), w);
        state.save(&d.home).unwrap();
        run.finish(&d.home).unwrap();

        assert!(
            plan(&d)
                .unwrap()
                .lines()
                .contains(&"A/x.opus: moved back from B/x.opus".to_string())
        );
        undo(&d, plan(&d).unwrap(), &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(d.library.join("A/x.opus")).unwrap(),
            "old"
        );
        let back = State::load(&d.home).unwrap();
        assert_eq!(
            back.outputs.keys().collect::<Vec<_>>(),
            [Path::new("A/x.opus")]
        );
    }

    #[test]
    fn a_move_then_a_retag_is_undone_in_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.moving(Path::new("A/x.opus"), Path::new("B/x.opus"));
        run.checkpoint(&d.home).unwrap();
        std::fs::create_dir_all(d.library.join("B")).unwrap();
        std::fs::rename(d.library.join("A/x.opus"), d.library.join("B/x.opus")).unwrap();
        run.keep(&d.library, Path::new("B/x.opus")).unwrap();
        std::fs::remove_file(d.library.join("B/x.opus")).unwrap();
        std::fs::write(d.library.join("B/x.opus"), "retagged").unwrap();
        let w = state.outputs.remove(Path::new("A/x.opus")).unwrap();
        let stamp = store::stamp_text(&d.library.join("B/x.opus"));
        state
            .outputs
            .insert("B/x.opus".into(), Written { stamp, ..w });
        state.save(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        run.finish(&d.home).unwrap();

        let planned = plan(&d).unwrap();
        assert_eq!(
            planned
                .lines()
                .iter()
                .filter(|l| l.contains("B/x.opus"))
                .count(),
            1,
            "the file moved and written over is said by its move: {:?}",
            planned.lines()
        );
        undo(&d, planned, &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(d.library.join("A/x.opus")).unwrap(),
            "old"
        );
        assert!(!d.library.join("B/x.opus").exists());
        let back = State::load(&d.home).unwrap();
        assert_eq!(
            back.outputs.keys().collect::<Vec<_>>(),
            [Path::new("A/x.opus")]
        );
    }

    /// A run that retags `A/x.opus`, its lyrics beside it left as they
    /// were, then the lyrics written anew with no run of their own, as the
    /// sync after an undo of a later run writes them.
    fn retagged_then_lyrics_written_since(root: &Path) -> Dirs {
        let (d, mut state) = library(root);
        std::fs::write(d.library.join("A/x.lrc"), "old words").unwrap();
        let lyrics = |library: &Path| crate::state::Sidecar::vouched(library, "A/x.lrc".into());
        let song = state.outputs.get_mut(Path::new("A/x.opus")).unwrap();
        song.stamp = store::stamp_text(&d.library.join("A/x.opus"));
        song.sidecars = vec![lyrics(&d.library)];
        state.save(&d.home).unwrap();
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.keep(&d.library, Path::new("A/x.opus")).unwrap();
        std::fs::remove_file(d.library.join("A/x.opus")).unwrap();
        std::fs::write(d.library.join("A/x.opus"), "retagged").unwrap();
        let song = state.outputs.get_mut(Path::new("A/x.opus")).unwrap();
        song.stamp = store::stamp_text(&d.library.join("A/x.opus"));
        state.save(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        run.finish(&d.home).unwrap();
        std::fs::write(d.library.join("A/x.lrc"), "new words, longer").unwrap();
        let song = state.outputs.get_mut(Path::new("A/x.opus")).unwrap();
        song.sidecars = vec![lyrics(&d.library)];
        state.save(&d.home).unwrap();
        d
    }

    #[test]
    fn a_file_beside_a_song_the_run_left_as_it_was_is_muman_s_as_written_since() {
        let dir = tempfile::tempdir().unwrap();
        let d = retagged_then_lyrics_written_since(dir.path());
        undo(&d, plan(&d).unwrap(), &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(d.library.join("A/x.opus")).unwrap(),
            "old"
        );
        let back = State::load(&d.home).unwrap();
        let song = &back.outputs[Path::new("A/x.opus")];
        assert!(
            song.sidecars
                .iter()
                .all(|s| crate::reconcile::sidecar_as_written(&d.library, s)),
            "{song:?}"
        );
    }

    #[test]
    fn a_song_none_of_whose_files_the_run_changed_is_put_back_as_written_since() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        let song = state.outputs.get_mut(Path::new("A/x.opus")).unwrap();
        song.stamp = store::stamp_text(&d.library.join("A/x.opus"));
        state.save(&d.home).unwrap();
        // A run whose write of the song failed: its record changed, and
        // none of its files.
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.changing(Path::new("A/x.opus"));
        let song = state.outputs.get_mut(Path::new("A/x.opus")).unwrap();
        song.sources = vec![SourceKey::youtube("aaaaaaaaaaa")];
        state.save(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        run.finish(&d.home).unwrap();
        // Written since with no run of its own, as an undo's sync writes.
        std::fs::write(d.library.join("A/x.opus"), "written since").unwrap();
        let song = state.outputs.get_mut(Path::new("A/x.opus")).unwrap();
        song.stamp = store::stamp_text(&d.library.join("A/x.opus"));
        song.digest = Some("since".into());
        let since = crate::resolve::testing::every_part(crate::resolve::Format::Copy {
            codec: crate::codec::Codec::Opus,
        });
        song.plan = Some(since.clone());
        state.save(&d.home).unwrap();
        undo(&d, plan(&d).unwrap(), &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        let back = &State::load(&d.home).unwrap().outputs[Path::new("A/x.opus")];
        assert_eq!(back.plan, Some(since), "the plan it was written from");
        assert_eq!(back.digest.as_deref(), Some("since"), "{back:?}");
        assert!(back.sources.is_empty(), "the sources it had: {back:?}");
        assert!(
            !crate::reconcile::changed_since_written(
                &d.library,
                &State::load(&d.home).unwrap().outputs,
                Path::new("A/x.opus")
            ),
            "{back:?}"
        );
    }

    #[test]
    fn a_move_then_a_retag_not_kept_is_written_again_where_it_moves_back() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        std::fs::write(
            d.home.join(MANIFEST),
            "version = 1\n[history]\nmax_size = \"2 B\"\n",
        )
        .unwrap();
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.moving(Path::new("A/x.opus"), Path::new("B/x.opus"));
        std::fs::create_dir_all(d.library.join("B")).unwrap();
        std::fs::rename(d.library.join("A/x.opus"), d.library.join("B/x.opus")).unwrap();
        run.keep(&d.library, Path::new("B/x.opus")).unwrap();
        std::fs::remove_file(d.library.join("B/x.opus")).unwrap();
        std::fs::write(d.library.join("B/x.opus"), "retagged").unwrap();
        let w = state.outputs.remove(Path::new("A/x.opus")).unwrap();
        let stamp = store::stamp_text(&d.library.join("B/x.opus"));
        state
            .outputs
            .insert("B/x.opus".into(), Written { stamp, ..w });
        state.save(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        run.finish(&d.home).unwrap();

        undo(&d, plan(&d).unwrap(), &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(d.library.join("A/x.opus")).unwrap(),
            "retagged"
        );
        let back = State::load(&d.home).unwrap();
        let written = &back.outputs[Path::new("A/x.opus")];
        assert!(
            written.plan.is_none() && written.stamp.is_none(),
            "the next sync writes it again: {written:?}"
        );
    }

    #[test]
    fn a_file_moved_by_a_run_stopped_before_it_said_so_follows() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.moving(Path::new("A/x.opus"), Path::new("B/x.opus"));
        run.checkpoint(&d.home).unwrap();
        std::fs::create_dir_all(d.library.join("B")).unwrap();
        std::fs::rename(d.library.join("A/x.opus"), d.library.join("B/x.opus")).unwrap();
        drop(run);
        assert_eq!(
            follow_moves(&d.home, &d.library, &mut state, Moves::Follow),
            1
        );
        assert_eq!(
            state.outputs.keys().collect::<Vec<_>>(),
            [Path::new("B/x.opus")]
        );
        assert_eq!(
            follow_moves(&d.home, &d.library, &mut state, Moves::Follow),
            0
        );
    }

    #[test]
    fn a_file_beside_a_song_follows_it_but_only_when_not_foreseen() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        std::fs::write(d.library.join("A/x.lrc"), "lyrics").unwrap();
        let lrc = crate::state::Sidecar {
            path: "A/x.lrc".into(),
            stamp: None,
            digest: None,
        };
        state
            .outputs
            .get_mut(Path::new("A/x.opus"))
            .unwrap()
            .sidecars = vec![lrc];
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.moving(Path::new("A/x.opus"), Path::new("B/x.opus"));
        run.checkpoint(&d.home).unwrap();
        std::fs::create_dir_all(d.library.join("B")).unwrap();
        std::fs::rename(d.library.join("A/x.opus"), d.library.join("B/x.opus")).unwrap();
        drop(run);
        let sidecar = |s: &State| s.outputs[Path::new("B/x.opus")].sidecars[0].path.clone();

        let mut foreseen = state.clone();
        assert_eq!(
            follow_moves(&d.home, &d.library, &mut foreseen, Moves::Foresee),
            1
        );
        assert_eq!(sidecar(&foreseen), Path::new("A/x.lrc"));
        assert!(d.library.join("A/x.lrc").exists(), "nothing moved");

        assert_eq!(
            follow_moves(&d.home, &d.library, &mut state, Moves::Follow),
            1
        );
        assert_eq!(sidecar(&state), Path::new("B/x.lrc"));
        assert!(d.library.join("B/x.lrc").exists());
    }

    #[test]
    fn a_chain_of_moves_follows_whole_and_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        std::fs::write(d.library.join("A/y.opus"), "other").unwrap();
        state.outputs.insert(
            "A/y.opus".into(),
            Written {
                stamp: Some("2:2".into()),
                ..written()
            },
        );
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        // x to a new folder, then y into the place x left.
        run.moving(Path::new("A/x.opus"), Path::new("B/x.opus"));
        run.moving(Path::new("A/y.opus"), Path::new("A/x.opus"));
        run.checkpoint(&d.home).unwrap();
        std::fs::create_dir_all(d.library.join("B")).unwrap();
        std::fs::rename(d.library.join("A/x.opus"), d.library.join("B/x.opus")).unwrap();
        std::fs::rename(d.library.join("A/y.opus"), d.library.join("A/x.opus")).unwrap();
        drop(run);
        assert_eq!(
            follow_moves(&d.home, &d.library, &mut state, Moves::Follow),
            2
        );
        assert_eq!(
            state.outputs[Path::new("B/x.opus")].stamp.as_deref(),
            Some("1:1")
        );
        assert_eq!(
            state.outputs[Path::new("A/x.opus")].stamp.as_deref(),
            Some("2:2")
        );
        assert!(!state.outputs.contains_key(Path::new("A/y.opus")));
        let saved = state.outputs.clone();
        assert_eq!(
            follow_moves(&d.home, &d.library, &mut state, Moves::Follow),
            0,
            "said so already"
        );
        assert_eq!(state.outputs, saved);
    }

    #[test]
    fn a_chain_stopped_halfway_follows_the_moves_made() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        std::fs::write(d.library.join("A/y.opus"), "other").unwrap();
        state.outputs.insert("A/y.opus".into(), written());
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.moving(Path::new("A/x.opus"), Path::new("B/x.opus"));
        run.moving(Path::new("A/y.opus"), Path::new("A/x.opus"));
        run.checkpoint(&d.home).unwrap();
        std::fs::create_dir_all(d.library.join("B")).unwrap();
        std::fs::rename(d.library.join("A/x.opus"), d.library.join("B/x.opus")).unwrap();
        drop(run);
        assert_eq!(
            follow_moves(&d.home, &d.library, &mut state, Moves::Follow),
            1
        );
        assert_eq!(
            state.outputs.keys().collect::<Vec<_>>(),
            [Path::new("A/y.opus"), Path::new("B/x.opus")]
        );
    }

    #[test]
    fn a_run_waiting_on_another_begins_where_that_one_ended() {
        let dir = tempfile::tempdir().unwrap();
        let (d, _) = library(dir.path());
        let first = Run::begin(&d.home).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let home = d.home.clone();
            let waiting = scope.spawn(move || {
                tx.send(()).unwrap();
                Run::begin(&home).unwrap().record.songs_before
            });
            rx.recv().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            std::fs::write(d.home.join(MANIFEST), "after").unwrap();
            first.finish(&d.home).unwrap();
            assert_eq!(waiting.join().unwrap().as_deref(), Some("after"));
        });
    }

    #[test]
    fn a_hand_edit_made_while_a_run_goes_is_no_part_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let (d, _) = library(dir.path());
        let mut run = Run::begin(&d.home).unwrap();
        let read = "version = 1\n# read by the run\n";
        std::fs::write(d.home.join(MANIFEST), read).unwrap();
        run.saw(&crate::manifest::Manifest::load(&d.home).unwrap());
        std::fs::write(d.home.join(MANIFEST), "version = 1\n# by hand\n").unwrap();
        run.finish(&d.home).unwrap();
        let last = runs(&d.home).unwrap().pop().and_then(|r| head(&r)).unwrap();
        assert_eq!(last.songs_after.as_deref(), Some(read));
        assert_eq!(
            songs_as_last_left(&d.home),
            Some(false),
            "an undo refuses to drop the hand edit"
        );
    }

    #[test]
    fn a_file_past_the_whole_budget_costs_no_other_run_its_record() {
        let dir = tempfile::tempdir().unwrap();
        let (d, state) = library(dir.path());
        std::fs::write(
            d.home.join(MANIFEST),
            "version = 1\n[history]\nmax_size = \"2 B\"\n",
        )
        .unwrap();
        let earlier = Run::begin(&d.home).unwrap();
        std::fs::write(
            d.home.join(MANIFEST),
            "version = 1\n[history]\nmax_size = \"2 B\"\n# edited\n",
        )
        .unwrap();
        earlier.finish(&d.home).unwrap();
        let kept = runs(&d.home).unwrap();
        assert_eq!(kept.len(), 1);
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.keep(&d.library, Path::new("A/x.opus")).unwrap();
        assert!(
            runs(&d.home).unwrap().contains(&kept[0]),
            "the earlier run is kept"
        );
    }

    #[test]
    fn an_undo_stopped_partway_finishes_when_run_again() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.keep(&d.library, Path::new("A/x.opus")).unwrap();
        std::fs::remove_file(d.library.join("A/x.opus")).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        state.outputs.clear();
        state.save(&d.home).unwrap();
        run.finish(&d.home).unwrap();

        let (at, mut record) = latest(&d.home, &State::load(&d.home).unwrap().outputs).unwrap();
        record.undoing = true;
        atomic::write(
            &at,
            &atomic::Name::new(RECORD).unwrap(),
            &serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        std::fs::rename(at.join("files/A/x.opus"), d.library.join("A/x.opus")).unwrap();
        std::fs::write(d.home.join(MANIFEST), "before").unwrap();
        undo(&d, plan(&d).unwrap(), &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        let back = State::load(&d.home).unwrap();
        assert!(back.outputs.contains_key(Path::new("A/x.opus")));
        assert_eq!(runs(&d.home).unwrap(), Vec::<PathBuf>::new());
    }

    #[test]
    fn an_undo_stopped_before_its_sync_ended_is_finished_by_a_run_that_syncs() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.keep(&d.library, Path::new("A/x.opus")).unwrap();
        std::fs::remove_file(d.library.join("A/x.opus")).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        state.outputs.clear();
        state.save(&d.home).unwrap();
        run.finish(&d.home).unwrap();

        drop(undo(&d, plan(&d).unwrap(), &mut Vec::new()).unwrap());
        assert_eq!(runs(&d.home).unwrap().len(), 1, "still the latest run");
        assert!(undo_stopped(&d.home));
        assert_eq!(songs_as_last_left(&d.home), Some(true));
        let again = plan(&d).unwrap();
        assert_eq!(again.lines(), ["Finish the undo stopped partway"]);

        let mut run = Run::begin(&d.home).unwrap();
        assert_eq!(run.record.songs_before.as_deref(), Some("before"));
        run.synced();
        run.finish(&d.home).unwrap();
        assert!(runs(&d.home).unwrap().is_empty(), "{:?}", runs(&d.home));
    }

    #[test]
    fn a_folder_with_no_record_is_let_go_of() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        let stray = history(&d.home).join("000000000001-000000000-1-0/files/A");
        std::fs::create_dir_all(&stray).unwrap();
        std::fs::write(stray.join("x.opus"), "held").unwrap();
        let run = Run::begin(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        run.finish(&d.home).unwrap();
        assert!(!history(&d.home).join("000000000001-000000000-1-0").exists());
        assert_eq!(runs(&d.home).unwrap().len(), 1);
    }

    #[test]
    fn the_budget_is_of_every_kept_run_together() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        std::fs::create_dir_all(&d.home).unwrap();
        std::fs::create_dir_all(&d.library).unwrap();
        let mib = vec![0_u8; 600 << 10];
        for n in 0..2 {
            std::fs::write(
                d.home.join(MANIFEST),
                format!("[history]\nmax_mib = 1\n# {n}\n"),
            )
            .unwrap();
            std::fs::write(d.library.join(format!("{n}.flac")), &mib).unwrap();
            let mut run = Run::begin(&d.home).unwrap();
            run.keep(&d.library, Path::new(&format!("{n}.flac")))
                .unwrap();
            std::fs::remove_file(d.library.join(format!("{n}.flac"))).unwrap();
            run.finish(&d.home).unwrap();
        }
        let kept: Vec<u64> = runs(&d.home).unwrap().iter().map(|r| held(r)).collect();
        assert_eq!(kept, [0, 600 << 10], "the older run made room: {kept:?}");
    }

    #[test]
    fn a_budget_lowered_since_lets_go_of_the_files_of_runs_kept_before() {
        let dir = tempfile::tempdir().unwrap();
        let (d, mut state) = library(dir.path());
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.keep(&d.library, Path::new("A/x.opus")).unwrap();
        std::fs::remove_file(d.library.join("A/x.opus")).unwrap();
        let lowered = "version = 1\n[history]\nmax_size = \"2 B\"\n";
        std::fs::write(d.home.join(MANIFEST), lowered).unwrap();
        state.outputs.clear();
        state.save(&d.home).unwrap();
        run.finish(&d.home).unwrap();
        let earlier = runs(&d.home).unwrap()[0].clone();
        assert_eq!(held(&earlier), 3);

        let run = Run::begin(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), format!("{lowered}# edited\n")).unwrap();
        run.finish(&d.home).unwrap();
        let kept = runs(&d.home).unwrap();
        assert_eq!(kept.len(), 2, "the earlier run keeps its record");
        assert_eq!(kept[0], earlier);
        assert_eq!(held(&earlier), 0);
        assert!(!earlier.join("files").exists());

        undo(&d, plan(&d).unwrap(), &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        let undoing = plan(&d).unwrap();
        assert_eq!(
            undoing.lines(),
            [
                "Put the song list back as it was",
                "A/x.opus: written again from its sources"
            ]
        );
        undo(&d, undoing, &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(d.home.join(MANIFEST)).unwrap(),
            "before"
        );
        let back = State::load(&d.home).unwrap();
        let written = &back.outputs[Path::new("A/x.opus")];
        assert!(written.plan.is_none() && written.stamp.is_none());
        assert!(!d.library.join("A/x.opus").exists());
    }

    #[test]
    fn only_the_latest_runs_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        std::fs::create_dir_all(&d.home).unwrap();
        for n in 0..crate::settings::History::default().runs + 2 {
            let run = Run::begin(&d.home).unwrap();
            std::fs::write(d.home.join(MANIFEST), n.to_string()).unwrap();
            run.finish(&d.home).unwrap();
        }
        assert_eq!(
            runs(&d.home).unwrap().len(),
            crate::settings::History::default().runs
        );
    }
}
