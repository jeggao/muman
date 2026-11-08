//! Every song resolved to its plan, each with a path of its own, and how
//! a song is named in messages.
//!
//! A song that fails to resolve, its file still in the library, is held
//! at the plan that file was made from: no song takes its path, and its
//! album's loudness is pooled with it as it was. What it failed with is
//! its own sources, never a picture or a record it shares, so no other
//! song's file is kept for it.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::facts::Facts;
use crate::manifest::{Manifest, Song};
use crate::naming::{self, Naming};
use crate::resolve::{self, Format, Input, Known, Resolved, Why};
use crate::source::SourceKey;
use crate::state::State;
use crate::tags::Field;

/// How a song is named in messages: its title and artist as resolved,
/// else its first source.
#[must_use]
pub fn name_of(
    song: &Song,
    resolved: Option<&Resolved>,
    facts: &BTreeMap<SourceKey, Facts>,
) -> String {
    let from_tags = |tags: &[(String, Vec<String>)]| {
        let get = |f: Field| {
            tags.iter()
                .find(|(k, _)| k == f.vorbis())
                .and_then(|(_, v)| v.first())
                .cloned()
        };
        get(Field::Title).map(|t| match get(Field::Artist) {
            Some(a) => format!("{t} — {a}"),
            None => t,
        })
    };
    resolved
        .and_then(|r| from_tags(&r.plan.tags))
        .or_else(|| {
            let first = song.sources.first()?;
            let offers = &facts.get(first)?.tags;
            let title = offers.get(&Field::Title)?.values.first()?.clone();
            Some(title)
        })
        .or_else(|| song.sources.first().map(ToString::to_string))
        .unwrap_or_else(|| "a song with no sources".to_string())
}

#[must_use]
pub fn path_of(r: &Resolved) -> PathBuf {
    let mut name = r.stem.as_os_str().to_os_string();
    name.push(".");
    name.push(r.plan.format.extension());
    PathBuf::from(name)
}

/// Give every song a path of its own: a later song resolving to one an
/// earlier song took is named with its audio's ID too, and numbered
/// when that is taken as well, as two manual files of one name are. The
/// names are the song list's alone, never how the library came to be,
/// so one list builds one library. Each name is held as a filesystem
/// blind to case folds it, by the song's stem, so a FLAC song and an Opus
/// song of one name are told apart before either writes a `.lrc`, and by
/// each file a song may write, so `Intro` and its `.en.lrc` never meet
/// `Intro.en` and its `.lrc`. `beside` names the files a song at a path
/// writes beside it by its plan.
pub(super) fn separate(
    planned: &mut [(usize, Resolved)],
    failed: &[(usize, Resolved)],
    naming: &Naming,
    beside: &dyn Fn(&Path, &resolve::Plan) -> Vec<PathBuf>,
) {
    use crate::relpath::folded;
    let files = |r: &Resolved| -> Vec<String> {
        let path = path_of(r);
        let names = beside(&path, &r.plan);
        std::iter::once(path)
            .chain(names)
            .map(|p| folded(&p))
            .collect()
    };
    let mut stems: BTreeSet<String> = BTreeSet::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut hold = |r: &Resolved, stems: &mut BTreeSet<String>| {
        let files = files(r);
        let free = !stems.contains(&folded(&r.stem)) && files.iter().all(|f| !taken.contains(f));
        if free {
            stems.insert(folded(&r.stem));
            taken.extend(files);
        }
        free
    };
    // A song that failed keeps the name its file has, so no song takes
    // its path, and its file, while it is held.
    for (_, r) in failed {
        hold(r, &mut stems);
    }
    for (_, r) in planned.iter_mut() {
        if hold(r, &mut stems) {
            continue;
        }
        let base = r.stem.clone();
        // A manual file's name may hold what a path must not.
        let id = naming.value(&r.plan.audio.key.short());
        for n in 1.. {
            r.stem = naming.suffixed(&base, &id, n);
            if hold(r, &mut stems) {
                break;
            }
        }
    }
}

/// Each song's place in the song list, and what it resolved to.
pub type Planned = Vec<(usize, Resolved)>;

/// One song of [`Planned`].
pub(super) type PlannedSong = (usize, Resolved);

/// Resolve every song, saying which could not be and returning their
/// sources with the plans, each song with a path of its own.
pub fn plan<W: Write>(
    manifest: &Manifest,
    state: &State,
    library: &Path,
    on_disk: Option<&BTreeSet<SourceKey>>,
    out: &mut W,
) -> Result<(Planned, BTreeSet<SourceKey>)> {
    plan_awaiting(manifest, state, library, on_disk, &BTreeSet::new(), out)
}

/// [`plan`], a song that cannot be resolved while a source of it is
/// `waiting` to be fetched said to wait, and not one that failed.
pub(super) fn plan_awaiting<W: Write>(
    manifest: &Manifest,
    state: &State,
    library: &Path,
    on_disk: Option<&BTreeSet<SourceKey>>,
    waiting: &BTreeSet<SourceKey>,
    out: &mut W,
) -> Result<(Planned, BTreeSet<SourceKey>)> {
    for key in &manifest.clean.unknown {
        crate::ui::warning(
            out,
            &format!("No cleaning rule is named `{key}` in the song list; it does nothing"),
        )?;
    }
    for line in crate::migrate::notices(&manifest.renamed, manifest.respelled_notice()) {
        crate::ui::info(out, &line)?;
    }
    for line in crate::settings::stale_warnings(&manifest.stale_defaults)
        .into_iter()
        .chain(manifest.misspelled())
    {
        crate::ui::warning(out, &line)?;
    }
    resolve_all(
        manifest,
        &manifest.settings,
        state,
        library,
        on_disk,
        waiting,
        out,
    )
}

/// [`plan`] under `settings` rather than the song list's, saying only
/// which songs could not be resolved and which paths are too long.
pub fn plan_with<W: Write>(
    manifest: &Manifest,
    settings: &crate::settings::Settings,
    state: &State,
    library: &Path,
    on_disk: Option<&BTreeSet<SourceKey>>,
    out: &mut W,
) -> Result<(Planned, BTreeSet<SourceKey>)> {
    resolve_all(
        manifest,
        settings,
        state,
        library,
        on_disk,
        &BTreeSet::new(),
        out,
    )
}

pub(super) fn resolve_all<W: Write>(
    manifest: &Manifest,
    settings: &crate::settings::Settings,
    state: &State,
    library: &Path,
    on_disk: Option<&BTreeSet<SourceKey>>,
    waiting: &BTreeSet<SourceKey>,
    out: &mut W,
) -> Result<(Planned, BTreeSet<SourceKey>)> {
    let resolution = resolve_songs(manifest, settings, state, library, on_disk, waiting)?;
    for &n in &resolution.awaiting {
        let name = name_of(&manifest.songs[n], None, &state.facts);
        crate::ui::info(out, &format!("Made once a sync fetches it: {name}"))?;
    }
    let mut failed = BTreeSet::new();
    for (n, e) in &resolution.unresolved {
        let song = &manifest.songs[*n];
        // A picture or record other songs share keeps no file of theirs.
        failed.extend(
            (song.sources.iter())
                .filter(|k| !crate::manifest::shareable(k))
                .cloned(),
        );
        let name = name_of(song, None, &state.facts);
        crate::ui::error(out, &format!("Failed: {name}: {e:#}"))?;
    }
    for line in crate::folder::said(&resolution.unplaced) {
        crate::ui::info(out, &line)?;
    }
    if let (Some(max), over @ 1..) = (settings.library.max_path, resolution.over_max_path) {
        crate::ui::warning(
            out,
            &format!(
                "{over} song(s) take more than [library] max_path = {max} characters even \
                 with their names cut short: the library folder, {}, takes {} itself",
                library.display(),
                naming::units(&library.to_string_lossy())
            ),
        )?;
    }
    Ok((resolution.planned, failed))
}

/// Each song of `unresolved` with a file the state records as made from
/// a plan: held at that plan while it fails, as its file stays in the
/// library, so the songs beside it take neither its path nor its place
/// in its album's loudness.
fn held(
    manifest: &Manifest,
    state: &State,
    unresolved: &[(usize, anyhow::Error)],
) -> Vec<(usize, Resolved)> {
    let files = super::files_of(unresolved.iter().map(|(n, _)| *n), manifest, &state.outputs);
    files
        .into_iter()
        .filter_map(|(n, path)| {
            let plan = state.outputs.get(&path)?.plan.clone()?;
            let stem = path.with_extension("");
            Some((
                n,
                Resolved {
                    plan,
                    stem,
                    why: resolve::Why::default(),
                },
            ))
        })
        .collect()
}

/// What resolving every song found, for a caller to say or count.
#[derive(Debug)]
pub struct Resolution {
    pub planned: Planned,
    /// Each song that could not be resolved, by its place, and why.
    pub unresolved: Vec<(usize, anyhow::Error)>,
    /// Each song that could not be resolved while a source of it waits
    /// to be fetched, by its place.
    pub awaiting: Vec<usize>,
    /// How many songs' paths take more than `[library] max_path`.
    pub over_max_path: usize,
    /// Each folder's cover file not written, and why.
    pub unplaced: Vec<crate::folder::Unplaced>,
}

/// What resolving a song reads beside the song: the song list, the
/// settings, what was measured, the names a path takes and the albums
/// the sources offer, made once for many songs. A song need not be
/// listed, so one made in memory, as songs merged, resolves as the sync
/// after would resolve it.
#[derive(Debug)]
pub struct Resolver<'a> {
    manifest: &'a Manifest,
    settings: &'a crate::settings::Settings,
    state: &'a State,
    naming: Naming,
    known: Known,
}

impl<'a> Resolver<'a> {
    pub fn new(
        manifest: &'a Manifest,
        settings: &'a crate::settings::Settings,
        state: &'a State,
        library: &Path,
    ) -> Result<Self> {
        Ok(Self {
            manifest,
            settings,
            state,
            naming: settings.library.naming(library)?.with_tail(settings.tail()),
            known: Known::of(manifest, &state.facts),
        })
    }

    /// What resolving `song` reads, with the sources `on_disk` when the
    /// store was looked at.
    pub fn input<'s>(
        &'s self,
        song: &'s Song,
        on_disk: Option<&'s BTreeSet<SourceKey>>,
    ) -> Input<'s> {
        let settings = self.settings;
        Input {
            song,
            album: song.album.as_ref().and_then(|a| self.manifest.album(a)),
            facts: &self.state.facts,
            on_disk,
            alignments: &self.state.alignments,
            lyrics: &settings.lyrics,
            clean: settings.clean.as_ref().unwrap_or(&self.manifest.clean),
            typography: settings.tags.typography,
            believe: settings.tags.believe,
            release: &settings.release,
            known: &self.known,
            naming: &self.naming,
            audio: &settings.audio,
            cover: &settings.cover,
            source_comments: settings.library.source_comments,
            pick: None,
        }
    }

    /// `song` resolved, listed or not.
    pub fn resolve(&self, song: &Song) -> Result<Resolved> {
        resolve::resolve(&self.input(song, None))
    }
}

/// Resolve again each song of an album its songs disagree on, first on the
/// edition most of them share, then with what they agree on
/// ([`crate::album`]).
fn agree_on_albums(
    planned: &mut Planned,
    resolver: &Resolver<'_>,
    manifest: &Manifest,
    state: &State,
    on_disk: Option<&BTreeSet<SourceKey>>,
) {
    let mut picks: BTreeMap<usize, crate::album::Pick> = crate::release::choose(
        planned,
        &manifest.songs,
        &state.facts,
        &|song, record| state.candidates(&song.sources, record),
        &resolver.settings.release,
    )
    .into_iter()
    .map(|(n, release)| {
        let pick = crate::album::Pick {
            release: Some(release),
            ..crate::album::Pick::default()
        };
        (n, pick)
    })
    .collect();
    let again = |planned: &mut Planned,
                 picks: &BTreeMap<usize, crate::album::Pick>,
                 of: &BTreeSet<usize>| {
        for (n, r) in planned.iter_mut().filter(|(n, _)| of.contains(n)) {
            let input = Input {
                pick: picks.get(n),
                ..resolver.input(&manifest.songs[*n], on_disk)
            };
            if let Ok(again) = resolve::resolve(&input) {
                *r = again;
            }
        }
    };
    let released: BTreeSet<usize> = picks.keys().copied().collect();
    again(planned, &picks, &released);
    for (n, edition) in crate::album::editions(planned, &manifest.songs, &state.facts) {
        picks.entry(n).or_default().edition = Some(edition);
    }
    let moved: BTreeSet<usize> = picks
        .iter()
        .filter(|(_, p)| p.edition.is_some())
        .map(|(n, _)| *n)
        .collect();
    again(planned, &picks, &moved);
    let spelled = crate::names::spell(planned, &state.facts, resolver.settings.tags.script);
    let respelled: BTreeSet<usize> = spelled.keys().copied().collect();
    for (n, values) in spelled {
        picks.entry(n).or_default().agreed = values;
    }
    again(planned, &picks, &respelled);
    let agreed = crate::album::agree(planned);
    let settled: BTreeSet<usize> = agreed.keys().copied().collect();
    for (n, values) in agreed {
        let pick = picks.entry(n).or_default();
        pick.agreed
            .retain(|a| !values.iter().any(|v| v.field == a.field));
        pick.agreed.extend(values);
    }
    again(planned, &picks, &settled);
    let consistent = crate::album::consistent(planned, &manifest.songs, &state.facts);
    let swayed: BTreeSet<usize> = consistent.keys().copied().collect();
    for (n, (cover, audio)) in consistent {
        let pick = picks.entry(n).or_default();
        (pick.cover, pick.audio_from) = (cover, audio);
    }
    again(planned, &picks, &swayed);
}

/// Resolve every song under `settings`, each with a path of its own,
/// saying nothing.
pub fn resolve_songs(
    manifest: &Manifest,
    settings: &crate::settings::Settings,
    state: &State,
    library: &Path,
    on_disk: Option<&BTreeSet<SourceKey>>,
    waiting: &BTreeSet<SourceKey>,
) -> Result<Resolution> {
    let fitted = BTreeMap::new();
    resolve_fitted(
        manifest,
        settings,
        state,
        library,
        (on_disk, waiting),
        &fitted,
    )
}

/// Level every song of `planned` again at the format fitting gave it,
/// as resolving levels a song at its best: a song lowered into another
/// layout is levelled by its mix, and its album's songs by their pool
/// with it. A song keeps its path and why it was lowered.
pub(super) fn level_as_fitted(
    manifest: &Manifest,
    state: &State,
    library: &Path,
    found: (Option<&BTreeSet<SourceKey>>, &BTreeSet<SourceKey>),
    planned: &mut Planned,
) -> Result<()> {
    let fitted: BTreeMap<usize, Format> =
        planned.iter().map(|(n, r)| (*n, r.plan.format)).collect();
    let again = resolve_fitted(manifest, &manifest.settings, state, library, found, &fitted)?;
    let mut again: BTreeMap<usize, Resolved> = again.planned.into_iter().collect();
    for (n, r) in planned.iter_mut() {
        if let Some(levelled) = again.remove(n).filter(|a| a.plan.format == r.plan.format) {
            r.plan = levelled.plan;
            r.why = Why {
                fit: r.why.fit.take(),
                ..levelled.why
            };
        }
    }
    Ok(())
}

/// [`resolve_songs`], each song of `fitted` levelled at the format given
/// there rather than at its best.
fn resolve_fitted(
    manifest: &Manifest,
    settings: &crate::settings::Settings,
    state: &State,
    library: &Path,
    (on_disk, waiting): (Option<&BTreeSet<SourceKey>>, &BTreeSet<SourceKey>),
    fitted: &BTreeMap<usize, Format>,
) -> Result<Resolution> {
    let resolver = Resolver::new(manifest, settings, state, library)?;
    let mut resolution = Resolution {
        planned: Vec::new(),
        unresolved: Vec::new(),
        awaiting: Vec::new(),
        over_max_path: 0,
        unplaced: Vec::new(),
    };
    for (n, song) in manifest.songs.iter().enumerate() {
        match resolve::resolve(&resolver.input(song, on_disk)) {
            Ok(r) => resolution.planned.push((n, r)),
            Err(_) if song.sources.iter().any(|k| waiting.contains(k)) => {
                resolution.awaiting.push(n);
            }
            Err(e) => resolution.unresolved.push((n, e)),
        }
    }
    agree_on_albums(&mut resolution.planned, &resolver, manifest, state, on_disk);
    if let Some(max) = settings.library.max_path {
        resolution.over_max_path = (resolution.planned.iter())
            .filter(|(_, r)| naming::units(&library.join(path_of(r)).to_string_lossy()) > max)
            .count();
    }
    let held = held(manifest, state, &resolution.unresolved);
    for (n, r) in &mut resolution.planned {
        if let Some(format) = fitted.get(n) {
            r.plan = r.plan.with_format(*format);
        }
    }
    crate::levels::settle(&mut resolution.planned, &held, state, settings);
    separate(
        &mut resolution.planned,
        &held,
        &resolver.naming,
        &|path, plan| crate::artifact::parts(plan).files(path),
    );
    resolution.unplaced = crate::folder::assign(
        &mut resolution.planned,
        &held,
        &settings.cover,
        &state.facts,
    );
    Ok(resolution)
}
