//! Sources whose keys change before a sync plans: keys renamed by the
//! site they came from, and manual files followed to where they moved.
//!
//! A key a site renames keeps its fetched file through the rename: the
//! file takes a second name at the new key's place, then the state and
//! then the song list name the new key, and only then does the old name
//! go. A run stopped between any two finds the file under the key the
//! list names, so it is not fetched again; stopped after the state, it
//! measures that one source again, and after the list, it leaves the
//! old name, which `purge` lists. A filesystem that makes no second
//! name, as FAT, moves the file after the state instead.
//!
//! A listed manual file gone from its path is followed by every run that
//! brings the library in line, so no command fails its song for a file
//! the user only moved, and an undo, which puts back the song list a
//! move was followed in but not the user's file, has it followed again.
//! Beside a move [`store::moved_manual`] finds by the file's stamp or
//! bytes, a settled song file no song lists that holds the same audio,
//! by the digest of its packets, is the file moved: a tagger that renames
//! files as it retags them leaves neither stamp nor bytes, and a file
//! whose record went with its new key, as an undo leaves it, none to
//! compare. The gone file's audio is what its song records it held, else
//! what measuring last read; the other file's, what measuring kept of it
//! at its revision, else read then, only while a listed file is gone.
//! Audio two files share they share by their packets, which no retag
//! touches; one recording in two files of another encoding is two audios.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Result;

use crate::manifest::{Edit, Manifest};
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::State;
use crate::store::{self, Kind, Located, Store};

/// The manual files neither a song nor a removed song lists, as
/// [`Store::unlisted`] splits them, but that a file once listed is no new
/// one still copying in, however lately it was touched.
#[derive(Debug, Clone, Default)]
pub struct Unlisted {
    /// Those settled, read as they are.
    pub ready: Vec<SourceKey>,
    /// Those arrived too lately to be read yet.
    pub settling: Vec<PathBuf>,
}

impl Unlisted {
    #[must_use]
    pub fn of(manifest: &Manifest, state: &State, store: &Store, wait: Duration) -> Self {
        let mut known = manifest.keys();
        known.extend(manifest.removed_keys());
        let (mut ready, mut settling) = store.unlisted(&known, SystemTime::now(), wait);
        settling.retain(|path| {
            let key = SourceKey::Manual(path.into());
            let was_listed = state.listed.contains(&key);
            if was_listed {
                ready.push(key);
            }
            !was_listed
        });
        Self { ready, settling }
    }
}

/// Each listed manual file gone from its path, by the file of `unlisted`
/// that is it moved, taken out of its list: by [`store::moved_manual`],
/// else a settled song file holding its audio, as the module says. What
/// a file holds is read from what measuring kept of it, else by `hear`.
pub fn moved(
    manifest: &Manifest,
    state: &State,
    store: &Store,
    unlisted: &mut Unlisted,
    mut hear: impl FnMut(&Located) -> Option<String>,
) -> BTreeMap<SourceKey, SourceKey> {
    let listed = manifest.keys();
    let mut moved = store::moved_manual(
        &listed,
        |k| state.recorded(k),
        store,
        &mut unlisted.ready,
        &mut unlisted.settling,
    );
    let song = |k: &SourceKey| matches!(k, SourceKey::Manual(m) if store::kind_of(m.path()) == Some(Kind::Media));
    let gone: Vec<(&SourceKey, String)> = listed
        .iter()
        .filter(|k| song(k) && !store.has(k) && !moved.contains_key(*k))
        .filter_map(|k| Some((k, audio_listed(manifest, state, k)?)))
        .collect();
    let mut heard: BTreeMap<SourceKey, Option<String>> = BTreeMap::new();
    for (gone, was) in gone {
        let at = unlisted.ready.iter().position(|k| {
            let audio = heard.entry(k.clone()).or_insert_with(|| {
                let at = store.locate(k)?;
                (state.facts.get(k))
                    .filter(|f| f.holds_for(&at.rev()) && f.hashed())
                    .and_then(|f| f.audio.as_ref()?.digest.clone())
                    .or_else(|| hear(&at))
            });
            audio.as_deref().is_some_and(|a| same_audio(a, &was))
        });
        if let Some(at) = at {
            moved.insert(gone.clone(), unlisted.ready.remove(at));
        }
    }
    moved
}

/// The audio a listed source held: as its song records it, else as
/// measuring last read it.
fn audio_listed(manifest: &Manifest, state: &State, key: &SourceKey) -> Option<String> {
    (manifest.song_with(key))
        .and_then(|s| s.held.get(key)?.audio.clone())
        .or_else(|| state.facts.get(key)?.audio.as_ref()?.digest.clone())
}

/// Whether two audio digests, cut to the song list's length, agree.
fn same_audio(a: &str, b: &str) -> bool {
    let n = crate::held::DIGITS;
    a.len() >= n && b.len() >= n && a[..n] == b[..n]
}

/// [`moved`]'s `hear`: the audio a song file holds, read by `runner`,
/// its files written below `scratch`.
pub fn hearing<'a, R: Runner>(
    runner: &'a R,
    scratch: &'a Path,
) -> impl FnMut(&Located) -> Option<String> + 'a {
    move |at| {
        let sub = super::Sub::new(scratch, "heard-").ok()?;
        crate::facts::hash(runner, at, sub.path()).ok()?.0.audio
    }
}

/// How [`Following::follow`] keeps what it follows.
pub(super) struct Following<'a> {
    pub home: &'a Path,
    pub lock: &'a crate::atomic::Lock,
    pub run: Option<&'a mut crate::history::Run>,
    pub dry_run: bool,
    pub settling: Duration,
}

impl Following<'_> {
    /// Follow each listed manual file to where it moved ([`moved`]), in
    /// the song list and the state, saved, the list noted in `run`; a dry
    /// run follows them in memory alone, so it reports on the songs as
    /// the next sync will find them.
    pub(super) fn follow<W: Write>(
        self,
        manifest: &mut Manifest,
        state: &mut State,
        store: &Store,
        hear: impl FnMut(&Located) -> Option<String>,
        out: &mut W,
    ) -> Result<()> {
        let mut unlisted = Unlisted::of(manifest, state, store, self.settling);
        let moved = moved(manifest, state, store, &mut unlisted, hear);
        if moved.is_empty() {
            return Ok(());
        }
        for (from, to) in &moved {
            if self.dry_run {
                crate::ui::info(
                    out,
                    &format!("Moved, followed by the next sync: {from} is now {to}"),
                )?;
                for song in &mut manifest.songs {
                    song.rename(from, to);
                }
            } else {
                crate::ui::info(out, &format!("Moved: {from} is now {to}"))?;
                manifest.edit(Edit::Rename {
                    from: from.clone(),
                    to: to.clone(),
                });
            }
        }
        if !self.dry_run {
            // The list before the state: stopped between, the next run
            // measures the file again, where the other way it would find
            // no record of the file gone to follow it by.
            manifest.save_locked(self.lock)?;
            if let Some(run) = self.run {
                run.saw(manifest);
            }
        }
        state.rekey(&moved)?;
        if !self.dry_run {
            state.save(self.home)?;
        }
        Ok(())
    }
}

/// Each listed manual file gone while a file no song lists is still
/// arriving, which may be it, moved by a copy that has not ended: its
/// song waits for it, as one waits for a fetch, rather than fail.
pub(crate) fn arriving(
    manifest: &Manifest,
    state: &State,
    store: &Store,
    settling: Duration,
) -> BTreeSet<SourceKey> {
    if Unlisted::of(manifest, state, store, settling)
        .settling
        .is_empty()
    {
        return BTreeSet::new();
    }
    manifest
        .keys()
        .into_iter()
        .filter(|k| matches!(k, SourceKey::Manual(_)) && !store.has(k))
        .collect()
}

/// Each listed source the song list's `[sites]` names otherwise, by the
/// key a fetch now names it: one of an extractor's name, as keys were
/// once named, by the site listing it, else by the site of the page its
/// facts or its song's record recorded; one of a domain a site lists as
/// an alias by that site. One no other song lists nor another source
/// takes. A source of yt-dlp's generic extractor keeps its name: what
/// it recorded is the mirror a redirect ended at, not the address it was
/// fetched by.
pub(super) fn sites_found(manifest: &Manifest, state: &State) -> BTreeMap<SourceKey, SourceKey> {
    let listed = manifest.keys();
    let sites = &manifest.settings.sites;
    let mut renames = BTreeMap::new();
    for song in &manifest.songs {
        for key in song.sources.iter().filter(|k| k.site() != Some("generic")) {
            let page = state
                .facts
                .get(key)
                .and_then(|f| f.served.as_ref()?.url.clone())
                .or_else(|| song.held.get(key)?.url.clone());
            let by_page = || {
                let (extractor, id) = key.site().zip(key.id())?;
                sites.fetched(extractor, page.as_deref(), id)
            };
            let to = sites
                .renamed(key)
                .or_else(|| key.is_old().then(by_page).flatten());
            if let Some(to) =
                to.filter(|t| t != key && !listed.contains(t) && !renames.values().any(|v| v == t))
            {
                renames.insert(key.clone(), to);
            }
        }
    }
    renames
}
