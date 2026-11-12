//! What a sync does with each song's file, decided for every song before
//! any is done: kept, moved, tagged again, written, or left alone.
//!
//! The decisions and what follows from them, the moves in their order,
//! the outputs they leave and the files pruned, make one [`Intent`]:
//! `status` shows it and a sync carries it out, so the two cannot say
//! different things. A sync departs from it only where a move or a write
//! fails, and judges pruning again by what was done.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::held::Drift;
use crate::library::Relocation;
use crate::manifest::{Edit, Manifest};
use crate::resolve::{Plan, Resolved};
use crate::source::SourceKey;
use crate::state::{Sidecar, State, Written};

use super::damage::{damaged_since_built, split_drifts};
use super::owned::{blocking, carried, carried_edit, edited_since_written};
use super::plan::{PlannedSong, path_of};
use super::prune::{Pruned, prune_plan};

/// What `old` records of the song's file, when that file is the song as
/// it would be written now: its plan, its file there and not empty, each
/// file beside it there.
pub(super) fn current<'a>(
    library: &Path,
    old: &'a BTreeMap<PathBuf, Written>,
    r: &Resolved,
) -> Option<&'a Written> {
    let path = path_of(r);
    let written = old.get(&path)?;
    let present = own_there(library, &path) && beside_there(library, written, &path, &r.plan);
    (present && written.plan.as_ref() == Some(&r.plan)).then_some(written)
}

/// Whether a song's own file is at `file`, and not empty, as a disk that
/// filled up leaves one: what decides whether the song is there to keep,
/// move or tag again. A file beside it gone is no song gone: it is written
/// again beside the song's file, which is left as it is ([`beside_there`]).
pub(super) fn own_there(library: &Path, file: &Path) -> bool {
    library.join(file).metadata().is_ok_and(|m| m.len() > 0)
}

/// Whether each file beside the song at `path` is there: each `written`
/// records beside it, and each of its folder's files `plan` has it hold.
pub(super) fn beside_there(library: &Path, written: &Written, path: &Path, plan: &Plan) -> bool {
    written.sidecar_paths().all(|s| library.join(s).exists()) && folder_there(library, path, plan)
}

/// Whether each file of its folder `plan` has the song at `path` hold is
/// there, muman's or one of the user's it leaves alone: one gone is
/// written again.
fn folder_there(library: &Path, path: &Path, plan: &Plan) -> bool {
    crate::artifact::parts(plan)
        .folder_files(path)
        .iter()
        .all(|f| std::fs::symlink_metadata(library.join(f)).is_ok())
}

/// Which file the state records is each song's, one file to one song:
/// the one made from the most of the song's own sources, those no other
/// song lists, as the state recorded them when it was written. So a song
/// whose path, plan or sources changed still has its file, and every
/// decision about it, to keep, move, guard, write or delete, is made of
/// that file; a song no file was made from has none. Ties go to the song
/// listed first, then to the path that sorts first.
pub fn files_of(
    songs: impl IntoIterator<Item = usize>,
    manifest: &Manifest,
    old: &BTreeMap<PathBuf, Written>,
) -> BTreeMap<usize, PathBuf> {
    let own = |keys: &[SourceKey]| -> BTreeSet<SourceKey> {
        keys.iter()
            .filter(|k| !crate::manifest::shareable(k))
            .cloned()
            .collect()
    };
    let mut scored: Vec<(usize, usize, &PathBuf)> = Vec::new();
    for n in songs {
        let mine = own(&manifest.songs[n].sources);
        for (path, written) in old {
            let shared = own(&written.sources).intersection(&mine).count();
            if shared > 0 {
                scored.push((shared, n, path));
            }
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(b.2)));
    let mut files = BTreeMap::new();
    let mut taken: BTreeSet<&PathBuf> = BTreeSet::new();
    for (_, n, path) in scored {
        if !files.contains_key(&n) && taken.insert(path) {
            files.insert(n, path.clone());
        }
    }
    files
}

/// What a sync does with one song, decided for every song before any is
/// done, so a sync does what `status` says it would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Doing {
    /// Its file is as it would be written.
    Keep,
    /// Its file moves to the song's path with the files `beside` it it
    /// carries, from and to, its tags written again when they changed or
    /// a file it leaves is written anew; moves are made by `order`, as a
    /// move into a path another move leaves runs after it.
    Move {
        from: PathBuf,
        beside: Vec<(PathBuf, PathBuf)>,
        retag: bool,
        order: usize,
    },
    /// Only its tags changed, or a file beside it is gone: they are
    /// written into its file, and that file beside it written again.
    Retag,
    /// Written from its sources.
    Write,
    /// Left alone: its file, changed since muman wrote it.
    Guard { file: PathBuf },
    /// Left alone: a file not muman's, `by`, is at its path or its
    /// lyrics': one muman wrote and the user `edited` since, or another.
    Unowned { by: PathBuf, edited: bool },
    /// Left where it is: a file not muman's, `by`, is at the song's path
    /// or its lyrics': one muman wrote and the user `edited` since, or
    /// another.
    Blocked {
        file: PathBuf,
        by: PathBuf,
        edited: bool,
    },
    /// Left as built: a source it was built from was fetched again and
    /// holds otherwise, not yet accepted, or holds damaged audio since.
    /// With no file built before left, a song of damaged audio is not
    /// written, and one of a source fetched again is.
    Held { file: Option<PathBuf> },
}

/// What a sync decides for every song, before it does any: each song's
/// file, the sources holding otherwise than it was built from, the songs
/// held as built for them and those of them damaged since, and what it
/// does with each.
#[derive(Debug)]
pub struct Foreseen {
    pub files: BTreeMap<usize, PathBuf>,
    pub drifts: Vec<Drift>,
    /// The songs held as built, never built from what a source holds
    /// now: those a source fetched again holds otherwise for, not yet
    /// accepted, and those of audio damaged since.
    pub held: BTreeSet<usize>,
    pub damaged: BTreeSet<usize>,
    pub decided: Vec<Doing>,
    /// The files no song keeps that the sync prunes, which a move may land
    /// where one is ([`freeable`]).
    pub freeable: BTreeSet<PathBuf>,
}

/// What a sync of `planned` into `library` decides, `force` and `accept`
/// as given: the one place `sync`, `status` and `info` decide it.
#[must_use]
pub fn foresee(
    manifest: &Manifest,
    state: &State,
    planned: &[PlannedSong],
    library: &Path,
    asked: (bool, bool),
) -> Foreseen {
    foresee_failing(manifest, state, planned, library, asked, &BTreeSet::new())
}

/// [`foresee`], the sources of `failed` failing the sync, whose files it
/// keeps.
pub(super) fn foresee_failing(
    manifest: &Manifest,
    state: &State,
    planned: &[PlannedSong],
    library: &Path,
    (force, accept): (bool, bool),
    failed: &BTreeSet<SourceKey>,
) -> Foreseen {
    let files = files_of(planned.iter().map(|(n, _)| *n), manifest, &state.outputs);
    let (drifts, holding, damaged) = holding(manifest, state, planned, (force, accept));
    let freeable = freeable(manifest, planned, &files, (&state.outputs, failed), library);
    let decided = decide(
        planned,
        &files,
        (&state.outputs, &freeable),
        library,
        force,
        (&holding, &damaged),
    );
    Foreseen {
        files,
        drifts,
        held: holding.union(&damaged).copied().collect(),
        damaged,
        decided,
        freeable,
    }
}

/// The outputs of `old` no song keeps that a sync prunes, before it is
/// done, in the way of a file a song of `planned` has: each no song has
/// for its file ([`files_of`]), none of whose own sources a song planned
/// lists or whose sources `failed`, as a song that fails keeps its file,
/// and not changed since written, which stays the user's, as only a file
/// in the way is read to tell. A song may move where one of them is, as
/// where another song moves away from: it is removed first, kept for
/// `undo` ([`crate::library::Ledger::free`]).
fn freeable(
    manifest: &Manifest,
    planned: &[PlannedSong],
    files: &BTreeMap<usize, PathBuf>,
    (old, failed): (&BTreeMap<PathBuf, Written>, &BTreeSet<SourceKey>),
    library: &Path,
) -> BTreeSet<PathBuf> {
    let theirs: BTreeSet<&PathBuf> = files.values().collect();
    let listed: BTreeSet<&SourceKey> = (planned.iter())
        .flat_map(|(n, _)| &manifest.songs[*n].sources)
        .filter(|k| !crate::manifest::shareable(k))
        .collect();
    let wanted: Vec<PathBuf> = (planned.iter())
        .flat_map(|(_, r)| {
            let path = path_of(r);
            let beside = crate::artifact::parts(&r.plan).files(&path);
            std::iter::once(path).chain(beside)
        })
        .collect();
    let landing = Landing::of(wanted.iter());
    old.iter()
        .filter(|(p, w)| !theirs.contains(p) && w.files(p).any(|f| landing.blocks(f)))
        .filter(|(_, w)| {
            !w.sources
                .iter()
                .any(|k| failed.contains(k) || listed.contains(k))
        })
        .filter(|(p, _)| !edited_since_written(library, old, p))
        .map(|(p, _)| p.clone())
        .collect()
}

/// Where files land, as moves or songs written: each a file of another
/// name is in the way of, folded as NTFS and APFS fold names, and each
/// folder they land in.
struct Landing {
    landed: BTreeSet<String>,
    under: BTreeSet<String>,
}

impl Landing {
    fn of<'a>(paths: impl Iterator<Item = &'a PathBuf>) -> Self {
        use crate::relpath::folded;
        let paths: Vec<&PathBuf> = paths.collect();
        Self {
            landed: paths.iter().map(|p| folded(p)).collect(),
            under: (paths.iter())
                .flat_map(|p| p.ancestors())
                .map(folded)
                .collect(),
        }
    }

    /// Whether a file at `f` is in the way: at a path that lands, at a
    /// folder of one, or under one, as a file is in the way of a folder of
    /// its name.
    fn blocks(&self, f: &Path) -> bool {
        use crate::relpath::folded;
        self.under.contains(&folded(f)) || f.ancestors().any(|a| self.landed.contains(&folded(a)))
    }
}

/// The files of `written`, the output at `path`, that freeing its path
/// removes: its own, and each beside it as muman wrote it; one the user
/// changed stays theirs, in the way of a move as any file of theirs.
fn freed_files<'a>(
    library: &'a Path,
    path: &'a Path,
    written: &'a Written,
) -> impl Iterator<Item = &'a Path> {
    std::iter::once(path).chain(
        (written.sidecars.iter())
            .filter(|s| super::owned::sidecar_as_written(library, s))
            .map(|s| s.path.as_path()),
    )
}

/// The sources holding otherwise than their songs were built from, and
/// of `planned` the songs held as built for them, not yet accepted, and
/// those of audio damaged since, `force` and `accept` as given.
fn holding(
    manifest: &Manifest,
    state: &State,
    planned: &[PlannedSong],
    (force, accept): (bool, bool),
) -> (Vec<Drift>, BTreeSet<usize>, BTreeSet<usize>) {
    let drifts = crate::held::drifts(manifest, &state.facts);
    let damaged = if force {
        BTreeSet::new()
    } else {
        damaged_since_built(planned, &drifts, &state.facts)
    };
    let (_, kept) = split_drifts(&drifts, &damaged, accept, state);
    let holding = kept.iter().map(|d| d.song).collect();
    (drifts, holding, damaged)
}

/// Each song of `planned` a sync leaves as built, [`Foreseen::held`] as
/// `foresee` holds it, with its file still in `library`: what a file of
/// it holds, not what its sources hold now, is what it weighs in its
/// album, its folder, the names songs take and the library's size. None
/// is left under `force`, which writes every song.
#[must_use]
pub fn left_as_built(
    manifest: &Manifest,
    state: &State,
    planned: &[PlannedSong],
    library: &Path,
    (force, accept): (bool, bool),
) -> BTreeMap<usize, PathBuf> {
    if force {
        return BTreeMap::new();
    }
    let (_, holding, damaged) = holding(manifest, state, planned, (force, accept));
    let files = files_of(planned.iter().map(|(n, _)| *n), manifest, &state.outputs);
    (holding.union(&damaged))
        .filter_map(|n| Some((*n, files.get(n)?.clone())))
        .filter(|(_, file)| library.join(file).exists())
        .collect()
}

/// Each song of `planned` a sync does not write for a file of the
/// user's, its own changed since muman wrote it or one in its way
/// ([`Doing::Guard`], [`Doing::Blocked`], [`Doing::Unowned`]), with its
/// own file where one is left: what it weighs in its folder is that file,
/// as a song left as built, and one with none holds no file there; the
/// folders' files are given again so.
#[must_use]
pub fn left_in_place(
    manifest: &Manifest,
    state: &State,
    planned: &[PlannedSong],
    library: &Path,
    asked: (bool, bool),
) -> BTreeMap<usize, Option<PathBuf>> {
    let foreseen = foresee(manifest, state, planned, library, asked);
    let own = |n: &usize| {
        (foreseen.files.get(n))
            .filter(|f| library.join(f).exists())
            .cloned()
    };
    (planned.iter().zip(&foreseen.decided))
        .filter_map(|((n, _), d)| match d {
            Doing::Guard { file } | Doing::Blocked { file, .. } => {
                Some((*n, own(n).filter(|f| f == file)))
            }
            Doing::Unowned { .. } => Some((*n, own(n))),
            _ => None,
        })
        .collect()
}

/// Each of `planned` kept as it is, decided before any moves: its file
/// as it would be written, or one built before from what a source no
/// longer holds, of the songs `holding` or `damaged` since built. A song
/// `damaged` is held with no file left too: what its record names is
/// not what its source holds, and only `--force` builds it from that.
pub(super) fn settled(
    planned: &[PlannedSong],
    files: &BTreeMap<usize, PathBuf>,
    old: &BTreeMap<PathBuf, Written>,
    library: &Path,
    (holding, damaged): Holding<'_>,
) -> Vec<Option<Doing>> {
    planned
        .iter()
        .map(|(n, r)| {
            if current(library, old, r).is_some() {
                return Some(Doing::Keep);
            }
            let file = files.get(n).filter(|f| library.join(f).exists());
            if damaged.contains(n) {
                return Some(Doing::Held {
                    file: file.cloned(),
                });
            }
            file.filter(|_| holding.contains(n)).map(|f| Doing::Held {
                file: Some(f.clone()),
            })
        })
        .collect()
}

/// The songs held as built: those a source fetched again holds otherwise
/// for, not yet accepted, and those of audio damaged since.
pub(super) type Holding<'a> = (&'a BTreeSet<usize>, &'a BTreeSet<usize>);

/// What a sync does with each of `planned`, by its file in `files`, as
/// [`settled`] first keeps or holds it:
///
/// | The song's file | Done |
/// |---|---|
/// | At its path, as it would be written | Kept |
/// | Built from what a source in `holding` no longer holds | Left as built until `--accept` |
/// | Any or none, its audio's source `damaged` since it was built | Left as built, or unwritten |
/// | Elsewhere, of the same media, its path free | Moved, then tags written if they changed |
/// | Changed since written, and not merely moving | Left alone until `--force` |
/// | Any, where a changed file is at its path | Left alone until `--force` |
/// | Elsewhere, beside a changed file a move would write again | Left where it is until `--force` |
/// | At its path, of the same media | Its tags written, and each file beside it that differs or is gone |
/// | Any, and a file not muman's at its path or its lyrics' | Left where it is until `--force` |
/// | None, and a file not muman's at its path or its lyrics' | Left alone until `--force` |
/// | Otherwise, or `force` | Written |
///
/// A path is free when no file of anyone's is there, folded as NTFS and
/// APFS fold names, nor a song's file where one of its folders goes, as
/// `Y.flac` is the folder of `Y.flac/Song.flac`, or only a song's file
/// that moves away first, or a folder all of whose songs' files do, as
/// when
/// two songs of one name become two of two names and the one told apart
/// takes the plain name; a name changing only its case is free to its
/// own file. Moves are listed in the order they can be made;
/// two songs swapping paths are written instead. A changed file moves
/// only when its tags need no writing, so nothing a tagger wrote is
/// written over, nor a file beside it the user changed that its new plan
/// would write again: the move would carry that file to where the song
/// cannot write it ([`carried_edit`]). A folder's file the user changed
/// is not carried, and keeps no song where it is
/// ([`stays`](super::owned::stays)), when its song moves for a change to
/// it, or while a song is left in its folder: decided again, with each
/// folder a song is left in, until every folder the songs end in was
/// counted.
pub(super) fn decide(
    planned: &[PlannedSong],
    files: &BTreeMap<usize, PathBuf>,
    (old, freeable): (&BTreeMap<PathBuf, Written>, &BTreeSet<PathBuf>),
    library: &Path,
    force: bool,
    holding: Holding<'_>,
) -> Vec<Doing> {
    if force {
        return vec![Doing::Write; planned.len()];
    }
    let fold = |p: &Path| crate::relpath::folded(p.parent().unwrap_or(Path::new("")));
    let mut filled: BTreeSet<String> = planned.iter().map(|(_, r)| fold(&path_of(r))).collect();
    // Files no planned song has, as a failed song's, stay where they are.
    let theirs: BTreeSet<&PathBuf> = files.values().collect();
    filled.extend(
        (old.keys())
            .filter(|p| !theirs.contains(p) && !freeable.contains(*p))
            .map(|p| fold(p)),
    );
    loop {
        let decided = decide_in(planned, files, (old, &filled, freeable), library, holding);
        let ended: BTreeSet<String> = planned
            .iter()
            .zip(&decided)
            .filter_map(|((_, r), d)| match d {
                Doing::Guard { file }
                | Doing::Blocked { file, .. }
                | Doing::Held { file: Some(file) } => Some(fold(file)),
                Doing::Unowned { .. } | Doing::Held { file: None } => None,
                _ => Some(fold(&path_of(r))),
            })
            .collect();
        if ended.is_subset(&filled) {
            return decided;
        }
        filled.extend(ended);
    }
}

/// [`decide`] once, a folder's file the user changed staying in each
/// folder of `filled`, folded, that a song is in once the sync is done,
/// and the files of each output `freeable` free for a move.
fn decide_in(
    planned: &[PlannedSong],
    files: &BTreeMap<usize, PathBuf>,
    (old, filled, freeable): (
        &BTreeMap<PathBuf, Written>,
        &BTreeSet<String>,
        &BTreeSet<PathBuf>,
    ),
    library: &Path,
    holding: Holding<'_>,
) -> Vec<Doing> {
    use crate::relpath::folded;
    let mut decided = settled(planned, files, old, library, holding);
    let mut moving = movable(planned, &decided, files, (old, filled), library);
    // Made in rounds: a move lands only where nothing is, or where a move
    // already chosen leaves, so the order chosen is an order they can run.
    let recorded: BTreeSet<&Path> = old.iter().flat_map(|(p, w)| w.files(p)).collect();
    let ours: BTreeSet<String> = recorded.iter().map(|p| folded(p)).collect();
    let pruned: BTreeSet<String> = (freeable.iter())
        .flat_map(|p| freed_files(library, p, &old[p]))
        .map(folded)
        .collect();
    let mut held: BTreeSet<String> = ours.difference(&pruned).cloned().collect();
    let free = |held: &BTreeSet<String>, p: &Path| {
        p.ancestors().all(|a| !held.contains(&folded(a)))
            && (recorded.contains(p)
                || !library.join(p).exists()
                || emptied(library, p, (&ours, held)))
    };
    let mut order = 0;
    loop {
        let before = moving.len();
        moving.retain(|(i, from, to, retag)| {
            let plan = &planned[*i].1.plan;
            let beside = carried(library, filled, &old[from], (from, to), plan);
            let fits = |from: &Path, to: &Path| own_place(library, from, to) || free(&held, to);
            if !fits(from, to) || beside.iter().any(|(a, b)| !fits(a, b)) {
                return true;
            }
            held.remove(&folded(from));
            held.insert(folded(to));
            for (a, b) in &beside {
                held.remove(&folded(a));
                held.insert(folded(b));
            }
            // A folder's file left behind, or one gone, is written anew where the song goes.
            let left = beside.len() < old[from].sidecars.len();
            decided[*i] = Some(Doing::Move {
                from: from.clone(),
                beside,
                retag: *retag || left,
                order,
            });
            order += 1;
            false
        });
        if moving.len() == before {
            break;
        }
    }
    planned
        .iter()
        .zip(decided)
        .map(|((n, r), d)| {
            if let Some(d) = d {
                return d;
            }
            let path = path_of(r);
            let file = files.get(n);
            let was = file.and_then(|f| old.get(f));
            let present = file.is_some_and(|f| own_there(library, f));
            let redo = was
                .and_then(|w| w.plan.as_ref())
                .map(|p| crate::artifact::redo(p, &r.plan));
            let same_media = present && redo.is_some_and(|d| !d.build);
            if let Some(file) = file.filter(|f| edited_since_written(library, old, f)) {
                return Doing::Guard { file: file.clone() };
            }
            // Whosever it is, a changed file at the path is not written over.
            if file != Some(&path) && edited_since_written(library, old, &path) {
                return Doing::Guard { file: path };
            }
            // A change to the song leaves a file beside it the user changed as it is,
            // and the song with it.
            let carried = file.filter(|f| **f != path && same_media).and_then(|f| {
                let by = carried_edit(library, (old, filled), (f, &path), &r.plan)?;
                Some((f, by))
            });
            if let Some((file, by)) = carried {
                return Doing::Blocked {
                    file: file.clone(),
                    by,
                    edited: true,
                };
            }
            // Its tags are written into its own file alone, and the files
            // beside it that differ where no file of the user's is in their way.
            let beside = redo.is_some_and(|d| !d.beside);
            if file == Some(&path)
                && same_media
                && (beside || blocking(library, old, &path, &r.plan).is_empty())
            {
                return Doing::Retag;
            }
            if let Some(by) = blocking(library, old, &path, &r.plan).into_iter().next() {
                let edited = super::owned::changed_since_muman(old, &by);
                // Its own file stays where it is rather than go for nothing.
                return match file.filter(|_| present) {
                    Some(file) => Doing::Blocked {
                        file: file.clone(),
                        by,
                        edited,
                    },
                    None => Doing::Unowned { by, edited },
                };
            }
            Doing::Write
        })
        .collect()
}

/// Each of `planned` not `decided` that could move to its path, by what
/// it would take and leave and whether its tags change: its file of the
/// same media, there, not changed since written where its tags change,
/// no file of the user's at a name it gains beside it, and none it
/// carries there that its plan writes again, songs in the folders
/// `filled` once they are done.
fn movable(
    planned: &[PlannedSong],
    decided: &[Option<Doing>],
    files: &BTreeMap<usize, PathBuf>,
    (old, filled): (&BTreeMap<PathBuf, Written>, &BTreeSet<String>),
    library: &Path,
) -> Vec<(usize, PathBuf, PathBuf, bool)> {
    let mut moving = Vec::new();
    for (i, (n, r)) in planned.iter().enumerate() {
        if decided[i].is_some() {
            continue;
        }
        let path = path_of(r);
        let Some(from) = files.get(n).filter(|f| **f != path) else {
            continue;
        };
        let was = &old[from];
        let present = own_there(library, from);
        let redo = was.plan.as_ref().map(|p| crate::artifact::redo(p, &r.plan));
        let same_media = present && redo.is_some_and(|d| !d.build);
        let retag = was.plan.as_ref() != Some(&r.plan);
        let gains =
            redo.is_none_or(|d| d.beside) && !blocking(library, old, &path, &r.plan).is_empty();
        if same_media
            && !gains
            && !(retag && edited_since_written(library, old, from))
            && carried_edit(library, (old, filled), (from, &path), &r.plan).is_none()
        {
            moving.push((i, from.clone(), path, retag));
        }
    }
    moving
}

/// Whether moving `from` to `to` only changes the case of its name, and
/// no other file is under the new spelling: then it takes its own place.
pub(super) fn own_place(library: &Path, from: &Path, to: &Path) -> bool {
    use crate::relpath::folded;
    folded(from) == folded(to)
        && (!library.join(to).exists()
            || same_file::is_same_file(library.join(from), library.join(to)).unwrap_or(false))
}

/// Whether the library's folder `dir` held files of `ours` and holds
/// nothing else but what a file manager leaves, each of them moved away,
/// `held` no longer: a song's file may move to where it is once they have,
/// as the folder is removed once empty, but never to a folder of the user's.
fn emptied(
    library: &Path,
    dir: &Path,
    (ours, held): (&BTreeSet<String>, &BTreeSet<String>),
) -> bool {
    let Ok(entries) = std::fs::read_dir(library.join(dir)) else {
        return false;
    };
    let mut any = false;
    let all = entries.flatten().all(|e| {
        let path = dir.join(e.file_name());
        let folded = crate::relpath::folded(&path);
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            any = true;
            emptied(library, &path, (ours, held))
        } else if ours.contains(&folded) {
            any = true;
            !held.contains(&folded)
        } else {
            crate::library::is_desktop_clutter(&e.file_name().to_string_lossy())
        }
    });
    any && all
}

/// Everything a sync does to the library, decided before it does any:
/// `status` shows it, and a sync carries it out, departing from it only
/// where a move or a write fails.
pub(super) struct Intent {
    /// What is done with each planned song, in the order of `planned`.
    pub decided: Vec<Doing>,
    /// The outputs no song keeps whose files go before the moves, as a
    /// move lands where one of them is ([`freeable`]).
    pub freed: Vec<PathBuf>,
    /// Each song's file moved to its song's path, with the files beside it
    /// it carries, in an order the moves can be made.
    pub moves: Vec<Relocation>,
    /// The outputs as the moves leave them, which pruning is judged by.
    pub moved: BTreeMap<PathBuf, Written>,
    /// What becomes of each file of `moved` no song makes once the sync
    /// is done.
    pub pruned: Vec<(PathBuf, Pruned)>,
    /// What each source held, recorded in the song list once the library
    /// is written.
    pub edits: Vec<Edit>,
}

/// The whole of what a sync does with `planned`, by what it decided of
/// each song: the moves in their order, the outputs they and the writes
/// leave, the files pruned, and the song list's records.
pub(super) fn intend(
    planned: &[PlannedSong],
    (decided, freeable): (Vec<Doing>, &BTreeSet<PathBuf>),
    manifest: &Manifest,
    old: &BTreeMap<PathBuf, Written>,
    library: &Path,
    (failed, forced): (&BTreeSet<SourceKey>, bool),
    edits: Vec<Edit>,
) -> Intent {
    let mut moves: Vec<(usize, Relocation)> = planned
        .iter()
        .zip(&decided)
        .filter_map(|((_, r), d)| match d {
            Doing::Move {
                from,
                beside,
                order,
                ..
            } => Some((
                *order,
                Relocation {
                    from: from.clone(),
                    to: path_of(r),
                    beside: beside.clone(),
                },
            )),
            _ => None,
        })
        .collect();
    moves.sort_by_key(|(order, _)| *order);
    let moves: Vec<Relocation> = moves.into_iter().map(|(_, m)| m).collect();
    let landing = Landing::of(
        (moves.iter()).flat_map(|m| std::iter::once(&m.to).chain(m.beside.iter().map(|(_, b)| b))),
    );
    let freed: Vec<PathBuf> = (freeable.iter())
        .filter(|p| old[*p].files(p).any(|f| landing.blocks(f)))
        .cloned()
        .collect();
    let mut relocated = old.clone();
    let mut pruned: Vec<(PathBuf, Pruned)> = Vec::new();
    for path in &freed {
        if let Some(written) = relocated.remove(path) {
            let files = written.files(path).map(Path::to_path_buf).collect();
            pruned.push((path.clone(), Pruned::Removed(files)));
        }
    }
    for m in &moves {
        if let Some(written) = relocated.remove(&m.from) {
            relocated.insert(m.to.clone(), moved_to(written, &m.beside));
        }
    }
    let arrived: Vec<Doing> = decided
        .iter()
        .map(|d| match d {
            Doing::Move { retag: true, .. } => Doing::Retag,
            Doing::Move { retag: false, .. } => Doing::Keep,
            d => d.clone(),
        })
        .collect();
    let mut after = kept(planned, &arrived, manifest, &relocated);
    for ((n, r), d) in planned.iter().zip(&arrived) {
        if matches!(d, Doing::Write | Doing::Retag) {
            let path = path_of(r);
            let written = Written {
                sources: manifest.songs[*n].sources.clone(),
                sidecars: crate::artifact::parts(&r.plan)
                    .files(&path)
                    .into_iter()
                    .map(Sidecar::at)
                    .collect(),
                plan: Some(r.plan.clone()),
                stamp: None,
                digest: None,
            };
            after.insert(path, written);
        }
    }
    pruned.extend(prune_plan(library, &relocated, &after, failed, forced));
    Intent {
        decided,
        freed,
        moves,
        moved: relocated,
        pruned,
        edits,
    }
}

/// What `written` records once its file has moved, each file beside it
/// that went with it as `beside` says, from and to, where it went; one
/// that [`stays`](super::owned::stays) is recorded where it is.
pub(super) fn moved_to(mut written: Written, beside: &[(PathBuf, PathBuf)]) -> Written {
    for sidecar in &mut written.sidecars {
        if let Some((_, to)) = beside.iter().find(|(from, _)| *from == sidecar.path) {
            sidecar.path.clone_from(to);
        }
    }
    written
}

/// The outputs a sync leaves as they are, by what `doing` does with each
/// song: its file kept, left alone or held, as `outputs` records it,
/// under the song's sources now, a file kept as it would be written
/// recording the plan, which keeps the gain the mode asks as a song
/// written does.
pub(super) fn kept(
    planned: &[PlannedSong],
    doing: &[Doing],
    manifest: &Manifest,
    outputs: &BTreeMap<PathBuf, Written>,
) -> BTreeMap<PathBuf, Written> {
    let mut kept = BTreeMap::new();
    for ((n, r), d) in planned.iter().zip(doing) {
        let path = match d {
            Doing::Keep => path_of(r),
            Doing::Guard { file }
            | Doing::Held { file: Some(file) }
            | Doing::Blocked { file, .. } => file.clone(),
            _ => continue,
        };
        let Some(mut written) = outputs.get(&path).cloned() else {
            continue;
        };
        if *d == Doing::Keep {
            written.plan = Some(r.plan.clone());
        }
        written.sources.clone_from(&manifest.songs[*n].sources);
        kept.insert(path, written);
    }
    kept
}
