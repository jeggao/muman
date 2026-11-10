//! A sync's intent carried out: songs' files moved to their new paths,
//! each song written or tagged again, what changed in each said, and the
//! library folder's time set after a change.
//!
//! A song tagged again is written no further than its parts reach: its
//! file is tagged only where its tags or the lyrics inside changed, and
//! of the files beside it only those whose part changed, or that are gone,
//! are written; the rest keep the records they were written with.
//!
//! Every song due is saved as about to be written before any is, and
//! each as written once it is, in a commit of its own that writes its
//! row alone ([`crate::state::State::save_output`]): a run stopped
//! partway writes again only the song it was writing, and the commit
//! costs the same whatever the size of the state.

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

use super::decide::{Doing, Intent, folder_there, kept, moved_to};
use super::owned::{claim, edited_since_written};
use super::plan::{Planned, PlannedSong, name_of, path_of};
use super::prune::prune;
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
/// the user changed stays in its folder ([`stays`](super::owned::stays)).
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
/// written at its new path instead, and a song that fails keeps the file
/// it had, so pruning is judged again by what was done.
pub(super) fn apply<R: Runner, W: Write>(
    job: &Writing<'_, R>,
    planned: &Planned,
    intent: Intent,
    (state, ledger): (&mut State, &mut Ledger<'_>),
    (failed, force): (&mut BTreeSet<SourceKey>, bool),
    out: &mut W,
) -> Result<Applied> {
    let library = &job.dirs.library;
    let arrived = relocate(intent.moves, state, ledger, out)?;
    if !arrived.is_empty() {
        // The files are already at their new paths; a failure before the
        // end of the run must not leave the state naming the old ones.
        state.save(&job.dirs.home)?;
        ledger.checkpoint()?;
    }
    // A song moved is kept, or has its tags written, at its new path; one
    // whose move failed is written there.
    let doing: Vec<Doing> = planned
        .iter()
        .zip(intent.decided)
        .map(|((_, r), d)| match d {
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
        .collect();
    let old = state.outputs.clone();
    // A folder's file one song hands another is stale to neither.
    let shared: BTreeSet<PathBuf> = planned
        .iter()
        .flat_map(|(_, r)| crate::artifact::parts(&r.plan).folder_files(&path_of(r)))
        .collect();
    let due = due(planned, &doing);
    let anew = beside_anew(library, &old, &due);
    let (mut pending, claims) = before_writing(job, (&due, &anew), state, ledger, force, out)?;
    say_left(&doing, job.manifest.settings.history.runs, out)?;
    let mut outputs = kept(planned, &doing, job.manifest, &old);
    let wrote = render_due(
        job,
        (&due, &anew),
        &old,
        &state.facts,
        (&claims, &shared),
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
    (due, anew): (&[(&PlannedSong, bool)], &BTreeSet<usize>),
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
            let kept = if *retag {
                kept_beside(&job.dirs.library, old, r)
            } else {
                Vec::new()
            };
            let was = |s: &PathBuf| {
                old.get(&path)
                    .and_then(|w| w.sidecars.iter().find(|k| k.path == *s))
                    .filter(|_| kept.contains(s))
                    .cloned()
            };
            let sidecars = match old.get(&path).filter(|_| *retag && !anew.contains(n)) {
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
    let replaced: Vec<Owned> = claims.values().flatten().cloned().collect();
    ledger.replacing(&replaced)?;
    if due.len() > 1 {
        crate::ui::info(out, &format!("Writing {} song(s)", due.len()))?;
    }
    Ok((pending, claims))
}

/// Write `r`'s song, at `job`, as a retag over the file `old` records
/// there, with `recorded` beside it: its tags written again only where
/// they or the lyrics inside changed, the lyrics inside made anew where
/// their part did, and the files beside it that changed, where
/// `beside_anew`.
fn retag_one<R: Runner>(
    runner: &R,
    job: &render::Job<'_>,
    (old, r): (&BTreeMap<PathBuf, Written>, &Resolved),
    recorded: &[PathBuf],
    beside_anew: bool,
) -> Result<render::Rendered> {
    let (library, parts) = (job.library, job.parts);
    let path = path_of(r);
    let was = old
        .get(&path)
        .and_then(|w| w.plan.as_ref())
        .map(crate::artifact::parts);
    let anew = was.as_ref().is_none_or(|p| p.lyrics != parts.lyrics);
    let tags = was.as_ref().is_none_or(|p| parts.redo_from(p).retag);
    let bytes = |p: &Path| std::fs::metadata(library.join(p)).map_or(0, |m| m.len());
    // Staged first and put last, so the song's file is as its record says
    // until every file beside it is written.
    let staged = tags
        .then(|| render::retag(runner, job, (&path, recorded), anew))
        .transpose()?;
    let beside = beside_anew
        .then(|| {
            let kept = kept_beside(library, old, r);
            render::beside(runner, job, &kept).map(|b| (kept, b))
        })
        .transpose()?;
    let mut done = match staged {
        Some(staged) => staged.put(library, job.claim)?,
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

/// The songs of `due` only retagged that write the files beside them
/// anew, by their place in the song list: judged once, before any song
/// is written, so a folder's file another song writes meanwhile changes
/// neither what a song writes nor what it records.
fn beside_anew(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    due: &[(&PlannedSong, bool)],
) -> BTreeSet<usize> {
    due.iter()
        .filter(|((_, r), retag)| *retag && beside_changes(library, old, r))
        .map(|((n, _), _)| *n)
        .collect()
}

/// Whether the files `r`'s plan has beside the song differ from those
/// `old` records at its path, or a file of its folder's is gone, so a
/// song only retagged writes them anew.
fn beside_changes(library: &Path, old: &BTreeMap<PathBuf, Written>, r: &Resolved) -> bool {
    old.get(&path_of(r))
        .and_then(|w| w.plan.as_ref())
        .is_none_or(|p| crate::artifact::redo(p, &r.plan).beside)
        || !folder_there(library, &path_of(r), &r.plan)
}

/// The files beside `r`'s song a retag leaves as they are: each its plan
/// writes whose part is as in the plan the song's file was written from,
/// recorded there as muman's, and there. A folder's file one song hands
/// another is not its to keep.
fn kept_beside(library: &Path, old: &BTreeMap<PathBuf, Written>, r: &Resolved) -> Vec<PathBuf> {
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
        .filter(|at| std::fs::symlink_metadata(library.join(at)).is_ok())
        .collect()
}

/// Say which songs' files a sync leaves alone, and why, `runs` kept to
/// undo it by.
fn say_left<W: Write>(doing: &[Doing], runs: usize, out: &mut W) -> Result<()> {
    let mut unowned = Vec::new();
    for d in doing {
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
                    "Left alone, {} changed since muman wrote it (`sync --force` writes it again)",
                    crate::relpath::show(by)
                ),
            )?,
            Doing::Unowned { by, .. } => unowned.push(by.clone()),
            Doing::Blocked { file, by, edited } => crate::ui::warning(
                out,
                &format!(
                    "Left at {}: {} {} (`sync --force` writes over it)",
                    crate::relpath::show(file),
                    crate::relpath::show(by),
                    if *edited {
                        "changed since muman wrote it"
                    } else {
                        "is not muman's"
                    }
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
    for path in unowned.iter().take(NAMED) {
        crate::ui::warning(
            out,
            &format!(
                "Left alone, not muman's: {} (`sync --force` writes over it{undo})",
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
    (due, anew): (&[(&PlannedSong, bool)], &BTreeSet<usize>),
    old: &BTreeMap<PathBuf, Written>,
    facts: &BTreeMap<SourceKey, Facts>,
    (claims, shared): (&Claims, &BTreeSet<PathBuf>),
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
    let render_one = |((n, r), retag): &(&PlannedSong, bool)| {
        let _working = writing.working(&progress::label(&r.stem));
        let claim = &claims[n];
        let path = path_of(r);
        let recorded: Vec<PathBuf> = old
            .get(&path)
            .map_or_else(Vec::new, |w| w.sidecar_paths().cloned().collect());
        let parts = crate::artifact::parts(&r.plan);
        let planned = parts.files(&path);
        let stale: Vec<PathBuf> = recorded
            .iter()
            .filter(|s| !planned.contains(s) && !shared.contains(*s))
            .cloned()
            .collect();
        if let Some((folder, made)) = placed.get(n) {
            let shared = parts.folder_files(&path);
            return render::place(folder, made, &dirs.library, (claim, &shared), &stale);
        }
        let work = Sub::new(scratch, "render-")?;
        let render_job = render::Job {
            parts: &parts,
            stem: &r.stem,
            library: &dirs.library,
            sources: located,
            scratch: work.path(),
            claim,
            stale: &stale,
        };
        if *retag {
            return retag_one(*runner, &render_job, (old, r), &recorded, anew.contains(n));
        }
        render::render(*runner, &render_job)
    };
    parallel::chunked(due, parallel::builds(), render_one, |chunk, rendered| {
        for (((n, r), retag), result) in chunk.iter().zip(rendered) {
            let song = &manifest.songs[*n];
            let name = name_of(song, Some(r), facts);
            match result {
                Ok(done) => {
                    let as_recorded = *retag && !anew.contains(n);
                    let (vouched, learned) =
                        vouch(job, (song, r), (&done, *retag, as_recorded), old, out)?;
                    wrote.written += 1;
                    wrote.learned.extend(learned);
                    if let Some(pending) = pending.as_deref_mut() {
                        pending.outputs.insert(done.audio.clone(), vouched.clone());
                        pending.save_output(&dirs.home, &done.audio)?;
                    }
                    wrote.outputs.insert(done.audio, vouched);
                }
                Err(e) => {
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

/// Say what writing `done` did to its song, and vouch for it and each
/// file beside it as written, those of a song only `retag`ged as they
/// were, and all of them as recorded where the retag wrote none of them
/// (`as_recorded`); with what its plan rendered to, when fitting asks.
fn vouch<R: Runner, W: Write>(
    job: &Writing<'_, R>,
    (song, r): (&Song, &Resolved),
    (done, retag, as_recorded): (&render::Rendered, bool, bool),
    old: &BTreeMap<PathBuf, Written>,
    out: &mut W,
) -> Result<(Written, Option<(String, state::Measured)>)> {
    let library = &job.dirs.library;
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
    crate::ui::success(
        out,
        &format!("{verb}: {}", crate::relpath::show(&done.audio)),
    )?;
    for problem in &done.problems {
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
    let kept = if retag {
        kept_beside(library, old, r)
    } else {
        Vec::new()
    };
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
