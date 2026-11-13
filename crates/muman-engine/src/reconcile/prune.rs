//! The files muman wrote that no song makes now, deleted through the
//! ledger, or left in place as the user's.
//!
//! What becomes of each file of an output no song keeps is decided in one
//! place, [`removal`], over the outputs as the sync's intent leaves them
//! ([`pruned`](super::decide::pruned)), and every message about it is
//! made from that decision: `status` says it, `remove` lists it before
//! asking, `check` reads it, and the sync's freeing and pruning carry it
//! out and say what they did. A file there that is muman's goes; one
//! beside the song the user changed since muman wrote it stays, theirs,
//! said so; one not there, or one a song kept or written has, as a
//! folder's file the next song of its folder takes, is only let go of,
//! said by no one.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::library::{Ledger, Owned, Removal};
use crate::source::SourceKey;
use crate::state::Written;

use super::owned::{edited_since_written, sidecar_as_written};

/// What becomes of an output no song keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pruned {
    /// Its files there go, `gone`, but each beside it the user changed
    /// since muman wrote it, `left` in place as theirs.
    Removed {
        gone: Vec<PathBuf>,
        left: Vec<PathBuf>,
    },
    /// Kept, as a song it came from failed this run.
    KeptForFailed,
    /// Left in place and no longer muman's: changed since written.
    LeftChanged,
}

impl Pruned {
    /// The files beside it it leaves in place as the user's.
    #[must_use]
    pub fn left(&self) -> &[PathBuf] {
        match self {
            Self::Removed { left, .. } => left,
            _ => &[],
        }
    }

    /// The files it removes.
    #[must_use]
    pub fn gone(&self) -> &[PathBuf] {
        match self {
            Self::Removed { gone, .. } => gone,
            _ => &[],
        }
    }
}

/// What removing `written`, the output at `path`, does with each of its
/// files, its own not changed since written: each there goes, but one
/// beside it the user changed since, which stays theirs unless
/// `rewritten`, its song written again elsewhere by a forced run, and
/// one `kept`, which another output takes.
pub(super) fn removal(
    library: &Path,
    (path, written): (&Path, &Written),
    rewritten: bool,
    kept: impl Fn(&Path) -> bool,
) -> Pruned {
    let there = |file: &Path| std::fs::symlink_metadata(library.join(file)).is_ok();
    let mut gone = Vec::new();
    let mut left = Vec::new();
    if there(path) && !kept(path) {
        gone.push(path.to_path_buf());
    }
    for sidecar in (written.sidecars.iter()).filter(|s| there(&s.path) && !kept(&s.path)) {
        if rewritten || sidecar_as_written(library, sidecar) {
            gone.push(sidecar.path.clone());
        } else {
            left.push(sidecar.path.clone());
        }
    }
    Pruned::Removed { gone, left }
}

/// What pruning `old` against `outputs` does to each file, deciding
/// alone, for a sync to carry out and a dry run to show. A `forced` run
/// wrote every song again, so a changed file of a song now written
/// elsewhere goes, kept for `undo` as any file written over is: a song
/// written from one of its own sources, as [`files_of`] tells a song's
/// file, never one it only shares with others, as a picture of the
/// user's given to an album.
///
/// [`files_of`]: super::decide::files_of
pub(super) fn prune_plan(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    outputs: &BTreeMap<PathBuf, Written>,
    failed: &BTreeSet<SourceKey>,
    forced: bool,
) -> Vec<(PathBuf, Pruned)> {
    let now: BTreeMap<String, PathBuf> = outputs
        .keys()
        .map(|p| (crate::relpath::folded(p), p.clone()))
        .collect();
    // A file beside another output stays, whether the same name or one a
    // filesystem blind to case takes for it.
    let taken = |file: &Path| {
        outputs.values().flat_map(Written::sidecar_paths).any(|l| {
            l == file
                || (crate::relpath::folded(l) == crate::relpath::folded(file)
                    && same_file::is_same_file(library.join(l), library.join(file))
                        .unwrap_or(false))
        })
    };
    let mut plan = Vec::new();
    for (path, written) in old {
        // On a filesystem blind to case and composition, a song renamed
        // only so is the same file as its new name: deleting the old name
        // would delete the new song. Elsewhere they are two files.
        let renamed = now.get(&crate::relpath::folded(path)).is_some_and(|new| {
            same_file::is_same_file(library.join(path), library.join(new)).unwrap_or(false)
        });
        if outputs.contains_key(path) || renamed {
            continue;
        }
        let rewritten = forced
            && outputs.values().any(|w| {
                (w.sources.iter())
                    .any(|k| !crate::manifest::shareable(k) && written.sources.contains(k))
            });
        let fate = if written.sources.iter().any(|k| failed.contains(k)) {
            Pruned::KeptForFailed
        } else if edited_since_written(library, old, path) && !rewritten {
            Pruned::LeftChanged
        } else {
            removal(library, (path, written), rewritten, taken)
        };
        plan.push((path.clone(), fate));
    }
    plan
}

/// Say what pruning or freeing as `fate` did, by `removal`: each file
/// that went, each beside it the user changed left in place, and each
/// that could not go, why. Whether the output is let go of: not while one
/// of its files is held open, which the next sync removes.
fn say<W: Write>(fate: &Pruned, removal: &Removal, out: &mut W) -> Result<bool> {
    let stuck: Vec<&(PathBuf, String)> = (removal.held.iter())
        .filter(|(f, _)| fate.gone().contains(f))
        .collect();
    for file in fate.gone().iter().filter(|f| removal.gone.contains(f)) {
        crate::ui::info(out, &format!("Removed: {}", crate::relpath::show(file)))?;
    }
    for (file, e) in &stuck {
        crate::ui::warning(
            out,
            &format!(
                "Could not remove {}: {e}; the next sync tries again",
                crate::relpath::show(file)
            ),
        )?;
    }
    if !stuck.is_empty() {
        return Ok(false);
    }
    for file in fate.left() {
        crate::ui::warning(
            out,
            &format!(
                "Left in place, changed since muman wrote it: {}; it is yours now",
                crate::relpath::show(file)
            ),
        )?;
    }
    Ok(true)
}

/// Remove the files of each output of `freed`, which no song keeps and
/// a move lands where one is, before the moves: through the ledger, kept
/// for `undo` apart from the files the run writes over
/// ([`Ledger::free`]), each said, and the output let go of. A file beside
/// it the user changed stays, theirs; a folder's file of `handed` stays
/// too, recorded beside the output it names, whose song writes it over.
/// Returns each file that could not go, held open, where no move may
/// land.
pub(super) fn free<W: Write>(
    ledger: &mut Ledger<'_>,
    (freed, handed): (&[PathBuf], &BTreeMap<PathBuf, PathBuf>),
    outputs: &mut BTreeMap<PathBuf, Written>,
    out: &mut W,
) -> Result<Vec<PathBuf>> {
    let root = ledger.root();
    let handing: BTreeMap<&Path, &PathBuf> = (handed.iter())
        .filter(|(_, to)| outputs.contains_key(*to))
        .map(|(file, to)| (file.as_path(), to))
        .collect();
    let plan: Vec<(&PathBuf, Pruned)> = (freed.iter())
        .filter_map(|path| {
            let written = outputs.get(path)?;
            let fate = removal(root, (path, written), false, |f| handing.contains_key(f));
            Some((path, fate))
        })
        .collect();
    let files: Vec<Owned> = (plan.iter())
        .flat_map(|(_, fate)| fate.gone())
        .filter_map(|file| ledger.owned(outputs, file, false))
        .collect();
    for (file, to) in handing {
        let sidecar = freed.iter().find_map(|path| {
            let sidecars = &mut outputs.get_mut(path)?.sidecars;
            let at = sidecars.iter().position(|s| s.path == *file)?;
            Some(sidecars.remove(at))
        });
        if let (Some(sidecar), Some(holder)) = (sidecar, outputs.get_mut(to)) {
            holder.sidecars.push(sidecar);
        }
    }
    let removal = ledger.free(&files)?;
    for (path, fate) in &plan {
        if say(fate, &removal, out)? {
            outputs.remove(*path);
        }
    }
    Ok(removal.held.into_iter().map(|(f, _)| f).collect())
}

/// Delete every file `old` lists that `outputs` does not, through the
/// ledger, which keeps each first, unless a song that failed this run
/// made it, which keeps it. A file changed since it was written is left,
/// and no longer muman's. Each file that goes is said, the files beside a
/// song with it, and each beside it left as the user's. Returns what
/// went.
pub(super) fn prune<W: Write>(
    ledger: &mut Ledger<'_>,
    old: &BTreeMap<PathBuf, Written>,
    outputs: &mut BTreeMap<PathBuf, Written>,
    failed: &BTreeSet<SourceKey>,
    forced: bool,
    out: &mut W,
) -> Result<Vec<PathBuf>> {
    let plan = prune_plan(ledger.root(), old, outputs, failed, forced);
    let doomed: Vec<Owned> = (plan.iter())
        .flat_map(|(_, fate)| fate.gone())
        .filter_map(|file| ledger.owned(old, file, forced))
        .collect();
    let removal = ledger.remove(&doomed)?;
    for (path, fate) in plan {
        match fate {
            Pruned::KeptForFailed => {
                outputs.insert(path.clone(), old[&path].clone());
            }
            Pruned::LeftChanged => crate::ui::warning(
                out,
                &format!(
                    "Left in place, changed since muman wrote it: {}; it is yours now",
                    crate::relpath::show(&path)
                ),
            )?,
            Pruned::Removed { .. } => {
                if !say(&fate, &removal, out)? {
                    outputs.insert(path.clone(), old[&path].clone());
                }
            }
        }
    }
    Ok(removal.gone)
}
