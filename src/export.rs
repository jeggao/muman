//! `muman export`: the song list and the library, once, in one zip:
//! `songs.toml` at its root and every song muman wrote under `library/`,
//! at its path in the library, with its lyrics.
//!
//! The library is exported as the last run left it, from what the state
//! records, so a song not written yet is not in it; `sync` first brings it
//! in step. Entries are stored, not compressed: audio does not compress.
//! The library and the state are only read, under the home's lock.
//!
//! With a size, songs are fitted by [`crate::fit`]. Each starts as its
//! library file, whose size is known, and may be encoded again from its
//! sources at the lower bitrates of `[audio] lossy`. A lower rung is
//! estimated as its bitrate times the song's length, plus what the
//! library file holds besides audio, its cover and embedded lyrics, and
//! `CONTAINER` for the container. The songs chosen are encoded into a
//! scratch folder beside the zip, and their real sizes replace the
//! estimates. A song's real size corrects its other estimates by as much
//! as it missed, and the songs not encoded yet by how much all missed on
//! average: a VBR encoder spends more on some music than its bitrate, a
//! pure tone half again. While the total is still over, the songs are
//! fitted again, at most `PASSES` times. A song changed since muman wrote it, or whose
//! sources are gone, can only be copied. Every entry is counted with what
//! the zip spends on it, `ENTRY` bytes and its name twice.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use lofty::file::TaggedFileExt;
use lofty::tag::ItemKey;
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::atomic::Lock;
use crate::dirs::{self, Dirs};
use crate::fit::{self, Item, Rung, Source};
use crate::manifest::Manifest;
use crate::parallel;
use crate::relpath;
use crate::render;
use crate::resolve::{Format, Plan};
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::State;
use crate::store::{Located, Store};

/// Where the songs go inside the zip.
const LIBRARY: &str = "library";
/// What a zip spends on one entry besides its name and data: a local
/// header of 30 bytes, a central one of 46, and room for their ZIP64
/// fields.
const ENTRY: u64 = 30 + 46 + 64;
/// What a zip spends once, at its end, ZIP64 records included.
const END: u64 = 22 + 56 + 20;
/// What a container adds to an encoded stream, as a share of it: Ogg's
/// page headers take about 1%, MP4's index less.
const CONTAINER: f64 = 0.015;
/// Bytes a file holds besides its stream, its cover and lyrics: headers
/// and tags.
const HEADERS: u64 = 4 * 1024;
/// What encoding a song again costs, as loss per minute of it, so a song
/// moved at all moves far rather than many a little.
const CHURN_PER_MINUTE: f64 = 0.25;
/// How many times songs are fitted again once real sizes are known.
const PASSES: usize = 4;
/// The most an encoding's real size is taken to miss its estimate by,
/// either way, when the song's other estimates are corrected by it.
const MISS: f64 = 3.0;

/// One song the library holds, as it is exported.
#[derive(Debug)]
struct Song {
    /// Its path in the library.
    path: PathBuf,
    lyrics: Option<PathBuf>,
    plan: Plan,
    /// What it may be encoded again from; none when it can only be copied.
    sources: Option<BTreeMap<SourceKey, Located>>,
}

/// A file to put in the zip: where it is read from and its name there.
#[derive(Debug, Clone)]
struct Entry {
    from: PathBuf,
    name: String,
}

/// What the zip spends on an entry of `bytes` named `name`.
fn entry_size(name: &str, bytes: u64) -> u64 {
    bytes + ENTRY + 2 * name.len() as u64
}

fn name_in_zip(path: &Path) -> String {
    format!("{LIBRARY}/{}", relpath::to_portable(path))
}

fn size_of(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |m| m.len())
}

/// What a song's entries take: its audio and lyrics, each as `files` holds
/// them, named by `path`.
fn entries_size(files: &[Entry]) -> u64 {
    files
        .iter()
        .map(|e| entry_size(&e.name, size_of(&e.from)))
        .sum()
}

/// The bytes a library file holds besides its audio: its pictures and
/// embedded lyrics, as `lofty` reads them, and its headers.
fn extras(path: &Path) -> u64 {
    let Ok(tagged) = lofty::read_from_path(path) else {
        return 64 * 1024 + HEADERS;
    };
    let held: usize = tagged
        .tags()
        .iter()
        .map(|tag| {
            let pictures: usize = tag.pictures().iter().map(|p| p.data().len()).sum();
            let lyrics = [ItemKey::Lyrics, ItemKey::UnsyncLyrics]
                .iter()
                .filter_map(|k| tag.get_string(*k))
                .map(str::len)
                .max()
                .unwrap_or(0);
            pictures + lyrics
        })
        .max()
        .unwrap_or(0);
    held as u64 + HEADERS
}

/// Export the song list and the library in `dirs` into the zip at
/// `output`, or `muman.zip` in it when it is a folder, fitting it into
/// `max_size` bytes when given. Returns whether every song went in as
/// chosen.
pub fn export<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    output: &Path,
    max_size: Option<u64>,
    out: &mut W,
) -> Result<bool> {
    let _lock = Lock::folder(&dirs.home)?;
    let list = dirs.home.join(dirs::MANIFEST);
    if !list.is_file() {
        bail!("{} holds no song list to export", dirs.home.display());
    }
    let manifest = Manifest::load(&dirs.home)?;
    let state = State::load(&dirs.home)?;
    let store = Store::scan(dirs)?;
    let output = if output.is_dir() {
        output.join("muman.zip")
    } else {
        output.to_path_buf()
    };
    let folder = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;

    let songs = songs(dirs, &state, &store, out)?;
    let mut files: Vec<Vec<Entry>> = songs
        .iter()
        .map(|s| library_entries(&dirs.library, s))
        .collect();
    let list_entry = Entry {
        from: list,
        name: dirs::MANIFEST.to_string(),
    };
    let mut ok = true;
    let mut encoded = 0_usize;
    let scratch = tempfile::tempdir_in(folder).context("creating a scratch folder")?;
    if let Some(max) = max_size {
        let fixed = entries_size(std::slice::from_ref(&list_entry)) + END;
        let budget = max
            .checked_sub(fixed)
            .ok_or_else(|| anyhow!("{} cannot hold even the song list", crate::ui::bytes(max)))?;
        let fitted = fit_into(
            runner,
            &manifest,
            &state,
            &songs,
            &files,
            budget,
            scratch.path(),
            out,
        )?;
        ok &= fitted.ok;
        for (n, chosen) in fitted.entries {
            files[n] = chosen;
            encoded += 1;
        }
    }

    let part = {
        let mut name = output.clone().into_os_string();
        name.push(".part");
        PathBuf::from(name)
    };
    let written = write_zip(
        &part,
        std::iter::once(&list_entry).chain(files.iter().flatten()),
    );
    if let Err(e) = written {
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    crate::atomic::rename(&part, &output)
        .with_context(|| format!("renaming {} to {}", part.display(), output.display()))?;
    let size = size_of(&output);
    crate::ui::success(
        out,
        &format!(
            "Exported the song list and {} song(s), {}, to {}",
            songs.len(),
            crate::ui::bytes(size),
            output.display()
        ),
    )?;
    if let Some(max) = max_size {
        if encoded > 0 {
            crate::ui::info(
                out,
                &format!(
                    "{encoded} song(s) encoded at a lower bitrate to fit {}",
                    crate::ui::bytes(max)
                ),
            )?;
        }
        if size > max {
            crate::ui::error(
                out,
                &format!("The zip came out over {}", crate::ui::bytes(max)),
            )?;
            ok = false;
        }
    }
    Ok(ok)
}

/// The songs the library holds as the state records them, in path order,
/// each with its sources when it may be encoded again.
fn songs<W: Write>(dirs: &Dirs, state: &State, store: &Store, out: &mut W) -> Result<Vec<Song>> {
    let mut songs = Vec::new();
    let mut missing = 0_usize;
    for (path, written) in &state.outputs {
        let Some(plan) = &written.plan else {
            continue;
        };
        if size_of(&dirs.library.join(path)) == 0 {
            missing += 1;
            continue;
        }
        let changed = crate::reconcile::changed_since_written(&dirs.library, &state.outputs, path);
        let keys = std::iter::once(&plan.audio.key)
            .chain(plan.cover.as_ref().map(|c| &c.key))
            .chain(plan.lyrics.as_ref().map(|l| &l.key));
        let located: Option<BTreeMap<SourceKey, Located>> =
            keys.map(|k| Some((k.clone(), store.locate(k)?))).collect();
        songs.push(Song {
            path: path.clone(),
            lyrics: written
                .lyrics
                .clone()
                .filter(|l| dirs.library.join(l).is_file()),
            plan: plan.clone(),
            sources: located.filter(|_| !changed),
        });
    }
    if missing > 0 {
        crate::ui::warning(
            out,
            &format!(
                "{missing} song(s) muman wrote are missing from the library and left out; `muman sync` writes them again"
            ),
        )?;
    }
    Ok(songs)
}

fn library_entries(library: &Path, song: &Song) -> Vec<Entry> {
    std::iter::once(&song.path)
        .chain(&song.lyrics)
        .map(|p| Entry {
            from: library.join(p),
            name: name_in_zip(p),
        })
        .collect()
}

/// The songs fitting chose to encode again, by their place, with their
/// entries, and whether every encoding asked for succeeded.
struct Fitted {
    entries: Vec<(usize, Vec<Entry>)>,
    ok: bool,
}

/// Fit the songs into `budget` bytes, encoding those chosen into
/// `scratch`, and fitting again on their real sizes while over.
#[allow(clippy::too_many_arguments)]
fn fit_into<R: Runner, W: Write>(
    runner: &R,
    manifest: &Manifest,
    state: &State,
    songs: &[Song],
    files: &[Vec<Entry>],
    budget: u64,
    scratch: &Path,
    out: &mut W,
) -> Result<Fitted> {
    let mut items: Vec<Item> = songs
        .iter()
        .zip(files)
        .map(|(song, entries)| item(manifest, state, song, entries))
        .collect();
    let estimates: Vec<Vec<u64>> = items
        .iter()
        .map(|i| i.rungs.iter().map(|r| r.bytes).collect())
        .collect();
    let mut ratios: Vec<Option<f64>> = vec![None; items.len()];
    let mut made: BTreeMap<(usize, usize), Vec<Entry>> = BTreeMap::new();
    let mut ok = true;
    for pass in 0..PASSES {
        let fit = fit::allocate(&items, &fit::Policy::exact(budget)).map_err(|floor| {
            anyhow!(
                "{} cannot hold the export: at the lowest bitrates it takes {}",
                crate::ui::bytes(budget),
                crate::ui::bytes(floor)
            )
        })?;
        let due: Vec<(usize, usize)> = fit
            .choice
            .iter()
            .enumerate()
            .filter(|(n, c)| **c > 0 && !made.contains_key(&(*n, **c)))
            .map(|(n, c)| (n, *c))
            .collect();
        if !due.is_empty() {
            crate::ui::info(
                out,
                &format!(
                    "Encoding {} song(s) at a lower bitrate to fit{}",
                    due.len(),
                    if pass > 0 { ", again" } else { "" }
                ),
            )?;
        }
        let step = crate::progress::step("Encoding", Some(due.len() as u64));
        let rendered = parallel::map(&due, parallel::builds(), |(n, c)| {
            let _working = step.working(&crate::progress::label(&songs[*n].path));
            encode(
                runner,
                &songs[*n],
                items[*n].rungs[*c].format,
                &scratch.join(format!("{n}-{c}")),
            )
        });
        for ((n, c), result) in due.into_iter().zip(rendered) {
            let first = items[n].rungs[0].bytes;
            match result {
                Ok(entries) => {
                    let real = entries_size(&entries);
                    #[allow(clippy::cast_precision_loss)]
                    let ratio = real as f64 / estimates[n][c].max(1) as f64;
                    ratios[n] = Some(ratio.clamp(1.0 / MISS, MISS));
                    items[n].rungs[c].bytes = real;
                    made.insert((n, c), entries);
                }
                Err(e) => {
                    ok = false;
                    crate::ui::warning(
                        out,
                        &format!(
                            "Could not encode {}: {e:#}; it is copied as it is",
                            relpath::show(&songs[n].path)
                        ),
                    )?;
                    // Saving nothing, it is never chosen again.
                    items[n].rungs[c].bytes = first;
                }
            }
        }
        correct(&mut items, &estimates, &ratios, &made);
        let total: u64 = fit
            .choice
            .iter()
            .enumerate()
            .map(|(n, c)| items[n].rungs[*c].bytes)
            .sum();
        let done = fit
            .choice
            .iter()
            .enumerate()
            .all(|(n, c)| *c == 0 || made.contains_key(&(n, *c)));
        if total <= budget && done {
            let entries = fit
                .choice
                .iter()
                .enumerate()
                .filter(|(_, c)| **c > 0)
                .filter_map(|(n, c)| Some((n, made.remove(&(n, *c))?)))
                .collect();
            return Ok(Fitted { entries, ok });
        }
    }
    bail!(
        "the songs could not be fitted into {} in {PASSES} tries; try a little more room",
        crate::ui::bytes(budget)
    )
}

/// Scale each estimate not yet replaced by a real size by how far the
/// song's encodings missed theirs, or, for a song not encoded yet, by how
/// far all did on average, by their logarithms.
fn correct(
    items: &mut [Item],
    estimates: &[Vec<u64>],
    ratios: &[Option<f64>],
    made: &BTreeMap<(usize, usize), Vec<Entry>>,
) {
    let seen: Vec<f64> = ratios.iter().flatten().map(|r| r.ln()).collect();
    if seen.is_empty() {
        return;
    }
    #[allow(clippy::cast_precision_loss)]
    let mean = (seen.iter().sum::<f64>() / seen.len() as f64).exp();
    for (n, item) in items.iter_mut().enumerate() {
        let ratio = ratios[n].unwrap_or(mean);
        for (c, rung) in item.rungs.iter_mut().enumerate().skip(1) {
            if !made.contains_key(&(n, c)) && rung.bytes != estimates[n][0] {
                #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                #[allow(clippy::cast_sign_loss)]
                let scaled = (estimates[n][c] as f64 * ratio) as u64;
                rung.bytes = scaled;
            }
        }
    }
}

/// One song's ladder: its library file as it is, then each lower rung it
/// may be encoded at, estimated.
fn item(manifest: &Manifest, state: &State, song: &Song, entries: &[Entry]) -> Item {
    let first = entries_size(entries);
    let mut item = Item {
        key: relpath::to_portable(&song.path),
        rungs: vec![Rung {
            format: song.plan.format,
            bytes: first,
            loss: 0.0,
            sigma: 0.0,
        }],
        churn: 0.0,
    };
    let facts = state.facts.get(&song.plan.audio.key);
    let (Some(_), Some(seconds), Some(audio)) = (
        &song.sources,
        facts.and_then(|f| f.duration),
        facts.and_then(|f| f.audio.as_ref()),
    ) else {
        return item;
    };
    let source = Source {
        lossless: audio.is_lossless(),
        bandwidth_hz: audio.quality.map(|q| q.bandwidth_hz),
        seconds,
    };
    item.churn = CHURN_PER_MINUTE * seconds / 60.0;
    let besides = extras(&entries[0].from)
        + entries[1..]
            .iter()
            .map(|e| entry_size(&e.name, size_of(&e.from)))
            .sum::<u64>()
        + entry_size(&entries[0].name, 0);
    for format in fit::lower(song.plan.format, audio.channels, &manifest.settings.audio) {
        let Format::Encode {
            kbps: Some(kbps), ..
        } = format
        else {
            continue;
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let stream = (f64::from(kbps) * 125.0 * seconds * (1.0 + CONTAINER)) as u64;
        let bytes = stream + besides;
        if bytes.saturating_mul(10) <= first.saturating_mul(9) {
            item.rungs.push(Rung {
                format,
                bytes,
                loss: fit::loss(&source, song.plan.format, format),
                sigma: 0.0,
            });
        }
    }
    item
}

/// Encode one song as `format` into `scratch`, returning its entries.
fn encode<R: Runner>(
    runner: &R,
    song: &Song,
    format: Format,
    scratch: &Path,
) -> Result<Vec<Entry>> {
    let sources = song
        .sources
        .as_ref()
        .ok_or_else(|| anyhow!("its sources are gone"))?;
    let plan = Plan {
        format,
        ..song.plan.clone()
    };
    let library = scratch.join("library");
    let stem = song.path.with_extension("");
    let rendered = render::render(
        runner,
        &render::Job {
            plan: &plan,
            stem: &stem,
            library: &library,
            sources,
            scratch: &scratch.join("work"),
        },
    )?;
    Ok(std::iter::once(&rendered.audio)
        .chain(&rendered.lyrics)
        .map(|p| Entry {
            from: library.join(p),
            name: name_in_zip(p),
        })
        .collect())
}

/// Write `entries` into a new zip at `path`, each stored as it is.
fn write_zip<'a>(path: &Path, entries: impl Iterator<Item = &'a Entry>) -> Result<()> {
    let entries: Vec<&Entry> = entries.collect();
    let step = crate::progress::step("Writing the zip", Some(entries.len() as u64));
    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut zip = ZipWriter::new(file);
    for entry in entries {
        let _working = step.working(&entry.name);
        let mut from =
            File::open(&entry.from).with_context(|| format!("reading {}", entry.from.display()))?;
        let large = from
            .metadata()
            .map_or(true, |m| m.len() >= u64::from(u32::MAX));
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(large);
        zip.start_file(entry.name.as_str(), options)
            .with_context(|| format!("adding {} to {}", entry.name, path.display()))?;
        io::copy(&mut from, &mut zip)
            .with_context(|| format!("adding {} to {}", entry.name, path.display()))?;
    }
    zip.finish()
        .with_context(|| format!("finishing {}", path.display()))?
        .sync_all()
        .with_context(|| format!("writing {}", path.display()))
}
