//! Each run that changes the song list or the library, kept so `undo`
//! can put both back: the song list before and after, the outputs
//! before, the library files it replaced or removed, as many as a budget
//! of bytes allows, and the files it moved. A file not kept is written
//! again from its sources instead.
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
//! [`crate::library::Ledger`], and again as it goes, so a run
//! stopped partway, by a crash or Ctrl-C, is still listed, as
//! interrupted, and can be undone. A folder with no record, left by a run
//! stopped before it wrote one, holds nothing `undo` could use and is
//! let go of with the runs past the count. A file such a run moved before
//! the state could say so is followed by the next run, by the moves the
//! record lists, so it stays muman's. `undo` goes in the reverse of the
//! run's order: files put back where the run wrote over them, then moves
//! reversed, as a song retagged where it moved to needs.
//!
//! A run's song list before is the one the last kept run left, so a hand
//! edit is undone with the run that applied it. A command that changes
//! the home holds a lock of its own from first to last, `edit` from the
//! moment its editor closes, so a run waiting on another begins where that
//! one ended rather than record the other's change as its own. The song
//! list a run leaves is the one it last read or wrote, as its sync
//! reports it ([`Run::saw`]), so a hand edit made while it ran, which it
//! never read, is the next run's. `undo`
//! refuses when the song list changed after the run, by hand or
//! otherwise, rather than lose those changes. It marks the record
//! `undoing` before it changes anything and skips each step already
//! done, so an undo stopped partway finishes when run again. A purge
//! cannot be undone in full: a fetched source is fetched again by the
//! next sync, and a file of the user's own is in the desktop's trash,
//! which muman cannot take it back from on every platform. The run
//! records each it trashed, and `undo` refuses until the user puts them
//! back, rather than list their songs again with nothing to make them.
//!
//! `run.json` carries a `version`; a record without one is of version 1.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::atomic;
use crate::dirs::{Dirs, MANIFEST};
use crate::state::{State, Written};
use crate::store;

const RECORD: &str = "run.json";
const VERSION: u32 = 1;

fn version_one() -> u32 {
    VERSION
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
    songs_before: Option<String>,
    songs_after: Option<String>,
    #[serde(with = "crate::relpath::portable_keys")]
    outputs: BTreeMap<PathBuf, Written>,
    /// Library files the run replaced or removed.
    #[serde(with = "crate::relpath::portable_keys")]
    touched: BTreeSet<PathBuf>,
    /// Those kept under the run's `files`.
    #[serde(with = "crate::relpath::portable_keys")]
    kept: BTreeSet<PathBuf>,
    /// The bytes those take.
    #[serde(default)]
    bytes: u64,
    /// Library files the run moved, in the order it moved them.
    #[serde(default)]
    moves: Vec<Moved>,
    /// Files of the user's own the run moved to the desktop's trash,
    /// under the home.
    #[serde(default, with = "crate::relpath::portable_keys")]
    trashed: BTreeSet<PathBuf>,
}

/// One run, recording as it goes; written ahead by [`Run::checkpoint`]
/// and at the end by [`Run::finish`] only when the run changed anything.
#[derive(Debug)]
pub struct Run {
    dir: PathBuf,
    record: Record,
    outputs_known: bool,
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
    /// Held while the run lasts, so no other begins meanwhile.
    _runs: atomic::Lock,
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
    home.join("history")
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
    songs_after: Option<String>,
    #[serde(default)]
    bytes: u64,
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
    crate::store::walk(&dir.join("files"), usize::MAX)
        .unwrap_or_default()
        .iter()
        .filter_map(|f| std::fs::metadata(f).ok())
        .map(|m| m.len())
        .sum()
}

impl Run {
    /// Begin recording, with the song list as it is before anything.
    pub fn begin(home: &Path) -> Result<Self> {
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
        let last = runs(home)
            .ok()
            .and_then(|mut r| r.pop())
            .and_then(|dir| head(&dir));
        let now = {
            // Read whole, not halfway through another run's save.
            let _lock = atomic::Lock::folder(home)?;
            songs_text(home)
        };
        let (songs_before, now) = match (last, now) {
            (Some(last), now) if last.complete && last.songs_after.is_some() => {
                (last.songs_after, now.ok().flatten())
            }
            (_, now) => {
                let now = now?;
                (now.clone(), now)
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
        let others = runs(home)
            .unwrap_or_default()
            .into_iter()
            .map(|dir| {
                let bytes = held(&dir);
                (dir, bytes)
            })
            .collect();
        // As they stand, for a run that ends before it reaches the
        // library: an empty record would read as every song added.
        let outputs = State::load(home).map(|s| s.outputs).unwrap_or_default();
        Ok(Self {
            dir: history(home).join(id),
            record: Record {
                version: VERSION,
                complete: false,
                songs_before,
                outputs,
                ..Record::default()
            },
            outputs_known: false,
            others,
            limits,
            lost: false,
            seen: Seen::Not,
            _runs: runs_lock,
        })
    }

    /// Note the song list as the run read or wrote it: the list the run
    /// leaves, so a hand edit made while it ran, which it never read, is
    /// the next run's.
    pub fn saw(&mut self, manifest: &crate::manifest::Manifest) {
        self.seen = Seen::As(manifest.text().map(str::to_string));
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
            self.record.outputs.clone_from(outputs);
            self.outputs_known = true;
        }
    }

    /// Keep a library file the run is about to replace or remove: linked
    /// where the filesystem allows, else copied while the budget lasts.
    pub fn keep(&mut self, library: &Path, rel: &Path) -> Result<()> {
        if self.lost {
            return Ok(());
        }
        if let Err(e) = self.try_keep(library, rel) {
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

    fn try_keep(&mut self, library: &Path, rel: &Path) -> Result<()> {
        let from = library.join(rel);
        let Ok(meta) = std::fs::metadata(&from) else {
            return Ok(());
        };
        if !self.record.touched.insert(rel.to_path_buf()) {
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
        let to = self.dir.join("files").join(rel);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let kept = std::fs::hard_link(&from, &to)
            .or_else(|_| std::fs::copy(&from, &to).map(drop))
            .is_ok();
        if kept {
            self.record.bytes += meta.len();
            self.record.kept.insert(rel.to_path_buf());
        }
        Ok(())
    }

    /// Record that the run is moving a library file, before it does.
    pub fn moving(&mut self, from: &Path, to: &Path) {
        self.record.moves.push(Moved {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        });
    }

    /// Record that the run moved `file`, under `home`, to the desktop's
    /// trash, which muman cannot take it back from on every platform.
    pub fn trashed(&mut self, home: &Path, file: &Path) {
        if let Ok(rel) = file.strip_prefix(home) {
            self.record.trashed.insert(rel.to_path_buf());
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

    fn write(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;
        let text = serde_json::to_vec(&self.record).context("writing the run record")?;
        atomic::write(&self.dir, &atomic::Name::new(RECORD)?, &text)
    }

    /// Write the record when the run changed the song list or the
    /// library, and let go of runs beyond the `[history]` count and of
    /// folders without a record.
    pub fn finish(mut self, home: &Path) -> Result<()> {
        let _lock = atomic::Lock::folder(home)?;
        self.record.songs_after = self.songs_left(home)?;
        self.record.complete = true;
        if self.lost {
            return Ok(());
        }
        if self.record.touched.is_empty()
            && self.record.moves.is_empty()
            && self.record.songs_after == self.record.songs_before
            && self.record.outputs_equal_current(home)
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
fn let_go(dir: &Path) -> Result<()> {
    let record = std::fs::read(dir.join(RECORD))
        .ok()
        .and_then(|t| serde_json::from_slice::<Record>(&t).ok());
    let gone = match record {
        Some(mut record) => {
            record.kept.clear();
            record.bytes = 0;
            let text = serde_json::to_vec(&record).context("writing the run record")?;
            atomic::write(dir, &atomic::Name::new(RECORD)?, &text)?;
            dir.join("files")
        }
        None => dir.to_path_buf(),
    };
    match std::fs::remove_dir_all(&gone) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("removing {}", gone.display()))
        }
        _ => Ok(()),
    }
}

impl Record {
    /// Whether the outputs the run began with are the outputs now: a
    /// run that only wrote new songs still changed the library.
    fn outputs_equal_current(&self, home: &Path) -> bool {
        State::load(home).is_ok_and(|s| {
            s.outputs.keys().collect::<Vec<_>>() == self.outputs.keys().collect::<Vec<_>>()
        })
    }
}

/// Whether the song list is as the last run that ended left it; `None`
/// when no such run is kept to tell.
#[must_use]
pub fn songs_as_last_left(home: &Path) -> Option<bool> {
    let last = runs(home).ok()?.pop().and_then(|dir| head(&dir))?;
    let after = last.songs_after.filter(|_| last.complete)?;
    Some(songs_text(home).ok()?.as_deref() == Some(after.as_str()))
}

/// The outputs a run stopped partway left where they were: each file it
/// moved before the state could say so follows to where it went, with
/// its lyrics if they went too, so the next run knows it as muman's.
/// Returns how many followed.
///
/// The run made its moves in the order the record lists them, so those
/// made are the ones before the first whose file is not where it went;
/// one into a path an earlier move left is a chain, as a song moving into
/// another's old place. They are followed all at once, from the outputs
/// as the run began, and only when the state still says so: the paths
/// they left that no move went into are still listed.
pub fn follow_moves(home: &Path, library: &Path, state: &mut State) -> usize {
    let Some(dir) = runs(home).ok().and_then(|mut r| r.pop()) else {
        return 0;
    };
    let Some(record) = std::fs::read(dir.join(RECORD))
        .ok()
        .and_then(|t| serde_json::from_slice::<Record>(&t).ok())
    else {
        return 0;
    };
    if record.complete || record.undoing {
        return 0;
    }
    let made: Vec<&Moved> = record
        .moves
        .iter()
        .take_while(|m| library.join(&m.to).exists())
        .collect();
    // A song's move, as the outputs stood when the run began; the rest
    // are its lyrics'.
    let songs: Vec<&Moved> = made
        .iter()
        .copied()
        .filter(|m| record.outputs.contains_key(&m.from))
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
    let before = state.outputs.clone();
    let mut followed = BTreeMap::new();
    for m in &songs {
        let Some(mut written) = before.get(&m.from).cloned() else {
            continue;
        };
        if let Some(to) = written.lyrics.as_ref().and_then(|l| went.get(l)) {
            written.lyrics = Some((*to).clone());
        } else if let Some(left) = written.lyrics.clone().filter(|l| library.join(l).exists()) {
            // Stopped between the audio and its lyrics: the lyrics follow now.
            let to = m.to.with_extension("lrc");
            if !library.join(&to).exists()
                && atomic::rename(&library.join(&left), &library.join(&to)).is_ok()
            {
                written.lyrics = Some(to);
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

    /// Whether `other` undoes the same run.
    #[must_use]
    pub fn same_run(&self, other: &Self) -> bool {
        self.dir == other.dir
    }
}

/// What undoing the latest run would do, refused as `undo` itself would
/// be: when the song list changed since, or the run wrote another
/// library folder.
pub fn plan(dirs: &Dirs) -> Result<UndoPlan> {
    let home = &dirs.home;
    let (dir, record) = latest(home)?;
    let songs = songs_text(home)?;
    // An undo of a home's first run leaves a song list listing nothing.
    let before = record
        .songs_before
        .clone()
        .or_else(|| Some(crate::manifest::NEW.to_string()));
    if songs != record.songs_after && !(record.undoing && songs == before) {
        return Err(crate::change::Refused(format!(
            "The song list changed after the last run; undoing it would lose those changes. \
             Edit {} by hand instead",
            home.join(MANIFEST).display()
        ))
        .into());
    }
    let state = State::load(home)?;
    if !state
        .library
        .as_deref()
        .is_some_and(|l| crate::platform::same_path(l, &dirs.library))
    {
        return Err(
            crate::change::Refused("The last run wrote another library folder".into()).into(),
        );
    }
    // What undoing would write over must be muman's as the run left it.
    // An undo resumed has put some files back already, and a run stopped
    // partway may have written files it never recorded: both know theirs.
    if record.complete && !record.undoing {
        let mut state = state.clone();
        crate::reconcile::vouch_for_copies(&dirs.library, &mut state);
        let owned: BTreeSet<&PathBuf> = state
            .outputs
            .iter()
            .flat_map(|(p, w)| std::iter::once(p).chain(&w.lyrics))
            .collect();
        for rel in &record.touched {
            if !dirs.library.join(rel).exists() {
                continue;
            }
            let why = if !owned.contains(rel) {
                "is not muman's"
            } else if crate::reconcile::changed_since_written(&dirs.library, &state.outputs, rel) {
                "changed since muman wrote it"
            } else {
                continue;
            };
            return Err(crate::change::Refused(format!(
                "{} {why}; undoing would write over it. Move it aside first",
                crate::relpath::show(rel)
            ))
            .into());
        }
    }
    // Its song would be listed again with no file to make it from.
    if let Some(rel) = record.trashed.iter().find(|rel| !home.join(rel).exists()) {
        return Err(crate::change::Refused(format!(
            "The last run moved {} to the trash; put it back there, then undo again",
            home.join(rel).display()
        ))
        .into());
    }
    let mut lines = Vec::new();
    if !record.complete {
        lines.push("The run was stopped before it ended".to_string());
    }
    if record.songs_before != record.songs_after {
        lines.push("Put the song list back as it was".to_string());
    }
    for rel in &record.touched {
        let how = if record.kept.contains(rel) {
            "put back"
        } else {
            "written again from its sources"
        };
        lines.push(format!("{}: {how}", crate::relpath::show(rel)));
    }
    for m in &record.moves {
        lines.push(format!(
            "{}: moved back from {}",
            crate::relpath::show(&m.from),
            crate::relpath::show(&m.to)
        ));
    }
    let moved: BTreeSet<&PathBuf> = record.moves.iter().map(|m| &m.to).collect();
    let added = state
        .outputs
        .into_keys()
        .filter(|p| !record.outputs.contains_key(p) && !moved.contains(p))
        .count();
    if added > 0 {
        lines.push(format!("Remove {added} file(s) the run added"));
    }
    Ok(UndoPlan { dir, record, lines })
}

fn latest(home: &Path) -> Result<(PathBuf, Record)> {
    let Some(dir) = runs(home)?.pop() else {
        return Err(crate::change::Refused("There is no run to undo".into()).into());
    };
    let text = std::fs::read(dir.join(RECORD))
        .with_context(|| format!("reading {}", dir.join(RECORD).display()))?;
    let record: Record = serde_json::from_slice(&text)
        .with_context(|| format!("{} cannot be read", dir.join(RECORD).display()))?;
    Ok((dir, record))
}

/// Carry out `plan`, made under the lock the caller holds: the song
/// list and the state back as they were before the run, its moved files
/// back where they were and its kept files in place. The sync that
/// follows writes again what was not kept and removes what the run
/// added. An undo stopped partway finishes when run again.
pub fn undo<W: Write>(dirs: &Dirs, plan: UndoPlan, out: &mut W) -> Result<()> {
    let home = &dirs.home;
    let UndoPlan {
        dir, mut record, ..
    } = plan;
    let mut state = State::load(home)?;
    if !record.undoing {
        record.undoing = true;
        let text = serde_json::to_vec(&record).context("writing the run record")?;
        atomic::write(&dir, &atomic::Name::new(RECORD)?, &text)?;
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
        crate::ui::info(out, &format!("Put back: {}", crate::relpath::show(rel)))?;
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
        crate::ui::info(
            out,
            &format!("Moved back: {}", crate::relpath::show(&m.from)),
        )?;
    }
    let left: Vec<PathBuf> = record.moves.iter().map(|m| m.to.clone()).collect();
    crate::library::remove_empty_folders(&dirs.library, &left);
    // Lyrics the run wrote beside a song it had before are recorded by
    // nothing once its record is put back, so they go now.
    for (rel, now) in &state.outputs {
        let Some(lyrics) = &now.lyrics else { continue };
        let before = record.outputs.get(rel).and_then(|b| b.lyrics.as_ref());
        let path = dirs.library.join(lyrics);
        if record.outputs.contains_key(rel)
            && before != Some(lyrics)
            && !record.kept.contains(lyrics)
            && path.exists()
        {
            atomic::remove(&path).with_context(|| format!("removing {}", path.display()))?;
            crate::ui::info(out, &format!("Removed: {}", crate::relpath::show(lyrics)))?;
        }
    }
    let mut outputs = state.outputs.clone();
    for m in &record.moves {
        outputs.remove(&m.to);
    }
    // A file put back that was no output before is the user's again.
    for rel in record
        .kept
        .iter()
        .filter(|r| !record.outputs.contains_key(*r))
    {
        outputs.remove(rel);
    }
    // A file moved back holds what the run wrote over it where it went,
    // unless what it held there was kept and put back.
    let written_over: BTreeSet<&PathBuf> = record
        .moves
        .iter()
        .filter(|m| record.touched.contains(&m.to) && !record.kept.contains(&m.to))
        .map(|m| &m.from)
        .collect();
    for (rel, before) in &record.outputs {
        let mut written = before.clone();
        if record.kept.contains(rel) {
            written.stamp = store::stamp_text(&dirs.library.join(rel));
        } else if record.touched.contains(rel) || written_over.contains(rel) {
            written.plan = None;
            written.stamp = None;
        }
        outputs.insert(rel.clone(), written);
    }
    state.outputs = outputs;
    // A home that had no song list before gets one that lists nothing, so
    // the sync after removes what the run added.
    let before = record
        .songs_before
        .as_deref()
        .unwrap_or(crate::manifest::NEW);
    atomic::write(home, &atomic::Name::new(MANIFEST)?, before.as_bytes())?;
    state.save(home)?;
    std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(root: &Path) -> Dirs {
        Dirs {
            home: root.join("home"),
            library: root.join("lib"),
        }
    }

    fn written() -> Written {
        Written {
            sources: Vec::new(),
            lyrics: None,
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
        undo(&d, plan(&d).unwrap(), &mut Vec::new()).unwrap();
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
            e.downcast_ref::<crate::change::Refused>().is_some(),
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
        undo(&d, plan(&d).unwrap(), &mut Vec::new()).unwrap();
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
        undo(&d, plan(&d).unwrap(), &mut Vec::new()).unwrap();
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

        undo(&d, plan(&d).unwrap(), &mut Vec::new()).unwrap();
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

        undo(&d, plan(&d).unwrap(), &mut Vec::new()).unwrap();
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
        assert_eq!(follow_moves(&d.home, &d.library, &mut state), 1);
        assert_eq!(
            state.outputs.keys().collect::<Vec<_>>(),
            [Path::new("B/x.opus")]
        );
        assert_eq!(follow_moves(&d.home, &d.library, &mut state), 0);
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
        assert_eq!(follow_moves(&d.home, &d.library, &mut state), 2);
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
            follow_moves(&d.home, &d.library, &mut state),
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
        assert_eq!(follow_moves(&d.home, &d.library, &mut state), 1);
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
        assert_eq!(runs(&d.home).unwrap().len(), 1);
        let mut run = Run::begin(&d.home).unwrap();
        run.outputs_before(&state.outputs);
        run.keep(&d.library, Path::new("A/x.opus")).unwrap();
        assert_eq!(runs(&d.home).unwrap().len(), 1, "the earlier run is kept");
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

        let (at, mut record) = latest(&d.home).unwrap();
        record.undoing = true;
        atomic::write(
            &at,
            &atomic::Name::new(RECORD).unwrap(),
            &serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        std::fs::rename(at.join("files/A/x.opus"), d.library.join("A/x.opus")).unwrap();
        std::fs::write(d.home.join(MANIFEST), "before").unwrap();
        undo(&d, plan(&d).unwrap(), &mut Vec::new()).unwrap();
        let back = State::load(&d.home).unwrap();
        assert!(back.outputs.contains_key(Path::new("A/x.opus")));
        assert_eq!(runs(&d.home).unwrap(), Vec::<PathBuf>::new());
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

        undo(&d, plan(&d).unwrap(), &mut Vec::new()).unwrap();
        let undoing = plan(&d).unwrap();
        assert_eq!(
            undoing.lines(),
            [
                "Put the song list back as it was",
                "A/x.opus: written again from its sources"
            ]
        );
        undo(&d, undoing, &mut Vec::new()).unwrap();
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
