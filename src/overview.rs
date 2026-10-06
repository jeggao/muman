//! `muman info`: what the library holds, whether it is in step with
//! the song list, and what in it could be better. Read from the song
//! list, the state and the folders alone: nothing is measured, run,
//! fetched or locked, so it answers at once and changes nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::ui::Style;
use anyhow::Result;

use crate::codec::Codec;
use crate::dirs::{Dirs, STATE};
use crate::facts::Facts;
use crate::lookup;
use crate::manifest::{LyricsPin, Manifest};
use crate::reconcile;
use crate::resolve::{Resolved, SINGLE};
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
/// A cover whose detail holds up to fewer pixels than this looks soft
/// on a large screen.
const SOFT_COVER: u32 = 500;
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
    /// What planning said of each song it could not plan.
    failures: Vec<u8>,
}

impl Context<'_> {
    fn name(&self, n: usize, r: &Resolved) -> String {
        reconcile::name_of(&self.manifest.songs[n], Some(r), &self.state.facts)
    }
}

/// Write the library's statistics, its status against the song list,
/// and its health, every song behind a count when `verbose`.
pub fn info<W: Write>(dirs: &Dirs, verbose: bool, out: &mut W) -> Result<()> {
    let manifest = Manifest::load(&dirs.home)?;
    let state = State::load(&dirs.home)?;
    let store = Store::scan(dirs)?;
    let mut failures = Vec::new();
    let (mut planned, _) = reconcile::plan(&manifest, &state, &dirs.library, None, &mut failures)?;
    crate::limit::as_written(&manifest, &state, &mut planned);
    let c = Context {
        dirs,
        verbose,
        manifest,
        state,
        store,
        planned,
        failures,
    };
    folders(&c, out)?;
    library(&c, out)?;
    sources(&c, out)?;
    status(&c, out)?;
    health(&c, out)
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
    let singles = c
        .planned
        .iter()
        .filter(|(_, r)| r.why.tags.iter().any(|t| t.from == SINGLE))
        .count();
    let albums: BTreeSet<(Option<String>, Option<String>)> = c
        .planned
        .iter()
        .filter(|(_, r)| !r.why.tags.iter().any(|t| t.from == SINGLE))
        .map(|(_, r)| (tag(r, Field::AlbumArtist), tag(r, Field::Album)))
        .collect();
    let artists: BTreeSet<Option<String>> = c
        .planned
        .iter()
        .map(|(_, r)| tag(r, Field::AlbumArtist))
        .collect();
    row(
        out,
        "Songs",
        &format!(
            "{} ({} on {} albums, {singles} singles)",
            c.manifest.songs.len(),
            c.planned.len() - singles,
            albums.len()
        ),
    )?;
    row(out, "Album artists", &artists.len().to_string())?;
    let seconds: f64 = c
        .planned
        .iter()
        .filter_map(|(_, r)| c.state.facts.get(&r.plan.audio.key)?.duration)
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
    let written: u64 = c
        .state
        .outputs
        .iter()
        .flat_map(|(p, w)| std::iter::once(p).chain(&w.lyrics))
        .filter_map(|p| store::stamp(&c.dirs.library.join(p)).map(|(size, _)| size))
        .sum();
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
    row(
        out,
        "Listed",
        &format!(
            "{} ({} fetched by yt-dlp, {kept} kept from lookups, {manual} manual)",
            listed.len(),
            listed.len() - kept - manual
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
            "{} fetched by yt-dlp, {} from lookups, {} manual",
            crate::ui::bytes(folder_size(&c.dirs.ytdlp())),
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

#[allow(clippy::too_many_lines)]
fn status<W: Write>(c: &Context<'_>, out: &mut W) -> Result<()> {
    let listed = c.manifest.keys();
    heading(out, "Status")?;
    if let Some(before) = c.state.library.as_deref().filter(|l| *l != c.dirs.library) {
        crate::ui::warning(
            out,
            &format!(
                "  Written to {} until now; the next sync writes every song here anew",
                before.display()
            ),
        )?;
    }
    let mut made: BTreeSet<PathBuf> = BTreeSet::new();
    let (mut current, mut due) = (0, Group::default());
    for (n, r) in &c.planned {
        let path = reconcile::path_of(r);
        let fresh = c.state.outputs.get(&path).is_some_and(|w| {
            w.plan.as_ref() == Some(&r.plan) && c.dirs.library.join(&path).exists()
        });
        if fresh {
            current += 1;
        } else {
            due.add(&c.name(*n, r));
        }
        made.insert(path);
    }
    let mut gone = Group::default();
    for path in c.state.outputs.keys().filter(|p| !made.contains(*p)) {
        gone.add(&crate::relpath::show(path));
    }
    row(out, "Up to date", &current.to_string())?;
    group(out, c.verbose, "To write", &due)?;
    group(out, c.verbose, "To remove", &gone)?;
    let mut unplanned = Group::default();
    for line in String::from_utf8_lossy(&c.failures).lines() {
        unplanned.add(line.trim_start_matches("Failed: "));
    }
    group(out, c.verbose, "Cannot be made", &unplanned)?;
    for p in lookup::ORDER {
        let mut wanted = Group::default();
        let due = lookup::due(&c.manifest, &c.state, &c.planned, state::now_secs(), false);
        for d in due.iter().filter(|d| d.find == p) {
            wanted.add(&reconcile::name_of(
                &c.manifest.songs[d.song],
                None,
                &c.state.facts,
            ));
        }
        group(out, c.verbose, &format!("Look up: {p}"), &wanted)?;
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
                    .is_some_and(|f| f.holds_for(&l.rev()) && f.tags_hold());
                if !holds {
                    to_measure.add(&key.to_string());
                }
            }
        }
    }
    group(out, c.verbose, "Sources missing", &missing)?;
    group(out, c.verbose, "Sources to measure", &to_measure)?;
    let (unlisted, settling) = c
        .store
        .unlisted(&listed, SystemTime::now(), crate::store::SETTLING);
    let mut new = Group::default();
    for key in unlisted {
        new.add(&key.to_string());
    }
    for path in settling {
        new.add(&SourceKey::Manual(path).to_string());
    }
    group(out, c.verbose, "Dropped in, not listed", &new)?;
    let mut unused = Group::default();
    for path in c
        .store
        .unused(&listed)
        .iter()
        .filter(|p| !p.starts_with(c.dirs.manual()))
    {
        unused.add(&path.display().to_string());
    }
    group(out, c.verbose, "Fetched, not listed", &unused)?;
    group(out, c.verbose, "Not muman's", &foreign(c)?)?;
    if let Some(age) = std::fs::metadata(c.dirs.home.join(STATE))
        .and_then(|m| m.modified())
        .ok()
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
        .flat_map(|(p, w)| std::iter::once(p.as_path()).chain(w.lyrics.as_deref()))
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
            match facts
                .get(&plan.audio.key)
                .and_then(|f| f.audio.as_ref()?.quality.as_ref())
            {
                Some(q) => {
                    if q.bandwidth_hz < NARROW_HZ {
                        h.narrow.add(&name);
                    }
                    if !q.is_stereo(manifest.settings.quality.stereo.incoherence) {
                        h.mono.add(&name);
                    }
                    if q.clipping > CLIPPED {
                        h.clipped.add(&name);
                    }
                }
                None => h.unmeasured.add(&name),
            }
            match &plan.cover {
                None => h.no_cover.add(&name),
                Some(c) => {
                    let quality = facts
                        .get(&c.key)
                        .and_then(|f| f.covers.iter().find(|x| x.at == c.at))
                        .and_then(|x| x.quality.as_ref());
                    if let Some(q) = quality {
                        if !q.is_square(manifest.settings.quality.square.tolerance) {
                            h.not_square.add(&name);
                        }
                        if q.effective < SOFT_COVER {
                            h.soft.add(&name);
                        }
                    }
                }
            }
            let off = manifest.songs[*n].lyrics == Some(LyricsPin::None);
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
    writeln!(out, "{}", Style::Accent.paint(title))?;
    Ok(())
}

fn row<W: Write>(out: &mut W, label: &str, value: &str) -> Result<()> {
    writeln!(out, "  {label:<LABEL$} {value}")?;
    Ok(())
}

/// A count, painted as a warning when not zero, and each member below
/// it when `verbose`.
fn group<W: Write>(out: &mut W, verbose: bool, label: &str, g: &Group) -> Result<()> {
    let n = g.len().to_string();
    let n = if g.len() == 0 {
        n
    } else {
        Style::Warning.paint(&n).to_string()
    };
    row(out, label, &n)?;
    if verbose {
        for member in &g.0 {
            writeln!(out, "      {}", Style::Muted.paint(member))?;
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
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
