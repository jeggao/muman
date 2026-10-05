//! The yt-dlp call that fetches sources into the store, and the list of
//! files it reports finishing.
//!
//! yt-dlp runs with `--ignore-config`, so a personal config cannot change
//! what a source holds; the song list's `[ytdlp]` is how options reach
//! it. Unfinished downloads wait in `partial/` under the state root,
//! beside the store, so an interrupted one resumes on the next run and a
//! finished one is renamed into the store rather than copied.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::settings::Ytdlp;
use crate::source::SourceKey;

/// `<handle>/<title> [<id>].mkv`. The byte limits keep a long title in
/// a multi-byte script under the filesystem's 255-byte name limit; the
/// fallbacks cover a channel with no handle.
pub const OUTPUT_TEMPLATE: &str =
    "%(uploader_id,channel_id,uploader|unknown).80B/%(title).150B [%(id)s].%(ext)s";

/// The default template's name, under a folder chosen beforehand.
#[must_use]
pub fn template_in(folder: &str) -> String {
    format!("{}/%(title).150B [%(id)s].%(ext)s", escape(folder))
}

/// A template naming one file exactly, whatever its extension ends up.
#[must_use]
pub fn template_at(relative: &Path) -> String {
    format!(
        "{}.%(ext)s",
        escape(&relative.with_extension("").to_string_lossy())
    )
}

fn escape(literal: &str) -> String {
    literal.replace('%', "%%")
}

/// Where a run's files go: the store, partial downloads, and the list of
/// finished files.
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
/// appended to `done` as its extractor, ID and JSON-quoted path.
#[must_use]
pub fn ytdlp_command(
    places: &Places<'_>,
    template: &str,
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
        "--output",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    cmd.push(template.into());
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
    // Extended attributes keep the source URL with the file; Windows'
    // and many removable filesystems have none to keep it in.
    if cfg!(unix) {
        cmd.push("--xattrs".into());
    }
    // Machine-translated captions run to hundreds per video and trip the
    // rate limit, where one failed track aborts the download.
    let mut extractor = String::from("youtube:skip=translated_subs");
    // skip=translated_subs leaves the translations of the speech
    // recognition track, one per language; OriginalSubs drops them before
    // any is fetched. Without it, generated captions are not asked for.
    if let Some(plugins) = places.plugins.filter(|_| options.plugins) {
        // Only the web_music client lists a track's square album art.
        extractor.push_str(";player_client=default,web_music");
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
    cmd.push("after_move:%(extractor_key)s %(id)s %(filepath)j".into());
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

/// The files in yt-dlp's `--print-to-file` output, one per line. A line
/// that does not parse is skipped rather than guessed at.
#[must_use]
pub fn finished(done: &str) -> Vec<Fetched> {
    done.lines()
        .filter_map(|line| {
            let mut parts = line.trim().splitn(3, ' ');
            let (extractor, id, path) = (parts.next()?, parts.next()?, parts.next()?);
            let key = SourceKey::parse(&format!("{}:{id}", extractor.to_ascii_lowercase())).ok()?;
            let path = serde_json::from_str::<String>(path).ok()?;
            Some(Fetched {
                key,
                path: PathBuf::from(path),
            })
        })
        .collect()
}

/// A `--download-archive` naming every key in `keys`, so a playlist's
/// videos already kept are skipped without being looked at.
#[must_use]
pub fn archive_lines<'a>(keys: impl IntoIterator<Item = &'a SourceKey>) -> String {
    keys.into_iter()
        .filter_map(|k| match k {
            SourceKey::Remote { extractor, id } => Some(format!("{extractor} {id}\n")),
            SourceKey::Manual(_) => None,
        })
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
        let cmd = ytdlp_command(&places(None), OUTPUT_TEMPLATE, None, &urls);
        let end = cmd.iter().position(|a| a == "--").expect("terminator");
        assert_eq!(
            &cmd[end + 1..],
            &[OsString::from("-dashid"), OsString::from("https://x")]
        );
    }

    #[test]
    fn originals_and_temp_are_separate_paths() {
        let cmd = ytdlp_command(&places(None), OUTPUT_TEMPLATE, None, &[]);
        let paths = value_after(&cmd, "--paths");
        assert_eq!(
            paths,
            vec![&OsString::from("/o"), &OsString::from("temp:/t")]
        );
    }

    #[test]
    fn an_archive_is_passed_when_given() {
        let cmd = ytdlp_command(
            &places(None),
            OUTPUT_TEMPLATE,
            Some(Path::new("/t/skip")),
            &[],
        );
        assert_eq!(
            value_after(&cmd, "--download-archive"),
            vec![&OsString::from("/t/skip")]
        );
    }

    #[test]
    fn every_original_ends_up_matroska() {
        let cmd = ytdlp_command(&places(None), OUTPUT_TEMPLATE, None, &[]);
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
        let cmd = ytdlp_command(&places(None), OUTPUT_TEMPLATE, None, &[]);
        assert_eq!(cmd[1], "--ignore-config");
    }

    #[test]
    fn translated_captions_are_skipped() {
        let cmd = ytdlp_command(&places(None), OUTPUT_TEMPLATE, None, &[]);
        let args = value_after(&cmd, "--extractor-args");
        assert!(args[0].to_string_lossy().contains("skip=translated_subs"));
    }

    #[test]
    fn with_the_plugin_original_captions_are_kept_too() {
        let cmd = ytdlp_command(&places(Some(Path::new("/p"))), OUTPUT_TEMPLATE, None, &[]);
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
        let cmd = ytdlp_command(&places(None), OUTPUT_TEMPLATE, None, &[]);
        assert!(!cmd.iter().any(|a| a == "--download-archive"));
    }

    #[test]
    fn progress_is_one_line_each() {
        let cmd = ytdlp_command(&places(None), OUTPUT_TEMPLATE, None, &[]);
        assert!(cmd.iter().any(|a| a == "--newline"));
        assert_eq!(value_after(&cmd, "--progress-template").len(), 1);
    }

    #[test]
    fn the_archive_names_remote_keys_only() {
        let keys = [
            SourceKey::youtube("aaaaaaaaaaa"),
            SourceKey::Manual("x.flac".into()),
        ];
        assert_eq!(archive_lines(&keys), "youtube aaaaaaaaaaa\n");
    }

    #[test]
    fn templates_escape_literal_percents() {
        assert_eq!(template_in("100%"), "100%%/%(title).150B [%(id)s].%(ext)s");
        assert_eq!(
            template_at(Path::new("chan/50% [id].mkv")),
            "chan/50%% [id].%(ext)s"
        );
    }

    #[test]
    fn without_the_plugin_no_generated_captions_are_asked_for() {
        let cmd = ytdlp_command(&places(None), OUTPUT_TEMPLATE, None, &[]);
        assert!(!cmd.iter().any(|a| a == "--write-auto-subs"));
        assert!(!cmd.iter().any(|a| a == "--use-postprocessor"));
    }

    #[test]
    fn subtitles_are_embedded_and_not_kept() {
        let cmd = ytdlp_command(&places(None), OUTPUT_TEMPLATE, None, &[]);
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
        let done = "Youtube aaaaaaaaaaa \"/o/r/\\u96e8 \\u29f8 x [aaaaaaaaaaa].mkv\"\n\nnot json\nYoutube bad \"/o/b.mkv\"\n";
        assert_eq!(
            finished(done),
            vec![Fetched {
                key: SourceKey::youtube("aaaaaaaaaaa"),
                path: PathBuf::from("/o/r/雨 ⧸ x [aaaaaaaaaaa].mkv"),
            }]
        );
    }
}
