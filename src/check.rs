//! `muman check`: the library and the sources against what earlier
//! runs recorded. Reads only, takes no lock; `--decode` decodes every
//! listed source in full, where a truncated download fails, and
//! `--upstream` asks each site whether it still serves each fetched
//! source as its song records, downloading nothing: of a source a site
//! re-encoded or took down, the copy in the store is the one to keep.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::codec::Codec;
use crate::dirs::Dirs;
use crate::held::{Held, Upstream};
use crate::manifest::Manifest;
use crate::parallel;
use crate::runner::{Line, Runner};
use crate::source::SourceKey;
use crate::state::State;
use crate::store::{self, Kind, Store};

/// Whether a library file muman writes could end in `extension`.
fn is_written(extension: &str) -> bool {
    extension == "lrc" || Codec::ALL.iter().any(|c| c.extension() == extension)
}

fn decode_command(path: &Path) -> Vec<OsString> {
    let mut cmd: Vec<OsString> = [
        "ffmpeg",
        "-hide_banner",
        "-nostdin",
        "-v",
        "error",
        "-xerror",
        "-i",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    cmd.push(path.as_os_str().to_os_string());
    cmd.extend(["-map", "0:a:0", "-f", "null", "-"].map(OsString::from));
    cmd
}

/// Say every problem found to `report`, and what it is doing to `out`.
/// Returns whether there was none.
#[allow(clippy::too_many_lines)]
pub fn check<R: Runner, W: Write, D: Write>(
    runner: &R,
    dirs: &Dirs,
    (decode, upstream): (bool, bool),
    out: &mut W,
    report: &mut D,
) -> Result<bool> {
    let manifest = Manifest::load(&dirs.home)?;
    for line in crate::migrate::notices(&manifest.renamed, manifest.respelled_notice()) {
        crate::ui::info(report, &line)?;
    }
    for line in crate::settings::stale_warnings(&manifest.stale_defaults) {
        crate::ui::warning(report, &line)?;
    }
    let state = State::load(&dirs.home)?;
    let store = Store::scan(dirs)?;
    let library = &dirs.library;
    let mut problems = 0_usize;
    let mut problem = |out: &mut D, text: String| {
        problems += 1;
        crate::ui::warning(out, &text)
    };

    let mut owned: BTreeSet<PathBuf> = BTreeSet::new();
    for (path, written) in &state.outputs {
        owned.insert(path.clone());
        owned.extend(written.lyrics.iter().cloned());
        let file = library.join(path);
        match std::fs::metadata(&file) {
            Err(_) => problem(
                report,
                format!(
                    "Missing, `sync` writes it again: {}",
                    crate::relpath::show(path)
                ),
            )?,
            Ok(m) if m.len() == 0 => problem(
                report,
                format!(
                    "Empty, `sync` writes it again: {}",
                    crate::relpath::show(path)
                ),
            )?,
            Ok(_)
                if written
                    .lyrics
                    .as_ref()
                    .is_some_and(|l| written.plan.is_some() && !library.join(l).exists()) =>
            {
                problem(
                    report,
                    format!(
                        "Lyrics missing, `sync` writes them again: {}",
                        crate::relpath::show(path)
                    ),
                )?;
            }
            Ok(_) if written.plan.is_none() => problem(
                report,
                format!(
                    "Not finished by an interrupted run, `sync` writes it again: {}",
                    crate::relpath::show(path)
                ),
            )?,
            Ok(_) => {
                let changed = written
                    .stamp
                    .as_ref()
                    .is_some_and(|s| store::stamp_text(&file).as_ref() != Some(s));
                if changed {
                    problem(
                        report,
                        format!(
                            "Changed since muman wrote it, so left alone: {} (`sync --force` writes it again)",
                            crate::relpath::show(path)
                        ),
                    )?;
                }
            }
        }
    }
    for file in store::files_below(library, usize::MAX, |_| true)? {
        let Ok(rel) = file.strip_prefix(library) else {
            continue;
        };
        if rel
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("part"))
        {
            problem(
                report,
                format!("Left by an interrupted run: {}", file.display()),
            )?;
            continue;
        }
        let ours = rel
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(is_written);
        if ours && !owned.contains(rel) {
            crate::ui::info(
                report,
                &format!("Not muman's, left alone: {}", crate::relpath::show(rel)),
            )?;
        }
    }

    let listed = manifest.keys();
    for key in &listed {
        if !store.has(key) {
            let page = manifest
                .song_with(key)
                .and_then(|s| s.held.get(key))
                .is_some_and(|h| h.url.is_some());
            let fetchable = key.url().is_some() || page || crate::provider::is_kept(key);
            let how = if fetchable {
                "`sync` fetches it again"
            } else {
                "its song cannot be written"
            };
            problem(report, format!("Missing from the store, {how}: {key}"))?;
        }
    }
    for (key, failure) in &state.failures {
        problem(
            report,
            format!(
                "Failed {} time(s) to {:?}: {key}: {}",
                failure.count, failure.step, failure.error
            ),
        )?;
    }
    if decode {
        let media: Vec<_> = listed
            .iter()
            .filter_map(|k| store.locate(k))
            .filter(|l| l.kind == Kind::Media)
            .collect();
        crate::ui::info(out, &format!("Decoding {} source(s)", media.len()))?;
        out.flush()?;
        let step = crate::progress::step("Decoding", Some(media.len() as u64));
        let decoded = parallel::map(&media, parallel::builds(), |l| {
            let _working = step.working(&crate::progress::label(&l.path));
            runner.run(&decode_command(&l.path))
        });
        for (l, result) in media.iter().zip(decoded) {
            if let Err(e) = result {
                problem(report, format!("Does not decode in full: {}: {e:#}", l.key))?;
            }
        }
    }
    if upstream {
        for (problem_found, line) in served(runner, &manifest, &store, out)? {
            if problem_found {
                problem(report, line)?;
            } else {
                crate::ui::info(report, &line)?;
            }
        }
    }
    if problems == 0 {
        crate::ui::success(report, "No problems found")?;
    }
    Ok(problems == 0)
}

/// What each site serves now of each fetched source its song records a
/// format for, by one yt-dlp run that downloads nothing: a line for each
/// served otherwise, with whether it is a problem, then how many are
/// served as fetched.
fn served<R: Runner, W: Write>(
    runner: &R,
    manifest: &Manifest,
    store: &Store,
    out: &mut W,
) -> Result<Vec<(bool, String)>> {
    let keep = |key: &SourceKey| {
        if store.has(key) {
            "keep the copy in the store, the only one of what was fetched"
        } else {
            "the store holds no copy of what was fetched either"
        }
    };
    let asked: Vec<(&SourceKey, &Held, String)> = manifest
        .songs
        .iter()
        .flat_map(|s| &s.held)
        .filter(|(k, h)| matches!(k, SourceKey::Remote { .. }) && h.format.is_some())
        .filter_map(|(k, h)| Some((k, h, k.url().or_else(|| h.url.clone())?)))
        .collect();
    if asked.is_empty() {
        return Ok(vec![(
            false,
            "No fetched source records a format to ask its site about; `sync` records them"
                .to_string(),
        )]);
    }
    crate::ui::info(
        out,
        &format!(
            "Asking the sites of {} source(s) what they serve now",
            asked.len()
        ),
    )?;
    out.flush()?;
    let urls: Vec<String> = asked.iter().map(|(_, _, u)| u.clone()).collect();
    let cmd = crate::download::upstream_command(&manifest.settings.ytdlp, &urls);
    let (mut listed, mut errors) = (Vec::new(), Vec::new());
    runner.stream(&cmd, &mut |line| match line {
        Line::Out(l) => listed.extend(
            crate::info::parse(l.as_bytes())
                .ok()
                .map(|i| (i, crate::info::tags_digest(l.as_bytes()))),
        ),
        Line::Err(l) => errors.push(l.trim().to_string()),
    })?;
    let mut lines = Vec::new();
    let mut same = 0;
    for (key, held, _) in &asked {
        let SourceKey::Remote { site, id } = key else {
            continue;
        };
        let info = listed.iter().find(|(i, _)| {
            i.id.as_deref() == Some(id.as_str())
                && (i.key().as_ref() == Some(key)
                    || i.extractor_key
                        .as_deref()
                        .is_some_and(|e| e.eq_ignore_ascii_case(site)))
        });
        let Some((info, tags)) = info else {
            let why = errors
                .iter()
                .find(|e| e.contains(&format!("] {id}:")))
                .map_or("its site listed nothing", |e| {
                    e.trim_start_matches("ERROR: ")
                });
            lines.push((true, format!("Not served now: {key}: {why}; {}", keep(key))));
            continue;
        };
        let (as_fetched, said) = judged(key, held, info, tags.as_deref(), keep(key));
        same += usize::from(as_fetched);
        lines.extend(said);
    }
    lines.push((
        false,
        format!("{same} of {} source(s) served as fetched", asked.len()),
    ));
    Ok(lines)
}

/// Whether a site serving `key` as `info` lists it, its tags of digest
/// `tags`, serves it as `held` records, and a line for each way it does
/// not, or serves it otherwise unharmed, with whether it is a problem;
/// `keep` says what of the store's copy.
fn judged(
    key: &SourceKey,
    held: &Held,
    info: &crate::info::VideoInfo,
    tags: Option<&str>,
    keep: &str,
) -> (bool, Vec<(bool, String)>) {
    let mut lines = Vec::new();
    let retitled = tags.is_some_and(|t| held.tags_differ(t));
    if retitled {
        lines.push((
            true,
            format!(
                "Serves other tags: {key}, as a title or artist edited; a fetch again brings \
                 them; {keep}"
            ),
        ));
    }
    let audio = match held.upstream(info) {
        Some(Upstream::Same) | None => true,
        Some(Upstream::Renumbered(f)) => {
            lines.push((
                false,
                format!("Served as fetched, now as format {f}: {key}"),
            ));
            true
        }
        Some(Upstream::Unsized(f)) => {
            lines.push((
                false,
                format!("Still serves format {f}, with no size to compare: {key}"),
            ));
            false
        }
        Some(Upstream::Resized { format, was, now }) => {
            lines.push((
                true,
                format!(
                    "Serves other audio: {key}: format {format} is {now} bytes, {was} when \
                     fetched; {keep}"
                ),
            ));
            false
        }
        Some(Upstream::Withdrawn(f)) => {
            lines.push((
                true,
                format!(
                    "No longer serves format {f}: {key}, a fetch again bringing another; {keep}"
                ),
            ));
            false
        }
    };
    (audio && !retitled, lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Written;
    use crate::testing::Fake;

    #[test]
    fn upstream_names_each_source_a_site_serves_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("lib"),
        };
        std::fs::create_dir_all(&dirs.home).unwrap();
        let song = |id: &str, size: u64| {
            format!(
                "[[song]]\nsources = [\"youtube.com:{id}\"]\nheld.\"youtube.com:{id}\" = \
                 {{ audio = \"0123456789abcdef\", format = \"251\", size = {size} }}\n"
            )
        };
        std::fs::write(
            dirs.manifest(),
            format!(
                "version = 1\n{}{}{}",
                song("aaaaaaaaaaa", 100),
                song("bbbbbbbbbbb", 200),
                song("ccccccccccc", 300)
            ),
        )
        .unwrap();
        let listing = |id: &str, size: u64| {
            format!(
                r#"{{"id": "{id}", "extractor_key": "Youtube", "formats": [{{"format_id": "251", "filesize": {size}, "acodec": "opus", "vcodec": "none"}}]}}"#
            )
        };
        let fake = Fake {
            lines: vec![listing("aaaaaaaaaaa", 100), listing("bbbbbbbbbbb", 250)],
            ..Fake::default()
        };
        let (mut out, mut report) = (Vec::new(), Vec::new());
        let ok = check(&fake, &dirs, (false, true), &mut out, &mut report).unwrap();
        let text = crate::ui::plain(&String::from_utf8(report).unwrap());
        assert!(!ok);
        assert!(
            text.contains(
                "Serves other audio: youtube.com:bbbbbbbbbbb: format 251 is 250 bytes, 200 when fetched"
            ),
            "{text}"
        );
        assert!(
            text.contains("Not served now: youtube.com:ccccccccccc: its site listed nothing"),
            "{text}"
        );
        assert!(!text.contains("youtube.com:aaaaaaaaaaa:"), "{text}");
        assert!(
            text.contains("1 of 3 source(s) served as fetched"),
            "{text}"
        );
        let ran = fake.calls();
        let asked = ran
            .iter()
            .find(|c| c.iter().any(|a| a == "--skip-download"))
            .unwrap();
        assert!(!asked.iter().any(|a| a == "--format"), "nothing fetched");
    }

    #[test]
    fn upstream_names_a_source_whose_site_serves_other_tags() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("lib"),
        };
        std::fs::create_dir_all(&dirs.home).unwrap();
        let listing = |title: &str| {
            format!(
                r#"{{"id": "aaaaaaaaaaa", "extractor_key": "Youtube", "title": "{title}", "epoch": 1, "formats": [{{"format_id": "251", "filesize": 100, "acodec": "opus", "vcodec": "none"}}]}}"#
            )
        };
        let was = crate::info::tags_digest(listing("Lantern Weather").as_bytes()).unwrap();
        std::fs::write(
            dirs.manifest(),
            format!(
                "version = 1\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n\
                 held.\"youtube.com:aaaaaaaaaaa\" = {{ audio = \"0123456789abcdef\", tags = \"{}\", \
                 format = \"251\", size = 100 }}\n",
                &was[..16]
            ),
        )
        .unwrap();
        let ask = |title: &str| {
            let fake = Fake {
                lines: vec![listing(title).replace("\"epoch\": 1", "\"epoch\": 2")],
                ..Fake::default()
            };
            let (mut out, mut report) = (Vec::new(), Vec::new());
            check(&fake, &dirs, (false, true), &mut out, &mut report).unwrap();
            crate::ui::plain(&String::from_utf8(report).unwrap())
        };
        let same = ask("Lantern Weather");
        assert!(
            same.contains("1 of 1 source(s) served as fetched"),
            "a new epoch is no change: {same}"
        );
        let retitled = ask("Paper Comets");
        assert!(
            retitled.contains("Serves other tags: youtube.com:aaaaaaaaaaa"),
            "{retitled}"
        );
        assert!(
            retitled.contains("0 of 1 source(s) served as fetched"),
            "{retitled}"
        );
    }

    #[test]
    fn missing_changed_and_stray_files_are_found() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("lib"),
        };
        std::fs::create_dir_all(dirs.library.join("A")).unwrap();
        std::fs::create_dir_all(&dirs.home).unwrap();
        std::fs::write(dirs.library.join("A/kept.opus"), "x").unwrap();
        std::fs::write(dirs.library.join("A/changed.opus"), "x").unwrap();
        std::fs::write(dirs.library.join("A/stray.opus"), "x").unwrap();
        std::fs::write(dirs.library.join("A/half.opus.part"), "x").unwrap();
        let mut state = State::default();
        let written = |stamp: Option<String>| Written {
            sources: Vec::new(),
            lyrics: None,
            plan: Some(crate::resolve::Plan {
                version: crate::resolve::RENDER_VERSION,
                format: crate::resolve::Format::Copy { codec: Codec::Opus },
                audio: crate::resolve::AudioRef {
                    key: crate::source::SourceKey::youtube("aaaaaaaaaaa"),
                    rev: "1".into(),
                    index: 1,
                },
                cover: None,
                lyrics: None,
                tags: Vec::new(),
            }),
            stamp,
        };
        let kept = store::stamp_text(&dirs.library.join("A/kept.opus"));
        state.outputs.insert("A/kept.opus".into(), written(kept));
        state
            .outputs
            .insert("A/changed.opus".into(), written(Some("1:1".into())));
        state.outputs.insert("A/gone.opus".into(), written(None));
        std::fs::write(dirs.library.join("A/sung.opus"), "x").unwrap();
        let sung = Written {
            lyrics: Some("A/sung.lrc".into()),
            ..written(store::stamp_text(&dirs.library.join("A/sung.opus")))
        };
        state.outputs.insert("A/sung.opus".into(), sung);
        state.save(&dirs.home).unwrap();
        let (mut out, mut report) = (Vec::new(), Vec::new());
        let ok = check(
            &Fake::default(),
            &dirs,
            (false, false),
            &mut out,
            &mut report,
        )
        .unwrap();
        assert!(out.is_empty(), "the problems are the report");
        let text = String::from_utf8(report).unwrap();
        assert!(!ok);
        assert!(
            text.contains("Missing, `sync` writes it again: A/gone.opus"),
            "{text}"
        );
        assert!(text.contains("Changed since muman wrote it"), "{text}");
        assert!(text.contains("A/changed.opus"), "{text}");
        assert!(!text.contains("A/kept.opus"), "{text}");
        assert!(
            text.contains("Not muman's, left alone: A/stray.opus"),
            "{text}"
        );
        assert!(text.contains("Left by an interrupted run"), "{text}");
        assert!(
            text.contains("Lyrics missing, `sync` writes them again: A/sung.opus"),
            "{text}"
        );
    }
}
