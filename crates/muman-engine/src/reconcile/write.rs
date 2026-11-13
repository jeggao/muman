//! A sync's intent carried out: songs' files moved to their new paths,
//! each song written or tagged again, what changed in each said, and the
//! library folder's time set after a change.
//!
//! A song tagged again is written no further than its parts reach: its
//! file is tagged only where its tags or the lyrics inside changed, and
//! of the files beside it only those whose part changed, or that are gone,
//! are written; the rest keep the records they were written with. A
//! song's own file is gone only when it is: a file beside it gone, as a
//! deleted `.lrc`, is written again beside it alone. Which files each
//! song writes, keeps and clears is the intent's ([`Rewrite`]), decided
//! with the moves before any is made; it is judged again here only where
//! a move or a free failed. What is said, and kept for `undo` first, is
//! those files, so a song whose `.lrc` alone was written is said by its
//! `.lrc`.
//!
//! Every song due is saved as about to be written before any is, and
//! each as written once it is, in a commit of its own that writes its
//! row alone ([`crate::state::State::save_output`]): a run stopped
//! partway writes again only the song it was writing, and the commit
//! costs the same whatever the size of the state. A song that fails once
//! a file of it is in place ([`crate::library::HalfWritten`]) keeps the
//! record saved before it was written, as a run stopped there leaves it:
//! the files at its paths stay muman's, and the next sync writes it
//! whole, where the record it had would take the files it wrote for the
//! user's.
//!
//! A panic building a song, in muman or a crate it calls, fails that song
//! alone, as an error does ([`crate::errors::caught`]), and clears the
//! `.part` files it staged. Unwinding there is safe: a build changes no
//! other song's files, and puts each of its own in place through the
//! ledger whole, so one stopped partway leaves its song as an error at
//! that point would, recorded as it was before.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::dirs::Dirs;
use crate::facts::Facts;
use crate::library::{Ledger, Owned, Relocation};
use crate::limit;
use crate::manifest::{Manifest, Song};
use crate::parallel;
use crate::progress;
use crate::render;
use crate::resolve::{CoverRef, LyricsRef, Plan, Resolved};
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::{self, Sidecar, State, Written};
use crate::store::{self, Located};

use super::decide::{
    Doing, Forced, Found, Intent, Rewrite, Rewrites, due, kept, moved_to, rewrites, shared,
};
use super::owned::{claim, edited_since_written};
use super::plan::{Planned, PlannedSong, name_of, path_of};
use super::prune::{free, prune};
use super::{NAMED, Sub};

/// What differs between two plans, in words.
pub(super) fn changes(old: &Plan, new: &Plan) -> Vec<&'static str> {
    let mut what = Vec::new();
    // Each codec has its renderer's version: a new codec says it.
    if old.version != new.version && old.format.codec() == new.format.codec() {
        what.push("muman's rendering");
    }
    if old.audio != new.audio {
        what.push("audio");
    } else if old.format != new.format {
        what.push("format");
    }
    let made = |p: &Plan| {
        p.cover.as_ref().map(|c| CoverRef {
            made: None,
            ..c.clone()
        })
    };
    if made(old) != made(new) {
        what.push("cover");
    } else if old.cover != new.cover {
        what.push("muman's rendering");
    }
    if old.folder != new.folder {
        what.push("folder cover");
    }
    let lines = |p: &Plan| p.lyrics.as_ref().map(LyricsRef::lines);
    let names = |p| {
        crate::artifact::parts(p)
            .lyric_text()
            .map(|t| t.names.clone())
    };
    let unwritten = |p: &Plan| {
        p.lyrics.as_ref().map(|l| LyricsRef {
            writing: 0,
            ..l.lines()
        })
    };
    let uncleaned = |p: &Plan| {
        unwritten(p).map(|l| LyricsRef {
            clean_off: Vec::new(),
            cleaning: 0,
            ..l
        })
    };
    if uncleaned(old) != uncleaned(new) {
        what.push("lyrics");
    } else if unwritten(old) != unwritten(new) || names(old) != names(new) {
        what.push("lyric cleaning");
    } else if lines(old) != lines(new) && !what.contains(&"muman's rendering") {
        what.push("muman's rendering");
    }
    let other_tags = |p: &Plan| {
        p.tags
            .iter()
            .filter(|(k, _)| {
                !crate::tags::Field::named(k).is_some_and(crate::tags::Field::is_loudness)
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    if other_tags(old) != other_tags(new) || old.lyrics_language() != new.lyrics_language() {
        what.push("tags");
    }
    if old.held_loudness() != new.held_loudness()
        || old.tags.len() - other_tags(old).len() != new.tags.len() - other_tags(new).len()
    {
        what.push("loudness");
    }
    what
}

/// Make the `moves` [`decide`](super::decide::decide) chose, each a song's file to its new path
/// with the files beside it it carries, as a new `[library]` template
/// makes them, rather than render them again. A file changed since it
/// was written moves too, still changed: written again at the new path,
/// it would be left at the old one, a second copy; but a folder's file
/// the user changed stays in its folder ([`stays`](super::owned::stays)),
/// unless a new template alone moves every song of its folder.
/// The ledger records each move before it is made. Returns the paths
/// moved to.
pub(super) fn relocate<W: Write>(
    moves: Vec<Relocation>,
    state: &mut State,
    ledger: &mut Ledger<'_>,
    out: &mut W,
) -> Result<BTreeSet<PathBuf>> {
    let results = ledger.relocate(&moves, out)?;
    let mut done = BTreeSet::new();
    for (m, result) in moves.into_iter().zip(results) {
        if let Err(e) = result {
            crate::ui::warning(
                out,
                &format!(
                    "Could not move {}: {e:#}; it is written again",
                    crate::relpath::show(&m.from)
                ),
            )?;
            continue;
        }
        let Some(written) = state.outputs.remove(&m.from) else {
            continue;
        };
        done.insert(m.to.clone());
        state
            .outputs
            .insert(m.to.clone(), moved_to(written, &m.beside));
    }
    Ok(done)
}

/// Set a folder's time to now, so players that rescan by it notice: a
/// song three levels down changes neither the root nor its children.
/// Best effort: a filesystem that refuses it costs only a rescan.
pub(super) fn touch<W: Write>(dir: &Path, out: &mut W) -> Result<()> {
    if let Err(e) = filetime::set_file_mtime(dir, filetime::FileTime::now()) {
        crate::ui::warning(
            out,
            &format!("Could not update the time of {}: {e}", dir.display()),
        )?;
    }
    Ok(())
}

/// What writing songs into the library takes beside the songs.
pub(super) struct Writing<'a, R: Runner> {
    pub runner: &'a R,
    pub dirs: &'a Dirs,
    pub manifest: &'a Manifest,
    pub located: &'a BTreeMap<SourceKey, Located>,
    /// The songs fitting rendered already, each in the folder it is in.
    pub placed: &'a limit::Made,
    pub fitted: Option<&'a limit::Fitted>,
    /// The run's scratch folder, which each render takes a folder of.
    pub scratch: &'a Path,
}

/// What carrying out an intent did.
pub(super) struct Applied {
    /// Whether every song was written.
    pub ok: bool,
    pub written: usize,
    pub moved: usize,
    pub removed: usize,
    /// What each plan fitting asked about rendered to, by
    /// [`limit::plan_key`].
    pub learned: Vec<(String, state::Measured)>,
    /// Each song that failed to be written, by its place in the song
    /// list, and why.
    pub songs_failed: Vec<(usize, String)>,
}

/// Carry out `intent` on `planned`: the moves, each song written or
/// tagged again as decided, and the files no song makes pruned, leaving
/// in the state the outputs as they are. A song whose move fails is
/// written at its new path instead, but one whose move lands where a
/// file freed for it could not go stays where it is, and a song that
/// fails keeps the file it had, so pruning is judged again by what was
/// done.
pub(super) fn apply<R: Runner, W: Write>(
    job: &Writing<'_, R>,
    planned: &Planned,
    intent: Intent,
    (state, ledger): (&mut State, &mut Ledger<'_>),
    (failed, force): (&mut BTreeSet<SourceKey>, bool),
    out: &mut W,
) -> Result<Applied> {
    let library = &job.dirs.library;
    let stuck = free(
        ledger,
        (&intent.freed, &intent.handed),
        &mut state.outputs,
        out,
    )?;
    // What could not be freed stays recorded as it is, for the next sync
    // to free, and no song lands on it: a song meant to move there stays
    // where it is, as its file is, rather than be written where it cannot be.
    let mut unfreed: BTreeMap<PathBuf, Written> = BTreeMap::new();
    for path in &intent.freed {
        if let Some(written) = state.outputs.get_mut(path) {
            let there = |s: &Sidecar| std::fs::symlink_metadata(library.join(&s.path)).is_ok();
            written.sidecars.retain(there);
            unfreed.insert(path.clone(), written.clone());
        }
    }
    let lands_stuck = |m: &Relocation| {
        let mut lands = std::iter::once(&m.to).chain(m.beside.iter().map(|(_, to)| to));
        lands.any(|to| stuck.contains(to))
    };
    let staying: BTreeSet<PathBuf> = (intent.moves.iter())
        .filter(|m| lands_stuck(m))
        .map(|m| m.from.clone())
        .collect();
    let meant = intent.moves.len();
    let moves: Vec<Relocation> = (intent.moves.into_iter())
        .filter(|m| !lands_stuck(m))
        .collect();
    let arrived = relocate(moves, state, ledger, out)?;
    let departed = !unfreed.is_empty() || arrived.len() != meant;
    if !arrived.is_empty() {
        // The files are already at their new paths; a failure before the
        // end of the run must not leave the state naming the old ones.
        state.save(&job.dirs.home)?;
        ledger.checkpoint()?;
    }
    let doing = arrived_as(
        planned,
        intent.decided,
        (&staying, &arrived),
        library,
        state,
    );
    let old = state.outputs.clone();
    let due = due(planned, &doing);
    // What each write does was decided with the moves; a move that failed
    // or a file that could not be freed has it judged again by what is.
    let anew = if departed {
        let placed = job.placed.keys().copied().collect();
        let found = Found::of(library, state);
        rewrites(&found, &old, &due, (&shared(planned), &placed), force)
    } else {
        debug_assert_eq!(old, intent.moved, "the outputs as the intent left them");
        intent.writes
    };
    let (mut pending, claims) = before_writing(job, (&due, &anew), state, ledger, force, out)?;
    say_left(planned, &doing, job.manifest.settings.history.runs, out)?;
    let mut outputs = kept(planned, &doing, job.manifest, &old);
    outputs.extend(unfreed);
    let wrote = render_due(
        job,
        (&due, &anew),
        &old,
        &state.facts,
        &claims,
        pending.as_mut(),
        out,
    )?;
    failed.extend(wrote.failed.iter().cloned());
    outputs.extend(wrote.outputs);
    // A folder's file a moved song left behind, cleared, can empty its folder.
    let beside_before = (due.iter())
        .filter_map(|((_, r), _)| old.get(&path_of(r)))
        .flat_map(|w| w.sidecar_paths().cloned());
    let emptied: Vec<PathBuf> = wrote
        .failed_at
        .iter()
        .cloned()
        .chain(beside_before)
        .collect();
    crate::library::remove_empty_folders(library, &emptied);
    let removed = prune(ledger, &old, &mut outputs, failed, force, out)?;
    let up_to_date = doing.iter().filter(|d| **d == Doing::Keep).count();
    if up_to_date > 0 {
        crate::ui::info(out, &format!("Up to date: {up_to_date} song(s)"))?;
    }
    state.outputs = outputs;
    Ok(Applied {
        ok: wrote.failed_at.is_empty(),
        written: wrote.written,
        moved: arrived.len(),
        removed: removed.len(),
        learned: wrote.learned,
        songs_failed: wrote.songs_failed,
    })
}

/// What is done with each of `planned` once the moves are made, by what
/// was `decided`: a song moved is kept, or has its tags written, at its
/// new path, one whose move failed is written there, and one whose move
/// lands where a file could not be freed, `staying`, is kept where it is,
/// as a song held as built is.
fn arrived_as(
    planned: &Planned,
    decided: Vec<Doing>,
    (staying, arrived): (&BTreeSet<PathBuf>, &BTreeSet<PathBuf>),
    library: &Path,
    state: &State,
) -> Vec<Doing> {
    planned
        .iter()
        .zip(decided)
        .map(|((_, r), d)| match d {
            Doing::Move { from, .. } if staying.contains(&from) => Doing::Held { file: Some(from) },
            Doing::Move { retag, .. } if arrived.contains(&path_of(r)) => {
                if retag {
                    Doing::Retag
                } else {
                    Doing::Keep
                }
            }
            Doing::Move { .. } if edited_since_written(library, &state.outputs, &path_of(r)) => {
                Doing::Guard { file: path_of(r) }
            }
            Doing::Move { .. } => Doing::Write,
            d => d,
        })
        .collect()
}

/// What writing songs did.
pub(super) struct Wrote {
    /// Each file written, as muman vouches for it.
    pub outputs: BTreeMap<PathBuf, Written>,
    pub written: usize,
    pub learned: Vec<(String, state::Measured)>,
    /// The sources of each song that failed.
    pub failed: BTreeSet<SourceKey>,
    /// Where each song that failed would have been written.
    pub failed_at: Vec<PathBuf>,
    /// Each song that failed, by its place in the song list, and why.
    pub songs_failed: Vec<(usize, String)>,
}

/// The files writing each song may write over or remove, by its place in
/// the song list, as [`claim`] finds them.
type Claims = BTreeMap<usize, Vec<Owned>>;

/// Record `due` as muman's before any is written, and no longer vouched
/// for, returning the state so recorded; and keep, through the ledger,
/// each file a write replaces, returning those of each song: muman's, and
/// when `forced` the user's own, taken over.
fn before_writing<R: Runner, W: Write>(
    job: &Writing<'_, R>,
    (due, anew): (&[(&PlannedSong, bool)], &Rewrites),
    state: &State,
    ledger: &mut Ledger<'_>,
    forced: bool,
    out: &mut W,
) -> Result<(Option<State>, Claims)> {
    let old = &state.outputs;
    let claims: Claims = due
        .iter()
        .map(|((n, r), _)| (*n, claim(ledger, old, &path_of(r), &r.plan, forced)))
        .collect();
    // Owned before written, and no longer vouched for: a crash between the two must leave
    // neither a file no run would delete nor a rewritten one that looks changed by someone
    // else, nor a file of the user's own beside it that the song leaves alone counted muman's.
    let mut pending = (!due.is_empty()).then(|| state.clone());
    if let Some(pending) = pending.as_mut() {
        for ((n, r), retag) in due {
            let path = path_of(r);
            // A retag keeps vouched for the files beside the song it leaves as they are.
            let kept = &anew[n].kept;
            let was = |s: &PathBuf| {
                old.get(&path)
                    .and_then(|w| w.sidecars.iter().find(|k| k.path == *s))
                    .filter(|_| kept.contains(s))
                    .cloned()
            };
            let sidecars = match old.get(&path).filter(|_| *retag && !anew[n].beside) {
                Some(w) => w.sidecars.clone(),
                None => crate::artifact::parts(&r.plan)
                    .touched(&path, old.get(&path))
                    .into_iter()
                    .filter(|s| {
                        std::fs::symlink_metadata(job.dirs.library.join(s)).is_err()
                            || claims[n].iter().any(|o| o.path() == s)
                            || kept.contains(s)
                    })
                    .map(|s| was(&s).unwrap_or_else(|| Sidecar::at(s)))
                    .collect(),
            };
            pending.outputs.insert(
                path,
                Written {
                    sources: job.manifest.songs[*n].sources.clone(),
                    sidecars,
                    plan: None,
                    stamp: None,
                    digest: None,
                },
            );
        }
        // Recorded ahead of a new file as of one written over, so a run
        // stopped among them is undone, the hand edit it applied with it.
        ledger.changing(
            pending
                .outputs
                .keys()
                .filter(|p| old.get(*p) != pending.outputs.get(*p)),
        );
        ledger.checkpoint()?;
        pending.save(&job.dirs.home)?;
    }
    let replaced: Vec<Owned> = (claims.iter())
        .flat_map(|(n, claim)| claim.iter().filter(|o| anew[n].replaces(o.path())))
        .cloned()
        .collect();
    ledger.replacing(&replaced)?;
    if due.len() > 1 {
        crate::ui::info(out, &format!("Writing {} song(s)", due.len()))?;
    }
    Ok((pending, claims))
}

/// Write `r`'s song, at `job`, as a retag over the file `old` records
/// there, with `recorded` beside it: its tags written again only where
/// they or the lyrics inside changed, the lyrics inside made anew where
/// their part did, and the files beside it that changed, as `rewrite`
/// judged them.
fn retag_one<R: Runner>(
    runner: &R,
    job: &render::Job<'_>,
    (old, r): (&BTreeMap<PathBuf, Written>, &Resolved),
    recorded: &[PathBuf],
    rewrite: &Rewrite,
) -> Result<render::Rendered> {
    let (library, parts) = (job.library, job.parts);
    let path = path_of(r);
    let was = old
        .get(&path)
        .and_then(|w| w.plan.as_ref())
        .map(crate::artifact::parts);
    let anew = was.as_ref().is_none_or(|p| p.lyrics != parts.lyrics);
    let tags = rewrite.audio;
    let bytes = |p: &Path| std::fs::metadata(library.join(p)).map_or(0, |m| m.len());
    // Staged first and put last, so the song's file is as its record says
    // until every file beside it is written.
    let staged = tags
        .then(|| render::retag(runner, job, (&path, recorded), anew))
        .transpose()?;
    let beside = rewrite
        .beside
        .then(|| {
            let kept = rewrite.kept.clone();
            render::beside(runner, job, &kept).map(|b| (kept, b))
        })
        .transpose()?;
    let changed = beside.as_ref().is_some_and(|(_, b)| b.changed);
    let mut done = match staged {
        Some(staged) => staged
            .put(library, job.claim)
            .map_err(|e| crate::library::half_written(e, changed))?,
        None => render::Rendered {
            audio_bytes: bytes(&path),
            audio: path.clone(),
            beside: recorded.iter().map(|b| (b.clone(), bytes(b))).collect(),
            problems: Vec::new(),
        },
    };
    if let Some((kept, beside)) = beside {
        done.beside = parts
            .files(&path)
            .into_iter()
            .filter_map(|file| match beside.files.iter().find(|(p, _)| *p == file) {
                Some(written) => Some(written.clone()),
                None => kept.contains(&file).then(|| (file.clone(), bytes(&file))),
            })
            .collect();
        done.problems.extend(beside.problems);
    }
    Ok(done)
}

/// Say which songs' files a sync leaves alone, and why, `runs` kept to
/// undo it by, and what `sync --force` does with the file in the way: one
/// a song's plan has is written over or again, and any other removed
/// ([`Forced`]).
fn say_left<W: Write>(
    planned: &[PlannedSong],
    doing: &[Doing],
    runs: usize,
    out: &mut W,
) -> Result<()> {
    let forced = Forced::of(planned);
    let mut unowned = Vec::new();
    for ((_, r), d) in planned.iter().zip(doing) {
        let over = |from: &Path, by: &Path, over: &'static str| {
            if forced.writes(r, from, by) {
                over
            } else {
                "removes it"
            }
        };
        match d {
            Doing::Guard { file } => crate::ui::warning(
                out,
                &format!(
                    "Left alone, changed since muman wrote it: {} (`sync --force` writes it again)",
                    crate::relpath::show(file)
                ),
            )?,
            Doing::Unowned { by, edited: true } => crate::ui::warning(
                out,
                &format!(
                    "Left alone, {} changed since muman wrote it (`sync --force` {})",
                    crate::relpath::show(by),
                    over(&path_of(r), by, "writes it again")
                ),
            )?,
            Doing::Unowned { by, .. } => {
                unowned.push((by.clone(), over(&path_of(r), by, "writes over it")));
            }
            Doing::Blocked { file, by, edited } => crate::ui::warning(
                out,
                &format!(
                    "Left at {}: {} {} (`sync --force` {})",
                    crate::relpath::show(file),
                    crate::relpath::show(by),
                    if *edited {
                        "changed since muman wrote it"
                    } else {
                        "is not muman's"
                    },
                    over(file, by, "writes over it")
                ),
            )?,
            _ => {}
        }
    }
    let undo = if runs == 0 {
        ""
    } else {
        ", and `undo` puts it back"
    };
    for (path, forced) in unowned.iter().take(NAMED) {
        crate::ui::warning(
            out,
            &format!(
                "Left alone, not muman's: {} (`sync --force` {forced}{undo})",
                crate::relpath::show(path)
            ),
        )?;
    }
    if unowned.len() > NAMED {
        crate::ui::warning(
            out,
            &format!(
                "… and {} more not muman's, left alone",
                unowned.len() - NAMED
            ),
        )?;
    }
    Ok(())
}

/// Render `due`, each from its sources, from the folder fitting rendered
/// it into, or by its tags alone into the file `old` records, saying
/// each; `pending` keeps each as it is vouched for.
fn render_due<R: Runner, W: Write>(
    job: &Writing<'_, R>,
    (due, anew): (&[(&PlannedSong, bool)], &Rewrites),
    old: &BTreeMap<PathBuf, Written>,
    facts: &BTreeMap<SourceKey, Facts>,
    claims: &Claims,
    mut pending: Option<&mut State>,
    out: &mut W,
) -> Result<Wrote> {
    let Writing {
        runner,
        dirs,
        manifest,
        located,
        placed,
        scratch,
        ..
    } = job;
    let mut wrote = Wrote {
        outputs: BTreeMap::new(),
        written: 0,
        learned: Vec::new(),
        failed: BTreeSet::new(),
        failed_at: Vec::new(),
        songs_failed: Vec::new(),
    };
    let writing = progress::step("Writing", Some(due.len() as u64));
    let build_one = |((n, r), retag): &(&PlannedSong, bool)| {
        let claim = &claims[n];
        let path = path_of(r);
        let recorded: Vec<PathBuf> = old
            .get(&path)
            .map_or_else(Vec::new, |w| w.sidecar_paths().cloned().collect());
        let parts = crate::artifact::parts(&r.plan);
        let stale = &anew[n].stale;
        if let Some((folder, made)) = placed.get(n) {
            let shared = parts.folder_files(&path);
            return render::place(folder, made, &dirs.library, (claim, &shared), stale);
        }
        let work = Sub::new(scratch, "render-")?;
        let render_job = render::Job {
            parts: &parts,
            stem: &r.stem,
            library: &dirs.library,
            sources: located,
            scratch: work.path(),
            claim,
            stale,
        };
        if *retag {
            return retag_one(*runner, &render_job, (old, r), &recorded, &anew[n]);
        }
        render::render(*runner, &render_job)
    };
    let render_one = |song: &(&PlannedSong, bool)| {
        let (_, r) = song.0;
        let _working = writing.working(&progress::label(&r.stem));
        crate::errors::caught(|| build_one(song))
    };
    parallel::chunked(due, parallel::builds(), render_one, |chunk, rendered| {
        for (((n, r), retag), result) in chunk.iter().zip(rendered) {
            let song = &manifest.songs[*n];
            let name = name_of(song, Some(r), facts);
            match result {
                Ok(done) => {
                    let (vouched, learned) =
                        vouch(job, (song, r), (&done, *retag, &anew[n]), old, out)?;
                    wrote.written += 1;
                    wrote.learned.extend(learned);
                    if let Some(pending) = pending.as_deref_mut() {
                        pending.outputs.insert(done.audio.clone(), vouched.clone());
                        pending.save_output(&dirs.home, &done.audio)?;
                    }
                    wrote.outputs.insert(done.audio, vouched);
                }
                Err(e) => {
                    let path = path_of(r);
                    let unfinished = (e.downcast_ref::<crate::library::HalfWritten>())
                        .and(pending.as_deref())
                        .and_then(|p| p.outputs.get(&path));
                    if let Some(unfinished) = unfinished {
                        wrote.outputs.insert(path, unfinished.clone());
                    }
                    // A picture or record other songs share keeps no file
                    // of theirs.
                    wrote.failed.extend(
                        (song.sources.iter())
                            .filter(|k| !crate::manifest::shareable(k))
                            .cloned(),
                    );
                    wrote.failed_at.push(path_of(r));
                    wrote.songs_failed.push((*n, format!("{e:#}")));
                    crate::ui::error(out, &format!("Failed: {name}: {e:#}"))?;
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(wrote)
}

/// Say which files writing a song wrote, `verb` saying what became of
/// the song `before` recorded: its own file where it was written, else
/// each file beside it written in its place, so a song whose `.lrc` alone
/// was gone is said by its `.lrc`. A file beside it it had none of, not
/// there before, is added, one there it takes over is said with the song,
/// one gone is written again, and each its last build left that it
/// has none of now is said removed, or left in place where the user
/// changed it: a song whose lyrics were taken off is said by its `.lrc`
/// removed. Its own file is named unwritten only where nothing else is.
fn say_written<W: Write>(
    (before, verb): (Option<&Written>, &str),
    (done, rewrite): (&render::Rendered, &Rewrite),
    as_recorded: bool,
    library: &Path,
    out: &mut W,
) -> Result<()> {
    let had = |path: &Path| before.is_some_and(|w| w.sidecars.iter().any(|s| s.path == path));
    let wrote: Vec<&PathBuf> = (done.beside.iter())
        .map(|(path, _)| path)
        .filter(|path| !as_recorded && !rewrite.kept.contains(path))
        .collect();
    let own = rewrite.audio || (wrote.is_empty() && rewrite.stale.is_empty());
    if own {
        crate::ui::success(
            out,
            &format!("{verb}: {}", crate::relpath::show(&done.audio)),
        )?;
    }
    for path in wrote {
        // A folder's file another song held is there already: this one takes it over.
        let said = if !had(path) && rewrite.gone.contains(path) {
            "Added"
        } else if !own {
            verb
        } else if rewrite.gone.contains(path) {
            "Written again"
        } else {
            continue;
        };
        crate::ui::success(out, &format!("{said}: {}", crate::relpath::show(path)))?;
    }
    for path in &rewrite.stale {
        if std::fs::symlink_metadata(library.join(path)).is_err() {
            crate::ui::info(out, &format!("Removed: {}", crate::relpath::show(path)))?;
        } else if !as_recorded {
            crate::ui::warning(
                out,
                &format!(
                    "Left in place, changed since muman wrote it: {}; it is yours now",
                    crate::relpath::show(path)
                ),
            )?;
        }
    }
    Ok(())
}

/// Say what writing `done` did to its song, by the files it wrote and
/// cleared as `rewrite` judged them, and vouch for it and each file
/// beside it as written, those of a song only `retag`ged as they were,
/// and all of them as recorded where the retag wrote none of them; with
/// what its plan rendered to, when fitting asks.
fn vouch<R: Runner, W: Write>(
    job: &Writing<'_, R>,
    (song, r): (&Song, &Resolved),
    (done, retag, rewrite): (&render::Rendered, bool, &Rewrite),
    old: &BTreeMap<PathBuf, Written>,
    out: &mut W,
) -> Result<(Written, Option<(String, state::Measured)>)> {
    let library = &job.dirs.library;
    let as_recorded = retag && !rewrite.beside;
    // A new format writes the song beside its old file, under another
    // extension.
    let stem = done.audio.with_extension("");
    let before = old.get(&done.audio).or_else(|| {
        old.iter()
            .find(|(p, w)| p.with_extension("") == stem && w.sources == song.sources)
            .map(|(_, w)| w)
    });
    let changed = before
        .and_then(|w| w.plan.as_ref())
        .map(|p| changes(p, &r.plan));
    // A song forced, or one a stopped run had begun, is neither new nor
    // changed.
    let verb = match (before, changed) {
        (_, Some(what)) if !what.is_empty() => format!("Updated ({})", what.join(", ")),
        (Some(_), _) => "Written again".to_string(),
        (None, _) => "Added".to_string(),
    };
    say_written((before, &verb), (done, rewrite), as_recorded, library, out)?;
    for problem in done.problems.iter().chain(&r.why.chapters) {
        crate::ui::warning(out, &format!("  {problem}"))?;
    }
    let learned = job.fitted.map(|f| {
        let measured = state::Measured {
            source: r.plan.audio.key.clone(),
            audio: done.audio_bytes,
            beside: done.beside.iter().map(|(_, bytes)| *bytes).collect(),
        };
        (limit::plan_key(&f.tools, &r.plan), measured)
    });
    let kept = &rewrite.kept;
    let sidecars = match old.get(&done.audio).filter(|_| as_recorded) {
        Some(w) => w.sidecars.clone(),
        None => done
            .beside
            .iter()
            .map(|(path, _)| {
                old.get(&done.audio)
                    .and_then(|w| w.sidecars.iter().find(|s| s.path == *path))
                    .filter(|_| kept.contains(path))
                    .cloned()
                    .unwrap_or_else(|| Sidecar::vouched(library, path.clone()))
            })
            .collect(),
    };
    let vouched = Written {
        sources: song.sources.clone(),
        sidecars,
        plan: Some(r.plan.clone()),
        stamp: store::stamp_text(&library.join(&done.audio)),
        digest: crate::hash::digest_file(&library.join(&done.audio)),
    };
    Ok((vouched, learned))
}
