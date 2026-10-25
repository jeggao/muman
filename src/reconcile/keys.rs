//! Sources whose keys change before a sync plans: keys renamed by the
//! site they came from, and manual files followed to where they moved.

use std::collections::BTreeMap;
use std::io::Write;
use std::time::{Duration, SystemTime};

use anyhow::Result;

use crate::manifest::Manifest;
use crate::source::SourceKey;
use crate::state::State;
use crate::store::{self, Store};

/// For a dry run, the manual files the next sync will follow to where
/// they moved, said and followed here in memory, so the run reports on
/// the songs as that sync will find them.
pub(super) fn foresee_moves<W: Write>(
    manifest: &mut Manifest,
    state: &mut State,
    store: &Store,
    settling: Duration,
    out: &mut W,
) -> Result<()> {
    let listed = manifest.keys();
    let mut known = listed.clone();
    known.extend(manifest.removed_keys());
    let (mut ready, _) = store.unlisted(&known, SystemTime::now(), settling);
    let moved = store::moved_manual(&listed, state, store, &mut ready);
    for (from, to) in &moved {
        crate::ui::info(
            out,
            &format!("Moved, followed by the next sync: {from} is now {to}"),
        )?;
        for song in &mut manifest.songs {
            song.rename(from, to);
        }
    }
    if !moved.is_empty() {
        state.rekey(&moved)?;
    }
    Ok(())
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
