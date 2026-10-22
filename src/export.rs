//! `muman export`: the song list and the library, once, in one zip or
//! one new folder: `songs.toml` at its root and every song muman wrote
//! under `library/`, at its path in the library, with its lyrics.
//!
//! The library is exported as the last run left it, from what the state
//! records, so a song not written yet is not in it; `sync` first brings it
//! in step. Entries are stored, not compressed: audio does not compress.
//! The library, the song list and the state are only read, under the
//! home's lock.
//!
//! **Settings.** An export may change any setting of `[library]`,
//! `[audio]` and `[loudness]` for itself alone, and its library is then the
//! one a sync under those settings would write from the same song list.
//! Every song is resolved again by `reconcile::plan_with`, under the
//! song list's settings and under the export's, so naming, paths kept
//! apart and loudness are the sync's own and no setting is read in two
//! ways. Both take every source measured as there, so both pick what the
//! last run picked, and their plans differ only where the settings do. A
//! song whose plan the change leaves as it was is its library file, copied
//! as the last run left it, one lowered for `[library] max_size` too, at
//! the path the export's settings give it; any other is rendered from its
//! sources into a scratch folder beside the zip, as a sync would write it.
//! So the library is a cache of the export. A song's file is the one
//! `reconcile::files_of` gives it, as a sync decides.
//!
//! A song changed since muman wrote it is copied as the library holds it:
//! the file is the user's. So is one whose sources are gone, or that the
//! export's settings resolve to no plan, as its file is the only one there
//! is; each is said, and the export fails. A song made anew runs no
//! `written` hook. The settings that place or cap the library folder,
//! `path`, `touch_root` and `max_size`, are no export's, and `block_size`
//! is a folder's alone. Nor is `[quality]`: the measures a sync takes follow
//! its weights, so an export could rank by measures never taken. Loudness
//! is refused for the same reason when the song list's mode measures none.
//!
//! With a size, songs are fitted by [`crate::fit`]. Each starts as the file
//! the export writes: its library file, whose size is known, or its plan
//! made anew, estimated from its facts as [`crate::limit`] estimates a
//! plan. Each may be encoded again from its sources at the lower bitrates
//! of the export's `[audio] lossy`, estimated as its bitrate times the
//! song's length, plus what the first holds besides audio: the cover and
//! embedded lyrics its library file holds, else `limit`'s guess at them.
//! The songs chosen are encoded into the scratch folder, and their real
//! sizes replace the estimates. A song's real size corrects its other
//! estimates by as much as it missed, and the songs not encoded yet by how
//! much all missed on average: a VBR encoder spends more on some music than
//! its bitrate, a pure tone half again. [`fit::settle`] fits again until
//! every song chosen is one measured, `PASSES` times at most before it
//! settles on sizes encoded alone. A song that can only be copied stays
//! its library file. Every entry is counted with what the zip spends on
//! it, `ENTRY` bytes and its name twice.
//!
//! Each file encoded for the zip is deleted from the scratch folder once it
//! is in, so an export made anew needs about its own size free beside it,
//! not twice that.
//!
//! **A folder.** `--folder` writes what the zip would hold as files, for
//! a card or a player. It is made whole in the scratch folder beside it,
//! the files encoded for it moved in and the library's copied with their
//! times, each flushed to the disk, then renamed into place: a run killed
//! partway leaves no folder half written. A folder there already is taken
//! only when empty, so no file of another's is written over or mixed in.
//! Each file counts whole blocks of `[library] block_size`, as the
//! library does, since a card's filesystem spends them.

use crate::units;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use lofty::file::TaggedFileExt;
use lofty::tag::ItemKey;
use toml_edit::DocumentMut;
use zip::CompressionMethod;
use zip::write::{FullFileOptions, ZipWriter};

use crate::atomic::Lock;
use crate::dirs::{self, Dirs};
use crate::facts::Facts;
use crate::fit::{self, Item, Rung, Source};
use crate::limit;
use crate::manifest::Manifest;
use crate::parallel;
use crate::reconcile;
use crate::relpath;
use crate::render;
use crate::resolve::{Format, Plan, Resolved};
use crate::runner::Runner;
use crate::settings::{Audio, Override, Settings};
use crate::source::SourceKey;
use crate::state::State;
use crate::store::{Located, Store};

/// Where the songs go inside the zip.
const LIBRARY: &str = "library";
/// What a zip spends on one entry besides its name and data: a local
/// header of 30 bytes, a central one of 46, room for their ZIP64 fields,
/// and the extended timestamp each carries, of 9.
const ENTRY: u64 = 30 + 46 + 64 + 2 * 9;

/// The extended timestamp's extra field, which says an entry's time in
/// UTC as Unix does, where a zip's own MS-DOS time names no zone.
const EXTENDED_TIMESTAMP: u16 = 0x5455;
/// What a zip spends once, at its end, ZIP64 records included.
const END: u64 = 22 + 56 + 20;
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

/// The tables `--set` reaches.
const TABLES: [&str; 3] = ["library", "audio", "loudness"];
/// The settings of those tables no export takes, and why.
const NOT_EXPORTS: [(&str, &str, &str); 3] = [
    (
        "library",
        "path",
        "the export is written where -o names, not into a library folder",
    ),
    (
        "library",
        "touch_root",
        "the export touches no library folder",
    ),
    ("library", "max_size", "--max-size caps the export"),
];

/// What `export` writes, and how.
#[derive(Debug, Clone, Copy)]
pub struct Options<'a> {
    /// The zip, or a folder to write `muman.zip` in; with `folder`, the
    /// folder to make. Unset, `muman.zip` or `muman` here.
    pub output: Option<&'a Path>,
    /// Write a folder rather than a zip.
    pub folder: bool,
    /// The most the export may take, in bytes.
    pub max_size: Option<u64>,
    /// Settings set over the song list's for the export alone, in order.
    pub settings: &'a [Override],
    /// Write the song list with its secrets replaced ([`crate::redact`]).
    pub redact: bool,
}

/// `KEY=VALUE` as `--set` reads it: a setting of `TABLES`, its value
/// TOML, or else text, so `audio.lossy=mp3` needs no quotes.
///
/// # Errors
///
/// When it is not `table.key=value`, or names a table or a setting no
/// export takes.
pub fn setting(arg: &str) -> std::result::Result<Override, String> {
    let (name, raw) = arg
        .split_once('=')
        .ok_or_else(|| format!("`{arg}` sets nothing; write KEY=VALUE, as audio.lossy=mp3"))?;
    let name = name.trim();
    let (table, key) = name.split_once('.').unwrap_or(("", name));
    if !table.is_empty() && !TABLES.contains(&table) {
        return Err(format!(
            "an export sets [library], [audio] and [loudness], not [{table}]"
        ));
    }
    if table.is_empty() || key.is_empty() || key.contains('.') {
        return Err(format!(
            "`{name}` names no setting; write its table and key, as audio.lossy"
        ));
    }
    if let Some((_, _, why)) = NOT_EXPORTS
        .iter()
        .find(|(t, k, _)| *t == table && *k == key)
    {
        return Err(format!("{name} is no export's setting: {why}"));
    }
    let value = raw
        .trim()
        .parse::<toml_edit::Value>()
        .unwrap_or_else(|_| raw.into());
    Ok(Override {
        table: table.into(),
        key: key.into(),
        value,
    })
}

/// The settings `export`'s flags stand for, then those of `set`, which
/// are set after them.
#[must_use]
pub fn overrides(
    codec: Option<&str>,
    template: Option<&str>,
    lyrics: Option<&str>,
    set: &[Override],
) -> Vec<Override> {
    let one = |table: &str, key: &str, value: toml_edit::Value| Override {
        table: table.into(),
        key: key.into(),
        value,
    };
    let mut all = Vec::new();
    if let Some(codec) = codec {
        let only: toml_edit::Array = std::iter::once(codec).collect();
        all.push(one("audio", "codecs", only.into()));
        all.push(one("audio", "lossy", codec.into()));
        all.push(one("audio", "lossless", codec.into()));
    }
    if let Some(template) = template {
        all.push(one("library", "template", template.into()));
    }
    if let Some(lyrics) = lyrics {
        all.push(one("library", "lyrics", lyrics.into()));
    }
    all.extend(set.iter().cloned());
    all
}

/// One song as it is exported.
#[derive(Debug)]
struct Song {
    /// Its path in the zip's library, without its extension.
    stem: PathBuf,
    /// Its file in the library, by its path there.
    path: PathBuf,
    lyrics: Option<PathBuf>,
    /// What its first rung is made from: its library file's plan, or
    /// the one the export's settings resolve it to.
    plan: Plan,
    /// Whether its first rung is `plan` made anew, not its library file.
    anew: bool,
    /// What it may be encoded from; none when it can only be copied.
    sources: Option<BTreeMap<SourceKey, Located>>,
}

/// The songs encoded for the zip, by their place and rung, with their
/// entries.
type Made = BTreeMap<(usize, usize), Vec<Entry>>;

/// A file to put in the zip: where it is read from and its name there.
#[derive(Debug, Clone)]
struct Entry {
    from: PathBuf,
    name: String,
    /// Whether `from` was encoded for the zip, and goes once it is in.
    made: bool,
}

/// What the zip spends on an entry of `bytes` named `name`.
fn entry_size(name: &str, bytes: u64) -> u64 {
    bytes + ENTRY + 2 * name.len() as u64
}

/// What the export is written as, by what each file takes in it.
#[derive(Debug, Clone, Copy)]
enum Target {
    Zip,
    /// A folder, each file in whole blocks of `[library] block_size`.
    Folder {
        block: u64,
    },
}

impl Target {
    /// What a file of `bytes` named `name` takes.
    fn takes(self, name: &str, bytes: u64) -> u64 {
        match self {
            Self::Zip => entry_size(name, bytes),
            Self::Folder { block } => limit::blocks(bytes, block),
        }
    }

    /// What it takes once, besides its files.
    fn end(self) -> u64 {
        match self {
            Self::Zip => END,
            Self::Folder { .. } => 0,
        }
    }
}

fn name_in_zip(path: &Path) -> String {
    format!("{LIBRARY}/{}", relpath::to_portable(path))
}

/// `stem` with the extension of `like`.
fn named(stem: &Path, like: &Path) -> PathBuf {
    let mut name = stem.as_os_str().to_os_string();
    if let Some(ext) = like.extension() {
        name.push(".");
        name.push(ext);
    }
    PathBuf::from(name)
}

fn size_of(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |m| m.len())
}

/// What `files` take in `target`, each as big as its file.
fn entries_size(target: Target, files: &[Entry]) -> u64 {
    files
        .iter()
        .map(|e| target.takes(&e.name, size_of(&e.from)))
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

/// Export the song list and the library in `dirs` into the zip or folder
/// `options` names, under the settings it changes, fitting it into its
/// size when given. Returns whether every song went in as chosen.
pub fn export<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    options: &Options<'_>,
    out: &mut W,
) -> Result<bool> {
    let _lock = Lock::folder(&dirs.home)?;
    let list = dirs.home.join(dirs::MANIFEST);
    if !list.is_file() {
        bail!("{} holds no song list to export", dirs.home.display());
    }
    // The library is as the last run left it; a song list edited since
    // would not describe it.
    if crate::history::songs_as_last_left(&dirs.home) == Some(false) {
        return Err(crate::change::Refused(
            "The song list changed since the last run; `muman sync` first, so the export's \
             library matches it"
                .into(),
        )
        .into());
    }
    let manifest = Manifest::load(&dirs.home)?;
    let settings = if options.settings.is_empty() {
        None
    } else {
        Some(settings_of(&manifest, options.settings)?)
    };
    let state = State::load(&dirs.home)?;
    let store = Store::scan(dirs)?;
    let (output, target) = target_of(options, settings.as_ref().unwrap_or(&manifest.settings))?;
    let beside = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));

    let (songs, mut ok) = songs(dirs, &manifest, settings.as_ref(), &state, &store, out)?;
    let audio = settings
        .as_ref()
        .map_or(&manifest.settings.audio, |s| &s.audio);
    let held: Vec<Vec<Entry>> = songs
        .iter()
        .map(|s| held_entries(&dirs.library, s))
        .collect();
    let scratch = crate::atomic::Scratch::within(beside)?;
    let list_entry = list_entry(list, options.redact, scratch.path(), out)?;
    let (choice, mut made) = if let Some(max) = options.max_size {
        let fixed = entries_size(target, std::slice::from_ref(&list_entry)) + target.end();
        let budget = max
            .checked_sub(fixed)
            .ok_or_else(|| anyhow!("{} cannot hold even the song list", crate::ui::bytes(max)))?;
        let fitted = fit_into(
            runner,
            (audio, target),
            &state,
            (&songs, &held),
            (max, budget),
            scratch.path(),
            out,
        )?;
        ok &= fitted.ok;
        (fitted.choice, fitted.made)
    } else {
        let (made, all) = anew(runner, &songs, scratch.path(), out)?;
        ok &= all;
        (vec![0; songs.len()], made)
    };
    let chosen = chosen(&songs, &held, &choice, &mut made, out)?;
    ok &= chosen.ok;
    let (entries, exported, lowered) = (chosen.entries, chosen.songs, chosen.lowered);

    let size = write(
        target,
        scratch.path(),
        &output,
        std::iter::once(&list_entry).chain(&entries),
    )?;
    crate::ui::success(
        out,
        &format!(
            "Exported the song list and {exported} song(s), {}, to {}",
            crate::ui::bytes(size),
            output.display()
        ),
    )?;
    if let Some(max) = options.max_size {
        if lowered > 0 {
            crate::ui::info(
                out,
                &format!(
                    "{lowered} song(s) encoded at a lower bitrate to fit {}",
                    crate::ui::bytes(max)
                ),
            )?;
        }
        if size > max {
            crate::ui::error(
                out,
                &format!("The export came out over {}", crate::ui::bytes(max)),
            )?;
            ok = false;
        }
    }
    Ok(ok)
}

/// The song list's entry: the file itself, or with `redact` a copy with
/// its secrets replaced, written in `scratch`.
fn list_entry<W: Write>(list: PathBuf, redact: bool, scratch: &Path, out: &mut W) -> Result<Entry> {
    let name = dirs::MANIFEST.to_string();
    if !redact {
        return Ok(Entry {
            from: list,
            name,
            made: false,
        });
    }
    let text = fs::read_to_string(&list).with_context(|| format!("reading {}", list.display()))?;
    let (redacted, n) = crate::redact::redact(&text)?;
    crate::atomic::write(
        scratch,
        &crate::atomic::Name::new(dirs::MANIFEST)?,
        redacted.as_bytes(),
    )?;
    crate::ui::info(out, &format!("{n} secret(s) redacted in the song list"))?;
    Ok(Entry {
        from: scratch.join(dirs::MANIFEST),
        name,
        made: true,
    })
}

/// Where the export goes and what it is written as, under `settings`.
fn target_of(options: &Options<'_>, settings: &Settings) -> Result<(PathBuf, Target)> {
    if !options.folder {
        let block = options
            .settings
            .iter()
            .any(|o| o.table == "library" && o.key == "block_size");
        if block {
            return Err(crate::cli::Invalid(
                "library.block_size counts only in a folder, and a zip has no blocks; give \
                 --folder, or leave it out"
                    .into(),
            )
            .into());
        }
        return Ok((
            zip_at(options.output.unwrap_or(Path::new("muman.zip")))?,
            Target::Zip,
        ));
    }
    Ok((
        folder_at(options.output.unwrap_or(Path::new("muman")))?,
        Target::Folder {
            block: settings.library.block_size.0,
        },
    ))
}

/// Write `entries` as `target` at `output`, made in `scratch` so a run
/// killed partway leaves it where the next clears it. Returns what it
/// takes.
fn write<'a>(
    target: Target,
    scratch: &Path,
    output: &Path,
    entries: impl Iterator<Item = &'a Entry>,
) -> Result<u64> {
    if let Target::Folder { .. } = target {
        let entries: Vec<&Entry> = entries.collect();
        let size = entries
            .iter()
            .map(|e| target.takes(&e.name, size_of(&e.from)))
            .sum();
        write_folder(&scratch.join("export"), output, entries.into_iter())?;
        return Ok(size);
    }
    let part = scratch.join("export.zip.part");
    write_zip(&part, entries)?;
    crate::atomic::rename(&part, output)
        .with_context(|| format!("renaming {} to {}", part.display(), output.display()))?;
    Ok(size_of(output))
}

/// The entries the zip holds of its songs, in name order.
struct Chosen {
    entries: Vec<Entry>,
    /// The songs they are.
    songs: usize,
    /// The songs lowered to fit.
    lowered: usize,
    /// Whether no song was left out.
    ok: bool,
}

/// Each song's entries at the rung `choice` gives it: what was encoded
/// for it, else its first rung made anew, else its library file. A song
/// whose name in the zip another took, as files of one name in two
/// cases would, is left out and said.
fn chosen<W: Write>(
    songs: &[Song],
    held: &[Vec<Entry>],
    choice: &[usize],
    made: &mut Made,
    out: &mut W,
) -> Result<Chosen> {
    let mut chosen = Chosen {
        entries: Vec::new(),
        songs: 0,
        lowered: 0,
        ok: true,
    };
    let mut taken = BTreeSet::new();
    for (n, song) in songs.iter().enumerate() {
        let c = choice[n];
        let entries = if let Some(entries) = made.remove(&(n, c)) {
            chosen.lowered += usize::from(c > 0);
            entries
        } else {
            made.remove(&(n, 0)).unwrap_or_else(|| held[n].clone())
        };
        let names: Vec<String> = entries.iter().map(|e| e.name.to_lowercase()).collect();
        if names.iter().any(|name| taken.contains(name)) {
            crate::ui::warning(
                out,
                &format!(
                    "{} is left out: another song takes its name in the zip",
                    relpath::show(&song.path)
                ),
            )?;
            chosen.ok = false;
            continue;
        }
        taken.extend(names);
        chosen.entries.extend(entries);
        chosen.songs += 1;
    }
    chosen.entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(chosen)
}

/// The settings the export writes under: the song list's, with
/// `overrides` set over them and checked as the song list's own are.
fn settings_of(manifest: &Manifest, overrides: &[Override]) -> Result<Settings> {
    let doc: DocumentMut = manifest
        .text()
        .unwrap_or_default()
        .parse()
        .context("reading the song list")?;
    let invalid = |e: anyhow::Error| {
        crate::cli::Invalid(format!(
            "{:#}",
            e.context("reading the settings given for the export")
        ))
    };
    let settings = crate::settings::read_with(&doc, overrides).map_err(invalid)?;
    crate::naming::Naming::new(&settings.library, Path::new("")).map_err(invalid)?;
    if settings.loudness.measured() && !manifest.settings.loudness.measured() {
        return Err(crate::change::Refused(
            "The export levels songs, and the song list's [loudness] mode = \"off\" measures \
             none; set its mode and `muman sync` first"
                .into(),
        )
        .into());
    }
    Ok(settings)
}

/// The sources a plan is made from, where the store holds every one.
fn located(store: &Store, plan: &Plan) -> Option<BTreeMap<SourceKey, Located>> {
    std::iter::once(&plan.audio.key)
        .chain(plan.cover.as_ref().map(|c| &c.key))
        .chain(plan.lyrics.as_ref().map(|l| &l.key))
        .map(|k| Some((k.clone(), store.locate(k)?)))
        .collect()
}

/// Songs the export's settings ask for and the library can only give as
/// it holds them, by why.
#[derive(Debug, Default)]
struct Kept {
    changed: usize,
    gone: usize,
    unresolved: usize,
}

/// The songs the library holds as the state records them, in its path
/// order, each as the export writes it: under `settings`, when given,
/// and else as the library holds it. Returns them with whether every song
/// the library should hold is there, as asked.
fn songs<W: Write>(
    dirs: &Dirs,
    manifest: &Manifest,
    settings: Option<&Settings>,
    state: &State,
    store: &Store,
    out: &mut W,
) -> Result<(Vec<Song>, bool)> {
    let owners: BTreeMap<PathBuf, usize> =
        reconcile::files_of(0..manifest.songs.len(), manifest, &state.outputs)
            .into_iter()
            .map(|(n, path)| (path, n))
            .collect();
    let mut planned: Option<(BTreeMap<usize, Resolved>, BTreeMap<usize, Resolved>)> = None;
    if let Some(settings) = settings {
        let (library, _) = reconcile::plan_with(
            manifest,
            &manifest.settings,
            state,
            &dirs.library,
            None,
            &mut io::sink(),
        )?;
        let (export, _) =
            reconcile::plan_with(manifest, settings, state, &dirs.library, None, out)?;
        planned = Some((library.into_iter().collect(), export.into_iter().collect()));
    }
    let mut songs = Vec::new();
    let mut missing = 0_usize;
    let mut kept = Kept::default();
    for (path, written) in &state.outputs {
        let Some(plan) = &written.plan else {
            continue;
        };
        if size_of(&dirs.library.join(path)) == 0 {
            missing += 1;
            continue;
        }
        let changed = reconcile::changed_since_written(&dirs.library, &state.outputs, path);
        let mut song = Song {
            stem: path.with_extension(""),
            path: path.clone(),
            lyrics: written
                .lyrics
                .clone()
                .filter(|l| dirs.library.join(l).is_file()),
            plan: plan.clone(),
            anew: false,
            sources: located(store, plan).filter(|_| !changed),
        };
        if let (Some((library, export)), Some(n)) = (&planned, owners.get(path)) {
            match (library.get(n), export.get(n)) {
                (Some(was), Some(now)) => {
                    song.stem.clone_from(&now.stem);
                    if now.plan != was.plan {
                        match located(store, &now.plan) {
                            _ if changed => kept.changed += 1,
                            None => kept.gone += 1,
                            Some(sources) => {
                                song.plan = now.plan.clone();
                                song.anew = true;
                                song.sources = Some(sources);
                            }
                        }
                    }
                }
                (None, Some(now)) => song.stem.clone_from(&now.stem),
                (_, None) => kept.unresolved += 1,
            }
        }
        songs.push(song);
    }
    if missing > 0 {
        crate::ui::warning(
            out,
            &format!(
                "{missing} song(s) muman wrote are missing from the library and left out; `muman sync` writes them again"
            ),
        )?;
    }
    let reasons = [
        (kept.changed, "changed since muman wrote them"),
        (kept.gone, "whose sources are gone"),
        (
            kept.unresolved,
            "that do not resolve under the settings given",
        ),
    ];
    for (count, why) in reasons.into_iter().filter(|(count, _)| *count > 0) {
        crate::ui::warning(
            out,
            &format!(
                "{count} song(s) {why} are exported as the library holds them, not as the \
                 settings given ask"
            ),
        )?;
    }
    Ok((
        songs,
        missing == 0 && reasons.iter().all(|(count, _)| *count == 0),
    ))
}

/// A song's library file and lyrics, named in the zip by its stem.
fn held_entries(library: &Path, song: &Song) -> Vec<Entry> {
    std::iter::once(&song.path)
        .chain(&song.lyrics)
        .map(|p| Entry {
            from: library.join(p),
            name: name_in_zip(&named(&song.stem, p)),
            made: false,
        })
        .collect()
}

/// The rung fitting chose for each song, by its place, what was encoded,
/// by song and rung, and whether every encoding asked for succeeded.
struct Fitted {
    choice: Vec<usize>,
    made: Made,
    ok: bool,
}

/// The zip `output` names: itself, or `muman.zip` in it when it is a
/// folder. A folder that is not there is refused rather than made, as a
/// typo would make one.
fn zip_at(output: &Path) -> Result<PathBuf> {
    let zip = if output.is_dir() {
        output.join("muman.zip")
    } else {
        output.to_path_buf()
    };
    let folder = zip
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !folder.is_dir() {
        bail!(
            "{} is no folder; make it first, or write the zip into one that is",
            folder.display()
        );
    }
    Ok(zip)
}

/// The folder `output` names, to be made: one not there, in a folder
/// that is, or one empty. A folder holding anything is refused, so no
/// file of another's is written over or mixed in with the export.
fn folder_at(output: &Path) -> Result<PathBuf> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !parent.is_dir() {
        bail!(
            "{} is no folder; make it first, or write the export into one that is",
            parent.display()
        );
    }
    if output.exists() {
        let empty = fs::read_dir(output)
            .with_context(|| format!("reading {}", output.display()))?
            .next()
            .is_none();
        if !empty {
            bail!(
                "{} is not empty; name a folder to make, such as {}",
                output.display(),
                output.join("muman").display()
            );
        }
    }
    Ok(output.to_path_buf())
}

/// Fit the songs into `budget` bytes by [`fit::settle`], encoding those
/// chosen into `scratch`; each encoding's real size replaces its
/// estimate, and corrects the estimates not measured yet.
fn fit_into<R: Runner, W: Write>(
    runner: &R,
    (audio, target): (&Audio, Target),
    state: &State,
    (songs, held): (&[Song], &[Vec<Entry>]),
    (max, budget): (u64, u64),
    scratch: &Path,
    out: &mut W,
) -> Result<Fitted> {
    let mut items: Vec<Item> = songs
        .iter()
        .zip(held)
        .map(|(song, entries)| item((audio, target), state, song, entries))
        .collect();
    let estimates: Vec<Vec<u64>> = items
        .iter()
        .map(|i| i.rungs.iter().map(|r| r.bytes).collect())
        .collect();
    let mut ratios: Vec<Option<f64>> = vec![None; items.len()];
    let mut made = Made::new();
    let mut ok = true;
    let mut again = false;
    let mut measure = |items: &mut [Item], wanted: &fit::Wanted| -> Result<()> {
        let due: Vec<(usize, usize, Format)> = wanted
            .iter()
            .filter(|(n, c)| !items[*n].rungs[*c].known)
            .map(|&(n, c)| (n, c, items[n].rungs[c].format))
            .collect();
        let results = make(runner, songs, &due, again, scratch, out)?;
        again |= due.iter().any(|(_, c, _)| *c > 0);
        for ((n, c, _), result) in due.into_iter().zip(results) {
            let rung = &mut items[n].rungs[c];
            if let Some(entries) = result {
                let real = entries_size(target, &entries);
                let ratio = real as f64 / estimates[n][c].max(1) as f64;
                ratios[n] = Some(ratio.clamp(1.0 / MISS, MISS));
                rung.bytes = real;
                rung.sigma = 0.0;
                rung.known = true;
                made.insert((n, c), entries);
            } else {
                ok = false;
                // A first rung not made is the library file, copied.
                if c == 0 {
                    rung.bytes = entries_size(target, &held[n]);
                    rung.sigma = 0.0;
                    rung.known = true;
                }
            }
        }
        correct(items, &estimates, &ratios);
        Ok(())
    };
    let settled = fit::settle(
        &mut items,
        &fit::Policy::exact(budget),
        PASSES,
        &mut measure,
    )?;
    let fit = match settled {
        fit::Settled::Fit(fit) => fit,
        fit::Settled::Over(floor) => bail!(
            "{} cannot hold the export: at the lowest bitrates it takes {}",
            crate::ui::bytes(max),
            crate::ui::bytes(floor + (max - budget))
        ),
    };
    Ok(Fitted {
        choice: fit.choice,
        made,
        ok,
    })
}

/// Make each song the export's settings ask for anew into `scratch`, by
/// its place and its first rung, with whether every one was made.
fn anew<R: Runner, W: Write>(
    runner: &R,
    songs: &[Song],
    scratch: &Path,
    out: &mut W,
) -> Result<(Made, bool)> {
    let due: Vec<(usize, usize, Format)> = songs
        .iter()
        .enumerate()
        .filter(|(_, s)| s.anew)
        .map(|(n, s)| (n, 0, s.plan.format))
        .collect();
    let results = make(runner, songs, &due, false, scratch, out)?;
    let all = results.iter().all(Option::is_some);
    let made = due
        .into_iter()
        .zip(results)
        .filter_map(|((n, c, _), made)| Some(((n, c), made?)))
        .collect();
    Ok((made, all))
}

/// Encode each song of `due` at its rung's format into `scratch`: what
/// each made, none where it could not be, which is said.
fn make<R: Runner, W: Write>(
    runner: &R,
    songs: &[Song],
    due: &[(usize, usize, Format)],
    again: bool,
    scratch: &Path,
    out: &mut W,
) -> Result<Vec<Option<Vec<Entry>>>> {
    if due.is_empty() {
        return Ok(Vec::new());
    }
    let anew = due.iter().filter(|(_, c, _)| *c == 0).count();
    if anew > 0 {
        crate::ui::info(
            out,
            &format!("Encoding {anew} song(s) as the settings given ask"),
        )?;
    }
    if due.len() > anew {
        crate::ui::info(
            out,
            &format!(
                "Encoding {} song(s) at a lower bitrate to fit{}",
                due.len() - anew,
                if again { ", again" } else { "" }
            ),
        )?;
    }
    let step = crate::progress::step("Encoding", Some(due.len() as u64));
    let rendered = parallel::map(due, parallel::builds(), |(n, c, format)| {
        let _working = step.working(&crate::progress::label(&songs[*n].path));
        encode(
            runner,
            &songs[*n],
            *format,
            &scratch.join(format!("{n}-{c}")),
        )
    });
    let mut made = Vec::with_capacity(due.len());
    for ((n, c, _), result) in due.iter().zip(rendered) {
        let shown = relpath::show(&songs[*n].path);
        match result {
            Ok((entries, problems)) => {
                for problem in problems {
                    crate::ui::warning(out, &format!("{shown}: {problem}"))?;
                }
                made.push(Some(entries));
            }
            Err(e) => {
                let then = if *c == 0 {
                    "it is exported as the library holds it"
                } else {
                    "it is not lowered"
                };
                crate::ui::warning(out, &format!("Could not encode {shown}: {e:#}; {then}"))?;
                made.push(None);
            }
        }
    }
    Ok(made)
}

/// Scale each estimate not yet replaced by a real size by how far the
/// song's encodings missed theirs, or, for a song not encoded yet, by how
/// far all did on average, by their logarithms.
fn correct(items: &mut [Item], estimates: &[Vec<u64>], ratios: &[Option<f64>]) {
    let seen: Vec<f64> = ratios.iter().flatten().map(|r| r.ln()).collect();
    if seen.is_empty() {
        return;
    }
    let mean = (seen.iter().sum::<f64>() / seen.len() as f64).exp();
    for (n, item) in items.iter_mut().enumerate() {
        let ratio = ratios[n].unwrap_or(mean);
        for (c, rung) in item.rungs.iter_mut().enumerate() {
            if !rung.known {
                let scaled = (estimates[n][c] as f64 * ratio) as u64;
                rung.bytes = scaled;
            }
        }
    }
}

/// One song's ladder: its first rung, the library file as it is or its
/// plan made anew, then each lower rung it may be encoded at, estimated.
fn item((audio, target): (&Audio, Target), state: &State, song: &Song, held: &[Entry]) -> Item {
    let facts = state.facts.get(&song.plan.audio.key);
    let seconds = facts.and_then(Facts::length).filter(|s| *s > 0.0);
    // The audio file's name and what it holds besides its stream, and what
    // its lyrics beside it take.
    let (name, extras, lyrics) = if song.anew {
        let (extras, sidecar) = limit::besides_audio(&song.plan);
        let name = |ext: &str| name_in_zip(&named(&song.stem, Path::new(&format!("_.{ext}"))));
        let lyrics = if sidecar > 0 {
            target.takes(&name("lrc"), sidecar)
        } else {
            0
        };
        (name(song.plan.format.extension()), extras, lyrics)
    } else {
        let lyrics = entries_size(target, &held[1..]);
        (held[0].name.clone(), extras(&held[0].from), lyrics)
    };
    let takes = |stream: u64| target.takes(&name, stream + extras) + lyrics;
    let first = if song.anew {
        let stream = match (facts, seconds) {
            (Some(f), Some(s)) => limit::estimate(song.plan.format, f, s).0 as u64,
            _ => 0,
        };
        takes(stream)
    } else {
        entries_size(target, held)
    };
    let mut item = Item {
        key: relpath::to_portable(&song.stem),
        rungs: vec![Rung {
            format: song.plan.format,
            bytes: first,
            loss: 0.0,
            sigma: 0.0,
            known: !song.anew,
        }],
        churn: 0.0,
    };
    let (Some(_), Some(facts), Some(seconds), Some(shape)) = (
        &song.sources,
        facts,
        seconds,
        facts.and_then(|f| f.audio.as_ref()),
    ) else {
        return item;
    };
    let source = Source {
        lossless: shape.is_lossless(),
        bandwidth_hz: shape.quality.map(|q| q.bandwidth_hz),
        seconds,
    };
    item.churn = CHURN_PER_MINUTE * units::minutes_of_seconds(seconds);
    for format in fit::lower(song.plan.format, &shape.shape(), audio) {
        let bytes = takes(limit::estimate(format, facts, seconds).0 as u64);
        if bytes.saturating_mul(10) <= first.saturating_mul(9) {
            item.rungs.push(Rung {
                format,
                bytes,
                loss: fit::loss(&source, song.plan.format, format),
                sigma: 0.0,
                known: false,
            });
        }
    }
    item
}

/// Encode one song as `format` into `scratch`, returning its entries and
/// what could not be added to it.
fn encode<R: Runner>(
    runner: &R,
    song: &Song,
    format: Format,
    scratch: &Path,
) -> Result<(Vec<Entry>, Vec<String>)> {
    let sources = song
        .sources
        .as_ref()
        .ok_or_else(|| anyhow!("its sources are gone"))?;
    let plan = if format == song.plan.format {
        song.plan.clone()
    } else {
        song.plan.with_format(format)
    };
    let library = scratch.join("library");
    let rendered = render::render(
        runner,
        &render::Job {
            plan: &plan,
            stem: &song.stem,
            library: &library,
            sources,
            scratch: &scratch.join("work"),
        },
    )?;
    let entries = std::iter::once(&rendered.audio)
        .chain(&rendered.lyrics)
        .map(|p| Entry {
            from: library.join(p),
            name: name_in_zip(p),
            made: true,
        })
        .collect();
    Ok((entries, rendered.problems))
}

/// `time` as a zip stamps an entry in MS-DOS time, in UTC to the even
/// second, beside the extended timestamp that says it is; none before
/// 1980, which MS-DOS time cannot say.
fn zip_time(time: std::time::SystemTime) -> Option<zip::DateTime> {
    let secs = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let (days, of_day) = (secs / 86_400, secs % 86_400);
    // Days since 1970 as a civil date, after Howard Hinnant's algorithm.
    let z = i64::try_from(days).ok()? + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    zip::DateTime::from_date_and_time(
        u16::try_from(year).ok()?,
        u8::try_from(month).ok()?,
        u8::try_from(day).ok()?,
        u8::try_from(of_day / 3_600).ok()?,
        u8::try_from(of_day % 3_600 / 60).ok()?,
        u8::try_from(of_day % 60).ok()?,
    )
    .ok()
}

/// Write `entries` into a new folder `staging`, then rename it to `to`:
/// each at its name, a file encoded for the export moved there and any
/// other copied with its time, every one on the disk before the folder
/// takes its place.
fn write_folder<'a>(
    staging: &Path,
    to: &Path,
    entries: impl Iterator<Item = &'a Entry>,
) -> Result<()> {
    let entries: Vec<&Entry> = entries.collect();
    let step = crate::progress::step("Writing the folder", Some(entries.len() as u64));
    for entry in entries {
        let _working = step.working(&entry.name);
        let file = entry
            .name
            .split('/')
            .fold(staging.to_path_buf(), |path, part| path.join(part));
        let made = |e: io::Error| {
            anyhow::Error::from(e).context(format!("writing {} to {}", entry.name, to.display()))
        };
        if let Some(folder) = file.parent() {
            fs::create_dir_all(folder).map_err(made)?;
        }
        if entry.made {
            crate::atomic::rename(&entry.from, &file).map_err(made)?;
        } else {
            fs::copy(&entry.from, &file).map_err(made)?;
            if let Ok(meta) = fs::metadata(&entry.from) {
                let time = filetime::FileTime::from_last_modification_time(&meta);
                filetime::set_file_mtime(&file, time).map_err(made)?;
            }
        }
        // Opened to write: Windows flushes no file opened only to read.
        fs::OpenOptions::new()
            .write(true)
            .open(&file)
            .and_then(|f| f.sync_all())
            .map_err(made)?;
    }
    // Windows renames no folder onto another, even an empty one.
    if to.is_dir() {
        fs::remove_dir(to).with_context(|| format!("replacing {}", to.display()))?;
    }
    crate::atomic::rename(staging, to)
        .with_context(|| format!("renaming {} to {}", staging.display(), to.display()))
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
        let meta = from.metadata().ok();
        let large = meta.as_ref().is_none_or(|m| m.len() >= u64::from(u32::MAX));
        let mut options = FullFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(large);
        let modified = meta.and_then(|m| m.modified().ok());
        if let Some(time) = modified.and_then(zip_time) {
            options = options.last_modified_time(time);
        }
        // MS-DOS time names no zone, and an extractor reads it as its own:
        // the UTC time, extended, is what Info-ZIP, 7-Zip and libarchive read.
        if let Some(secs) = modified
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| u32::try_from(d.as_secs()).ok())
        {
            let mut field = vec![1];
            field.extend(secs.to_le_bytes());
            options
                .add_extra_field(EXTENDED_TIMESTAMP, field, false)
                .with_context(|| format!("stamping {}", entry.name))?;
        }
        zip.start_file(entry.name.as_str(), options)
            .with_context(|| format!("adding {} to {}", entry.name, path.display()))?;
        io::copy(&mut from, &mut zip)
            .with_context(|| format!("adding {} to {}", entry.name, path.display()))?;
        // Closed first: Windows deletes no file held open.
        drop(from);
        if entry.made {
            let _ = fs::remove_file(&entry.from);
        }
    }
    zip.finish()
        .with_context(|| format!("finishing {}", path.display()))?
        .sync_all()
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Codec;

    #[test]
    fn a_setting_is_a_table_s_key_and_its_value_toml_or_else_text() {
        let set = setting("audio.lossy=mp3").unwrap();
        assert_eq!((set.table.as_str(), set.key.as_str()), ("audio", "lossy"));
        assert_eq!(set.value.as_str(), Some("mp3"));
        let set = setting("library.template={{ title }}").unwrap();
        assert_eq!(set.value.as_str(), Some("{{ title }}"));
        assert_eq!(
            setting("library.max_name_bytes=100")
                .unwrap()
                .value
                .as_integer(),
            Some(100)
        );
        assert!(setting("audio.codecs=[\"mp3\"]").unwrap().value.is_array());
        for refused in [
            "audio.lossy",
            "lossy=mp3",
            "quality.purity.weight=1",
            "ytdlp.format=ba",
            "library.path=/x",
            "library.max_size=1GiB",
        ] {
            assert!(setting(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn a_codec_given_writes_every_song_in_it_and_set_comes_last() {
        let later = setting("audio.lossless=flac").unwrap();
        let all = overrides(Some("mp3"), None, Some("embedded"), &[later]);
        let named: Vec<String> = all
            .iter()
            .map(|o| format!("{}.{}={}", o.table, o.key, o.value.to_string().trim()))
            .collect();
        assert_eq!(
            named,
            [
                "audio.codecs=[\"mp3\"]",
                "audio.lossy=\"mp3\"",
                "audio.lossless=\"mp3\"",
                "library.lyrics=\"embedded\"",
                "audio.lossless=\"flac\"",
            ]
        );
    }

    #[test]
    fn an_entry_is_stamped_with_its_file_s_time() {
        let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_717_243_506);
        let stamp = zip_time(at).unwrap();
        assert_eq!((stamp.year(), stamp.month(), stamp.day()), (2024, 6, 1));
        assert_eq!((stamp.hour(), stamp.minute(), stamp.second()), (12, 5, 6));
        assert!(zip_time(std::time::UNIX_EPOCH).is_none());
    }

    #[test]
    fn an_entry_says_its_time_in_utc_beside_ms_dos_time() {
        let dir = tempfile::tempdir().unwrap();
        let song = dir.path().join("song.opus");
        fs::write(&song, b"opus").unwrap();
        let secs = 1_717_243_506;
        filetime::set_file_mtime(&song, filetime::FileTime::from_unix_time(secs, 0)).unwrap();
        let zip = dir.path().join("out.zip");
        let entry = Entry {
            from: song,
            name: "library/A/song.opus".into(),
            made: false,
        };
        write_zip(&zip, std::iter::once(&entry)).unwrap();
        let mut archive = zip::ZipArchive::new(File::open(&zip).unwrap()).unwrap();
        let read = archive.by_index(0).unwrap();
        let stamped = read.extra_data_fields().find_map(|f| match f {
            zip::extra_fields::ExtraField::ExtendedTimestamp(t) => t.mod_time(),
            _ => None,
        });
        assert_eq!(stamped, Some(u32::try_from(secs).unwrap()));
        drop(read);
        let written = fs::metadata(&zip).unwrap().len();
        assert!(
            written <= entry_size("library/A/song.opus", 4) + END,
            "ENTRY counts it"
        );
    }

    fn rung(bytes: u64, known: bool) -> fit::Rung {
        fit::Rung {
            format: Format::Copy { codec: Codec::Flac },
            bytes,
            loss: 0.0,
            sigma: 0.0,
            known,
        }
    }

    #[test]
    fn estimates_follow_how_far_encodings_missed() {
        let mut items = vec![
            Item {
                key: "a".into(),
                rungs: vec![rung(100, true), rung(52, true), rung(20, false)],
                churn: 0.0,
            },
            Item {
                key: "b".into(),
                rungs: vec![rung(100, true), rung(40, false)],
                churn: 0.0,
            },
        ];
        let estimates = vec![vec![100, 40, 20], vec![100, 40]];
        correct(&mut items, &estimates, &[Some(1.3), None]);
        assert_eq!(items[0].rungs[1].bytes, 52, "a size encoded stays");
        assert_eq!(items[0].rungs[2].bytes, 26, "the song's own miss");
        assert_eq!(items[1].rungs[1].bytes, 52, "the others' miss");
    }
}
