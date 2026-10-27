//! Every song resolved to its plan, each with a path of its own, and how
//! a song is named in messages.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::clean::Albums;
use crate::facts::Facts;
use crate::manifest::{Manifest, Song};
use crate::naming::{self, Naming};
use crate::resolve::{self, Input, Resolved};
use crate::source::SourceKey;
use crate::state::State;
use crate::tags::Field;

/// How a song is named in messages: its title and artist as resolved,
/// else its first source.
pub(crate) fn name_of(
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

pub(crate) fn path_of(r: &Resolved) -> PathBuf {
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
pub(crate) type Planned = Vec<(usize, Resolved)>;

/// One song of [`Planned`].
pub(super) type PlannedSong = (usize, Resolved);

/// Resolve every song, saying which could not be and returning their
/// sources with the plans, each song with a path of its own.
pub(crate) fn plan<W: Write>(
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
pub(crate) fn plan_with<W: Write>(
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
        failed.extend(song.sources.iter().cloned());
        let name = name_of(song, None, &state.facts);
        crate::ui::error(out, &format!("Failed: {name}: {e:#}"))?;
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

/// What resolving every song found, for a caller to say or count.
pub(crate) struct Resolution {
    pub planned: Planned,
    /// Each song that could not be resolved, by its place, and why.
    pub unresolved: Vec<(usize, anyhow::Error)>,
    /// Each song that could not be resolved while a source of it waits
    /// to be fetched, by its place.
    pub awaiting: Vec<usize>,
    /// How many songs' paths take more than `[library] max_path`.
    pub over_max_path: usize,
}

/// What resolving a song reads beside the song: the song list, the
/// settings, what was measured, the names a path takes and the albums
/// the sources offer, made once for many songs. A song need not be
/// listed, so one made in memory, as songs merged, resolves as the sync
/// after would resolve it.
pub(crate) struct Resolver<'a> {
    manifest: &'a Manifest,
    settings: &'a crate::settings::Settings,
    state: &'a State,
    naming: Naming,
    albums: Albums,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(
        manifest: &'a Manifest,
        settings: &'a crate::settings::Settings,
        state: &'a State,
        library: &Path,
    ) -> Result<Self> {
        Ok(Self {
            manifest,
            settings,
            state,
            naming: Naming::new(&settings.library, library)?
                .with_tail(crate::outputs::tail(settings)),
            albums: Albums::of(state.facts.values().map(|f| &f.tags)),
        })
    }

    /// What resolving `song` reads, with the sources `on_disk` when the
    /// store was looked at.
    pub(crate) fn input<'s>(
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
            clean: &self.manifest.clean,
            albums: &self.albums,
            naming: &self.naming,
            audio: &settings.audio,
            cover: &settings.cover.pick,
            source_comments: settings.library.source_comments,
        }
    }

    /// `song` resolved, listed or not.
    pub(crate) fn resolve(&self, song: &Song) -> Result<Resolved> {
        resolve::resolve(&self.input(song, None))
    }
}

/// Resolve every song under `settings`, each with a path of its own,
/// saying nothing.
pub(crate) fn resolve_songs(
    manifest: &Manifest,
    settings: &crate::settings::Settings,
    state: &State,
    library: &Path,
    on_disk: Option<&BTreeSet<SourceKey>>,
    waiting: &BTreeSet<SourceKey>,
) -> Result<Resolution> {
    let resolver = Resolver::new(manifest, settings, state, library)?;
    let mut resolution = Resolution {
        planned: Vec::new(),
        unresolved: Vec::new(),
        awaiting: Vec::new(),
        over_max_path: 0,
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
    if let Some(max) = settings.library.max_path {
        resolution.over_max_path = (resolution.planned.iter())
            .filter(|(_, r)| naming::units(&library.join(path_of(r)).to_string_lossy()) > max)
            .count();
    }
    crate::loudness::settle(&mut resolution.planned, state, settings);
    separate(
        &mut resolution.planned,
        &resolver.naming,
        &crate::outputs::planned,
    );
    Ok(resolution)
}
