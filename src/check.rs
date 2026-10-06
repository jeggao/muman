//! `muman check`: the library and the sources against what earlier
//! runs recorded. Reads only, takes no lock; `--decode` decodes every
//! listed source in full, where a truncated download fails.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::codec::Codec;
use crate::dirs::Dirs;
use crate::manifest::Manifest;
use crate::parallel;
use crate::runner::Runner;
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

/// Every file below `dir`, hidden ones included.
fn files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for path in entries.filter_map(|e| Some(e.ok()?.path())) {
        if path.is_dir() {
            found.extend(files(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// Say every problem found to `report`, and what it is doing to `out`.
/// Returns whether there was none.
#[allow(clippy::too_many_lines)]
pub fn check<R: Runner, W: Write, D: Write>(
    runner: &R,
    dirs: &Dirs,
    decode: bool,
    out: &mut W,
    report: &mut D,
) -> Result<bool> {
    let manifest = Manifest::load(&dirs.home)?;
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
    for file in files(library) {
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
            let fetchable = key.url().is_some() || crate::provider::is_kept(key);
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
    if problems == 0 {
        crate::ui::success(report, "No problems found")?;
    }
    Ok(problems == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Written;
    use crate::testing::Fake;

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
        let ok = check(&Fake::default(), &dirs, false, &mut out, &mut report).unwrap();
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
