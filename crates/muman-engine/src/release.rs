//! Which release each song is on, of the releases its recording is on.
//!
//! A song's MusicBrainz record holds one release: the one MusicBrainz
//! ranks first for the recording, then the earliest. So the songs of one
//! rip land on the album, a compilation, a box set and a live video of
//! the same concert, side by side; measured on a library of about two
//! thousand songs, one concert's 33 songs were split 19 to 14 between its
//! album and its video. An AcoustID lookup keeps every recording a print
//! names with the releases each is on ([`crate::acoustid::Candidate`]),
//! and this pass chooses among them, `[release] prefer` saying how:
//!
//! - `album`: the songs of one album unit ([`crate::album::units_of`]),
//!   the files of one folder under one album name or the songs of one
//!   `[[album]]`, are put on one release, the one that holds
//!   [`MAJORITY`] of them or more: of those, the one that holds the most,
//!   then whose title is the unit's own album's, whose track numbers are
//!   most often the files' own, that lacks the fewest tracks, an album
//!   before an EP before a single and one of no secondary type first,
//!   then the earliest; a unit [`MAJORITY`] of whose songs are on one
//!   release already stays on it, unless another holds more of them or
//!   `avoid` names its kind. Each song takes the track it is on there, songs
//!   matched to tracks by least cost ([`assign`]), its track numbers and
//!   the recording's length the costs, so two songs never share a track.
//! - `original`: each song is put on the first album its recording came
//!   out on, of no secondary type.
//!
//! A release whose types `[release] avoid` names is chosen only where no
//! other would be. A song whose album or release is set by hand is left
//! out, and so is a song on the release chosen already. A song whose
//! `release` pins it to one of its recording's releases is put on it,
//! and counts for no album unit.
//!
//! A song moved takes the release fields this pass knows from its
//! record's place on the release ([`Pick::offers`]): album, release and
//! release group IDs, date, country, types, track and disc numbers and
//! totals, and the recording's ID where the release holds another
//! recording of the print than the record's. Its record's other release
//! fields are left out, as another release's, and the album pass gives
//! it those its album's other songs on the release have ([`crate::album`]).

use std::collections::{BTreeMap, BTreeSet};

use crate::acoustid::{Candidate, CandidateRelease};
use crate::facts::Facts;
use crate::manifest::Song;
use crate::resolve::Resolved;
use crate::settings::{Release, ReleasePrefer};
use crate::similar;
use crate::source::SourceKey;
use crate::tags::{Field, Offer, Offers};

/// The share of an album unit's songs a release must hold, in percent,
/// to be the unit's: under it, the songs are more likely on several
/// releases, as a folder of a box set's discs.
pub const MAJORITY: usize = 60;

/// What moving a song to another release gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    /// The record whose release fields are given in place of its own.
    pub key: SourceKey,
    /// The release fields it gives, and the recording's ID where the
    /// release holds another recording than the record's.
    pub offers: Offers,
    /// Why, for `status`.
    pub why: String,
}

/// One place a song's recording can be: a track of a release.
#[derive(Clone, Copy)]
struct Place<'a> {
    /// The record whose lookup named it.
    key: &'a SourceKey,
    recording: &'a Candidate,
    release: &'a CandidateRelease,
}

impl Place<'_> {
    fn at(&self) -> (u32, u32) {
        (
            self.release.disc.unwrap_or(1),
            self.release.track.unwrap_or(0),
        )
    }
}

/// One song as this pass reads it.
struct Member<'a> {
    n: usize,
    /// The release it is on now.
    on: Option<&'a str>,
    places: Vec<Place<'a>>,
    /// The track and disc its files number it.
    numbered: Option<(u32, u32)>,
    /// Its audio's length.
    ms: Option<i64>,
    /// The recording its record is.
    recording: Option<&'a str>,
    /// The release its `release` pins it to.
    pin: Option<&'a str>,
}

/// The release each song of `planned` is moved to, by its place in the
/// song list; `candidates` gives the recordings the AcoustID lookup that
/// found a record named, by the song and the record.
#[must_use]
pub fn choose<'a>(
    planned: &'a [(usize, Resolved)],
    songs: &'a [Song],
    facts: &'a BTreeMap<SourceKey, Facts>,
    candidates: &dyn Fn(&'a Song, &'a SourceKey) -> &'a [Candidate],
    settings: &Release,
) -> BTreeMap<usize, Pick> {
    let members: Vec<Member<'a>> = planned
        .iter()
        .filter_map(|(n, r)| member(*n, r, songs.get(*n)?, facts, candidates))
        .collect();
    let mut picks = match settings.prefer {
        ReleasePrefer::Album => by_album(&members, songs, facts, settings),
        ReleasePrefer::Original => members
            .iter()
            .filter(|m| m.pin.is_none())
            .filter_map(|m| Some((m.n, original(m, settings)?)))
            .collect(),
    };
    for m in members.iter().filter(|m| m.pin.is_some() && m.on != m.pin) {
        let pinned = (m.places.iter()).find(|p| Some(p.release.id.as_str()) == m.pin);
        if let Some(place) = pinned {
            let why = format!(
                "on \"{}\", the release it is pinned to",
                place.release.title
            );
            picks.insert(m.n, pick(m, place, why));
        }
    }
    picks
}

fn member<'a>(
    n: usize,
    r: &'a Resolved,
    song: &'a Song,
    facts: &'a BTreeMap<SourceKey, Facts>,
    candidates: &dyn Fn(&'a Song, &'a SourceKey) -> &'a [Candidate],
) -> Option<Member<'a>> {
    let tag = |f: Field| {
        r.plan
            .tags
            .iter()
            .zip(&r.why.tags)
            .find(|((k, _), _)| k == f.vorbis())
            .map(|((_, v), why)| (v.first().map(String::as_str), why.from.as_str()))
    };
    let by_hand =
        |f: Field| tag(f).is_some_and(|(_, from)| matches!(from, "song.tags" | "album.tags"));
    if by_hand(Field::Album) || by_hand(Field::MusicBrainzAlbumId) {
        return None;
    }
    // A record vetting stops from giving its release names none here.
    let aside = |key: &SourceKey| {
        (r.why.doubts.iter()).any(|d| {
            matches!(d, crate::resolve::Doubt::SetAside { key: k, release: false, .. } if k == key)
        })
    };
    let records = song.sources.iter().filter(|k| {
        matches!(k, SourceKey::Remote { site, .. } if site == crate::provider::MUSICBRAINZ)
            && !aside(k)
    });
    let places: Vec<Place<'a>> = records
        .flat_map(|key| {
            candidates(song, key).iter().flat_map(move |recording| {
                (recording.releases.iter()).map(move |release| Place {
                    key,
                    recording,
                    release,
                })
            })
        })
        .collect();
    let first = |key: &SourceKey, f: Field| -> Option<&'a str> {
        facts
            .get(key)?
            .tags
            .get(&f)?
            .values
            .first()
            .map(String::as_str)
    };
    let number = |key: &SourceKey, f: Field| first(key, f).and_then(|v| v.parse::<u32>().ok());
    let numbered = song
        .sources
        .iter()
        .filter(|k| k.file().is_some())
        .find_map(|k| {
            Some((
                number(k, Field::Disc).unwrap_or(1),
                number(k, Field::Track)?,
            ))
        });
    let recording = places
        .first()
        .and_then(|p| first(p.key, Field::MusicBrainzTrackId));
    let ms = facts
        .get(&r.plan.audio.key)
        .and_then(Facts::length)
        .map(|s| (s * 1000.0).round() as i64);
    Some(Member {
        n,
        on: tag(Field::MusicBrainzAlbumId).and_then(|(v, _)| v),
        places,
        numbered,
        ms,
        recording,
        pin: song.release.as_deref(),
    })
}

/// Whether `release` is of a kind `[release] avoid` names.
fn avoided(release: &CandidateRelease, settings: &Release) -> bool {
    release
        .types
        .iter()
        .any(|t| settings.avoid.iter().any(|a| a.eq_ignore_ascii_case(t)))
}

/// How an album ranks among releases by its types, smaller first: an
/// album, an EP, a single, anything else; then whether it has a
/// secondary type, as a compilation or a live album.
fn kind(release: &CandidateRelease) -> (u8, bool) {
    let primary = match release.types.first().map(|t| t.to_ascii_lowercase()) {
        Some(t) if t == "album" => 0,
        Some(t) if t == "ep" => 1,
        Some(t) if t == "single" => 2,
        _ => 3,
    };
    (primary, release.types.len() > 1)
}

/// The first album `m`'s recording came out on: of no secondary type,
/// the earliest.
fn original(m: &Member<'_>, settings: &Release) -> Option<Pick> {
    let place = m
        .places
        .iter()
        .filter(|p| kind(p.release) == (0, false))
        .min_by_key(|p| {
            let date = crate::record::date_key(p.release.date.as_deref());
            (avoided(p.release, settings), date, p.release.id.as_str())
        })?;
    (m.on != Some(place.release.id.as_str())).then(|| {
        pick(
            m,
            place,
            format!(
                "on \"{}\", the first album its recording came out on",
                place.release.title
            ),
        )
    })
}

/// The release each album unit is on, its songs matched to its tracks.
fn by_album(
    members: &[Member<'_>],
    songs: &[Song],
    facts: &BTreeMap<SourceKey, Facts>,
    settings: &Release,
) -> BTreeMap<usize, Pick> {
    let mut units: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, m) in members.iter().enumerate().filter(|(_, m)| m.pin.is_none()) {
        for unit in songs
            .get(m.n)
            .map(|s| crate::album::units_of(s, facts))
            .unwrap_or_default()
        {
            units.entry(unit).or_default().push(i);
        }
    }
    let mut order: Vec<(&String, &Vec<usize>)> = units.iter().collect();
    order.sort_by_key(|(key, of)| (std::cmp::Reverse(of.len()), *key));
    let mut done: BTreeSet<usize> = BTreeSet::new();
    let mut picks = BTreeMap::new();
    for (_, of) in order {
        let unit: Vec<&Member<'_>> = of
            .iter()
            .filter(|i| !done.contains(*i))
            .map(|&i| &members[i])
            .filter(|m| !m.places.is_empty() || m.on.is_some())
            .collect();
        if unit.len() < 2 {
            continue;
        }
        let best = best(&unit, songs, facts, settings);
        // A unit on one release already stays, unless another holds more
        // of it or `avoid` names the kind it is on.
        let chosen = match settled(&unit) {
            None => best,
            Some((on, count)) => match known(&unit, on) {
                Some((release, held))
                    if !avoided(release, settings) && best.is_none_or(|(_, more)| more <= held) =>
                {
                    Some((release, held))
                }
                Some(_) => best,
                None if best.is_some_and(|(_, more)| more > count) => best,
                None => continue,
            },
        };
        let Some((release, held)) = chosen else {
            continue;
        };
        done.extend(of);
        let why = format!(
            "on \"{}\", the release {held} of its album's {} songs are on",
            release.title,
            unit.len()
        );
        for (m, place) in assign(&unit, &release.id) {
            if m.on != Some(release.id.as_str()) {
                picks.insert(m.n, pick(m, &place, why.clone()));
            }
        }
    }
    picks
}

/// The release [`MAJORITY`] of `unit` or more are on already, and how
/// many are.
fn settled<'a>(unit: &[&Member<'a>]) -> Option<(&'a str, usize)> {
    let mut on: BTreeMap<&str, usize> = BTreeMap::new();
    for release in unit.iter().filter_map(|m| m.on) {
        *on.entry(release).or_default() += 1;
    }
    on.into_iter()
        .find(|(_, n)| n * 100 >= MAJORITY * unit.len())
}

/// Release `id` as the places of `unit` know it, with how many of its
/// songs are on it or can be; none where no place names it, so no song
/// can be moved to it.
fn known<'a>(unit: &[&Member<'a>], id: &str) -> Option<(&'a CandidateRelease, usize)> {
    let release = (unit.iter())
        .flat_map(|m| &m.places)
        .find(|p| p.release.id == id)?
        .release;
    let held = (unit.iter())
        .filter(|m| m.on == Some(id) || m.places.iter().any(|p| p.release.id == id))
        .count();
    Some((release, held))
}

/// The release of `unit` that holds [`MAJORITY`] of its songs or more,
/// ranked as the module says, with how many it holds.
fn best<'a>(
    unit: &[&Member<'a>],
    songs: &[Song],
    facts: &BTreeMap<SourceKey, Facts>,
    settings: &Release,
) -> Option<(&'a CandidateRelease, usize)> {
    let mut holds: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    let mut known: BTreeMap<&str, &'a CandidateRelease> = BTreeMap::new();
    for m in unit {
        for p in &m.places {
            holds.entry(p.release.id.as_str()).or_default().insert(m.n);
            known.entry(p.release.id.as_str()).or_insert(p.release);
        }
        if let Some(on) = m.on {
            holds.entry(on).or_default().insert(m.n);
        }
    }
    let albums: Vec<Vec<&str>> = unit
        .iter()
        .filter_map(|m| songs.get(m.n))
        .flat_map(|s| &s.sources)
        .filter(|k| k.file().is_some())
        .filter_map(|k| facts.get(k)?.tags.get(&Field::Album))
        .map(|o| o.values.iter().map(String::as_str).collect())
        .collect();
    let numbered = |id: &str| {
        unit.iter()
            .filter(|m| {
                m.places
                    .iter()
                    .any(|p| p.release.id == id && m.numbered == Some(p.at()))
            })
            .count()
    };
    known
        .into_iter()
        .filter_map(|(id, release)| {
            let held = holds.get(id)?.len();
            (held * 100 >= MAJORITY * unit.len()).then_some((release, held))
        })
        .min_by_key(|(release, held)| {
            let named = match similar::titles_agree(&[release.title.as_str()], &albums, true) {
                Some(true) => 0,
                None => 1,
                Some(false) => 2,
            };
            let lacks = release
                .tracks
                .filter(|_| release.discs.is_none_or(|d| d <= 1))
                .map_or(0, |t| (t as usize).saturating_sub(*held));
            (
                avoided(release, settings),
                std::cmp::Reverse(*held),
                named,
                std::cmp::Reverse(numbered(&release.id)),
                lacks,
                kind(release),
                crate::record::date_key(release.date.as_deref()),
                release.id.clone(),
            )
        })
}

/// What a song's place on a track costs where its files number it
/// otherwise, a disc apart counting as much.
const NUMBERED_OTHERWISE: i64 = 4;

/// What every whole [`LENGTH_STEP_MS`] between the song's audio and the
/// recording on a track costs.
const LENGTH_STEP_MS: i64 = 2_000;

/// The most a length apart costs.
const LENGTH_MOST: i64 = 10;

/// The songs of `unit` that can be on release `id`, each matched to one
/// of its tracks at least cost: a track numbered otherwise than the
/// song's files number it costs [`NUMBERED_OTHERWISE`], and each
/// [`LENGTH_STEP_MS`] the recording's length is from the song's audio
/// one more, to [`LENGTH_MOST`].
fn assign<'m, 'a>(unit: &[&'m Member<'a>], id: &str) -> Vec<(&'m Member<'a>, Place<'a>)> {
    let rows: Vec<(&Member<'a>, Vec<Place<'a>>)> = unit
        .iter()
        .map(|m| {
            let on: Vec<Place<'a>> = m
                .places
                .iter()
                .copied()
                .filter(|p| p.release.id == id)
                .collect();
            (*m, on)
        })
        .filter(|(_, on)| !on.is_empty())
        .collect();
    let tracks: Vec<(u32, u32)> = rows
        .iter()
        .flat_map(|(_, on)| on.iter().map(Place::at))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let cost = |m: &Member<'_>, p: &Place<'_>| {
        let numbered = match m.numbered {
            Some(at) if at != p.at() => NUMBERED_OTHERWISE,
            _ => 0,
        };
        let length = m.ms.zip(p.recording.length_ms).map_or(0, |(heard, said)| {
            ((i64::try_from(said).unwrap_or(i64::MAX) - heard).abs() / LENGTH_STEP_MS)
                .min(LENGTH_MOST)
        });
        numbered + length
    };
    let costs: Vec<Vec<Option<i64>>> = rows
        .iter()
        .map(|(m, on)| {
            tracks
                .iter()
                .map(|t| on.iter().filter(|p| p.at() == *t).map(|p| cost(m, p)).min())
                .collect()
        })
        .collect();
    least_cost(&costs)
        .into_iter()
        .enumerate()
        .filter_map(|(row, col)| {
            let (m, on) = &rows[row];
            let track = tracks[col?];
            let place = on
                .iter()
                .filter(|p| p.at() == track)
                .min_by_key(|p| (cost(m, p), p.recording.id.as_str()))?;
            Some((*m, *place))
        })
        .collect()
}

/// The column each row of `costs` takes, each column taken once at
/// most, at least total cost, as many rows as can be placed placed: the
/// Hungarian method over a square matrix, a missing cost dearer than any
/// sum of the others. `None` for a row no column takes at a cost.
fn least_cost(costs: &[Vec<Option<i64>>]) -> Vec<Option<usize>> {
    let rows = costs.len();
    let cols = costs.first().map_or(0, Vec::len);
    let n = rows.max(cols);
    let known: i64 = costs.iter().flatten().flatten().sum();
    let missing = known
        .saturating_add(1)
        .saturating_mul(i64::try_from(n).unwrap_or(1) + 1);
    let at = |i: usize, j: usize| -> i64 {
        if i < rows && j < cols {
            costs[i][j].unwrap_or(missing)
        } else {
            0
        }
    };
    // Potentials and the matching, 1-based with 0 the free row.
    let (mut u, mut v) = (vec![0_i64; n + 1], vec![0_i64; n + 1]);
    let (mut matched, mut way) = (vec![0_usize; n + 1], vec![0_usize; n + 1]);
    for i in 1..=n {
        matched[0] = i;
        let mut j0 = 0;
        let mut least = vec![i64::MAX; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[j0] = true;
            let i0 = matched[j0];
            let (mut delta, mut j1) = (i64::MAX, 0);
            for j in 1..=n {
                if !used[j] {
                    let reduced = at(i0 - 1, j - 1) - u[i0] - v[j];
                    if reduced < least[j] {
                        least[j] = reduced;
                        way[j] = j0;
                    }
                    if least[j] < delta {
                        delta = least[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=n {
                if used[j] {
                    u[matched[j]] += delta;
                    v[j] -= delta;
                } else {
                    least[j] -= delta;
                }
            }
            j0 = j1;
            if matched[j0] == 0 {
                break;
            }
        }
        while j0 != 0 {
            let j1 = way[j0];
            matched[j0] = matched[j1];
            j0 = j1;
        }
    }
    let mut out = vec![None; rows];
    for j in 1..=n {
        let i = matched[j];
        if i >= 1 && i <= rows && j <= cols && costs[i - 1][j - 1].is_some() {
            out[i - 1] = Some(j - 1);
        }
    }
    out
}

/// What `place` gives `m`: the release's fields, and the recording's ID
/// where it is another than the record's.
fn pick(m: &Member<'_>, place: &Place<'_>, why: String) -> Pick {
    let r = place.release;
    let mut offers = Offers::new();
    let mut give = |field: Field, values: Vec<String>| {
        if !values.is_empty() {
            offers.insert(
                field,
                Offer {
                    values,
                    structured: true,
                    also: Vec::new(),
                },
            );
        }
    };
    let number = |n: Option<u32>| n.map(|n| n.to_string()).into_iter().collect();
    give(Field::Album, vec![r.title.clone()]);
    give(Field::MusicBrainzAlbumId, vec![r.id.clone()]);
    give(
        Field::MusicBrainzReleaseGroupId,
        r.group_id.clone().into_iter().collect(),
    );
    give(Field::Date, r.date.clone().into_iter().collect());
    give(
        Field::ReleaseCountry,
        r.country.clone().into_iter().collect(),
    );
    give(
        Field::ReleaseType,
        r.types.iter().map(|t| t.to_lowercase()).collect(),
    );
    give(Field::Track, number(r.track));
    give(Field::TrackTotal, number(r.tracks));
    give(Field::Disc, number(r.disc));
    give(Field::DiscTotal, number(r.discs));
    let recording = place.recording.id.to_string();
    if m.recording != Some(recording.as_str()) {
        give(Field::MusicBrainzTrackId, vec![recording]);
    }
    Pick {
        key: place.key.clone(),
        offers,
        why,
    }
}

#[cfg(test)]
mod tests;
