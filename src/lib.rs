//! Keep a music library in step with a song list: every song gathered
//! from its sources, the best of each aspect picked by measurement, and
//! the result written as one tagged, covered track with its lyrics.

pub mod acoustid;
pub mod acquire;
pub mod align;
pub mod analysis;
pub mod atomic;
pub mod attach;
pub mod change;
pub mod check;
pub mod clean;
pub mod cli;
pub mod codec;
pub mod coverart;
pub mod cue;
pub mod dirs;
pub mod download;
pub mod duplicates;
pub mod editor;
pub mod export;
pub mod facts;
pub mod ffmpeg;
pub mod fingerprint;
pub mod fit;
pub mod held;
pub mod history;
pub mod hooks;
pub mod http;
pub mod identify;
pub mod info;
pub mod library;
pub mod limit;
pub mod lookup;
pub mod loudness;
pub mod lrclib;
pub mod lyrics;
pub mod manifest;
pub mod migrate;
pub mod music;
pub mod musicbrainz;
pub mod naming;
pub mod ogg;
pub mod overview;
pub mod parallel;
pub mod picture;
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
pub mod similar;
pub mod sites;
pub mod source;
pub mod state;
pub mod store;
pub mod tagfile;
pub mod tags;
pub mod template;
#[cfg(test)]
mod testing;
pub mod ui;
pub mod units;
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
    // Recorded in the state, a folder named relative to where one run
    // started would be another folder to a run started elsewhere.
    for dir in [&mut job.dirs.home, &mut job.dirs.library] {
        if let Ok(full) = std::path::absolute(&*dir) {
            // Without a trailing `/` or `.`, so one folder is one spelling.
            *dir = full.components().collect();
        }
    }
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
        Command::Check { decode, .. } => *decode,
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
        // A reader that stopped early, as `| head` does, wants no more.
        Err(e)
            if e.chain().any(|c| {
                c.downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
            }) =>
        {
            ExitCode::from(4)
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
    let writes = matches!(
        job.command,
        Command::Add { .. }
            | Command::Sync { .. }
            | Command::Remove { .. }
            | Command::Restore { .. }
            | Command::Set { .. }
            | Command::Edit { .. }
    );
    if writes {
        crate::manifest::present(&dirs.home)?;
        say_renamed(&dirs.home, out)?;
    }
    match &job.command {
        Command::Status { query, all } => {
            // Read only: a home not made yet is not made by asking.
            if !dirs.home.exists() {
                ui::info(
                    out,
                    &format!("No home at {}: nothing listed yet", dirs.home.display()),
                )?;
                return Ok(true);
            }
            let opts = Options {
                dry_run: true,
                settling: job.settling,
                ..Options::default()
            };
            let report = reconcile::Report {
                out: data,
                query,
                all: *all,
            };
            reconcile::reconcile_into(runner, dirs, opts, None, out, Some(report))
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
        Command::Check { decode, upstream } => {
            check::check(runner, dirs, (*decode, *upstream), out, data)
        }
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
            // As a run holds it, so no run begins between the undo and its sync.
            let _runs = Lock::runs(&dirs.home)?;
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
            to,
            no_match,
            matching,
            tags,
        } => {
            create(&dirs.home)?;
            let mut run = Run::begin(&dirs.home)?;
            let mut left_out = 0;
            let mut unread = Vec::new();
            let mut refused = None;
            let done = (|| {
                let (files, urls): (Vec<&String>, Vec<&String>) =
                    inputs.iter().partition(|i| Path::new(i).exists());
                if let Some(missing) = urls.iter().find(|u| names_a_path(u)) {
                    bail!("{missing} is no file");
                }
                let (loose, files) = split_loose(&files)?;
                let mut ok = true;
                let sites = Manifest::load(&dirs.home)?.settings.sites;
                let mut beside = Vec::new();
                let mut proposals = import(runner, dirs, &sites, &files, &mut beside, out)?;
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
                    loose: !loose.is_empty(),
                };
                let mut prompter = prompter;
                let listed = list(
                    runner,
                    dirs,
                    proposals,
                    &how,
                    change::reborrow(&mut prompter),
                    out,
                )?;
                let attaching = attach::How {
                    yes: matching.yes,
                    to: to.clone(),
                    verbose: job.verbose,
                };
                // A loose file's refusal waits for the songs added with it
                // to be written.
                let attached = match attach::attach(
                    runner, dirs, &loose, &beside, &attaching, prompter, out,
                ) {
                    Ok(a) => a,
                    Err(e) if e.downcast_ref::<change::Refused>().is_some() => {
                        refused = Some(e);
                        attach::Attached::default()
                    }
                    Err(e) => return Err(e),
                };
                left_out = attached.left_out;
                unread = attached.unread;
                give_tags(dirs, &attached.songs, &how, listed, out)?;
                ok &= look_up(job, runner, http, false, &declined, out)?;
                ok &= sync(Options::default(), Some(&mut run), out)?;
                Ok(ok)
            })();
            let ok = recorded(run, &dirs.home, done)?;
            if let Some(e) = refused {
                return Err(e);
            }
            if !unread.is_empty() {
                bail!(
                    "{} file(s) could not be added: {}",
                    unread.len(),
                    unread.join(", ")
                );
            }
            if left_out > 0 {
                return Err(change::Refused(format!(
                    "{left_out} file(s) were not added: name their songs with --to, give -y, \
                     or run on a terminal"
                ))
                .into());
            }
            Ok(ok)
        }
        Command::Sync {
            rematch,
            force,
            retry,
            accept,
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
                    loose: false,
                };
                list(runner, dirs, proposals, &how, prompter, out)?;
                ok &= look_up(job, runner, http, *rematch, &BTreeSet::new(), out)?;
                let opts = Options {
                    force: *force,
                    retry: *retry,
                    accept: *accept,
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
            // Begun once the editor is closed, so an open editor holds no
            // other run back.
            let mut run: Option<Run> = None;
            let edited = editor::edit(
                dirs,
                query,
                *all,
                change::reborrow(&mut prompter),
                &mut editor::launch,
                &mut || {
                    if run.is_none() {
                        run = Some(Run::begin(&dirs.home)?);
                    }
                    Ok(())
                },
                out,
            );
            let Some(mut run) = run else {
                return edited.map(|_| true);
            };
            let done = edited.and_then(|edited| {
                if edited {
                    sync(Options::default(), Some(&mut run), out)
                } else {
                    Ok(true)
                }
            });
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
        Command::Status { .. } => &["Measuring", "Comparing"],
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
    query::check_format(template, &views)?;
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

/// Say each setting the song list names the old way, which the command
/// about to write it renames; a list that does not read says so later.
fn say_renamed<W: Write>(home: &Path, out: &mut W) -> Result<()> {
    if let Ok(manifest) = Manifest::load(home) {
        let respelled = manifest.respelled_notice();
        for renamed in manifest
            .renamed
            .iter()
            .map(ToString::to_string)
            .chain(respelled)
        {
            ui::info(out, &format!("Renamed in the song list: {renamed}"))?;
        }
    }
    Ok(())
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
    let temp = atomic::Scratch::new()?;
    let ytdlp = job.dirs.ytdlp();
    let partial = job.dirs.home.join("partial");
    clear_stale(&partial, manifest.settings.ytdlp.keep_partial);
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
        sites: &manifest.settings.sites,
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
fn clear_stale(partial: &Path, limit: std::time::Duration) {
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
    /// Whether loose files may yet be given to songs, and so the tags.
    loose: bool,
}

/// List each proposal as a song, or as a source of the song it is the
/// same recording as, and set the tags given on each: whether any song
/// was given them.
fn list<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    proposals: Vec<Proposal>,
    how: &Listing,
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    let edits = identify::identify(
        runner,
        dirs,
        proposals,
        how.mode,
        how.verbose,
        prompter,
        out,
    )?;
    if edits.is_empty() && !how.tags.is_empty() && !how.loose {
        ui::warning(out, "No song was added, so no tag was set")?;
    }
    let mut manifest = Manifest::load(&dirs.home)?;
    let mut any = false;
    for edit in edits {
        let tagged = match &edit {
            Edit::Add { sources, .. } if !how.tags.is_empty() => manifest::id_of(sources).cloned(),
            _ => None,
        };
        manifest.edit(edit);
        if let Some(key) = tagged {
            any = true;
            manifest.edit(Edit::Tag {
                key,
                tags: how.tags.clone(),
            });
        }
    }
    manifest.save()?;
    Ok(any)
}

/// Set the tags given on the command line on every song a loose file was
/// given to, after the file's own, so the command line wins.
fn give_tags<W: Write>(
    dirs: &Dirs,
    songs: &[SourceKey],
    how: &Listing,
    listed: bool,
    out: &mut W,
) -> Result<()> {
    if how.tags.is_empty() || !how.loose {
        return Ok(());
    }
    if songs.is_empty() && !listed {
        return Ok(ui::warning(
            out,
            "No song was given a file, so no tag was set",
        )?);
    }
    let mut manifest = Manifest::load(&dirs.home)?;
    let mut seen = BTreeSet::new();
    for key in songs.iter().filter(|k| seen.insert(*k)) {
        manifest.edit(Edit::Tag {
            key: key.clone(),
            tags: how.tags.clone(),
        });
    }
    manifest.save()
}

/// The files of `files` `add` gives to songs rather than lists, each with
/// what it holds, and the rest. A loose file named as an audio file given
/// with it, in its folder, is that file's sidecar and copied with it.
fn split_loose<'a>(files: &[&'a String]) -> Result<(attach::Loose, Vec<&'a String>)> {
    let mut loose = Vec::new();
    let mut rest = Vec::new();
    for file in files {
        match attach::sort(Path::new(file))? {
            Some(what) => loose.push((PathBuf::from(file), what)),
            None => rest.push(*file),
        }
    }
    let canonical = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let audio: Vec<PathBuf> = rest
        .iter()
        .map(|f| PathBuf::from(f.as_str()))
        .filter(|p| p.is_file())
        .collect();
    let beside: Vec<PathBuf> = audio
        .iter()
        .flat_map(|a| sidecars(a))
        .map(|p| canonical(&p))
        .collect();
    loose.retain(|(path, _)| !beside.contains(&canonical(path)));
    Ok((loose, rest))
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
    for twin in store.twins() {
        ui::warning(
            out,
            &format!(
                "Not read, as another file in the manual folder has its name once \
                 normalized: {}; rename one of them",
                relpath::show(twin)
            ),
        )?;
    }
    let (mut ready, mut settling) = store.unlisted(&known, SystemTime::now(), wait);
    // A file listed before is no new one still copying in, however
    // recently it was touched.
    settling.retain(|path| {
        let key = SourceKey::Manual(path.into());
        let was_listed = state.listed.contains(&key);
        if was_listed {
            ready.push(key);
        }
        !was_listed
    });
    let unlisted: Vec<SourceKey> = ready
        .iter()
        .filter(|k| state.listed.contains(k))
        .cloned()
        .collect();
    ready.retain(|k| !unlisted.contains(k));
    for key in unlisted {
        ui::info(
            out,
            &format!(
                "Taken out of the song list by hand, so kept out: {key}; \
                 `muman restore` lists it again"
            ),
        )?;
        manifest.edit(Edit::Tombstone {
            note: format!("{key}, taken out of the song list by hand"),
            key,
        });
    }
    for path in settling {
        ui::info(
            out,
            &format!(
                "Still being copied in, left for a later run: {}",
                SourceKey::Manual(path.into())
            ),
        )?;
    }
    let renames = store::moved_manual(&listed, &state, &store, &mut ready);
    for (gone, to) in &renames {
        ui::info(out, &format!("Moved: {gone} is now {to}"))?;
        manifest.edit(Edit::Rename {
            from: gone.clone(),
            to: to.clone(),
        });
    }
    manifest.save()?;
    // So the song's plan names its file anew and the song is not written
    // again for a file that only moved.
    if !renames.is_empty() {
        let _lock = atomic::Lock::folder(&dirs.home)?;
        let mut state = State::load(&dirs.home)?;
        state.rekey(&renames)?;
        state.save(&dirs.home)?;
    }
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
/// moves or overwrites; a name already taken is an error. Each song file
/// copied is noted in `beside` with where it came from, for the loose
/// files given with it.
fn import<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    sites: &crate::sites::Sites,
    paths: &[&String],
    beside: &mut Vec<(PathBuf, SourceKey)>,
    out: &mut W,
) -> Result<Vec<Proposal>> {
    let mut proposals = Vec::new();
    for path in paths.iter().map(Path::new) {
        let name = path
            .file_name()
            .with_context(|| format!("{} names no file", path.display()))?;
        if let Some(key) = fetched_key(runner, sites, path) {
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
        if path.is_file() && store::kind_of(path).is_none() {
            bail!(
                "{} is no song, lyrics, picture or tags muman reads",
                path.display()
            );
        }
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
            if let SourceKey::Manual(m) = &key
                && let Some(folder) = path.parent()
            {
                let rel = m.path().strip_prefix(name).unwrap_or(m.path());
                let origin = if path.is_dir() {
                    path.join(rel)
                } else {
                    folder.join(m.path())
                };
                beside.push((origin, key.clone()));
            }
            proposals.push(Proposal {
                label: key.short(),
                sources: vec![key],
                album: None,
            });
        }
    }
    Ok(proposals)
}

/// Whether `input`, which names nothing on disk, was meant as a path
/// rather than an address for yt-dlp: written from a folder, or a file
/// name muman reads, with no scheme.
fn names_a_path(input: &str) -> bool {
    let from_folder = ["/", "./", "../", "~"].iter().any(|p| input.starts_with(p))
        || input.contains('\\')
        || input.as_bytes().get(1) == Some(&b':')
            && input
                .as_bytes()
                .get(2)
                .is_some_and(|b| *b == b'/' || *b == b'\\');
    let loose = Path::new(input)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| attach::LOOSE.contains(&e.to_ascii_lowercase().as_str()));
    let a_file = !input.contains("://") && (store::kind_of(Path::new(input)).is_some() || loose);
    !input.contains("://") && from_folder || a_file
}

/// The song files at or under `path` in the manual folder.
fn media_at(store: &Store, dirs: &Dirs, path: &Path) -> Result<Vec<SourceKey>> {
    let files = if path.is_dir() {
        crate::store::files_below(path, usize::MAX, |_| true)?
    } else {
        vec![path.to_path_buf()]
    };
    Ok(files
        .into_iter()
        .filter_map(|f| {
            Some(SourceKey::Manual(
                f.strip_prefix(dirs.manual()).ok()?.into(),
            ))
        })
        .filter(|k| store.locate(k).is_some_and(|l| l.kind == Kind::Media))
        .collect())
}

/// The key of an original yt-dlp fetched, from the info JSON inside it.
fn fetched_key<R: Runner>(
    runner: &R,
    sites: &crate::sites::Sites,
    path: &Path,
) -> Option<SourceKey> {
    id_of(path)?;
    let probed = probe::parse(&runner.output(&probe::ffprobe_command(path)).ok()?).ok()?;
    let attachment = probed.attachment_named("info.json")?;
    let temp = atomic::Scratch::new().ok()?;
    let json = temp.path().join("info.json");
    runner
        .run(&ffmpeg::dump_command(&[(
            path,
            vec![(attachment.ordinal, json.clone())],
        )]))
        .ok()?;
    let info = info::parse(&std::fs::read(json).ok()?).ok()?;
    info.key(sites)
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
    for file in crate::store::files_below(from, usize::MAX, |_| true)? {
        let rel = file.strip_prefix(from).unwrap_or(&file);
        copy_new(&file, &to.join(rel))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
