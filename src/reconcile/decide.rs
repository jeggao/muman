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
use crate::manifest::{Edit, Manifest};
use crate::resolve::{LyricsRef, Plan, Resolved};
use crate::source::SourceKey;
use crate::state::{Sidecar, State, Written};

use super::damage::{damaged_since_built, split_drifts};
use super::owned::{edited_since_written, foreign};
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
    let present = library.join(&path).metadata().is_ok_and(|m| m.len() > 0)
        && written.sidecar_paths().all(|s| library.join(s).exists());
    (present && written.plan.as_ref() == Some(&r.plan)).then_some(written)
}

/// Which file the state records is each song's, one file to one song:
/// the one made from the most of the song's own sources, those no other
/// song lists, as the state recorded them when it was written. So a song
/// whose path, plan or sources changed still has its file, and every
/// decision about it, to keep, move, guard, write or delete, is made of
/// that file; a song no file was made from has none. Ties go to the song
/// listed first, then to the path that sorts first.
pub(crate) fn files_of(
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

/// Whether two plans make the same audio, cover and lyrics: a file of
/// one is a file of the other with its tags written again. Lyrics cleaned
/// by the song's names are cleaned anew under other names, so a song
/// retitled is written again, as it would be made from the start.
pub(super) fn media_equal(a: &Plan, b: &Plan) -> bool {
    a.version == b.version
        && a.format == b.format
        && a.chain() == b.chain()
        // A retag sets an Opus header's gain, but only a write puts back the one it had.
        && (a.opus_header().is_none() || b.opus_header().is_some())
        && a.audio == b.audio
        && a.cover == b.cover
        && a.lyrics.as_ref().map(LyricsRef::lines) == b.lyrics.as_ref().map(LyricsRef::lines)
        && crate::render::lyrics_names(a) == crate::render::lyrics_names(b)
        // MP4 chapters are ffmpeg's to write.
        && a.chapters == b.chapters
}

/// What a sync does with one song, decided for every song before any is
/// done, so a sync does what `status` says it would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Doing {
    /// Its file is as it would be written.
    Keep,
    /// Its file moves to the song's path, its tags written again when
    /// they changed; moves are made by `order`, as a move into a path
    /// another move leaves runs after it.
    Move {
        from: PathBuf,
        retag: bool,
        order: usize,
    },
    /// Only its tags changed: they are written into its file.
    Retag,
    /// Written from its sources.
    Write,
    /// Left alone: its file, changed since muman wrote it.
    Guard { file: PathBuf },
    /// Left alone: a file not muman's, `by`, is at its path or its
    /// lyrics'.
    Unowned { by: PathBuf },
    /// Left where it is: a file not muman's, `by`, is at the song's path
    /// or its lyrics'.
    Blocked { file: PathBuf, by: PathBuf },
    /// Left as built: a source it was built from was fetched again and
    /// holds otherwise, not yet accepted.
    Held { file: PathBuf },
}

/// What a sync decides for every song, before it does any: each song's
/// file, the sources holding otherwise than it was built from, those
/// damaged since, and what it does with each.
pub(crate) struct Foreseen {
    pub files: BTreeMap<usize, PathBuf>,
    pub drifts: Vec<Drift>,
    pub damaged: BTreeSet<usize>,
    pub decided: Vec<Doing>,
}

/// What a sync of `planned` into `library` decides, `force` and `accept`
/// as given: the one place `sync`, `status` and `info` decide it.
pub(crate) fn foresee(
    manifest: &Manifest,
    state: &State,
    planned: &[PlannedSong],
    library: &Path,
    (force, accept): (bool, bool),
) -> Foreseen {
    let files = files_of(planned.iter().map(|(n, _)| *n), manifest, &state.outputs);
    let drifts = crate::held::drifts(manifest, &state.facts);
    let damaged = if force {
        BTreeSet::new()
    } else {
        damaged_since_built(planned, &drifts, &state.facts)
    };
    let (_, kept) = split_drifts(&drifts, &damaged, accept, state);
    let mut holding: BTreeSet<usize> = kept.iter().map(|d| d.song).collect();
    holding.extend(&damaged);
    let decided = decide(planned, &files, &state.outputs, library, force, &holding);
    Foreseen {
        files,
        drifts,
        damaged,
        decided,
    }
}

/// Each of `planned` kept as it is, decided before any moves: its file
/// as it would be written, or one built before from what a source in
/// `holding` no longer holds.
pub(super) fn settled(
    planned: &[PlannedSong],
    files: &BTreeMap<usize, PathBuf>,
    old: &BTreeMap<PathBuf, Written>,
    library: &Path,
    holding: &BTreeSet<usize>,
) -> Vec<Option<Doing>> {
    planned
        .iter()
        .map(|(n, r)| {
            if current(library, old, r).is_some() {
                return Some(Doing::Keep);
            }
            files
                .get(n)
                .filter(|f| holding.contains(n) && library.join(f).exists())
                .map(|f| Doing::Held { file: f.clone() })
        })
        .collect()
}

/// What a sync does with each of `planned`, by its file in `files`, as
/// [`settled`] first keeps or holds it:
///
/// | The song's file | Done |
/// |---|---|
/// | At its path, as it would be written | Kept |
/// | Built from what a source in `holding` no longer holds | Left as built until `--accept` |
/// | Elsewhere, of the same media, its path free | Moved, then tags written if they changed |
/// | Changed since written, and not merely moving | Left alone until `--force` |
/// | Any, where a changed file is at its path | Left alone until `--force` |
/// | At its path, of the same media | Its tags written |
/// | Any, and a file not muman's at its path or its lyrics' | Left where it is until `--force` |
/// | None, and a file not muman's at its path or its lyrics' | Left alone until `--force` |
/// | Otherwise, or `force` | Written |
///
/// A path is free when no file of anyone's is there, folded as NTFS and
/// APFS fold names, or only a song's file that moves away first, as when
/// two songs of one name become two of two names and the one told apart
/// takes the plain name; a name changing only its case is free to its
/// own file. Moves are listed in the order they can be made;
/// two songs swapping paths are written instead. A changed file moves
/// only when its tags need no writing, so nothing a tagger wrote is
/// written over.
pub(super) fn decide(
    planned: &[PlannedSong],
    files: &BTreeMap<usize, PathBuf>,
    old: &BTreeMap<PathBuf, Written>,
    library: &Path,
    force: bool,
    holding: &BTreeSet<usize>,
) -> Vec<Doing> {
    use crate::relpath::folded;
    if force {
        return vec![Doing::Write; planned.len()];
    }
    let mut decided = settled(planned, files, old, library, holding);
    // Each song that could move, by what it would take and leave.
    let mut moving: Vec<(usize, PathBuf, PathBuf, bool)> = Vec::new();
    for (i, (n, r)) in planned.iter().enumerate() {
        if decided[i].is_some() {
            continue;
        }
        let path = path_of(r);
        let Some(from) = files.get(n).filter(|f| **f != path) else {
            continue;
        };
        let was = &old[from];
        let present = library.join(from).metadata().is_ok_and(|m| m.len() > 0)
            && was.sidecar_paths().all(|s| library.join(s).exists());
        let same_media = present && was.plan.as_ref().is_some_and(|p| media_equal(p, &r.plan));
        let retag = was.plan.as_ref() != Some(&r.plan);
        if same_media && !(retag && edited_since_written(library, old, from)) {
            moving.push((i, from.clone(), path, retag));
        }
    }
    // Made in rounds: a move lands only where nothing is, or where a move
    // already chosen leaves, so the order chosen is an order they can run.
    let recorded: BTreeSet<&Path> = old.iter().flat_map(|(p, w)| w.files(p)).collect();
    let mut held: BTreeSet<String> = recorded.iter().map(|p| folded(p)).collect();
    let free = |held: &BTreeSet<String>, p: &Path| {
        !held.contains(&folded(p)) && (recorded.contains(p) || !library.join(p).exists())
    };
    let mut order = 0;
    loop {
        let before = moving.len();
        moving.retain(|(i, from, to, retag)| {
            let beside: Vec<(&Path, PathBuf)> = old[from]
                .sidecar_paths()
                .map(|s| (s.as_path(), crate::outputs::moved(from, to, s)))
                .collect();
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
            decided[*i] = Some(Doing::Move {
                from: from.clone(),
                retag: *retag,
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
            let present = file.is_some_and(|f| {
                library.join(f).metadata().is_ok_and(|m| m.len() > 0)
                    && was.is_none_or(|w| w.sidecar_paths().all(|s| library.join(s).exists()))
            });
            let same_media = present
                && was
                    .and_then(|w| w.plan.as_ref())
                    .is_some_and(|p| media_equal(p, &r.plan));
            if let Some(file) = file.filter(|f| edited_since_written(library, old, f)) {
                return Doing::Guard { file: file.clone() };
            }
            // Whosever it is, a changed file at the path is not written over.
            if file != Some(&path) && edited_since_written(library, old, &path) {
                return Doing::Guard { file: path };
            }
            // Its tags are written into its own file alone.
            if file == Some(&path) && same_media {
                return Doing::Retag;
            }
            if let Some(by) = foreign(library, old, &path).into_iter().next() {
                // Its own file stays where it is rather than go for nothing.
                return match file.filter(|_| present) {
                    Some(file) => Doing::Blocked {
                        file: file.clone(),
                        by,
                    },
                    None => Doing::Unowned { by },
                };
            }
            Doing::Write
        })
        .collect()
}

/// Whether moving `from` to `to` only changes the case of its name, and
/// no other file is under the new spelling: then it takes its own place.
pub(super) fn own_place(library: &Path, from: &Path, to: &Path) -> bool {
    use crate::relpath::folded;
    folded(from) == folded(to)
        && (!library.join(to).exists()
            || same_file::is_same_file(library.join(from), library.join(to)).unwrap_or(false))
}

/// Everything a sync does to the library, decided before it does any:
/// `status` shows it, and a sync carries it out, departing from it only
/// where a move or a write fails.
pub(super) struct Intent {
    /// What is done with each planned song, in the order of `planned`.
    pub decided: Vec<Doing>,
    /// Each song's file moved to its song's path, in an order the moves
    /// can be made.
    pub moves: Vec<(PathBuf, PathBuf)>,
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
    decided: Vec<Doing>,
    manifest: &Manifest,
    old: &BTreeMap<PathBuf, Written>,
    library: &Path,
    (failed, forced): (&BTreeSet<SourceKey>, bool),
    edits: Vec<Edit>,
) -> Intent {
    let mut moves: Vec<(usize, PathBuf, PathBuf)> = planned
        .iter()
        .zip(&decided)
        .filter_map(|((_, r), d)| match d {
            Doing::Move { from, order, .. } => Some((*order, from.clone(), path_of(r))),
            _ => None,
        })
        .collect();
    moves.sort_by_key(|(order, ..)| *order);
    let moves: Vec<(PathBuf, PathBuf)> = moves.into_iter().map(|(_, f, t)| (f, t)).collect();
    let mut relocated = old.clone();
    for (from, to) in &moves {
        if let Some(written) = relocated.remove(from) {
            relocated.insert(to.clone(), moved_to(written, from, to));
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
                sidecars: crate::outputs::planned(&path, &r.plan)
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
    let pruned = prune_plan(library, &relocated, &after, failed, forced);
    Intent {
        decided,
        moves,
        moved: relocated,
        pruned,
        edits,
    }
}

/// What `written` records once its file has moved from `from` to `to`,
/// each file beside it with it.
pub(super) fn moved_to(mut written: Written, from: &Path, to: &Path) -> Written {
    for sidecar in &mut written.sidecars {
        sidecar.path = crate::outputs::moved(from, to, &sidecar.path);
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
            Doing::Guard { file } | Doing::Held { file } | Doing::Blocked { file, .. } => {
                file.clone()
            }
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
