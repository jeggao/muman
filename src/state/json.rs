//! `state.json`, read and written. An earlier version's file reads as
//! this one's: a cache entry of another shape is dropped, a file beside
//! an output version 2 named by its place is recorded as one of this
//! version's, and keys an extractor named are respelled by their site;
//! saved, the state is of this version.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

use super::{Measured, SITES, Sidecar, State, VERSION, Written};
use crate::atomic;
use crate::dirs::STATE;
use crate::resolve::Plan;
use crate::source::SourceKey;

impl State {
    /// The state in `home`; an empty one when there is none yet.
    pub fn load(home: &Path) -> Result<Self> {
        let file = home.join(STATE);
        let text = match std::fs::read(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    version: VERSION,
                    ..Self::default()
                });
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
        };
        let mut state: Self = serde_json::from_slice(&text).with_context(|| {
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
        // Read, an earlier version's outputs are this one's: saved, they
        // are of this version, which the earlier refuses.
        state.version = VERSION;
        Ok(state)
    }

    pub fn save(&self, home: &Path) -> Result<()> {
        let mut text = serde_json::to_vec_pretty(self).context("writing the state")?;
        text.push(b'\n');
        // A run that changed nothing leaves the home untouched.
        if std::fs::read(home.join(STATE)).is_ok_and(|held| held == text) {
            return Ok(());
        }
        atomic::write(home, &atomic::Name::new(STATE)?, &text)
    }

    /// Record `trusted` as [`Self::trusted`] under the folder's lock,
    /// every other key left as the file holds it: a state an earlier
    /// version wrote is brought up to this one by a run that reads it
    /// whole, not here.
    pub fn keep_trusted(home: &Path, trusted: &BTreeMap<String, String>) -> Result<()> {
        let _lock = atomic::Lock::folder(home)?;
        let file = home.join(STATE);
        let mut held: serde_json::Value = match std::fs::read(&file) {
            Ok(text) => serde_json::from_slice(&text)
                .with_context(|| format!("{} cannot be read", file.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                serde_json::json!({ "version": VERSION })
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
        };
        let keys = held
            .as_object_mut()
            .with_context(|| format!("{} holds no state", file.display()))?;
        if trusted.is_empty() {
            keys.remove("trusted");
        } else {
            keys.insert("trusted".into(), serde_json::to_value(trusted)?);
        }
        let mut text = serde_json::to_vec_pretty(&held).context("writing the state")?;
        text.push(b'\n');
        atomic::write(home, &atomic::Name::new(STATE)?, &text)
    }
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
