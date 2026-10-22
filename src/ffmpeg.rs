//! ffmpeg command lines that write several files from one run: each
//! start costs about 100 ms of library loading against a few
//! milliseconds of work, so the outputs of a step share one. One output
//! of a run may go to its stdout, [`PIPE`], to be read as it is written
//! rather than from a file: [`run_piped`].
//!
//! Every file ffmpeg or ffprobe reads is named to it by [`Input::args`],
//! the one place an input is given. ffmpeg picks a demuxer by what a
//! file holds, never by its name, and some demuxers read past the file:
//! a DASH or HLS manifest fetches every address it lists, a concat
//! script opens every file it names. A song file given to `add`, a
//! torrent's or a manual source is anyone's, so a `.flac` holding a
//! manifest would reach the network, offline or not. So an input may be
//! read only by the demuxers of what muman reads (`-format_whitelist`):
//! the audio containers and streams the store takes as songs, Matroska
//! for every fetched source, the subtitles `add` converts, the chapters
//! muman writes for an MP4, and the pictures a song is covered with. Any
//! other is refused once the probe names it, before it reads further.
//! `-protocol_whitelist file` keeps the input itself a file, but alone it
//! stopped no manifest's fetch in ffmpeg 6.1, whose DASH demuxer opens
//! its addresses as it chooses: the demuxers are the guard.
//!
//! A path is made absolute first. ffmpeg reads a name as an address when
//! what precedes its first `:` could be a scheme, so a file given as
//! `http:host?x.flac`, `concat:a|b` or `subfile,,…:`, relative to the
//! current folder, would be fetched or pieced together; `--` ends
//! options, not addresses. An absolute path starts with `/`, a drive or
//! `\\`, which no scheme does.
//!
//! A JPEG longer than ffmpeg's first probe is read by the `image2`
//! demuxer, chosen by its name's extension, so `image2` is kept; but it
//! reads a name holding `%d` as a numbered sequence of other files, and
//! one holding `%` and a glob character as a glob of them, and a picture
//! so named is `image2`'s whatever it holds. A path that could read so
//! (`reads_as_pattern`) is read without `image2`, so such a picture is
//! refused rather than taken for other files; a song so named is read
//! as ever, since `image2` takes no name of an audio extension.
//! `-pattern_type none` would read the picture as itself, but every
//! other demuxer refuses an option it does not know, and which one reads
//! a file is known only once it is probed.

use std::ffi::{OsStr, OsString};
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
        let args = dumps
            .iter()
            .flat_map(|(ordinal, to)| {
                [
                    format!("-dump_attachment:t:{ordinal}").into(),
                    to.as_os_str().to_os_string(),
                ]
            })
            .collect();
        cmd.extend(Input::new(args, input).args());
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

    /// This input as ffmpeg and ffprobe take one: its options, the
    /// protocol and demuxers it may be read with, then `-i` and its path,
    /// made absolute.
    #[must_use]
    pub fn args(&self) -> Vec<OsString> {
        let path = if self.path.is_absolute() {
            self.path.clone()
        } else {
            std::path::absolute(&self.path).unwrap_or_else(|_| Path::new(".").join(&self.path))
        };
        let mut demuxers = [AUDIO, TEXT, PICTURES].concat();
        if !reads_as_pattern(path.as_os_str()) {
            demuxers.push(IMAGE2);
        }
        let mut args = self.args.clone();
        args.extend(
            ["-protocol_whitelist", "file", "-format_whitelist"]
                .into_iter()
                .map(OsString::from),
        );
        args.push(demuxers.join(",").into());
        args.push("-i".into());
        args.push(path.into_os_string());
        args
    }
}

/// The demuxers of the audio files the store takes as songs, by the
/// name `-format_whitelist` takes: one name of a demuxer that has several
/// matches it, as `mov` matches `mov,mp4,m4a,3gp,3g2,mj2`. Every name
/// here and below is one ffmpeg 5.1 has.
const AUDIO: &[&str] = &[
    "aac", "ac3", "aiff", "ape", "asf", "au", "caf", "dsf", "dts", "dtshd", "eac3", "flac", "iff",
    "matroska", "mlp", "mov", "mp3", "ogg", "shn", "tak", "truehd", "tta", "w64", "wav", "wv",
];

/// The subtitles `add` converts to lyrics, and the chapters muman writes
/// for an MP4.
const TEXT: &[&str] = &["ffmetadata", "srt", "webvtt"];

/// The pictures a song is covered with, each read as itself.
const PICTURES: &[&str] = &[
    "bmp_pipe",
    "gif",
    "gif_pipe",
    "jpeg_pipe",
    "png_pipe",
    "webp_pipe",
];

/// The demuxer that reads a picture chosen by its name's extension, and
/// a name shaped as a pattern as the files it matches.
const IMAGE2: &str = "image2";

/// Whether `image2` may read `path` as a pattern of other files' names: a
/// `%` followed by digits and `d`, a sequence, or by a glob character.
/// It finds more than ffmpeg would, as `%%d`, which ffmpeg reads as the
/// text `%d`, so it errs only towards leaving `image2` out.
fn reads_as_pattern(path: &OsStr) -> bool {
    let bytes = path.as_encoded_bytes();
    bytes.iter().enumerate().any(|(at, &b)| {
        let rest = &bytes[at + 1..];
        let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
        b == b'%'
            && (rest.get(digits) == Some(&b'd')
                || rest.first().is_some_and(|c| b"*?[]{}".contains(c)))
    })
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
        cmd.extend(input.args());
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
        let (a, b) = (rooted("a"), rooted("b"));
        let cmd = text(&outputs_command(
            &[&a, &b],
            &[Output::of(&["-f", "null"], Path::new("-"))],
        ));
        let inputs: Vec<&str> = cmd
            .windows(2)
            .filter(|w| w[0] == "-i")
            .map(|w| w[1].as_str())
            .collect();
        assert_eq!(inputs, [a.to_str().unwrap(), b.to_str().unwrap()]);
        assert_eq!(cmd[cmd.len() - 3..], ["-f", "null", "-"]);
    }

    fn rooted(name: &str) -> PathBuf {
        let root = if cfg!(windows) { r"C:\" } else { "/" };
        Path::new(root).join("music").join(name)
    }

    fn text(cmd: &[OsString]) -> Vec<String> {
        cmd.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn demuxers_of(args: &[String]) -> Vec<&str> {
        let at = args.iter().position(|a| a == "-format_whitelist").unwrap();
        args[at + 1].split(',').collect()
    }

    #[test]
    fn an_input_is_read_only_by_the_demuxers_muman_reads() {
        let song = rooted("Lantern Weather.flac");
        let args = text(&Input::new(vec!["-ss".into(), "1".into()], &song).args());
        let demuxers = [AUDIO, TEXT, PICTURES, &[IMAGE2]].concat().join(",");
        assert_eq!(
            args,
            [
                "-ss",
                "1",
                "-protocol_whitelist",
                "file",
                "-format_whitelist",
                &demuxers,
                "-i",
                song.to_str().unwrap(),
            ]
        );
        let dump = text(&dump_command(&[(&song, vec![(2, rooted("info.json"))])]));
        let at = dump.iter().position(|a| a == "-i").unwrap();
        assert_eq!(dump[at - 6], "-dump_attachment:t:2");
        assert_eq!(demuxers_of(&dump), demuxers.split(',').collect::<Vec<_>>());
    }

    #[test]
    fn a_relative_path_is_never_read_as_an_address() {
        for name in [
            "http:127.0.0.1:8912?x.flac",
            "concat:a.flac|b.flac",
            "subfile,,0,9,:x.flac",
        ] {
            let args = text(&Input::from(Path::new(name)).args());
            let path = args.last().unwrap();
            assert!(path.ends_with(name), "{name}: {path}");
            assert!(
                Path::new(path).is_absolute() || path.starts_with('.'),
                "{name}: {path}"
            );
        }
    }

    #[test]
    fn a_name_image2_reads_as_a_pattern_is_read_without_it() {
        for name in [
            "cover%d.jpg",
            "cover%03d.png",
            "cover%*.jpg",
            "cover%{1,2}.jpg",
            "cover%%d.jpg",
        ] {
            let args = text(&Input::from(rooted(name).as_path()).args());
            assert!(!demuxers_of(&args).contains(&IMAGE2), "{name}");
            assert!(demuxers_of(&args).contains(&"jpeg_pipe"), "{name}");
        }
        for name in ["cover.jpg", "100% Hits.jpg", "half 50%.png", "odd%"] {
            let args = text(&Input::from(rooted(name).as_path()).args());
            assert!(demuxers_of(&args).contains(&IMAGE2), "{name}");
        }
    }

    /// Each extension muman reads, and the demuxer ffprobe 6.1 named for
    /// a file of it ffmpeg wrote; for one ffmpeg writes none, as `.ape`,
    /// the demuxer its source reads it with.
    const READ_AS: &[(&str, &str)] = &[
        ("aac", "aac"),
        ("ac3", "ac3"),
        ("aif", "aiff"),
        ("aifc", "aiff"),
        ("aiff", "aiff"),
        ("alac", "caf"),
        ("ape", "ape"),
        ("au", "au"),
        ("caf", "caf"),
        ("dff", "iff"),
        ("dsf", "dsf"),
        ("dts", "dts"),
        ("eac3", "eac3"),
        ("flac", "flac"),
        ("m4a", "mov"),
        ("m4b", "mov"),
        ("mka", "matroska"),
        ("mkv", "matroska"),
        ("mlp", "mlp"),
        ("mov", "mov"),
        ("mp2", "mp3"),
        ("mp3", "mp3"),
        ("mp4", "mov"),
        ("oga", "ogg"),
        ("ogg", "ogg"),
        ("opus", "ogg"),
        ("rf64", "wav"),
        ("shn", "shn"),
        ("spx", "ogg"),
        ("tak", "tak"),
        ("thd", "truehd"),
        ("tta", "tta"),
        ("w64", "w64"),
        ("wav", "wav"),
        ("webm", "matroska"),
        ("wma", "asf"),
        ("wv", "wv"),
        ("srt", "srt"),
        ("vtt", "webvtt"),
        ("ffmeta", "ffmetadata"),
        ("jpg", "jpeg_pipe"),
        ("jpeg", "jpeg_pipe"),
        ("png", "png_pipe"),
        ("webp", "webp_pipe"),
    ];

    #[test]
    fn every_file_muman_reads_has_its_demuxer() {
        let taken = crate::store::MEDIA
            .iter()
            .chain(&crate::store::IMAGES)
            .chain(&["srt", "vtt"]);
        for ext in taken {
            assert!(READ_AS.iter().any(|(e, _)| e == ext), "{ext}");
        }
        let all = [AUDIO, TEXT, PICTURES].concat();
        for (ext, demuxer) in READ_AS {
            assert!(all.contains(demuxer), "{ext}: {demuxer}");
        }
    }
}
