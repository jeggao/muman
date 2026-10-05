//! The library brought in line with the song list, offline: every listed
//! source measured, the comparisons each song needs made, each song
//! resolved to a plan, a song rendered only when its plan changed, and
//! every file muman wrote that no song makes now deleted.

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
use crate::manifest::{Manifest, Song};
use crate::naming::{self, Naming};
use crate::parallel;
use crate::render::{self, Rendered};
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
}

/// How long measuring runs between saves of what it measured.
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
                .is_none_or(|f| f.due(Some(&l.rev()), now))
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
    for chunk in numbered.chunks(parallel::builds() * 8) {
        let measured = parallel::map(chunk, parallel::builds(), |(n, l)| {
            facts::gather(runner, l, &scratch.join(format!("facts-{n}")))
        });
        for ((_, located), result) in chunk.iter().zip(measured) {
            match result {
                Ok(f) => {
                    state.facts.insert(located.key.clone(), f);
                    state.failures.remove(&located.key);
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
    }
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
            score: found.score,
            coverage: found.coverage,
            a_ms: ms(ea),
            b_ms: ms(eb),
            a_extra_ms: align::audible_outside(ea, eb.len(), found.offset_ms),
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
    if old.format != new.format || old.audio != new.audio {
        what.push("audio");
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
fn separate(planned: &mut [(usize, Resolved)]) {
    let lower = |r: &Resolved| crate::relpath::folded(&path_of(r));
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for (_, r) in planned.iter_mut() {
        if taken.insert(lower(r)) {
            continue;
        }
        let base = r.stem.clone();
        let id = r.plan.audio.key.short();
        for n in 1.. {
            r.stem = naming::suffixed(&base, &id, n);
            if taken.insert(lower(r)) {
                break;
            }
        }
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
        checkpoint: &mut |s: &State| s.save(home),
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
        state.save(home)?;
    }

    let (planned, mut failed) = plan(&manifest, &state, &dirs.library, out)?;
    let mut ok = failed.is_empty();
    adopt(&mut state, &dirs.library, out)?;
    let old = state.outputs.clone();
    let current = |r: &Resolved| {
        let path = path_of(r);
        let written = old.get(&path)?;
        let present = dirs
            .library
            .join(&path)
            .metadata()
            .is_ok_and(|m| m.len() > 0)
            && written
                .lyrics
                .as_ref()
                .is_none_or(|l| dirs.library.join(l).exists());
        (present && written.plan.as_ref() == Some(&r.plan)).then_some(written)
    };
    let changed_since = |path: &Path| changed_since_written(&dirs.library, &old, path);
    let (due, guarded): (Vec<&PlannedSong>, Vec<&PlannedSong>) = planned
        .iter()
        .filter(|(_, r)| opts.force || current(r).is_none())
        .partition(|(_, r)| opts.force || !changed_since(&path_of(r)));

    if opts.dry_run {
        status(
            &manifest, &planned, &old, &state, &store, &listed, dirs, out,
        )?;
        drop(lock);
        return Ok(ok);
    }
    if let Some(run) = run.as_deref_mut() {
        run.outputs_before(&old);
    }

    let located: BTreeMap<SourceKey, Located> = listed
        .iter()
        .filter_map(|k| Some((k.clone(), store.locate(k)?)))
        .collect();
    if !due.is_empty() {
        // Owned before written: a crash between the two must not leave a
        // file no run would ever delete.
        let mut pending = state.clone();
        for (n, r) in &due {
            let path = path_of(r);
            let lyrics = Some(path.with_extension("lrc"));
            pending.outputs.entry(path).or_insert_with(|| Written {
                sources: manifest.songs[*n].sources.clone(),
                lyrics,
                plan: None,
                stamp: None,
            });
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
        }
    }
    if due.len() > 1 {
        crate::ui::info(out, &format!("Writing {} song(s)", due.len()))?;
    }
    let rendered: Vec<Result<Rendered>> = parallel::map(&due, parallel::builds(), |(n, r)| {
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
    });

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
    for (_, r) in &guarded {
        crate::ui::warning(
            out,
            &format!(
                "Left alone, changed since muman wrote it: {} (`sync --force` writes it again)",
                path_of(r).display()
            ),
        )?;
    }
    let mut changed = false;
    let mut written = 0_usize;
    for ((n, r), result) in due.iter().zip(rendered) {
        let song = &manifest.songs[*n];
        let name = name_of(song, Some(r), &state.facts);
        match result {
            Ok(done) => {
                changed = true;
                let verb = match old.get(&done.audio).and_then(|w| w.plan.as_ref()) {
                    Some(p) => format!("Updated ({})", changes(p, &r.plan).join(", ")),
                    None => "Added".to_string(),
                };
                crate::ui::success(out, &format!("{verb}: {}", done.audio.display()))?;
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
                outputs.insert(
                    done.audio.clone(),
                    Written {
                        sources: song.sources.clone(),
                        lyrics: done.lyrics,
                        plan: Some(r.plan.clone()),
                        stamp: store::stamp_text(&dirs.library.join(&done.audio)),
                    },
                );
            }
            Err(e) => {
                ok = false;
                failed.extend(song.sources.iter().cloned());
                crate::ui::error(out, &format!("Failed: {name}: {e:#}"))?;
            }
        }
    }
    let mut keep = |rel: &Path| match run.as_deref_mut() {
        Some(run) => run.keep(&dirs.library, rel),
        None => Ok(()),
    };
    let removed = prune(&dirs.library, &old, &mut outputs, &failed, &mut keep, out)?;
    let up_to_date = planned.len() - due.len() - guarded.len();
    if up_to_date > 0 {
        crate::ui::info(out, &format!("Up to date: {up_to_date} song(s)"))?;
    }

    state.outputs = outputs;
    // A removed song's measures stay while its sources do, so restoring
    // it measures nothing again.
    let mut kept = listed.clone();
    kept.extend(manifest.removed_keys().into_iter().filter(|k| store.has(k)));
    state.facts.retain(|k, _| kept.contains(k));
    state.failures.retain(|k, _| listed.contains(k));
    state
        .alignments
        .retain(|a| kept.contains(&a.a) && kept.contains(&a.b));
    state.save(home)?;
    manifest.save_locked(&lock)?;
    if changed || !removed.is_empty() {
        // Some players rescan only when the root or a folder directly inside
        // it is newer than their last run; a song three levels down changes
        // neither.
        touch(&dirs.library, out)?;
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

/// Whether a file muman wrote has changed since, by its size and
/// time: a tagger or player that rewrote it.
fn changed_since_written(library: &Path, old: &BTreeMap<PathBuf, Written>, path: &Path) -> bool {
    old.get(path)
        .and_then(|w| w.stamp.as_ref())
        .zip(store::stamp_text(&library.join(path)))
        .is_some_and(|(was, now)| *was != now)
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
            alignments: &state.alignments,
            lyrics: &manifest.lyrics,
            clean: &manifest.clean,
            albums: &albums,
            naming: &naming,
            audio: &manifest.settings.audio,
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
    separate(&mut planned);
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

/// Delete every file `old` lists that `outputs` does not, unless a song
/// that failed this run made it, which keeps it, each handed to `keep`
/// first. A file changed since it was written is left, and no longer
/// muman's. Returns what went.
fn prune<W: Write>(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    outputs: &mut BTreeMap<PathBuf, Written>,
    failed: &BTreeSet<SourceKey>,
    keep: &mut dyn FnMut(&Path) -> Result<()>,
    out: &mut W,
) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    let now: BTreeSet<String> = outputs.keys().map(|p| crate::relpath::folded(p)).collect();
    for (path, written) in old {
        // On a filesystem blind to case and composition, a song renamed
        // only so is the same file as its new name: deleting the old name
        // would delete the new song.
        if outputs.contains_key(path) || now.contains(&crate::relpath::folded(path)) {
            continue;
        }
        if written.sources.iter().any(|k| failed.contains(k)) {
            outputs.insert(path.clone(), written.clone());
            continue;
        }
        if changed_since_written(library, old, path) {
            crate::ui::warning(
                out,
                &format!(
                    "Left in place, changed since muman wrote it: {}; it is yours now",
                    path.display()
                ),
            )?;
            continue;
        }
        let mut held = false;
        for file in std::iter::once(path).chain(&written.lyrics) {
            if outputs.values().any(|w| w.lyrics.as_ref() == Some(file)) {
                continue;
            }
            keep(file)?;
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
            outputs.insert(path.clone(), written.clone());
            continue;
        }
        crate::ui::info(out, &format!("Removed: {}", path.display()))?;
    }
    remove_empty_folders(library, &removed);
    Ok(removed)
}

/// Remove each folder a removed file leaves empty, up to the library.
fn remove_empty_folders(library: &Path, removed: &[PathBuf]) {
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

/// Set a folder's time to now, so players that rescan by it notice.
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

/// Say, song by song, what a sync would write and why each aspect comes
/// from where it does, then what it would remove and what lies unused.
#[allow(clippy::too_many_arguments)]
fn status<W: Write>(
    manifest: &Manifest,
    planned: &[(usize, Resolved)],
    old: &BTreeMap<PathBuf, Written>,
    state: &State,
    store: &Store,
    listed: &BTreeSet<SourceKey>,
    dirs: &Dirs,
    out: &mut W,
) -> Result<()> {
    let library = &dirs.library;
    let mut made: BTreeSet<PathBuf> = BTreeSet::new();
    for (n, r) in planned {
        let song = &manifest.songs[*n];
        let path = path_of(r);
        let verdict = match old.get(&path).and_then(|w| w.plan.as_ref()) {
            Some(p) if p == &r.plan && library.join(&path).exists() => "up to date".to_string(),
            Some(p) => format!("changes: {}", changes(p, &r.plan).join(", ")),
            None => "new".to_string(),
        };
        writeln!(
            out,
            "{}",
            crate::ui::Style::Accent.paint(&name_of(song, Some(r), &state.facts))
        )?;
        writeln!(
            out,
            "  → {} ({verdict})",
            crate::ui::Style::Path.paint(&path.display().to_string())
        )?;
        writeln!(out, "  audio   {}: {}", r.plan.audio.key, r.why.audio)?;
        if let (Some(c), Some(why)) = (&r.plan.cover, &r.why.cover) {
            writeln!(out, "  cover   {}: {why}", c.key)?;
        }
        if let (Some(l), Some(why)) = (&r.plan.lyrics, &r.why.lyrics) {
            writeln!(out, "  lyrics  {}: {why}", l.key)?;
        }
        for ((key, values), why) in r.plan.tags.iter().zip(&r.why.tags) {
            let cleaned = if why.cleaned.is_empty() {
                String::new()
            } else {
                format!(", cleaned: {}", why.cleaned.join(", "))
            };
            writeln!(
                out,
                "  {key:<12} {} ({}{cleaned})",
                values.join("; "),
                why.from
            )?;
        }
        made.insert(path);
    }
    for path in old.keys().filter(|p| !made.contains(*p)) {
        crate::ui::warning(out, &format!("Would remove: {}", path.display()))?;
    }
    let mut known = listed.clone();
    known.extend(manifest.removed_keys());
    let (unlisted, settling) = store.unlisted(&known, SystemTime::now());
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
                path.display()
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
        crate::ui::info(out, &format!("Unused: {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
