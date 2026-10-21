//! The subprocess seam: every yt-dlp, aria2c, ffmpeg and ffprobe call
//! goes through [`Runner`], so tests drive the pipeline without spawning
//! one; [`System`] is the one that spawns, with each tool as
//! [`crate::tools`] finds it from the song list's `[tools.*]`.
//!
//! Every program [`System`] starts is held in `LIVE` until it has been
//! waited for, so a signal that stops muman stops them too ([`stop`]):
//! a Ctrl-C reaches a terminal's whole process group, but a `kill` or a
//! service manager's stop reaches muman alone, and aria2c, left behind,
//! would go on fetching into the home. Once [`stop`] begins nothing more
//! starts, so none is spawned between it and the exit. While a program
//! of the user's own runs in the terminal, as an editor, a signal is
//! that program's to answer ([`Interactive`]): Ctrl-C in vim is no stop.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

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
        let _ = child.kill();
    }
    true
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

    /// [`Self::stream`] with `env` added to the program's environment.
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

    /// Fail now, with [`ToolError`], when `name` cannot be found or is
    /// below its floor, so a run that needs it stops before its first
    /// song rather than failing every one.
    pub fn require(&self, name: &str) -> Result<()> {
        let index = tools::TOOLS
            .iter()
            .position(|t| t.name == name)
            .ok_or_else(|| anyhow!("{name} is not a tool muman runs"))?;
        self.tool(index).map(drop).map_err(Into::into)
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
        self.stream_env(cmd, &[], on_line)
    }

    fn stream_env(
        &self,
        cmd: &[OsString],
        env: &[(String, String)],
        on_line: &mut dyn FnMut(Line<'_>),
    ) -> Result<bool> {
        let mut command = self.command(cmd)?;
        command
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = Spawned::new(&mut command, cmd)?;
        let (tx, rx) = mpsc::channel();
        let readers = [
            child.0.take_stdout().map(|r| reader(r, true, tx.clone())),
            child.0.take_stderr().map(|r| reader(r, false, tx)),
        ];
        for (out, line) in rx {
            on_line(if out {
                Line::Out(&line)
            } else {
                Line::Err(&line)
            });
        }
        for r in readers.into_iter().flatten() {
            let _ = r.join();
        }
        Ok(child.0.wait()?.success())
    }

    fn tool(&self, name: &str) -> Option<Status> {
        let index = tools::TOOLS.iter().position(|t| t.name == name)?;
        Some(Status {
            tool: &tools::TOOLS[index],
            found: self.tool(index).cloned(),
        })
    }
}

/// The error of `cmd` exiting with `status`, naming the tail of `stderr`.
fn failed(cmd: &[OsString], status: std::process::ExitStatus, stderr: &[u8]) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    let tail = without_addresses(&lines[lines.len().saturating_sub(10)..].join("\n"));
    anyhow!(
        "{} failed (exit {}): {tail}",
        name(cmd),
        status.code().unwrap_or(-1)
    )
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
    fn say(cmd: &[OsString]) {
        let line = cmd
            .iter()
            .map(|a| quoted(&a.to_string_lossy()))
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

/// Send each line of `pipe` until it closes; a carriage return ends a
/// line too, as progress written in place does.
fn reader<R: Read + Send + 'static>(
    pipe: R,
    out: bool,
    tx: mpsc::Sender<(bool, String)>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut pipe = BufReader::new(pipe);
        let mut buf = Vec::new();
        while pipe.read_until(b'\n', &mut buf).is_ok_and(|n| n > 0) {
            for part in buf
                .split(|b| *b == b'\r' || *b == b'\n')
                .filter(|p| !p.is_empty())
            {
                if tx
                    .send((out, String::from_utf8_lossy(part).into_owned()))
                    .is_err()
                {
                    return;
                }
            }
            buf.clear();
        }
    })
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

    #[test]
    fn the_running_executable_prints_no_version_a_tool_would() {
        let exe = std::env::current_exe().unwrap();
        let tool = tools::tool("ffmpeg").unwrap();
        let printed = version_of(tool, &[exe.into_os_string()]);
        assert!(printed.is_err() || tools::Version::parse(&printed.unwrap()).is_none());
    }
}
