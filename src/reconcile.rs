//! The library brought in line with the song list, offline: every listed
//! source measured, the comparisons each song needs made, each song
//! resolved to a plan and the songs fitted into `[library] max_size`
//! when it is set, a song rendered only when its plan changed, and every
//! file muman wrote that no song makes now deleted.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};

use crate::align;
use crate::atomic::Lock;
use crate::clean::Albums;
use crate::dirs::Dirs;
use crate::facts::{self, Facts};
use crate::history::Run;
use crate::hooks;
use crate::limit;
use crate::manifest::{Manifest, Song};
use crate::naming::{self, Naming};
use crate::parallel;
use crate::render;
use crate::resolve::{self, Input, Plan, Resolved};
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::{self, Aligned, State, Step, Written};
use crate::store::{self, Located, Store};
use crate::tags::{self, Field};

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Render every song, changed or not.
    pub force: bool,
    /// Only say what a sync would do.
    pub dry_run: bool,
    /// Measure again sources that could not be read before.
    pub retry: bool,
    /// How long a dropped-in file waits after arriving.
    pub settling: Duration,
    /// How long writing runs between saves of what it wrote; unset, every
    /// [`CHECKPOINT`].
    pub checkpoint: Option<Duration>,
}

/// Songs left alone for a file not muman's that a run names one by one;
/// a lost state file makes every song such a one.
const NAMED: usize = 10;

/// How long measuring or writing runs between saves of what it did.
const CHECKPOINT: Duration = Duration::from_secs(30);

/// What measuring does besides measuring.
pub struct Measuring<'a> {
    /// Measure again a source that could not be read at this revision.
    pub retry: bool,
    /// Keep what has been measured so far, as a crash would lose it.
    pub checkpoint: &'a mut dyn FnMut(&State) -> Result<()>,
}

impl std::fmt::Debug for Measuring<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Measuring")
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

/// Measure every source in `keys` the store holds whose facts are not
/// current, into `state`, kept by the checkpoint as it goes and at the
/// end. A file that cannot be read is said so, and is not read again
/// until it changes.
pub fn measure<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    keys: &BTreeSet<SourceKey>,
    state: &mut State,
    scratch: &Path,
    how: &mut Measuring<'_>,
    out: &mut W,
) -> Result<()> {
    let outdated: Vec<Located> = keys
        .iter()
        .filter_map(|k| store.locate(k))
        .filter(|l| {
            !state
                .facts
                .get(&l.key)
                .is_some_and(|f| f.holds_for(&l.rev()))
        })
        .collect();
    let now = state::now_secs();
    let (due, failed): (Vec<Located>, Vec<Located>) = outdated.into_iter().partition(|l| {
        how.retry
            || state
                .failures
                .get(&l.key)
                .is_none_or(|f| f.step != Step::Measure || f.due(Some(&l.rev()), now))
    });
    if !failed.is_empty() {
        crate::ui::warning(
            out,
            &format!(
                "Not reading {} source(s) that could not be read before; `sync --retry` tries again",
                failed.len()
            ),
        )?;
    }
    let retagged = retag(runner, store, keys, state, scratch, out)?;
    if due.is_empty() {
        if retagged {
            (how.checkpoint)(state)?;
        }
        return Ok(());
    }
    crate::ui::info(out, &format!("Measuring {} source(s)", due.len()))?;
    let numbered: Vec<(usize, &Located)> = due.iter().enumerate().collect();
    let mut kept = Instant::now();
    parallel::chunked(
        &numbered,
        parallel::builds(),
        |(n, l)| facts::gather(runner, l, &scratch.join(format!("facts-{n}"))),
        |chunk, measured| {
            for ((_, located), result) in chunk.iter().zip(measured) {
                match result {
                    Ok(f) => {
                        state.facts.insert(located.key.clone(), f);
                        state.clear_failure(&located.key);
                    }
                    Err(e) => {
                        crate::ui::warning(out, &format!("Could not read {}: {e:#}", located.key))?;
                        state.record_failure(
                            &located.key,
                            Step::Measure,
                            Some(located.rev()),
                            format!("{e:#}"),
                        );
                    }
                }
            }
            if kept.elapsed() >= CHECKPOINT {
                (how.checkpoint)(state)?;
                kept = Instant::now();
            }
            Ok::<(), anyhow::Error>(())
        },
    )?;
    (how.checkpoint)(state)
}

/// Read again the tags of every source whose measures still hold but
/// whose tags were read another way. Returns whether any were.
fn retag<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    keys: &BTreeSet<SourceKey>,
    state: &mut State,
    scratch: &Path,
    out: &mut W,
) -> Result<bool> {
    let due: Vec<Located> = keys
        .iter()
        .filter_map(|k| store.locate(k))
        .filter(|l| {
            state
                .facts
                .get(&l.key)
                .is_some_and(|f| f.holds_for(&l.rev()) && !f.tags_hold())
        })
        .collect();
    if due.is_empty() {
        return Ok(false);
    }
    crate::ui::info(out, &format!("Reading the tags of {} source(s)", due.len()))?;
    let numbered: Vec<(usize, &Located)> = due.iter().enumerate().collect();
    let read = parallel::map(&numbered, parallel::builds(), |(n, l)| {
        facts::retag(runner, l, &scratch.join(format!("tags-{n}")))
    });
    for (located, result) in due.iter().zip(read) {
        match result {
            Ok((tags, release)) => {
                if let Some(f) = state.facts.get_mut(&located.key) {
                    f.tags = tags;
                    f.release = release;
                    f.tags_method = tags::METHOD.to_string();
                }
            }
            Err(e) => crate::ui::warning(out, &format!("Could not read {}: {e:#}", located.key))?,
        }
    }
    Ok(true)
}

/// Make every comparison the songs' resolving may ask for that is not
/// made already at the sources' revisions. Returns whether any was.
fn compare<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    songs: &[Song],
    state: &mut State,
    out: &mut W,
) -> Result<bool> {
    let rev = |k: &SourceKey| state.facts.get(k).map(|f| f.rev.clone());
    let due: Vec<(SourceKey, SourceKey, (String, String))> = songs
        .iter()
        .flat_map(|s| resolve::wanted_alignments(s, &state.facts))
        .filter(|(a, b)| store.has(a) && store.has(b))
        .filter_map(|(a, b)| {
            let revs = (rev(&a)?, rev(&b)?);
            state
                .alignment(&a, &b, &revs)
                .is_none()
                .then_some((a, b, revs))
        })
        .collect();
    if due.is_empty() {
        return Ok(false);
    }
    crate::ui::info(
        out,
        &format!("Comparing {} pair(s) of recordings", due.len().div_ceil(2)),
    )?;
    let keys: Vec<SourceKey> = due
        .iter()
        .flat_map(|(a, b, _)| [a.clone(), b.clone()])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let envelopes: HashMap<SourceKey, Vec<f64>> = keys
        .iter()
        .zip(parallel::map(&keys, parallel::builds(), |k| {
            let path = store.locate(k)?.path;
            runner
                .output(&align::pcm_command(&path))
                .ok()
                .map(|pcm| align::envelope(&pcm))
        }))
        .filter_map(|(k, e)| Some((k.clone(), e?)))
        .collect();
    let compared = parallel::map(&due, parallel::builds(), |(a, b, revs)| {
        let (ea, eb) = (envelopes.get(a)?, envelopes.get(b)?);
        let found = align::align(ea, eb).unwrap_or(align::Alignment {
            offset_ms: 0,
            stretch_ppm: 0,
            score: 0.0,
            coverage: 0.0,
        });
        let ms = |e: &[f64]| i64::try_from(e.len()).unwrap_or(i64::MAX) * 10;
        Some(Aligned {
            a: a.clone(),
            b: b.clone(),
            method: align::METHOD.to_string(),
            revs: revs.clone(),
            offset_ms: found.offset_ms,
            stretch_ppm: found.stretch_ppm,
            score: found.score,
            coverage: found.coverage,
            a_ms: ms(ea),
            b_ms: ms(eb),
            a_extra_ms: align::audible_outside(ea, eb.len(), &found),
        })
    });
    for aligned in compared.into_iter().flatten() {
        state.record_alignment(aligned);
    }
    Ok(true)
}

/// How a song is named in messages: its title and artist as resolved,
/// else its first source.
pub(crate) fn name_of(
    song: &Song,
    resolved: Option<&Resolved>,
    facts: &BTreeMap<SourceKey, Facts>,
) -> String {
    let from_tags = |tags: &[(String, Vec<String>)]| {
        let get = |f: Field| {
            tags.iter()
                .find(|(k, _)| k == f.vorbis())
                .and_then(|(_, v)| v.first())
                .cloned()
        };
        get(Field::Title).map(|t| match get(Field::Artist) {
            Some(a) => format!("{t} — {a}"),
            None => t,
        })
    };
    resolved
        .and_then(|r| from_tags(&r.plan.tags))
        .or_else(|| {
            let first = song.sources.first()?;
            let offers = &facts.get(first)?.tags;
            let title = offers.get(&Field::Title)?.values.first()?.clone();
            Some(title)
        })
        .or_else(|| song.sources.first().map(ToString::to_string))
        .unwrap_or_else(|| "a song with no sources".to_string())
}

/// What differs between two plans, in words.
fn changes(old: &Plan, new: &Plan) -> Vec<&'static str> {
    let mut what = Vec::new();
    if old.version != new.version {
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
    if old.lyrics != new.lyrics {
        what.push("lyrics");
    }
    if old.tags != new.tags {
        what.push("tags");
    }
    what
}

pub(crate) fn path_of(r: &Resolved) -> PathBuf {
    let mut name = r.stem.as_os_str().to_os_string();
    name.push(".");
    name.push(r.plan.format.extension());
    PathBuf::from(name)
}

/// Give every song a path of its own: a later song resolving to one an
/// earlier song took is named with its audio's ID too, and numbered
/// when that is taken as well, as two manual files of one name are.
fn separate(planned: &mut [(usize, Resolved)], max_name: usize) {
    // By stem, not path: a FLAC song and an Opus song of one name would
    // write the same `.lrc`.
    let lower = |r: &Resolved| crate::relpath::folded(&r.stem);
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for (_, r) in planned.iter_mut() {
        if taken.insert(lower(r)) {
            continue;
        }
        let base = r.stem.clone();
        let id = r.plan.audio.key.short();
        for n in 1.. {
            r.stem = naming::suffixed(&base, &id, n, max_name);
            if taken.insert(lower(r)) {
                break;
            }
        }
    }
}

/// Keep `state` before the run's end: whole on a sync, and on a dry run
/// only what it measured, which a dry run may cache but never what the
/// library holds.
fn persist(home: &Path, state: &State, dry_run: bool, lock: &Lock) -> Result<()> {
    if dry_run {
        State::keep_caches(home, state, lock)
    } else {
        state.save(home)
    }
}

/// Bring the library in line with the song list, recording what it
/// replaces and removes in `run`. Returns whether every song was
/// written.
#[allow(clippy::too_many_lines)]
pub fn reconcile<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    opts: Options,
    mut run: Option<&mut Run>,
    out: &mut W,
) -> Result<bool> {
    let lock = Lock::folder(&dirs.home)?;
    let home = &dirs.home;
    let mut manifest = Manifest::load(home)?;
    let mut state = State::load(home)?;
    let store = Store::scan(dirs)?;
    let temp = tempfile::tempdir().context("creating a temporary directory")?;
    let listed = manifest.keys();
    let mut how = Measuring {
        retry: opts.retry,
        checkpoint: &mut |s: &State| persist(home, s, opts.dry_run, &lock),
    };
    measure(
        runner,
        &store,
        &listed,
        &mut state,
        temp.path(),
        &mut how,
        out,
    )?;
    if compare(runner, &store, &manifest.songs, &mut state, out)? {
        persist(home, &state, opts.dry_run, &lock)?;
    }

    let on_disk: BTreeSet<SourceKey> = listed.iter().filter(|k| store.has(k)).cloned().collect();
    let (mut planned, mut failed) = plan(&manifest, &state, &dirs.library, Some(&on_disk), out)?;
    let mut ok = failed.is_empty();
    adopt(&mut state, &dirs.library, out)?;
    let located: BTreeMap<SourceKey, Located> = listed
        .iter()
        .filter_map(|k| Some((k.clone(), store.locate(k)?)))
        .collect();
    // Beside the home rather than in the system's temporary folder, which
    // may be memory: fitting may render much of the library into it.
    let workshop = tempfile::tempdir_in(home).context("creating a folder to fit the library in")?;
    let (fitted, placed) = if manifest.settings.library.max_size.is_some() {
        if sizes(runner, &store, &planned, &mut state, temp.path(), out)? {
            persist(home, &state, opts.dry_run, &lock)?;
        }
        let block = manifest.settings.library.block_size.0;
        let at = limit::Workshop {
            sources: &located,
            scratch: workshop.path(),
            library: &dirs.library,
            kept: limit::kept(&state, &dirs.library, &failed, block),
        };
        let fitted = limit::fit_library(runner, &manifest, &mut state, &mut planned, &at, out)?;
        if fitted.as_ref().is_some_and(|(f, _)| f.rendered > 0) {
            persist(home, &state, opts.dry_run, &lock)?;
        }
        if let Some((f, _)) = fitted.as_ref().filter(|(f, _)| !f.left_out.is_empty()) {
            ok = false;
            left_out(&manifest, &state, f, out)?;
        }
        fitted.map_or((None, HashMap::new()), |(f, made)| (Some(f), made))
    } else {
        (None, HashMap::new())
    };
    if let Some(run) = run.as_deref_mut() {
        run.outputs_before(&state.outputs);
    }
    let moving = Moving {
        library: &dirs.library,
        home,
        dry_run: opts.dry_run,
    };
    let moved = relocate(&planned, &mut state, &moving, run.as_deref_mut(), out)?;
    if !moved.is_empty() && !opts.dry_run {
        // The files are already at their new paths; a failure before the
        // end of the run must not leave the state naming the old ones.
        state.save(home)?;
        if let Some(run) = run.as_deref_mut() {
            run.checkpoint(home)?;
        }
    }
    let old = state.outputs.clone();
    let current = |r: &Resolved| current(&dirs.library, &old, r);
    let changed_since = |path: &Path| changed_since_written(&dirs.library, &old, path);
    let foreign = |r: &Resolved| foreign(&dirs.library, &old, &path_of(r));
    let (due, guarded): (Vec<&PlannedSong>, Vec<&PlannedSong>) = planned
        .iter()
        .filter(|(_, r)| opts.force || current(r).is_none())
        .partition(|(_, r)| opts.force || !changed_since(&path_of(r)));
    let (due, unowned): (Vec<&PlannedSong>, Vec<&PlannedSong>) = due
        .into_iter()
        .partition(|(_, r)| opts.force || foreign(r).is_empty());

    if opts.dry_run {
        let shown = Shown {
            manifest: &manifest,
            planned: &planned,
            guarded: &guarded,
            unowned: &unowned,
            old: &old,
            state: &state,
            failed: &failed,
            moved: &moved,
        };
        status(&shown, &store, &listed, dirs, opts.settling, out)?;
        if let Some(f) = &fitted {
            fit_summary(f, out)?;
        }
        drop(lock);
        return Ok(ok);
    }
    // Owned before written, and no longer vouched for: a crash between the
    // two must leave neither a file no run would delete nor a rewritten
    // one that looks changed by someone else.
    let mut pending = (!due.is_empty()).then(|| state.clone());
    if let Some(pending) = pending.as_mut() {
        for (n, r) in &due {
            let path = path_of(r);
            let lyrics = Some(path.with_extension("lrc"));
            pending.outputs.insert(
                path,
                Written {
                    sources: manifest.songs[*n].sources.clone(),
                    lyrics,
                    plan: None,
                    stamp: None,
                },
            );
        }
        pending.save(home)?;
    }
    if let Some(run) = run.as_deref_mut() {
        for (_, r) in &due {
            if let Some(written) = old.get(&path_of(r)) {
                run.keep(&dirs.library, &path_of(r))?;
                if let Some(lyrics) = &written.lyrics {
                    run.keep(&dirs.library, lyrics)?;
                }
            }
            // Written over only when forced, and then kept to put back.
            for file in foreign(r) {
                run.keep(&dirs.library, &file)?;
            }
        }
        if !due.is_empty() {
            run.checkpoint(home)?;
        }
    }
    if due.len() > 1 {
        crate::ui::info(out, &format!("Writing {} song(s)", due.len()))?;
    }

    let mut outputs: BTreeMap<PathBuf, Written> = BTreeMap::new();
    for (n, r) in &planned {
        let kept = current(r).filter(|_| !opts.force).or_else(|| {
            guarded
                .iter()
                .any(|(m, _)| m == n)
                .then(|| old.get(&path_of(r)))
                .flatten()
        });
        if let Some(written) = kept {
            outputs.insert(
                path_of(r),
                Written {
                    sources: manifest.songs[*n].sources.clone(),
                    ..written.clone()
                },
            );
        }
    }
    for (_, r) in unowned.iter().take(NAMED) {
        crate::ui::warning(
            out,
            &format!(
                "Left alone, not muman's: {} (`sync --force` writes over it, and `undo` puts it back)",
                crate::relpath::show(&path_of(r))
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
    for (_, r) in &guarded {
        crate::ui::warning(
            out,
            &format!(
                "Left alone, changed since muman wrote it: {} (`sync --force` writes it again)",
                crate::relpath::show(&path_of(r))
            ),
        )?;
    }
    let mut changed = false;
    let mut written = 0_usize;
    let mut learned = Vec::new();
    let every = opts.checkpoint.unwrap_or(CHECKPOINT);
    let mut kept_at = Instant::now();
    let render_one = |(n, r): &&PlannedSong| {
        if let Some((folder, made)) = placed.get(n) {
            return render::place(folder, made, &dirs.library);
        }
        render::render(
            runner,
            &render::Job {
                plan: &r.plan,
                stem: &r.stem,
                library: &dirs.library,
                sources: &located,
                scratch: &temp.path().join(format!("render-{n}")),
            },
        )
    };
    parallel::chunked(&due, parallel::builds(), render_one, |chunk, rendered| {
        for ((n, r), result) in chunk.iter().zip(rendered) {
            let song = &manifest.songs[*n];
            let name = name_of(song, Some(r), &state.facts);
            match result {
                Ok(done) => {
                    changed = true;
                    // A new format writes the song beside its old file, under
                    // another extension.
                    let stem = done.audio.with_extension("");
                    let before = old.get(&done.audio).or_else(|| {
                        old.iter()
                            .find(|(p, w)| {
                                p.with_extension("") == stem && w.sources == song.sources
                            })
                            .map(|(_, w)| w)
                    });
                    let verb = match before.and_then(|w| w.plan.as_ref()) {
                        Some(p) => format!("Updated ({})", changes(p, &r.plan).join(", ")),
                        None => "Added".to_string(),
                    };
                    crate::ui::success(
                        out,
                        &format!("{verb}: {}", crate::relpath::show(&done.audio)),
                    )?;
                    for problem in &done.problems {
                        crate::ui::warning(out, &format!("  {problem}"))?;
                    }
                    written += 1;
                    // Before the stamp, so a hook that tags the file further
                    // does not make it look changed by something else.
                    let values = hooks::written_values(&dirs.library, &done.audio);
                    let values: Vec<(&str, &str)> =
                        values.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    hooks::run(runner, &manifest.hooks, hooks::Event::Written, &values, out)?;
                    if let Some(f) = &fitted {
                        let measured = state::Measured {
                            source: r.plan.audio.key.clone(),
                            audio: done.audio_bytes,
                            lyrics: done.lyrics_bytes,
                        };
                        learned.push((limit::plan_key(&f.tools, &r.plan), measured));
                    }
                    let vouched = Written {
                        sources: song.sources.clone(),
                        lyrics: done.lyrics,
                        plan: Some(r.plan.clone()),
                        stamp: store::stamp_text(&dirs.library.join(&done.audio)),
                    };
                    if let Some(pending) = pending.as_mut() {
                        pending.outputs.insert(done.audio.clone(), vouched.clone());
                        if kept_at.elapsed() >= every {
                            pending.save(home)?;
                            if let Some(run) = run.as_deref_mut() {
                                run.checkpoint(home)?;
                            }
                            kept_at = Instant::now();
                        }
                    }
                    outputs.insert(done.audio.clone(), vouched);
                }
                Err(e) => {
                    ok = false;
                    failed.extend(song.sources.iter().cloned());
                    crate::ui::error(out, &format!("Failed: {name}: {e:#}"))?;
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;
    let mut keep = |files: &[PathBuf]| match run.as_deref_mut() {
        Some(run) => {
            for rel in files {
                run.keep(&dirs.library, rel)?;
            }
            run.checkpoint(home)
        }
        None => Ok(()),
    };
    let removed = prune(&dirs.library, &old, &mut outputs, &failed, &mut keep, out)?;
    let up_to_date = planned.len() - due.len() - guarded.len() - unowned.len();
    if up_to_date > 0 {
        crate::ui::info(out, &format!("Up to date: {up_to_date} song(s)"))?;
    }

    state.outputs = outputs;
    state.sizes.extend(learned);
    if let Some(f) = &fitted {
        let block = manifest.settings.library.block_size.0;
        let size = limit::library_size(&state, &dirs.library, block);
        if size > f.max {
            crate::ui::warning(
                out,
                &format!(
                    "The library takes {}, over its max_size of {}; the next sync fits it again",
                    crate::ui::bytes(size),
                    crate::ui::bytes(f.max)
                ),
            )?;
        } else {
            fit_summary(
                &limit::Fitted {
                    projected: size,
                    ..f.clone()
                },
                out,
            )?;
        }
    }
    // A removed song's measures stay while its sources do, so restoring
    // it measures nothing again.
    let mut kept = listed.clone();
    kept.extend(manifest.removed_keys().into_iter().filter(|k| store.has(k)));
    state.facts.retain(|k, _| kept.contains(k));
    match &fitted {
        Some(f) => state.sizes.retain(|key, _| f.keys.contains(key)),
        None => state.sizes.clear(),
    }
    state.failures.retain(|k, _| listed.contains(k));
    state
        .alignments
        .retain(|a| kept.contains(&a.a) && kept.contains(&a.b));
    state.save(home)?;
    manifest.save_locked(&lock)?;
    if changed || !removed.is_empty() {
        if manifest.settings.library.touch_root {
            touch(&dirs.library, out)?;
        }
        let (written, removed) = (written.to_string(), removed.len().to_string());
        let library = dirs.library.to_string_lossy();
        let values = [
            ("library", library.as_ref()),
            ("written", written.as_str()),
            ("removed", removed.as_str()),
        ];
        hooks::run(runner, &manifest.hooks, hooks::Event::Changed, &values, out)?;
    }
    Ok(ok)
}

/// The audio stream of a source to measure, and where the source is.
struct Sizing(u32, Located);

/// Measure the audio bytes of each planned song's source that a copy
/// would carry and whose facts lack them, for fitting. Returns whether
/// any were.
fn sizes<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    planned: &Planned,
    state: &mut State,
    scratch: &Path,
    out: &mut W,
) -> Result<bool> {
    let due: BTreeMap<SourceKey, Sizing> = planned
        .iter()
        .filter(|(_, r)| {
            matches!(
                r.plan.format,
                resolve::Format::Copy { .. } | resolve::Format::Encode { kbps: None, .. }
            )
        })
        .filter(|(_, r)| {
            state
                .facts
                .get(&r.plan.audio.key)
                .and_then(|f| f.audio.as_ref())
                .is_some_and(|a| a.bytes.is_none())
        })
        .filter_map(|(_, r)| {
            let located = store.locate(&r.plan.audio.key)?;
            Some((
                r.plan.audio.key.clone(),
                Sizing(r.plan.audio.index, located),
            ))
        })
        .collect();
    if due.is_empty() {
        return Ok(false);
    }
    crate::ui::info(
        out,
        &format!("Measuring the audio size of {} source(s)", due.len()),
    )?;
    let due: Vec<(usize, (SourceKey, Sizing))> = due.into_iter().enumerate().collect();
    let measured = parallel::map(
        &due,
        parallel::builds(),
        |(n, (_, Sizing(index, located)))| {
            facts::audio_bytes(runner, located, *index, &scratch.join(format!("size-{n}"))).ok()
        },
    );
    for ((_, (key, _)), bytes) in due.iter().zip(measured) {
        if let Some(audio) = state.facts.get_mut(key).and_then(|f| f.audio.as_mut()) {
            audio.bytes = bytes;
        }
    }
    Ok(true)
}

/// Say which songs were left out for want of room, and what the library
/// would need.
fn left_out<W: Write>(
    manifest: &Manifest,
    state: &State,
    fitted: &limit::Fitted,
    out: &mut W,
) -> Result<()> {
    const NAMED: usize = 10;
    let names: Vec<String> = fitted
        .left_out
        .iter()
        .take(NAMED)
        .map(|n| name_of(&manifest.songs[*n], None, &state.facts))
        .collect();
    let more = fitted.left_out.len().saturating_sub(NAMED);
    crate::ui::error(
        out,
        &format!(
            "max_size {} cannot hold the library: at the lowest bitrates it takes {}; left out: {}{} (raise [library] max_size or lower [audio] min_kbps)",
            crate::ui::bytes(fitted.max),
            crate::ui::bytes(fitted.floor.unwrap_or(0)),
            names.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        ),
    )?;
    Ok(())
}

/// Say how full the library is projected to be against its limit.
fn fit_summary<W: Write>(fitted: &limit::Fitted, out: &mut W) -> Result<()> {
    crate::ui::info(
        out,
        &format!(
            "Library: {} of {} (max_size); {} song(s) below their best format",
            crate::ui::bytes(fitted.projected),
            crate::ui::bytes(fitted.max),
            fitted.lowered
        ),
    )?;
    Ok(())
}

/// Whether a file muman wrote has changed since, by its size and
/// time: a tagger or player that rewrote it.
/// What `old` records of the song's file, when that file is the song as
/// it would be written now: its plan, its file there and not empty, its
/// lyrics there.
fn current<'a>(
    library: &Path,
    old: &'a BTreeMap<PathBuf, Written>,
    r: &Resolved,
) -> Option<&'a Written> {
    let path = path_of(r);
    let written = old.get(&path)?;
    let present = library.join(&path).metadata().is_ok_and(|m| m.len() > 0)
        && written
            .lyrics
            .as_ref()
            .is_none_or(|l| library.join(l).exists());
    (present && written.plan.as_ref() == Some(&r.plan)).then_some(written)
}

/// The files at a song's path, its audio or its lyrics, that muman did
/// not write: a file of the user's own a sync must not write over. A file
/// that is one muman wrote under a name differing only in case, as a
/// filesystem blind to case reports, is muman's.
fn foreign(library: &Path, old: &BTreeMap<PathBuf, Written>, path: &Path) -> Vec<PathBuf> {
    if old.contains_key(path) {
        return Vec::new();
    }
    let lyrics = path.with_extension("lrc");
    let owned = |rel: &Path| {
        old.iter().any(|(p, w)| {
            std::iter::once(p).chain(&w.lyrics).any(|o| {
                same_file::is_same_file(library.join(o), library.join(rel)).unwrap_or(false)
            })
        })
    };
    [path.to_path_buf(), lyrics]
        .into_iter()
        .filter(|rel| library.join(rel).exists() && !owned(rel))
        .collect()
}

pub(crate) fn changed_since_written(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    path: &Path,
) -> bool {
    old.get(path)
        .and_then(|w| w.stamp.as_ref())
        .zip(store::stamp_text(&library.join(path)))
        .is_some_and(|(was, now)| *was != now)
}

/// The moves relocating makes: each song's file from where an output of
/// the same plan lies to the song's path, when nobody holds that path.
fn moves_of(planned: &[PlannedSong], state: &State, library: &Path) -> Vec<(PathBuf, PathBuf)> {
    use crate::relpath::folded;
    // Folded, since on NTFS and APFS a path differing only in case is the
    // same file: moving onto it would overwrite another song.
    let wanted: BTreeSet<String> = planned.iter().map(|(_, r)| folded(&path_of(r))).collect();
    let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (_, r) in planned {
        let to = path_of(r);
        if state
            .outputs
            .get(&to)
            .is_some_and(|w| w.plan.as_ref() == Some(&r.plan))
        {
            continue;
        }
        // Only onto a path nobody holds: no other output of muman's, and
        // no file of anyone's, which the render would guard.
        let occupied = state.outputs.keys().any(|p| folded(p) == folded(&to))
            || library.join(&to).exists()
            || moves.iter().any(|(_, t)| folded(t) == folded(&to));
        if occupied {
            continue;
        }
        let from = state.outputs.iter().find(|(p, w)| {
            w.plan.as_ref() == Some(&r.plan)
                && !wanted.contains(&folded(p))
                && !moves.iter().any(|(f, _)| f == *p)
                && library.join(p).metadata().is_ok_and(|m| m.len() > 0)
                && !changed_since_written(library, &state.outputs, p)
        });
        if let Some((from, _)) = from {
            moves.push((from.clone(), to));
        }
    }
    moves
}

/// Where relocating moves files, and whether it only says so.
struct Moving<'a> {
    library: &'a Path,
    home: &'a Path,
    dry_run: bool,
}

/// Move each song whose plan is unchanged but whose path is not, as a
/// new `[library]` template makes it, rather than render it again; only
/// a file muman wrote and nobody changed since is moved, and only to a
/// path no other song takes. Each move is recorded in `run` before it is
/// made. A dry run says what would move.
fn relocate<W: Write>(
    planned: &[PlannedSong],
    state: &mut State,
    how: &Moving<'_>,
    mut run: Option<&mut Run>,
    out: &mut W,
) -> Result<BTreeSet<PathBuf>> {
    let (library, dry_run) = (how.library, how.dry_run);
    let moves = moves_of(planned, state, library);
    let lyrics_of = |from: &PathBuf, to: &Path| {
        let written = &state.outputs[from];
        written
            .lyrics
            .clone()
            .zip(written.lyrics.as_ref().map(|_| to.with_extension("lrc")))
    };
    if let Some(run) = run.as_deref_mut().filter(|_| !dry_run && !moves.is_empty()) {
        for (from, to) in &moves {
            run.moving(from, to);
            if let Some((a, b)) = lyrics_of(from, to) {
                run.moving(&a, &b);
            }
        }
        run.checkpoint(how.home)?;
    }
    let mut left = Vec::new();
    let mut done = BTreeSet::new();
    for (from, to) in moves {
        let mut written = state.outputs[&from].clone();
        let lyrics = written.lyrics.as_ref().map(|_| to.with_extension("lrc"));
        if dry_run {
            crate::ui::info(
                out,
                &format!(
                    "Would move: {} → {}",
                    crate::relpath::show(&from),
                    crate::relpath::show(&to)
                ),
            )?;
        } else {
            let result = move_file(library, &from, &to).and_then(|()| {
                match (&written.lyrics, &lyrics) {
                    // A song whose lyrics cannot follow goes back whole.
                    (Some(a), Some(b)) => move_file(library, a, b).inspect_err(|_| {
                        let _ = move_file(library, &to, &from);
                    }),
                    _ => Ok(()),
                }
            });
            if let Err(e) = result {
                if let Some(run) = run.as_deref_mut() {
                    run.not_moved(&from, &to);
                    if let (Some(a), Some(b)) = (&written.lyrics, &lyrics) {
                        run.not_moved(a, b);
                    }
                }
                crate::ui::warning(
                    out,
                    &format!(
                        "Could not move {}: {e:#}; it is written again",
                        crate::relpath::show(&from)
                    ),
                )?;
                continue;
            }
            crate::ui::info(
                out,
                &format!(
                    "Moved: {} → {}",
                    crate::relpath::show(&from),
                    crate::relpath::show(&to)
                ),
            )?;
            left.push(from.clone());
        }
        written.lyrics = lyrics;
        state.outputs.remove(&from);
        done.insert(to.clone());
        state.outputs.insert(to, written);
    }
    remove_empty_folders(library, &left);
    Ok(done)
}

/// Rename a library file, creating its new folder. A rename that only
/// changes case goes through a temporary name, which a filesystem blind
/// to case would otherwise take for a rename onto itself.
fn move_file(library: &Path, from: &Path, to: &Path) -> Result<()> {
    let (from, to) = (library.join(from), library.join(to));
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let step = |a: &Path, b: &Path| {
        crate::atomic::rename(a, b)
            .with_context(|| format!("moving {} to {}", a.display(), b.display()))
    };
    if crate::relpath::folded(&from) == crate::relpath::folded(&to) {
        let mut temp = to.clone().into_os_string();
        temp.push(".moving");
        let temp = PathBuf::from(temp);
        step(&from, &temp)?;
        step(&temp, &to)
    } else {
        step(&from, &to)
    }
}

/// Each song's place in the song list, and what it resolved to.
pub(crate) type Planned = Vec<(usize, Resolved)>;

/// One song of [`Planned`].
type PlannedSong = (usize, Resolved);

/// Resolve every song, saying which could not be and returning their
/// sources with the plans, each song with a path of its own.
pub(crate) fn plan<W: Write>(
    manifest: &Manifest,
    state: &State,
    library: &Path,
    on_disk: Option<&BTreeSet<SourceKey>>,
    out: &mut W,
) -> Result<(Planned, BTreeSet<SourceKey>)> {
    let naming = Naming::new(&manifest.settings.library, library)?;
    let mut failed = BTreeSet::new();
    let mut planned = Vec::new();
    for key in &manifest.clean.unknown {
        crate::ui::warning(
            out,
            &format!("No cleaning rule is named `{key}` in the song list; it does nothing"),
        )?;
    }
    let albums = Albums::of(state.facts.values().map(|f| &f.tags));
    for (n, song) in manifest.songs.iter().enumerate() {
        let input = Input {
            song,
            album: song.album.as_ref().and_then(|a| manifest.album(a)),
            facts: &state.facts,
            on_disk,
            alignments: &state.alignments,
            lyrics: &manifest.lyrics,
            clean: &manifest.clean,
            albums: &albums,
            naming: &naming,
            audio: &manifest.settings.audio,
            quality: &manifest.settings.quality,
            placement: manifest.settings.library.lyrics,
        };
        match resolve::resolve(&input) {
            Ok(r) => planned.push((n, r)),
            Err(e) => {
                failed.extend(song.sources.iter().cloned());
                let name = name_of(song, None, &state.facts);
                crate::ui::error(out, &format!("Failed: {name}: {e:#}"))?;
            }
        }
    }
    separate(&mut planned, manifest.settings.library.max_name_bytes);
    Ok((planned, failed))
}

/// Take `library` as the folder the outputs are in; the files written
/// into another before are no longer muman's to delete.
fn adopt<W: Write>(state: &mut State, library: &Path, out: &mut W) -> Result<()> {
    if let Some(before) = state
        .library
        .as_deref()
        .filter(|l| !crate::platform::same_path(l, library))
    {
        crate::ui::warning(
            out,
            &format!(
                "The library moved from {}; the files written there are left alone",
                before.display()
            ),
        )?;
        state.outputs.clear();
    }
    state.library = Some(library.to_path_buf());
    Ok(())
}

/// What becomes of a file `old` lists that the outputs do not.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pruned {
    /// Deleted, with its lyrics unless another output takes them.
    Removed(Vec<PathBuf>),
    /// Kept, as a song it came from failed this run.
    KeptForFailed,
    /// Left in place and no longer muman's: changed since written.
    LeftChanged,
}

/// What pruning `old` against `outputs` does to each file, deciding
/// alone, for a sync to carry out and a dry run to show.
fn prune_plan(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    outputs: &BTreeMap<PathBuf, Written>,
    failed: &BTreeSet<SourceKey>,
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
        } else if changed_since_written(library, old, path) {
            Pruned::LeftChanged
        } else {
            Pruned::Removed(
                std::iter::once(path)
                    .chain(&written.lyrics)
                    .filter(|file| !outputs.values().any(|w| w.lyrics.as_ref() == Some(*file)))
                    .cloned()
                    .collect(),
            )
        };
        plan.push((path.clone(), fate));
    }
    plan
}

/// Delete every file `old` lists that `outputs` does not, unless a song
/// that failed this run made it, which keeps it, all handed to `keep`
/// first. A file changed since it was written is left, and no longer
/// muman's. Returns what went.
fn prune<W: Write>(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    outputs: &mut BTreeMap<PathBuf, Written>,
    failed: &BTreeSet<SourceKey>,
    keep: &mut dyn FnMut(&[PathBuf]) -> Result<()>,
    out: &mut W,
) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    let plan = prune_plan(library, old, outputs, failed);
    let doomed: Vec<PathBuf> = plan
        .iter()
        .filter_map(|(_, fate)| match fate {
            Pruned::Removed(files) => Some(files.iter().cloned()),
            _ => None,
        })
        .flatten()
        .collect();
    if !doomed.is_empty() {
        keep(&doomed)?;
    }
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
        let mut held = false;
        for file in &files {
            match crate::atomic::remove(&library.join(file)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    // One file held open must not stop the run before the
                    // state is saved; it stays listed and goes next time.
                    crate::ui::warning(
                        out,
                        &format!(
                            "Could not remove {}: {e}; the next sync tries again",
                            file.display()
                        ),
                    )?;
                    held = true;
                }
                _ => removed.push(file.clone()),
            }
        }
        if held {
            outputs.insert(path.clone(), old[&path].clone());
            continue;
        }
        crate::ui::info(out, &format!("Removed: {}", crate::relpath::show(&path)))?;
    }
    remove_empty_folders(library, &removed);
    Ok(removed)
}

/// Remove each folder a removed file leaves empty, up to the library.
pub(crate) fn remove_empty_folders(library: &Path, removed: &[PathBuf]) {
    let mut folders: Vec<PathBuf> = removed
        .iter()
        .flat_map(|p| {
            p.ancestors()
                .skip(1)
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        })
        .filter(|p| !p.as_os_str().is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    folders.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for folder in folders {
        let folder = library.join(folder);
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        let names: Vec<_> = entries.flatten().map(|e| e.file_name()).collect();
        if names
            .iter()
            .all(|n| is_desktop_clutter(&n.to_string_lossy()))
        {
            for name in &names {
                let _ = std::fs::remove_file(folder.join(name));
            }
            // A folder that still holds anything stays; the error says so.
            let _ = std::fs::remove_dir(&folder);
        }
    }
}

/// Files a file manager leaves in a folder it showed, which keep an
/// emptied album folder from being removed otherwise.
fn is_desktop_clutter(name: &str) -> bool {
    matches!(name, "desktop.ini" | "Thumbs.db" | ".DS_Store") || name.starts_with("._")
}

/// Set a folder's time to now, so players that rescan by it notice: a
/// song three levels down changes neither the root nor its children.
/// Best effort: a filesystem that refuses it costs only a rescan.
fn touch<W: Write>(dir: &Path, out: &mut W) -> Result<()> {
    if let Err(e) = filetime::set_file_mtime(dir, filetime::FileTime::now()) {
        crate::ui::warning(
            out,
            &format!("Could not update the time of {}: {e}", dir.display()),
        )?;
    }
    Ok(())
}

/// Where each aspect of a song's plan comes from, and why.
fn show_why<W: Write>(r: &Resolved, out: &mut W) -> Result<()> {
    writeln!(out, "  audio   {}: {}", r.plan.audio.key, r.why.audio)?;
    if let Some(why) = &r.why.fit {
        writeln!(out, "  format  {why}")?;
    }
    if let (Some(c), Some(why)) = (&r.plan.cover, &r.why.cover) {
        writeln!(out, "  cover   {}: {why}", c.key)?;
    }
    if let (Some(l), Some(why)) = (&r.plan.lyrics, &r.why.lyrics) {
        writeln!(out, "  lyrics  {}: {why}", l.key)?;
    }
    let width = r.plan.tags.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for ((key, values), why) in r.plan.tags.iter().zip(&r.why.tags) {
        let cleaned = if why.cleaned.is_empty() {
            String::new()
        } else {
            format!(", cleaned: {}", why.cleaned.join(", "))
        };
        writeln!(
            out,
            "  {key:<width$} {} ({}{cleaned})",
            values.join("; "),
            why.from
        )?;
    }
    Ok(())
}

/// What a dry run shows: the decisions a sync would act on.
struct Shown<'a> {
    manifest: &'a Manifest,
    planned: &'a [(usize, Resolved)],
    /// Songs due but left alone, changed since written.
    guarded: &'a [&'a PlannedSong],
    /// Songs due but left alone, a file of someone else's at their path.
    unowned: &'a [&'a PlannedSong],
    old: &'a BTreeMap<PathBuf, Written>,
    state: &'a State,
    failed: &'a BTreeSet<SourceKey>,
    moved: &'a BTreeSet<PathBuf>,
}

/// Say what a sync would do, by the decisions it would make: each song's
/// file and where each aspect comes from, then what it would remove.
fn status<W: Write>(
    shown: &Shown<'_>,
    store: &Store,
    listed: &BTreeSet<SourceKey>,
    dirs: &Dirs,
    settling: Duration,
    out: &mut W,
) -> Result<()> {
    let Shown {
        manifest,
        planned,
        guarded,
        unowned,
        old,
        state,
        failed,
        moved,
    } = shown;
    let library = &dirs.library;
    let mut after: BTreeMap<PathBuf, Written> = BTreeMap::new();
    for (n, r) in *planned {
        let song = &manifest.songs[*n];
        let path = path_of(r);
        let left_alone = guarded.iter().any(|(m, _)| m == n);
        let same_plan = old.get(&path).and_then(|w| w.plan.as_ref()) == Some(&r.plan);
        let verdict = if moved.contains(&path) && same_plan {
            "moved".to_string()
        } else if current(library, old, r).is_some() {
            "up to date".to_string()
        } else if left_alone {
            "left alone, changed since muman wrote it".to_string()
        } else if unowned.iter().any(|(m, _)| m == n) {
            "left alone, not muman's".to_string()
        } else {
            match old.get(&path).and_then(|w| w.plan.as_ref()) {
                Some(p) if p == &r.plan => "written again".to_string(),
                Some(p) => format!("changes: {}", changes(p, &r.plan).join(", ")),
                None => "new".to_string(),
            }
        };
        writeln!(
            out,
            "{}",
            crate::ui::Style::Accent.paint(&name_of(song, Some(r), &state.facts))
        )?;
        writeln!(
            out,
            "  → {} ({verdict})",
            crate::ui::Style::Path.paint(&crate::relpath::show(&path))
        )?;
        show_why(r, out)?;
        let written = if left_alone {
            old.get(&path).cloned()
        } else {
            Some(Written {
                sources: song.sources.clone(),
                lyrics: r.plan.lyrics.as_ref().map(|_| path.with_extension("lrc")),
                plan: Some(r.plan.clone()),
                stamp: None,
            })
        };
        if let Some(written) = written {
            after.insert(path, written);
        }
    }
    for (path, fate) in prune_plan(library, old, &after, failed) {
        match fate {
            Pruned::Removed(_) => crate::ui::warning(
                out,
                &format!("Would remove: {}", crate::relpath::show(&path)),
            )?,
            Pruned::LeftChanged => crate::ui::warning(
                out,
                &format!(
                    "Would leave in place, changed since muman wrote it: {}",
                    crate::relpath::show(&path)
                ),
            )?,
            Pruned::KeptForFailed => {}
        }
    }
    let mut known = listed.clone();
    known.extend(manifest.removed_keys());
    let (unlisted, settling) = store.unlisted(&known, SystemTime::now(), settling);
    for key in unlisted {
        crate::ui::info(
            out,
            &format!("Not listed yet, added by the next sync: {key}"),
        )?;
    }
    for path in settling {
        crate::ui::info(
            out,
            &format!(
                "Still being copied in, left for a later run: {}",
                crate::relpath::show(&path)
            ),
        )?;
    }
    // A manual file not listed is added by the next sync; only fetched
    // files can lie unused.
    for path in store
        .unused(listed)
        .iter()
        .filter(|p| p.starts_with(dirs.ytdlp()))
    {
        crate::ui::info(out, &format!("Unused: {}", crate::relpath::show(path)))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
