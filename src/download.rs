//! The yt-dlp call that fetches sources into the store, and the list of
//! files it reports finishing.
//!
//! yt-dlp runs with `--ignore-config`, so a personal config cannot change
//! what a source holds; the song list's `[ytdlp]` is how options reach
//! it. `[ytdlp] args` go to every yt-dlp run muman makes ([`ending`]),
//! the listing and the YouTube Music lookups as well as the fetch, since
//! what one needs to reach a site, as a login or a JavaScript runtime,
//! each needs; `extractor_args` go to a fetch and the check of what a
//! source offers upstream, the runs that choose among its formats.
//!
//! Neither goes on the command line, which any user of the machine sees
//! (`ps`, `/proc/<pid>/cmdline`), and which would show a password, a
//! cookie header or a PO token to them all. Each run writes them to
//! config files in its scratch folder, its owner's alone
//! ([`crate::atomic`]), and names them with `--config-locations`
//! ([`Passed`]), which yt-dlp reads under `--ignore-config` too: one of
//! `args`, and one of `extractor_args` then `args`, in the order the
//! command line had them. yt-dlp splits a config file as a POSIX shell
//! does (`shlex`), so each argument is single-quoted, a `'` in it closed,
//! escaped and reopened, which keeps any text exactly: spaces, `#`, `\`,
//! `$` and line breaks. The file begins with a UTF-8 byte-order mark,
//! which yt-dlp heeds before the locale's encoding, no UTF-8 on Windows
//! unless Python is told so. The path itself goes through yt-dlp's `~`
//! and `$VAR` expansion; a scratch folder's name holds neither.
//!
//! yt-dlp puts what a config file holds before the command line's own
//! options, wherever `--config-locations` stands, so where an argument of
//! the song list's and an option of muman's set the same thing, muman's
//! wins. muman's own are the output's form, paths and template, the
//! embedding, the retry sleeps, `--no-playlist`, and those `[ytdlp]` has
//! a key for, as `format`, which is where they are set.
//!
//! Unfinished downloads wait in `partial/` under the state root,
//! beside the store, named by the extractor and ID alone, so an
//! interrupted one resumes at the same path on the next run whatever
//! its uploader is called by then; a finished one is renamed into the
//! store's staging folder rather than copied, and from there to its
//! key's place ([`crate::store::place`]).
//!
//! The info JSON yt-dlp embeds holds the cookies and headers its
//! requests sent and the signed addresses of what it fetched
//! ([`crate::info`]), and no yt-dlp option leaves them out of the formats
//! it lists, which muman reads. So each finished file is [`scrub`]bed
//! before it is placed: its info JSON dumped, all but what muman reads
//! dropped, and the file remuxed once with the JSON put back in its
//! place. A remux keeps every stream's packets, pictures and chapters, so
//! nothing a source is measured or known by changes. A file whose JSON
//! holds nothing else, or none at all, is left as it is. A file that
//! cannot be scrubbed is not placed: its fetch fails, as one yt-dlp did
//! not finish does.
//!
//! yt-dlp remuxes a single-file download into `<id>.mkv` at that name,
//! with no temporary one, and deletes what it remuxed only once ffmpeg
//! is done; and a later run takes an `<id>.mkv` it finds for finished.
//! A run stopped mid-remux, as a Ctrl-C stops ffmpeg with it, leaves an
//! `<id>.mkv` cut short beside its whole original. So before a fetch
//! each such `.mkv` is deleted ([`drop_cut_remuxes`]), and yt-dlp
//! remuxes the original again.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::runner::Runner;
use crate::settings::Ytdlp;
use crate::sites::Sites;
use crate::source::SourceKey;
use crate::{ffmpeg, info, probe};

/// `<extractor>/<id>.mkv`: what names a download the same every time it
/// is tried, nothing a site may rename.
pub const OUTPUT_TEMPLATE: &str = "%(extractor_key)s/%(id)s.%(ext)s";

/// Where a run's files go: the store's staging folder, partial
/// downloads, and the list of finished files.
#[derive(Debug, Clone, Copy)]
pub struct Places<'a> {
    pub store: &'a Path,
    pub temp: &'a Path,
    pub done: &'a Path,
    /// The folder muman's postprocessors are loaded from, if any.
    pub plugins: Option<&'a Path>,
    /// The song list's `[ytdlp]`.
    pub options: &'a Ytdlp,
    /// Its `args` and `extractor_args`, as yt-dlp reads them.
    pub passed: &'a Passed,
    /// A file holding the URLs, one per line, in place of the command
    /// line, which Windows caps at 32,767 characters.
    pub batch: Option<&'a Path>,
}

/// The config file of `[ytdlp] args`.
const EVERY: &str = "ytdlp-args.conf";

/// The config file of `[ytdlp] extractor_args`, then `args`.
const CHOOSING: &str = "ytdlp-fetch.conf";

/// `[ytdlp] args` and `extractor_args` in the config files a run's
/// yt-dlp reads them from, off its command line; none for a song list
/// that sets neither.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Passed {
    /// `args`, for every run.
    every: Option<PathBuf>,
    /// `extractor_args`, then `args`, for the runs that choose among a
    /// source's formats.
    choosing: Option<PathBuf>,
}

impl Passed {
    /// Write the arguments of `options` into `dir`, a folder of the
    /// run's own that holds no such file yet.
    pub fn write(options: &Ytdlp, dir: &Path) -> Result<Self> {
        let extractor = options.extractor_args.iter().flat_map(|(extractor, args)| {
            ["--extractor-args".into(), format!("{extractor}:{args}")]
        });
        let choosing: Vec<String> = extractor.chain(options.args.iter().cloned()).collect();
        let file = |name: &str, args: &[String]| -> Result<Option<PathBuf>> {
            if args.is_empty() {
                return Ok(None);
            }
            let path = dir.join(name);
            crate::atomic::private_file(&path, &config(args))?;
            Ok(Some(path))
        };
        Ok(Self {
            every: file(EVERY, &options.args)?,
            choosing: file(CHOOSING, &choosing)?,
        })
    }
}

/// `--config-locations` naming `file`, when there is one.
fn located(file: Option<&Path>) -> Vec<OsString> {
    file.map(|f| vec!["--config-locations".into(), f.as_os_str().to_os_string()])
        .unwrap_or_default()
}

/// `args` as a yt-dlp config file, which yt-dlp splits back into them
/// exactly: each single-quoted on a line of its own.
fn config(args: &[String]) -> Vec<u8> {
    let mut text = String::from('\u{feff}');
    for arg in args {
        text.push('\'');
        text.push_str(&arg.replace('\'', r#"'"'"'"#));
        text.push_str("'\n");
    }
    text.into_bytes()
}

/// Delete each `<id>.mkv` in a download folder of `partial` that has a
/// song file of the same stem beside it: a remux stopped short.
pub fn drop_cut_remuxes(partial: &Path) {
    let folders = std::fs::read_dir(partial)
        .into_iter()
        .flatten()
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.is_dir() && !p.ends_with(crate::source::TORRENT));
    for folder in folders {
        let files: Vec<PathBuf> = std::fs::read_dir(&folder)
            .into_iter()
            .flatten()
            .filter_map(|e| Some(e.ok()?.path()))
            .collect();
        for mkv in files
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e == "mkv"))
        {
            let original = files.iter().any(|p| {
                p != mkv
                    && p.file_stem() == mkv.file_stem()
                    && crate::store::kind_of(p) == Some(crate::store::Kind::Media)
            });
            if original {
                let _ = std::fs::remove_file(mkv);
            }
        }
    }
}

/// yt-dlp's argv. Everything is embedded into one `.mkv`, so a source
/// stays a single file. Partial downloads stay in `temp` until finished,
/// a video `archive` names is skipped unseen, and each finished file is
/// appended to `done` as its extractor and ID, then its path, its page
/// and the address it was reached by, each JSON-quoted.
#[must_use]
pub fn ytdlp_command(
    places: &Places<'_>,
    archive: Option<&Path>,
    urls: &[String],
) -> Vec<OsString> {
    let mut cmd: Vec<OsString> = [
        "yt-dlp",
        // The library step depends on what this list produces, so a
        // personal yt-dlp config must not change it.
        "--ignore-config",
        "--merge-output-format",
        "mkv",
        // A single-file format skips the merge; the attachments below
        // need Matroska either way.
        "--remux-video",
        "mkv",
        // Without a sleep yt-dlp retries at once, which is what keeps a
        // rate limit tripped.
        "--retry-sleep",
        "http:exp=1:30",
        "--retry-sleep",
        "fragment:exp=1:30",
        "--retry-sleep",
        "extractor:exp=1:30",
        "--embed-metadata",
        "--embed-thumbnail",
        "--embed-chapters",
        "--embed-info-json",
        // --embed-subs writes a person's subtitles on its own only while
        // no generated captions are asked for; no-keep-subs then has the
        // written files deleted once embedded, as they otherwise are not.
        "--embed-subs",
        "--write-subs",
        "--compat-options",
        "no-keep-subs",
        // A watch URL inside a playlist means the one video; a playlist
        // arrives at its own address (fetch::list).
        "--no-playlist",
        "--replace-in-metadata",
        "uploader_id",
        "^@",
        "",
        // The info JSON leaves out the address yt-dlp was given, which a
        // source of the generic extractor is named and fetched again by
        // (source::page); a field of muman's own keeps it.
        "--parse-metadata",
        "original_url:(?P<muman_original_url>.+)",
        "--output",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    cmd.push(OUTPUT_TEMPLATE.into());
    let options = places.options;
    for (flag, value) in [
        ("--format", options.format.clone()),
        ("--sub-langs", options.sub_langs.clone()),
        (
            "--concurrent-fragments",
            options.concurrent_fragments.to_string(),
        ),
    ] {
        cmd.push(flag.into());
        cmd.push(value.into());
    }
    let plugins = places.plugins.filter(|_| options.plugins);
    // skip=translated_subs leaves the translations of the speech
    // recognition track, one per language; OriginalSubs drops them before
    // any is fetched. Without it, generated captions are not asked for.
    if let Some(plugins) = plugins {
        cmd.extend(["--write-auto-subs", "--no-plugin-dirs", "--plugin-dirs"].map(OsString::from));
        cmd.push(plugins.as_os_str().to_os_string());
        for pp in ["OriginalSubs", "AlbumArt"] {
            cmd.push("--use-postprocessor".into());
            cmd.push(format!("{pp}:when=pre_process").into());
        }
    }
    cmd.push("--paths".into());
    cmd.push(places.store.as_os_str().to_os_string());
    cmd.push("--paths".into());
    cmd.push(prefixed("temp:", places.temp));
    if let Some(archive) = archive {
        cmd.push("--download-archive".into());
        cmd.push(archive.as_os_str().to_os_string());
    }
    cmd.extend(crate::ytdlp_log::progress_args().map(OsString::from));
    cmd.push("--print-to-file".into());
    cmd.push(
        "after_move:%(extractor_key)s %(id)j %(filepath)j %(webpage_url)j %(original_url)j".into(),
    );
    cmd.push(places.done.as_os_str().to_os_string());
    cmd.extend(located(places.passed.choosing.as_deref()));
    if let Some(batch) = places.batch {
        cmd.push("--batch-file".into());
        cmd.push(batch.as_os_str().to_os_string());
    }
    // A video ID may begin with a dash.
    cmd.push("--".into());
    cmd.extend(urls.iter().map(OsString::from));
    cmd
}

/// `cmd`, a yt-dlp run of muman's, with the config file of `[ytdlp]
/// args` and `target` after the `--` that ends its options, as a video ID
/// may begin with a dash.
#[must_use]
pub fn ending(mut cmd: Vec<OsString>, passed: &Passed, target: &str) -> Vec<OsString> {
    cmd.extend(located(passed.every.as_deref()));
    cmd.push("--".into());
    cmd.push(target.into());
    cmd
}

/// yt-dlp's argv that prints, one JSON object a line, what each of
/// `urls` offers now, downloading nothing: asked as a fetch asks, so it
/// lists the formats a fetch would choose from.
#[must_use]
pub fn upstream_command(passed: &Passed, urls: &[String]) -> Vec<OsString> {
    let mut cmd: Vec<OsString> = [
        "yt-dlp",
        "--ignore-config",
        "--skip-download",
        "--dump-json",
        "--no-playlist",
        "--ignore-errors",
        "--retry-sleep",
        "extractor:exp=1:30",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    cmd.extend(located(passed.choosing.as_deref()));
    cmd.push("--".into());
    cmd.extend(urls.iter().map(OsString::from));
    cmd
}

fn prefixed(prefix: &str, path: &Path) -> OsString {
    let mut s = OsString::from(prefix);
    s.push(path.as_os_str());
    s
}

/// A file yt-dlp finished, by the key it is a source under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub key: SourceKey,
    pub path: PathBuf,
}

/// The files in yt-dlp's `--print-to-file` output, one per line, keyed
/// as `sites` names them. A line that does not parse is skipped rather
/// than guessed at.
#[must_use]
pub fn finished(done: &str, sites: &Sites) -> Vec<Fetched> {
    done.lines()
        .filter_map(|line| {
            let (extractor, rest) = line.trim().split_once(' ')?;
            let mut quoted = serde_json::Deserializer::from_str(rest).into_iter::<String>();
            let (id, file) = (quoted.next()?.ok()?, quoted.next()?.ok()?);
            // A field yt-dlp has no value for is written `NA`, unquoted; an
            // address has no space.
            let mut urls = rest[quoted.byte_offset()..]
                .split_whitespace()
                .map(|s| serde_json::from_str::<String>(s).ok());
            let (webpage, original) = (urls.next().flatten(), urls.next().flatten());
            let page = crate::source::page(extractor, webpage.as_deref(), original.as_deref());
            let key = sites.fetched(extractor, page, &id)?;
            Some(Fetched {
                key,
                path: PathBuf::from(file),
            })
        })
        .collect()
}

/// One stream of a file being scrubbed, as ffprobe lists it.
#[derive(Debug, Default, Deserialize)]
struct Stream {
    index: usize,
    #[serde(default)]
    codec_type: String,
    #[serde(default)]
    disposition: BTreeMap<String, u8>,
    #[serde(default)]
    tags: BTreeMap<String, String>,
}

impl Stream {
    fn tag(&self, key: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Default, Deserialize)]
struct Streams {
    #[serde(default)]
    streams: Vec<Stream>,
}

/// Keep only what muman reads of the info JSON embedded in the Matroska
/// file at `path`, its scratch in `scratch`; whether the file was
/// written again.
///
/// ffmpeg reads a JPEG or PNG attachment as a picture stream, which a
/// copy would write back as a video track, so each is dumped by its
/// packet, its bytes as they are, and attached again as it was.
pub fn scrub<R: Runner>(runner: &R, path: &Path, scratch: &Path) -> Result<bool> {
    let probed: Streams = serde_json::from_slice(&runner.output(&probe::ffprobe_command(path))?)
        .context("parsing ffprobe JSON")?;
    let attachments: Vec<&Stream> = probed
        .streams
        .iter()
        .filter(|s| s.codec_type == "attachment")
        .collect();
    let Some(ordinal) = attachments.iter().position(|s| {
        s.tag("filename")
            .is_some_and(|f| f.eq_ignore_ascii_case("info.json"))
    }) else {
        return Ok(false);
    };
    let pictures: Vec<&Stream> = probed
        .streams
        .iter()
        .filter(|s| s.disposition.get("attached_pic") == Some(&1))
        .collect();
    let scratch = scratch.join("scrub");
    std::fs::create_dir_all(&scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let json = scratch.join("info.json");
    let dumped: Vec<PathBuf> = (0..pictures.len())
        .map(|n| scratch.join(format!("picture-{n}")))
        .collect();
    let mut dump = ffmpeg::dump_command(&[(path, vec![(ordinal, json.clone())])]);
    let at = dump.iter().position(|a| a == "-map").unwrap_or(dump.len());
    let packets = pictures.iter().zip(&dumped).flat_map(|(p, to)| {
        [
            "-map".into(),
            format!("0:{}", p.index).into(),
            "-c".into(),
            "copy".into(),
            "-f".into(),
            "data".into(),
            to.as_os_str().to_os_string(),
        ]
    });
    dump.splice(at..at, packets.collect::<Vec<OsString>>());
    runner.run(&dump)?;
    let read = std::fs::read(&json).with_context(|| format!("reading {}", json.display()))?;
    let Some(kept) = info::only_read(&read) else {
        return Ok(false);
    };
    std::fs::write(&json, kept).with_context(|| format!("writing {}", json.display()))?;

    let mut cmd = ffmpeg::base();
    cmd.extend(ffmpeg::Input::from(path).args());
    cmd.extend(["-map", "0"].map(OsString::from));
    for dropped in std::iter::once(attachments[ordinal]).chain(pictures.iter().copied()) {
        cmd.push("-map".into());
        cmd.push(format!("-0:{}", dropped.index).into());
    }
    // As yt-dlp copies a file it fetched: a data stream, as an MP4's
    // chapter track, does not go into Matroska.
    cmd.extend(["-dn", "-ignore_unknown", "-c", "copy"].map(OsString::from));
    let attach = std::iter::once((json.as_path(), "application/json", "info.json")).chain(
        pictures.iter().zip(&dumped).map(|(p, to)| {
            (
                to.as_path(),
                p.tag("mimetype").unwrap_or("image/jpeg"),
                p.tag("filename").unwrap_or("cover.jpg"),
            )
        }),
    );
    for (n, (file, mimetype, filename)) in attach.enumerate() {
        let specifier = format!("-metadata:s:t:{}", attachments.len() - 1 + n);
        cmd.push("-attach".into());
        cmd.push(file.as_os_str().to_os_string());
        cmd.push(specifier.clone().into());
        cmd.push(format!("mimetype={mimetype}").into());
        cmd.push(specifier.into());
        cmd.push(format!("filename={filename}").into());
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".scrubbed");
    let to = path.with_file_name(name);
    cmd.extend(["-f", "matroska"].map(OsString::from));
    cmd.push(to.as_os_str().to_os_string());
    runner.run(&cmd)?;
    crate::atomic::rename(&to, path)
        .with_context(|| format!("moving {} to {}", to.display(), path.display()))?;
    Ok(true)
}

/// A `--download-archive` naming every key in `keys`, so a playlist's
/// videos already kept are skipped without being looked at.
#[must_use]
pub fn archive_lines<'a>(keys: impl IntoIterator<Item = &'a SourceKey>, sites: &Sites) -> String {
    keys.into_iter()
        .filter_map(|k| sites.archive_line(k))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    static OPTIONS: std::sync::LazyLock<Ytdlp> = std::sync::LazyLock::new(Ytdlp::default);
    static PASSED: Passed = Passed {
        every: None,
        choosing: None,
    };

    fn places(plugins: Option<&Path>) -> Places<'_> {
        Places {
            store: Path::new("/o"),
            temp: Path::new("/t"),
            done: Path::new("/d"),
            plugins,
            options: &OPTIONS,
            passed: &PASSED,
            batch: None,
        }
    }

    fn value_after<'a>(cmd: &'a [OsString], flag: &str) -> Vec<&'a OsString> {
        cmd.iter()
            .zip(cmd.iter().skip(1))
            .filter(|(f, _)| *f == flag)
            .map(|(_, v)| v)
            .collect()
    }

    #[test]
    fn urls_follow_the_option_terminator() {
        let urls = vec!["-dashid".to_string(), "https://x".to_string()];
        let cmd = ytdlp_command(&places(None), None, &urls);
        let end = cmd.iter().position(|a| a == "--").expect("terminator");
        assert_eq!(
            &cmd[end + 1..],
            &[OsString::from("-dashid"), OsString::from("https://x")]
        );
    }

    #[test]
    fn originals_and_temp_are_separate_paths() {
        let cmd = ytdlp_command(&places(None), None, &[]);
        let paths = value_after(&cmd, "--paths");
        assert_eq!(
            paths,
            vec![&OsString::from("/o"), &OsString::from("temp:/t")]
        );
    }

    #[test]
    fn an_archive_is_passed_when_given() {
        let cmd = ytdlp_command(&places(None), Some(Path::new("/t/skip")), &[]);
        assert_eq!(
            value_after(&cmd, "--download-archive"),
            vec![&OsString::from("/t/skip")]
        );
    }

    #[test]
    fn every_original_ends_up_matroska() {
        let cmd = ytdlp_command(&places(None), None, &[]);
        assert_eq!(
            value_after(&cmd, "--merge-output-format"),
            vec![&OsString::from("mkv")]
        );
        assert_eq!(
            value_after(&cmd, "--remux-video"),
            vec![&OsString::from("mkv")]
        );
    }

    #[test]
    fn a_remux_stopped_short_is_deleted_and_every_other_download_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = |rel: &str| {
            let path = rel
                .split('/')
                .fold(dir.path().to_path_buf(), |p, part| p.join(part));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"x").unwrap();
            path
        };
        let cut = file("Jamendo/1234567.mkv");
        let original = file("Jamendo/1234567.flac");
        let merged = file("Youtube/vid00000001.mkv");
        let cover = file("Youtube/vid00000001.webp");
        let part = file("Youtube/vid00000002.f251.webm.part");
        let torrent = file("torrent/0123/files/1.mkv");
        let beside = file("torrent/0123/files/1.flac");
        drop_cut_remuxes(dir.path());
        assert!(!cut.exists());
        for kept in [original, merged, cover, part, torrent, beside] {
            assert!(kept.exists(), "{}", kept.display());
        }
    }

    #[test]
    fn a_personal_config_is_ignored() {
        let cmd = ytdlp_command(&places(None), None, &[]);
        assert_eq!(cmd[1], "--ignore-config");
    }

    #[test]
    fn the_song_list_s_arguments_are_named_by_a_config_file_and_never_listed() {
        let dir = tempfile::tempdir().unwrap();
        let options = Ytdlp {
            args: vec!["--password".into(), "pass word".into()],
            ..Ytdlp::default()
        };
        let passed = Passed::write(&options, dir.path()).unwrap();
        let mut places = places(Some(Path::new("/p")));
        places.passed = &passed;
        let every = dir.path().join(EVERY).into_os_string();
        let choosing = dir.path().join(CHOOSING).into_os_string();
        for (cmd, file) in [
            (ytdlp_command(&places, None, &[]), &choosing),
            (upstream_command(&passed, &[]), &choosing),
            (ending(vec!["yt-dlp".into()], &passed, "u"), &every),
        ] {
            assert_eq!(value_after(&cmd, "--config-locations"), [file], "{cmd:?}");
            assert!(
                !cmd.iter()
                    .any(|a| a == "--extractor-args" || a == "--password")
            );
            let end = cmd.iter().position(|a| a == "--").unwrap();
            assert!(cmd.iter().position(|a| a == "--config-locations").unwrap() < end);
        }
        assert_eq!(
            std::fs::read_to_string(&every).unwrap(),
            "\u{feff}'--password'\n'pass word'\n"
        );
        let choosing = std::fs::read_to_string(&choosing).unwrap();
        assert!(
            choosing.starts_with(
                "\u{feff}'--extractor-args'\n\
                 'youtube:skip=translated_subs;player_client=default,web_music'\n"
            ),
            "{choosing}"
        );
        assert!(
            choosing.ends_with("'--password'\n'pass word'\n"),
            "{choosing}"
        );
    }

    #[test]
    fn without_arguments_no_config_file_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let none = Ytdlp {
            extractor_args: BTreeMap::new(),
            ..Ytdlp::default()
        };
        let passed = Passed::write(&none, dir.path()).unwrap();
        assert_eq!(passed, Passed::default());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let cmd = ending(vec!["yt-dlp".into()], &passed, "u");
        assert_eq!(cmd, ["yt-dlp", "--", "u"]);
    }

    #[test]
    fn an_argument_is_quoted_as_yt_dlp_splits_it_back() {
        let quoted = config(&[
            "it's a \"name\"".into(),
            String::new(),
            "#not a comment $HOME \\n\nnext line ☂".into(),
        ]);
        assert_eq!(
            String::from_utf8(quoted).unwrap(),
            "\u{feff}'it'\"'\"'s a \"name\"'\n''\n'#not a comment $HOME \\n\nnext line ☂'\n"
        );
    }

    #[test]
    fn without_an_archive_nothing_is_skipped() {
        let cmd = ytdlp_command(&places(None), None, &[]);
        assert!(!cmd.iter().any(|a| a == "--download-archive"));
    }

    #[test]
    fn progress_is_one_line_each() {
        let cmd = ytdlp_command(&places(None), None, &[]);
        assert!(cmd.iter().any(|a| a == "--newline"));
        assert_eq!(value_after(&cmd, "--progress-template").len(), 1);
    }

    #[test]
    fn the_archive_names_remote_keys_only() {
        let keys = [
            SourceKey::youtube("aaaaaaaaaaa"),
            SourceKey::Manual("x.flac".into()),
        ];
        assert_eq!(
            archive_lines(&keys, &Sites::default()),
            "youtube aaaaaaaaaaa\n"
        );
    }

    #[test]
    fn without_the_plugin_no_generated_captions_are_asked_for() {
        let cmd = ytdlp_command(&places(None), None, &[]);
        assert!(!cmd.iter().any(|a| a == "--write-auto-subs"));
        assert!(!cmd.iter().any(|a| a == "--use-postprocessor"));
    }

    #[test]
    fn the_address_given_is_kept_in_the_info_json() {
        let cmd = ytdlp_command(&places(None), None, &[]);
        assert_eq!(
            value_after(&cmd, "--parse-metadata"),
            vec![&OsString::from("original_url:(?P<muman_original_url>.+)")]
        );
    }

    /// A fetch's streams: audio, its info JSON, a JPEG cover ffmpeg reads
    /// as a picture, and a WebP one it leaves an attachment.
    const FETCHED: &str = r#"{"streams": [
        {"index": 0, "codec_type": "audio", "codec_name": "opus"},
        {"index": 1, "codec_type": "attachment", "tags": {"filename": "info.json", "mimetype": "application/json"}},
        {"index": 2, "codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1},
         "tags": {"filename": "cover.jpg", "mimetype": "image/jpeg"}},
        {"index": 3, "codec_type": "attachment", "tags": {"filename": "cover.webp", "mimetype": "image/webp"}}
    ]}"#;

    #[test]
    fn a_fetched_file_keeps_no_cookie_or_header_and_every_picture() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("vid00000001.mkv");
        std::fs::write(&file, b"mkv").unwrap();
        let fake = crate::testing::Fake::default()
            .probe("vid00000001", FETCHED)
            .info(
                "vid00000001",
                r#"{"id": "vid00000001", "title": "Lantern Weather", "cookies": "sess=TOKEN-0001",
                    "formats": [{"format_id": "251", "http_headers": {"Cookie": "TOKEN-0001"}}]}"#,
            );
        assert!(scrub(&fake, &file, dir.path()).unwrap());
        let attached = fake.attached.lock().unwrap();
        assert!(!attached[0].contains("TOKEN"), "{attached:?}");
        assert!(attached[0].contains("Lantern Weather"), "{attached:?}");
        let remux = fake
            .calls()
            .into_iter()
            .find(|c| c.iter().any(|a| a == "-attach"))
            .unwrap()
            .join(" ");
        assert!(remux.contains("-map 0 -map -0:1 -map -0:2 "), "{remux}");
        assert!(
            remux.contains(
                "-metadata:s:t:1 mimetype=application/json -metadata:s:t:1 filename=info.json"
            ),
            "{remux}"
        );
        assert!(
            remux
                .contains("-metadata:s:t:2 mimetype=image/jpeg -metadata:s:t:2 filename=cover.jpg"),
            "{remux}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"mkv");

        let clean = crate::testing::Fake::default()
            .probe("vid00000001", FETCHED)
            .info("vid00000001", r#"{"id": "vid00000001"}"#);
        assert!(!scrub(&clean, &file, dir.path()).unwrap());
        assert!(clean.attached.lock().unwrap().is_empty());
    }

    #[test]
    fn subtitles_are_embedded_and_not_kept() {
        let cmd = ytdlp_command(&places(None), None, &[]);
        for flag in ["--embed-subs", "--write-subs"] {
            assert!(cmd.iter().any(|a| a == flag), "{flag}");
        }
        assert_eq!(
            value_after(&cmd, "--compat-options"),
            vec![&OsString::from("no-keep-subs")]
        );
    }

    #[test]
    fn finished_files_decode_their_key_and_escaped_path() {
        let done = "Youtube \"aaaaaaaaaaa\" \"/o/r/\\u96e8 \\u29f8 x [aaaaaaaaaaa].mkv\" \
                    \"https://www.youtube.com/watch?v=aaaaaaaaaaa\" \
                    \"https://music.youtube.com/watch?v=aaaaaaaaaaa\"\n\nnot json\n\
                    Youtube \"bad\" \"/o/b.mkv\" NA NA\n\
                    Youtube aaaaaaaaaab \"/o/b.mkv\" NA NA\n\
                    Generic \"a\" \"/o/x/a [a].mp3\" \"https://mirror7.files.example/0/a.mp3\" \
                    \"https://www.files.example/download/a.mp3\"\n\
                    Funkwhale \"b\" \"/o/x/b [b].mp3\" NA NA\n\
                    ArchiveOrg \"i/P_1.mp3\" \"/o/u/P_1.mp3 [i\\u29f8P_1.mp3].mp3\" \
                    \"https://archive.org/details/i\" NA\n\
                    ArchiveOrg \"i/01 Lantern #2.mp3\" \"/o/ArchiveOrg/i\\u29f801 Lantern #2.mp3.mp3\" \
                    \"https://archive.org/details/i\" NA\n";
        assert_eq!(
            finished(done, &Sites::default()),
            vec![
                Fetched {
                    key: SourceKey::youtube("aaaaaaaaaaa"),
                    path: PathBuf::from("/o/r/雨 ⧸ x [aaaaaaaaaaa].mkv"),
                },
                Fetched {
                    key: SourceKey::parse("files.example:a").unwrap(),
                    path: PathBuf::from("/o/x/a [a].mp3"),
                },
                Fetched {
                    key: SourceKey::parse("funkwhale:b").unwrap(),
                    path: PathBuf::from("/o/x/b [b].mp3"),
                },
                Fetched {
                    key: SourceKey::parse("archive.org:i/P_1.mp3").unwrap(),
                    path: PathBuf::from("/o/u/P_1.mp3 [i⧸P_1.mp3].mp3"),
                },
                Fetched {
                    key: SourceKey::parse("archive.org:i/01 Lantern #2.mp3").unwrap(),
                    path: PathBuf::from("/o/ArchiveOrg/i⧸01 Lantern #2.mp3.mp3"),
                },
            ]
        );
    }
}
