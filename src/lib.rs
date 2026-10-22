//! Keep a music library in step with a song list: every song gathered
//! from its sources, the best of each aspect picked by measurement, and
//! the result written as one tagged, covered track with its lyrics.

pub mod acoustid;
pub mod acquire;
pub mod add;
pub mod align;
pub mod analysis;
pub mod aria2;
pub mod atomic;
pub mod attach;
pub mod change;
pub mod chapters;
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
pub mod integrity;
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
pub mod sync;
pub mod tagfile;
pub mod tags;
pub mod template;
#[cfg(test)]
mod testing;
pub mod tools;
pub mod torrent;
pub mod trust;
pub mod ui;
pub mod units;
pub mod ytdlp_log;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use crate::http::{HttpTransport, UreqTransport};
use anyhow::{Context, Result};
use clap::FromArgMatches;

use crate::atomic::Lock;
use crate::cli::{Cli, Command, Matching};
use crate::dirs::Dirs;
use crate::history::Run;
use crate::identify::Mode;
use crate::manifest::Manifest;
use crate::platform::Defaults;
use crate::reconcile::Options;
use crate::runner::{Runner, System, Traced};
use crate::state::State;
use crate::ui::{InquirePrompter, Prompter};

/// One run, with every path resolved.
#[derive(Debug)]
// Each is a flag of its own on the command line.
#[expect(clippy::struct_excessive_bools)]
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
    /// Whether the run reaches no network.
    pub offline: bool,
    /// Trust the commands and servers the song list sets without asking
    /// ([`trust`]).
    pub trust: bool,
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
            offline: cli.offline,
            trust: cli.trust,
        }
    }
}

/// On Ctrl-C or a signal to stop, stop every program muman started, then
/// exit; one a program of the user's own in the terminal gets is its own.
fn stop_on_signals() {
    let _ = ctrlc::set_handler(|| {
        if runner::stop() {
            std::process::exit(i32::from(runner::STOPPED_EXIT));
        }
    });
}

#[must_use]
pub fn run() -> ExitCode {
    stop_on_signals();
    let cli = cli::command()
        .try_get_matches()
        .and_then(|m| Cli::from_arg_matches(&m))
        .map_err(|e| e.format(&mut cli::command()))
        .unwrap_or_else(|e| e.exit());
    if let Command::Completions { shell } = cli.command {
        return completions(shell);
    }
    let live = std::io::stderr().is_terminal();
    let kind = progress::Kind::of(cli.progress, live, taskbar_progress());
    let _progress = progress::install(progress::Progress::new(kind, Box::new(std::io::stderr())));
    let mut err = progress::Console::for_kind(anstream::stderr(), kind);
    let Some(defaults) = platform::defaults() else {
        let _ = ui::error(&mut err, "the system names no home folder for this user");
        return ExitCode::from(2);
    };
    let home = cli.home.clone().unwrap_or_else(|| defaults.home.clone());
    // A song list that does not read is reported by the command itself,
    // which names the problem; the default folder and programs stand
    // meanwhile.
    let listed = settings::of_home(&home).ok().flatten().unwrap_or_default();
    let library = if cli.library.is_some() {
        None
    } else {
        listed.library.folder(&home)
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
    let system = System::new(listed.tools.named(&job.dirs.home));
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
        Err(e) if e.downcast_ref::<tools::ToolError>().is_some() => {
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
        Err(e) if e.downcast_ref::<cli::Invalid>().is_some() => {
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

/// Write `shell`'s completion script to stdout, before anything else
/// `run` does: a package build writes the scripts where there may be no
/// home, song list or ffmpeg, and none is needed.
fn completions(shell: clap_complete::Shell) -> ExitCode {
    match cli::completions(shell, &mut std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::from(4),
        Err(e) => {
            let _ = ui::error(
                &mut anstream::stderr(),
                &format!("writing the completion script: {e}"),
            );
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

pub(crate) fn mode(m: Matching) -> Mode {
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
// One match over every command; its arms read as the command list does.
#[expect(clippy::too_many_lines)]
pub fn run_with<R: Runner, W: Write, D: Write>(
    job: &Job,
    runner: &R,
    http: &(dyn HttpTransport + Sync),
    mut prompter: Option<&mut dyn Prompter>,
    out: &mut W,
    data: &mut D,
) -> Result<bool> {
    let dirs = &job.dirs;
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
        dirs.apart()?;
        crate::manifest::present(&dirs.home)?;
        say_renamed(&dirs.home, out)?;
    }
    if trust::gated(&job.command) {
        let held = trust::held_in(&dirs.home)?;
        if !trust::gate(
            &dirs.home,
            &held,
            job.trust,
            change::reborrow(&mut prompter),
            out,
        )? {
            return Ok(true);
        }
    }
    // yt-dlp is found when a run first fetches; a library made from
    // files alone never needs it.
    if needs_ffmpeg(&job.command) {
        runner.require("ffmpeg")?;
        runner.require("ffprobe")?;
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
            let found: Vec<tools::Status> = tools::TOOLS
                .iter()
                .filter_map(|t| runner.tool(t.name))
                .collect();
            overview::programs(&found, data)?;
            Ok(true)
        }
        Command::Completions { shell } => {
            cli::completions(*shell, data)?;
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
        Command::Check { upstream: true, .. } if job.offline => Err(change::Refused(
            "--upstream asks each site, and --offline reaches none".into(),
        )
        .into()),
        Command::Check { decode, upstream } => {
            check::check(runner, dirs, (*decode, *upstream), out, data)
        }
        Command::Export {
            output,
            folder,
            max_size,
            codec,
            template,
            lyrics,
            set,
        } => {
            let settings = export::overrides(
                codec.as_deref(),
                template.as_deref(),
                lyrics.as_deref(),
                set,
            );
            let options = export::Options {
                output: output.as_deref(),
                folder: *folder,
                max_size: *max_size,
                settings: &settings,
            };
            export::export(runner, dirs, &options, out)
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
            if !change::confirmed(&confirm, change::reborrow(&mut prompter), out)? {
                return Ok(true);
            }
            let back = planned.songs_before().unwrap_or(crate::manifest::NEW);
            let held = trust::held_of(back).context("reading the song list undo puts back")?;
            if !trust::gate(&dirs.home, &held, job.trust, prompter, out)? {
                return Ok(true);
            }
            // As a run holds it, so no run begins between the undo and its sync.
            let _runs = Lock::runs(&dirs.home)?;
            let undone = {
                let _lock = Lock::folder(&dirs.home)?;
                let again = history::plan(dirs)?;
                if !again.same_run(&planned) {
                    return Err(change::Refused(
                        "Another run changed the history meanwhile; run `undo` again".into(),
                    )
                    .into());
                }
                history::undo(dirs, again, out)?
            };
            let opts = Options {
                before: undone.changed(),
                ..Options::default()
            };
            let ok = reconcile_now(job, runner, opts, None, out)?;
            undone.finish()?;
            Ok(ok)
        }
        Command::Add {
            inputs,
            to,
            no_match,
            from,
            matching,
            tags,
        } => add::run(
            job,
            runner,
            http,
            prompter,
            out,
            add::Request {
                inputs,
                to,
                no_match: *no_match,
                from: from.as_deref(),
                matching: *matching,
                tags,
            },
        ),
        Command::Sync {
            rematch,
            force,
            retry,
            accept,
            update_defaults,
            matching,
        } => sync::run(
            job,
            runner,
            http,
            prompter,
            out,
            sync::Request {
                rematch: *rematch,
                force: *force,
                retry: *retry,
                accept: *accept,
                update_defaults: *update_defaults,
                matching: *matching,
            },
        ),
        Command::Remove {
            query,
            purge,
            confirm,
        } => {
            let mut run = Run::begin(&dirs.home)?;
            let done = (|| {
                if !change::remove(dirs, query, *purge, confirm, prompter, &mut run, out)? {
                    return Ok(true);
                }
                reconcile_now(job, runner, Options::default(), Some(&mut run), out)
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
                let mut ok = sync::fetch_missing(job, runner, http, &manifest, true, out)?;
                ok &= reconcile_now(job, runner, Options::default(), Some(&mut run), out)?;
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
                reconcile_now(job, runner, Options::default(), Some(&mut run), out)
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
                    reconcile_now(job, runner, Options::default(), Some(&mut run), out)
                } else {
                    Ok(true)
                }
            });
            recorded(run, &dirs.home, done)
        }
    }
}

/// The offline phase with the run's settling time, as every command
/// that writes ends.
pub(crate) fn reconcile_now<R: Runner, W: Write>(
    job: &Job,
    runner: &R,
    opts: Options,
    run: Option<&mut Run>,
    out: &mut W,
) -> Result<bool> {
    let opts = Options {
        settling: job.settling,
        ..opts
    };
    reconcile::reconcile(runner, &job.dirs, opts, run, out)
}

/// Whether `command` may run ffmpeg or ffprobe: an export only when it
/// encodes, `check` only when it decodes.
pub(crate) fn needs_ffmpeg(command: &Command) -> bool {
    match command {
        Command::Info
        | Command::List { .. }
        | Command::Duplicates { .. }
        | Command::Purge { .. }
        | Command::Completions { .. } => false,
        Command::Check { decode, .. } => *decode,
        Command::Export {
            max_size,
            codec,
            template,
            lyrics,
            set,
            ..
        } => {
            max_size.is_some()
                || codec.is_some()
                || template.is_some()
                || lyrics.is_some()
                || !set.is_empty()
        }
        _ => true,
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
        Command::Export { folder: true, .. } => &["Encoding", "Writing the folder"],
        Command::Export { .. } => &["Encoding", "Writing the zip"],
        Command::Duplicates { .. } => &["Comparing prints"],
        Command::List { .. }
        | Command::Info
        | Command::Purge { .. }
        | Command::Completions { .. } => &[],
    }
}

/// Finish `run` whatever became of its command, so one that stopped on
/// an error is recorded as far as it got; the command's own error wins.
pub(crate) fn recorded(run: Run, home: &Path, done: Result<bool>) -> Result<bool> {
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
    let state = State::load(&dirs.home)?;
    let views = if removed {
        manifest
            .removed
            .iter()
            .map(|r| query::removed_view(r, &state))
            .collect()
    } else {
        query::views(&manifest, &state, &dirs.library)?
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

pub(crate) fn create(dir: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests;
