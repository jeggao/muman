//! Which library files are muman's: a file at a song's path it did not
//! write, one changed since it wrote it, and the outputs that stay its
//! when the library folder moves or a copy gives them new times.
//!
//! Every command that judges a library file, a sync, its dry run, `check`,
//! `info`, `export` and a query's commands, `remove`'s listing among them,
//! reads the state as the library is found ([`as_found`]), in one order:
//! the moves a stopped run made followed, a library folder named anew
//! taken ([`adopt`]), then each file a copy gave new times, its bytes as
//! written, vouched for ([`vouch_for_copies`]). Only then is any file
//! judged, by its plan or by its stamp, so a library put back from a
//! backup without its times is planned, pruned and listed as the sync
//! after it writes it, and no step decides on stamps another step then
//! finds stale.
//!
//! The records of a library folder muman was pointed away from are kept
//! while it is not there, as a drive unplugged or a share not mounted, so
//! pointed back at it, its files as muman wrote them are muman's again
//! ([`rehome`]). A folder that is there but holds no file, as the mount
//! point of a share not mounted, is as good as not there: it is no sign
//! the files are gone. A folder's records go when the folder holds files
//! and none of those they name, or past the [`LEFT_KEPT`] folders muman
//! wrote into last, so the state does not grow with every folder it was
//! ever pointed at.

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
/// leaves in its folder ([`stays`]).
pub(super) fn carried_edit(
    library: &Path,
    (old, filled): (&BTreeMap<PathBuf, Written>, &BTreeSet<String>),
    (from, to): (&Path, &Path),
    plan: &Plan,
) -> Option<PathBuf> {
    let written = old.get(from)?;
    let changed = written.plan.as_ref() != Some(plan);
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
        .filter(|s| !sidecar_as_written(library, s) && !stays(library, (filled, changed), from, s))
        .find(|s| !kept.contains(&crate::outputs::moved(from, to, &s.path)))
        .map(|s| s.path.clone())
}

/// Whether `sidecar`, beside the song's file at `from`, stays where it is
/// when the song moves: a folder's file the user changed since muman
/// wrote it, when the song moves for a change to it (`changed`, its plan
/// not the one its file was written from), or when it is in one of the
/// folders `filled`, folded, that songs are in once the sync is done
/// ([`crate::folder`]). Only a new path template alone carries it off,
/// with every song of its folder.
pub(super) fn stays(
    library: &Path,
    (filled, changed): (&BTreeSet<String>, bool),
    from: &Path,
    sidecar: &Sidecar,
) -> bool {
    let folder = sidecar.path.parent().unwrap_or(Path::new(""));
    crate::outputs::of_folder(from, &sidecar.path)
        && (changed || filled.contains(&crate::relpath::folded(folder)))
        && !sidecar_as_written(library, sidecar)
}

/// The files beside the song's file at `from`, as `written` records
/// them, that moving it to `to` carries, each from and to: all but those
/// gone, which are written anew where it goes, those that [`stay`](stays)
/// in a folder of `filled`, and a folder's file `plan` does not make where
/// it goes ([`crate::folder`]).
pub(super) fn carried(
    library: &Path,
    filled: &BTreeSet<String>,
    written: &Written,
    (from, to): (&Path, &Path),
    plan: &Plan,
) -> Vec<(PathBuf, PathBuf)> {
    let made = crate::artifact::parts(plan).files(to);
    let changed = written.plan.as_ref() != Some(plan);
    written
        .sidecars
        .iter()
        .filter(|s| std::fs::symlink_metadata(library.join(&s.path)).is_ok())
        .filter(|s| !stays(library, (filled, changed), from, s))
        .map(|s| (s.path.clone(), crate::outputs::moved(from, to, &s.path)))
        .filter(|(at, to)| !crate::outputs::of_folder(from, at) || made.contains(to))
        .collect()
}

/// The files at a song's paths, its audio and those beside it, that
/// writing it may write over or remove, each to be kept for `undo` first:
/// muman's, under its name or one a filesystem blind to case takes for it,
/// and, when `forced`, one of the user's own, taken over, but never a
/// folder's muman never wrote. A folder's file muman wrote and the user
/// changed since is written again, as a song's own is.
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
        .filter(|rel| {
            !foreign.contains(rel)
                || (forced && (!folder.contains(rel) || changed_since_muman(old, rel)))
        })
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

/// [`sidecar_as_written`] of `sidecar` with its file at `file`: where it
/// has none, only one recorded by its place alone is.
pub(super) fn sidecar_written_at(file: Option<&Path>, sidecar: &Sidecar) -> bool {
    sidecar.stamp.is_none()
        || file.is_some_and(|f| {
            !matches!(
                stamped(f, sidecar.stamp.as_ref(), sidecar.digest.as_ref()),
                AsWritten::Not
            )
        })
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
/// song list names another, from the records of them: `outputs`, those
/// muman wrote last, into whichever folder, and `earlier`, those it wrote
/// into `library` itself before it was pointed away from it. Each file at
/// its place in `library` as one of them says muman wrote it, moved there,
/// copied or left there, stays muman's, re-stamped where a copy gave it
/// new times: by the record under which all its song's files are as
/// written, else the first under which its own file is. The rest are let
/// go of, to be written anew, or left alone where another file is in the
/// way. How many files, a song's and those beside it, were kept and let
/// go of.
pub fn rehome(
    outputs: &mut BTreeMap<PathBuf, Written>,
    earlier: BTreeMap<PathBuf, Written>,
    library: &Path,
) -> (usize, usize) {
    let mut records: BTreeMap<PathBuf, Vec<Written>> = BTreeMap::new();
    for (path, written) in std::mem::take(outputs).into_iter().chain(earlier) {
        records.entry(path).or_default().push(written);
    }
    let files = |w: &Written| 1 + w.sidecars.len();
    let (mut kept, mut gone) = (0, 0);
    for (path, candidates) in records {
        let named = candidates.first().map_or(0, files);
        let mut held: Vec<(Written, bool, usize)> = candidates
            .into_iter()
            .filter_map(|w| {
                let all = files(&w);
                found_at(library, &path, w).map(|(w, whole)| (w, whole, all))
            })
            .collect();
        let whole = held.iter().position(|(_, whole, _)| *whole).unwrap_or(0);
        if held.is_empty() {
            gone += named;
        } else {
            let (written, _, all) = held.swap_remove(whole);
            kept += files(&written);
            gone += all - files(&written);
            outputs.insert(path, written);
        }
    }
    (kept, gone)
}

/// `written`, the record of the output at `path` in `library`, re-stamped
/// as each of its files is found there, and whether all of them are as
/// written; none when the output's own file is not, and without each
/// file beside it that is not.
fn found_at(library: &Path, path: &Path, mut written: Written) -> Option<(Written, bool)> {
    match as_written(&library.join(path), &written) {
        AsWritten::Copied(now) => written.stamp = Some(now),
        AsWritten::Same => {}
        AsWritten::Not => return None,
    }
    let before = written.sidecars.len();
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
    let whole = written.sidecars.len() == before;
    Some((written, whole))
}

/// The records `state` keeps of the outputs muman wrote into `library`
/// before it was pointed away from it, taken out of it for [`rehome`].
fn take_left(state: &mut State, library: &Path) -> BTreeMap<PathBuf, Written> {
    let key = (state.left.keys())
        .find(|l| crate::platform::same_path(l, library))
        .cloned();
    key.and_then(|k| state.left.remove(&k)).unwrap_or_default()
}

/// The most library folders muman was pointed away from whose records
/// the state keeps: a drive or two plugged in by turns, and the folders
/// before them, without a copy of the outputs for every folder it was
/// ever pointed at.
pub const LEFT_KEPT: usize = 4;

/// A library folder named anew, as [`adopt`] took it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adopted {
    /// The folder the outputs were in.
    pub from: PathBuf,
    /// How many files in the new one, as muman wrote them, stay its.
    pub kept: usize,
    /// How many files it recorded are let go of.
    pub gone: usize,
}

impl Adopted {
    /// Warn on `out` where the outputs were and how many files stay muman's.
    pub fn say<W: Write>(&self, out: &mut W) -> Result<()> {
        let Self {
            from: before,
            kept,
            gone,
        } = self;
        crate::ui::warning(
            out,
            &format!(
                "The library moved from {}: {kept} file(s) here as muman wrote them stay its, \
                 {gone} others are let go of; the files left there are left alone",
                before.display()
            ),
        )?;
        Ok(())
    }
}

/// Take `library` as the folder the outputs are in: the files there as
/// muman wrote them stay its ([`rehome`]), those it wrote there before it
/// was pointed away from it among them, and those written into another
/// are no longer muman's to delete, their records kept should it be
/// pointed back at that one, as the module's docs say. What it took, when
/// the folder was named anew.
pub fn adopt(state: &mut State, library: &Path) -> Result<Option<Adopted>> {
    if library.exists() && !library.is_dir() {
        anyhow::bail!("the library {} is a file, not a folder", library.display());
    }
    let before = (state.library.clone()).filter(|l| !crate::platform::same_path(l, library));
    let adopted = before.map(|from| {
        let earlier = take_left(state, library);
        state.left.insert(from.clone(), state.outputs.clone());
        forget_left(&mut state.left);
        let (kept, gone) = rehome(&mut state.outputs, earlier, library);
        Adopted { from, kept, gone }
    });
    state.library = Some(library.to_path_buf());
    Ok(adopted)
}

/// Let go of the records of each folder of `left` that holds files and
/// none they name, and of all but the [`LEFT_KEPT`] folders muman wrote
/// into last, by the newest time a record of each holds. A folder not
/// there keeps its records, as a drive unplugged comes back, and so does
/// one that holds no file, as a share's mount point while it is not
/// mounted.
fn forget_left(left: &mut BTreeMap<PathBuf, BTreeMap<PathBuf, Written>>) {
    left.retain(|folder, outputs| {
        !holds_files(folder)
            || (outputs.iter())
                .flat_map(|(path, w)| w.files(path))
                .any(|file| std::fs::symlink_metadata(folder.join(file)).is_ok())
    });
    let newest = |outputs: &BTreeMap<PathBuf, Written>| {
        (outputs.values())
            .flat_map(|w| std::iter::once(&w.stamp).chain(w.sidecars.iter().map(|s| &s.stamp)))
            .flatten()
            .filter_map(|s| store::stamp_of_rev(s).map(|(_, ns)| ns))
            .max()
            .unwrap_or(0)
    };
    let mut by_age: Vec<(u128, PathBuf)> = (left.iter())
        .map(|(folder, outputs)| (newest(outputs), folder.clone()))
        .collect();
    by_age.sort();
    let over = left.len().saturating_sub(LEFT_KEPT);
    for (_, folder) in by_age.into_iter().take(over) {
        left.remove(&folder);
    }
}

/// Whether `folder`, or a folder in it, holds a file other than a hidden
/// one, as a desktop leaves in any folder it shows: an empty folder is
/// no sign the files once in it are gone.
fn holds_files(folder: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let hidden = entry.file_name().to_string_lossy().starts_with('.');
        match entry.file_type() {
            Ok(t) if t.is_dir() => holds_files(&entry.path()),
            Ok(_) => !hidden,
            Err(_) => true,
        }
    })
}

/// Take the library `dirs` names as it is found, before any of its files
/// is judged, in the order the module's docs give: the moves a stopped run
/// made into it followed ([`crate::history::follow_moves`]), finished in
/// the library only under [`Moves::Follow`](crate::history::Moves::Follow),
/// as a sync alone does, and otherwise each file a move of case alone left
/// at its temporary name counted where it goes ([`State::halfway`]), as
/// that sync finds it; the folder taken as the outputs' ([`adopt`]); and
/// copies with new times vouched for ([`vouch_for_copies`]). The folder
/// the outputs were in, when it was named anew.
pub fn as_found(
    dirs: &crate::dirs::Dirs,
    state: &mut State,
    moves: crate::history::Moves,
) -> Result<Option<Adopted>> {
    let library = &dirs.library;
    if (state.library.as_deref()).is_some_and(|l| crate::platform::same_path(l, library)) {
        if moves == crate::history::Moves::Foresee {
            state.halfway = crate::library::halfway(library, &crate::history::moving(&dirs.home));
        }
        crate::history::follow_moves(&dirs.home, library, state, moves);
    }
    let adopted = adopt(state, library)?;
    vouch_for_copies(library, state);
    Ok(adopted)
}

/// The state in `dirs`' home as the library is found ([`as_found`]), for
/// a command that judges library files and changes none of them.
pub fn found(dirs: &crate::dirs::Dirs) -> Result<State> {
    let mut state = State::load(&dirs.home)?;
    as_found(dirs, &mut state, crate::history::Moves::Foresee)?;
    Ok(state)
}
