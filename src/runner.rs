//! The subprocess seam: every yt-dlp, ffmpeg and ffprobe call goes
//! through [`Runner`], so tests drive the pipeline without spawning one.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{OnceLock, mpsc};

use anyhow::{Context, Result, anyhow};

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

    /// The Chromaprint words of the audio `fingerprint::output` decoded
    /// to `pcm`. Tests answer with chosen prints instead of computing.
    fn fingerprint(&self, pcm: &Path) -> Result<Vec<u32>> {
        crate::fingerprint::compute(pcm).map(|p| p.0)
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
}

/// A tool muman runs that could not be found; `run` exits 2 on it.
#[derive(Debug)]
pub struct MissingTool {
    pub name: &'static str,
    pub variable: &'static str,
}

impl std::fmt::Display for MissingTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} not found: install it, or name it in {}",
            self.name, self.variable
        )
    }
}

impl std::error::Error for MissingTool {}

/// The tools commands name, each with the variable that overrides it.
const TOOLS: [(&str, &str); 3] = [
    ("ffmpeg", "MUMAN_FFMPEG"),
    ("ffprobe", "MUMAN_FFPROBE"),
    ("yt-dlp", "MUMAN_YT_DLP"),
];

/// The production runner. Commands name a tool by its bare name; each
/// is found the first time it runs, so a run that never fetches never
/// needs yt-dlp. A tool is looked for, in order:
///
/// 1. In its variable (`MUMAN_FFMPEG`, `MUMAN_FFPROBE`, `MUMAN_YT_DLP`):
///    a path or a name, and for yt-dlp a whole command such as
///    `python -m yt_dlp`.
/// 1. Beside the muman executable, as a bundle ships them.
/// 1. On `PATH`, with Windows' `PATHEXT` extensions such as `.cmd`.
#[derive(Debug, Default)]
pub struct System {
    found: [OnceLock<Option<Vec<OsString>>>; 3],
}

impl System {
    /// Fail now, with [`MissingTool`], when `name` cannot be found, so a
    /// run that needs it stops before its first song rather than failing
    /// every one.
    pub fn require(&self, name: &str) -> Result<()> {
        let index = TOOLS
            .iter()
            .position(|(n, _)| *n == name)
            .ok_or_else(|| anyhow!("{name} is not a tool muman runs"))?;
        self.tool(index).map(drop)
    }

    /// The command line `tool` starts with, found once.
    fn tool(&self, index: usize) -> Result<&[OsString]> {
        let (name, variable) = TOOLS[index];
        self.found[index]
            .get_or_init(|| find(name, std::env::var_os(variable)))
            .as_deref()
            .ok_or_else(|| MissingTool { name, variable }.into())
    }

    /// `cmd` with its tool resolved, ready to spawn.
    fn command(&self, cmd: &[OsString]) -> Result<Command> {
        let program: Vec<OsString> = match TOOLS.iter().position(|(n, _)| cmd[0] == *n) {
            Some(i) => self.tool(i)?.to_vec(),
            None => vec![cmd[0].clone()],
        };
        let mut command = Command::new(&program[0]);
        command.args(&program[1..]);
        if cmd[0] == "yt-dlp" {
            // yt-dlp finds ffmpeg on PATH only; tell it which one muman uses.
            if let Ok([ffmpeg, ..]) = self.tool(0) {
                command.arg("--ffmpeg-location").arg(ffmpeg);
            }
            // Python writes a pipe in the legacy code page on Windows,
            // which mangles any path outside it; UTF-8 everywhere.
            command
                .env("PYTHONUTF8", "1")
                .env("PYTHONIOENCODING", "utf-8");
        }
        command.args(&cmd[1..]).stdin(Stdio::null());
        Ok(command)
    }
}

/// How `name` is started: from `variable` when set, else beside the
/// running executable, else from `PATH`.
fn find(name: &str, variable: Option<OsString>) -> Option<Vec<OsString>> {
    if let Some(value) = variable.filter(|v| !v.is_empty()) {
        let words = shell_words::split(&value.to_string_lossy())
            .ok()
            .filter(|w| !w.is_empty())
            .map_or_else(
                || vec![value.clone()],
                |w| w.into_iter().map(OsString::from).collect(),
            );
        let program = which::which(&words[0]).ok()?;
        return Some(
            std::iter::once(program.into_os_string())
                .chain(words.into_iter().skip(1))
                .collect(),
        );
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .and_then(|dir| which::which_in(name, Some(dir), ".").ok());
    beside
        .or_else(|| which::which(name).ok())
        .map(|p| vec![p.into_os_string()])
}

impl Runner for System {
    fn run(&self, cmd: &[OsString]) -> Result<()> {
        self.output(cmd).map(drop)
    }

    fn output(&self, cmd: &[OsString]) -> Result<Vec<u8>> {
        let out = self
            .command(cmd)?
            .output()
            .with_context(|| format!("spawning {}", name(cmd)))?;
        if out.status.success() {
            return Ok(out.stdout);
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lines: Vec<&str> = stderr.lines().collect();
        let tail = lines[lines.len().saturating_sub(10)..].join("\n");
        Err(anyhow!(
            "{} failed (exit {}): {tail}",
            name(cmd),
            out.status.code().unwrap_or(-1)
        ))
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
        let mut child = self
            .command(cmd)?
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning {}", name(cmd)))?;
        let (tx, rx) = mpsc::channel();
        let readers = [
            child.stdout.take().map(|r| reader(r, true, tx.clone())),
            child.stderr.take().map(|r| reader(r, false, tx)),
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
        Ok(child.wait()?.success())
    }
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
    fn a_variable_names_a_whole_command() {
        let exe = std::env::current_exe().unwrap();
        let line = format!("\"{}\" -m yt_dlp", exe.display());
        let found = find("yt-dlp", Some(line.into())).unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(found[1], "-m");
    }

    #[test]
    fn a_tool_found_nowhere_is_none() {
        assert!(find("muman-no-such-tool", Some("muman-no-such-tool".into())).is_none());
    }

    #[test]
    fn a_missing_tool_says_which_variable_names_it() {
        let missing = MissingTool {
            name: "ffprobe",
            variable: "MUMAN_FFPROBE",
        };
        assert!(missing.to_string().contains("MUMAN_FFPROBE"));
    }
}
