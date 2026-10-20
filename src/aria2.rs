//! aria2c fetching a torrent's files into the store: its arguments, what
//! its lines say, and each finished file placed at its key's place.
//!
//! aria2c runs with `--no-conf`, so a personal `aria2.conf` cannot change
//! what is fetched or where; the song list's `[torrent]` is how options
//! reach it, `args` last so they win. Its DHT routing table is kept with
//! the unfinished fetches rather than in the user's own aria2 folder.
//!
//! A magnet link names a torrent without its metainfo, which aria2c asks
//! its peers for first (`--bt-metadata-only`), saving it beside the
//! unfinished fetches as `<v1 hash>.torrent`; muman then reads and keeps
//! it as a `.torrent` file given to `add` is kept.
//!
//! A fetch selects the files muman takes ([`crate::torrent::selected`])
//! by their place in the torrent, and names each `files/<place>.<ext>`
//! in a folder of the torrent's hash under `partial/torrent`
//! (`--index-out`), so aria2c never writes a path the torrent names,
//! which may not suit this system, and a fetch interrupted resumes at the
//! same paths, its pieces checked again (`--check-integrity`). Pieces
//! overlap the files next to a selected one, which aria2c writes in part
//! and removes once done. Only when the run finishes whole are the files
//! placed: a file aria2c has not finished may have holes where pieces
//! came out of order, whatever its size.
//!
//! A listed file gone from the store is fetched again by `sync`
//! ([`missing`]), with the lyrics and pictures it brings: from the
//! torrent's metainfo where the store keeps it, else from a magnet link
//! of its hash and the trackers the song list records, its metainfo
//! asked of its peers and kept again. Files of the torrent no song lists
//! are not fetched again, so what `purge` deleted stays deleted. One that
//! failed lately waits, as a missing video does.
//!
//! A fetch with no byte arriving for `[torrent] stall` stops, aria2c
//! reporting it unfinished, as a torrent no peer seeds leaves it; it is
//! a failure a later sync tries again. aria2c reports each tracker it
//! cannot reach as an error, as a torrent's dead trackers make it while
//! others serve; these are said only when the fetch fails.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::manifest::Manifest;
use crate::runner::{Line, Runner};
use crate::settings::Torrent;
use crate::source::SourceKey;
use crate::state::{Failure, Step};
use crate::store::Store;
use crate::torrent::{File, Magnet, Metainfo};

/// Where aria2c runs and how.
#[derive(Debug, Clone, Copy)]
pub struct Places<'a> {
    /// The torrents' folder in the store.
    pub store: &'a Path,
    /// Where unfinished fetches wait, a folder for each torrent.
    pub partial: &'a Path,
    pub options: &'a Torrent,
}

/// What one aria2c run said beyond its exit.
#[derive(Debug, Default)]
struct Said {
    /// Whether a download was left unfinished, as a stall leaves it.
    unfinished: bool,
    /// Whether a download ended in an error.
    failed: bool,
    errors: Vec<String>,
}

/// The arguments every aria2c run takes, writing into `dir`.
fn common(places: &Places<'_>, dir: &Path) -> Vec<OsString> {
    let flag = |name: &str, value: &Path| {
        let mut arg = OsString::from(format!("--{name}="));
        arg.push(value);
        arg
    };
    let mut cmd: Vec<OsString> = vec![
        "aria2c".into(),
        "--no-conf=true".into(),
        flag("dir", dir),
        "--console-log-level=warn".into(),
        "--summary-interval=1".into(),
        "--show-console-readout=false".into(),
        "--enable-color=false".into(),
        "--download-result=full".into(),
        "--auto-file-renaming=false".into(),
        "--allow-overwrite=true".into(),
        "--file-allocation=none".into(),
        format!("--bt-stop-timeout={}", places.options.stall.as_secs()).into(),
        format!(
            "--seed-time={}",
            places.options.seed_time.as_secs_f64() / 60.0
        )
        .into(),
        flag("dht-file-path", &places.partial.join("dht.dat")),
        flag("dht-file-path6", &places.partial.join("dht6.dat")),
    ];
    cmd.extend(places.options.args.iter().map(OsString::from));
    cmd
}

/// The run that asks a magnet link's peers for its torrent's metainfo.
#[must_use]
pub fn metainfo_command(places: &Places<'_>, link: &str) -> Vec<OsString> {
    let mut cmd = common(places, places.partial);
    cmd.push("--bt-metadata-only=true".into());
    cmd.push("--bt-save-metadata=true".into());
    cmd.push(link.into());
    cmd
}

/// Where a selected file is written while it is fetched, in `dir`.
fn index_path(file: &File) -> PathBuf {
    let name = match file.path.extension() {
        Some(ext) => format!("{}.{}", file.index, ext.to_string_lossy()),
        None => file.index.to_string(),
    };
    Path::new("files").join(name)
}

/// The run that fetches `files` of the torrent `metainfo` describes into
/// `dir`.
#[must_use]
pub fn fetch_command(
    places: &Places<'_>,
    metainfo: &Path,
    dir: &Path,
    files: &[&File],
) -> Vec<OsString> {
    let mut cmd = common(places, dir);
    cmd.extend([
        "--continue=true".into(),
        "--check-integrity=true".into(),
        "--bt-remove-unselected-file=true".into(),
    ]);
    let select: Vec<String> = files.iter().map(|f| f.index.to_string()).collect();
    cmd.push(format!("--select-file={}", select.join(",")).into());
    for file in files {
        let mut arg = OsString::from(format!("--index-out={}=", file.index));
        arg.push(index_path(file));
        cmd.push(arg);
    }
    cmd.push(metainfo.into());
    cmd
}

/// Run `cmd`, its progress noted on `step`: whether it succeeded, and
/// what it said.
fn stream<R: Runner>(
    runner: &R,
    cmd: &[OsString],
    step: &crate::progress::Step,
) -> Result<(bool, Said)> {
    let mut said = Said::default();
    let mut results = false;
    let ok = runner.stream(cmd, &mut |line| {
        let text = match line {
            Line::Out(t) | Line::Err(t) => t.trim(),
        };
        if text.starts_with("Download Results:") {
            results = true;
        } else if results && text.contains('|') {
            match text.split('|').nth(1).map(str::trim) {
                Some("INPR") => said.unfinished = true,
                Some("ERR") => said.failed = true,
                _ => {}
            }
        } else if let Some(readout) = text.strip_prefix("[#") {
            if let Some(progress) = readout.split_once(' ').map(|(_, p)| p) {
                step.note(&format!("aria2c: {}", progress.trim_end_matches(']')));
            }
        } else if let Some(error) = error_of(text)
            && !said.errors.contains(&error)
        {
            said.errors.push(error);
        }
    })?;
    Ok((ok && !said.unfinished && !said.failed, said))
}

/// What an error or warning line says, without aria2c's time and
/// connection number.
fn error_of(line: &str) -> Option<String> {
    let at = line.find("[ERROR]").or_else(|| line.find("[WARN]"))?;
    let rest = &line[at..];
    let rest = rest.split_once(']').map_or(rest, |(_, r)| r).trim();
    let rest = rest
        .strip_prefix("CUID#")
        .and_then(|r| r.split_once(" - "))
        .map_or(rest, |(_, r)| r);
    Some(rest.trim().to_string())
}

/// Why a run that did not succeed failed.
fn failure(places: &Places<'_>, said: &Said) -> String {
    if said.unfinished {
        return format!(
            "no peer sent anything for {}; a later sync tries again",
            crate::units::Time(places.options.stall.as_millis() as i64).exact()
        );
    }
    said.errors
        .last()
        .map_or_else(|| "aria2c failed".to_string(), |e| format!("aria2c: {e}"))
}

/// The metainfo of the torrent `magnet` names, from its peers.
pub fn metainfo_of<R: Runner, W: Write>(
    runner: &R,
    out: &mut W,
    places: &Places<'_>,
    magnet: &Magnet,
) -> Result<Vec<u8>> {
    let Some(v1) = &magnet.v1 else {
        bail!(
            "{} names a v2 torrent alone, and no fetcher here reads v2 torrents",
            magnet.name.as_deref().unwrap_or(magnet.hash())
        );
    };
    std::fs::create_dir_all(places.partial)
        .with_context(|| format!("creating {}", places.partial.display()))?;
    let step = crate::progress::step("Fetching metainfo", None);
    let (ok, said) = stream(runner, &metainfo_command(places, &magnet.link()), &step)?;
    let saved = places.partial.join(format!("{v1}.torrent"));
    if !ok || !saved.exists() {
        for e in &said.errors {
            crate::ui::warning(out, &format!("aria2c: {e}"))?;
        }
        bail!(
            "the metainfo of {} did not arrive: {}",
            magnet.name.as_deref().unwrap_or(v1),
            failure(places, &said)
        );
    }
    let bytes = std::fs::read(&saved).with_context(|| format!("reading {}", saved.display()))?;
    let _ = std::fs::remove_file(&saved);
    Ok(bytes)
}

/// What a fetch of a torrent's files brought.
#[derive(Debug, Default)]
pub struct Fetched {
    /// The key of each file placed in the store.
    pub arrived: Vec<SourceKey>,
    /// Why the fetch failed, where it did; nothing is placed then.
    pub failure: Option<String>,
}

/// Fetch `files` of `meta`, whose metainfo the store keeps, and place each
/// at its key's place.
pub fn fetch<R: Runner, W: Write>(
    runner: &R,
    out: &mut W,
    places: &Places<'_>,
    meta: &Metainfo,
    files: &[&File],
) -> Result<Fetched> {
    ensure!(
        meta.fetchable(),
        "{} is a v2 torrent alone, and no fetcher here reads v2 torrents",
        meta.name
    );
    if files.is_empty() {
        return Ok(Fetched::default());
    }
    let hash = meta.hash();
    let metainfo = crate::store::metainfo_path(places.store, hash);
    let dir = places.partial.join(hash);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let step = crate::progress::step("Fetching", None);
    let (ok, said) = stream(
        runner,
        &fetch_command(places, &metainfo, &dir, files),
        &step,
    )?;
    if !ok {
        for e in &said.errors {
            crate::ui::warning(out, &format!("aria2c: {e}"))?;
        }
        return Ok(Fetched {
            arrived: Vec::new(),
            failure: Some(failure(places, &said)),
        });
    }
    let mut arrived = Vec::new();
    for file in files {
        let from = dir.join(index_path(file));
        let SourceKey::Torrent(key) = meta.key(file) else {
            continue;
        };
        if !from.is_file() {
            crate::ui::warning(
                out,
                &format!("aria2c finished without {}", file.path.display()),
            )?;
            continue;
        }
        match crate::store::place_torrent(places.store, &key, &from) {
            Ok(_) => arrived.push(SourceKey::Torrent(key)),
            Err(e) => crate::ui::warning(out, &format!("{e:#}"))?,
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(Fetched {
        arrived,
        failure: None,
    })
}

/// Keep the metainfo `bytes` of the torrent of `hash` in the store's
/// torrents' folder, `root`, unless it is kept.
pub fn keep_metainfo(root: &Path, hash: &str, bytes: &[u8]) -> Result<()> {
    std::fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
    if !crate::store::metainfo_path(root, hash).exists() {
        crate::atomic::write(
            root,
            &crate::atomic::Name::new(format!("{hash}.torrent"))?,
            bytes,
        )?;
    }
    Ok(())
}

/// Fetch again every listed torrent's file the store lacks, one that
/// failed lately waiting unless `retry`: each that did not arrive, with
/// why.
pub fn missing<R: Runner, W: Write>(
    runner: &R,
    out: &mut W,
    places: &Places<'_>,
    store: &Store,
    manifest: &Manifest,
    failures: &std::collections::BTreeMap<SourceKey, Failure>,
    retry: bool,
) -> Result<Vec<(SourceKey, String)>> {
    let now = crate::state::now_secs();
    let gone: Vec<SourceKey> = manifest
        .keys()
        .into_iter()
        .filter(|k| matches!(k, SourceKey::Torrent(_)) && !store.has(k))
        .collect();
    let (gone, waiting): (Vec<SourceKey>, Vec<SourceKey>) = gone.into_iter().partition(|k| {
        retry
            || failures
                .get(k)
                .filter(|f| f.step == Step::Fetch)
                .is_none_or(|f| f.due(None, now))
    });
    if !waiting.is_empty() {
        crate::ui::warning(
            out,
            &format!(
                "Not fetching {} missing torrent file(s) that failed lately; `sync --retry` \
                 tries now",
                waiting.len()
            ),
        )?;
    }
    let mut by_hash: std::collections::BTreeMap<&str, Vec<&SourceKey>> =
        std::collections::BTreeMap::new();
    for key in &gone {
        if let SourceKey::Torrent(t) = key {
            by_hash.entry(t.hash()).or_default().push(key);
        }
    }
    let mut failed = Vec::new();
    for (hash, keys) in by_hash {
        crate::ui::info(
            out,
            &format!("Fetching {} missing torrent file(s) again", keys.len()),
        )?;
        let why = match again(runner, out, places, manifest, hash, &keys) {
            Ok(arrived) => {
                for key in keys.iter().filter(|k| !arrived.contains(k)) {
                    failed.push(((*key).clone(), "aria2c did not finish it".to_string()));
                }
                continue;
            }
            Err(e) => format!("{e:#}"),
        };
        for key in keys {
            crate::ui::error(out, &format!("Could not fetch {key} again: {why}"))?;
            failed.push((key.clone(), why.clone()));
        }
    }
    Ok(failed)
}

/// Fetch the files `keys` name of the torrent of `hash` again, with the
/// lyrics and pictures a listed song file of it brings: the keys that
/// arrived.
fn again<R: Runner, W: Write>(
    runner: &R,
    out: &mut W,
    places: &Places<'_>,
    manifest: &Manifest,
    hash: &str,
    keys: &[&SourceKey],
) -> Result<Vec<SourceKey>> {
    let kept = crate::store::metainfo_path(places.store, hash);
    let bytes = if kept.is_file() {
        std::fs::read(&kept).with_context(|| format!("reading {}", kept.display()))?
    } else {
        let recorded = manifest.torrents.get(hash).cloned().unwrap_or_default();
        let magnet = Magnet {
            v1: (hash.len() == crate::torrent::V1_HEX).then(|| hash.to_string()),
            v2: recorded
                .v2
                .clone()
                .or_else(|| (hash.len() == crate::torrent::V2_HEX).then(|| hash.to_string())),
            name: recorded.name.clone(),
            trackers: recorded.trackers,
        };
        let bytes = metainfo_of(runner, out, places, &magnet)
            .context("its metainfo is gone from the store; give its .torrent to `add` again")?;
        keep_metainfo(places.store, hash, &bytes)?;
        bytes
    };
    let meta = Metainfo::read(&bytes)?;
    ensure!(
        meta.hash() == hash,
        "{} is the metainfo of another torrent",
        kept.display()
    );
    let listed: Vec<&Path> = manifest
        .keys()
        .into_iter()
        .filter_map(|k| match k {
            SourceKey::Torrent(t) if t.hash() == hash => Some(
                meta.files
                    .iter()
                    .find(|f| crate::relpath::normalized(&f.path) == t.path())?
                    .path
                    .as_path(),
            ),
            _ => None,
        })
        .collect();
    let wanted: Vec<&File> = crate::torrent::selected(&meta)
        .into_iter()
        .filter(|f| {
            let key = meta.key(f);
            if keys.contains(&&key) {
                return true;
            }
            let SourceKey::Torrent(t) = &key else {
                return false;
            };
            crate::store::kind_of(&f.path) != Some(crate::store::Kind::Media)
                && listed.iter().any(|s| f.path.parent() == s.parent())
                && !crate::store::torrent_path(places.store, t).is_file()
        })
        .collect();
    let fetched = fetch(runner, out, places, &meta, &wanted)?;
    if let Some(failure) = fetched.failure {
        bail!(failure);
    }
    Ok(fetched.arrived)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Torrent {
        Torrent {
            args: vec!["--listen-port=51413".into()],
            ..Torrent::default()
        }
    }

    fn text(cmd: &[OsString]) -> Vec<String> {
        cmd.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_fetch_selects_its_files_and_names_each_by_its_place() {
        let options = options();
        let places = Places {
            store: Path::new("store"),
            partial: Path::new("partial"),
            options: &options,
        };
        let files = [
            File {
                path: PathBuf::from("01 Paper Comets.flac"),
                length: 3,
                root: None,
                index: 1,
            },
            File {
                path: ["CD 2", "cover"].iter().collect(),
                length: 3,
                root: None,
                index: 4,
            },
        ];
        let cmd = text(&fetch_command(
            &places,
            Path::new("store/h.torrent"),
            Path::new("partial/h"),
            &files.iter().collect::<Vec<_>>(),
        ));
        assert_eq!(cmd[0], "aria2c");
        assert!(cmd.contains(&"--no-conf=true".to_string()));
        assert!(cmd.contains(&"--select-file=1,4".to_string()));
        assert!(cmd.contains(&"--seed-time=0".to_string()));
        assert!(cmd.contains(&"--bt-stop-timeout=600".to_string()));
        let out = |i: &str, p: &[&str]| {
            format!(
                "--index-out={i}={}",
                p.iter().collect::<PathBuf>().display()
            )
        };
        assert!(cmd.contains(&out("1", &["files", "1.flac"])));
        assert!(cmd.contains(&out("4", &["files", "4"])));
        let args = cmd.iter().position(|a| a == "--listen-port=51413").unwrap();
        assert!(args > cmd.iter().position(|a| a == "--no-conf=true").unwrap());
        assert_eq!(cmd.last().unwrap(), "store/h.torrent");
    }

    #[test]
    fn a_stall_and_errors_are_read_from_its_lines() {
        let lines = [
            "10/20 10:50:27 [ERROR] CUID#12 - Download aborted. URI=http://tracker.example/a",
            "[#552677 1.2MiB/40MiB(3%) CN:4 SD:2 DL:300KiB ETA:2m]",
            "Download Results:",
            "gid   |stat|avg speed  |  %|path/URI",
            "======+====+===========+===+=====",
            "6fcf70|INPR|       0B/s|  3|partial/h/files/1.flac",
        ];
        let runner = crate::testing::Fake {
            lines: lines.iter().map(ToString::to_string).collect(),
            on_stream: Some(Box::new(|_: &[String]| false)),
            ..crate::testing::Fake::default()
        };
        let step = crate::progress::step("Fetching", None);
        let (ok, said) = stream(&runner, &["aria2c".into()], &step).unwrap();
        assert!(!ok);
        assert!(said.unfinished);
        assert_eq!(
            said.errors,
            ["Download aborted. URI=http://tracker.example/a"]
        );
        let options = options();
        let places = Places {
            store: Path::new("s"),
            partial: Path::new("p"),
            options: &options,
        };
        assert!(failure(&places, &said).starts_with("no peer sent anything for 10 min"));
    }
}
