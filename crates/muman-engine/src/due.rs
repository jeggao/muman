//! What a sync fetches, decided once: the fetches, `status` and `info`
//! all read it, so none says a song is made from a source a sync would
//! not fetch.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use anyhow::Result;

use crate::manifest::Manifest;
use crate::source::SourceKey;
use crate::state;
use crate::store::Store;

/// Why a sync fetches a listed source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetch {
    /// The store holds no file of it.
    Missing,
    /// The store's copy is damaged, and a whole one takes its place.
    Damaged,
}

/// What a sync fetches of the listed sources, and what it would but holds
/// off, as its fetch failed lately.
#[derive(Debug, Default)]
pub struct Due {
    pub now: BTreeMap<SourceKey, Fetch>,
    pub waiting: BTreeSet<SourceKey>,
}

/// What a sync fetches of what `manifest` lists, the store holding
/// `store`, `failures` recorded, at `now`: a source yt-dlp fetched with an
/// address to fetch it again from, or a torrent's file, missing from the
/// store or damaged there and noted to be fetched again; one whose fetch
/// failed lately waits, as one fetched again too often no longer comes,
/// unless `retry`. A torrent's file no fetch mends is not fetched while
/// the store holds it, `retry` or not.
#[must_use]
pub fn due(
    manifest: &Manifest,
    store: &Store,
    failures: &BTreeMap<SourceKey, state::Failure>,
    retry: bool,
    now: u64,
) -> Due {
    let fetchable = |k: &SourceKey| match k {
        SourceKey::Torrent(_) => true,
        SourceKey::Manual(_) => false,
        SourceKey::Remote { .. } => {
            !crate::provider::is_kept(k)
                && (manifest.settings.sites.fetch_url(k).is_some()
                    || manifest
                        .song_with(k)
                        .and_then(|s| s.held.get(k)?.url.as_ref())
                        .is_some())
        }
    };
    let mut due = Due::default();
    for key in manifest.keys().into_iter().filter(fetchable) {
        let failure = failures.get(&key);
        let damaged =
            failure.is_some_and(|f| f.step == state::Step::Refetch && (retry || f.due(None, now)));
        let fetch = match (store.has(&key), damaged) {
            (_, true) => Fetch::Damaged,
            (false, false) => Fetch::Missing,
            (true, false) => continue,
        };
        let waits = fetch == Fetch::Missing
            && !retry
            && failure.is_some_and(|f| f.step == state::Step::Fetch && !f.due(None, now));
        if waits {
            due.waiting.insert(key);
        } else {
            due.now.insert(key, fetch);
        }
    }
    due
}

/// The listed torrent files the store lacks or holds damaged, by their
/// torrent's hash, each said; those that failed lately wait unless
/// `retry`, and are said as waiting.
pub fn torrents<W: Write>(out: &mut W, due: &Due) -> Result<BTreeMap<String, Vec<SourceKey>>> {
    let torrent = |k: &&SourceKey| matches!(k, SourceKey::Torrent(_));
    let waiting = due.waiting.iter().filter(torrent).count();
    if waiting > 0 {
        crate::ui::warning(
            out,
            &format!(
                "Not fetching {waiting} missing torrent file(s) that failed lately; `sync --retry` \
                 tries now"
            ),
        )?;
    }
    let mut by_hash: BTreeMap<String, Vec<SourceKey>> = BTreeMap::new();
    let (mut repaired, mut gone) = (0, 0);
    for (key, fetch) in &due.now {
        if let SourceKey::Torrent(t) = key {
            by_hash
                .entry(t.hash().to_string())
                .or_default()
                .push(key.clone());
            match fetch {
                Fetch::Damaged => repaired += 1,
                Fetch::Missing => gone += 1,
            }
        }
    }
    if repaired > 0 {
        crate::ui::info(
            out,
            &format!("Fetching {repaired} damaged torrent file(s) again"),
        )?;
    }
    if gone > 0 {
        crate::ui::info(
            out,
            &format!("Fetching {gone} missing torrent file(s) again"),
        )?;
    }
    Ok(by_hash)
}
