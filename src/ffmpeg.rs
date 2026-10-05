//! ffmpeg command lines that write several files from one run: each
//! start costs about 100 ms of library loading against a few
//! milliseconds of work, so the outputs of a step share one.

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

#[must_use]
pub fn outputs_command(inputs: &[&Path], outputs: &[Output]) -> Vec<OsString> {
    let mut cmd = base();
    for input in inputs {
        cmd.push("-i".into());
        cmd.push(input.as_os_str().to_os_string());
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
    if outputs.is_empty() {
        return Vec::new();
    }
    match runner.run(&outputs_command(inputs, outputs)) {
        Ok(()) => outputs.iter().map(|_| Ok(())).collect(),
        Err(e) if outputs.len() == 1 => vec![Err(e)],
        Err(_) => outputs
            .iter()
            .map(|o| runner.run(&outputs_command(inputs, std::slice::from_ref(o))))
            .collect(),
    }
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
