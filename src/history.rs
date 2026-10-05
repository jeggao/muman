//! Each run that changes the song list or the library, kept so `undo`
//! can put both back: the song list before and after, the outputs
//! before, and the library files it replaced or removed, as many as a
//! budget of bytes allows. A file not kept is written again from its
//! sources instead.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::atomic;
use crate::dirs::{Dirs, MANIFEST};
use crate::state::{State, Written};
use crate::store;

/// How many runs are kept to undo.
pub const KEPT_RUNS: usize = 3;
/// How many bytes of library files one run keeps.
pub const KEPT_BYTES: u64 = 2 << 30;

const RECORD: &str = "run.json";

/// What one run changed, as `undo` reads it.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Record {
    songs_before: Option<String>,
    songs_after: Option<String>,
    outputs: BTreeMap<PathBuf, Written>,
    /// Library files the run replaced or removed.
    touched: BTreeSet<PathBuf>,
    /// Those kept under the run's `files`.
    kept: BTreeSet<PathBuf>,
}

/// One run, recording as it goes; written by [`Run::finish`] only when
/// the run changed anything.
#[derive(Debug)]
pub struct Run {
    dir: PathBuf,
    record: Record,
    outputs_known: bool,
    bytes: u64,
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

impl Run {
    /// Begin recording, with the song list as it is before anything.
    pub fn begin(home: &Path) -> Result<Self> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let id = format!(
            "{:012}-{:09}-{}",
            now.as_secs(),
            now.subsec_nanos(),
            std::process::id()
        );
        Ok(Self {
            dir: history(home).join(id),
            record: Record {
                songs_before: songs_text(home)?,
                ..Record::default()
            },
            outputs_known: false,
            bytes: 0,
        })
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
        let from = library.join(rel);
        let Ok(meta) = std::fs::metadata(&from) else {
            return Ok(());
        };
        if !self.record.touched.insert(rel.to_path_buf()) {
            return Ok(());
        }
        if self.bytes + meta.len() > KEPT_BYTES {
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
            self.bytes += meta.len();
            self.record.kept.insert(rel.to_path_buf());
        }
        Ok(())
    }

    /// Write the record when the run changed the song list or the
    /// library, and let go of runs beyond [`KEPT_RUNS`].
    pub fn finish(mut self, home: &Path) -> Result<()> {
        self.record.songs_after = songs_text(home)?;
        if self.record.touched.is_empty()
            && self.record.songs_after == self.record.songs_before
            && self.record.outputs_equal_current(home)
        {
            let _ = std::fs::remove_dir_all(&self.dir);
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;
        let text = serde_json::to_vec(&self.record).context("writing the run record")?;
        atomic::write(&self.dir, RECORD, &text)?;
        let all = runs(home)?;
        for old in all.iter().take(all.len().saturating_sub(KEPT_RUNS)) {
            std::fs::remove_dir_all(old).with_context(|| format!("removing {}", old.display()))?;
        }
        Ok(())
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

/// What undoing the latest run would do, in words, or why it cannot.
pub fn describe(home: &Path) -> Result<Vec<String>> {
    let (_, record) = latest(home)?;
    let mut lines = Vec::new();
    if record.songs_before != record.songs_after {
        lines.push("Put the song list back as it was".to_string());
    }
    for rel in &record.touched {
        let how = if record.kept.contains(rel) {
            "put back"
        } else {
            "written again from its sources"
        };
        lines.push(format!("{}: {how}", rel.display()));
    }
    let added = State::load(home)?
        .outputs
        .into_keys()
        .filter(|p| !record.outputs.contains_key(p))
        .count();
    if added > 0 {
        lines.push(format!("Remove {added} file(s) the run added"));
    }
    Ok(lines)
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

/// Put the song list and the state back as they were before the latest
/// run, and its kept files in place, under the lock the caller holds.
/// The sync that follows writes again what was not kept and removes
/// what the run added. Refused when the song list changed since.
pub fn undo<W: Write>(dirs: &Dirs, out: &mut W) -> Result<()> {
    let home = &dirs.home;
    let (dir, record) = latest(home)?;
    if songs_text(home)? != record.songs_after {
        bail!(
            "the song list changed after the last run; undoing it would lose those changes. \
             Edit {} by hand instead",
            home.join(MANIFEST).display()
        );
    }
    let mut state = State::load(home)?;
    if !state
        .library
        .as_deref()
        .is_some_and(|l| crate::platform::same_path(l, &dirs.library))
    {
        bail!("the last run wrote another library folder");
    }
    for rel in &record.kept {
        let to = dirs.library.join(rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let from = dir.join("files").join(rel);
        std::fs::rename(&from, &to)
            .or_else(|_| std::fs::copy(&from, &to).map(drop))
            .with_context(|| format!("putting back {}", to.display()))?;
        crate::ui::info(out, &format!("Put back: {}", rel.display()))?;
    }
    let mut outputs = state.outputs.clone();
    for (rel, before) in &record.outputs {
        let mut written = before.clone();
        if record.kept.contains(rel) {
            written.stamp = store::stamp_text(&dirs.library.join(rel));
        } else if record.touched.contains(rel) {
            written.plan = None;
            written.stamp = None;
        }
        outputs.insert(rel.clone(), written);
    }
    state.outputs = outputs;
    match &record.songs_before {
        Some(text) => atomic::write(home, MANIFEST, text.as_bytes())?,
        None => {
            let _ = std::fs::remove_file(home.join(MANIFEST));
        }
    }
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
        }
    }

    #[test]
    fn a_run_that_changes_nothing_leaves_no_record() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        std::fs::create_dir_all(&d.home).unwrap();
        Run::begin(&d.home).unwrap().finish(&d.home).unwrap();
        assert!(runs(&d.home).unwrap().is_empty());
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

        assert!(describe(&d.home).unwrap().len() == 2);
        undo(&d, &mut Vec::new()).unwrap();
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
    fn undo_is_refused_once_the_song_list_changed_again() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        std::fs::create_dir_all(&d.home).unwrap();
        let run = Run::begin(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "after").unwrap();
        run.finish(&d.home).unwrap();
        std::fs::write(d.home.join(MANIFEST), "edited by hand").unwrap();
        let e = undo(&d, &mut Vec::new()).unwrap_err();
        assert!(format!("{e:#}").contains("changed after"), "{e:#}");
    }

    #[test]
    fn only_the_latest_runs_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        std::fs::create_dir_all(&d.home).unwrap();
        for n in 0..KEPT_RUNS + 2 {
            let run = Run::begin(&d.home).unwrap();
            std::fs::write(d.home.join(MANIFEST), n.to_string()).unwrap();
            run.finish(&d.home).unwrap();
        }
        assert_eq!(runs(&d.home).unwrap().len(), KEPT_RUNS);
    }
}
