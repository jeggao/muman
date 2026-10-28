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

/// Delete every file `old` lists that `outputs` does not, through the
/// ledger, which keeps each first, unless a song that failed this run
/// made it, which keeps it. A file changed since it was written is left,
/// and no longer muman's. Returns what went.
pub(super) fn prune<W: Write>(
    ledger: &mut Ledger<'_>,
    old: &BTreeMap<PathBuf, Written>,
    outputs: &mut BTreeMap<PathBuf, Written>,
    failed: &BTreeSet<SourceKey>,
    forced: bool,
    out: &mut W,
) -> Result<Vec<PathBuf>> {
    let plan = prune_plan(ledger.root(), old, outputs, failed, forced);
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
            crate::ui::info(out, &format!("Removed: {}", crate::relpath::show(&path)))?;
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
