//! The files muman wrote that no song makes now, deleted through the
//! ledger, or left in place as the user's.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::library::{Ledger, Owned};
use crate::source::SourceKey;
use crate::state::Written;

use super::owned::edited_since_written;

/// What becomes of a file `old` lists that the outputs do not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Pruned {
    /// Deleted, with each file beside it that another output takes not.
    Removed(Vec<PathBuf>),
    /// Kept, as a song it came from failed this run.
    KeptForFailed,
    /// Left in place and no longer muman's: changed since written.
    LeftChanged,
}

/// What pruning `old` against `outputs` does to each file, deciding
/// alone, for a sync to carry out and a dry run to show. A `forced` run
/// wrote every song again, so a changed file of a song now written
/// elsewhere goes, kept for `undo` as any file written over is.
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
        let fate = if written.sources.iter().any(|k| failed.contains(k)) {
            Pruned::KeptForFailed
        } else if edited_since_written(library, old, path)
            && !(forced
                && outputs
                    .values()
                    .any(|w| w.sources.iter().any(|k| written.sources.contains(k))))
        {
            Pruned::LeftChanged
        } else {
            // A file beside another output stays, whether the same name or
            // one a filesystem blind to case takes for it.
            let taken = |file: &Path| {
                outputs.values().flat_map(Written::sidecar_paths).any(|l| {
                    l == file
                        || (crate::relpath::folded(l) == crate::relpath::folded(file)
                            && same_file::is_same_file(library.join(l), library.join(file))
                                .unwrap_or(false))
                })
            };
            Pruned::Removed(
                written
                    .files(path)
                    .filter(|file| !taken(file))
                    .map(Path::to_path_buf)
                    .collect(),
            )
        };
        plan.push((path.clone(), fate));
    }
    plan
}

/// Remove the files of each output of `freed`, which no song keeps and
/// a move lands where one is, before the moves: through the ledger, kept
/// for `undo` apart from the files the run writes over
/// ([`Ledger::free`]), each said, and the output let go of. A file beside
/// it the user changed stays, theirs. Returns each file that could not
/// go, held open, where no move may land.
pub(super) fn free<W: Write>(
    ledger: &mut Ledger<'_>,
    freed: &[PathBuf],
    outputs: &mut BTreeMap<PathBuf, Written>,
    out: &mut W,
) -> Result<Vec<PathBuf>> {
    let root = ledger.root();
    let there = |file: &&Path| std::fs::symlink_metadata(root.join(file)).is_ok();
    let files: Vec<Owned> = (freed.iter())
        .filter_map(|path| Some((path, outputs.get(path)?)))
        .flat_map(|(path, written)| written.files(path))
        .filter(there)
        .filter_map(|file| ledger.owned(outputs, file, false))
        .collect();
    let removal = ledger.free(&files)?;
    for file in &removal.gone {
        crate::ui::info(out, &format!("Removed: {}", crate::relpath::show(file)))?;
    }
    for (file, e) in &removal.held {
        crate::ui::warning(
            out,
            &format!(
                "Could not remove {}: {e}; the next sync tries again",
                file.display()
            ),
        )?;
    }
    let stuck: Vec<PathBuf> = removal.held.into_iter().map(|(f, _)| f).collect();
    for path in freed {
        if (outputs.get(path)).is_some_and(|w| !w.files(path).any(|f| stuck.iter().any(|s| s == f)))
        {
            outputs.remove(path);
        }
    }
    Ok(stuck)
}

/// Delete every file `old` lists that `outputs` does not, through the
/// ledger, which keeps each first, unless a song that failed this run
/// made it, which keeps it. A file changed since it was written is left,
/// and no longer muman's. Each file that goes is said, the files beside a
/// song with it. Returns what went.
pub(super) fn prune<W: Write>(
    ledger: &mut Ledger<'_>,
    old: &BTreeMap<PathBuf, Written>,
    outputs: &mut BTreeMap<PathBuf, Written>,
    failed: &BTreeSet<SourceKey>,
    forced: bool,
    out: &mut W,
) -> Result<Vec<PathBuf>> {
    let plan = prune_plan(ledger.root(), old, outputs, failed, forced);
    let there = |file: &PathBuf| std::fs::symlink_metadata(ledger.root().join(file)).is_ok();
    let gone_before: BTreeSet<PathBuf> = plan
        .iter()
        .filter_map(|(_, fate)| match fate {
            Pruned::Removed(files) => Some(files.iter().filter(|f| !there(f)).cloned()),
            _ => None,
        })
        .flatten()
        .collect();
    let doomed: Vec<Owned> = plan
        .iter()
        .filter_map(|(_, fate)| match fate {
            Pruned::Removed(files) => Some(files.iter()),
            _ => None,
        })
        .flatten()
        .filter_map(|file| ledger.owned(old, file, forced))
        .collect();
    let removal = ledger.remove(&doomed)?;
    for (path, fate) in plan {
        let files = match fate {
            Pruned::KeptForFailed => {
                outputs.insert(path.clone(), old[&path].clone());
                continue;
            }
            Pruned::LeftChanged => {
                crate::ui::warning(
                    out,
                    &format!(
                        "Left in place, changed since muman wrote it: {}; it is yours now",
                        crate::relpath::show(&path)
                    ),
                )?;
                continue;
            }
            Pruned::Removed(files) => files,
        };
        let stuck: Vec<&(PathBuf, String)> = removal
            .held
            .iter()
            .filter(|(f, _)| files.contains(f))
            .collect();
        if stuck.is_empty() {
            // A file deleted by hand is only let go of, not said removed.
            for file in files.iter().filter(|f| removal.gone.contains(f)) {
                if !gone_before.contains(file) {
                    crate::ui::info(out, &format!("Removed: {}", crate::relpath::show(file)))?;
                }
            }
            continue;
        }
        // One file held open must not stop the run before the state is
        // saved; it stays listed and goes next time.
        for (file, e) in stuck {
            crate::ui::warning(
                out,
                &format!(
                    "Could not remove {}: {e}; the next sync tries again",
                    file.display()
                ),
            )?;
        }
        outputs.insert(path.clone(), old[&path].clone());
    }
    Ok(removal.gone)
}
