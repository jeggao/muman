//! Which library files are muman's: a file at a song's path it did not
//! write, one changed since it wrote it, and the outputs that stay its
//! when the library folder moves or a copy gives them new times.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::library::{Ledger, Owned};
use crate::resolve::Plan;
use crate::state::{Sidecar, State, Written};
use crate::store;

/// The files at a song's path, its audio or one beside it `plan` writes
/// or its last build left, that muman did not write: a file of the
/// user's own a sync must not write over or remove, a `.lrc` beside a song
/// muman wrote among them, one muman wrote beside a song and the user
/// changed since, and a link, whether or not it leads anywhere. A file
/// that is one muman wrote under a name differing only in case, as a
/// filesystem blind to case reports, is muman's.
pub(super) fn foreign(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    path: &Path,
    plan: &Plan,
) -> Vec<PathBuf> {
    use crate::relpath::folded;
    let owned = |rel: &Path| {
        let fold = folded(rel);
        let is = |o: &Path| {
            o == rel
                || (folded(o) == fold
                    && same_file::is_same_file(library.join(o), library.join(rel)).unwrap_or(false))
        };
        old.iter().any(|(p, w)| {
            is(p)
                || w.sidecars
                    .iter()
                    .any(|s| is(&s.path) && sidecar_as_written(library, s))
        })
    };
    std::iter::once(path.to_path_buf())
        .chain(crate::artifact::parts(plan).touched(path, old.get(path)))
        .filter(|rel| std::fs::symlink_metadata(library.join(rel)).is_ok() && !owned(rel))
        .collect()
}

/// Whether `rel`, a [`foreign`] file, is one muman wrote, a song's or
/// one beside it, that the user changed since: said so, not as a file
/// muman never wrote.
pub(super) fn changed_since_muman(old: &BTreeMap<PathBuf, Written>, rel: &Path) -> bool {
    old.iter()
        .any(|(p, w)| p == rel || w.sidecars.iter().any(|s| s.path == rel))
}

/// [`foreign`] files that keep the song from being written: all but its
/// folder's, which a song leaves alone and is written beside
/// ([`crate::folder`]).
pub(super) fn blocking(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    path: &Path,
    plan: &Plan,
) -> Vec<PathBuf> {
    let folder = crate::artifact::parts(plan).folder_files(path);
    foreign(library, old, path, plan)
        .into_iter()
        .filter(|f| !folder.contains(f))
        .collect()
}

/// A file beside the song's file at `from`, written by muman and changed
/// by the user since, that moving the song to `to` as `plan` makes it
/// would write over or remove: one whose part `plan` changes or drops. A
/// move carries it with the song, so it is in the song's way at `to` as
/// a changed file already there would be; one a move leaves as it is, as
/// a new path template moves it, is not, nor a folder's file a move
/// leaves in its folder, one of `filled` ([`stays`]).
pub(super) fn carried_edit(
    library: &Path,
    (old, filled): (&BTreeMap<PathBuf, Written>, &BTreeSet<String>),
    (from, to): (&Path, &Path),
    plan: &Plan,
) -> Option<PathBuf> {
    let written = old.get(from)?;
    let now = crate::artifact::parts(plan);
    let was = written.plan.as_ref().map(crate::artifact::parts);
    let kept: Vec<PathBuf> = now
        .beside
        .iter()
        .filter(|b| was.as_ref().is_some_and(|p| p.beside.contains(b)))
        .filter_map(|b| b.path(to))
        .collect();
    written
        .sidecars
        .iter()
        .filter(|s| std::fs::symlink_metadata(library.join(&s.path)).is_ok())
        .filter(|s| !sidecar_as_written(library, s) && !stays(library, filled, from, s))
        .find(|s| !kept.contains(&crate::outputs::moved(from, to, &s.path)))
        .map(|s| s.path.clone())
}

/// Whether `sidecar`, beside the song's file at `from`, stays where it is
/// when the song moves: a folder's file the user changed since muman
/// wrote it, in one of the folders `filled`, folded, that songs are in
/// once the sync is done ([`crate::folder`]).
pub(super) fn stays(
    library: &Path,
    filled: &BTreeSet<String>,
    from: &Path,
    sidecar: &Sidecar,
) -> bool {
    let folder = sidecar.path.parent().unwrap_or(Path::new(""));
    crate::outputs::of_folder(from, &sidecar.path)
        && filled.contains(&crate::relpath::folded(folder))
        && !sidecar_as_written(library, sidecar)
}

/// The files beside the song's file at `from`, as `written` records
/// them, that moving it to `to` carries, each from and to: all but those
/// that [`stay`](stays) in a folder of `filled`.
pub(super) fn carried(
    library: &Path,
    filled: &BTreeSet<String>,
    written: &Written,
    (from, to): (&Path, &Path),
) -> Vec<(PathBuf, PathBuf)> {
    written
        .sidecars
        .iter()
        .filter(|s| !stays(library, filled, from, s))
        .map(|s| (s.path.clone(), crate::outputs::moved(from, to, &s.path)))
        .collect()
}

/// The files at a song's paths, its audio and those beside it, that
/// writing it may write over or remove, each to be kept for `undo` first:
/// muman's, under its name or one a filesystem blind to case takes for it,
/// and, when `forced`, one of the user's own, taken over, but never a
/// folder's of the user's own.
pub(super) fn claim(
    ledger: &Ledger<'_>,
    old: &BTreeMap<PathBuf, Written>,
    path: &Path,
    plan: &Plan,
    forced: bool,
) -> Vec<Owned> {
    let library = ledger.root();
    let foreign = foreign(library, old, path, plan);
    let made = crate::artifact::parts(plan);
    let folder = made.folder_files(path);
    std::iter::once(path.to_path_buf())
        .chain(made.touched(path, old.get(path)))
        .filter(|rel| std::fs::symlink_metadata(library.join(rel)).is_ok())
        .filter(|rel| !foreign.contains(rel) || (forced && !folder.contains(rel)))
        .map(|rel| {
            ledger
                .owned(old, &rel, true)
                .unwrap_or_else(|| Ledger::taken(&rel))
        })
        .collect()
}

/// Whether `file` is empty, as a disk that filled up leaves one: it holds
/// no one's work, so it counts as unchanged since written and a sync
/// writes it again, as `check` says.
pub(super) fn emptied(file: &Path) -> bool {
    std::fs::metadata(file).is_ok_and(|m| m.len() == 0)
}

/// Whether a file muman wrote has changed since, by its size and
/// time: a tagger or player that rewrote it.
#[must_use]
pub fn changed_since_written(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    path: &Path,
) -> bool {
    if emptied(&library.join(path)) {
        return false;
    }
    old.get(path)
        .and_then(|w| w.stamp.as_ref())
        .zip(store::stamp_text(&library.join(path)))
        .is_some_and(|(was, now)| *was != now)
}

/// [`changed_since_written`], or, its size and time as written, its
/// bytes not the ones written: a tagger set to keep files' times, its
/// edit fitting the tag's padding. The bytes are read only here, for a
/// file about to be written over, moved with new tags or removed, so a
/// sync reads no file it leaves as it is.
#[must_use]
pub fn edited_since_written(library: &Path, old: &BTreeMap<PathBuf, Written>, path: &Path) -> bool {
    if changed_since_written(library, old, path) {
        return true;
    }
    let file = library.join(path);
    let Some(written) = old.get(path).filter(|w| w.stamp.is_some()) else {
        return false;
    };
    written.digest.as_ref().is_some_and(|was| {
        !emptied(&file) && crate::hash::digest_file(&file).is_some_and(|now| now != *was)
    })
}

/// Take each output whose size or time changed since it was written,
/// but whose bytes are the ones written, as muman's still: a backup put
/// back or a copy made without its times. Only those are read.
pub fn vouch_for_copies(library: &Path, state: &mut State) {
    for (path, written) in &mut state.outputs {
        if let AsWritten::Copied(now) = as_written(&library.join(path), written) {
            written.stamp = Some(now);
        }
        for sidecar in &mut written.sidecars {
            if let AsWritten::Copied(now) = sidecar_written(library, sidecar) {
                sidecar.stamp = Some(now);
            }
        }
    }
}

/// Whether a file beside an output is as muman wrote it, by its size and
/// time or its bytes; one an earlier version recorded by its place alone
/// is.
#[must_use]
pub fn sidecar_as_written(library: &Path, sidecar: &Sidecar) -> bool {
    sidecar.stamp.is_none() || !matches!(sidecar_written(library, sidecar), AsWritten::Not)
}

fn sidecar_written(library: &Path, sidecar: &Sidecar) -> AsWritten {
    stamped(
        &library.join(&sidecar.path),
        sidecar.stamp.as_ref(),
        sidecar.digest.as_ref(),
    )
}

/// Whether a file is as muman wrote it.
pub(super) enum AsWritten {
    /// By its size and time.
    Same,
    /// By its bytes, a copy having given it this stamp, new times.
    Copied(String),
    /// Changed, not there, or written before muman recorded how.
    Not,
}

/// Whether `file` is as `written` records muman wrote it.
pub(super) fn as_written(file: &Path, written: &Written) -> AsWritten {
    stamped(file, written.stamp.as_ref(), written.digest.as_ref())
}

/// Whether `file` is as its stamp and digest, as written, say.
fn stamped(file: &Path, stamp: Option<&String>, digest: Option<&String>) -> AsWritten {
    let (Some(was), Some(now)) = (stamp, store::stamp_text(file)) else {
        return AsWritten::Not;
    };
    if now == *was {
        return AsWritten::Same;
    }
    match digest {
        Some(digest) if crate::hash::digest_file(file).as_ref() == Some(digest) => {
            AsWritten::Copied(now)
        }
        _ => AsWritten::Not,
    }
}

/// The outputs of a library folder named anew, as `--library` or the
/// song list names another: each whose file is at its place in `library`
/// as muman wrote it, moved there or copied, stays muman's, re-stamped
/// where a copy gave it new times; the rest are let go of, to be written
/// anew, or left alone where another file is in the way. How many were
/// kept and let go of.
pub fn rehome(outputs: &mut BTreeMap<PathBuf, Written>, library: &Path) -> (usize, usize) {
    let before = outputs.len();
    outputs.retain(
        |path, written| match as_written(&library.join(path), written) {
            AsWritten::Copied(now) => {
                written.stamp = Some(now);
                true
            }
            AsWritten::Same => true,
            AsWritten::Not => false,
        },
    );
    for written in outputs.values_mut() {
        written
            .sidecars
            .retain_mut(|s| match sidecar_written(library, s) {
                AsWritten::Copied(now) => {
                    s.stamp = Some(now);
                    true
                }
                AsWritten::Same => true,
                AsWritten::Not => s.stamp.is_none(),
            });
    }
    (outputs.len(), before - outputs.len())
}

/// Take `library` as the folder the outputs are in: the files there as
/// muman wrote them stay its ([`rehome`]), and those written into another
/// before are no longer muman's to delete.
pub fn adopt<W: Write>(state: &mut State, library: &Path, out: &mut W) -> Result<()> {
    if library.exists() && !library.is_dir() {
        anyhow::bail!("the library {} is a file, not a folder", library.display());
    }
    if let Some(before) = state
        .library
        .clone()
        .filter(|l| !crate::platform::same_path(l, library))
    {
        let (kept, gone) = rehome(&mut state.outputs, library);
        crate::ui::warning(
            out,
            &format!(
                "The library moved from {}: {kept} file(s) here as muman wrote them stay its, \
                 {gone} are written here anew; the files left there are left alone",
                before.display()
            ),
        )?;
    }
    state.library = Some(library.to_path_buf());
    Ok(())
}
