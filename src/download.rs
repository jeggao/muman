//! The yt-dlp call that fetches sources into the store, and the list of
//! files it reports finishing.
//!
//! yt-dlp runs with `--ignore-config`, so a personal config cannot change
//! what a source holds; the song list's `[ytdlp]` is how options reach
//! it. Unfinished downloads wait in `partial/` under the state root,
//! beside the store, named by the extractor and ID alone, so an
//! interrupted one resumes at the same path on the next run whatever
//! its uploader is called by then; a finished one is renamed into the
//! store's staging folder rather than copied, and from there to its
//! key's place ([`crate::store::place`]).
//!
//! The info JSON yt-dlp embeds holds the cookies and headers its
//! requests sent ([`crate::info`]), and no yt-dlp option leaves them out
//! of the formats it lists, which muman reads. So each finished file is
//! [`scrub`]bed before it is placed: its info JSON dumped, those fields
//! dropped, and the file remuxed once with the JSON put back in its
//! place. A remux keeps every stream's packets, pictures and chapters, so
//! nothing a source is measured or known by changes. A file whose JSON
//! holds none of them, or none at all, is left as it is.

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
    /// A file holding the URLs, one per line, in place of the command
    /// line, which Windows caps at 32,767 characters.
    pub batch: Option<&'a Path>,
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
    let extractor = extractor_args(plugins.is_some());
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
    cmd.push("--extractor-args".into());
    cmd.push(extractor.into());
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
        "after_move:%(extractor_key)s %(id)s %(filepath)j %(webpage_url)j %(original_url)j".into(),
    );
    cmd.push(places.done.as_os_str().to_os_string());
    cmd.extend(options.args.iter().map(OsString::from));
    if let Some(batch) = places.batch {
        cmd.push("--batch-file".into());
        cmd.push(batch.as_os_str().to_os_string());
    }
    // A video ID may begin with a dash.
    cmd.push("--".into());
    cmd.extend(urls.iter().map(OsString::from));
    cmd
}

/// The `--extractor-args` a fetch passes, the clients it asks with
/// `plugins` loaded.
fn extractor_args(plugins: bool) -> String {
    // Machine-translated captions run to hundreds per video and trip the
    // rate limit, where one failed track aborts the download.
    let mut extractor = String::from("youtube:skip=translated_subs");
    if plugins {
        // Only the web_music client lists a track's square album art.
        extractor.push_str(";player_client=default,web_music");
    }
    extractor
}

/// yt-dlp's argv that prints, one JSON object a line, what each of
/// `urls` offers now, downloading nothing: asked as a fetch asks, so it
/// lists the formats a fetch would choose from.
#[must_use]
pub fn upstream_command(options: &Ytdlp, urls: &[String]) -> Vec<OsString> {
    let mut cmd: Vec<OsString> = [
        "yt-dlp",
        "--ignore-config",
        "--skip-download",
        "--dump-json",
        "--no-playlist",
        "--ignore-errors",
        "--retry-sleep",
        "extractor:exp=1:30",
        "--extractor-args",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    cmd.push(extractor_args(options.plugins).into());
    cmd.extend(options.args.iter().map(OsString::from));
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
            let mut parts = line.trim().splitn(3, ' ');
            let (extractor, id, rest) = (parts.next()?, parts.next()?, parts.next()?);
            let mut path = serde_json::Deserializer::from_str(rest).into_iter::<String>();
            let file = path.next()?.ok()?;
            // A field yt-dlp has no value for is written `NA`, unquoted; an
            // address has no space.
            let mut urls = rest[path.byte_offset()..]
                .split_whitespace()
                .map(|s| serde_json::from_str::<String>(s).ok());
            let (webpage, original) = (urls.next().flatten(), urls.next().flatten());
            let page = crate::source::page(extractor, webpage.as_deref(), original.as_deref());
            let key = sites.fetched(extractor, page, id)?;
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

/// Drop what the fetch's requests sent from the info JSON embedded in
/// the Matroska file at `path`, its scratch in `scratch`; whether the
/// file was written again.
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
    let Some(kept) = info::without_sent(&read) else {
        return Ok(false);
    };
    std::fs::write(&json, kept).with_context(|| format!("writing {}", json.display()))?;

    let mut cmd = ffmpeg::base();
    cmd.push("-i".into());
    cmd.push(path.as_os_str().to_os_string());
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

    fn places(plugins: Option<&Path>) -> Places<'_> {
        Places {
            store: Path::new("/o"),
            temp: Path::new("/t"),
            done: Path::new("/d"),
            plugins,
            options: &OPTIONS,
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
    fn a_personal_config_is_ignored() {
        let cmd = ytdlp_command(&places(None), None, &[]);
        assert_eq!(cmd[1], "--ignore-config");
    }

    #[test]
    fn translated_captions_are_skipped() {
        let cmd = ytdlp_command(&places(None), None, &[]);
        let args = value_after(&cmd, "--extractor-args");
        assert!(args[0].to_string_lossy().contains("skip=translated_subs"));
    }

    #[test]
    fn with_the_plugin_original_captions_are_kept_too() {
        let cmd = ytdlp_command(&places(Some(Path::new("/p"))), None, &[]);
        assert!(cmd.iter().any(|a| a == "--write-auto-subs"));
        let clear = cmd.iter().position(|a| a == "--no-plugin-dirs").unwrap();
        let add = cmd.iter().position(|a| a == "--plugin-dirs").unwrap();
        assert!(clear < add, "clearing after adding would drop the plugin");
        assert_eq!(
            value_after(&cmd, "--plugin-dirs"),
            vec![&OsString::from("/p")]
        );
        assert_eq!(
            value_after(&cmd, "--use-postprocessor"),
            vec![
                &OsString::from("OriginalSubs:when=pre_process"),
                &OsString::from("AlbumArt:when=pre_process"),
            ]
        );
        let args = value_after(&cmd, "--extractor-args");
        assert!(args[0].to_string_lossy().contains("web_music"));
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
        let done = "Youtube aaaaaaaaaaa \"/o/r/\\u96e8 \\u29f8 x [aaaaaaaaaaa].mkv\" \
                    \"https://www.youtube.com/watch?v=aaaaaaaaaaa\" \
                    \"https://music.youtube.com/watch?v=aaaaaaaaaaa\"\n\nnot json\n\
                    Youtube bad \"/o/b.mkv\" NA NA\n\
                    Generic a \"/o/x/a [a].mp3\" \"https://mirror7.files.example/0/a.mp3\" \
                    \"https://www.files.example/download/a.mp3\"\n\
                    Funkwhale b \"/o/x/b [b].mp3\" NA NA\n\
                    ArchiveOrg i/P_1.mp3 \"/o/u/P_1.mp3 [i\\u29f8P_1.mp3].mp3\" \
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
            ]
        );
    }
}
