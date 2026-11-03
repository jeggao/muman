//! `state.json`, as muman kept the state before `state.db`, read once to
//! bring a home over. An earlier version's file reads as this one's: a
//! cache entry of another shape is dropped, a file beside an output
//! version 2 named by its place is recorded as one of this version's,
//! and keys an extractor named are respelled by their site.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

use super::{Measured, SITES, Sidecar, State, VERSION, Written};
#[cfg(test)]
use crate::dirs::STATE;
use crate::resolve::Plan;
use crate::source::SourceKey;

/// The state `text`, the `state.json` at `file`, holds, as any version
/// wrote it.
pub(super) fn read(file: &Path, text: &[u8]) -> Result<State> {
    let mut state: State = serde_json::from_slice(text).with_context(|| {
        format!(
            "{} cannot be read; move it aside to start over, which forgets which library files are muman's",
            file.display()
        )
    })?;
    if state.version > VERSION {
        bail!(
            "{} is format version {}, newer than this muman's {VERSION}",
            file.display(),
            state.version
        );
    }
    if state.version < SITES {
        let keys = state.outputs.values().flat_map(|w| &w.sources);
        state.respelled = keys
            .chain(state.facts.keys())
            .filter_map(|k| Some((k.clone(), k.spelled_before()?)))
            .collect();
    }
    state.version = VERSION;
    Ok(state)
}

/// Write `state` to `home`'s `state.json` as muman wrote it before the
/// store, making a home an earlier muman left.
#[cfg(test)]
pub(crate) fn write(state: &State, home: &Path) -> Result<()> {
    let mut text = serde_json::to_vec_pretty(state).context("writing the state")?;
    text.push(b'\n');
    crate::atomic::write(home, &crate::atomic::Name::new(STATE)?, &text)
}

/// A cache's entries, without any an earlier version wrote in another
/// shape: they are made again, where failing would lose `outputs` too.
pub(super) fn lenient_map<'de, D, K, V>(d: D) -> std::result::Result<BTreeMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: DeserializeOwned + Ord,
    V: DeserializeOwned,
{
    let raw = BTreeMap::<String, serde_json::Value>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .filter_map(|(k, v)| {
            let key = serde_json::from_value(serde_json::Value::String(k)).ok()?;
            Some((key, serde_json::from_value(v).ok()?))
        })
        .collect())
}

pub(super) fn lenient_list<'de, D, T>(d: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let raw = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect())
}

/// [`Written`] as any version wrote it: version 2 named its lyrics alone,
/// by their place.
#[derive(Deserialize)]
pub(super) struct StoredWritten {
    sources: Vec<SourceKey>,
    #[serde(default)]
    sidecars: Vec<Sidecar>,
    #[serde(default, with = "crate::relpath::portable_opt")]
    lyrics: Option<PathBuf>,
    #[serde(default)]
    plan: Option<Plan>,
    #[serde(default)]
    stamp: Option<String>,
    #[serde(default)]
    digest: Option<String>,
}

impl From<StoredWritten> for Written {
    fn from(w: StoredWritten) -> Self {
        let mut sidecars = w.sidecars;
        if let Some(lyrics) = w.lyrics.filter(|l| !sidecars.iter().any(|s| s.path == *l)) {
            sidecars.push(Sidecar::at(lyrics));
        }
        Self {
            sources: w.sources,
            sidecars,
            plan: w.plan,
            stamp: w.stamp,
            digest: w.digest,
        }
    }
}

/// [`Measured`] as any version wrote it: version 2 kept its lyrics file's
/// size alone, 0 for none.
#[derive(Deserialize)]
pub(super) struct StoredMeasured {
    source: SourceKey,
    audio: u64,
    #[serde(default)]
    beside: Vec<u64>,
    #[serde(default)]
    lyrics: u64,
}

impl From<StoredMeasured> for Measured {
    fn from(m: StoredMeasured) -> Self {
        let mut beside = m.beside;
        if m.lyrics > 0 && beside.is_empty() {
            beside.push(m.lyrics);
        }
        Self {
            source: m.source,
            audio: m.audio,
            beside,
        }
    }
}
