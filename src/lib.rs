//! Keep a music library in step with a song list: every song gathered
//! from its sources, the best of each aspect picked by measurement, and
//! the result written as one tagged, covered track with its lyrics.

// Tests make and change the files they check; shipped code changes one
// only where an `#[expect]` says why.
#![cfg_attr(test, expect(clippy::disallowed_methods))]

pub use muman_engine::acquire;
pub use muman_net::acoustid;
pub mod add;
pub use muman_core::atomic;
pub use muman_engine::album;
pub use muman_engine::artifact;
pub use muman_media::align;
pub use muman_media::analysis;
pub use muman_net::aria2;
pub mod attach;
pub mod change;
pub use muman_media::chapters;
pub mod check;
pub use muman_media::clean;
pub mod cli;
pub use muman_core::defaults;
pub use muman_core::dirs;
pub use muman_engine::due;
pub use muman_media::codec;
pub use muman_media::cue;
pub use muman_media::cues;
pub use muman_net::coverart;
pub use muman_net::download;
pub mod duplicates;
pub mod editor;
pub use muman_core::errors;
pub mod export;
pub use muman_core::fingerprint;
pub use muman_core::given;
pub use muman_engine::fit;
pub use muman_engine::folder;
pub use muman_media::facts;
pub use muman_media::ffmpeg;
pub mod guided;
pub use muman_config::held;
pub use muman_config::hooks;
pub use muman_config::manifest;
pub use muman_config::migrate;
pub use muman_core::hash;
pub use muman_core::http;
#[cfg(any(test, feature = "testing"))]
pub use muman_core::kill;
pub use muman_core::lang;
pub use muman_engine::history;
pub use muman_engine::identify;
pub use muman_engine::levels;
pub use muman_engine::library;
pub use muman_engine::limit;
pub use muman_engine::lookup;
pub use muman_engine::outputs;
pub use muman_media::info;
pub use muman_media::integrity;
pub use muman_media::loudness;
pub use muman_media::lyrics;
pub use muman_media::naming;
pub use muman_media::ogg;
pub use muman_net::lrclib;
pub use muman_net::music;
pub use muman_net::musicbrainz;
pub mod overview;
pub use muman_config::providers;
pub use muman_core::parallel;
pub use muman_core::platform;
pub use muman_core::portable;
pub use muman_core::progress;
pub use muman_core::provider;
pub use muman_media::picture;
pub use muman_media::probe;
pub use muman_net::plugins;
pub mod purge;
pub use muman_engine::query;
pub use muman_engine::reconcile;
pub use muman_media::quality;
pub use muman_media::record;
pub mod redact;
pub use muman_config::settings;
pub use muman_core::relpath;
pub use muman_core::runner;
pub use muman_core::sites;
pub use muman_core::source;
pub use muman_core::store;
pub use muman_engine::render;
pub use muman_engine::resolve;
pub use muman_engine::rows;
pub use muman_engine::state;
pub use muman_media::similar;
pub mod sync;
pub use muman_config::terms;
pub use muman_core::template;
#[cfg(any(test, feature = "testing"))]
pub use muman_core::testing;
pub use muman_core::tools;
pub use muman_core::torrent;
pub use muman_media::tagfile;
pub use muman_media::tags;
pub mod trust;
pub use muman_core::ui;
pub use muman_core::units;
pub use muman_net::ytdlp_log;

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
use crate::query::QueryMatch;
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
    /// The run whose hook runs this one, if one does.
    pub hooked: Option<Hooked>,
}

/// The run a hook runs muman from: its home, and the library it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hooked {
    pub home: PathBuf,
    pub library: Option<PathBuf>,
}

/// The run whose hook runs this one, from the variables `var` reads.
pub fn hooked_from(var: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Option<Hooked> {
    Some(Hooked {
        home: var(hooks::HOOK_HOME)?.into(),
        library: var("MUMAN_LIBRARY").map(PathBuf::from),
    })
}

impl Job {
    #[must_use]
    pub fn from_cli(cli: Cli, defaults: Defaults) -> Self {
        Self {
            dirs: Dirs {
                home: cli.home.unwrap_or(defaults.home),
                library: cli.library.unwrap_or(defaults.library),
                locks: defaults.cache.as_ref().map(|c| c.join("locks")),
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
            hooked: None,
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
// Reaches the terminal: progress and messages on stderr, and the answers a prompt reads.
#[expect(clippy::disallowed_methods)]
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
    job.hooked = hooked_from(&|name| std::env::var_os(name));
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
// Writes the shell completions asked for to stdout.
#[expect(clippy::disallowed_methods)]
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
    let traits = traits(&job.command);
    if traits.locks
        && let Some(hooked) = &job.hooked
    {
        refuse_hooked(hooked, dirs)?;
    }
    progress::current().plan(traits.steps);
    if traits.apart {
        dirs.apart()?;
        if !matches!(job.command, Command::Undo { .. }) && history::undo_unfinished(&dirs.home) {
            return Err(change::Refused(
                "An undo stopped partway has not put the library back yet; run `muman undo` to \
                 finish it"
                    .into(),
            )
            .into());
        }
    }
    if traits.edits_list {
        crate::state::present(&dirs.home)?;
        say_renamed(&dirs.home, out)?;
    }
    if traits.gated {
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
    if traits.ffmpeg {
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
            reconcile::reconcile_into(runner, dirs, opts, None, out, Some(report)).map(|s| s.ok)
        }
        Command::Info { state: true } => {
            state::State::dump(&dirs.home, data)?;
            Ok(true)
        }
        Command::Info { state: false } => {
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
        Command::Duplicates {
            query,
            resolve,
            merge,
            yes,
            dry_run,
        } if *resolve || merge.is_some() => {
            let mut run = Run::begin(&dirs.home)?;
            let mut due = hooks::Due::default();
            let confirm = cli::Confirm {
                yes: *yes,
                all: false,
                dry_run: *dry_run,
            };
            let done = (|| {
                let changed = if *resolve {
                    duplicates::resolve(runner, dirs, query, &confirm, prompter, out)?
                } else {
                    duplicates::merge(runner, dirs, query, &confirm, prompter, out)?
                };
                if !changed {
                    return Ok(true);
                }
                reconcile_now(
                    job,
                    runner,
                    Options::default(),
                    Some(&mut run),
                    &mut due,
                    out,
                )
            })();
            finished(job, runner, run, done, &due, out)
        }
        Command::Duplicates { query, .. } => {
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
            force,
            max_size,
            codec,
            template,
            lyrics,
            redact,
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
                redact: *redact,
                force: *force,
            };
            export::export(runner, dirs, &options, out)
        }
        Command::Undo { yes, dry_run } => {
            let planned = history::plan(dirs)?;
            ui::info(out, "Undoing the last run:")?;
            for line in planned.lines() {
                ui::line(out, &format!("  {line}"))?;
            }
            let confirm = cli::Confirm {
                yes: *yes,
                all: false,
                dry_run: *dry_run,
            };
            let ask = change::Ask::YesNo("Undo the last run?".into());
            if !change::confirmed(&confirm, &ask, change::reborrow(&mut prompter), out)? {
                return Ok(true);
            }
            let back = planned.songs_before().unwrap_or(crate::manifest::NEW);
            let held = trust::held_of(back).context("reading the song list undo puts back")?;
            if !trust::gate(&dirs.home, &held, job.trust, prompter, out)? {
                return Ok(true);
            }
            // As a run holds it, so no run begins between the undo and its sync.
            let runs = Lock::runs(&dirs.home)?;
            let undone = {
                let _lock = Lock::folder(&dirs.home)?;
                let _library = Lock::library(&dirs.library, dirs.lock_folder()?);
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
            let mut due = hooks::Due::default();
            let ok = reconcile_now(job, runner, opts, None, &mut due, out)?;
            undone.finish()?;
            drop(runs);
            run_hooks(job, runner, &due, out)?;
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
            until_done,
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
                until_done: *until_done,
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
            let mut due = hooks::Due::default();
            let done = (|| {
                if !change::remove(dirs, query, *purge, confirm, prompter, &mut run, out)? {
                    return Ok(true);
                }
                reconcile_now(
                    job,
                    runner,
                    Options::default(),
                    Some(&mut run),
                    &mut due,
                    out,
                )
            })();
            finished(job, runner, run, done, &due, out)
        }
        Command::Restore { query, confirm } => {
            let mut run = Run::begin(&dirs.home)?;
            let mut due = hooks::Due::default();
            let done = (|| {
                if !change::restore(dirs, query, confirm, prompter, out)? {
                    return Ok(true);
                }
                let manifest = Manifest::load(&dirs.home)?;
                let mut ok = sync::fetch_missing(job, runner, http, &manifest, true, out)?;
                ok &= reconcile_now(
                    job,
                    runner,
                    Options::default(),
                    Some(&mut run),
                    &mut due,
                    out,
                )?;
                Ok(ok)
            })();
            finished(job, runner, run, done, &due, out)
        }
        Command::Set { terms, confirm } => {
            let mut run = Run::begin(&dirs.home)?;
            let mut due = hooks::Due::default();
            let done = (|| {
                if !change::set(dirs, terms, confirm, prompter, out)? {
                    return Ok(true);
                }
                reconcile_now(
                    job,
                    runner,
                    Options::default(),
                    Some(&mut run),
                    &mut due,
                    out,
                )
            })();
            finished(job, runner, run, done, &due, out)
        }
        Command::Edit { query, all } => {
            // Begun once the editor is closed, so an open editor holds no
            // other run back.
            let mut run: Option<Run> = None;
            let mut due = hooks::Due::default();
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
                    reconcile_now(
                        job,
                        runner,
                        Options::default(),
                        Some(&mut run),
                        &mut due,
                        out,
                    )
                } else {
                    Ok(true)
                }
            });
            finished(job, runner, run, done, &due, out)
        }
    }
}

/// The offline phase with the run's settling time, as every command
/// that writes ends; the hooks it makes due are added to `due`.
pub(crate) fn reconcile_now<R: Runner, W: Write>(
    job: &Job,
    runner: &R,
    opts: Options,
    run: Option<&mut Run>,
    due: &mut hooks::Due,
    out: &mut W,
) -> Result<bool> {
    let opts = Options {
        settling: job.settling,
        ..opts
    };
    let synced = reconcile::reconcile(runner, &job.dirs, opts, run, out)?;
    due.then(synced.due);
    Ok(synced.ok)
}

/// Run the hooks `due` names, once the command has let go of the home; a
/// muman a hook started runs none.
pub(crate) fn run_hooks<R: Runner, W: Write>(
    job: &Job,
    runner: &R,
    due: &hooks::Due,
    out: &mut W,
) -> Result<()> {
    if job.hooked.is_some() {
        return Ok(());
    }
    due.run(runner, &job.dirs.home, out)
}

/// What [`run_with`] must know of a command before running it.
// Each bool is one question run_with asks of every command.
#[expect(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Traits {
    /// It changes the library, the home's sources or the song list, so
    /// the library and the home must lie apart ([`Dirs::apart`]).
    pub apart: bool,
    /// It writes the song list: the list must be there while the library
    /// holds songs, and each setting it renames is said first.
    pub edits_list: bool,
    /// It may run a tool, a hook or a request on the song list's
    /// settings, so asks first ([`trust`]). `undo` asks apart, about the
    /// list it puts back.
    pub gated: bool,
    /// It may run ffmpeg or ffprobe: an export only when it encodes,
    /// `check` only when it decodes.
    pub ffmpeg: bool,
    /// It may lock the home or the library, so a hook cannot run it on
    /// the library of the run that started it under another home
    /// ([`Hooked`]).
    pub locks: bool,
    /// The long steps it may take, in the order it takes them, which
    /// number them as it goes.
    pub steps: &'static [&'static str],
}

/// Refuse to run on the library `hooked`, the run whose hook this is,
/// named in the `MUMAN_LIBRARY` the hook inherits, under another home:
/// two homes would write one library.
fn refuse_hooked(hooked: &Hooked, dirs: &Dirs) -> Result<()> {
    let elsewhere = !platform::same_path(&hooked.home, &dirs.home);
    if elsewhere
        && hooked
            .library
            .as_deref()
            .is_some_and(|l| platform::same_path(l, &dirs.library))
    {
        return Err(change::Refused(format!(
            "a hook cannot run this on {}, the library of the run that started it, under \
             another home; pass --library to name another",
            dirs.library.display()
        ))
        .into());
    }
    Ok(())
}

/// What `command` is, one arm per command, so a new command says each.
pub(crate) fn traits(command: &Command) -> Traits {
    const SYNC: &[&str] = &[
        "Fetching",
        "Measuring",
        "Looking up",
        "Comparing",
        "Writing",
    ];
    const CHANGE: &[&str] = &["Measuring", "Comparing", "Writing"];
    let edits = |steps| Traits {
        apart: true,
        edits_list: true,
        gated: true,
        ffmpeg: true,
        locks: true,
        steps,
    };
    let reads = Traits {
        apart: false,
        edits_list: false,
        gated: false,
        ffmpeg: false,
        locks: true,
        steps: &[],
    };
    match command {
        Command::Add { .. } | Command::Sync { .. } => edits(SYNC),
        Command::Restore { .. } => edits(&["Fetching", "Measuring", "Comparing", "Writing"]),
        Command::Remove { .. } | Command::Set { .. } | Command::Edit { .. } => edits(CHANGE),
        Command::Undo { .. } => Traits {
            apart: true,
            ffmpeg: true,
            steps: CHANGE,
            ..reads
        },
        Command::Purge { .. } => Traits {
            apart: true,
            ..reads
        },
        Command::Status { .. } => Traits {
            gated: true,
            ffmpeg: true,
            steps: &["Measuring", "Comparing"],
            ..reads
        },
        Command::Check { decode, upstream } => Traits {
            gated: *decode || *upstream,
            ffmpeg: *decode,
            steps: &["Decoding"],
            ..reads
        },
        Command::Export {
            folder,
            max_size,
            codec,
            template,
            lyrics,
            set,
            ..
        } => {
            let encodes = max_size.is_some()
                || codec.is_some()
                || template.is_some()
                || lyrics.is_some()
                || !set.is_empty();
            Traits {
                gated: encodes,
                ffmpeg: encodes,
                steps: if *folder {
                    &["Encoding", "Writing the folder"]
                } else {
                    &["Encoding", "Writing the zip"]
                },
                ..reads
            }
        }
        Command::Duplicates { resolve, merge, .. } if *resolve || merge.is_some() => {
            edits(&["Comparing prints", "Comparing", "Measuring", "Writing"])
        }
        Command::Duplicates { .. } => Traits {
            locks: false,
            steps: &["Comparing prints"],
            ..reads
        },
        Command::Info { .. } => Traits {
            gated: true,
            ..reads
        },
        Command::List { .. } | Command::Completions { .. } => Traits {
            locks: false,
            ..reads
        },
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

/// [`recorded`], then the hooks `due` names, run once `run` has let go of
/// the home, whether or not its command stopped on an error.
pub(crate) fn finished<R: Runner, W: Write>(
    job: &Job,
    runner: &R,
    run: Run,
    done: Result<bool>,
    due: &hooks::Due,
    out: &mut W,
) -> Result<bool> {
    let done = recorded(run, &job.dirs.home, done);
    run_hooks(job, runner, due, out)?;
    done
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
        match ui::line(data, &query::format(template, view)) {
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
