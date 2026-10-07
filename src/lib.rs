//! Keep a music library in step with a song list: every song gathered
//! from its sources, the best of each aspect picked by measurement, and
//! the result written as one tagged, covered track with its lyrics.

pub mod acoustid;
pub mod acquire;
pub mod align;
pub mod atomic;
pub mod change;
pub mod check;
pub mod clean;
pub mod cli;
pub mod codec;
pub mod coverart;
pub mod dirs;
pub mod download;
pub mod duplicates;
pub mod editor;
pub mod export;
pub mod facts;
pub mod ffmpeg;
pub mod fingerprint;
pub mod fit;
pub mod history;
pub mod hooks;
pub mod http;
pub mod identify;
pub mod info;
pub mod limit;
pub mod lookup;
pub mod lrclib;
pub mod lyrics;
pub mod manifest;
pub mod music;
pub mod musicbrainz;
pub mod naming;
pub mod overview;
pub mod parallel;
pub mod platform;
pub mod plugins;
pub mod probe;
pub mod progress;
pub mod provider;
pub mod purge;
pub mod quality;
pub mod query;
pub mod reconcile;
pub mod relpath;
pub mod render;
pub mod resolve;
pub mod runner;
pub mod settings;
pub mod source;
pub mod state;
pub mod store;
pub mod tags;
pub mod template;
#[cfg(test)]
mod testing;
pub mod ui;
pub mod ytdlp_log;

use std::cell::Cell;
use std::collections::BTreeSet;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use crate::http::{HttpTransport, UreqTransport};
use anyhow::{Context, Result, bail};
use clap::FromArgMatches;

use crate::acquire::{Acquire, Additions, Fetcher, Proposal};
use crate::atomic::Lock;
use crate::cli::{Cli, Command, Matching};
use crate::dirs::Dirs;
use crate::history::Run;
use crate::identify::Mode;
use crate::manifest::{Edit, Manifest, Tags};
use crate::platform::Defaults;
use crate::reconcile::Options;
use crate::runner::{Runner, System, Traced};
use crate::source::{SourceKey, id_of};
use crate::state::State;
use crate::store::{Kind, Store};
use crate::ui::{InquirePrompter, Prompter};

/// One run, with every path resolved.
#[derive(Debug)]
pub struct Job {
    pub command: Command,
    pub dirs: Dirs,
    /// Where the yt-dlp plugins are written; none in tests, which run
    /// without them.
    pub cache: Option<PathBuf>,
    /// Whether output goes to a terminal, where yt-dlp's progress is one
    /// line rewritten in place.
    pub live: bool,
    /// Say each command run and why each new source matched or not.
    pub verbose: bool,
    /// How long a file dropped into the manual folder waits, so one still
    /// being copied in is not read half written.
    pub settling: Duration,
    /// The pace of every request to each service in the run.
    pub throttles: lookup::Throttles,
    /// How long steps say how far they have got.
    pub progress: progress::Mode,
}

impl Job {
    #[must_use]
    pub fn from_cli(cli: Cli, defaults: Defaults) -> Self {
        Self {
            dirs: Dirs {
                home: cli.home.unwrap_or(defaults.home),
                library: cli.library.unwrap_or(defaults.library),
            },
            command: cli.command,
            verbose: cli.verbose,
            settling: store::SETTLING,
            throttles: lookup::Throttles::polite(),
            cache: defaults.cache,
            live: false,
            progress: cli.progress,
        }
    }
}

#[must_use]
pub fn run() -> ExitCode {
    let cli = cli::command()
        .try_get_matches()
        .and_then(|m| Cli::from_arg_matches(&m))
        .map_err(|e| e.format(&mut cli::command()))
        .unwrap_or_else(|e| e.exit());
    let live = std::io::stderr().is_terminal();
    let kind = progress::Kind::of(cli.progress, live, taskbar_progress());
    let _progress = progress::install(progress::Progress::new(kind, Box::new(std::io::stderr())));
    let mut err = progress::Console::new(anstream::stderr());
    let Some(defaults) = platform::defaults() else {
        let _ = ui::error(&mut err, "the system names no home folder for this user");
        return ExitCode::from(2);
    };
    let home = cli.home.clone().unwrap_or_else(|| defaults.home.clone());
    // A song list that does not read is reported by the command itself,
    // which names the problem; the default folder stands meanwhile.
    let library = if cli.library.is_some() {
        None
    } else {
        settings::library_folder(&home).ok().flatten()
    };
    let defaults = Defaults {
        library: library.unwrap_or(defaults.library),
        ..defaults
    };
    let mut job = Job::from_cli(cli, defaults);
    job.live = live;
    let mut inquire = InquirePrompter;
    let prompter: Option<&mut dyn Prompter> =
        (std::io::stdin().is_terminal() && job.live).then_some(&mut inquire);
    let system = System::default();
    // yt-dlp is found when a run first fetches; a library made from
    // files alone never needs it.
    let needs_ffmpeg = match &job.command {
        Command::Info
        | Command::List { .. }
        | Command::Duplicates { .. }
        | Command::Purge { .. } => false,
        Command::Check { decode } => *decode,
        Command::Export { max_size, .. } => max_size.is_some(),
        _ => true,
    };
    if needs_ffmpeg && let Err(e) = system.require("ffmpeg").and(system.require("ffprobe")) {
        let _ = ui::error(&mut err, &format!("{e:#}"));
        return ExitCode::from(2);
    }
    let mut data = anstream::stdout();
    let http = UreqTransport;
    let ran = if job.verbose {
        run_with(&job, &Traced(system), &http, prompter, &mut err, &mut data)
    } else {
        run_with(&job, &system, &http, prompter, &mut err, &mut data)
    };
    match ran {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(4),
        Err(e) if e.downcast_ref::<runner::MissingTool>().is_some() => {
            let _ = ui::error(&mut err, &format!("{e:#}"));
            ExitCode::from(2)
        }
        Err(e) if e.downcast_ref::<change::Refused>().is_some() => {
            let _ = ui::error(&mut err, &format!("{e:#}"));
            ExitCode::from(5)
        }
        Err(e) => {
            let _ = ui::error(&mut err, &format!("{e:#}"));
            ExitCode::from(4)
        }
    }
}

/// Whether the terminal shows a task's progress in its tab or taskbar
/// from `OSC 9;4`, as Windows Terminal, `ConEmu`, `WezTerm` and Ghostty do;
/// another would print the sequence.
fn taskbar_progress() -> bool {
    let var = |name: &str| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned());
    var("WT_SESSION").is_some()
        || var("ConEmuANSI").as_deref() == Some("ON")
        || matches!(var("TERM_PROGRAM").as_deref(), Some("WezTerm" | "ghostty"))
}

fn mode(m: Matching) -> Mode {
    if m.new {
        Mode::New
    } else if m.yes {
        Mode::Yes
    } else {
        Mode::Ask
    }
}

/// Run one command, asking LRCLIB and MusicBrainz through `http`, writing
/// what it lists or reports, as `list`, `status`, `info` and `check` do,
/// to `data`, and every other message to `out`. Returns whether every
/// step succeeded.
#[allow(clippy::too_many_lines)]
pub fn run_with<R: Runner, W: Write, D: Write>(
    job: &Job,
    runner: &R,
    http: &(dyn HttpTransport + Sync),
    mut prompter: Option<&mut dyn Prompter>,
    out: &mut W,
    data: &mut D,
) -> Result<bool> {
    let dirs = &job.dirs;
    let sync = |opts: Options, run: Option<&mut Run>, out: &mut W| {
        let opts = Options {
            settling: job.settling,
            ..opts
        };
        reconcile::reconcile(runner, dirs, opts, run, out)
    };
    progress::current().plan(steps_of(&job.command));
    match &job.command {
        Command::Status => {
            let opts = Options {
                dry_run: true,
                settling: job.settling,
                ..Options::default()
            };
            reconcile::reconcile_into(runner, dirs, opts, None, out, Some(data))
        }
        Command::Info => {
            overview::info(dirs, job.verbose, data)?;
            Ok(true)
        }
        Command::Duplicates { query } => {
            duplicates::report(dirs, query, out, data)?;
            Ok(true)
        }
        Command::Purge { yes, dry_run } => {
            let confirm = cli::Confirm {
                yes: *yes,
                all: false,
                dry_run: *dry_run,
            };
            purge::purge(dirs, &confirm, prompter, out)?;
            Ok(true)
        }
        Command::List {
            query,
            format,
            keys,
            removed,
        } => {
            list_songs(dirs, query, format.as_deref(), *keys, *removed, data)?;
            Ok(true)
        }
        Command::Check { decode } => check::check(runner, dirs, *decode, out, data),
        Command::Export { output, max_size } => {
            export::export(runner, dirs, output, *max_size, out)
        }
        Command::Undo { yes, dry_run } => {
            let planned = history::plan(dirs)?;
            ui::info(out, "Undoing the last run:")?;
            for line in planned.lines() {
                writeln!(out, "  {line}")?;
            }
            let confirm = cli::Confirm {
                yes: *yes,
                all: false,
                dry_run: *dry_run,
            };
            if !change::confirmed(&confirm, prompter, out)? {
                return Ok(true);
            }
            {
                let _lock = Lock::folder(&dirs.home)?;
                let again = history::plan(dirs)?;
                if !again.same_run(&planned) {
                    return Err(change::Refused(
                        "Another run changed the history meanwhile; run `undo` again".into(),
                    )
                    .into());
                }
                history::undo(dirs, again, out)?;
            }
            sync(Options::default(), None, out)
        }
        Command::Add {
            inputs,
            no_match,
            matching,
            tags,
        } => {
            create(&dirs.home)?;
            let mut run = Run::begin(&dirs.home)?;
            let done = (|| {
                let (files, urls): (Vec<&String>, Vec<&String>) =
                    inputs.iter().partition(|i| Path::new(i).exists());
                let mut ok = true;
                let mut proposals = import(runner, dirs, &files, out)?;
                if !urls.is_empty() {
                    let urls: Vec<String> = urls.into_iter().cloned().collect();
                    let (fine, additions) =
                        network(job, runner, out, |acquire| acquire.urls(&urls, !no_match))?;
                    ok &= fine;
                    proposals.extend(record(dirs, additions)?);
                }
                let declined: BTreeSet<SourceKey> = if *no_match {
                    proposals.iter().flat_map(|p| p.sources.clone()).collect()
                } else {
                    BTreeSet::new()
                };
                let how = Listing {
                    mode: mode(*matching),
                    verbose: job.verbose,
                    tags: tags.tags(),
                };
                list(runner, dirs, proposals, &how, prompter, out)?;
                ok &= look_up(job, runner, http, false, &declined, out)?;
                ok &= sync(Options::default(), Some(&mut run), out)?;
                Ok(ok)
            })();
            recorded(run, &dirs.home, done)
        }
        Command::Sync {
            rematch,
            force,
            retry,
            update_defaults,
            matching,
        } => {
            create(&dirs.home)?;
            let mut run = Run::begin(&dirs.home)?;
            let done = (|| {
                if *update_defaults {
                    update_settings(&dirs.home, out)?;
                }
                let proposals = dropped_in(dirs, job.settling, out)?;
                let manifest = Manifest::load(&dirs.home)?;
                let mut ok = fetch_missing(job, runner, http, &manifest, *retry, out)?;
                let how = Listing {
                    mode: mode(*matching),
                    verbose: job.verbose,
                    tags: Vec::new(),
                };
                list(runner, dirs, proposals, &how, prompter, out)?;
                ok &= look_up(job, runner, http, *rematch, &BTreeSet::new(), out)?;
                let opts = Options {
                    force: *force,
                    retry: *retry,
                    ..Options::default()
                };
                ok &= sync(opts, Some(&mut run), out)?;
                Ok(ok)
            })();
            recorded(run, &dirs.home, done)
        }
        Command::Remove {
            query,
            purge,
            confirm,
        } => {
            let mut run = Run::begin(&dirs.home)?;
            let done = (|| {
                if !change::remove(dirs, query, *purge, confirm, prompter, out)? {
                    return Ok(true);
                }
                sync(Options::default(), Some(&mut run), out)
            })();
            recorded(run, &dirs.home, done)
        }
        Command::Restore { query, confirm } => {
            let mut run = Run::begin(&dirs.home)?;
            let done = (|| {
                if !change::restore(dirs, query, confirm, prompter, out)? {
                    return Ok(true);
                }
                let manifest = Manifest::load(&dirs.home)?;
                let mut ok = fetch_missing(job, runner, http, &manifest, true, out)?;
                ok &= sync(Options::default(), Some(&mut run), out)?;
                Ok(ok)
            })();
            recorded(run, &dirs.home, done)
        }
        Command::Set { terms, confirm } => {
            let mut run = Run::begin(&dirs.home)?;
            let done = (|| {
                if !change::set(dirs, terms, confirm, prompter, out)? {
                    return Ok(true);
                }
                sync(Options::default(), Some(&mut run), out)
            })();
            recorded(run, &dirs.home, done)
        }
        Command::Edit { query, all } => {
            let mut run = Run::begin(&dirs.home)?;
            let done = (|| {
                let edited = editor::edit(
                    dirs,
                    query,
                    *all,
                    change::reborrow(&mut prompter),
                    &mut editor::launch,
                    out,
                )?;
                if !edited {
                    return Ok(true);
                }
                sync(Options::default(), Some(&mut run), out)
            })();
            recorded(run, &dirs.home, done)
        }
    }
}

/// The long steps a command may take, in the order it takes them, which
/// number them as it goes.
fn steps_of(command: &Command) -> &'static [&'static str] {
    const SYNC: &[&str] = &[
        "Fetching",
        "Measuring",
        "Looking up",
        "Comparing",
        "Writing",
    ];
    match command {
        Command::Add { .. } | Command::Sync { .. } => SYNC,
        Command::Restore { .. } => &["Fetching", "Measuring", "Comparing", "Writing"],
        Command::Remove { .. }
        | Command::Set { .. }
        | Command::Edit { .. }
        | Command::Undo { .. } => &["Measuring", "Comparing", "Writing"],
        Command::Status => &["Measuring", "Comparing"],
        Command::Check { .. } => &["Decoding"],
        Command::Export { .. } => &["Encoding", "Writing the zip"],
        Command::Duplicates { .. } => &["Comparing prints"],
        Command::List { .. } | Command::Info | Command::Purge { .. } => &[],
    }
}

/// Finish `run` whatever became of its command, so one that stopped on
/// an error is recorded as far as it got; the command's own error wins.
fn recorded(run: Run, home: &Path, done: Result<bool>) -> Result<bool> {
    let finished = run.finish(home);
    let ok = done?;
    finished?;
    Ok(ok)
}

/// Write the songs the query matches to `data`, or the removed ones.
fn list_songs<D: Write>(
    dirs: &Dirs,
    terms: &[String],
    format: Option<&str>,
    keys: bool,
    removed: bool,
    data: &mut D,
) -> Result<()> {
    let manifest = Manifest::load(&dirs.home)?;
    let parsed = query::Query::parse(terms, &query::extractors(&manifest))?;
    let views = if removed {
        manifest.removed.iter().map(query::removed_view).collect()
    } else {
        query::views(&manifest, &State::load(&dirs.home)?, &dirs.library)?
    };
    parsed.check_fields(&views)?;
    let template = match (format, keys, removed) {
        (Some(f), _, _) => f,
        (None, true, _) => "{key}",
        (None, false, true) => "{key}\\t{title}",
        (None, false, false) => "{key}\\t{artist}\\t{title}\\t{album}",
    };
    for view in views
        .iter()
        .filter(|v| !v.keys.is_empty() && parsed.matches(v))
    {
        match writeln!(data, "{}", query::format(template, view)) {
            // A reader such as `head` that has read enough closes the pipe.
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
            written => written?,
        }
    }
    Ok(())
}

/// Make every lookup due, as `lookup::run` does.
fn look_up<R: Runner, W: Write>(
    job: &Job,
    runner: &R,
    http: &(dyn HttpTransport + Sync),
    force: bool,
    declined: &BTreeSet<SourceKey>,
    out: &mut W,
) -> Result<bool> {
    let (ok, ()) = network(job, runner, out, |acquire| {
        Ok((
            lookup::run(acquire, &job.dirs, http, &job.throttles, force, declined)?,
            (),
        ))
    })?;
    Ok(ok)
}

/// Fetch again every listed source gone from the store, those that
/// failed lately left for later unless `retry`, and record each that
/// fails. Returns whether every one fetched arrived.
fn fetch_missing<R: Runner, W: Write>(
    job: &Job,
    runner: &R,
    http: &(dyn HttpTransport + Sync),
    manifest: &Manifest,
    retry: bool,
    out: &mut W,
) -> Result<bool> {
    let mut state = State::load(&job.dirs.home)?;
    let (mut ok, mut failed) = network(job, runner, out, |acquire| {
        acquire.missing(manifest, &state.failures, retry)
    })?;
    let records = lookup::refetch(&job.dirs, manifest, http, &job.throttles)?;
    for (key, error) in &records {
        ok = false;
        ui::error(out, &format!("Could not fetch {key} again: {error}"))?;
    }
    failed.extend(records);
    let store = Store::scan(&job.dirs)?;
    let arrived: Vec<SourceKey> = state
        .failures
        .iter()
        .filter(|(k, f)| f.step == state::Step::Fetch && store.has(k))
        .map(|(k, _)| k.clone())
        .collect();
    for key in &arrived {
        state.clear_failure(key);
    }
    if !failed.is_empty() || !arrived.is_empty() {
        for (key, error) in failed {
            state.record_failure(&key, state::Step::Fetch, None, error);
        }
        State::keep_measures(&job.dirs.home, &state)?;
    }
    Ok(ok)
}

fn create(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))
}

/// Move the song list's settings still at an earlier edition's default
/// to the current one, saying which moved.
fn update_settings<W: Write>(home: &Path, out: &mut W) -> Result<()> {
    let mut manifest = Manifest::load(home)?;
    let moved = manifest.stale_defaults.clone();
    manifest.edit(Edit::UpdateDefaults);
    manifest.save()?;
    if moved.is_empty() {
        ui::info(out, "Every setting is at this muman's defaults")?;
    }
    for s in &moved {
        ui::info(out, &format!("Updated: `{s}` to {}", s.now))?;
    }
    Ok(())
}

/// Run one network step with a fetcher into the store, and every key a
/// song lists or a release replaced known.
fn network<R: Runner, W: Write, T>(
    job: &Job,
    runner: &R,
    out: &mut W,
    step: impl FnOnce(&mut Acquire<'_, R, W>) -> Result<(bool, T)>,
) -> Result<(bool, T)> {
    let manifest = Manifest::load(&job.dirs.home)?;
    let state = State::load(&job.dirs.home)?;
    let store = Store::scan(&job.dirs)?;
    let temp = tempfile::tempdir().context("creating a temporary directory")?;
    let ytdlp = job.dirs.ytdlp();
    let partial = job.dirs.home.join("partial");
    clear_stale(&partial, manifest.settings.ytdlp.partial_days);
    let plugins = match &job.cache {
        Some(cache) if manifest.settings.ytdlp.plugins => Some(plugins::folder(cache)?),
        _ => None,
    };
    let fetcher = Fetcher {
        store: &ytdlp,
        temp: temp.path(),
        partial: &partial,
        plugins: plugins.as_deref(),
        options: &manifest.settings.ytdlp,
        live: job.live,
        runs: Cell::new(0),
    };
    let removed = manifest.removed_keys();
    let mut known: BTreeSet<SourceKey> = manifest.keys();
    known.extend(removed.iter().cloned());
    known.extend(state.replaced.keys().cloned());
    let mut acquire = Acquire {
        runner,
        fetcher: &fetcher,
        out,
        known,
        removed,
        store: &store,
    };
    step(&mut acquire)
}

/// Remove what yt-dlp left unfinished longer than `days` ago, in any
/// folder its template makes, and the folders that leaves empty;
/// younger, a later run resumes it.
fn clear_stale(partial: &Path, days: u64) {
    let limit = std::time::Duration::from_secs(days * 24 * 3600);
    let mut folders = Vec::new();
    let mut left = vec![partial.to_path_buf()];
    while let Some(dir) = left.pop() {
        for path in std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(|e| Some(e.ok()?.path()))
        {
            if path.is_dir() {
                left.push(path.clone());
                folders.push(path);
                continue;
            }
            let old = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > limit));
            if old {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    // Deepest first; only a folder left empty goes.
    folders.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for dir in folders {
        let _ = std::fs::remove_dir(dir);
    }
}

/// Record what a network step found beyond its proposals: albums and
/// renames in the song list, replaced uploads in the state.
fn record(dirs: &Dirs, additions: Additions) -> Result<Vec<Proposal>> {
    let mut manifest = Manifest::load(&dirs.home)?;
    for (source, tracks) in additions.albums {
        manifest.edit(Edit::AddAlbum {
            source,
            tracks: Some(tracks),
        });
    }
    for (from, to) in additions.renames {
        manifest.edit(Edit::Rename { from, to });
    }
    manifest.save()?;
    State::keep_lookups(&dirs.home, &additions.looked_up, &additions.replaced)?;
    Ok(additions.proposals)
}

/// How proposals are listed.
struct Listing {
    mode: Mode,
    verbose: bool,
    /// Set on every song a proposal is listed as or added to.
    tags: Tags,
}

/// List each proposal as a song, or as a source of the song it is the
/// same recording as, and set the tags given on each.
fn list<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    proposals: Vec<Proposal>,
    how: &Listing,
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<()> {
    let edits = identify::identify(
        runner,
        dirs,
        proposals,
        how.mode,
        how.verbose,
        prompter,
        out,
    )?;
    if edits.is_empty() && !how.tags.is_empty() {
        ui::warning(out, "No song was added, so no tag was set")?;
    }
    let mut manifest = Manifest::load(&dirs.home)?;
    for edit in edits {
        let tagged = match &edit {
            Edit::Add { sources, .. } if !how.tags.is_empty() => manifest::id_of(sources).cloned(),
            _ => None,
        };
        manifest.edit(edit);
        if let Some(key) = tagged {
            manifest.edit(Edit::Tag {
                key,
                tags: how.tags.clone(),
            });
        }
    }
    manifest.save()
}

/// Song files dropped into the manual folder that no song lists, each a
/// proposal; a listed file moved within the folder is followed instead.
fn dropped_in<W: Write>(dirs: &Dirs, wait: Duration, out: &mut W) -> Result<Vec<Proposal>> {
    let mut manifest = Manifest::load(&dirs.home)?;
    let state = State::load(&dirs.home)?;
    let store = Store::scan(dirs)?;
    let listed = manifest.keys();
    let mut known = listed.clone();
    known.extend(manifest.removed_keys());
    let (mut ready, settling) = store.unlisted(&known, SystemTime::now(), wait);
    for path in settling {
        ui::info(
            out,
            &format!(
                "Still being copied in, left for a later run: {}",
                SourceKey::Manual(path)
            ),
        )?;
    }
    let gone = listed
        .iter()
        .filter(|k| matches!(k, SourceKey::Manual(_)) && !store.has(k));
    for gone in gone {
        let Some(was) = state
            .facts
            .get(gone)
            .and_then(|f| store::stamp_of_rev(&f.rev))
        else {
            continue;
        };
        let moved = ready
            .iter()
            .position(|k| store.locate(k).and_then(|l| store::stamp(&l.path)) == Some(was));
        if let Some(at) = moved {
            let to = ready.remove(at);
            ui::info(out, &format!("Moved: {gone} is now {to}"))?;
            manifest.edit(Edit::Rename {
                from: gone.clone(),
                to,
            });
        }
    }
    manifest.save()?;
    Ok(ready
        .into_iter()
        .map(|key| Proposal {
            label: key.short(),
            sources: vec![key],
            album: None,
        })
        .collect())
}

/// Copy files and folders the user named into the sources: an original
/// yt-dlp fetched into the store under its key, anything else into the
/// manual folder with the lyrics and pictures named as it is. Never
/// moves or overwrites; a name already taken is an error.
fn import<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    paths: &[&String],
    out: &mut W,
) -> Result<Vec<Proposal>> {
    let mut proposals = Vec::new();
    for path in paths.iter().map(Path::new) {
        let name = path
            .file_name()
            .with_context(|| format!("{} names no file", path.display()))?;
        if let Some(key) = fetched_key(runner, path) {
            let folder = path
                .parent()
                .and_then(Path::file_name)
                .map_or_else(|| "unknown".into(), std::ffi::OsStr::to_os_string);
            copy_new(path, &dirs.ytdlp().join(folder).join(name))?;
            ui::info(
                out,
                &format!("Copied {} into the store as {key}", path.display()),
            )?;
            proposals.push(Proposal {
                label: key.short(),
                sources: vec![key],
                album: None,
            });
            continue;
        }
        let to = dirs.manual().join(name);
        if path.is_dir() {
            copy_tree(path, &to)?;
        } else {
            copy_new(path, &to)?;
            for sidecar in sidecars(path) {
                if let Some(n) = sidecar.file_name() {
                    copy_new(&sidecar, &dirs.manual().join(n))?;
                }
            }
        }
        ui::info(
            out,
            &format!("Copied {} into {}", path.display(), dirs.manual().display()),
        )?;
        let store = Store::scan(dirs)?;
        for key in media_at(&store, dirs, &to)? {
            proposals.push(Proposal {
                label: key.short(),
                sources: vec![key],
                album: None,
            });
        }
    }
    Ok(proposals)
}

/// The song files at or under `path` in the manual folder.
fn media_at(store: &Store, dirs: &Dirs, path: &Path) -> Result<Vec<SourceKey>> {
    let files = if path.is_dir() {
        walk_all(path)?
    } else {
        vec![path.to_path_buf()]
    };
    Ok(files
        .into_iter()
        .filter_map(|f| f.strip_prefix(dirs.manual()).ok().map(Path::to_path_buf))
        .map(SourceKey::Manual)
        .filter(|k| store.locate(k).is_some_and(|l| l.kind == Kind::Media))
        .collect())
}

fn walk_all(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            found.extend(walk_all(&path)?);
        } else {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

/// The key of an original yt-dlp fetched, from the info JSON inside it.
fn fetched_key<R: Runner>(runner: &R, path: &Path) -> Option<SourceKey> {
    id_of(path)?;
    let probed = probe::parse(&runner.output(&probe::ffprobe_command(path)).ok()?).ok()?;
    let attachment = probed.attachment_named("info.json")?;
    let temp = tempfile::tempdir().ok()?;
    let json = temp.path().join("info.json");
    runner
        .run(&ffmpeg::dump_command(&[(
            path,
            vec![(attachment.ordinal, json.clone())],
        )]))
        .ok()?;
    let info = info::parse(&std::fs::read(json).ok()?).ok()?;
    let extractor = info.extractor_key?.to_ascii_lowercase();
    SourceKey::parse(&format!("{extractor}:{}", info.id?)).ok()
}

/// The `.lrc` and pictures named as a file is, beside it.
fn sidecars(path: &Path) -> Vec<PathBuf> {
    ["lrc", "jpg", "jpeg", "png", "webp"]
        .iter()
        .map(|ext| path.with_extension(ext))
        .filter(|p| p != path && p.is_file())
        .collect()
}

fn copy_new(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        bail!("{} exists already; rename one of them first", to.display());
    }
    if let Some(dir) = to.parent() {
        create(dir)?;
    }
    std::fs::copy(from, to)
        .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        bail!("{} exists already; rename one of them first", to.display());
    }
    for file in walk_all(from)? {
        let rel = file.strip_prefix(from).unwrap_or(&file);
        copy_new(&file, &to.join(rel))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
