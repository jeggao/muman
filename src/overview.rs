//! `muman info`: what the library holds, whether it is in step with
//! the song list, and what in it could be better. Read from the song
//! list, the state and the folders alone: nothing is measured, fetched
//! or locked, so it answers at once and changes nothing; the programs
//! are run for their versions alone ([`programs`]). Whether
//! the library is in step is what a sync would do with each song, by the
//! decisions a sync makes (`reconcile::foresight`), on the state as a
//! sync would take it, so `info`, `status` and `sync` never disagree.
//! Songs are counted on albums as loudness and folder covers group them
//! (`album::of_each`): a song naming no album is a single, an album of
//! its own, and only an album artist named is counted.
//!
//! `--changes` answers what those songs would change into, value by
//! value: each song's plan as its file was written, the one a state
//! entry keeps, against the plan a sync would make now, with the reason
//! `status` gives for the new value. It is how a change to picking or
//! cleaning is checked on a whole library before a sync writes it: a
//! song in step, or not written yet, has nothing to compare. Tags are
//! named by their Vorbis comment, as written; `path`, `audio`, `format`,
//! `cover` and `lyrics` name the rest.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::ui::{Painted, Style};
use anyhow::Result;

use crate::codec::Codec;
use crate::dirs::Dirs;
use crate::facts::Facts;
use crate::lookup;
use crate::manifest::{LyricsPin, Manifest};
use crate::quality::SOFT_COVER;
use crate::reconcile;
use crate::resolve::{Plan, Resolved};
use crate::source::SourceKey;
use crate::state;
use crate::state::State;
use crate::store::{self, Store};
use crate::tags::Field;

/// A lossy encoder's lowpass below this leaves the top of the audible
/// range out.
const NARROW_HZ: f64 = 16_000.0;
/// Clipped samples above this share are heard.
const CLIPPED: f64 = 0.001;
/// The label column's width.
const LABEL: usize = 24;

/// Songs that share one trait, named so `--verbose` can list them.
#[derive(Default)]
struct Group(Vec<String>);

impl Group {
    fn add(&mut self, name: &str) {
        self.0.push(name.to_string());
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

/// What every section reads.
struct Context<'a> {
    dirs: &'a Dirs,
    verbose: bool,
    manifest: Manifest,
    state: State,
    store: Store,
    planned: Vec<(usize, Resolved)>,
    /// What the next sync decides of each song planned.
    foreseen: reconcile::Foreseen,
    /// What it does with each file.
    intent: reconcile::Intent,
    /// Each song that could not be planned, named, and why.
    unresolved: Vec<String>,
    /// The library folder written to before, where it was another.
    moved_from: Option<PathBuf>,
}

impl Context<'_> {
    fn name(&self, n: usize, r: &Resolved) -> String {
        let song = &self.manifest.songs[n];
        let name = reconcile::name_of(song, Some(r), &self.state.facts);
        song.id()
            .map_or_else(|| name.clone(), |id| format!("{id} — {name}"))
    }
}

/// Write the library's statistics, its status against the song list,
/// and its health, every song behind a count when `verbose`.
pub fn info<W: Write>(dirs: &Dirs, verbose: bool, out: &mut W) -> Result<()> {
    let c = context(dirs, verbose)?;
    folders(&c, out)?;
    library(&c, out)?;
    sources(&c, out)?;
    status(&c, out)?;
    health(&c, out)
}

/// The song list, the state as a sync takes it and each song planned as
/// a sync would plan it, for every section to read.
fn context(dirs: &Dirs, verbose: bool) -> Result<Context<'_>> {
    let manifest = Manifest::load(&dirs.home)?;
    let mut state = State::load(&dirs.home)?;
    let moved_from = reconcile::as_found(dirs, &mut state, crate::history::Moves::Foresee)?
        .map(|adopted| adopted.from);
    let store = Store::scan(dirs)?;
    // A source a sync fetches is there by the time it plans; one it does
    // not, as a file of the user's own or one whose fetch failed lately, is not.
    let ahead = reconcile::Ahead::foreseen(&manifest, &state, &store);
    let foresight = reconcile::foresight(&reconcile::Grounds {
        manifest: &manifest,
        state: &state,
        store: &store,
        library: &dirs.library,
        ahead: &ahead,
        asked: (false, false),
    })?;
    let unresolved = (foresight.unresolved.iter())
        .map(|(n, e)| {
            let name = reconcile::name_of(&manifest.songs[*n], None, &state.facts);
            format!("{name}: {e:#}")
        })
        .collect();
    Ok(Context {
        dirs,
        verbose,
        planned: foresight.planned,
        foreseen: foresight.foreseen,
        intent: foresight.intent,
        manifest,
        state,
        store,
        unresolved,
        moved_from,
    })
}

/// What a changes view names besides tags.
const ASPECTS: [&str; 5] = ["path", "audio", "format", "cover", "lyrics"];

/// Write each value a sync would change in each song: the value its file
/// holds now, the one it would hold and why, for `fields` alone when any
/// are named, then how many songs each field changes in.
pub fn changes<W: Write>(dirs: &Dirs, fields: &[String], out: &mut W) -> Result<()> {
    let c = context(dirs, false)?;
    let wanted = wanted(&c, fields)?;
    let foreseen = &c.foreseen;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let (mut changed, mut unwritten) = (0, 0);
    for (n, r) in &c.planned {
        let file = foreseen.files.get(n);
        let Some((file, was)) =
            file.and_then(|f| Some((f, c.state.outputs.get(f)?.plan.as_ref()?)))
        else {
            unwritten += 1;
            continue;
        };
        let differ: Vec<(String, String)> = differences(file, was, r)
            .into_iter()
            .filter(|(field, _)| wanted.is_empty() || wanted.contains(field))
            .collect();
        if differ.is_empty() {
            continue;
        }
        changed += 1;
        heading(out, &c.name(*n, r))?;
        let width = differ.iter().map(|(f, _)| f.len()).max().unwrap_or(0);
        for (field, change) in &differ {
            crate::ui::line(out, &format!("  {field:<width$} {change}"))?;
            *counts.entry(field.clone()).or_default() += 1;
        }
    }
    if changed == 0 {
        crate::ui::line(out, "No song would change")?;
    } else {
        let mut by_count: Vec<(&String, &usize)> = counts.iter().collect();
        by_count.sort_by_key(|(field, n)| (std::cmp::Reverse(**n), *field));
        let said: Vec<String> = by_count.iter().map(|(f, n)| format!("{f} {n}")).collect();
        crate::ui::line(
            out,
            &format!("{changed} song(s) would change: {}", said.join(", ")),
        )?;
    }
    if unwritten > 0 {
        crate::ui::line(out, &format!("{unwritten} song(s) not written yet"))?;
    }
    if !c.unresolved.is_empty() {
        crate::ui::line(
            out,
            &format!("{} song(s) cannot be made", c.unresolved.len()),
        )?;
    }
    Ok(())
}

/// `fields` as the changes view names them, refused when one names
/// nothing a song is written with.
fn wanted(c: &Context<'_>, fields: &[String]) -> Result<BTreeSet<String>> {
    let written: BTreeSet<&str> = (c.planned.iter().map(|(_, r)| &r.plan))
        .chain(c.state.outputs.values().filter_map(|w| w.plan.as_ref()))
        .flat_map(|p| p.tags.iter().map(|(k, _)| k.as_str()))
        .collect();
    let mut wanted = BTreeSet::new();
    for field in fields {
        let lower = field.to_ascii_lowercase();
        let name = if ASPECTS.contains(&lower.as_str()) {
            lower
        } else {
            crate::tags::vorbis_key(field)
        };
        if !ASPECTS.contains(&name.as_str())
            && Field::named(field).is_none()
            && !written.contains(name.as_str())
        {
            return Err(crate::change::Refused(format!(
                "No song is written with `{field}`: a tag such as title, artist, album, \
                 album_artist, genre or date, or one of {}",
                ASPECTS.join(", ")
            ))
            .into());
        }
        wanted.insert(name);
    }
    Ok(wanted)
}

/// Each field whose value differs between the plan `was`, written at
/// `file`, and `r`: the field and its value before and after, with why.
fn differences(file: &Path, was: &Plan, r: &Resolved) -> Vec<(String, String)> {
    let now = &r.plan;
    let mut differ = Vec::new();
    let mut add = |field: &str, before: &str, after: &str, why: &str| {
        let why = if why.is_empty() {
            String::new()
        } else {
            format!(" ({why})")
        };
        differ.push((field.to_string(), format!("{before} → {after}{why}")));
    };
    let path = reconcile::path_of(r);
    if file != path {
        add(
            "path",
            &crate::relpath::show(file),
            &crate::relpath::show(&path),
            "",
        );
    }
    if was.audio.key != now.audio.key {
        let (before, after) = (was.audio.key.to_string(), now.audio.key.to_string());
        add("audio", &before, &after, &r.why.audio);
    }
    if was.format != now.format {
        add("format", &was.format.describe(), &now.format.describe(), "");
    }
    let key = |k: Option<&SourceKey>| k.map_or_else(|| "none".to_string(), ToString::to_string);
    let (before, after) = (
        key(was.cover.as_ref().map(|c| &c.key)),
        key(now.cover.as_ref().map(|c| &c.key)),
    );
    if before != after {
        add(
            "cover",
            &before,
            &after,
            r.why.cover.as_deref().unwrap_or(""),
        );
    }
    let (before, after) = (
        key(was.lyrics.as_ref().map(|l| &l.key)),
        key(now.lyrics.as_ref().map(|l| &l.key)),
    );
    if before != after {
        add(
            "lyrics",
            &before,
            &after,
            r.why.lyrics.as_deref().unwrap_or(""),
        );
    } else if was.lyrics != now.lyrics {
        add("lyrics", &before, "written again", "");
    }
    let values = |tags: &[(String, Vec<String>)], k: &str| {
        tags.iter().find(|(t, _)| t == k).map_or_else(
            || "none".to_string(),
            |(_, v)| crate::tags::escaped(&v.join("; ")),
        )
    };
    for ((k, _), why) in now.tags.iter().zip(&r.why.tags) {
        let (before, after) = (values(&was.tags, k), values(&now.tags, k));
        if before != after {
            let cleaned = if why.cleaned.is_empty() {
                String::new()
            } else {
                format!(", cleaned: {}", why.cleaned.join(", "))
            };
            add(k, &before, &after, &format!("{}{cleaned}", why.from));
        }
    }
    for (k, v) in &was.tags {
        if !now.tags.iter().any(|(t, _)| t == k) {
            add(k, &crate::tags::escaped(&v.join("; ")), "none", "");
        }
    }
    differ
}

/// Each program muman runs as it was found: its version and path, or
/// why it cannot be run, so a report of a problem names them.
pub fn programs<W: Write>(found: &[crate::tools::Status], out: &mut W) -> Result<()> {
    if found.is_empty() {
        return Ok(());
    }
    heading(out, "Programs")?;
    for status in found {
        let (style, value) = match &status.found {
            Ok(f) => {
                let program = f.command[0].to_string_lossy();
                let value = match f.version {
                    Some(v) => format!("{v} at {program}"),
                    None => format!("{program} (version not read)"),
                };
                (Style::Plain, value)
            }
            Err(e) => (Style::Muted, e.to_string()),
        };
        painted_row(out, status.tool.name, style.paint(&value))?;
    }
    Ok(())
}

/// Where the song list and the library are, as flags, variables and the
/// song list's `[library] path` resolved them.
fn folders<W: Write>(c: &Context<'_>, out: &mut W) -> Result<()> {
    heading(out, "Folders")?;
    row(out, "Song list", &c.dirs.manifest().display().to_string())?;
    row(out, "Library", &c.dirs.library.display().to_string())
}

fn library<W: Write>(c: &Context<'_>, out: &mut W) -> Result<()> {
    heading(out, "Library")?;
    let tag = |r: &Resolved, f: Field| {
        r.plan
            .tags
            .iter()
            .find(|(k, _)| k == f.vorbis())
            .and_then(|(_, v)| v.first().cloned())
    };
    let album_of = crate::album::of_each(c.planned.iter().map(|(_, r)| r));
    let albums: BTreeSet<&crate::album::Album> = album_of.iter().flatten().collect();
    let singles = album_of.iter().filter(|a| a.is_none()).count();
    let artists: BTreeSet<String> = c
        .planned
        .iter()
        .filter_map(|(_, r)| tag(r, Field::AlbumArtist))
        .collect();
    row(
        out,
        "Songs",
        &format!(
            "{} ({} on {} album(s), {singles} single(s))",
            c.manifest.songs.len(),
            c.planned.len() - singles,
            albums.len()
        ),
    )?;
    row(out, "Album artists", &artists.len().to_string())?;
    let seconds: f64 = c
        .planned
        .iter()
        .filter_map(|(_, r)| c.state.facts.get(&r.plan.audio.key)?.length())
        .sum();
    row(out, "Playing time", &duration(seconds))?;
    let formats: Vec<String> = Codec::ALL
        .into_iter()
        .filter_map(|codec| {
            let of = |encoded: bool| {
                c.planned
                    .iter()
                    .filter(|(_, r)| {
                        r.plan.format.codec() == codec && (!encoded || r.plan.format.is_encoded())
                    })
                    .count()
            };
            let all = of(false);
            (all > 0).then(|| format!("{all} {} ({} encoded)", codec.label(), of(true)))
        })
        .collect();
    row(
        out,
        "Formats",
        &if formats.is_empty() {
            "none".to_string()
        } else {
            formats.join(", ")
        },
    )?;
    let written = crate::limit::library_size(
        &c.state,
        &c.dirs.library,
        c.manifest.settings.library.block_size.0,
    );
    row(
        out,
        "On disk",
        &format!(
            "{}{} in {}",
            crate::ui::bytes(written),
            c.manifest
                .settings
                .library
                .max_size
                .map(|max| format!(" of {} (max_size)", crate::ui::bytes(max.0)))
                .unwrap_or_default(),
            c.dirs.library.display()
        ),
    )?;

    Ok(())
}

fn sources<W: Write>(c: &Context<'_>, out: &mut W) -> Result<()> {
    heading(out, "Sources")?;
    let listed = c.manifest.keys();
    let kept = listed
        .iter()
        .filter(|k| crate::provider::is_kept(k))
        .count();
    let manual = listed
        .iter()
        .filter(|k| matches!(k, SourceKey::Manual(_)))
        .count();
    let torrent = listed
        .iter()
        .filter(|k| matches!(k, SourceKey::Torrent(_)))
        .count();
    let torrents = if torrent > 0 {
        format!(", {torrent} from torrents")
    } else {
        String::new()
    };
    row(
        out,
        "Listed",
        &format!(
            "{} ({} fetched by yt-dlp{torrents}, {kept} kept from lookups, {manual} manual)",
            listed.len(),
            listed.len() - kept - manual - torrent
        ),
    )?;
    let lookups: u64 = crate::store::KEPT
        .iter()
        .map(|k| folder_size(&k.dir(c.dirs)))
        .sum();
    row(
        out,
        "Stored",
        &format!(
            "{} fetched by yt-dlp{}, {} from lookups, {} manual",
            crate::ui::bytes(folder_size(&c.dirs.ytdlp())),
            if torrent > 0 {
                format!(
                    ", {} from torrents",
                    crate::ui::bytes(folder_size(&c.dirs.torrent()))
                )
            } else {
                String::new()
            },
            crate::ui::bytes(lookups),
            crate::ui::bytes(folder_size(&c.dirs.manual()))
        ),
    )?;
    let mut per_song = [0_usize; 3];
    for song in &c.manifest.songs {
        per_song[song.sources.len().clamp(1, 3) - 1] += 1;
    }
    row(
        out,
        "Sources per song",
        &format!(
            "{} with one, {} with two, {} with more",
            per_song[0], per_song[1], per_song[2]
        ),
    )?;
    row(out, "Uploads replaced", &c.state.replaced.len().to_string())?;

    Ok(())
}

// The steps read in the order they run; split up, the order would be in the call sites.
#[expect(clippy::too_many_lines)]
fn status<W: Write>(c: &Context<'_>, out: &mut W) -> Result<()> {
    let listed = c.manifest.keys();
    heading(out, "Status")?;
    if let Some(before) = &c.moved_from {
        crate::ui::warning(
            out,
            &format!(
                "  Written to {} until now; the next sync takes the library here as it finds it",
                before.display()
            ),
        )?;
    }
    let (mut current, mut due, mut moving, mut alone) =
        (0, Group::default(), Group::default(), Group::default());
    for ((n, r), d) in c.planned.iter().zip(&c.intent.decided) {
        match reconcile::verdict(d, None, &r.plan).0 {
            reconcile::Fate::Kept => current += 1,
            reconcile::Fate::Moved => moving.add(&c.name(*n, r)),
            reconcile::Fate::Alone => alone.add(&c.name(*n, r)),
            reconcile::Fate::New | reconcile::Fate::Changed | reconcile::Fate::Again => {
                due.add(&c.name(*n, r));
            }
        }
    }
    let mut gone = Group::default();
    for file in c.intent.removals().flat_map(|(_, fate)| fate.gone()) {
        gone.add(&crate::relpath::show(file));
    }
    let mut unplanned = Group::default();
    row(out, "Up to date", &current.to_string())?;
    group(out, c.verbose, "To write", &due)?;
    group(out, c.verbose, "To move", &moving)?;
    group(out, c.verbose, "Left alone", &alone)?;
    group(out, c.verbose, "To remove", &gone)?;
    for song in &c.unresolved {
        unplanned.add(song);
    }
    group(out, c.verbose, "Cannot be made", &unplanned)?;
    let verdicts = lookup::verdicts(
        &c.manifest,
        &c.state,
        &c.planned,
        state::now_secs(),
        lookup::Ask::default(),
    )?;
    for p in lookup::ORDER {
        let mut wanted = Group::default();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for (n, q, v) in &verdicts {
            if *q != p {
                continue;
            }
            let kind = match v {
                lookup::Verdict::Due(_) => {
                    wanted.add(&reconcile::name_of(
                        &c.manifest.songs[*n],
                        None,
                        &c.state.facts,
                    ));
                    continue;
                }
                lookup::Verdict::Waits(_) => "waiting on another",
                lookup::Verdict::Later(_) => "asked again later",
                lookup::Verdict::Skipped(_) => "skipped",
                lookup::Verdict::CannotAsk(_) => "cannot ask",
                _ => continue,
            };
            *counts.entry(kind).or_default() += 1;
        }
        group(out, c.verbose, &format!("Look up: {p}"), &wanted)?;
        if !counts.is_empty() {
            let said: Vec<String> = counts.iter().map(|(k, n)| format!("{n} {k}")).collect();
            row(out, &format!("Lookups on {p}"), &said.join(", "))?;
        }
    }
    let (mut missing, mut to_measure) = (Group::default(), Group::default());
    for key in &listed {
        match c.store.locate(key) {
            None => missing.add(&key.to_string()),
            Some(l) => {
                let holds = c
                    .state
                    .facts
                    .get(key)
                    .is_some_and(|f| f.holds_for(&l.rev()) && f.tags_hold(l.kind));
                if !holds {
                    to_measure.add(&key.to_string());
                }
            }
        }
    }
    group(out, c.verbose, "Sources missing", &missing)?;
    group(out, c.verbose, "Sources to measure", &to_measure)?;
    // As the next sync finds them: a removed song's files, and a file moved
    // or a copy of one, are not listed by it.
    let mut found =
        reconcile::Unlisted::of(&c.manifest, &c.state, &c.store, crate::store::SETTLING);
    reconcile::moved(&c.manifest, &c.state, &c.store, &mut found, |_| None);
    let reconcile::Unlisted {
        ready, settling, ..
    } = found;
    let mut new = Group::default();
    for key in ready {
        new.add(&key.to_string());
    }
    for path in settling {
        new.add(&SourceKey::Manual(path.into()).to_string());
    }
    group(out, c.verbose, "Dropped in, not listed", &new)?;
    let (mut unused, mut strays) = (Group::default(), Group::default());
    let mut known = listed.clone();
    known.extend(c.manifest.removed_keys());
    for path in &c.store.purgeable(c.dirs, &known, &c.state.copies) {
        if c.store.leftovers().contains(path) {
            strays.add(&path.display().to_string());
        } else {
            unused.add(&path.display().to_string());
        }
    }
    group(out, c.verbose, "Fetched, not listed", &unused)?;
    group(out, c.verbose, "Strays in sources", &strays)?;
    group(out, c.verbose, "Not muman's", &foreign(c)?)?;
    if let Some(age) = crate::state::written_at(&c.dirs.home)
        .and_then(|t| SystemTime::now().duration_since(t).ok())
    {
        row(
            out,
            "Last written",
            &format!("{} ago", duration(age.as_secs_f64())),
        )?;
    }

    Ok(())
}

/// The files in the library muman did not write.
fn foreign(c: &Context<'_>) -> Result<Group> {
    let mut foreign = Group::default();
    let ours: BTreeSet<&Path> = c
        .state
        .outputs
        .iter()
        .flat_map(|(p, w)| w.files(p))
        .collect();
    for file in store::walk(&c.dirs.library, usize::MAX)? {
        if let Ok(rel) = file.strip_prefix(&c.dirs.library)
            && !ours.contains(rel)
        {
            foreign.add(&crate::relpath::show(rel));
        }
    }
    Ok(foreign)
}

fn health<W: Write>(c: &Context<'_>, out: &mut W) -> Result<()> {
    heading(out, "Health")?;
    let health = Health::of(&c.manifest, &c.planned, &c.state.facts, &|n, r| {
        c.name(n, r)
    });
    group(out, c.verbose, "Damaged audio", &health.damaged)?;
    group(out, c.verbose, "Audio under 16 kHz", &health.narrow)?;
    group(out, c.verbose, "Mono audio", &health.mono)?;
    group(out, c.verbose, "Clipped audio", &health.clipped)?;
    group(out, c.verbose, "Audio not measured", &health.unmeasured)?;
    group(out, c.verbose, "No cover", &health.no_cover)?;
    group(out, c.verbose, "Cover not square", &health.not_square)?;
    group(out, c.verbose, "Cover under 500 px", &health.soft)?;
    group(out, c.verbose, "No lyrics", &health.no_lyrics)?;
    group(out, c.verbose, "Lyrics untimed", &health.untimed)?;
    group(out, c.verbose, "No artist or title", &health.untagged)?;
    group(out, c.verbose, "No date", &health.undated)?;
    Ok(())
}

/// What could be better in the songs as they would be written.
#[derive(Default)]
struct Health {
    /// Audio cut off or missing some, as [`crate::integrity`] judges it.
    damaged: Group,
    narrow: Group,
    mono: Group,
    clipped: Group,
    unmeasured: Group,
    no_cover: Group,
    not_square: Group,
    soft: Group,
    no_lyrics: Group,
    untimed: Group,
    untagged: Group,
    undated: Group,
}

impl Health {
    fn of(
        manifest: &Manifest,
        planned: &[(usize, Resolved)],
        facts: &BTreeMap<SourceKey, Facts>,
        name: &dyn Fn(usize, &Resolved) -> String,
    ) -> Self {
        let mut h = Self::default();
        for (n, r) in planned {
            let name = name(*n, r);
            let plan = &r.plan;
            if facts
                .get(&plan.audio.key)
                .is_some_and(|f| f.damage().is_some())
            {
                h.damaged.add(&name);
            }
            match facts
                .get(&plan.audio.key)
                .and_then(|f| f.audio.as_ref()?.quality.as_ref())
            {
                Some(q) => {
                    if q.bandwidth_hz < NARROW_HZ {
                        h.narrow.add(&name);
                    }
                    if !q.is_stereo(manifest.settings.audio.pick.stereo.incoherence) {
                        h.mono.add(&name);
                    }
                    if q.clipping > CLIPPED {
                        h.clipped.add(&name);
                    }
                }
                None => h.unmeasured.add(&name),
            }
            match &plan.cover {
                // A song list that writes no cover misses none.
                None if manifest.settings.cover.out.is_empty() => {}
                None => h.no_cover.add(&name),
                Some(c) => {
                    let quality = facts
                        .get(&c.key)
                        .and_then(|f| f.covers.iter().find(|x| x.at == c.at))
                        .and_then(|x| x.quality.as_ref());
                    if let Some(q) = quality {
                        if !q.is_square(manifest.settings.cover.pick.square.tolerance) {
                            h.not_square.add(&name);
                        }
                        if q.effective < SOFT_COVER {
                            h.soft.add(&name);
                        }
                    }
                }
            }
            let off = manifest.songs[*n].lyrics == Some(LyricsPin::None)
                || manifest.settings.lyrics.out.is_empty();
            match &plan.lyrics {
                None if !off => h.no_lyrics.add(&name),
                None => {}
                Some(l) => {
                    let timed = facts
                        .get(&l.key)
                        .and_then(|f| f.lyrics.iter().find(|x| x.at == l.at))
                        .is_some_and(|x| x.timing.is_some());
                    if !timed {
                        h.untimed.add(&name);
                    }
                }
            }
            let has = |f: Field| plan.tags.iter().any(|(k, _)| k == f.vorbis());
            if !has(Field::Artist) || !has(Field::Title) {
                h.untagged.add(&name);
            }
            if !has(Field::Date) {
                h.undated.add(&name);
            }
        }
        h
    }
}

fn heading<W: Write>(out: &mut W, title: &str) -> Result<()> {
    crate::ui::painted(out, &[Style::Accent.paint(title)])?;
    Ok(())
}

fn row<W: Write>(out: &mut W, label: &str, value: &str) -> Result<()> {
    painted_row(out, label, Style::Plain.paint(value))
}

fn painted_row<W: Write>(out: &mut W, label: &str, value: Painted<'_>) -> Result<()> {
    let label = format!("  {label:<LABEL$} ");
    crate::ui::painted(out, &[Style::Plain.paint(&label), value])?;
    Ok(())
}

/// A count, painted as a warning when not zero, and each member below
/// it when `verbose`.
fn group<W: Write>(out: &mut W, verbose: bool, label: &str, g: &Group) -> Result<()> {
    let n = g.len().to_string();
    let style = if g.len() == 0 {
        Style::Plain
    } else {
        Style::Warning
    };
    painted_row(out, label, style.paint(&n))?;
    if verbose {
        for member in &g.0 {
            crate::ui::painted(
                out,
                &[Style::Plain.paint("      "), Style::Muted.paint(member)],
            )?;
        }
    }
    Ok(())
}

/// Every file's size under `dir`, hidden and unfinished ones too.
fn folder_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

/// Seconds as days, hours and minutes, the two largest that apply.
fn duration(seconds: f64) -> String {
    let minutes = (seconds / 60.0).round() as u64;
    let (d, h, m) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    match (d, h) {
        (0, 0) => format!("{m} min"),
        (0, _) => format!("{h} h {m} min"),
        _ => format!("{d} d {h} h"),
    }
}

#[cfg(test)]
mod tests;
