//! Whether each source of a song names the song's recording, before
//! anything it offers counts: the same recording, unsure, or another.
//!
//! A MusicBrainz record comes to a song by a lookup: AcoustID ranks the
//! recordings a print was submitted with and never refuses one, and a
//! print linked to another song names another song. Unvetted, such a
//! record lends the song another artist's album, track, date and,
//! through the Cover Art Archive, cover; held to the song's title, it is
//! held to a title it may have given the song itself. So each source is
//! held to what the song's *other* sources claim, never to what it
//! supplied itself: leave one out. A claim is a structured offer, a
//! field kept for it, as cleaned, or a tag set by hand; a value read off
//! a video's title is too loose to judge by.
//!
//! Only a source without audio of its own is vetted: its one link to the
//! song is the lookup that found it. A source with audio is the song's by
//! its fingerprint or the song list, and its tags are what the others are
//! held to; were it vetted too, a file and a record that disagree would
//! each set the other aside. A source a pin names is never set aside,
//! and tags set by hand are claims, never vetted.
//!
//! Each kind of evidence agrees, disagrees or says nothing, in whole
//! points:
//!
//! | Evidence | Agrees | Disagrees |
//! |---|---|---|
//! | An ISRC or recording ID another source names too | [`IDENTIFIER`] | nothing: one song has many |
//! | Its title against the others' | [`TITLE_ALIKE`] | [`TITLE_APART`], or a version they never name |
//! | Its credits against the others', a name or an artist ID shared | [`CREDIT_SHARED`] | [`NO_CREDIT_SHARED`] |
//! | The length it states against the chosen audio's | [`LENGTH_CLOSE`] | [`LENGTH_FAR`] |
//! | Its album against the album the song's files name | [`RELEASE_ALIKE`] | [`RELEASE_APART`] |
//! | Its release group against its album's ([`crate::album::unit_groups`]) | [`UNIT_ALIKE`] | [`UNIT_APART`], and unsure |
//!
//! Titles and albums are compared as [`similar::titles_agree`] reads
//! them, as files write them, so one no rule compares, as kanji with
//! romaji, says nothing; credits as [`similar::credits_agree`] does, by
//! a whole name, so a tribute act's `Marlo Venn Tribute Band` shares
//! nothing with `Marlo Venn`. A performer a file's title names before
//! ` / ` is one of its credits ([`similar::performers`]).
//!
//! A field the song's audio sources or tags set by hand claim is held to
//! those alone; only a field none of them claims is held to the other
//! vetted sources, since two records one lookup found may well name one
//! wrong song together.
//!
//! - **Other**, giving the song nothing: a title and credits that both
//!   disagree, or a title naming an instrumental or a karaoke take that
//!   no other source's title names ([`VOICELESS`]), with no identifier
//!   shared, whatever led the lookup to it; or points at or under
//!   [`OTHER_AT`].
//! - **Unsure**, giving no release fields and no recording IDs, which
//!   name what it describes: points at or under [`UNSURE_AT`]; credits
//!   that disagree while its album is not the files', as a cover act's
//!   record whose title agrees; or another release group than its
//!   album's, unless its album is the files' own or they name none, as a
//!   box set's record of a song that came in its own album.
//! - **Same** otherwise, and for a source with nothing to compare; yet
//!   with no recording IDs while its credits share no name with the
//!   others': an ID names a recording, and another artist's of the same
//!   title is another recording, though its release may be the files',
//!   as a concert's.
//!
//! The points rank as the evidence does: an identifier outweighs every
//! name together; a title that disagrees outweighs one credit shared, so
//! a record of another song by the song's artist is unsure, while one
//! that agrees makes up for no credit shared, as the record of a concert
//! crediting each singer where the files credit its star; a title and
//! credits that both disagree reach [`OTHER_AT`] whatever the length and
//! album say; a length that disagrees makes a source no more than
//! unsure, most often because the song's audio is another cut than the
//! one the record was looked up for; and an album that disagrees alone
//! says too little, files being tagged with a compilation's or their own
//! name for an album, but tips a source whose credits disagree too.
//!
//! The Cover Art Archive's front of a release group or release that a
//! source set aside names, and no source still believed does, is set
//! aside with it: it is that album's cover. `[tags.clean.structure]
//! vetting` switches vetting off ([`crate::clean::VETTING`]).
//!
//! Sources with audio are told apart by the recording they hold, the
//! song's or another, in [`recordings`].

pub mod recordings;

use std::collections::{BTreeMap, BTreeSet};

use crate::provider::COVERART;
use crate::similar;
use crate::source::SourceKey;
use crate::tags::{Field, Scope};

/// An ISRC or a recording's MusicBrainz ID another source names too:
/// the one evidence that outweighs every name.
pub const IDENTIFIER: i64 = 12;
/// A title alike to another source's: as much as no credit shared takes
/// away, so a title that agrees is enough unless the album disagrees too.
pub const TITLE_ALIKE: i64 = 5;
/// A title apart from every other source's, or naming another version.
pub const TITLE_APART: i64 = -8;
/// A credited name, or an artist's MusicBrainz ID, another source names.
pub const CREDIT_SHARED: i64 = 3;
/// No credited name another source names, where some compare.
pub const NO_CREDIT_SHARED: i64 = -5;
/// A stated length within [`CLOSE_MS`] of the chosen audio's.
pub const LENGTH_CLOSE: i64 = 2;
/// A stated length [`FAR_MS`] or more from the chosen audio's.
pub const LENGTH_FAR: i64 = -3;
/// An album alike to one the song's files name: with a credit shared,
/// enough to outweigh a title told apart only by a version or a number
/// the files leave out, as a live album's track the files title plainly.
/// This and the album unit's points are what `check --calibrate` fitted
/// together on one library ([`crate::calibrate`]).
pub const RELEASE_ALIKE: i64 = 5;
/// An album, and an album artist, apart from those the song's files name:
/// nothing once the album unit is weighed, which on one library told
/// another recording as well alone; with credits that disagree too, the
/// source is unsure.
pub const RELEASE_APART: i64 = 0;
/// The release group the song's album unit is on, the songs of one
/// folder and album or of one `[[album]]` ([`crate::album::unit_groups`]):
/// with the files' album, enough to outweigh a title that disagrees, as
/// one a tag cut short or spelled a number in, but not a title and
/// credits that both do.
pub const UNIT_ALIKE: i64 = 4;
/// Another release group than the album unit's: on one library one in
/// twenty of its songs' own records against two in three of other songs'.
/// It also gives no release field, unless its album is the files' own,
/// as a concert's video beside its album.
pub const UNIT_APART: i64 = -7;
/// The points at or under which a source may name another recording.
pub const UNSURE_AT: i64 = -3;
/// The points at or under which a source names another recording.
pub const OTHER_AT: i64 = -10;
/// The versions of a sung song that are other recordings whatever else
/// agrees: a single holds its instrumental beside the song, by the same
/// artist, as long and on the same album, and AcoustID may link the two
/// takes' prints, so the title alone tells them apart.
pub const VOICELESS: [&str; 2] = ["instrumental", "karaoke"];
/// How near a stated length is to agree: a release's length and a rip's
/// differ by a second or two.
pub const CLOSE_MS: i64 = 3_000;
/// How far a stated length is to disagree: past the gap AcoustID allows a
/// match ([`crate::acoustid::LOOSE_GAP_S`]), and past what an intro or a
/// fade takes from one cut.
pub const FAR_MS: i64 = 30_000;

/// What vetting makes of a source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// It names the song's recording, or nothing says otherwise.
    #[default]
    Same,
    /// It may name another release or recording: it gives no release
    /// fields and no recording IDs.
    Unsure,
    /// It names another recording: it gives the song nothing.
    Other,
}

/// What one source says of the song, as vetting reads it.
#[derive(Debug, Clone)]
pub struct Claims<'a> {
    pub key: &'a SourceKey,
    /// The values of each field it offers structured, cleaned.
    pub offers: BTreeMap<Field, &'a [String]>,
    /// Whether it is vetted: no audio of its own, and no pin names it.
    pub vetted: bool,
    /// The length it says its recording plays, in ms.
    pub length_ms: Option<u64>,
}

impl Claims<'_> {
    fn values(&self, field: Field) -> impl Iterator<Item = &str> {
        self.offers
            .get(&field)
            .into_iter()
            .flat_map(|v| v.iter().map(String::as_str))
    }

    /// Its credits, its artists' names alike.
    fn credits(&self) -> Vec<&str> {
        self.values(Field::Artist)
            .chain(self.values(Field::Artists))
            .collect()
    }
}

/// What a source was held to and how each compared: `Some(true)` agrees,
/// `Some(false)` disagrees, `None` said nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Evidence {
    identifier: bool,
    title: Option<bool>,
    credits: Option<bool>,
    length: Option<bool>,
    release: Option<bool>,
    /// Its release group against those its album units are on: unknown
    /// unless each unit is on one, and against none where the files name
    /// no album, which setting its release aside would leave the song.
    unit: Option<bool>,
    /// Its title names one of [`VOICELESS`] that none of the titles it
    /// is held to names.
    voiceless: bool,
}

impl Evidence {
    fn points(self) -> i64 {
        let side = |e: Option<bool>, agrees: i64, disagrees: i64| match e {
            Some(true) => agrees,
            Some(false) => disagrees,
            None => 0,
        };
        i64::from(self.identifier) * IDENTIFIER
            + side(self.title, TITLE_ALIKE, TITLE_APART)
            + side(self.credits, CREDIT_SHARED, NO_CREDIT_SHARED)
            + side(self.length, LENGTH_CLOSE, LENGTH_FAR)
            + side(self.release, RELEASE_ALIKE, RELEASE_APART)
            + side(self.unit, UNIT_ALIKE, UNIT_APART)
    }

    fn verdict(self) -> Verdict {
        let points = self.points();
        let unnamed = self.title == Some(false) && self.credits == Some(false);
        if ((unnamed || self.voiceless) && !self.identifier) || points <= OTHER_AT {
            Verdict::Other
        } else if points <= UNSURE_AT
            || (self.credits == Some(false) && self.release == Some(false))
            || (self.unit == Some(false) && self.release != Some(true))
        {
            Verdict::Unsure
        } else {
            Verdict::Same
        }
    }

    /// The evidence in words, as `status` says it.
    fn describe(self) -> Vec<String> {
        let mut said = Vec::new();
        if self.identifier {
            said.push("an ISRC or recording ID in common".to_string());
        }
        match (self.title, self.credits) {
            (Some(false), Some(false)) => said.push("no title or credit in common".to_string()),
            (title, credits) => {
                said.extend(title.map(|t| {
                    if t {
                        "the title agrees"
                    } else {
                        "another title"
                    }
                    .to_string()
                }));
                said.extend(credits.map(|c| {
                    if c {
                        "a credit in common"
                    } else {
                        "no credit in common"
                    }
                    .to_string()
                }));
            }
        }
        if self.voiceless {
            said.push("an instrumental or karaoke take no other title names".to_string());
        }
        said.extend(self.length.map(|l| {
            if l {
                "its length agrees"
            } else {
                "another length than the audio's"
            }
            .to_string()
        }));
        said.extend(self.release.map(|r| {
            if r {
                "on the files' album"
            } else {
                "on another album than the files'"
            }
            .to_string()
        }));
        said.extend(self.unit.map(|u| {
            if u {
                "on the release its album is on"
            } else {
                "on another release than its album's"
            }
            .to_string()
        }));
        said
    }
}

/// The vetting of each of a song's sources set aside, even in part, and
/// the evidence each vetted source was weighed by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Vets(BTreeMap<SourceKey, Vet>, BTreeMap<SourceKey, Evidence>);

/// What vetting made of one source set aside.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Vet {
    verdict: Verdict,
    /// Whether it gives recording IDs, as a source believed does unless
    /// its credits share no name with the others'.
    ids: bool,
    /// Whether its credits count as a believed source's: not where they
    /// share no name with the others' and its album is not the files'
    /// own, as a cover act's.
    credits: bool,
    /// Why, in words.
    why: String,
}

impl Vets {
    /// Each of `sources` held to the others' claims and to `hand`, the
    /// tags set by hand; a stated length to `audio_ms`, the chosen
    /// audio's; a release group to `units`, the release group each of the
    /// song's album units is on, if any.
    #[must_use]
    pub fn of(
        sources: &[Claims<'_>],
        hand: &BTreeMap<Field, Vec<String>>,
        audio_ms: Option<i64>,
        units: &[Option<&str>],
    ) -> Self {
        let (mut vets, mut weighed) = (BTreeMap::new(), BTreeMap::new());
        for s in sources
            .iter()
            .filter(|s| s.vetted && (!s.offers.is_empty() || s.length_ms.is_some()))
        {
            let others: Vec<&Claims<'_>> = sources.iter().filter(|o| o.key != s.key).collect();
            let e = evidence(s, &others, hand, audio_ms, units);
            weighed.insert(s.key.clone(), e);
            let verdict = e.verdict();
            let ids = verdict == Verdict::Same && e.credits != Some(false);
            let credits = ids || (verdict == Verdict::Same && e.release == Some(true));
            if !ids {
                let why = describe(s, e);
                let vet = Vet {
                    verdict,
                    ids,
                    credits,
                    why,
                };
                vets.insert(s.key.clone(), vet);
            }
        }
        let mut vets = Self(vets, weighed);
        vets.covers(sources);
        vets
    }

    /// Each vetted source and how each kind of evidence compared for it,
    /// by name, an identifier in common aside: `Some(true)` agreed,
    /// `Some(false)` disagreed, `None` said nothing.
    pub fn weighed(&self) -> impl Iterator<Item = (&SourceKey, [(&'static str, Option<bool>); 5])> {
        self.1.iter().map(|(key, e)| {
            let sides = [
                ("title", e.title),
                ("credits", e.credits),
                ("length", e.length),
                ("release", e.release),
                ("unit", e.unit),
            ];
            (key, sides)
        })
    }

    /// Set aside the vetted Cover Art Archive front of each release group
    /// or release a source set aside names and none believed does.
    fn covers(&mut self, sources: &[Claims<'_>]) {
        let ids = |believed: bool| -> BTreeSet<(&SourceKey, &str)> {
            sources
                .iter()
                .filter(|s| self.gives(s.key, Field::Album) == believed)
                .flat_map(|s| {
                    s.values(Field::MusicBrainzReleaseGroupId)
                        .chain(s.values(Field::MusicBrainzAlbumId))
                        .map(move |id| (s.key, id))
                })
                .collect()
        };
        let named: BTreeSet<&str> = ids(true).into_iter().map(|(_, id)| id).collect();
        let doubted = ids(false);
        for s in sources.iter().filter(|s| s.vetted) {
            let SourceKey::Remote { site, id } = s.key else {
                continue;
            };
            if site != COVERART || named.contains(id.as_str()) {
                continue;
            }
            if let Some((by, _)) = doubted.iter().find(|(_, d)| *d == id.as_str()) {
                let vet = Vet {
                    verdict: Verdict::Other,
                    ids: false,
                    credits: false,
                    why: format!("the cover of the album {by} names"),
                };
                self.0.insert(s.key.clone(), vet);
            }
        }
    }

    #[must_use]
    pub fn verdict(&self, key: &SourceKey) -> Verdict {
        self.0.get(key).map_or(Verdict::Same, |v| v.verdict)
    }

    /// Whether `key`'s credits rank as a believed source's: a record
    /// crediting no one the others do is believed for its artists only
    /// where its album is the files' own, as a concert's crediting each
    /// singer.
    #[must_use]
    pub fn credits(&self, key: &SourceKey) -> bool {
        self.0.get(key).is_none_or(|v| v.credits)
    }

    /// Whether `key` gives the song `field`: nothing once it names another
    /// recording, no release field while unsure, and no recording ID but
    /// where it is believed and shares a credit, or nothing says.
    #[must_use]
    pub fn gives(&self, key: &SourceKey, field: Field) -> bool {
        let Some(vet) = self.0.get(key) else {
            return true;
        };
        match (vet.verdict, field.scope()) {
            (_, Scope::RecordingId) => vet.ids,
            (Verdict::Same, _) => true,
            (Verdict::Unsure, scope) => scope != Scope::Release,
            (Verdict::Other, _) => false,
        }
    }

    /// Each source set aside and why, as `status` shows it.
    #[must_use]
    pub fn set_aside(&self) -> Vec<String> {
        self.aside()
            .into_iter()
            .map(|(key, why, _)| format!("{key}: {why}"))
            .collect()
    }

    /// Each source set aside, what of it and why, and whether it still
    /// gives its release.
    #[must_use]
    pub fn aside(&self) -> Vec<(&SourceKey, String, bool)> {
        self.0
            .iter()
            .map(|(key, vet)| {
                let why = match vet.verdict {
                    Verdict::Same => format!("its IDs, {}", vet.why),
                    Verdict::Unsure => format!("its release and IDs, {}", vet.why),
                    Verdict::Other => vet.why.clone(),
                };
                (key, why, vet.verdict == Verdict::Same)
            })
            .collect()
    }
}

/// `s` held to `others` and `hand`.
fn evidence(
    s: &Claims<'_>,
    others: &[&Claims<'_>],
    hand: &BTreeMap<Field, Vec<String>>,
    audio_ms: Option<i64>,
    units: &[Option<&str>],
) -> Evidence {
    let by_hand = |field: Field| -> Vec<&str> {
        hand.get(&field)
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect()
    };
    let each = |field: Field| -> Vec<Vec<&str>> {
        let of = |vetted: bool| -> Vec<Vec<&str>> {
            let hand = (!vetted).then(|| by_hand(field));
            (others.iter())
                .filter(|o| o.vetted == vetted)
                .map(|o| o.values(field).collect::<Vec<_>>())
                .chain(hand)
                .filter(|v| !v.is_empty())
                .collect()
        };
        let anchored = of(false);
        if anchored.is_empty() {
            of(true)
        } else {
            anchored
        }
    };
    let theirs = |field: Field| -> Vec<&str> { each(field).into_iter().flatten().collect() };
    let shares = |field: Field| {
        let mine: Vec<&str> = s.values(field).collect();
        theirs(field)
            .iter()
            .any(|t| mine.iter().any(|m| similar::caseless(m, t)))
    };
    let titles = theirs(Field::Title);
    let their_credits: Vec<&str> = theirs(Field::Artist)
        .into_iter()
        .chain(theirs(Field::Artists))
        .chain(titles.iter().flat_map(|t| similar::performers(t)))
        .collect();
    let files: Vec<&Claims<'_>> = others
        .iter()
        .copied()
        .filter(|o| o.key.file().is_some())
        .collect();
    let of_files = |field: Field| -> Vec<Vec<&str>> {
        files
            .iter()
            .map(|f| f.values(field).collect::<Vec<_>>())
            .filter(|v| !v.is_empty())
            .collect()
    };
    let mine = |field: Field| -> Vec<&str> { s.values(field).collect() };
    let voiceless = |titles: &[&str]| -> BTreeSet<&'static str> {
        (titles.iter())
            .flat_map(|t| similar::versions_in(t))
            .filter(|v| VOICELESS.contains(v))
            .collect()
    };
    let unsung = voiceless(&mine(Field::Title));
    let held_to = each(Field::Title);
    let release = match similar::titles_agree(&mine(Field::Album), &of_files(Field::Album), true) {
        Some(false) => {
            let artists: Vec<&str> = of_files(Field::AlbumArtist).concat();
            let artist = similar::credits_agree(&mine(Field::AlbumArtist), &artists);
            (artist != Some(true)).then_some(false)
        }
        alike => alike,
    };
    Evidence {
        identifier: shares(Field::Isrc) || shares(Field::MusicBrainzTrackId),
        title: similar::titles_agree(&mine(Field::Title), &held_to, false),
        credits: if shares(Field::MusicBrainzArtistId) {
            Some(true)
        } else {
            similar::credits_agree(&s.credits(), &their_credits)
        },
        length: s.length_ms.zip(audio_ms).and_then(|(stated, heard)| {
            let gap = (i64::try_from(stated).unwrap_or(i64::MAX) - heard).abs();
            match gap {
                g if g <= CLOSE_MS => Some(true),
                g if g >= FAR_MS => Some(false),
                _ => None,
            }
        }),
        release,
        unit: {
            let groups = mine(Field::MusicBrainzReleaseGroupId);
            if groups.is_empty() || units.is_empty() {
                None
            } else if units.iter().flatten().any(|u| groups.contains(u)) {
                Some(true)
            } else {
                let named = !of_files(Field::Album).is_empty();
                (named && units.iter().all(Option::is_some)).then_some(false)
            }
        },
        voiceless: !unsung.is_empty()
            && !held_to.is_empty()
            && held_to.iter().all(|t| voiceless(t).is_disjoint(&unsung)),
    }
}

/// `s` and the evidence it was set aside on, as `titled "…" by …: …`.
fn describe(s: &Claims<'_>, e: Evidence) -> String {
    let title = s
        .values(Field::Title)
        .next()
        .map(|t| format!("titled \"{t}\""));
    let artist = s.values(Field::Artist).next().map(|a| format!("by {a}"));
    let album = s.values(Field::Album).next().map(|a| format!("on \"{a}\""));
    let names: Vec<String> = [title, artist, album].into_iter().flatten().collect();
    let said = e.describe().join(", ");
    match names.join(" ") {
        names if names.is_empty() => said,
        names => format!("{names}: {said}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUP: &str = "00000000-0000-0000-0000-0000000000b1";

    fn record() -> SourceKey {
        SourceKey::parse("musicbrainz:00000000-0000-0000-0000-000000000001").unwrap()
    }

    fn cover() -> SourceKey {
        SourceKey::parse(&format!("coverart:{GROUP}")).unwrap()
    }

    /// `Lantern Weather` by Marlo Venn on `Rooms of Salt`, as a file tags it.
    const FILE: [(Field, &str); 4] = [
        (Field::Title, "Lantern Weather"),
        (Field::Artist, "Marlo Venn"),
        (Field::Album, "Rooms of Salt"),
        (Field::AlbumArtist, "Marlo Venn"),
    ];

    fn claims<'a>(
        key: &'a SourceKey,
        offered: &'a [(Field, Vec<String>)],
        vetted: bool,
        length_ms: Option<u64>,
    ) -> Claims<'a> {
        Claims {
            key,
            offers: offered.iter().map(|(f, v)| (*f, v.as_slice())).collect(),
            vetted,
            length_ms,
        }
    }

    /// The vetting of a record offering `offers` and stating `length_ms`,
    /// beside the cover of [`GROUP`] and a file with audio offering `file`,
    /// where it offers anything, against the tags `hand` sets.
    fn vet(
        offers: &[(Field, &str)],
        length_ms: Option<u64>,
        file: &[(Field, &str)],
        hand: &[(Field, &str)],
    ) -> Vets {
        vet_in(offers, length_ms, file, hand, &[])
    }

    /// [`vet`] in album units on `units`.
    fn vet_in(
        offers: &[(Field, &str)],
        length_ms: Option<u64>,
        file: &[(Field, &str)],
        hand: &[(Field, &str)],
        units: &[Option<&str>],
    ) -> Vets {
        let values = |fields: &[(Field, &str)]| -> Vec<(Field, Vec<String>)> {
            fields
                .iter()
                .map(|(f, v)| (*f, vec![(*v).to_string()]))
                .collect()
        };
        let (offers, file, hand) = (values(offers), values(file), values(hand));
        let (keys, the_file) = ([record(), cover()], SourceKey::Manual("a.flac".into()));
        let mut all = vec![
            claims(&keys[0], &offers, true, length_ms),
            claims(&keys[1], &[], true, None),
        ];
        if !file.is_empty() {
            all.push(claims(&the_file, &file, false, None));
        }
        Vets::of(&all, &hand.into_iter().collect(), Some(241_000), units)
    }

    #[test]
    fn a_record_of_another_title_by_another_artist_names_another_recording() {
        let other = [
            (Field::Title, "Night Ferry"),
            (Field::Artist, "Kiri Hoshino"),
            (Field::Album, "Harbor Lights"),
            (Field::MusicBrainzReleaseGroupId, GROUP),
        ];
        let vets = vet(&other, Some(241_000), &FILE, &[]);
        assert_eq!(vets.verdict(&record()), Verdict::Other);
        assert_eq!(vets.verdict(&cover()), Verdict::Other, "its album's cover");
        assert_eq!(
            vets.set_aside()[1],
            format!(
                "{}: titled \"Night Ferry\" by Kiri Hoshino on \"Harbor Lights\": \
                 no title or credit in common, its length agrees, on another album than the files'",
                record()
            )
        );
        let named_too = [
            FILE.as_slice(),
            &[(Field::MusicBrainzReleaseGroupId, GROUP)],
        ]
        .concat();
        let vets = vet(&other, None, &named_too, &[]);
        assert_eq!(
            vets.verdict(&cover()),
            Verdict::Same,
            "the files' album too"
        );
        let vets = vet(&other[..2], None, &[], &FILE[..2]);
        assert_eq!(
            vets.verdict(&record()),
            Verdict::Other,
            "held to tags set by hand"
        );
    }

    #[test]
    fn a_tribute_act_s_record_gives_no_release_whose_title_agrees() {
        let tribute = [
            (Field::Title, "Lantern Weather"),
            (Field::Artist, "Marlo Venn Tribute Band"),
            (Field::Album, "A Tribute to Marlo Venn"),
            (Field::AlbumArtist, "Marlo Venn Tribute Band"),
            (Field::MusicBrainzReleaseGroupId, GROUP),
        ];
        let vets = vet(&tribute, None, &FILE, &[]);
        assert_eq!(vets.verdict(&record()), Verdict::Unsure);
        assert!(vets.gives(&record(), Field::Title));
        assert!(!vets.gives(&record(), Field::Album));
        assert!(!vets.gives(&record(), Field::MusicBrainzTrackId));
        assert_eq!(vets.verdict(&cover()), Verdict::Other);

        let on_its_album = [FILE[0], FILE[1], (Field::Album, "A Tribute to Marlo Venn")];
        let vets = vet(&tribute, None, &on_its_album, &[]);
        assert_eq!(
            vets.verdict(&record()),
            Verdict::Same,
            "as a concert's singers"
        );
        assert!(vets.gives(&record(), Field::Album));
        assert!(
            !vets.gives(&record(), Field::MusicBrainzTrackId),
            "another's ID"
        );
        assert!(vets.set_aside()[0].contains(": its IDs, titled"));
    }

    #[test]
    fn what_no_rule_compares_says_nothing_and_a_lone_record_is_believed() {
        let kanji = [
            (Field::Title, "夜の船"),
            (Field::Artist, "マルロ"),
            (Field::Album, "塩の部屋"),
        ];
        assert_eq!(
            vet(&kanji, None, &FILE, &[]).verdict(&record()),
            Verdict::Same
        );
        let alone = vet(&kanji, Some(100_000), &[], &[]);
        assert_eq!(
            alone.verdict(&record()),
            Verdict::Unsure,
            "a length far off"
        );
        assert!(
            vet(&FILE, None, &[], &[]).set_aside().is_empty(),
            "never held to itself"
        );
    }

    #[test]
    fn a_record_of_an_instrumental_no_other_title_names_names_another_recording() {
        let instrumental = [
            (Field::Title, "Lantern Weather (Instrumental)"),
            (Field::Artist, "Marlo Venn"),
            (Field::Album, "Rooms of Salt"),
        ];
        let vets = vet(&instrumental, Some(241_000), &FILE, &[]);
        assert_eq!(
            vets.verdict(&record()),
            Verdict::Other,
            "whatever its artist, album and length say"
        );
        assert!(
            vets.set_aside()[0].contains("an instrumental or karaoke take no other title names"),
            "{:?}",
            vets.set_aside()
        );
        let hangul = [
            (Field::Title, "랜턴 웨더 (Instrumental)"),
            (Field::Artist, "마를로"),
            (Field::Album, "Rooms of Salt"),
        ];
        assert_eq!(
            vet(&hangul, Some(241_000), &FILE, &[]).verdict(&record()),
            Verdict::Other,
            "a title in other letters"
        );
        let off_vocal = [
            (Field::Title, "Lantern Weather (Off Vocal)"),
            FILE[1],
            FILE[2],
        ];
        assert_eq!(
            vet(&instrumental, Some(241_000), &off_vocal, &[]).verdict(&record()),
            Verdict::Same,
            "the files' own instrumental"
        );
        let isrc = (Field::Isrc, "XX0000000001");
        let (shared, tagged) = (
            [&instrumental[..], &[isrc]].concat(),
            [&FILE[..], &[isrc]].concat(),
        );
        assert_ne!(
            vet(&shared, Some(241_000), &tagged, &[]).verdict(&record()),
            Verdict::Other,
            "an identifier in common"
        );
        assert_eq!(
            vet(&instrumental, Some(241_000), &[], &[]).verdict(&record()),
            Verdict::Same,
            "no title to hold it to"
        );
    }

    #[test]
    fn an_identifier_outweighs_names_and_another_title_by_the_artist_is_unsure() {
        let other = [
            (Field::Title, "Night Ferry"),
            (Field::Artist, "Marlo Venn"),
            (Field::Isrc, "XX0000000001"),
        ];
        let tagged = [FILE.as_slice(), &[(Field::Isrc, "xx0000000001")]].concat();
        assert_eq!(
            vet(&other, None, &tagged, &[]).verdict(&record()),
            Verdict::Same
        );
        assert_eq!(
            vet(&other, None, &FILE, &[]).verdict(&record()),
            Verdict::Unsure
        );
    }

    #[test]
    fn the_release_its_album_is_on_counts_both_ways() {
        let cut_short = [
            (Field::Title, "Night Ferry"),
            (Field::Artist, "マルロ"),
            (Field::Album, "Rooms of Salt"),
            (Field::MusicBrainzReleaseGroupId, GROUP),
        ];
        let alone = vet(&cut_short, None, &FILE, &[]);
        assert_eq!(alone.verdict(&record()), Verdict::Unsure);
        let held = vet_in(&cut_short, None, &FILE, &[], &[Some(GROUP)]);
        assert_eq!(held.verdict(&record()), Verdict::Same, "its unit's release");

        let named = [
            FILE.as_slice(),
            &[(Field::MusicBrainzReleaseGroupId, GROUP)],
        ]
        .concat();
        let elsewhere = "00000000-0000-0000-0000-0000000000c2";
        let harbor = [FILE[0], FILE[1], (Field::Album, "Harbor Lights")];
        let apart = vet_in(&named, None, &harbor, &[], &[Some(elsewhere)]);
        assert_eq!(apart.verdict(&record()), Verdict::Unsure);
        assert!(apart.gives(&record(), Field::Title));
        assert!(!apart.gives(&record(), Field::Album));
        assert_eq!(
            apart.verdict(&cover()),
            Verdict::Other,
            "its release's cover"
        );
        assert!(
            apart.set_aside()[1].ends_with("on another release than its album's"),
            "{:?}",
            apart.set_aside()
        );
        for (file, units, why) in [
            (
                &FILE[..],
                &[Some(elsewhere)][..],
                "its album the files' own",
            ),
            (
                &harbor[..],
                &[Some(elsewhere), None],
                "another unit on no release",
            ),
            (&FILE[..2], &[Some(elsewhere)], "files naming no album"),
        ] {
            let vets = vet_in(&named, None, file, &[], units);
            assert_eq!(vets.verdict(&record()), Verdict::Same, "{why}");
        }
        assert_eq!(
            vet_in(&FILE, None, &FILE[..2], &[], &[Some(elsewhere)]).verdict(&record()),
            Verdict::Same,
            "a record naming no release group"
        );
    }
}
