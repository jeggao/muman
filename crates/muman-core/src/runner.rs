//! The subprocess seam: every yt-dlp, aria2c, ffmpeg and ffprobe call
//! goes through [`Runner`], so tests drive the pipeline without spawning
//! one; [`System`] is the one that spawns, with each tool as
//! [`crate::tools`] finds it from the song list's `[tools.*]`.
//!
//! Every program [`System`] starts is held in `LIVE` until it has been
//! waited for, so a signal that stops muman stops them too ([`stop`]):
//! a `kill` or a service manager's stop reaches muman alone, and
//! aria2c, left behind, would go on fetching into the home. Once
//! [`stop`] begins nothing more starts, so none is spawned between it
//! and the exit. While a program of the user's own runs in the terminal,
//! as an editor, a signal is that program's to answer ([`Interactive`]):
//! Ctrl-C in vim is no stop.
//!
//! What those programs started is stopped with them where the system
//! allows. On Unix each starts in a process group of its own, and
//! [`stop`] kills the whole group: a hook's background job, the ffmpeg
//! a yt-dlp started, the program a wrapper script runs. A Ctrl-C at the
//! terminal so reaches muman alone, which stops each group. On Windows
//! the program muman started is ended alone: a job object would hold
//! what it starts, and no safe interface offers one, so what it started
//! runs on, though a Ctrl-C at the console reaches every program there.
//!
//! A program streamed is waited for, not what it leaves running: a
//! background job it started, as a hook's `mpc update &` or a daemon,
//! holds its pipes open, so once it exits they are read for
//! [`AFTER_EXIT`] more at most, for what it wrote last, and let go.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use shared_child::SharedChild;

use anyhow::{Context, Result, anyhow};

use crate::tools::{self, Found, Named, Status, Tool, ToolError};

/// Every program started and not yet waited for, and whether muman is
/// stopping, which refuses any more.
static LIVE: Mutex<(bool, Vec<Arc<SharedChild>>)> = Mutex::new((false, Vec::new()));

/// How many programs of the user's own run in the terminal now.
static INTERACTIVE: AtomicUsize = AtomicUsize::new(0);

/// The exit code of a run a signal stopped: 128 and SIGINT's number, as
/// shells report a program Ctrl-C stopped.
pub const STOPPED_EXIT: u8 = 130;

/// Stop every program muman started and start no more: what a signal
/// does before muman exits. `false` while a program of the user's own
/// runs in the terminal, whose signal it is.
pub fn stop() -> bool {
    if INTERACTIVE.load(Ordering::SeqCst) > 0 {
        return false;
    }
    let mut live = LIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    live.0 = true;
    for child in &live.1 {
        end(child);
    }
    true
}

/// Kill `child` and, on Unix, every program in its process group.
fn end(child: &SharedChild) {
    #[cfg(unix)]
    if let Ok(group) = i32::try_from(child.id()) {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(group),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    let _ = child.kill();
}

/// Held while a program of the user's own runs in the terminal, so a
/// signal is left to it.
#[derive(Debug)]
pub struct Interactive(());

impl Interactive {
    #[must_use]
    pub fn begin() -> Self {
        INTERACTIVE.fetch_add(1, Ordering::SeqCst);
        Self(())
    }
}

impl Drop for Interactive {
    fn drop(&mut self) {
        INTERACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A program started and held in [`LIVE`] until dropped.
struct Spawned(Arc<SharedChild>);

impl Spawned {
    fn new(command: &mut Command, cmd: &[OsString]) -> Result<Self> {
        let mut live = LIVE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        anyhow::ensure!(!live.0, "muman is stopping; {} was not started", name(cmd));
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(command, 0);
        let child = Arc::new(
            SharedChild::spawn(command).with_context(|| format!("spawning {}", name(cmd)))?,
        );
        live.1.push(Arc::clone(&child));
        Ok(Self(child))
    }

    /// Wait for it, its stdout and stderr read to their ends apart, so a
    /// program writing much to one never stalls on the other.
    fn output(self) -> Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
        let drain = |pipe: Option<Box<dyn Read + Send>>| {
            pipe.map(|mut p| {
                std::thread::spawn(move || {
                    let mut bytes = Vec::new();
                    let _ = p.read_to_end(&mut bytes);
                    bytes
                })
            })
        };
        let out = drain(
            self.0
                .take_stdout()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let err = drain(
            self.0
                .take_stderr()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let joined = |t: Option<std::thread::JoinHandle<Vec<u8>>>| {
            t.and_then(|t| t.join().ok()).unwrap_or_default()
        };
        let (stdout, stderr) = (joined(out), joined(err));
        Ok((self.0.wait()?, stdout, stderr))
    }
}

impl Drop for Spawned {
    fn drop(&mut self) {
        let mut live = LIVE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        live.1.retain(|c| !Arc::ptr_eq(c, &self.0));
    }
}

/// One line a streamed program wrote, without its line ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line<'a> {
    Out(&'a str),
    Err(&'a str),
}

/// `Sync`, as library builds and lookups run on several threads.
pub trait Runner: Sync {
    /// Run to completion with stdout discarded; an error carries the
    /// tail of stderr.
    fn run(&self, cmd: &[OsString]) -> Result<()>;

    /// Run to completion and return stdout.
    fn output(&self, cmd: &[OsString]) -> Result<Vec<u8>>;

    /// Run with each line it writes, on either stream, handed to
    /// `on_line` as it arrives. Returns whether it exited successfully.
    fn stream(&self, cmd: &[OsString], on_line: &mut dyn FnMut(Line<'_>)) -> Result<bool>;

    /// Run to completion with stdout handed to `on_bytes` as it arrives,
    /// in pieces of any size; an error carries the tail of stderr. One
    /// that cannot stream hands it over whole at the end.
    fn pipe(&self, cmd: &[OsString], on_bytes: &mut dyn FnMut(&[u8])) -> Result<()> {
        self.output(cmd).map(|out| on_bytes(&out))
    }

    /// The Chromaprint words of the audio `fingerprint::output` decoded
    /// to `pcm`. Tests answer with chosen prints instead of computing.
    fn fingerprint(&self, pcm: &Path) -> Result<Vec<u32>> {
        crate::fingerprint::compute(pcm)
    }

    /// [`Self::stream`] with `env` added to the program's environment, and
    /// its stdout and stderr one pipe, so their lines keep the order they
    /// were written in, each a [`Line::Out`].
    fn stream_env(
        &self,
        cmd: &[OsString],
        env: &[(String, String)],
        on_line: &mut dyn FnMut(Line<'_>),
    ) -> Result<bool> {
        let _ = env;
        self.stream(cmd, on_line)
    }

    /// The tool called `name` as this runner finds it, for `info`; a
    /// runner that runs no real program names none.
    fn tool(&self, name: &str) -> Option<Status> {
        let _ = name;
        None
    }

    /// Fail now, with [`ToolError`], when the tool called `name` cannot
    /// be found or is below its floor, so a run that needs it stops
    /// before its first song rather than failing every one; a runner that
    /// runs no real program has every one.
    fn require(&self, name: &str) -> Result<()> {
        let _ = name;
        Ok(())
    }
}

/// The production runner. Commands name a tool by its bare name; each
/// is found and its version read the first time it runs
/// ([`crate::tools`]), so a run that never fetches never needs yt-dlp.
#[derive(Debug)]
pub struct System {
    /// Each of [`tools::TOOLS`] as the song list names it.
    named: [Named; tools::TOOLS.len()],
    found: [OnceLock<Result<Found, ToolError>>; tools::TOOLS.len()],
}

impl Default for System {
    fn default() -> Self {
        Self::new(tools::TOOLS.each_ref().map(Named::default_for))
    }
}

impl System {
    /// Each of [`tools::TOOLS`] run as `named` names it, in that order.
    #[must_use]
    pub fn new(named: [Named; tools::TOOLS.len()]) -> Self {
        Self {
            named,
            found: Default::default(),
        }
    }

    /// The tool at `index` of [`tools::TOOLS`], found and checked once.
    fn tool(&self, index: usize) -> Result<&Found, ToolError> {
        let tool = &tools::TOOLS[index];
        self.found[index]
            .get_or_init(|| locate(tool, std::env::var_os(tool.variable), &self.named[index]))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// `cmd` with its tool resolved, ready to spawn.
    // Every program muman runs is started here.
    #[expect(clippy::disallowed_methods)]
    fn command(&self, cmd: &[OsString]) -> Result<Command> {
        let program: Vec<OsString> = match tools::TOOLS.iter().position(|t| cmd[0] == t.name) {
            Some(i) => self.tool(i)?.command.clone(),
            None => vec![cmd[0].clone()],
        };
        let mut command = Command::new(&program[0]);
        command.args(&program[1..]);
        if cmd[0] == "yt-dlp" {
            // yt-dlp finds ffmpeg on PATH only; tell it which one muman uses.
            if let Ok(Found {
                command: ffmpeg, ..
            }) = self.tool(0)
            {
                command.arg("--ffmpeg-location").arg(&ffmpeg[0]);
            }
            python_utf8(&mut command);
        }
        command.args(&cmd[1..]).stdin(Stdio::null());
        Ok(command)
    }
}

/// Python writes a pipe in the legacy code page on Windows, which
/// mangles any path outside it; UTF-8 everywhere.
fn python_utf8(command: &mut Command) {
    command
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8");
}

/// `tool` as `variable`, this machine's, names it, else as `named` does,
/// with its version read and, where `named` asks, checked.
fn locate(
    tool: &'static Tool,
    variable: Option<OsString>,
    named: &Named,
) -> Result<Found, ToolError> {
    let variable = variable.filter(|v| !v.is_empty());
    let missing = ToolError::Missing {
        tool,
        named: named.other_than(tool).filter(|_| variable.is_none()),
    };
    let command = tools::find(variable, named).ok_or(missing)?;
    match version_of(tool, &command) {
        Ok(printed) => tools::accept(tool, command, &printed, named.version_check),
        Err(_) if !named.version_check => Ok(Found {
            command,
            version: None,
        }),
        Err(e) => Err(e),
    }
}

/// What `tool`, started by `command`, prints for its version flag.
// Runs a tool for its version.
#[expect(clippy::disallowed_methods)]
fn version_of(tool: &'static Tool, command: &[OsString]) -> Result<String, ToolError> {
    let program = command[0].to_string_lossy().into_owned();
    let unusable = |error: String| ToolError::Unusable {
        tool,
        program: program.clone(),
        error,
    };
    let mut version = Command::new(&command[0]);
    version
        .args(&command[1..])
        .arg(tool.version_flag)
        .stdin(Stdio::null());
    if tool.name == "yt-dlp" {
        python_utf8(&mut version);
    }
    version.stdout(Stdio::piped()).stderr(Stdio::piped());
    let (status, stdout, stderr) = Spawned::new(&mut version, command)
        .and_then(Spawned::output)
        .map_err(|e| unusable(format!("{e:#}")))?;
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        let said = stderr.lines().last().unwrap_or_default().trim();
        return Err(unusable(format!(
            "exit {}{}{said}",
            status.code().unwrap_or(-1),
            if said.is_empty() { "" } else { ", " }
        )));
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

impl Runner for System {
    fn run(&self, cmd: &[OsString]) -> Result<()> {
        self.output(cmd).map(drop)
    }

    fn output(&self, cmd: &[OsString]) -> Result<Vec<u8>> {
        let mut command = self.command(cmd)?;
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let (status, stdout, stderr) = Spawned::new(&mut command, cmd)?.output()?;
        if status.success() {
            return Ok(stdout);
        }
        Err(failed(cmd, status, &stderr))
    }

    fn pipe(&self, cmd: &[OsString], on_bytes: &mut dyn FnMut(&[u8])) -> Result<()> {
        let mut command = self.command(cmd)?;
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = Spawned::new(&mut command, cmd)?;
        // Drained apart, so a program writing much to stderr never stalls.
        let stderr = child.0.take_stderr().map(|mut e| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = e.read_to_end(&mut bytes);
                bytes
            })
        });
        if let Some(mut out) = child.0.take_stdout() {
            let mut buf = vec![0; 64 * 1024];
            loop {
                match out.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => on_bytes(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e).with_context(|| format!("reading {}", name(cmd))),
                }
            }
        }
        let stderr = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
        let status = child.0.wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(failed(cmd, status, &stderr))
        }
    }

    fn stream(&self, cmd: &[OsString], on_line: &mut dyn FnMut(Line<'_>)) -> Result<bool> {
        let mut command = self.command(cmd)?;
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = Spawned::new(&mut command, cmd)?;
        let (tx, rx) = mpsc::sync_channel(AHEAD);
        if let Some(r) = child.0.take_stdout() {
            reader(r, true, tx.clone());
        }
        if let Some(r) = child.0.take_stderr() {
            reader(r, false, tx.clone());
        }
        relayed(&child, tx, &rx, on_line)
    }

    fn stream_env(
        &self,
        cmd: &[OsString],
        env: &[(String, String)],
        on_line: &mut dyn FnMut(Line<'_>),
    ) -> Result<bool> {
        let mut command = self.command(cmd)?;
        let (pipe, write) = std::io::pipe().context("making a pipe")?;
        command
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdout(write.try_clone().context("making a pipe")?)
            .stderr(write);
        let child = Spawned::new(&mut command, cmd)?;
        // It holds muman's copies of the pipe's end, which would keep the
        // pipe open once the program's close.
        drop(command);
        let (tx, rx) = mpsc::sync_channel(AHEAD);
        reader(pipe, true, tx.clone());
        relayed(&child, tx, &rx, on_line)
    }

    fn tool(&self, name: &str) -> Option<Status> {
        let index = tools::TOOLS.iter().position(|t| t.name == name)?;
        Some(Status {
            tool: &tools::TOOLS[index],
            found: self.tool(index).cloned(),
        })
    }

    fn require(&self, name: &str) -> Result<()> {
        let index = tools::TOOLS
            .iter()
            .position(|t| t.name == name)
            .ok_or_else(|| anyhow!("{name} is not a tool muman runs"))?;
        self.tool(index).map(drop).map_err(Into::into)
    }
}

/// A program muman ran that ended in failure: its name, its exit code,
/// none when a signal stopped it, and the last line it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exited {
    pub program: String,
    pub code: Option<i32>,
    pub said: String,
}

impl Exited {
    /// Whether it ended for no fault of what it read: stopped by a
    /// signal, or exiting with a code past 128, as a shell says a program
    /// a signal stopped did. The same file may read the next time.
    #[must_use]
    pub fn by_chance(&self) -> bool {
        self.code.is_none_or(|c| c > 128)
    }
}

impl std::fmt::Display for Exited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.code {
            Some(code) => write!(f, "{} failed (exit {code}): {}", self.program, self.said),
            None => write!(
                f,
                "{} failed (stopped by a signal): {}",
                self.program, self.said
            ),
        }
    }
}

impl std::error::Error for Exited {}

/// Whether `e` says nothing of the file a program read: a program a
/// signal stopped, or one that could not be started.
#[must_use]
pub fn by_chance(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<Exited>().is_some_and(Exited::by_chance)
            || c.downcast_ref::<std::io::Error>().is_some()
    })
}

/// The error of `cmd` exiting with `status`, in one line: the last line
/// of `stderr` that says anything, where ffmpeg and ffprobe say what
/// stopped them. The lines before it go to a trace, not to every sync
/// that meets the same file.
fn failed(cmd: &[OsString], status: std::process::ExitStatus, stderr: &[u8]) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(stderr);
    let said = stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(without_addresses)
        .unwrap_or_default();
    anyhow::Error::new(Exited {
        program: name(cmd),
        code: status.code(),
        said,
    })
}

/// `text` without the memory addresses ffmpeg names its parts by, as
/// `[mp3 @ 0x55d0c1a2]`, which say nothing to a reader: `[mp3]`.
fn without_addresses(text: &str) -> String {
    static ADDRESS: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r" @ 0x[0-9a-fA-F]+\]").expect("valid"));
    ADDRESS.replace_all(text, "]").into_owned()
}

/// A runner that says each command, as a shell would read it, before
/// running it.
#[derive(Debug)]
pub struct Traced<R>(pub R);

impl<R: Runner> Traced<R> {
    // Shows a traced command on stderr.
    #[expect(clippy::disallowed_methods)]
    fn say(cmd: &[OsString]) {
        let line = redacted(cmd)
            .iter()
            .map(|a| quoted(a))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = crate::ui::trace(&mut std::io::stderr().lock(), &line);
    }
}

impl<R: Runner> Runner for Traced<R> {
    fn run(&self, cmd: &[OsString]) -> Result<()> {
        Self::say(cmd);
        self.0.run(cmd)
    }

    fn output(&self, cmd: &[OsString]) -> Result<Vec<u8>> {
        Self::say(cmd);
        self.0.output(cmd)
    }

    fn stream(&self, cmd: &[OsString], on_line: &mut dyn FnMut(Line<'_>)) -> Result<bool> {
        Self::say(cmd);
        self.0.stream(cmd, on_line)
    }

    fn pipe(&self, cmd: &[OsString], on_bytes: &mut dyn FnMut(&[u8])) -> Result<()> {
        Self::say(cmd);
        self.0.pipe(cmd, on_bytes)
    }

    fn fingerprint(&self, pcm: &Path) -> Result<Vec<u32>> {
        self.0.fingerprint(pcm)
    }

    fn stream_env(
        &self,
        cmd: &[OsString],
        env: &[(String, String)],
        on_line: &mut dyn FnMut(Line<'_>),
    ) -> Result<bool> {
        Self::say(cmd);
        self.0.stream_env(cmd, env, on_line)
    }

    fn tool(&self, name: &str) -> Option<Status> {
        self.0.tool(name)
    }

    fn require(&self, name: &str) -> Result<()> {
        self.0.require(name)
    }
}

/// The path of an output ffmpeg writes to its stdout.
pub const PIPE: &str = "pipe:1";

/// What follows an option whose value is, or may hold, a secret, so a
/// trace and an export's redaction hide alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Takes {
    /// A secret whole: a password, a user name, a token.
    Secret,
    /// `Field:value`, the field kept.
    Header,
    /// `extractor:key=value;...`, as yt-dlp's `--extractor-args`, of which
    /// the arguments [`extractor_secret`] names.
    ExtractorArgs,
    /// An address, of which all after its host may name a login or a
    /// tracker's passkey, in its path or its query.
    Address,
}

/// yt-dlp's options whose value is, or may hold, a secret: its account,
/// two-factor and TV provider logins, a video's password, a client
/// certificate's password, headers sent, extractor arguments, and
/// proxies.
pub const YTDLP_SECRET: &[(&str, Takes)] = &[
    ("--username", Takes::Secret),
    ("-u", Takes::Secret),
    ("--password", Takes::Secret),
    ("-p", Takes::Secret),
    ("--twofactor", Takes::Secret),
    ("-2", Takes::Secret),
    ("--video-password", Takes::Secret),
    ("--ap-username", Takes::Secret),
    ("--ap-password", Takes::Secret),
    ("--client-certificate-password", Takes::Secret),
    ("--add-header", Takes::Header),
    ("--add-headers", Takes::Header),
    ("--extractor-args", Takes::ExtractorArgs),
    ("--proxy", Takes::Address),
    ("--geo-verification-proxy", Takes::Address),
];

/// aria2c's options whose value is, or may hold, a secret: HTTP, FTP and
/// proxy logins, proxies, the RPC interface's secret and login, headers
/// sent, and trackers. Given in `[torrent] args`, each goes to the run's
/// config file rather than its command line (`aria2.rs`).
pub const ARIA2C_SECRET: &[(&str, Takes)] = &[
    ("--header", Takes::Header),
    ("--http-user", Takes::Secret),
    ("--http-passwd", Takes::Secret),
    ("--ftp-user", Takes::Secret),
    ("--ftp-passwd", Takes::Secret),
    ("--all-proxy", Takes::Address),
    ("--all-proxy-user", Takes::Secret),
    ("--all-proxy-passwd", Takes::Secret),
    ("--http-proxy", Takes::Address),
    ("--http-proxy-user", Takes::Secret),
    ("--http-proxy-passwd", Takes::Secret),
    ("--https-proxy", Takes::Address),
    ("--https-proxy-user", Takes::Secret),
    ("--https-proxy-passwd", Takes::Secret),
    ("--ftp-proxy", Takes::Address),
    ("--ftp-proxy-user", Takes::Secret),
    ("--ftp-proxy-passwd", Takes::Secret),
    ("--rpc-user", Takes::Secret),
    ("--rpc-passwd", Takes::Secret),
    ("--rpc-secret", Takes::Secret),
    ("--bt-tracker", Takes::Address),
];

/// Whether aria2c's long option `name`, without its dashes, is one of
/// [`ARIA2C_SECRET`].
#[must_use]
pub fn aria2c_secret(name: &str) -> bool {
    ARIA2C_SECRET
        .iter()
        .any(|(o, _)| o.strip_prefix("--") == Some(name))
}

/// Whether an extractor argument named `key` holds a secret: a token, as
/// YouTube's `po_token`, a password or a cookie, or what names a
/// signed-in account's session, YouTube's `visitor_data` and
/// `data_sync_id`.
#[must_use]
pub fn extractor_secret(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    ["token", "password", "secret", "cookie", "api_key"]
        .iter()
        .any(|s| key.contains(s))
        || ["visitor_data", "data_sync_id"].contains(&key.as_str())
}

/// What stands for a value hidden.
const HIDDEN: &str = "<hidden>";

/// `cmd` as a trace says it: the value of each option that may carry a
/// secret hidden, given as `--name value` or `--name=value`, a header's
/// name kept; and in yt-dlp's `--extractor-args`, a PO token and visitor
/// data, which stand for a session.
fn redacted(cmd: &[OsString]) -> Vec<String> {
    let ytdlp = cmd.first().is_some_and(|p| p == "yt-dlp");
    let secret = |name: &str| {
        YTDLP_SECRET
            .iter()
            .chain(ARIA2C_SECRET)
            .any(|(o, _)| o.strip_prefix("--") == Some(name))
    };
    let short = |arg: &str| {
        YTDLP_SECRET
            .iter()
            .any(|(o, _)| o.len() == 2 && arg.get(..2) == Some(*o))
    };
    let mut said = Vec::new();
    let mut next: Option<String> = None;
    let mut options = true;
    for arg in cmd.iter().map(|a| a.to_string_lossy()) {
        if let Some(name) = next.take() {
            said.push(hidden(&name, &arg));
            continue;
        }
        let long = arg.strip_prefix("--").filter(|_| options);
        let (name, value) = long.map_or((None, None), |o| match o.split_once('=') {
            Some((name, value)) => (Some(name), Some(value)),
            None => (Some(o), None),
        });
        match (name, value) {
            _ if arg == "--" => {
                options = false;
                said.push(arg.into_owned());
            }
            (Some(name), Some(value)) if secret(name) => {
                said.push(format!("--{name}={}", hidden(name, value)));
            }
            (Some(name), None) if secret(name) => {
                next = Some(name.to_string());
                said.push(arg.into_owned());
            }
            (None, _) if ytdlp && options && short(&arg) => {
                if arg.len() == 2 {
                    next = Some(arg.to_string());
                    said.push(arg.into_owned());
                } else {
                    said.push(format!("{}{HIDDEN}", &arg[..2]));
                }
            }
            _ => said.push(arg.into_owned()),
        }
    }
    said
}

/// The value of the option `name` as a trace says it.
fn hidden(name: &str, value: &str) -> String {
    match name {
        "extractor-args" => match value.split_once(':') {
            Some((extractor, args)) => {
                let args: Vec<String> = args
                    .split(';')
                    .map(|part| match part.split_once('=') {
                        Some((key, _)) if extractor_secret(key) => format!("{key}={HIDDEN}"),
                        _ => part.to_string(),
                    })
                    .collect();
                format!("{extractor}:{}", args.join(";"))
            }
            None => value.to_string(),
        },
        "header" | "add-header" | "add-headers" => match value.split_once(':') {
            Some((header, _)) => format!("{header}: {HIDDEN}"),
            None => HIDDEN.to_string(),
        },
        _ => HIDDEN.to_string(),
    }
}

/// `arg` as one shell word.
fn quoted(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_alphanumeric() || "-_./:=,+@%".contains(c));
    if plain {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// What a streamed program said: a line, on stdout or not, or that it
/// exited.
enum Said {
    Line(bool, String),
    Exited,
}

/// How long a streamed program's pipes are read after it exits, for
/// what a program it left running holds open.
const AFTER_EXIT: Duration = Duration::from_secs(1);

/// How many lines a streamed program's pipes are read ahead of the one
/// handed on, so a program saying much faster than it is relayed waits
/// rather than fill memory.
const AHEAD: usize = 1024;

/// Send each line of `pipe` until it closes; a carriage return ends a
/// line too, as progress written in place does. Not joined: a program
/// left running by the one streamed may hold the pipe for ever.
fn reader<R: Read + Send + 'static>(pipe: R, out: bool, tx: mpsc::SyncSender<Said>) {
    std::thread::spawn(move || {
        let mut pipe = BufReader::new(pipe);
        let mut buf = Vec::new();
        while pipe.read_until(b'\n', &mut buf).is_ok_and(|n| n > 0) {
            for part in buf
                .split(|b| *b == b'\r' || *b == b'\n')
                .filter(|p| !p.is_empty())
            {
                let line = String::from_utf8_lossy(part).into_owned();
                if tx.send(Said::Line(out, line)).is_err() {
                    return;
                }
            }
            buf.clear();
        }
    });
}

/// Hand each line of `child`'s that `rx` brings to `on_line`, as it
/// comes, and whether it exited successfully; `tx` says when it exits.
fn relayed(
    child: &Spawned,
    tx: mpsc::SyncSender<Said>,
    rx: &mpsc::Receiver<Said>,
    on_line: &mut dyn FnMut(Line<'_>),
) -> Result<bool> {
    let waited = Arc::clone(&child.0);
    std::thread::spawn(move || {
        let _ = waited.wait();
        let _ = tx.send(Said::Exited);
    });
    relay(rx, on_line);
    Ok(child.0.wait()?.success())
}

/// Hand each line `rx` brings to `on_line` until every pipe closes, or
/// [`AFTER_EXIT`] once the program has exited.
fn relay(rx: &mpsc::Receiver<Said>, on_line: &mut dyn FnMut(Line<'_>)) {
    let mut exited: Option<Instant> = None;
    loop {
        let said = match exited {
            None => rx.recv().ok(),
            Some(at) => match AFTER_EXIT.checked_sub(at.elapsed()) {
                Some(left) => rx.recv_timeout(left).ok(),
                None => None,
            },
        };
        match said {
            None => return,
            Some(Said::Exited) => exited = Some(Instant::now()),
            Some(Said::Line(true, line)) => on_line(Line::Out(&line)),
            Some(Said::Line(false, line)) => on_line(Line::Err(&line)),
        }
    }
}

fn name(cmd: &[OsString]) -> String {
    cmd.first()
        .map_or_else(|| "?".to_string(), |s| s.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_names_no_memory_address() {
        assert_eq!(
            without_addresses("[mp3 @ 0x558127e226c0] Invalid frame size (176)"),
            "[mp3] Invalid frame size (176)"
        );
    }

    #[test]
    fn a_trace_hides_every_login_password_header_and_token() {
        let cmd = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();
        let ytdlp = cmd(&[
            "yt-dlp",
            "--username",
            "marlo",
            "--password=hunter-0001",
            "-pHUNTER-0002",
            "-u",
            "marlo",
            "--add-headers",
            "Authorization:Bearer 0003",
            "--extractor-args",
            "youtube:po_token=web.gvs+0004;player_client=default",
            "--format",
            "ba/b",
            "--",
            "-p",
        ]);
        assert_eq!(
            redacted(&ytdlp).join(" "),
            "yt-dlp --username <hidden> --password=<hidden> -p<hidden> -u <hidden> \
             --add-headers Authorization: <hidden> --extractor-args \
             youtube:po_token=<hidden>;player_client=default --format ba/b -- -p"
        );
        let aria2c = cmd(&[
            "aria2c",
            "--header=Cookie: uid=0005",
            "--http-passwd",
            "0006",
            "--rpc-secret=0007",
            "-p",
            "--bt-tracker=https://tracker.example/0008/announce",
        ]);
        let said = redacted(&aria2c).join(" ");
        assert!(!said.contains("000"), "{said}");
        assert!(said.contains("--header=Cookie: <hidden>"), "{said}");
        assert!(said.contains(" -p "), "an option of aria2c's own: {said}");
        let ytdlp = cmd(&[
            "yt-dlp",
            "--proxy",
            "canary01:pw@proxy.example:3128",
            "--geo-verification-proxy=https://canary02@geo.example",
            "--extractor-args",
            "youtube:data_sync_id=canary03;player_client=web",
        ]);
        let aria2c = cmd(&[
            "aria2c",
            "--all-proxy=marlo:canary04@proxy.example:3128",
            "--https-proxy",
            "http://canary05@proxy.example",
        ]);
        for said in [redacted(&ytdlp), redacted(&aria2c)] {
            let said = said.join(" ");
            assert!(!said.contains("canary"), "{said}");
        }
    }

    #[test]
    fn a_tool_that_is_no_program_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ffmpeg");
        std::fs::write(&file, "not a program").unwrap();
        let tool = tools::tool("ffmpeg").unwrap();
        let err = version_of(tool, &[file.clone().into_os_string()]).unwrap_err();
        assert!(matches!(err, ToolError::Unusable { .. }), "{err}");
        assert!(err.to_string().contains("MUMAN_FFMPEG"));
    }

    #[test]
    fn a_program_whose_version_is_not_checked_runs_though_it_prints_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir
            .path()
            .join(format!("ffmpeg-fork{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&file, "not a program").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let tool = tools::tool("ffmpeg").unwrap();
        let mut named = Named {
            command: vec![file.clone().into_os_string()],
            version_check: false,
        };
        let found = locate(tool, None, &named).unwrap();
        assert_eq!(
            (found.command, found.version),
            (vec![file.into_os_string()], None)
        );
        named.version_check = true;
        let err = locate(tool, None, &named).unwrap_err();
        assert!(matches!(err, ToolError::Unusable { .. }), "{err}");
    }

    #[test]
    fn a_program_the_song_list_names_and_no_machine_has_is_named_in_the_error() {
        let tool = tools::tool("yt-dlp").unwrap();
        let named = Named {
            command: vec!["muman-no-such-fork".into()],
            version_check: true,
        };
        let err = locate(tool, None, &named).unwrap_err().to_string();
        assert!(
            err.contains("[tools.yt-dlp] command names muman-no-such-fork"),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_program_stopped_takes_what_it_started_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let late = dir.path().join("late");
        let script = format!("(sleep 1; touch '{}') & wait", late.display());
        let cmd = ["sh", "-c", &script].map(OsString::from);
        let mut command = System::default().command(&cmd).unwrap();
        let started = Spawned::new(&mut command, &cmd).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        end(&started.0);
        assert!(!started.0.wait().unwrap().success());
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(!late.exists(), "the background job outlived its program");
    }

    #[cfg(unix)]
    #[test]
    fn a_program_streamed_is_not_held_by_the_job_it_left_running() {
        let cmd = ["sh", "-c", "sleep 30 & echo done"].map(OsString::from);
        let start = Instant::now();
        let mut said = Vec::new();
        let ran = System::default().stream(&cmd, &mut |line| {
            let (Line::Out(l) | Line::Err(l)) = line;
            said.push(l.to_string());
        });
        assert!(ran.unwrap());
        assert_eq!(said, ["done"]);
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_program_s_stdout_and_stderr_lines_keep_their_order() {
        let script = "for n in 1 2 3 4 5 6 7 8; do echo out $n; echo err $n >&2; done";
        let cmd = ["sh", "-c", script].map(OsString::from);
        let mut said = Vec::new();
        let ran = System::default().stream_env(&cmd, &[], &mut |line| {
            let (Line::Out(l) | Line::Err(l)) = line;
            said.push(l.to_string());
        });
        assert!(ran.unwrap());
        let written: Vec<String> = (1..=8)
            .flat_map(|n| [format!("out {n}"), format!("err {n}")])
            .collect();
        assert_eq!(said, written);
    }

    #[test]
    fn the_running_executable_prints_no_version_a_tool_would() {
        let exe = std::env::current_exe().unwrap();
        let tool = tools::tool("ffmpeg").unwrap();
        let printed = version_of(tool, &[exe.into_os_string()]);
        assert!(printed.is_err() || tools::Version::parse(&printed.unwrap()).is_none());
    }
}
