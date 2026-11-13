//! Which of a song's audio sources hold the song's recording, and which
//! another, as a live take or another song filed beside it does.
//!
//! The song's audio sources fall into recordings by how their audio
//! compares ([`crate::align`]): two that align, either way round, or that
//! hold the same packets, one audio digest, are one recording, and so is
//! every source a chain of such pairs joins. The sources are taken in
//! sorted order and a recording is named by its first, so the order of
//! `sources` changes none. A source too short to compare with any other
//! ([`crate::align::comparable`]), and holding no other's packets, is of
//! no recording known: it is set aside, neither the song's recording nor
//! another, and ranks as the song's.
//!
//! With one recording nothing is decided. With several, the song's is:
//!
//! 1. The one whose lengths fit a record [`crate::vet`] believes on its
//!    names, one of its sources within [`CLOSE_MS`] of the length the
//!    record states, where the record fits no other recording. A lookup
//!    asks with the song's chosen audio, AcoustID by its print and length
//!    and MusicBrainz by its length, so a record a lookup found fits that
//!    audio's recording by how it was found. Which audio it asked with is
//!    not kept; it is taken to be the audio the measures alone rank
//!    first, ties included, which every lookup made before recordings
//!    were told apart asked with, and the source the lookup was made
//!    from, as its row keeps it: the song's first source of its own, most
//!    often the audio the song began with, and asked with while it was
//!    the song's only audio. A record counts only for a recording holding
//!    none of them, or where a file names its recording ID, and records
//!    that count for two recordings decide nothing.
//! 2. Else the one with the most whole sources, then the most sources,
//!    where one holds more than every other. A damaged source
//!    ([`crate::integrity`]) is counted last: a hole can part it from its
//!    own recording where comparing cannot step past it
//!    ([`crate::align::align_lacking`]), and two copies of one damaged
//!    file are one recording by their digest, so damaged copies would
//!    otherwise outvote the one whole copy and rank it as another
//!    recording.
//! 3. Else none: every recording ranks alike, and the song is picked as
//!    though it held one.
//!
//! A source of another recording than the song's ranks after every source
//! of the song's, ahead of every measure, but is never left out: a pin
//! takes it, and it still plays where it is all the song has.
//!
//! A source whose speed is more than [`SPEED_PPM`] off the rest of its
//! recording's is named, not ranked lower: its speed is the median of how
//! much longer it plays the recording than each source of its recording it
//! aligns with, and a copy played faster plays higher too. Of 2,301
//! aligning pairs in one library, 509 differed in speed by 0.1% or more
//! and 40 by 0.5% or more. Which speed is the recording's own no measure
//! says, so none is ranked by it.

use std::collections::{BTreeMap, BTreeSet};

use super::CLOSE_MS;
use crate::source::SourceKey;
use crate::state::Aligned;

/// How far a source's speed may be from the rest of its recording's
/// before it is named, in parts per million: 0.5%, about a twelfth of a
/// semitone in pitch.
pub const SPEED_PPM: i64 = 5_000;

/// One of the song's audio sources, as recordings are told apart.
#[derive(Debug, Clone)]
pub struct Audible<'a> {
    pub key: &'a SourceKey,
    /// How long it plays, in ms.
    pub ms: Option<i64>,
    /// The digest of its audio's packets.
    pub digest: Option<&'a str>,
    /// Whether its measures alone rank it first, ties included: the
    /// audio a lookup asked with.
    pub first: bool,
    /// Whether its audio is all there ([`crate::integrity`]).
    pub whole: bool,
}

/// A record vetting believes, as a length to hold the recordings to.
#[derive(Debug, Clone)]
pub struct Record<'a> {
    pub key: &'a SourceKey,
    /// The length it states its recording plays, in ms.
    pub ms: i64,
    /// Whether a file names its recording ID.
    pub named: bool,
    /// The sources the lookups that found it were made from.
    pub from: Vec<&'a SourceKey>,
}

/// What decided the song's recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum By {
    /// A record, stating this length in ms.
    Record(i64),
    Most,
}

/// Where one source falls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Place {
    group: usize,
    ms: Option<i64>,
    /// How much longer it plays the recording than the rest of its
    /// recording, in parts per million.
    speed_ppm: i64,
}

/// A song's audio sources in recordings, and which is the song's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recordings {
    /// Each recording's sources, sorted, in the order of their first.
    groups: Vec<Vec<SourceKey>>,
    of: BTreeMap<SourceKey, Place>,
    /// The sources too short to compare with any other.
    short: BTreeSet<SourceKey>,
    /// The song's recording, and what decided it.
    song: Option<(usize, By)>,
}

impl Recordings {
    /// `audible` in recordings, by `aligned`, the comparison of one with
    /// another, the song's among them as `records` say.
    pub fn of<'x>(
        audible: &[Audible<'_>],
        records: &[Record<'_>],
        aligned: impl Fn(&SourceKey, &SourceKey) -> Option<&'x Aligned>,
    ) -> Self {
        let mut sorted: Vec<&Audible<'_>> = audible.iter().collect();
        sorted.sort_by_key(|a| a.key);
        sorted.dedup_by_key(|a| a.key);
        let same = |a: &Audible<'_>, b: &Audible<'_>| a.digest.is_some() && a.digest == b.digest;
        // How long a source played when it was compared, which is what
        // decided whether it could be.
        let compared_ms = |a: &Audible<'_>| {
            sorted.iter().find_map(|b| {
                (aligned(a.key, b.key).map(|x| x.a_ms))
                    .or_else(|| aligned(b.key, a.key).map(|x| x.b_ms))
            })
        };
        let short: BTreeSet<SourceKey> = sorted
            .iter()
            .filter(|a| {
                compared_ms(a).is_some_and(|ms| !crate::align::comparable(ms, ms))
                    && !sorted.iter().any(|b| b.key != a.key && same(a, b))
            })
            .map(|a| a.key.clone())
            .collect();
        sorted.retain(|a| !short.contains(a.key));
        let fits = |a: &SourceKey, b: &SourceKey| aligned(a, b).filter(|x| x.fits());
        let joined = |a: &Audible<'_>, b: &Audible<'_>| {
            same(a, b) || fits(a.key, b.key).is_some() || fits(b.key, a.key).is_some()
        };
        let mut root: Vec<usize> = (0..sorted.len()).collect();
        for i in 0..sorted.len() {
            for j in i + 1..sorted.len() {
                if joined(sorted[i], sorted[j]) {
                    let (a, b) = (top(&root, i), top(&root, j));
                    root[a.max(b)] = a.min(b);
                }
            }
        }
        let mut groups: Vec<Vec<SourceKey>> = Vec::new();
        let mut index: BTreeMap<usize, usize> = BTreeMap::new();
        let mut of = BTreeMap::new();
        for (i, a) in sorted.iter().enumerate() {
            let group = *index.entry(top(&root, i)).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[group].push(a.key.clone());
            let place = Place {
                group,
                ms: a.ms,
                speed_ppm: 0,
            };
            of.insert(a.key.clone(), place);
        }
        for a in &sorted {
            let place = &of[a.key];
            let mut speeds: Vec<i64> = groups[place.group]
                .iter()
                .filter(|o| *o != a.key)
                .filter_map(|o| {
                    let same = sorted
                        .iter()
                        .any(|b| b.key == o && a.digest.is_some() && b.digest == a.digest);
                    if same {
                        return Some(0);
                    }
                    fits(a.key, o)
                        .map(|x| x.stretch_ppm)
                        .or_else(|| fits(o, a.key).map(|x| -x.stretch_ppm))
                })
                .collect();
            speeds.sort_unstable();
            let speed_ppm = match speeds.len() {
                0 => 0,
                n if n % 2 == 1 => speeds[n / 2],
                n => i64::midpoint(speeds[n / 2 - 1], speeds[n / 2]),
            };
            if let Some(p) = of.get_mut(a.key) {
                p.speed_ppm = speed_ppm;
            }
        }
        let song = (groups.len() > 1)
            .then(|| song_of(&groups, &of, &sorted, records))
            .flatten();
        Self {
            groups,
            of,
            short,
            song,
        }
    }

    /// Whether `key` holds another recording than the song's: 1 if so,
    /// to rank it after every source of the song's, else 0.
    #[must_use]
    pub fn rank(&self, key: &SourceKey) -> i64 {
        i64::from(self.outside(key))
    }

    fn outside(&self, key: &SourceKey) -> bool {
        match (self.song, self.of.get(key)) {
            (Some((song, _)), Some(place)) => place.group != song,
            _ => false,
        }
    }

    /// Whether the song holds more than one recording.
    #[must_use]
    pub fn several(&self) -> bool {
        self.groups.len() > 1
    }

    /// Which recording `key` holds, in words, as `status` says it:
    /// `another recording than the song's 2 others: aligns with none, 31
    /// s longer than its record`.
    #[must_use]
    pub fn describe(&self, key: &SourceKey) -> String {
        let details = self.details(key);
        if details.is_empty() {
            self.lead(key)
        } else {
            format!("{}: {}", self.lead(key), details.join(", "))
        }
    }

    /// What `key`'s recording adds to how it measures, where it is one of
    /// several or plays at another speed than the rest of it.
    #[must_use]
    pub fn aside(&self, key: &SourceKey) -> Option<String> {
        let lead = self.several().then(|| self.lead(key));
        let said: Vec<String> = lead.into_iter().chain(self.speed(key)).collect();
        (!said.is_empty()).then(|| said.join(", "))
    }

    /// Each source but `chosen` of another recording than its, or at
    /// another speed than the rest of its own, with which it is.
    #[must_use]
    pub fn others(&self, chosen: &SourceKey) -> Vec<(SourceKey, String)> {
        let mine = self.of.get(chosen).map(|p| p.group);
        self.of
            .iter()
            .filter(|(key, place)| {
                *key != chosen && (Some(place.group) != mine || self.speed(key).is_some())
            })
            .map(|(key, _)| (key.clone(), self.describe(key)))
            .collect()
    }

    fn lead(&self, key: &SourceKey) -> String {
        let Some(place) = self.of.get(key) else {
            return if self.short.contains(key) {
                "too short to compare with the others".to_string()
            } else {
                "not compared".to_string()
            };
        };
        let size = self.groups[place.group].len();
        match self.song {
            _ if !self.several() && size == 1 && self.short.is_empty() => {
                "the only audio".to_string()
            }
            _ if !self.several() && size == 1 => {
                "the only audio long enough to compare".to_string()
            }
            _ if !self.several() => {
                format!("the same recording as the song's {}", others(size - 1))
            }
            Some((song, by)) if song == place.group => match by {
                By::Record(_) => "the song's recording by its record".to_string(),
                By::Most => format!(
                    "the song's recording, as {size} of its {} sources",
                    self.of.len()
                ),
            },
            Some((song, _)) => format!(
                "another recording than the song's {}",
                others(self.groups[song].len())
            ),
            None => format!(
                "one of {} recordings, none the song's by a record or by most sources",
                self.groups.len()
            ),
        }
    }

    fn details(&self, key: &SourceKey) -> Vec<String> {
        let Some(place) = self.of.get(key) else {
            return Vec::new();
        };
        let alone = self.several() && self.groups[place.group].len() == 1;
        let gap = match (self.song, place.ms) {
            (Some((_, By::Record(stated))), Some(ms)) if (ms - stated).abs() > CLOSE_MS => {
                let s = ((ms - stated).abs() + 500) / 1000;
                let longer = if ms > stated { "longer" } else { "shorter" };
                Some(format!("{s} s {longer} than its record"))
            }
            _ => None,
        };
        alone
            .then(|| "aligns with none".to_string())
            .into_iter()
            .chain(gap)
            .chain(self.speed(key))
            .collect()
    }

    /// How far `key`'s speed is from the rest of its recording's, where
    /// it is more than [`SPEED_PPM`].
    fn speed(&self, key: &SourceKey) -> Option<String> {
        let ppm = self.of.get(key)?.speed_ppm;
        (ppm.abs() > SPEED_PPM).then(|| {
            let slower = if ppm > 0 { "slower" } else { "faster" };
            let percent = ppm.abs() as f64 / 10_000.0;
            format!("plays {percent:.1}% {slower} than the rest of its recording")
        })
    }
}

/// `n` other sources, in words: `1 other`, `2 others`.
fn others(n: usize) -> String {
    match n {
        1 => "1 other".to_string(),
        n => format!("{n} others"),
    }
}

/// Whether `a` and `b` are one recording, as [`Recordings::of`] tells a
/// song's sources apart by `aligned`: `None` where it cannot tell, the
/// two neither compared nor holding the same packets, or either too short
/// to compare. A source joins a song only on this rule, so its song never
/// holds it as another recording on a comparison made after.
pub fn one<'x>(
    a: &Audible<'_>,
    b: &Audible<'_>,
    aligned: impl Fn(&SourceKey, &SourceKey) -> Option<&'x Aligned>,
) -> Option<bool> {
    let same = a.digest.is_some() && a.digest == b.digest;
    if !same && aligned(a.key, b.key).is_none() && aligned(b.key, a.key).is_none() {
        return None;
    }
    let found = Recordings::of(&[a.clone(), b.clone()], &[], aligned);
    if found.short.contains(a.key) || found.short.contains(b.key) {
        return None;
    }
    Some(!found.several())
}

/// The root of `i`'s set, each set rooted at its smallest member.
fn top(root: &[usize], mut i: usize) -> usize {
    while root[i] != i {
        i = root[i];
    }
    i
}

/// The song's recording among `groups`, and what decided it: a record
/// fitting it alone that no source of it led to, else the most whole
/// sources, then the most sources.
fn song_of(
    groups: &[Vec<SourceKey>],
    of: &BTreeMap<SourceKey, Place>,
    sorted: &[&Audible<'_>],
    records: &[Record<'_>],
) -> Option<(usize, By)> {
    let first: BTreeSet<&SourceKey> = sorted.iter().filter(|a| a.first).map(|a| a.key).collect();
    let mut records: Vec<&Record<'_>> = records.iter().collect();
    records.sort_by_key(|r| r.key);
    let mut decided: BTreeMap<usize, i64> = BTreeMap::new();
    for r in records {
        let fitting: BTreeSet<usize> = of
            .values()
            .filter(|p| p.ms.is_some_and(|ms| (ms - r.ms).abs() <= CLOSE_MS))
            .map(|p| p.group)
            .collect();
        let [group] = fitting.into_iter().collect::<Vec<_>>()[..] else {
            continue;
        };
        let led_inside = groups[group]
            .iter()
            .any(|k| first.contains(k) || r.from.contains(&k));
        if r.named || !led_inside {
            decided.entry(group).or_insert(r.ms);
        }
    }
    if let [(group, ms)] = decided.into_iter().collect::<Vec<_>>()[..] {
        return Some((group, By::Record(ms)));
    }
    let whole: BTreeSet<&SourceKey> = sorted.iter().filter(|a| a.whole).map(|a| a.key).collect();
    let count = |g: &Vec<SourceKey>| (g.iter().filter(|k| whole.contains(k)).count(), g.len());
    let most = groups.iter().map(count).max()?;
    match groups
        .iter()
        .enumerate()
        .filter(|(_, g)| count(g) == most)
        .collect::<Vec<_>>()[..]
    {
        [(group, _)] => Some((group, By::Most)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> SourceKey {
        SourceKey::Manual(format!("{name}.flac").as_str().into())
    }

    /// `a` aligning with `b`, `a` playing `stretch_ppm` longer.
    fn pair(a: &SourceKey, b: &SourceKey, stretch_ppm: i64) -> [Aligned; 2] {
        let one = |a: &SourceKey, b: &SourceKey, stretch_ppm| Aligned {
            a: a.clone(),
            b: b.clone(),
            method: crate::align::METHOD.into(),
            revs: (a.to_string(), b.to_string()),
            offset_ms: 0,
            stretch_ppm,
            score: 0.99,
            coverage: 1.0,
            a_ms: 200_000,
            b_ms: 200_000,
            a_extra_ms: 0,
            a_lacks_ms: 0,
            b_lacks_ms: 0,
        };
        [one(a, b, stretch_ppm), one(b, a, -stretch_ppm)]
    }

    fn audible(key: &SourceKey, ms: i64) -> Audible<'_> {
        Audible {
            key,
            ms: Some(ms),
            digest: None,
            first: false,
            whole: true,
        }
    }

    fn recordings(
        audible: &[Audible<'_>],
        records: &[Record<'_>],
        alignments: &[Aligned],
    ) -> Recordings {
        Recordings::of(audible, records, |a, b| {
            alignments.iter().find(|x| &x.a == a && &x.b == b)
        })
    }

    #[test]
    fn sources_aligning_or_sharing_packets_are_one_recording_in_any_order() {
        let keys = [key("a"), key("b"), key("c"), key("d")];
        let [a, b, c, d] = &keys;
        let alignments = pair(b, c, 0);
        let mut all = vec![
            audible(a, 200_000),
            audible(b, 200_000),
            audible(c, 200_000),
            audible(d, 231_000),
        ];
        all[0].digest = Some("x");
        all[1].digest = Some("x");
        let heard = recordings(&all, &[], &alignments);
        assert_eq!(
            heard.groups,
            [vec![a.clone(), b.clone(), c.clone()], vec![d.clone()]]
        );
        assert_eq!((heard.rank(a), heard.rank(d)), (0, 1), "the most sources");
        all.reverse();
        assert_eq!(recordings(&all, &[], &alignments), heard);
        assert_eq!(
            heard.describe(d),
            "another recording than the song's 3 others: aligns with none"
        );
        assert_eq!(heard.others(a), [(d.clone(), heard.describe(d))]);
    }

    #[test]
    fn a_record_counts_only_for_a_recording_no_lookup_asked_with() {
        let (a, b, c, mb) = (key("a"), key("b"), key("c"), key("mb"));
        let alignments = pair(&a, &b, 0);
        let mut all = vec![
            audible(&a, 231_000),
            audible(&b, 230_000),
            audible(&c, 200_000),
        ];
        let record = |named| Record {
            key: &mb,
            ms: 200_500,
            named,
            from: Vec::new(),
        };
        let r = recordings(&all, &[record(false)], &alignments);
        assert_eq!((r.rank(&a), r.rank(&c)), (1, 0), "the record over most");
        assert_eq!(
            r.describe(&a),
            "another recording than the song's 1 other: 31 s longer than its record"
        );
        assert_eq!(
            r.describe(&c),
            "the song's recording by its record: aligns with none"
        );

        let found_from_c = Record {
            from: vec![&c],
            ..record(false)
        };
        let r = recordings(&all, &[found_from_c], &alignments);
        assert_eq!((r.rank(&a), r.rank(&c)), (0, 1), "looked up from it");

        all[2].first = true;
        let r = recordings(&all, &[record(false)], &alignments);
        assert_eq!((r.rank(&a), r.rank(&c)), (0, 1), "its lookup's own audio");
        let r = recordings(&all, &[record(true)], &alignments);
        assert_eq!((r.rank(&a), r.rank(&c)), (1, 0), "a file names it");
    }

    #[test]
    fn a_source_too_short_to_compare_is_of_no_recording() {
        let (a, b, c) = (key("a"), key("b"), key("c"));
        let mut alignments = pair(&a, &b, 0).to_vec();
        let short = |mut x: Aligned| {
            (x.score, x.coverage) = (0.0, 0.0);
            if x.a == c {
                x.a_ms = 3_000;
            } else {
                x.b_ms = 3_000;
            }
            x
        };
        for other in [&a, &b] {
            alignments.extend(pair(&c, other, 0).map(short));
        }
        let all = [
            audible(&a, 200_000),
            audible(&b, 200_000),
            audible(&c, 3_000),
        ];
        let r = recordings(&all, &[], &alignments);
        assert!(!r.several());
        assert_eq!((r.rank(&a), r.rank(&c)), (0, 0));
        assert_eq!(r.describe(&c), "too short to compare with the others");
        assert_eq!(r.others(&a), []);
        let r = recordings(&all[1..], &[], &alignments);
        assert_eq!(r.describe(&b), "the only audio long enough to compare");
        assert_eq!(r.aside(&b), None);
    }

    #[test]
    fn copies_of_one_damaged_file_outvote_no_whole_copy() {
        let (a, b, c) = (key("a"), key("b"), key("c"));
        let mut all = vec![
            audible(&a, 200_000),
            audible(&b, 198_000),
            audible(&c, 198_000),
        ];
        for damaged in &mut all[1..] {
            damaged.digest = Some("x");
            damaged.whole = false;
        }
        let r = recordings(&all, &[], &[]);
        assert_eq!((r.rank(&a), r.rank(&b), r.rank(&c)), (0, 1, 1));
        all[0].whole = false;
        let r = recordings(&all, &[], &[]);
        assert_eq!(
            (r.rank(&a), r.rank(&b)),
            (1, 0),
            "all damaged: the most sources"
        );
    }

    #[test]
    fn recordings_alike_in_sources_rank_alike_and_a_fast_copy_is_named() {
        let (a, b, c) = (key("a"), key("b"), key("c"));
        let alignments = pair(&b, &c, -6_000);
        let r = recordings(&[audible(&a, 200_000), audible(&b, 200_000)], &[], &[]);
        assert_eq!((r.rank(&a), r.rank(&b)), (0, 0));
        assert_eq!(
            r.describe(&b),
            "one of 2 recordings, none the song's by a record or by most sources: \
             aligns with none"
        );

        let r = recordings(
            &[audible(&b, 200_000), audible(&c, 200_000)],
            &[],
            &alignments,
        );
        assert!(!r.several());
        assert_eq!(
            r.aside(&b).as_deref(),
            Some("plays 0.6% faster than the rest of its recording")
        );
        assert_eq!(r.others(&b), [(c.clone(), r.describe(&c))]);
        assert_eq!(
            r.describe(&c),
            "the same recording as the song's 1 other: plays 0.6% slower than the rest of \
             its recording"
        );
        let r = recordings(&[audible(&a, 200_000)], &[], &[]);
        assert_eq!(r.describe(&a), "the only audio");
    }
}
