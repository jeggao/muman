//! How well each kind of evidence vetting weighs tells a song's record
//! from another's, counted on the library itself, for `check
//! --calibrate`.
//!
//! Vetting's points ([`crate::vet`]) were set by hand on one library.
//! Here records are labelled the *same* recording as a song's or
//! *another* by what vetting never weighs, and each label counts only
//! for the kinds of evidence its choosing never looked at: a record
//! chosen for sharing a title agrees on it by construction.
//!
//! | Label | Chosen by |
//! |---|---|
//! | The same where its recording ID is one the song's files carry; another where they carry others | nothing |
//! | The same where it shares an ISRC with files that carry no recording ID | nothing |
//! | The same where an AcoustID lookup of the song's own audio found it | title, credits and length, which the lookup ranks its recordings by |
//! | The same where it was that lookup's only candidate | length, which the lookup keeps recordings by |
//! | Another: a record listed on another song sharing one of the song's titles | title |
//! | Another: a record listed on another song sharing one of its credits | credits |
//!
//! A remaster keeps its ISRC under another recording ID, so an ISRC
//! speaks only where the files carry no ID. A lookup refuses a recording
//! whose title and credits both disagree, so even its only candidate
//! never shows both; and it finds a wrong record now and then, so the
//! counts for the same recording run a little low. A look-alike, at most
//! [`LOOKALIKES`] of each kind a song, is weighed on the song as if it
//! were listed there, and skipped where it names a recording the song's
//! records or lookups name or shares an ISRC with the song's sources: one
//! recording listed on two songs.
//!
//! Each kind of evidence is then counted by how often it agreed,
//! disagreed or said nothing for each label, and the points those counts
//! suggest are their log odds: twice the base-2 logarithm of how much
//! likelier an agreement, or a disagreement, is for the same recording
//! than for another, rounded, once each label counts [`MIN_LABELLED`]
//! records or more. Nothing is changed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;

use crate::facts::Facts;
use crate::manifest::{Manifest, Song};
use crate::reconcile::Resolver;
use crate::settings::Settings;
use crate::similar;
use crate::source::SourceKey;
use crate::state::{Looked, Outcome, State};
use crate::tags::Field;

/// At most this many look-alikes of each kind are weighed on a song.
pub const LOOKALIKES: usize = 2;

/// The fewest records of each label that suggest points.
pub const MIN_LABELLED: usize = 20;

/// How often one kind of evidence agreed, disagreed or said nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub agreed: usize,
    pub disagreed: usize,
    pub silent: usize,
}

impl Counts {
    fn add(&mut self, side: Option<bool>) {
        match side {
            Some(true) => self.agreed += 1,
            Some(false) => self.disagreed += 1,
            None => self.silent += 1,
        }
    }

    fn total(self) -> usize {
        self.agreed + self.disagreed + self.silent
    }
}

/// One kind of evidence counted for each label.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub same: Counts,
    pub other: Counts,
}

impl Tally {
    /// The points its counts suggest for an agreement and a disagreement,
    /// each twice the base-2 log of its odds; none while a label counts
    /// fewer than [`MIN_LABELLED`].
    #[must_use]
    pub fn suggested(&self) -> Option<(i64, i64)> {
        let (same, other) = (self.same.total(), self.other.total());
        if same < MIN_LABELLED || other < MIN_LABELLED {
            return None;
        }
        // One more of each, and two more of the whole, keeps a count of
        // nought from an infinite weight.
        let share = |n: usize, of: usize| (n as f64 + 1.0) / (of as f64 + 2.0);
        let points =
            |s: usize, o: usize| (2.0 * (share(s, same) / share(o, other)).log2()).round() as i64;
        Some((
            points(self.same.agreed, self.other.agreed),
            points(self.same.disagreed, self.other.disagreed),
        ))
    }
}

/// How a record was labelled, as the module's table says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Label {
    same: bool,
    /// The way, in words.
    how: &'static str,
    /// The kinds of evidence choosing it looked at, which it is not
    /// counted for.
    chosen_by: &'static [&'static str],
}

const BY_ID: &str = "a recording ID";
const BY_ISRC: &str = "an ISRC";
const PRINT: Label = Label {
    same: true,
    how: "a lookup of the audio",
    chosen_by: &["title", "credits", "length"],
};
const PRINT_ALONE: Label = Label {
    same: true,
    how: "a lookup's only candidate",
    chosen_by: &["length"],
};
const TITLE_ALIKE: Label = Label {
    same: false,
    how: "another song's title",
    chosen_by: &["title"],
};
const CREDIT_ALIKE: Label = Label {
    same: false,
    how: "another song's credit",
    chosen_by: &["credits"],
};

/// What the library's labelled records showed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Calibration {
    /// Records labelled the same recording, and another.
    pub same: usize,
    pub other: usize,
    /// How many records each way labelled, by the way in words.
    pub how: BTreeMap<&'static str, usize>,
    /// Each kind of evidence, by name.
    pub by: BTreeMap<&'static str, Tally>,
}

impl Calibration {
    fn count(&mut self, label: Label, sides: [(&'static str, Option<bool>); 5]) {
        if label.same {
            self.same += 1;
        } else {
            self.other += 1;
        }
        *self.how.entry(label.how).or_default() += 1;
        for (name, side) in sides {
            if label.chosen_by.contains(&name) {
                continue;
            }
            let tally = self.by.entry(name).or_default();
            if label.same {
                tally.same.add(side);
            } else {
                tally.other.add(side);
            }
        }
    }
}

/// Each listed song's records, and its look-alikes, labelled and their
/// evidence counted, as the module says.
pub fn calibrate(
    manifest: &Manifest,
    settings: &Settings,
    state: &State,
    library: &Path,
) -> Result<Calibration> {
    let resolver = Resolver::new(manifest, settings, state, library)?;
    let lookups = state.acoustid_lookups();
    let alike = Lookalikes::of(&manifest.songs, &state.facts);
    let mut calibration = Calibration::default();
    for (n, song) in manifest.songs.iter().enumerate() {
        let Ok(vets) = crate::resolve::weigh(&resolver.input(song, None)) else {
            continue;
        };
        for (key, sides) in vets.weighed() {
            if let Some(label) = label(song, key, &state.facts, &lookups) {
                calibration.count(label, sides);
            }
        }
        for (record, label) in alike.of_song(n, song, &state.facts, &lookups) {
            let mut listed = song.clone();
            listed.sources.push(record.clone());
            let Ok(vets) = crate::resolve::weigh(&resolver.input(&listed, None)) else {
                continue;
            };
            if let Some((_, sides)) = vets.weighed().find(|(k, _)| *k == record) {
                calibration.count(label, sides);
            }
        }
    }
    Ok(calibration)
}

/// The values of `field` the source `key` offers, lowercased.
fn values(facts: &BTreeMap<SourceKey, Facts>, key: &SourceKey, field: Field) -> Vec<String> {
    (facts.get(key).and_then(|facts| facts.tags.get(&field)))
        .map(|o| o.values.iter().map(|v| v.to_lowercase()).collect())
        .unwrap_or_default()
}

fn is_record(key: &SourceKey) -> bool {
    matches!(key, SourceKey::Remote { site, .. } if site == crate::provider::MUSICBRAINZ)
}

/// The record `key` of `song` labelled by the IDs its files carry, else
/// by a lookup of its own audio, as the module says; none where neither
/// speaks.
fn label(
    song: &Song,
    key: &SourceKey,
    facts: &BTreeMap<SourceKey, Facts>,
    lookups: &BTreeMap<&SourceKey, &Looked>,
) -> Option<Label> {
    by_id(song, key, facts).or_else(|| by_print(song, key, lookups))
}

fn by_id(song: &Song, key: &SourceKey, facts: &BTreeMap<SourceKey, Facts>) -> Option<Label> {
    let files: Vec<&SourceKey> = song.sources.iter().filter(|k| k.file().is_some()).collect();
    let ids: Vec<String> = (files.iter())
        .flat_map(|k| values(facts, k, Field::MusicBrainzTrackId))
        .collect();
    let isrcs: Vec<String> = (files.iter())
        .flat_map(|k| values(facts, k, Field::Isrc))
        .collect();
    let mine = values(facts, key, Field::MusicBrainzTrackId);
    let shared = |of: &[String], with: &[String]| of.iter().any(|v| with.contains(v));
    let labelled = |same: bool, how: &'static str| Label {
        same,
        how,
        chosen_by: &[],
    };
    if !ids.is_empty() && !mine.is_empty() {
        Some(labelled(shared(&mine, &ids), BY_ID))
    } else if ids.is_empty() && shared(&values(facts, key, Field::Isrc), &isrcs) {
        Some(labelled(true, BY_ISRC))
    } else {
        None
    }
}

fn by_print(
    song: &Song,
    key: &SourceKey,
    lookups: &BTreeMap<&SourceKey, &Looked>,
) -> Option<Label> {
    let found: Vec<&Looked> = (song.sources.iter())
        .filter_map(|k| lookups.get(k).copied())
        .filter(|l| matches!(&l.outcome, Outcome::Found(k) if k == key))
        .collect();
    if found.is_empty() {
        None
    } else if found.iter().any(|l| l.candidates.len() == 1) {
        Some(PRINT_ALONE)
    } else {
        Some(PRINT)
    }
}

/// Each record the songs list, by its folded title and by each name it
/// credits, with the song listing it.
struct Lookalikes<'a> {
    by_title: BTreeMap<String, BTreeSet<(&'a SourceKey, usize)>>,
    by_credit: BTreeMap<String, BTreeSet<(&'a SourceKey, usize)>>,
}

impl<'a> Lookalikes<'a> {
    fn of(songs: &'a [Song], facts: &BTreeMap<SourceKey, Facts>) -> Self {
        let mut alike = Self {
            by_title: BTreeMap::new(),
            by_credit: BTreeMap::new(),
        };
        for (n, song) in songs.iter().enumerate() {
            for key in song.sources.iter().filter(|k| is_record(k)) {
                for title in titles(facts, key) {
                    alike.by_title.entry(title).or_default().insert((key, n));
                }
                for name in credits(facts, key) {
                    alike.by_credit.entry(name).or_default().insert((key, n));
                }
            }
        }
        alike
    }

    /// The records of other songs sharing a title, then a credit, with
    /// song `n`, as the module says.
    fn of_song(
        &self,
        n: usize,
        song: &Song,
        facts: &BTreeMap<SourceKey, Facts>,
        lookups: &BTreeMap<&SourceKey, &Looked>,
    ) -> Vec<(&'a SourceKey, Label)> {
        let named: BTreeSet<String> = (song.sources.iter())
            .filter(|k| is_record(k))
            .flat_map(|k| values(facts, k, Field::MusicBrainzTrackId))
            .chain(
                (song.sources.iter())
                    .filter_map(|k| lookups.get(k))
                    .flat_map(|l| &l.candidates)
                    .map(|c| c.id.as_str().to_lowercase()),
            )
            .chain(
                (song.sources.iter())
                    .filter_map(|k| lookups.get(k))
                    .flat_map(|l| match &l.outcome {
                        Outcome::Found(k) => values(facts, k, Field::MusicBrainzTrackId),
                        _ => Vec::new(),
                    }),
            )
            .collect();
        let isrcs: BTreeSet<String> = (song.sources.iter())
            .flat_map(|k| values(facts, k, Field::Isrc))
            .collect();
        let fresh = |key: &SourceKey, m: usize| {
            m != n
                && !song.sources.contains(key)
                && !(values(facts, key, Field::MusicBrainzTrackId).iter())
                    .any(|v| named.contains(v))
                && !(values(facts, key, Field::Isrc).iter()).any(|v| isrcs.contains(v))
        };
        let mut chosen: Vec<(&'a SourceKey, Label)> = Vec::new();
        let mine = |of: fn(&BTreeMap<SourceKey, Facts>, &SourceKey) -> BTreeSet<String>| {
            (song.sources.iter())
                .flat_map(|k| of(facts, k))
                .collect::<BTreeSet<String>>()
        };
        for (index, label, keys) in [
            (&self.by_title, TITLE_ALIKE, mine(titles)),
            (&self.by_credit, CREDIT_ALIKE, mine(credits)),
        ] {
            let found: BTreeSet<(&'a SourceKey, usize)> = (keys.iter())
                .filter_map(|k| index.get(k))
                .flatten()
                .copied()
                .filter(|(key, m)| fresh(key, *m))
                .filter(|(key, _)| !chosen.iter().any(|(c, _)| c == key))
                .collect();
            let picked: BTreeSet<&SourceKey> = found.iter().map(|(key, _)| *key).collect();
            chosen.extend(picked.into_iter().take(LOOKALIKES).map(|key| (key, label)));
        }
        chosen
    }
}

/// The titles `key` offers, folded.
fn titles(facts: &BTreeMap<SourceKey, Facts>, key: &SourceKey) -> BTreeSet<String> {
    (facts.get(key).and_then(|f| f.tags.get(&Field::Title)))
        .map(|o| o.values.iter().map(|v| similar::fold(v)).collect())
        .unwrap_or_default()
}

/// The names `key` credits, folded every way [`similar::credited`] does.
fn credits(facts: &BTreeMap<SourceKey, Facts>, key: &SourceKey) -> BTreeSet<String> {
    (facts.get(key).and_then(|f| f.tags.get(&Field::Artist)))
        .map(|o| o.values.iter().flat_map(|v| similar::credited(v)).collect())
        .unwrap_or_default()
}

/// The calibration as `check --calibrate` prints it, line by line: how
/// many records each way labelled, then each kind's counts for each label
/// and the points they suggest beside the points vetting gives.
#[must_use]
pub fn lines(c: &Calibration) -> Vec<String> {
    let current: BTreeMap<&str, (i64, i64)> = BTreeMap::from([
        ("title", (crate::vet::TITLE_ALIKE, crate::vet::TITLE_APART)),
        (
            "credits",
            (crate::vet::CREDIT_SHARED, crate::vet::NO_CREDIT_SHARED),
        ),
        ("length", (crate::vet::LENGTH_CLOSE, crate::vet::LENGTH_FAR)),
        (
            "release",
            (crate::vet::RELEASE_ALIKE, crate::vet::RELEASE_APART),
        ),
        ("unit", (crate::vet::UNIT_ALIKE, 0)),
    ]);
    let mut out = vec![format!(
        "Records labelled: {} the same recording, {} another",
        c.same, c.other
    )];
    if !c.how.is_empty() {
        let how: Vec<String> = (c.how.iter())
            .map(|(how, n)| format!("{how} {n}"))
            .collect();
        out.push(format!("  by {}", how.join(", ")));
    }
    if c.by.is_empty() {
        out.push("  no evidence weighed: is `[tags.clean.structure] vetting` off?".into());
    }
    for (name, tally) in &c.by {
        let shown = |n: Counts| {
            format!(
                "{} agreed, {} disagreed, {} silent",
                n.agreed, n.disagreed, n.silent
            )
        };
        let suggested = tally.suggested().map_or_else(
            || "too few to suggest".to_string(),
            |(a, d)| format!("suggests {a:+} / {d:+}"),
        );
        let (a, d) = current.get(name).copied().unwrap_or_default();
        out.push(format!(
            "  {name}: same {}; another {}; {suggested}, now {a:+} / {d:+}",
            shown(tally.same),
            shown(tally.other),
        ));
    }
    out
}

#[cfg(test)]
mod tests;
