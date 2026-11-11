//! `muman check`: the library and the sources against what earlier
//! runs recorded. Reads only, takes no lock; damage is what each
//! source's integrity measure found ([`crate::integrity`]), and
//! `--decode` measures it afresh for every listed source, as files can
//! change on disk unseen, and
//! `--upstream` asks each site whether it still serves each fetched
//! source as its song records, downloading nothing: of a source a site
//! re-encoded or took down, the copy in the store is the one to keep.
//!
//! What it reports without running a tool is [`crate::diagnose`]'s,
//! which `status` counts too.

use std::collections::BTreeMap;
use std::io::Write;

use anyhow::Result;

use crate::diagnose::Said;
use crate::dirs::Dirs;
use crate::held::{Held, Upstream};
use crate::manifest::Manifest;
use crate::parallel;
use crate::runner::{Line, Runner};
use crate::source::SourceKey;
use crate::store::{self, Kind, Store};

/// Print how often each kind of evidence vetting weighs agrees for the
/// library's records, labelled by ID, and the points those counts
/// suggest ([`muman_engine::calibrate`]); nothing is checked or changed.
pub fn calibrate<D: Write>(dirs: &Dirs, data: &mut D) -> Result<bool> {
    let manifest = crate::manifest::Manifest::load(&dirs.home)?;
    let state = crate::state::State::load(&dirs.home)?;
    let c =
        muman_engine::calibrate::calibrate(&manifest, &manifest.settings, &state, &dirs.library)?;
    for line in muman_engine::calibrate::lines(&c) {
        crate::ui::line(data, &line)?;
    }
    Ok(true)
}

/// Say every problem found to `report`, and what it is doing to `out`.
/// Returns whether there was none.
pub fn check<R: Runner, W: Write, D: Write>(
    runner: &R,
    dirs: &Dirs,
    (decode, upstream): (bool, bool),
    out: &mut W,
    report: &mut D,
) -> Result<bool> {
    let crate::diagnose::Diagnosis {
        said,
        manifest,
        state,
        store,
    } = crate::diagnose::diagnose(dirs)?;
    let mut problems = 0_usize;
    let mut problem = |out: &mut D, text: String| {
        problems += 1;
        crate::ui::warning(out, &text)
    };
    for line in said {
        match line {
            Said::Info(line) => crate::ui::info(report, &line)?,
            Said::Warning(line) => crate::ui::warning(report, &line)?,
            Said::Problem(line) => problem(report, line)?,
        }
    }
    let listed = manifest.keys();
    let media: Vec<_> = listed
        .iter()
        .filter_map(|k| store.locate(k))
        .filter(|l| l.kind == Kind::Media)
        .collect();
    if decode {
        if !media.is_empty() {
            crate::ui::info(out, &format!("Decoding {} source(s)", media.len()))?;
        }
        out.flush()?;
        let scratch = crate::atomic::Scratch::new()?;
        let step = crate::progress::step("Decoding", Some(media.len() as u64));
        let numbered: Vec<(usize, &store::Located)> = media.iter().enumerate().collect();
        let measured = parallel::map(&numbered, parallel::builds(), |(n, l)| {
            let _working = step.working(&crate::progress::label(&l.path));
            crate::facts::integrity(runner, l, &scratch.path().join(format!("decode-{n}")))
        });
        for (l, result) in media.iter().zip(measured) {
            match result {
                Ok(m) => {
                    if let Some(damage) = m.as_ref().and_then(crate::integrity::Measure::damage) {
                        problem(
                            report,
                            format!("Damaged audio: {}", damage.describe(&l.key)),
                        )?;
                    }
                }
                Err(e) => problem(report, format!("Does not decode: {}: {e:#}", l.key))?,
            }
        }
    } else {
        let unchecked = media
            .iter()
            .filter(|l| {
                state
                    .facts
                    .get(&l.key)
                    .is_some_and(|f| f.holds_for(&l.rev()) && f.integrity().is_none())
            })
            .count();
        if unchecked > 0 {
            crate::ui::info(
                report,
                &format!(
                    "{unchecked} source(s) not checked for damage yet: `sync` or `status` \
                     measures them, `check --decode` here"
                ),
            )?;
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

/// What the sites listed, each with a digest of its tags.
type Listed = Vec<(crate::info::VideoInfo, Option<String>)>;

/// Ask the sites of `urls` what they serve, by one yt-dlp run that
/// downloads nothing, adding to `listed` and `errors`.
fn ask_sites<R: Runner>(
    runner: &R,
    passed: &crate::download::Passed,
    urls: &[String],
    listed: &mut Listed,
    errors: &mut Vec<String>,
) -> Result<bool> {
    let cmd = crate::download::upstream_command(passed, urls);
    runner.stream(&cmd, &mut |line| match line {
        Line::Out(l) => listed.extend(
            crate::info::parse(l.as_bytes())
                .ok()
                .map(|i| (i, crate::info::tags_digest(l.as_bytes()))),
        ),
        Line::Err(l) => errors.push(l.trim().to_string()),
    })
}

/// What the sites served, with yt-dlp's errors, and each site that
/// reached no server, with why.
struct Asked {
    listed: Listed,
    errors: Vec<String>,
    unreachable: BTreeMap<String, String>,
}

/// The site each of `keys` is of, and the first of its keys.
fn sites<'a>(keys: &[&'a SourceKey]) -> BTreeMap<&'a str, usize> {
    let mut first = BTreeMap::new();
    for (n, key) in keys.iter().enumerate() {
        if let SourceKey::Remote { site, .. } = key {
            first.entry(site.as_str()).or_insert(n);
        }
    }
    first
}

/// Ask the sites what each of `keys`, at `urls`, serves now: one source of
/// each site first, as a site that reaches no server is down, or the
/// network is, and its other sources would each wait out yt-dlp's retries
/// to fail alike, as though taken down; then the rest of the sites it
/// reached.
fn ask_by_site<R: Runner>(
    runner: &R,
    ytdlp: &crate::settings::Ytdlp,
    keys: &[&SourceKey],
    urls: &[String],
) -> Result<Asked> {
    let scratch = crate::atomic::Scratch::new()?;
    let passed = crate::download::Passed::write(ytdlp, scratch.path())?;
    let (mut listed, mut errors): (Listed, Vec<String>) = (Vec::new(), Vec::new());
    let first = sites(keys);
    let probes: Vec<String> = first.values().map(|&n| urls[n].clone()).collect();
    ask_sites(runner, &passed, &probes, &mut listed, &mut errors)?;
    let unreachable: BTreeMap<String, String> = first
        .iter()
        .filter_map(|(&site, &n)| {
            let SourceKey::Remote { id, .. } = keys[n] else {
                return None;
            };
            if listed
                .iter()
                .any(|(i, _)| i.id.as_deref() == Some(id.as_str()))
            {
                return None;
            }
            let named = errors.iter().find(|e| e.contains(&format!("] {id}:")));
            named
                .or_else(|| errors.iter().find(|e| crate::ytdlp_log::unreachable(e)))
                .filter(|e| crate::ytdlp_log::unreachable(e))
                .map(|e| {
                    (
                        site.to_string(),
                        e.trim_start_matches("ERROR: ").to_string(),
                    )
                })
        })
        .collect();
    let site_of = |n: usize| match keys[n] {
        SourceKey::Remote { site, .. } => site.as_str(),
        _ => "",
    };
    let rest: Vec<String> = (0..keys.len())
        .filter(|n| !first.values().any(|f| f == n) && !unreachable.contains_key(site_of(*n)))
        .map(|n| urls[n].clone())
        .collect();
    if !rest.is_empty() && unreachable.len() < first.len() {
        ask_sites(runner, &passed, &rest, &mut listed, &mut errors)?;
    }
    Ok(Asked {
        listed,
        errors,
        unreachable,
    })
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
            "keep the copy in sources/yt-dlp, the only one of what was fetched"
        } else {
            "sources/yt-dlp holds no copy of what was fetched either"
        }
    };
    let asked: Vec<(&SourceKey, &Held, String)> = manifest
        .songs
        .iter()
        .flat_map(|s| &s.held)
        .filter(|(k, h)| matches!(k, SourceKey::Remote { .. }) && h.format.is_some())
        .filter_map(|(k, h)| {
            let url = manifest
                .settings
                .sites
                .fetch_url(k)
                .or_else(|| h.url.clone())?;
            Some((k, h, url))
        })
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
    let keys: Vec<&SourceKey> = asked.iter().map(|(k, _, _)| *k).collect();
    let Asked {
        listed,
        errors,
        unreachable,
    } = ask_by_site(runner, &manifest.settings.ytdlp, &keys, &urls)?;
    if unreachable.len() == sites(&keys).len() {
        let why = unreachable.values().next().cloned().unwrap_or_default();
        return Ok(vec![(
            true,
            format!("No site can be reached, so none was asked what it serves: {why}"),
        )]);
    }
    let mut lines = Vec::new();
    let mut same = 0;
    for (key, held, _) in &asked {
        let SourceKey::Remote { site, id } = key else {
            continue;
        };
        if let Some(why) = unreachable.get(site.as_str()) {
            lines.push((
                true,
                format!("Not asked: {key}: its site cannot be reached: {why}"),
            ));
            continue;
        }
        let info = listed.iter().find(|(i, _)| {
            i.id.as_deref() == Some(id.as_str())
                && (i.key(&manifest.settings.sites).as_ref() == Some(key)
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
            locks: Some(dir.path().to_path_buf()),
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
    fn upstream_with_no_site_reachable_asks_one_and_says_so_once() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("lib"),
            locks: Some(dir.path().to_path_buf()),
        };
        std::fs::create_dir_all(&dirs.home).unwrap();
        let song = |id: &str| {
            format!(
                "[[song]]\nsources = [\"youtube.com:{id}\"]\nheld.\"youtube.com:{id}\" = \
                 {{ audio = \"0123456789abcdef\", format = \"251\", size = 1 }}\n"
            )
        };
        let list = format!(
            "version = 1\n{}{}",
            song("aaaaaaaaaaa"),
            song("bbbbbbbbbbb")
        );
        std::fs::write(dirs.manifest(), list).unwrap();
        let fake = Fake {
            err_lines: vec![
                "ERROR: [youtube] aaaaaaaaaaa: Unable to download API page: \
                 Unable to connect to proxy"
                    .into(),
            ],
            ..Fake::default()
        };
        let (mut out, mut report) = (Vec::new(), Vec::new());
        let ok = check(&fake, &dirs, (false, true), &mut out, &mut report).unwrap();
        let text = crate::ui::plain(&String::from_utf8(report).unwrap());
        assert!(!ok);
        assert!(text.contains("No site can be reached"), "{text}");
        assert!(!text.contains("Not served now"), "{text}");
        let asked = fake.calls();
        assert_eq!(asked.len(), 1, "{asked:?}");
    }

    #[test]
    fn upstream_asks_the_sites_it_reaches_and_names_the_one_it_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("lib"),
            locks: Some(dir.path().to_path_buf()),
        };
        std::fs::create_dir_all(&dirs.home).unwrap();
        let song = |key: &str| {
            format!(
                "[[song]]\nsources = [\"{key}\"]\nheld.\"{key}\" = \
                 {{ audio = \"0123456789abcdef\", format = \"251\", size = 100 }}\n"
            )
        };
        let list = format!(
            "version = 1\n{}{}",
            song("youtube.com:aaaaaaaaaaa"),
            song("archive.org:item00000001")
        );
        std::fs::write(dirs.manifest(), list).unwrap();
        let fake = Fake {
            lines: vec![
                r#"{"id": "aaaaaaaaaaa", "extractor_key": "Youtube", "formats": [{"format_id": "251", "filesize": 100, "acodec": "opus", "vcodec": "none"}]}"#
                    .into(),
            ],
            err_lines: vec![
                "ERROR: [archive.org] item00000001: Unable to download JSON metadata: \
                 Temporary failure in name resolution"
                    .into(),
            ],
            ..Fake::default()
        };
        let (mut out, mut report) = (Vec::new(), Vec::new());
        let ok = check(&fake, &dirs, (false, true), &mut out, &mut report).unwrap();
        let text = crate::ui::plain(&String::from_utf8(report).unwrap());
        assert!(!ok);
        assert!(!text.contains("No site can be reached"), "{text}");
        assert!(
            text.contains("Not asked: archive.org:item00000001: its site cannot be reached"),
            "{text}"
        );
        assert!(!text.contains("youtube.com:aaaaaaaaaaa:"), "{text}");
    }

    #[test]
    fn upstream_names_a_source_whose_site_serves_other_tags() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("lib"),
            locks: Some(dir.path().to_path_buf()),
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
            locks: Some(dir.path().to_path_buf()),
        };
        std::fs::create_dir_all(dirs.library.join("A")).unwrap();
        std::fs::create_dir_all(&dirs.home).unwrap();
        std::fs::write(dirs.library.join("A/kept.opus"), "x").unwrap();
        std::fs::write(dirs.library.join("A/changed.opus"), "x").unwrap();
        std::fs::write(dirs.library.join("A/stray.opus"), "x").unwrap();
        std::fs::write(dirs.library.join("A/notes.txt"), "x").unwrap();
        std::fs::write(dirs.library.join("A/.hidden"), "x").unwrap();
        std::fs::write(dirs.library.join("A/half.opus.part"), "x").unwrap();
        let mut state = crate::state::State::default();
        let written = |stamp: Option<String>| Written {
            sources: Vec::new(),
            sidecars: Vec::new(),
            plan: Some(crate::resolve::Plan {
                folder: None,
                version: crate::resolve::render_version(crate::codec::Codec::Opus),
                format: crate::resolve::Format::Copy {
                    codec: crate::codec::Codec::Opus,
                },
                audio: crate::resolve::AudioRef {
                    key: crate::source::SourceKey::youtube("aaaaaaaaaaa"),
                    rev: "1".into(),
                    index: 1,
                },
                cover: None,
                lyrics: None,
                tags: Vec::new(),
                loudness: None,
                tagging: crate::resolve::TAGGING,
                chapters: Vec::new(),
            }),
            stamp,
            digest: None,
        };
        let kept = store::stamp_text(&dirs.library.join("A/kept.opus"));
        state.outputs.insert("A/kept.opus".into(), written(kept));
        state
            .outputs
            .insert("A/changed.opus".into(), written(Some("1:1".into())));
        state.outputs.insert("A/gone.opus".into(), written(None));
        std::fs::write(dirs.library.join("A/sung.opus"), "x").unwrap();
        let sung = Written {
            sidecars: vec![crate::state::Sidecar::at("A/sung.lrc".into())],
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
        assert!(
            text.contains("Not muman's, left alone: A/notes.txt"),
            "{text}"
        );
        assert!(!text.contains(".hidden"), "{text}");
        assert!(text.contains("Left by an interrupted run"), "{text}");
        assert!(
            text.contains("Missing beside its song, `sync` writes it again: A/sung.lrc"),
            "{text}"
        );

        // While a run is under way, what it writes is its own.
        let _held = crate::atomic::Lock::runs(&dirs.home).unwrap();
        let mut report = Vec::new();
        check(
            &Fake::default(),
            &dirs,
            (false, false),
            &mut Vec::new(),
            &mut report,
        )
        .unwrap();
        let text = String::from_utf8(report).unwrap();
        assert!(text.contains("A muman run is under way"), "{text}");
        assert!(!text.contains("Left by an interrupted run"), "{text}");
    }
}
