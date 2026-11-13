//! What a sync does with each song's file, decided for every song before
//! any is done: kept, moved, tagged again, written, or left alone.
//!
//! The decisions and what follows from them, the moves in their order,
//! what each write does with each of its song's files, the outputs they
//! leave and the files pruned and cleared, make one [`Intent`], every
//! library file's fate in it. It is decided in one way from one set of
//! inputs, the [`Grounds`]: the song list, the state as the library is
//! found, a move a stopped run left halfway counted where it goes
//! ([`Found`]), the sources there and what the sync waits on and fetches
//! first. `sync` carries it out and `status` shows it ([`Grounds::intend`]),
//! and `check`, `info` and `remove`'s listing read it ([`foresight`]), so
//! none of them can say otherwise of a file than the sync does with it;
//! a dry run hands it on file by file ([`Foretold`]), which a test holds
//! each sync to. What each write does is judged on the library as the
//! frees and moves before it leave it ([`Found::moving`]), so a write
//! neither removes nor keeps a file the intent did not say: a file beside
//! a song its record names no more, as a folder's file the user changed
//! that stayed in the folder the song left, is written anew where the song
//! is. What it does with each file follows from the moves chosen: a file
//! a move lands on is freed for it, never handed to a song as a folder's
//! file it takes over. A sync departs from it only where a move, a free or
//! a write fails, and judges the writes and pruning again by what was done.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::held::Drift;
use crate::library::Relocation;
use crate::manifest::{Edit, Manifest};
use crate::resolve::{Plan, Resolved};
use crate::source::SourceKey;
use crate::state::{Sidecar, State, Written};
use crate::store::Store;

use super::damage::{damaged_since_built, split_drifts};
use super::owned::{blocking, carried, carried_edit, edited_since_written};
use super::plan::{Planned, PlannedSong, path_of};
use super::prune::{Pruned, prune_plan, removal};

/// The library a sync judges a file by: as it finds it once it has
/// followed a stopped run, each file a move of case alone left at its
/// temporary name where it goes ([`State::halfway`]), which the sync
/// finishes before it judges any; and, once the frees and moves its intent
/// makes before any song is written are counted ([`Found::moving`]), each
/// file where they leave it, so what each write does is decided before
/// any move is made.
#[derive(Debug, Clone)]
pub struct Found<'a> {
    pub root: &'a Path,
    /// Each path whose file is elsewhere now, by where it is, or that
    /// holds none once the frees and moves are made.
    at: BTreeMap<PathBuf, Option<PathBuf>>,
}

impl<'a> Found<'a> {
    pub(super) fn of(library: &'a Path, state: &State) -> Self {
        let at = (state.halfway.iter())
            .map(|to| {
                let moving = crate::library::moving_name(to);
                (to.clone(), Some(moving))
            })
            .collect();
        Self { root: library, at }
    }

    /// The library once the files of `gone` are removed and `moves` made,
    /// in their order.
    pub(super) fn moving<'p>(
        mut self,
        gone: impl IntoIterator<Item = &'p PathBuf>,
        moves: &[Relocation],
    ) -> Self {
        for file in gone {
            self.at.insert(file.clone(), None);
        }
        for m in moves {
            for (from, to) in m.files() {
                let real = self
                    .at
                    .get(from)
                    .cloned()
                    .unwrap_or_else(|| Some(from.clone()));
                self.at.insert(from.clone(), None);
                self.at.insert(to.clone(), real);
            }
        }
        self
    }

    /// Where the file at `rel` is in the library folder now: none when it
    /// is gone by then.
    pub(super) fn locate(&self, rel: &Path) -> Option<PathBuf> {
        match self.at.get(rel) {
            Some(at) => at.as_ref().map(|at| self.root.join(at)),
            None => Some(self.root.join(rel)),
        }
    }

    /// Whether a file, a link among them, is at `rel`.
    pub(super) fn there(&self, rel: &Path) -> bool {
        self.locate(rel)
            .is_some_and(|at| std::fs::symlink_metadata(at).is_ok())
    }

    /// Whether `sidecar` is as muman wrote it, where it is found.
    pub(super) fn as_written(&self, sidecar: &Sidecar) -> bool {
        super::owned::sidecar_written_at(self.locate(&sidecar.path).as_deref(), sidecar)
    }
}

/// What `old` records of the song's file, when that file is the song as
/// it would be written now: its plan, its file there and not empty, each
/// file beside it there.
pub(super) fn current<'a>(
    found: &Found<'_>,
    old: &'a BTreeMap<PathBuf, Written>,
    r: &Resolved,
) -> Option<&'a Written> {
    let path = path_of(r);
    let written = old.get(&path)?;
    let present = own_there(found, &path) && beside_there(found, written, &path, &r.plan);
    (present && written.plan.as_ref() == Some(&r.plan)).then_some(written)
}

/// Whether a song's own file is at `file`, and not empty, as a disk that
/// filled up leaves one: what decides whether the song is there to keep,
/// move or tag again. A file beside it gone is no song gone: it is written
/// again beside the song's file, which is left as it is ([`beside_there`]).
pub(super) fn own_there(found: &Found<'_>, file: &Path) -> bool {
    (found.locate(file)).is_some_and(|at| at.metadata().is_ok_and(|m| m.len() > 0))
}

/// Whether each file beside the song at `path` is there: each `written`
/// records beside it, and each of its folder's files `plan` has it hold.
pub(super) fn beside_there(found: &Found<'_>, written: &Written, path: &Path, plan: &Plan) -> bool {
    written.sidecar_paths().all(|s| found.there(s)) && folder_there(found, path, plan)
}

/// Whether each file of its folder `plan` has the song at `path` hold is
/// there, muman's or one of the user's it leaves alone: one gone is
/// written again.
fn folder_there(found: &Found<'_>, path: &Path, plan: &Plan) -> bool {
    crate::artifact::parts(plan)
        .folder_files(path)
        .iter()
        .all(|f| found.there(f))
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
        &Found::of(library, state),
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

    /// Whether a file lands at `f`, folded.
    fn lands(&self, f: &Path) -> bool {
        self.landed.contains(&crate::relpath::folded(f))
    }

    /// Whether a file at `f` is in the way: at a path that lands, at a
    /// folder of one, or under one, as a file is in the way of a folder of
    /// its name.
    fn blocks(&self, f: &Path) -> bool {
        use crate::relpath::folded;
        self.under.contains(&folded(f)) || f.ancestors().any(|a| self.landed.contains(&folded(a)))
    }
}

/// The files freeing each output of `freeable` takes out of a move's
/// way, folded: all of its files, but one beside it the user changed,
/// theirs, in the way of a move as any file of theirs ([`removal`]).
fn freed_files(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    freeable: &BTreeSet<PathBuf>,
) -> BTreeSet<String> {
    let mut freed = BTreeSet::new();
    for path in freeable {
        let left = removal(library, (path, &old[path]), false, |_| false);
        freed.extend(
            (old[path].files(path))
                .filter(|f| !left.left().iter().any(|l| l == f))
                .map(crate::relpath::folded),
        );
    }
    freed
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
        .filter(|(_, file)| Found::of(library, state).there(file))
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
            .filter(|f| Found::of(library, state).there(f))
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
    found: &Found<'_>,
    (holding, damaged): Holding<'_>,
) -> Vec<Option<Doing>> {
    planned
        .iter()
        .map(|(n, r)| {
            if current(found, old, r).is_some() {
                return Some(Doing::Keep);
            }
            let file = files.get(n).filter(|f| found.there(f));
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
    found: &Found<'_>,
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
        let decided = decide_in(planned, files, (old, &filled, freeable), found, holding);
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
    found: &Found<'_>,
    holding: Holding<'_>,
) -> Vec<Doing> {
    use crate::relpath::folded;
    let library = found.root;
    let mut decided = settled(planned, files, old, found, holding);
    let mut moving = movable(planned, &decided, files, (old, filled), found);
    // Made in rounds: a move lands only where nothing is, or where a move
    // already chosen leaves, so the order chosen is an order they can run.
    let recorded: BTreeSet<&Path> = old.iter().flat_map(|(p, w)| w.files(p)).collect();
    let ours: BTreeSet<String> = recorded.iter().map(|p| folded(p)).collect();
    let pruned = freed_files(library, old, freeable);
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
            let present = file.is_some_and(|f| own_there(found, f));
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
    found: &Found<'_>,
) -> Vec<(usize, PathBuf, PathBuf, bool)> {
    let library = found.root;
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
        let present = own_there(found, from);
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

/// Everything a sync does to the library, decided before it does any,
/// each file's fate in it: `status` shows it, and a sync carries it out,
/// departing from it only where a move, a removal or a write fails.
#[derive(Debug)]
pub struct Intent {
    /// What is done with each planned song, in the order of `planned`.
    pub decided: Vec<Doing>,
    /// The outputs no song keeps whose files go before the moves, as a
    /// move lands where one of them is ([`freeable`]).
    pub freed: Vec<PathBuf>,
    /// Each folder's file of `freed` a song's plan has where it ends,
    /// with the song's file whose record takes it before the moves: the
    /// song writes it over rather than it be removed and written again.
    /// One a move lands on is not handed but freed, as [`decide`] counted
    /// it when it chose that move: a song moving into a folder with the
    /// folder's file it holds carries it there.
    pub handed: BTreeMap<PathBuf, PathBuf>,
    /// Each song's file moved to its song's path, with the files beside it
    /// it carries, in an order the moves can be made.
    pub moves: Vec<Relocation>,
    /// The outputs as the frees, the hand-overs and the moves leave them,
    /// which each song written and pruning are judged by.
    pub moved: BTreeMap<PathBuf, Written>,
    /// What writing each song written or tagged again does with each of
    /// its files, by its place in the song list.
    pub writes: Rewrites,
    /// What becomes of each file of `moved` no song makes once the sync
    /// is done.
    pub pruned: Vec<(PathBuf, Pruned)>,
    /// What becomes of each file a song written leaves that its plan has
    /// none of now, by the song's path: cleared as the song is written,
    /// or left in place where the user changed it.
    pub cleared: Vec<(PathBuf, Pruned)>,
    /// What each source held, recorded in the song list once the library
    /// is written.
    pub edits: Vec<Edit>,
}

impl Intent {
    /// Every file a sync of it removes or leaves in place as the user's,
    /// with the output it was of: those pruned, then those its songs
    /// written clear.
    pub fn removals(&self) -> impl Iterator<Item = &(PathBuf, Pruned)> {
        self.pruned.iter().chain(&self.cleared)
    }

    /// Each library file a sync of it changes, and how, the files a
    /// stopped run left in the library `found` holds first: what a dry run
    /// foretells, which the sync then does, file for file.
    #[must_use]
    pub fn foretold(&self, found: &Found<'_>, leftovers: &crate::library::Leftovers) -> Foretold {
        let mut fates: BTreeMap<PathBuf, Fated> = BTreeMap::new();
        for part in leftovers.parts(found.root) {
            fates.insert(part.clone(), Fated::Gone);
        }
        for (to, at) in found
            .at
            .iter()
            .filter_map(|(to, at)| Some((to, at.as_ref()?)))
        {
            fates.insert(at.clone(), Fated::Gone);
            fates.insert(to.clone(), Fated::From(at.clone()));
        }
        for file in self.removals().flat_map(|(_, fate)| fate.gone()) {
            fates.insert(file.clone(), Fated::Gone);
        }
        for m in &self.moves {
            for (from, to) in m.files() {
                let origin = match fates.get(from) {
                    Some(Fated::From(origin)) => origin.clone(),
                    _ => from.clone(),
                };
                fates.insert(from.clone(), Fated::Gone);
                fates.insert(to.clone(), Fated::From(origin));
            }
        }
        for rewrite in self.writes.values() {
            if rewrite.audio {
                fates.insert(rewrite.path.clone(), Fated::Written);
            }
            for file in
                (rewrite.files.iter()).filter(|f| rewrite.beside && !rewrite.kept.contains(f))
            {
                fates.insert(file.clone(), Fated::Written);
            }
        }
        Foretold(fates)
    }
}

/// Each library file a sync changes, by its path, and what is there once
/// it is done; a file not named is left as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Foretold(pub BTreeMap<PathBuf, Fated>);

/// What is at a path of the library once a sync is done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fated {
    /// No file.
    Gone,
    /// The file that was at the path it names before the sync, moved here
    /// as it was.
    From(PathBuf),
    /// A file the sync wrote.
    Written,
}

/// The whole of what a sync does with `planned`, by what it decided of
/// each song: the moves in their order, the outputs they and the writes
/// leave, what each write does with each of its song's files, the files
/// pruned and cleared, and the song list's records. The songs of `placed`
/// fitting rendered already, and are put in place whole.
// Each input is one a sync decides by; grouped, they would only be split again.
#[expect(clippy::too_many_arguments)]
pub(super) fn intend(
    planned: &[PlannedSong],
    (decided, freeable): (Vec<Doing>, &BTreeSet<PathBuf>),
    manifest: &Manifest,
    old: &BTreeMap<PathBuf, Written>,
    found: &Found<'_>,
    (failed, forced): (&BTreeSet<SourceKey>, bool),
    placed: &BTreeSet<usize>,
    edits: Vec<Edit>,
) -> Intent {
    let library = found.root;
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
                Relocation::new(from.clone(), path_of(r), beside.clone()),
            )),
            _ => None,
        })
        .collect();
    moves.sort_by_key(|(order, _)| *order);
    let moves: Vec<Relocation> = moves.into_iter().map(|(_, m)| m).collect();
    let landing = Landing::of((moves.iter()).flat_map(|m| m.files().map(|(_, to)| to)));
    let freed: Vec<PathBuf> = (freeable.iter())
        .filter(|p| old[*p].files(p).any(|f| landing.blocks(f)))
        .cloned()
        .collect();
    let holders = holders(planned, &decided);
    let mut handed = BTreeMap::new();
    let mut relocated = old.clone();
    let mut pruned: Vec<(PathBuf, Pruned)> = Vec::new();
    for path in &freed {
        if let Some(mut written) = relocated.remove(path) {
            let handing: BTreeMap<PathBuf, PathBuf> = (written.sidecars.iter())
                .filter(|s| super::owned::sidecar_as_written(library, s) && !landing.lands(&s.path))
                .filter_map(|s| Some((s.path.clone(), holders.get(&s.path)?.clone())))
                .filter(|(_, to)| relocated.contains_key(to))
                .collect();
            let fate = removal(library, (path, &written), false, |f| {
                handing.contains_key(f)
            });
            pruned.push((path.clone(), fate));
            // Recorded beside the output it names, as the sync's freeing hands it.
            for (file, to) in &handing {
                let at = written.sidecars.iter().position(|s| s.path == *file);
                if let (Some(at), Some(holder)) = (at, relocated.get_mut(to)) {
                    holder.sidecars.push(written.sidecars.remove(at));
                }
            }
            handed.extend(handing);
        }
    }
    for m in &moves {
        if let Some(written) = relocated.remove(m.from()) {
            relocated.insert(m.to().clone(), moved_to(written, m.beside()));
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
    let due = due(planned, &arrived);
    let gone: Vec<&PathBuf> = pruned.iter().flat_map(|(_, fate)| fate.gone()).collect();
    let found = &found.clone().moving(gone, &moves);
    let writes = rewrites(found, &relocated, &due, (&shared(planned), placed), forced);
    let mut after = kept(planned, &arrived, manifest, &relocated);
    let (written, cleared) = written(&due, &writes, (manifest, &relocated), found, forced);
    after.extend(written);
    pruned.extend(prune_plan(library, &relocated, &after, failed, forced));
    Intent {
        decided,
        freed,
        handed,
        moves,
        moved: relocated,
        writes,
        pruned,
        cleared,
        edits,
    }
}

/// What the state records of each song of `due` once `writes` wrote it,
/// over the outputs `relocated` holds, and what becomes of each file its
/// last build left that its plan has none of now: cleared, or left in
/// place where the user changed it, unless `forced`.
fn written(
    due: &[(&PlannedSong, bool)],
    writes: &Rewrites,
    (manifest, relocated): (&Manifest, &BTreeMap<PathBuf, Written>),
    found: &Found<'_>,
    forced: bool,
) -> (BTreeMap<PathBuf, Written>, Vec<(PathBuf, Pruned)>) {
    let mut after = BTreeMap::new();
    let mut cleared = Vec::new();
    for ((n, r), _) in due {
        let path = path_of(r);
        let rewrite = &writes[n];
        let was = |f: &PathBuf| {
            (relocated.get(&path).into_iter())
                .flat_map(|w| &w.sidecars)
                .find(|s| s.path == *f)
        };
        let sidecars = (rewrite.files.iter())
            .map(|f| was(f).cloned().unwrap_or_else(|| Sidecar::at(f.clone())))
            .collect();
        if !rewrite.stale.is_empty() {
            let (gone, left) = (rewrite.stale.iter().cloned())
                .partition(|s| forced || was(s).is_none_or(|k| found.as_written(k)));
            cleared.push((path.clone(), Pruned::Removed { gone, left }));
        }
        let written = Written {
            sources: manifest.songs[*n].sources.clone(),
            sidecars,
            plan: Some(r.plan.clone()),
            stamp: None,
            digest: None,
        };
        after.insert(path, written);
    }
    (after, cleared)
}

/// What a sync waits on and fetches before it plans, beside what the
/// sources folder holds.
#[derive(Debug, Default, Clone)]
pub struct Ahead {
    /// The sources a sync fetches before it plans, counted there.
    pub to_fetch: BTreeSet<SourceKey>,
    /// The sources a song waits on: still being copied in, to be fetched,
    /// or gone where a moved file cannot be told.
    pub waiting: BTreeSet<SourceKey>,
    /// The files not followed for being unsure, whose songs are held as
    /// built.
    pub unsure: BTreeSet<SourceKey>,
}

impl Ahead {
    /// The sources of the songs a sync fails, whose files it keeps: those
    /// of the songs that could not be resolved, `unresolved`, and those
    /// waited on or not followed.
    #[must_use]
    pub fn failing(&self, mut unresolved: BTreeSet<SourceKey>) -> BTreeSet<SourceKey> {
        unresolved.extend(self.waiting.iter().cloned());
        unresolved.extend(self.unsure.iter().cloned());
        unresolved
    }

    /// What the sync a command that changes nothing foresees waits on and
    /// fetches: what is due to be fetched for want of its file, a file
    /// still being copied in, and the files a move cannot be told of, as
    /// that sync, which fetches first, finds them.
    #[must_use]
    pub fn foreseen(manifest: &Manifest, state: &State, store: &Store) -> Self {
        let due = crate::due::due(
            manifest,
            store,
            &state.failures,
            false,
            crate::state::now_secs(),
        );
        let to_fetch: BTreeSet<SourceKey> = (due.now.into_iter())
            .filter(|(_, f)| *f == crate::due::Fetch::Missing)
            .map(|(k, _)| k)
            .collect();
        let mut unlisted = super::Unlisted::of(manifest, state, store, crate::store::SETTLING);
        let moves = super::moved(manifest, state, store, &mut unlisted, |_| None);
        let waiting = super::arriving(manifest, state, store, crate::store::SETTLING)
            .into_iter()
            .chain(to_fetch.iter().cloned())
            .chain(moves.unsure.iter().flat_map(|(gone, _)| gone).cloned())
            .collect();
        let unsure = (moves.unsure.iter())
            .flat_map(|(_, files)| files)
            .cloned()
            .collect();
        Self {
            to_fetch,
            waiting,
            unsure,
        }
    }
}

/// What a sync plans and decides from, which every command saying what a
/// sync does reads alike: the song list, the state as the library is
/// found ([`as_found`](super::as_found)), the sources there, what the
/// sync waits on and fetches first, and whether it was asked to force and
/// to accept.
#[derive(Debug, Clone, Copy)]
pub struct Grounds<'a> {
    pub manifest: &'a Manifest,
    pub state: &'a State,
    pub store: &'a Store,
    pub library: &'a Path,
    pub ahead: &'a Ahead,
    pub asked: (bool, bool),
}

impl Grounds<'_> {
    /// The sources songs are made from: listed, there and reading, or
    /// fetched before the sync plans.
    #[must_use]
    pub fn on_disk(&self) -> BTreeSet<SourceKey> {
        let (store, state) = (self.store, self.state);
        (self.manifest.keys().into_iter())
            .filter(|k| {
                (store.has(k) && !super::unread(state, store, k)) || self.ahead.to_fetch.contains(k)
            })
            .collect()
    }

    /// What a sync decides of every song of `planned` and does with every
    /// file, the sources of `failed` failing it and fitting having put
    /// the songs of `placed` together whole: the one way `sync`,
    /// `status`, `check`, `info` and `remove` decide it.
    #[must_use]
    pub fn intend(
        &self,
        planned: &[PlannedSong],
        failed: &BTreeSet<SourceKey>,
        placed: &BTreeSet<usize>,
        edits: Vec<Edit>,
    ) -> (Foreseen, Intent) {
        let foreseen = foresee_failing(
            self.manifest,
            self.state,
            planned,
            self.library,
            self.asked,
            failed,
        );
        let intent = intend(
            planned,
            (foreseen.decided.clone(), &foreseen.freeable),
            self.manifest,
            &self.state.outputs,
            &Found::of(self.library, self.state),
            (failed, self.asked.0),
            placed,
            edits,
        );
        (foreseen, intent)
    }
}

/// What a sync from some grounds does, foreseen by a command that changes
/// nothing: each song planned, those that could not be, and what is done
/// with each song and each file.
#[derive(Debug)]
pub struct Foresight {
    pub planned: Planned,
    /// Each song that could not be resolved, by its place, and why.
    pub unresolved: Vec<(usize, anyhow::Error)>,
    /// Each song that could not be resolved while a source of it waits
    /// to be fetched, by its place.
    pub awaiting: Vec<usize>,
    /// The sources of the songs that fail.
    pub failed: BTreeSet<SourceKey>,
    pub foreseen: Foreseen,
    pub intent: Intent,
}

/// What a sync from `grounds` does, as a command that changes nothing
/// says it: each song resolved as the sync resolves it, at the format
/// its file was fitted to where `[library] max_size` fits songs, without
/// rendering any, then decided ([`Grounds::intend`]).
pub fn foresight(grounds: &Grounds<'_>) -> anyhow::Result<Foresight> {
    let on_disk = grounds.on_disk();
    let resolution = super::resolve_songs(
        grounds.manifest,
        &grounds.manifest.settings,
        grounds.state,
        grounds.library,
        (Some(&on_disk), &grounds.ahead.waiting),
        grounds.asked,
    )?;
    let mut planned = resolution.planned;
    crate::limit::as_written(grounds.manifest, grounds.state, &mut planned);
    let failed = grounds.ahead.failing(
        (resolution.unresolved.iter())
            .flat_map(|(n, _)| &grounds.manifest.songs[*n].sources)
            .filter(|k| !crate::manifest::shareable(k))
            .cloned()
            .collect(),
    );
    let (foreseen, intent) = grounds.intend(&planned, &failed, &BTreeSet::new(), Vec::new());
    Ok(Foresight {
        planned,
        unresolved: resolution.unresolved,
        awaiting: resolution.awaiting,
        failed,
        foreseen,
        intent,
    })
}

/// The files a forced sync of the songs planned writes, each song's own
/// and each its plan has beside it: what it says it writes again, or
/// writes over, of a file left alone now, and every other file of a
/// song's it removes, as a `.lrc` the user changed beside a song that
/// takes no lyrics any more.
#[derive(Debug, Default)]
pub struct Forced(BTreeSet<PathBuf>);

impl Forced {
    #[must_use]
    pub fn of(planned: &[PlannedSong]) -> Self {
        Self(
            (planned.iter())
                .flat_map(|(_, r)| {
                    let path = path_of(r);
                    let beside = crate::artifact::parts(&r.plan).files(&path);
                    std::iter::once(path).chain(beside)
                })
                .collect(),
        )
    }

    /// Whether a forced sync writes `file` again, one of the files of
    /// `r`'s song at `from`, where the song goes: else it removes it.
    #[must_use]
    pub fn writes(&self, r: &Resolved, from: &Path, file: &Path) -> bool {
        file == from
            || self
                .0
                .contains(&crate::outputs::moved(from, &path_of(r), file))
    }
}

/// Each folder's file a song of `planned` has where it ends, with the
/// song's own file before the moves, of the songs whose record is there
/// and that `decided` writes over it: moved or tagged again ([`Intent::handed`]).
fn holders(planned: &[PlannedSong], decided: &[Doing]) -> BTreeMap<PathBuf, PathBuf> {
    let mut holders = BTreeMap::new();
    for ((_, r), d) in planned.iter().zip(decided) {
        let path = path_of(r);
        let own = match d {
            Doing::Move { from, .. } => from.clone(),
            Doing::Retag => path.clone(),
            _ => continue,
        };
        for file in crate::artifact::parts(&r.plan).folder_files(&path) {
            holders.insert(file, own.clone());
        }
    }
    holders
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

/// The songs of `planned` that `doing` writes, each with whether only its
/// tags are.
pub(super) fn due<'a>(planned: &'a [PlannedSong], doing: &[Doing]) -> Vec<(&'a PlannedSong, bool)> {
    planned
        .iter()
        .zip(doing)
        .filter_map(|(song, d)| match d {
            Doing::Write => Some((song, false)),
            Doing::Retag => Some((song, true)),
            _ => None,
        })
        .collect()
}

/// The folders' files each song of `planned` holds where it ends, which
/// one song hands another: stale to neither.
pub(super) fn shared(planned: &[PlannedSong]) -> BTreeSet<PathBuf> {
    planned
        .iter()
        .flat_map(|(_, r)| crate::artifact::parts(&r.plan).folder_files(&path_of(r)))
        .collect()
}

/// Which of its files writing a song changes, judged before any song is
/// written, so a folder's file another song writes meanwhile changes
/// neither what a song writes nor what it records, nor what is said of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    /// The song's own file.
    pub path: PathBuf,
    /// It is written: every song's but one only retagged whose tags are as
    /// its file holds them.
    pub audio: bool,
    /// The files beside it are written anew: every song's but one only
    /// retagged whose files beside it are as they were written and there,
    /// each of them recorded beside it. One its record names no more, as
    /// a folder's file the user changed that stayed in the folder the
    /// song left, is written anew where the song is.
    pub beside: bool,
    /// Those beside it a retag leaves as they are ([`kept_beside`]).
    pub kept: Vec<PathBuf>,
    /// Those beside it its plan has that are gone.
    pub gone: Vec<PathBuf>,
    /// Those its last build left that it has none of now, and are there:
    /// cleared, unless the user changed them since.
    pub stale: Vec<PathBuf>,
    /// The files its record names beside it once it is written: those
    /// recorded, when none is written anew, else each its plan has but a
    /// folder's file of the user's it leaves alone.
    pub files: Vec<PathBuf>,
}

impl Rewrite {
    /// Whether writing the song writes over or removes `rel`, one of its
    /// files: kept for `undo` first.
    pub(super) fn replaces(&self, rel: &Path) -> bool {
        if rel == self.path {
            self.audio
        } else {
            self.beside && !self.kept.iter().any(|k| k == rel)
        }
    }
}

/// [`Rewrite`] of each song written, by its place in the song list.
pub type Rewrites = BTreeMap<usize, Rewrite>;

/// What writing each of `due` changes, judged by what `old` records and
/// the library holds before any is written; `shared` the folders' files
/// one song hands another, stale to neither, and `placed` the songs
/// fitting rendered already, which are put in place whole, as a forced
/// sync writes every file.
pub(super) fn rewrites(
    found: &Found<'_>,
    old: &BTreeMap<PathBuf, Written>,
    due: &[(&PlannedSong, bool)],
    (shared, placed): (&BTreeSet<PathBuf>, &BTreeSet<usize>),
    forced: bool,
) -> Rewrites {
    let there = |p: &PathBuf| found.there(p);
    due.iter()
        .map(|((n, r), retag)| {
            let retag = *retag && !placed.contains(n);
            let path = path_of(r);
            let parts = crate::artifact::parts(&r.plan);
            let planned = parts.files(&path);
            let stale = stale(old, r, shared)
                .into_iter()
                .filter(|s| there(s))
                .collect();
            // A folder's file there that is not muman's is left alone.
            let muman = |f: &PathBuf| {
                old.iter().any(|(p, w)| {
                    p == f
                        || w.sidecars
                            .iter()
                            .any(|s| s.path == *f && found.as_written(s))
                })
            };
            let left = |f: &PathBuf| {
                parts.folder_files(&path).contains(f)
                    && there(f)
                    && !muman(f)
                    && !(forced && super::owned::changed_since_muman(old, f))
            };
            let written: Vec<PathBuf> = planned.iter().filter(|f| !left(f)).cloned().collect();
            let recorded = old.get(&path);
            let unrecorded = written
                .iter()
                .any(|f| recorded.is_none_or(|w| !w.sidecar_paths().any(|s| s == f)));
            let beside = !retag || unrecorded || beside_changes(found, old, r);
            let files = match recorded.filter(|_| !beside) {
                Some(w) => w.sidecar_paths().cloned().collect(),
                None => written,
            };
            let rewrite = Rewrite {
                path: path.clone(),
                audio: !retag || tags_due(old, r),
                beside,
                kept: if retag {
                    kept_beside(found, old, r)
                } else {
                    Vec::new()
                },
                gone: planned.into_iter().filter(|p| !there(p)).collect(),
                stale,
                files,
            };
            (*n, rewrite)
        })
        .collect()
}

/// The files `old` records beside `r`'s song, at its path, that its plan
/// has none of now: what writing it clears, but a folder's file one song
/// hands another, of `shared`.
fn stale(
    old: &BTreeMap<PathBuf, Written>,
    r: &Resolved,
    shared: &BTreeSet<PathBuf>,
) -> Vec<PathBuf> {
    let path = path_of(r);
    let planned = crate::artifact::parts(&r.plan).files(&path);
    old.get(&path)
        .into_iter()
        .flat_map(Written::sidecar_paths)
        .filter(|s| !planned.contains(s) && !shared.contains(*s))
        .cloned()
        .collect()
}

/// Whether the files `r`'s plan has beside the song differ from those
/// `old` records at its path, or one of them is gone, so a song only
/// retagged writes them anew.
fn beside_changes(found: &Found<'_>, old: &BTreeMap<PathBuf, Written>, r: &Resolved) -> bool {
    let path = path_of(r);
    old.get(&path).is_none_or(|w| {
        w.plan
            .as_ref()
            .is_none_or(|p| crate::artifact::redo(p, &r.plan).beside)
            || !beside_there(found, w, &path, &r.plan)
    })
}

/// Whether a song only retagged writes its own file: where its tags or
/// the lyrics inside differ from the plan `old` records it written from.
pub(super) fn tags_due(old: &BTreeMap<PathBuf, Written>, r: &Resolved) -> bool {
    old.get(&path_of(r))
        .and_then(|w| w.plan.as_ref())
        .is_none_or(|p| crate::artifact::redo(p, &r.plan).retag)
}

/// The files beside `r`'s song a retag leaves as they are: each its plan
/// writes whose part is as in the plan the song's file was written from,
/// recorded there as muman's, and there. A folder's file one song hands
/// another is not its to keep.
pub(super) fn kept_beside(
    found: &Found<'_>,
    old: &BTreeMap<PathBuf, Written>,
    r: &Resolved,
) -> Vec<PathBuf> {
    let path = path_of(r);
    let Some((w, was)) = old.get(&path).and_then(|w| w.plan.as_ref().map(|p| (w, p))) else {
        return Vec::new();
    };
    let before = crate::artifact::parts(was);
    crate::artifact::parts(&r.plan)
        .beside
        .iter()
        .filter(|b| before.beside.contains(b))
        .filter_map(|b| b.path(&path))
        .filter(|at| w.sidecar_paths().any(|s| s == at))
        .filter(|at| found.there(at))
        .collect()
}
