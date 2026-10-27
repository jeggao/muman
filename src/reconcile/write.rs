//! A sync's intent carried out: songs' files moved to their new paths,
//! each song written or tagged again, what changed in each said, and the
//! library folder's time set after a change.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::dirs::Dirs;
use crate::facts::Facts;
use crate::hooks;
use crate::library::{Ledger, Owned, Relocation};
use crate::limit;
use crate::manifest::{Manifest, Song};
use crate::parallel;
use crate::progress;
use crate::render;
use crate::resolve::{LyricsRef, Plan, Resolved};
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::{self, State, Written};
use crate::store::{self, Located};

use super::decide::{Doing, Intent, kept, moved_to};
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
    if old.cover != new.cover {
        what.push("cover");
    }
    let lines = |p: &Plan| p.lyrics.as_ref().map(LyricsRef::lines);
    let uncleaned = |p: &Plan| {
        p.lyrics.as_ref().map(|l| LyricsRef {
            clean_off: Vec::new(),
            cleaning: 0,
            ..l.lines()
        })
    };
    if uncleaned(old) != uncleaned(new) {
        what.push("lyrics");
    } else if lines(old) != lines(new) || render::lyrics_names(old) != render::lyrics_names(new) {
        what.push("lyric cleaning");
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

/// Make the `moves` [`decide`](super::decide::decide) chose, each a song's file to its new path,
/// as a new `[library]` template makes them, rather than render them
/// again. A file changed since it was written moves too, still changed:
/// written again at the new path, it would be left at the old one, a
/// second copy. The ledger records each move before it is made. Returns
/// the paths moved to.
pub(super) fn relocate<W: Write>(
    moves: Vec<(PathBuf, PathBuf)>,
    state: &mut State,
    ledger: &mut Ledger<'_>,
    out: &mut W,
) -> Result<BTreeSet<PathBuf>> {
    let pairs: Vec<Relocation> = moves
        .into_iter()
        .map(|(from, to)| {
            let lyrics = state.outputs[&from]
                .lyrics
                .clone()
                .map(|l| (l, to.with_extension("lrc")));
            Relocation { from, to, lyrics }
        })
        .collect();
    let results = ledger.relocate(&pairs, out)?;
    let mut done = BTreeSet::new();
    for (m, result) in pairs.into_iter().zip(results) {
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
        state.outputs.insert(m.to.clone(), moved_to(written, &m.to));
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
    /// How long writing runs between saves of what it wrote.
    pub every: Duration,
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
    let due = due(planned, &doing);
    let (mut pending, claims) = before_writing(job, &due, state, ledger, force, out)?;
    say_left(&doing, out)?;
    let mut outputs = kept(planned, &doing, job.manifest, &old);
    let wrote = render_due(
        job,
        &due,
        &old,
        &state.facts,
        &claims,
        (pending.as_mut(), ledger),
        out,
    )?;
    failed.extend(wrote.failed.iter().cloned());
    outputs.extend(wrote.outputs);
    crate::library::remove_empty_folders(library, &wrote.failed_at);
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
    due: &[(&PlannedSong, bool)],
    state: &State,
    ledger: &mut Ledger<'_>,
    forced: bool,
    out: &mut W,
) -> Result<(Option<State>, Claims)> {
    let old = &state.outputs;
    let claims: Claims = due
        .iter()
        .map(|((n, r), _)| (*n, claim(ledger, old, &path_of(r), forced)))
        .collect();
    // Owned before written, and no longer vouched for: a crash between the two must leave
    // neither a file no run would delete nor a rewritten one that looks changed by someone
    // else, nor lyrics of the user's own that the song leaves alone counted muman's.
    let mut pending = (!due.is_empty()).then(|| state.clone());
    if let Some(pending) = pending.as_mut() {
        for ((n, r), _) in due {
            let path = path_of(r);
            let lyrics = Some(path.with_extension("lrc")).filter(|l| {
                std::fs::symlink_metadata(job.dirs.library.join(l)).is_err()
                    || claims[n].iter().any(|o| o.path() == l)
            });
            pending.outputs.insert(
                path,
                Written {
                    sources: job.manifest.songs[*n].sources.clone(),
                    lyrics,
                    plan: None,
                    stamp: None,
                    digest: None,
                },
            );
        }
        // Recorded ahead of a new file as of one written over, so a run
        // stopped among them is undone, the hand edit it applied with it.
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

/// Say which songs' files a sync leaves alone, and why.
fn say_left<W: Write>(doing: &[Doing], out: &mut W) -> Result<()> {
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
            Doing::Unowned { by } => unowned.push(by.clone()),
            Doing::Blocked { file, by } => crate::ui::warning(
                out,
                &format!(
                    "Left at {}: {} is not muman's (`sync --force` writes over it)",
                    crate::relpath::show(file),
                    crate::relpath::show(by)
                ),
            )?,
            _ => {}
        }
    }
    for path in unowned.iter().take(NAMED) {
        crate::ui::warning(
            out,
            &format!(
                "Left alone, not muman's: {} (`sync --force` writes over it, and `undo` puts it back)",
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
/// it into, or by its tags alone into the file `old` records, saying each
/// and running the hooks on it; `pending` keeps each as it is vouched for.
fn render_due<R: Runner, W: Write>(
    job: &Writing<'_, R>,
    due: &[(&PlannedSong, bool)],
    old: &BTreeMap<PathBuf, Written>,
    facts: &BTreeMap<SourceKey, Facts>,
    claims: &Claims,
    (mut pending, ledger): (Option<&mut State>, &mut Ledger<'_>),
    out: &mut W,
) -> Result<Wrote> {
    let Writing {
        runner,
        dirs,
        manifest,
        located,
        placed,
        scratch,
        every,
        ..
    } = job;
    let mut wrote = Wrote {
        outputs: BTreeMap::new(),
        written: 0,
        learned: Vec::new(),
        failed: BTreeSet::new(),
        failed_at: Vec::new(),
    };
    let mut kept_at = Instant::now();
    let writing = progress::step("Writing", Some(due.len() as u64));
    let render_one = |((n, r), retag): &(&PlannedSong, bool)| {
        let _working = writing.working(&progress::label(&r.stem));
        let claim = &claims[n];
        if let Some((folder, made)) = placed.get(n) {
            return render::place(folder, made, &dirs.library, claim);
        }
        if *retag {
            let path = path_of(r);
            let lyrics = old.get(&path).and_then(|w| w.lyrics.as_deref());
            return render::retag(&dirs.library, &path, lyrics, &r.plan, claim);
        }
        render::render(
            *runner,
            &render::Job {
                plan: &r.plan,
                stem: &r.stem,
                library: &dirs.library,
                sources: located,
                scratch: Sub::new(scratch, "render-")?.path(),
                claim,
            },
        )
    };
    parallel::chunked(due, parallel::builds(), render_one, |chunk, rendered| {
        for (((n, r), _), result) in chunk.iter().zip(rendered) {
            let song = &manifest.songs[*n];
            let name = name_of(song, Some(r), facts);
            match result {
                Ok(done) => {
                    let (vouched, learned) = vouch(job, (song, r), &done, old, out)?;
                    wrote.written += 1;
                    wrote.learned.extend(learned);
                    if let Some(pending) = pending.as_deref_mut() {
                        pending.outputs.insert(done.audio.clone(), vouched.clone());
                        if kept_at.elapsed() >= *every {
                            pending.save(&dirs.home)?;
                            ledger.checkpoint()?;
                            kept_at = Instant::now();
                        }
                    }
                    wrote.outputs.insert(done.audio, vouched);
                }
                Err(e) => {
                    wrote.failed.extend(song.sources.iter().cloned());
                    wrote.failed_at.push(path_of(r));
                    crate::ui::error(out, &format!("Failed: {name}: {e:#}"))?;
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(wrote)
}

/// Say what writing `done` did to its song, run the hooks on its file,
/// and vouch for it as written; with what its plan rendered to, when
/// fitting asks.
fn vouch<R: Runner, W: Write>(
    job: &Writing<'_, R>,
    (song, r): (&Song, &Resolved),
    done: &render::Rendered,
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
    // Before the stamp, so a hook that tags the file further does not make
    // it look changed by something else.
    let values = hooks::written_values(library, &done.audio);
    let values: Vec<(&str, &str)> = values.iter().map(|(k, v)| (*k, v.as_str())).collect();
    hooks::run(
        job.runner,
        &job.manifest.hooks,
        hooks::Event::Written,
        &values,
        out,
    )?;
    let learned = job.fitted.map(|f| {
        let measured = state::Measured {
            source: r.plan.audio.key.clone(),
            audio: done.audio_bytes,
            lyrics: done.lyrics_bytes,
        };
        (limit::plan_key(&f.tools, &r.plan), measured)
    });
    let vouched = Written {
        sources: song.sources.clone(),
        lyrics: done.lyrics.clone(),
        plan: Some(r.plan.clone()),
        stamp: store::stamp_text(&library.join(&done.audio)),
        digest: crate::facts::digest_file(&library.join(&done.audio)),
    };
    Ok((vouched, learned))
}
