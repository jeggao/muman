//! What muman keeps for itself in `state.json`: the library files it
//! wrote and the plan each was written from, which is what lets it
//! delete only its own files and rebuild only what changed, and caches
//! of what it measured, which are safe to lose.
//!
//! | Key | Safe to lose |
//! |---|---|
//! | `outputs` | No: without it nothing is deleted, and every song renders again |
//! | `replaced` | Mostly: a playlist added again fetches those uploads |
//! | `library`, `alignments`, `facts` | Yes: measured or set again |
//! | `sizes` | Yes: songs are rendered again to measure them |
//! | `failures`, `lookups` | Yes: each is tried or made again at once |
//!
//! An output with no plan is one muman owns but cannot vouch for, as one
//! a run records just before writing it: a crash between the two leaves
//! a file the next run writes again or deletes, rather than one no run
//! would ever delete. An output whose size and time differ from those
//! recorded was changed by something else, a tagger or a player: it is
//! not written over, and once no song makes it, it is left in place and
//! dropped from `outputs`, no longer muman's. When the library folder
//! moves, the files in the old one are left alone.
//!
//! A source that could not be read is not read again until its revision,
//! each of its files' size and modification time, changes; one that could
//! not be fetched again waits an hour, doubling with each failure up to a
//! week. A failure ends when the step succeeds: a step that runs without
//! the run's lock, as fetching and lookups do, keeps what it measured
//! through [`State::keep_measures`], which merges it into the file as it
//! is now by one rule, so a failure one step cleared is cleared on disk
//! too and one another process recorded meanwhile stays.
//!
//! A cache entry an earlier version wrote in another shape is dropped and
//! made again; only `outputs` failing to parse refuses the file, since
//! losing it would lose which library files are muman's.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::align;
use crate::atomic;
use crate::dirs::STATE;
use crate::facts::Facts;
use crate::provider::Provider;
use crate::resolve::Plan;
use crate::source::SourceKey;

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    /// The library folder the outputs are relative to.
    #[serde(default)]
    pub library: Option<PathBuf>,
    /// Every audio file written, by its path in the library.
    #[serde(default, with = "crate::relpath::portable_keys")]
    pub outputs: BTreeMap<PathBuf, Written>,
    /// Uploads a release was kept in place of, so listing them again
    /// fetches nothing.
    #[serde(default, deserialize_with = "lenient_map")]
    pub replaced: BTreeMap<SourceKey, SourceKey>,
    #[serde(default, deserialize_with = "lenient_list")]
    pub alignments: Vec<Aligned>,
    #[serde(default, deserialize_with = "lenient_map")]
    pub facts: BTreeMap<SourceKey, Facts>,
    /// What could not be done for a source, so it is not tried again at
    /// once.
    #[serde(default, deserialize_with = "lenient_map")]
    pub failures: BTreeMap<SourceKey, Failure>,
    /// Each lookup a song made from one of its sources, and what it found.
    #[serde(default, deserialize_with = "lenient_list")]
    pub lookups: Vec<Looked>,
    /// What each plan fitting asked about renders to, by
    /// [`crate::limit::plan_key`].
    #[serde(default, deserialize_with = "lenient_map")]
    pub sizes: BTreeMap<String, Measured>,
    /// Sources whose failures this process cleared, for a merge to clear
    /// on disk.
    #[serde(skip)]
    pub(crate) cleared: BTreeSet<SourceKey>,
}

/// A lookup of `find` made from the source `from`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Looked {
    pub from: SourceKey,
    /// The provider looked up, by name.
    pub find: String,
    /// `lookup::method` of that provider when it was made.
    pub method: String,
    /// Seconds since the Unix epoch.
    pub at: u64,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Found(SourceKey),
    Nothing,
    /// A record of the song that has no words.
    Instrumental,
    /// The user asked for no lookup, as `add --no-match` does.
    Declined,
    Failed {
        count: u32,
        error: String,
    },
}

impl Looked {
    #[must_use]
    pub fn now(from: SourceKey, find: Provider, outcome: Outcome) -> Self {
        Self {
            from,
            find: find.name().to_string(),
            method: crate::lookup::method(find).to_string(),
            at: now_secs(),
            outcome,
        }
    }

    /// Whether it is due again: one that found nothing after
    /// `recheck_days`, one that failed after an hour doubling with each
    /// failure; one made another way at once.
    #[must_use]
    pub fn due(&self, now: u64, recheck_days: u64) -> bool {
        let find = Provider::named(&self.find).ok();
        if find.is_none_or(|p| crate::lookup::method(p) != self.method) {
            return true;
        }
        let age = now.saturating_sub(self.at);
        match &self.outcome {
            Outcome::Found(_) | Outcome::Instrumental | Outcome::Declined => false,
            Outcome::Nothing => age >= recheck_days.saturating_mul(24 * 3600),
            Outcome::Failed { count, .. } => {
                age >= 3600_u64
                    .saturating_mul(1 << count.saturating_sub(1).min(16))
                    .min(MAX_WAIT_SECS)
            }
        }
    }
}

/// A cache's entries, without any an earlier version wrote in another
/// shape: they are made again, where failing would lose `outputs` too.
fn lenient_map<'de, D, K, V>(d: D) -> std::result::Result<BTreeMap<K, V>, D::Error>
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

fn lenient_list<'de, D, T>(d: D) -> std::result::Result<Vec<T>, D::Error>
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Written {
    /// The song's sources when it was written, which is how a song
    /// whose build failed is known to keep this file.
    pub sources: Vec<SourceKey>,
    /// The lyrics written beside it.
    #[serde(default, with = "crate::relpath::portable_opt")]
    pub lyrics: Option<PathBuf>,
    /// What it was made from; none for a file an earlier version wrote,
    /// or one a run was about to write.
    #[serde(default)]
    pub plan: Option<Plan>,
    /// Its size and modification time when written, by which a file
    /// changed since is told from one muman wrote.
    #[serde(default)]
    pub stamp: Option<String>,
}

/// What a plan renders to, as rendered once: its audio file and its
/// lyrics file, in bytes, before any hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measured {
    /// The plan's audio source, which keeps the entry while it is listed.
    pub source: SourceKey,
    pub audio: u64,
    pub lyrics: u64,
}

/// A step that failed for a source: what it was, how often, when last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub step: Step,
    /// The revision it failed at, where the step reads the file.
    pub rev: Option<String>,
    pub error: String,
    pub count: u32,
    /// Seconds since the Unix epoch.
    pub last: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Step {
    /// Reading and measuring the file.
    Measure,
    /// Fetching it again after it went missing.
    Fetch,
}

/// The longest a failed fetch waits before it is tried again.
const MAX_WAIT_SECS: u64 = 7 * 24 * 3600;

impl Failure {
    /// Whether the step is due again: a measure when the file changed, a
    /// fetch once an hour has passed, doubling with each failure.
    #[must_use]
    pub fn due(&self, rev: Option<&str>, now: u64) -> bool {
        match self.step {
            Step::Measure => self.rev.as_deref() != rev,
            Step::Fetch => {
                let wait = 3600_u64
                    .saturating_mul(1 << self.count.saturating_sub(1).min(16))
                    .min(MAX_WAIT_SECS);
                now.saturating_sub(self.last) >= wait
            }
        }
    }
}

/// Seconds since the Unix epoch.
#[must_use]
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// One comparison of two sources' audio, `a` against `b`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aligned {
    pub a: SourceKey,
    pub b: SourceKey,
    pub method: String,
    /// The revisions of `a` and `b` it was made on.
    pub revs: (String, String),
    /// How much later `a` plays `b`'s first moment.
    pub offset_ms: i64,
    /// How much longer `a` plays the recording than `b`, in parts per
    /// million.
    #[serde(default)]
    pub stretch_ppm: i64,
    pub score: f64,
    /// The share of `a`'s windows that agree with the offset.
    pub coverage: f64,
    /// How long each side's audio is.
    pub a_ms: i64,
    pub b_ms: i64,
    /// How long `a` plays sound outside the stretch it shares with `b`.
    pub a_extra_ms: i64,
}

impl Aligned {
    #[must_use]
    pub fn alignment(&self) -> align::Alignment {
        align::Alignment {
            offset_ms: self.offset_ms,
            stretch_ppm: self.stretch_ppm,
            score: self.score,
            coverage: self.coverage,
        }
    }

    #[must_use]
    pub fn fits(&self) -> bool {
        self.alignment().fits()
    }

    /// How much of `a` is sound that is not the recording it shares
    /// with `b`, as a video's intro or outro is. Silence padding counts
    /// for nothing, and so do the few windows inside that disagree in
    /// any pair's quiet passages.
    #[must_use]
    pub fn unmatched_ms(&self) -> i64 {
        self.a_extra_ms
    }
}

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
        let state: Self = serde_json::from_slice(&text).with_context(|| {
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
        Ok(state)
    }

    pub fn save(&self, home: &Path) -> Result<()> {
        let mut text = serde_json::to_vec_pretty(self).context("writing the state")?;
        text.push(b'\n');
        atomic::write(home, STATE, &text)
    }

    /// The comparison of `a` against `b` at these revisions, if made.
    #[must_use]
    pub fn alignment(
        &self,
        a: &SourceKey,
        b: &SourceKey,
        revs: &(String, String),
    ) -> Option<&Aligned> {
        self.alignments
            .iter()
            .find(|x| &x.a == a && &x.b == b && x.method == align::METHOD && &x.revs == revs)
    }

    /// Record that `step` failed for `key` now.
    pub fn record_failure(
        &mut self,
        key: &SourceKey,
        step: Step,
        rev: Option<String>,
        error: String,
    ) {
        let count = self
            .failures
            .get(key)
            .filter(|f| f.step == step)
            .map_or(0, |f| f.count);
        self.failures.insert(
            key.clone(),
            Failure {
                step,
                rev,
                error,
                count: count + 1,
                last: now_secs(),
            },
        );
    }

    /// Forget that a step failed for `key`, as its success shows.
    pub fn clear_failure(&mut self, key: &SourceKey) {
        if self.failures.remove(key).is_some() {
            self.cleared.insert(key.clone());
        }
    }

    /// Take `measured`'s facts and failures over these: a fact replaces
    /// the one kept, a failure `measured` cleared is cleared, and one it
    /// recorded replaces the one kept.
    pub fn merge_caches(&mut self, measured: &Self) {
        for (key, facts) in &measured.facts {
            self.facts.insert(key.clone(), facts.clone());
        }
        for key in &measured.cleared {
            self.failures.remove(key);
        }
        for (key, failure) in &measured.failures {
            self.failures.insert(key.clone(), failure.clone());
        }
    }

    /// The lookup of `find` made from `from`.
    #[must_use]
    pub fn looked(&self, from: &SourceKey, find: Provider) -> Option<&Looked> {
        self.lookups
            .iter()
            .find(|l| &l.from == from && l.find == find.name())
    }

    /// Record lookups and the uploads releases replaced in the state file
    /// as it is now, under its lock: a failure counts on from the last.
    pub fn keep_lookups(
        home: &Path,
        looked: &[Looked],
        replaced: &[(SourceKey, SourceKey)],
    ) -> Result<()> {
        if looked.is_empty() && replaced.is_empty() {
            return Ok(());
        }
        let _lock = atomic::Lock::folder(home)?;
        let mut state = Self::load(home)?;
        for new in looked {
            let mut new = new.clone();
            let before = state
                .lookups
                .iter()
                .position(|l| l.from == new.from && l.find == new.find);
            if let Some(at) = before {
                let old = state.lookups.remove(at);
                if let (Outcome::Failed { count, .. }, Outcome::Failed { count: c, .. }) =
                    (&old.outcome, &mut new.outcome)
                {
                    *c = count + 1;
                }
            }
            state.lookups.push(new);
        }
        state.replaced.extend(replaced.iter().cloned());
        state.save(home)
    }

    /// Merge what `measured` holds of facts and failures into the state
    /// file as it is now, under its lock, by [`Self::merge_caches`].
    pub fn keep_measures(home: &Path, measured: &Self) -> Result<()> {
        let _lock = atomic::Lock::folder(home)?;
        let mut state = Self::load(home)?;
        state.merge_caches(measured);
        state.save(home)
    }

    pub fn record_alignment(&mut self, aligned: Aligned) {
        self.alignments
            .retain(|x| !(x.a == aligned.a && x.b == aligned.b));
        self.alignments.push(aligned);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_merge_clears_what_a_step_cleared_and_keeps_what_another_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (
            SourceKey::youtube("aaaaaaaaaaa"),
            SourceKey::youtube("bbbbbbbbbbb"),
            SourceKey::youtube("ccccccccccc"),
        );
        let mut disk = State::default();
        disk.record_failure(&a, Step::Fetch, None, "gone".into());
        disk.record_failure(&b, Step::Measure, Some("1".into()), "unreadable".into());
        disk.save(dir.path()).unwrap();
        let mut measured = State::load(dir.path()).unwrap();
        measured.clear_failure(&a);
        let mut meanwhile = State::load(dir.path()).unwrap();
        meanwhile.record_failure(&c, Step::Fetch, None, "gone too".into());
        meanwhile.save(dir.path()).unwrap();
        State::keep_measures(dir.path(), &measured).unwrap();
        let after = State::load(dir.path()).unwrap();
        assert_eq!(
            after.failures.keys().collect::<Vec<_>>(),
            [&b, &c],
            "a's success clears it; b and c stay"
        );
    }

    fn aligned(offset_ms: i64, coverage: f64, a_ms: i64, b_ms: i64) -> Aligned {
        let start = offset_ms.max(0);
        let end = a_ms.min(b_ms + offset_ms);
        Aligned {
            a: SourceKey::youtube("aaaaaaaaaaa"),
            b: SourceKey::youtube("bbbbbbbbbbb"),
            method: align::METHOD.into(),
            revs: ("1".into(), "2".into()),
            offset_ms,
            stretch_ppm: 0,
            score: 0.99,
            coverage,
            a_ms,
            b_ms,
            a_extra_ms: a_ms - (end - start).max(0),
        }
    }

    #[test]
    fn a_video_s_intro_is_unmatched_and_the_release_is_not() {
        let video = aligned(20_000, 1.0, 200_000, 180_000);
        assert_eq!(video.unmatched_ms(), 20_000);
        let release = aligned(-20_000, 1.0, 180_000, 200_000);
        assert_eq!(release.unmatched_ms(), 0);
    }

    #[test]
    fn a_missing_state_is_empty_and_a_saved_one_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = State::load(dir.path()).unwrap();
        assert!(state.outputs.is_empty());
        state.replaced.insert(
            SourceKey::youtube("uuuuuuuuuuu"),
            SourceKey::youtube("ttttttttttt"),
        );
        state.record_alignment(aligned(0, 1.0, 1, 1));
        state.record_alignment(aligned(5, 1.0, 1, 1));
        state.save(dir.path()).unwrap();
        let back = State::load(dir.path()).unwrap();
        assert_eq!(back.replaced, state.replaced);
        assert_eq!(
            back.alignments.len(),
            1,
            "a comparison made again replaces the old"
        );
        let revs = ("1".to_string(), "2".to_string());
        assert_eq!(
            back.alignment(&state.alignments[0].a, &state.alignments[0].b, &revs)
                .unwrap()
                .offset_ms,
            5
        );
        assert!(
            back.alignment(
                &state.alignments[0].a,
                &state.alignments[0].b,
                &("1".into(), "3".into())
            )
            .is_none()
        );
    }

    #[test]
    fn cache_entries_of_another_shape_are_dropped_and_outputs_kept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(STATE),
            r#"{"version": 1,
                "outputs": {"A/B/C.opus": {"sources": ["youtube:aaaaaaaaaaa"]}},
                "alignments": [{"a": "youtube:aaaaaaaaaaa", "old": true}],
                "facts": {"youtube:aaaaaaaaaaa": {"rev": "1"}}}"#,
        )
        .unwrap();
        let state = State::load(dir.path()).unwrap();
        assert_eq!(state.outputs.len(), 1);
        assert!(state.alignments.is_empty() && state.facts.is_empty());
    }

    #[test]
    fn a_broken_or_newer_state_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(STATE), "{").unwrap();
        assert!(format!("{:#}", State::load(dir.path()).unwrap_err()).contains("move it aside"));
        std::fs::write(dir.path().join(STATE), "{\"version\": 9}").unwrap();
        assert!(format!("{:#}", State::load(dir.path()).unwrap_err()).contains("newer"));
    }
}
