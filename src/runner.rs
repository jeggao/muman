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
}

/// A tool muman runs that could not be found; `run` exits 2 on it.
#[derive(Debug)]
pub struct MissingTool {
    pub name: &'static str,
    pub variable: &'static str,
}

impl std::fmt::Display for MissingTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match std::env::var_os(self.variable).filter(|v| !v.is_empty()) {
            Some(named) => write!(
                f,
                "{} names {}, which is no program found: give its path, or unset {} to find \
                 {} on the PATH",
                self.variable,
                named.to_string_lossy(),
                self.variable,
                self.name
            ),
            None => write!(
                f,
                "{} not found: install it, or name it in {}",
                self.name, self.variable
            ),
        }
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
        // A path or name first, whatever it holds: `C:\Tools\ffmpeg.exe`
        // and `/Applications/My Tools/ffmpeg` are one word each.
        if let Ok(program) = which::which(&value) {
            return Some(vec![program.into_os_string()]);
        }
        // Then a command with arguments, as `python -m yt_dlp`.
        let text = value.to_string_lossy();
        let words: Vec<String> = if cfg!(windows) {
            windows_words(&text)
        } else {
            shell_words::split(&text).ok()?
        };
        let (first, rest) = words.split_first()?;
        let program = which::which(first).ok()?;
        return Some(
            std::iter::once(program.into_os_string())
                .chain(rest.iter().map(OsString::from))
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

/// `text` split into words the way Windows programs read a command line:
/// at spaces outside double quotes, the quotes dropped, and `\` taken
/// as it is, since it separates folders.
pub(crate) fn windows_words(text: &str) -> Vec<String> {
    let (mut words, mut word, mut quoted) = (Vec::new(), String::new(), false);
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
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
        Err(failed(cmd, out.status, &out.stderr))
    }

    fn pipe(&self, cmd: &[OsString], on_bytes: &mut dyn FnMut(&[u8])) -> Result<()> {
        let mut child = self
            .command(cmd)?
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning {}", name(cmd)))?;
        // Drained apart, so a program writing much to stderr never stalls.
        let stderr = child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = e.read_to_end(&mut bytes);
                bytes
            })
        });
        if let Some(mut out) = child.stdout.take() {
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
        let status = child.wait()?;
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
    fn a_variable_names_a_whole_command() {
        let exe = std::env::current_exe().unwrap();
        let line = format!("\"{}\" -m yt_dlp", exe.display());
        let found = find("yt-dlp", Some(line.into())).unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(found[1], "-m");
    }

    #[test]
    fn a_variable_naming_a_path_with_spaces_is_one_program() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("My Tools");
        std::fs::create_dir(&folder).unwrap();
        let exe = std::env::current_exe().unwrap();
        let copy = folder.join(exe.file_name().unwrap());
        std::fs::copy(&exe, &copy).unwrap();
        let found = find("ffmpeg", Some(copy.clone().into())).unwrap();
        assert_eq!(found, [copy.into_os_string()]);
    }

    #[test]
    fn windows_words_keep_quoted_spaces_and_backslashes() {
        assert_eq!(
            windows_words(r#""C:\Program Files\Python\python.exe" -m yt_dlp"#),
            [r"C:\Program Files\Python\python.exe", "-m", "yt_dlp"]
        );
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
