//! ffmpeg command lines that write several files from one run: each
//! start costs about 100 ms of library loading against a few
//! milliseconds of work, so the outputs of a step share one. One output
//! of a run may go to its stdout, [`PIPE`], to be read as it is written
//! rather than from a file: [`run_piped`].

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::runner::Runner;

#[must_use]
pub fn base() -> Vec<OsString> {
    [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// Write the chosen attachments of each input out. `-dump_attachment` is
/// an input option and needs an output to run at all; a zero-length copy
/// of the first input's audio to `null` is the cheapest one.
#[must_use]
pub fn dump_command(inputs: &[(&Path, Vec<(usize, PathBuf)>)]) -> Vec<OsString> {
    let mut cmd = base();
    for (input, dumps) in inputs {
        for (ordinal, to) in dumps {
            cmd.push(format!("-dump_attachment:t:{ordinal}").into());
            cmd.push(to.as_os_str().to_os_string());
        }
        cmd.push("-i".into());
        cmd.push(input.as_os_str().to_os_string());
    }
    cmd.extend(
        ["-map", "0:a:0", "-c", "copy", "-t", "0", "-f", "null", "-"]
            .into_iter()
            .map(OsString::from),
    );
    cmd
}

/// One file an ffmpeg run writes: its options, ending in the format, and
/// its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub args: Vec<OsString>,
    pub path: PathBuf,
}

impl Output {
    #[must_use]
    pub fn new(args: Vec<OsString>, path: &Path) -> Self {
        Self {
            args,
            path: path.to_path_buf(),
        }
    }

    #[must_use]
    pub fn of(args: &[&str], path: &Path) -> Self {
        Self::new(args.iter().map(OsString::from).collect(), path)
    }
}

/// One file an ffmpeg run reads, and the options it is read with, as
/// where it is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub args: Vec<OsString>,
    pub path: PathBuf,
}

impl Input {
    #[must_use]
    pub fn new(args: Vec<OsString>, path: &Path) -> Self {
        Self {
            args,
            path: path.to_path_buf(),
        }
    }
}

impl From<&Path> for Input {
    fn from(path: &Path) -> Self {
        Self::new(Vec::new(), path)
    }
}

#[must_use]
pub fn outputs_command(inputs: &[&Path], outputs: &[Output]) -> Vec<OsString> {
    let inputs: Vec<Input> = inputs.iter().map(|p| Input::from(*p)).collect();
    command(&inputs, outputs)
}

#[must_use]
pub fn command(inputs: &[Input], outputs: &[Output]) -> Vec<OsString> {
    let mut cmd = base();
    for input in inputs {
        cmd.extend(input.args.iter().cloned());
        cmd.push("-i".into());
        cmd.push(input.path.as_os_str().to_os_string());
    }
    for output in outputs {
        cmd.extend(output.args.iter().cloned());
        cmd.push(output.path.as_os_str().to_os_string());
    }
    cmd
}

/// Every output in one run; when that fails, each in its own, so one
/// bad output costs only itself and its error is its own. One result per
/// output, in order.
pub fn run_outputs<R: Runner>(runner: &R, inputs: &[&Path], outputs: &[Output]) -> Vec<Result<()>> {
    let inputs: Vec<Input> = inputs.iter().map(|p| Input::from(*p)).collect();
    run(runner, &inputs, outputs)
}

/// [`run_outputs`] of inputs read with options of their own.
pub fn run<R: Runner>(runner: &R, inputs: &[Input], outputs: &[Output]) -> Vec<Result<()>> {
    if outputs.is_empty() {
        return Vec::new();
    }
    match runner.run(&command(inputs, outputs)) {
        Ok(()) => outputs.iter().map(|_| Ok(())).collect(),
        Err(e) if outputs.len() == 1 => vec![Err(e)],
        Err(_) => outputs
            .iter()
            .map(|o| runner.run(&command(inputs, std::slice::from_ref(o))))
            .collect(),
    }
}

/// The path of an output written to ffmpeg's stdout.
pub const PIPE: &str = "pipe:1";

/// What a piped output's reader is handed.
#[derive(Debug)]
pub enum Piped<'a> {
    /// The next bytes the output wrote.
    Bytes(&'a [u8]),
    /// The output begins again, its run tried alone.
    Again,
}

/// [`run`], with `piped`, written to [`PIPE`], handed to `sink` as it
/// arrives: every output in one run, and when that fails, each in its
/// own. One result per output, then the piped one's.
pub fn run_piped<R: Runner>(
    runner: &R,
    inputs: &[Input],
    outputs: &[Output],
    piped: &Output,
    sink: &mut dyn FnMut(Piped<'_>),
) -> (Vec<Result<()>>, Result<()>) {
    let mut all = outputs.to_vec();
    all.push(piped.clone());
    let Err(e) = runner.pipe(&command(inputs, &all), &mut |b| sink(Piped::Bytes(b))) else {
        return (outputs.iter().map(|_| Ok(())).collect(), Ok(()));
    };
    sink(Piped::Again);
    let alone = runner.pipe(&command(inputs, std::slice::from_ref(piped)), &mut |b| {
        sink(Piped::Bytes(b));
    });
    if outputs.is_empty() {
        return (Vec::new(), alone.map_err(|_| e));
    }
    let results = outputs
        .iter()
        .map(|o| runner.run(&command(inputs, std::slice::from_ref(o))))
        .collect();
    (results, alone)
}

/// A binary graymap, as `-c:v pgm` writes it: its width, height and
/// 8-bit pixels; `None` for anything else.
#[must_use]
pub fn read_pgm(bytes: &[u8]) -> Option<(u32, u32, &[u8])> {
    let mut fields = Vec::new();
    let mut at = 0;
    while fields.len() < 4 {
        while bytes.get(at)?.is_ascii_whitespace() {
            at += 1;
        }
        if bytes[at] == b'#' {
            while *bytes.get(at)? != b'\n' {
                at += 1;
            }
            continue;
        }
        let start = at;
        while !bytes.get(at)?.is_ascii_whitespace() {
            at += 1;
        }
        fields.push(std::str::from_utf8(&bytes[start..at]).ok()?);
    }
    let (w, h): (u32, u32) = (fields[1].parse().ok()?, fields[2].parse().ok()?);
    if fields[0] != "P5" || fields[3] != "255" {
        return None;
    }
    let pixels = bytes.get(at + 1..)?;
    (pixels.len() >= w as usize * h as usize).then_some((w, h, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_graymap_header_gives_its_size() {
        let mut pgm = b"P5\n# made by ffmpeg\n3 2\n255\n".to_vec();
        pgm.extend([1, 2, 3, 4, 5, 6]);
        assert_eq!(read_pgm(&pgm), Some((3, 2, &[1, 2, 3, 4, 5, 6][..])));
        assert_eq!(read_pgm(b"P6\n1 1\n255\nxyz"), None);
        assert_eq!(read_pgm(b"P5\n4 4\n255\n12"), None, "too few pixels");
    }

    #[test]
    fn outputs_follow_every_input() {
        let cmd = outputs_command(
            &[Path::new("/a"), Path::new("/b")],
            &[Output::of(&["-f", "null"], Path::new("-"))],
        );
        let text: Vec<String> = cmd
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let tail = &text[text.len() - 7..];
        assert_eq!(tail, ["-i", "/a", "-i", "/b", "-f", "null", "-"]);
    }
}
