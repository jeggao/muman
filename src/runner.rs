//! The subprocess seam: every yt-dlp, ffmpeg and ffprobe call goes
//! through [`Runner`], so tests drive the pipeline without spawning one.

use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;

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
}

/// The production runner.
#[derive(Debug, Default)]
pub struct System {
    /// The ffmpeg that runs a command writing a `chromaprint` output,
    /// in place of the one on PATH.
    pub fingerprint: Option<PathBuf>,
}

impl System {
    fn program<'a>(&'a self, cmd: &'a [OsString]) -> &'a OsStr {
        match &self.fingerprint {
            Some(ffmpeg) if cmd[0] == "ffmpeg" && cmd.iter().any(|a| a == "chromaprint") => {
                ffmpeg.as_os_str()
            }
            _ => &cmd[0],
        }
    }
}

impl Runner for System {
    fn run(&self, cmd: &[OsString]) -> Result<()> {
        self.output(cmd).map(drop)
    }

    fn output(&self, cmd: &[OsString]) -> Result<Vec<u8>> {
        let out = Command::new(self.program(cmd))
            .args(&cmd[1..])
            .stdin(Stdio::null())
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
        let mut child = Command::new(self.program(cmd))
            .args(&cmd[1..])
            .stdin(Stdio::null())
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
