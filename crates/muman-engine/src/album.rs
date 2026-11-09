//! The songs of one album made to agree, once every song is resolved on
//! its own: which edition of the album they are on, how the album and
//! its artist are spelled, and its date.
//!
//! Each song picks its release from what its own sources offer, so the
//! songs of one album can land on different editions of it. MusicBrainz
//! keeps a recording's release as the best ranked of that recording's
//! own releases, then the earliest, and holds many albums as several
//! releases, each spelled and dated its own way: measured on a library of
//! about two thousand songs, 26 albums of 525 were split across folders
//! this way, one of seventeen songs on three releases titled with `’`,
//! with `'`, and dated a year later in another country. Files tagged
//! before muman split the same way where one is spelled `On The Border`
//! and its album's others `On the Border`, or one is dated 1976 and the
//! nineteen others on its release 2018.
//!
//! An album is a MusicBrainz release group, the album whichever its
//! edition, else an album artist and album as [`similar::fold`] compares
//! them, in one year. A song on no release group joins the one release
//! group of its folded name, so an upload naming the album alone joins
//! the album's songs whatever date it gives; two release groups of one
//! name, as two albums named for their artist, stay apart.
//!
//! - The edition: where an album's songs are on more than one release,
//!   the release most of them can be on wins, then as MusicBrainz ranks
//!   a recording's releases: official, no secondary type, album before
//!   EP before single, then the earliest and fullest date. A song whose
//!   MusicBrainz record holds that release among the album's other
//!   editions ([`crate::musicbrainz::Record::editions`]) takes its
//!   release fields from it ([`Pick::edition`]). One on another recording
//!   than the release holds keeps its own.
//! - The names: an album or album artist spelled differently by songs of
//!   one album, but alike once folded, is spelled as most of them spell
//!   it, the first song in the song list breaking a tie.
//! - The release: songs on one release, by its MusicBrainz ID, give each
//!   other the fields of the whole release, [`WHOLE`], as most of them
//!   have it, a song lacking one taking it; a song on none in an album of
//!   at most one release takes its date alone.
//!
//! - The cover and the audio ([`consistent`]): the design a pinned cover
//!   shows, else the one most of an album's songs show
//!   ([`crate::picture::alike`]), which a song showing another shows too
//!   from a source of its own that holds it; and the folder most of their
//!   audio comes from ([`origin_of`]), a
//!   disc's folder counting as its album's, which wins a song's audio
//!   between sources its measures rank alike, so an album plays from one
//!   rip, gapless and levelled alike. A song takes nothing from another
//!   song's sources: what each source held when its song was built stays
//!   that song's own record.
//!
//! A tag set by hand is never changed and never counts, and a song whose
//! album is set by hand is left out. A song resolved alone, as songs
//! merged in memory, takes nothing from its album.

use std::collections::{BTreeMap, BTreeSet};

use crate::facts::Facts;
use crate::manifest::Song;
use crate::picture::{self, Alike, Look};
use crate::resolve::{self, Resolved};
use crate::similar;
use crate::source::SourceKey;
use crate::tags::Field;

/// What a song takes from the other songs of its album.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pick {
    /// The source its release fields come from and the edition of that
    /// source's record they come from, by its place in the source's facts.
    pub edition: Option<(SourceKey, usize)>,
    /// The release it is moved to, of those its recording is on
    /// ([`crate::release`]), which an edition gives way to.
    pub release: Option<crate::release::Pick>,
    /// Values set over its own, as its album's songs agree on them.
    pub agreed: Vec<Agreed>,
    /// The design of the cover most of its album's songs show, which it
    /// shows too from a source of its own that holds it.
    pub cover: Option<Look>,
    /// The folder most of its album's audio comes from ([`origin_of`]),
    /// which wins between its sources its measures rank alike.
    pub audio_from: Option<String>,
}

/// One field's value as an album's songs agree on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agreed {
    pub field: Field,
    pub values: Vec<String>,
    /// Where it came from, for `status`.
    pub from: String,
}

/// The fields of a whole release, the same on each of its tracks: the
/// rest of [`crate::tags::Scope::Release`] is a track's or a medium's.
pub const WHOLE: [Field; 18] = [
    Field::Album,
    Field::AlbumArtist,
    Field::AlbumArtistSort,
    Field::AlbumSort,
    Field::Date,
    Field::OriginalDate,
    Field::ReleaseCountry,
    Field::ReleaseStatus,
    Field::ReleaseType,
    Field::Label,
    Field::CatalogNumber,
    Field::Barcode,
    Field::Asin,
    Field::Script,
    Field::Compilation,
    Field::DiscTotal,
    Field::MusicBrainzAlbumArtistId,
    Field::MusicBrainzReleaseGroupId,
];

/// Where a tag set by hand comes from.
const BY_HAND: [&str; 2] = ["song.tags", "album.tags"];

/// One resolved song as the album pass reads it.
struct Member<'a> {
    /// Its place in the song list.
    n: usize,
    r: &'a Resolved,
}

impl<'a> Member<'a> {
    /// `field`'s values and where they came from.
    fn tag(&self, field: Field) -> Option<(&'a [String], &'a str)> {
        let r = self.r;
        r.plan
            .tags
            .iter()
            .zip(&r.why.tags)
            .find(|((k, _), _)| k == field.vorbis())
            .map(|((_, v), why)| (v.as_slice(), why.from.as_str()))
    }

    fn first(&self, field: Field) -> Option<&'a str> {
        self.tag(field)?.0.first().map(String::as_str)
    }

    fn by_hand(&self, field: Field) -> bool {
        self.tag(field)
            .is_some_and(|(_, from)| BY_HAND.contains(&from))
    }

    /// Whether it is on an album its sources name, not set by hand.
    fn on_album(&self) -> bool {
        self.tag(Field::Album)
            .is_some_and(|(_, from)| from != resolve::SINGLE && !BY_HAND.contains(&from))
    }

    /// Its album artist and album, folded.
    fn name(&self) -> (String, String) {
        let fold = |f: Field| {
            self.tag(f)
                .map(|(v, _)| similar::fold(&v.join(" ")))
                .unwrap_or_default()
        };
        (fold(Field::AlbumArtist), fold(Field::Album))
    }

    fn year(&self) -> String {
        self.first(Field::Date)
            .map(|d| d.chars().take(4).collect())
            .unwrap_or_default()
    }
}

fn members(planned: &[(usize, Resolved)]) -> Vec<Member<'_>> {
    planned
        .iter()
        .map(|(n, r)| Member { n: *n, r })
        .filter(Member::on_album)
        .collect()
}

/// The release each song of an album that spans several takes, where its
/// record holds it as another edition: by the song's place in the song
/// list, the source and the edition.
#[must_use]
pub fn editions(
    planned: &[(usize, Resolved)],
    songs: &[Song],
    facts: &BTreeMap<SourceKey, Facts>,
) -> BTreeMap<usize, (SourceKey, usize)> {
    let mut groups: BTreeMap<&str, Vec<Placed<'_>>> = BTreeMap::new();
    for m in members(planned) {
        if m.by_hand(Field::MusicBrainzAlbumId) {
            continue;
        }
        let (Some(group), Some(release)) = (
            m.first(Field::MusicBrainzReleaseGroupId),
            m.first(Field::MusicBrainzAlbumId),
        ) else {
            continue;
        };
        let mut options = vec![Option_ {
            id: release,
            rank: rank_of(|f| m.tag(f).map(|(v, _)| v)),
            at: None,
        }];
        if let Some((key, f)) = album_source(&m, songs, facts) {
            for (at, offers) in f.editions.iter().enumerate() {
                let of = |field: Field| offers.get(&field).map(|o| o.values.as_slice());
                let first = |field: Field| of(field).and_then(|v| v.first()).map(String::as_str);
                if first(Field::MusicBrainzReleaseGroupId) == Some(group)
                    && let Some(id) = first(Field::MusicBrainzAlbumId)
                {
                    options.push(Option_ {
                        id,
                        rank: rank_of(of),
                        at: Some((key, at)),
                    });
                }
            }
        }
        groups
            .entry(group)
            .or_default()
            .push(Placed { n: m.n, options });
    }
    let mut picked = BTreeMap::new();
    for placed in groups.values() {
        let on: BTreeSet<&str> = placed.iter().map(|p| p.options[0].id).collect();
        if on.len() < 2 {
            continue;
        }
        let mut can: BTreeMap<&str, (usize, &Rank<'_>)> = BTreeMap::new();
        for option in placed.iter().flat_map(|p| &p.options) {
            can.entry(option.id).or_insert((0, &option.rank)).0 += 1;
        }
        let Some(best) = can
            .iter()
            .min_by(|a, b| {
                (std::cmp::Reverse(a.1.0), a.1.1, a.0).cmp(&(std::cmp::Reverse(b.1.0), b.1.1, b.0))
            })
            .map(|(id, _)| *id)
        else {
            continue;
        };
        for p in placed {
            if p.options[0].id == best {
                continue;
            }
            if let Some((key, at)) = p.options.iter().find(|o| o.id == best).and_then(|o| o.at) {
                picked.insert(p.n, (key.clone(), at));
            }
        }
    }
    picked
}

/// A song of a release group and the releases it can be on, its own
/// first.
struct Placed<'a> {
    n: usize,
    options: Vec<Option_<'a>>,
}

/// A release a song can be on.
struct Option_<'a> {
    id: &'a str,
    rank: Rank<'a>,
    /// The source and edition giving it; none for the song's own.
    at: Option<(&'a SourceKey, usize)>,
}

/// How MusicBrainz ranks a release, smaller first: official, no
/// secondary type, its primary type, then its date.
type Rank<'a> = (
    bool,
    bool,
    u8,
    (bool, &'a str, std::cmp::Reverse<usize>, &'a str),
);

fn rank_of<'a>(of: impl Fn(Field) -> Option<&'a [String]>) -> Rank<'a> {
    let first = |f: Field| of(f).and_then(|v| v.first()).map(String::as_str);
    let types = of(Field::ReleaseType).unwrap_or_default();
    let primary = match types.first().map(String::as_str) {
        Some("album") => 0,
        Some("ep") => 1,
        Some("single") => 2,
        _ => 3,
    };
    (
        first(Field::ReleaseStatus) != Some("official"),
        types.len() > 1,
        primary,
        crate::record::date_key(first(Field::Date)),
    )
}

/// The source a song's album came from, with its facts.
fn album_source<'a>(
    m: &Member<'_>,
    songs: &'a [Song],
    facts: &'a BTreeMap<SourceKey, Facts>,
) -> Option<(&'a SourceKey, &'a Facts)> {
    let (_, from) = m.tag(Field::Album)?;
    let key = songs
        .get(m.n)?
        .sources
        .iter()
        .find(|k| k.to_string() == from)?;
    Some((key, facts.get(key)?))
}

/// The values each song takes from its album's other songs, by its place
/// in the song list.
#[must_use]
pub fn agree(planned: &[(usize, Resolved)]) -> BTreeMap<usize, Vec<Agreed>> {
    let all = members(planned);
    let albums = albums(&all);
    let mut agreed: BTreeMap<usize, Vec<Agreed>> = BTreeMap::new();
    for album in albums.iter().filter(|a| a.len() > 1) {
        agree_on(album, &mut agreed);
    }
    agreed
}

/// What each song on an album takes from the album as a whole, where that
/// changes its pick, by its place in the song list: the design of the
/// cover most of the album's songs show, for a song showing another that
/// a source of its own holds that design in; and the folder most of the
/// album's audio comes from, for a song whose audio comes from another
/// that has a source in it. A song takes nothing from another song's
/// sources, so what each holds stays its own.
#[must_use]
pub fn consistent(
    planned: &[(usize, Resolved)],
    songs: &[Song],
    facts: &BTreeMap<SourceKey, Facts>,
) -> BTreeMap<usize, (Option<Look>, Option<String>)> {
    let all = members(planned);
    let mut out: BTreeMap<usize, (Option<Look>, Option<String>)> = BTreeMap::new();
    for album in albums(&all).iter().filter(|a| a.len() > 1) {
        let shown: Vec<Option<(Look, u32)>> = album.iter().map(|m| shown(m.r, facts)).collect();
        let pinned: Vec<Option<(Look, u32)>> = album
            .iter()
            .zip(&shown)
            .map(|(m, look)| {
                let pin = songs.get(m.n).and_then(|s| s.cover.as_ref());
                look.filter(|_| pin.is_some() && pin == m.r.plan.cover.as_ref().map(|c| &c.key))
            })
            .collect();
        if let Some(design) = design(&pinned).or_else(|| design(&shown)) {
            for (m, look) in album.iter().zip(&shown) {
                let alike = look.is_some_and(|(l, _)| picture::alike(&l, &design) == Alike::Same);
                let holds = || {
                    own(songs, m.n, facts)
                        .flat_map(|f| &f.covers)
                        .filter_map(|c| c.look)
                        .any(|l| picture::alike(&l, &design) == Alike::Same)
                };
                if !alike && holds() {
                    out.entry(m.n).or_default().0 = Some(design);
                }
            }
        }
        let origins: Vec<Option<String>> = album
            .iter()
            .map(|m| origin_of(&m.r.plan.audio.key))
            .collect();
        if let Some(origin) = most(&origins) {
            for (m, from) in album.iter().zip(&origins) {
                let holds = songs.get(m.n).is_some_and(|s| {
                    s.sources.iter().any(|k| {
                        facts.get(k).is_some_and(|f| f.audio.is_some())
                            && origin_of(k).as_deref() == Some(origin.as_str())
                    })
                });
                if from.as_deref() != Some(origin.as_str()) && holds {
                    out.entry(m.n).or_default().1 = Some(origin.clone());
                }
            }
        }
    }
    out
}

/// The facts of each source song `n` lists.
fn own<'f>(
    songs: &'f [Song],
    n: usize,
    facts: &'f BTreeMap<SourceKey, Facts>,
) -> impl Iterator<Item = &'f Facts> {
    songs
        .get(n)
        .into_iter()
        .flat_map(|s| &s.sources)
        .filter_map(|k| facts.get(k))
}

/// How the cover `r` takes looks, and its detail in pixels.
fn shown(r: &Resolved, facts: &BTreeMap<SourceKey, Facts>) -> Option<(Look, u32)> {
    let cover = r.plan.cover.as_ref()?;
    let c = facts
        .get(&cover.key)?
        .covers
        .iter()
        .find(|c| c.at == cover.at)?;
    Some((c.look?, c.quality.map_or(0, |q| q.effective)))
}

/// The design most of an album's covers show, `shown` as each song shows
/// one: covers grouped by [`picture::alike`] reading them one design; of
/// groups alike in size, the one holding the most detail; its sharpest
/// cover stands for it.
fn design(shown: &[Option<(Look, u32)>]) -> Option<Look> {
    let mut group: Vec<usize> = (0..shown.len()).collect();
    for i in 0..shown.len() {
        for j in i + 1..shown.len() {
            if let (Some((a, _)), Some((b, _))) = (shown[i], shown[j])
                && picture::alike(&a, &b) == Alike::Same
            {
                let (gi, gj) = (group[i], group[j]);
                let low = gi.min(gj);
                for g in &mut group {
                    if *g == gi || *g == gj {
                        *g = low;
                    }
                }
            }
        }
    }
    let has = |i: usize| shown[i].is_some();
    let size = |g: usize| {
        (0..shown.len())
            .filter(|&i| has(i) && group[i] == g)
            .count()
    };
    let sharpest = |g: usize| {
        (0..shown.len())
            .filter(|&i| group[i] == g)
            .filter_map(|i| shown[i])
            .max_by_key(|(_, detail)| *detail)
    };
    let best = (0..shown.len())
        .filter(|&i| has(i))
        .map(|i| group[i])
        .max_by_key(|&g| (size(g), sharpest(g).map(|(_, d)| d), std::cmp::Reverse(g)))?;
    sharpest(best).map(|(look, _)| look)
}

/// The value most of `values` are, where two or more are and no other is
/// as common.
fn most(values: &[Option<String>]) -> Option<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for v in values.iter().flatten() {
        *counts.entry(v).or_default() += 1;
    }
    let top = counts.values().copied().max()?;
    let mut leaders = counts.iter().filter(|(_, n)| **n == top);
    let (first, _) = leaders.next()?;
    (top >= 2 && leaders.next().is_none()).then(|| (*first).to_string())
}

/// The folder a file of a torrent or of the manual folder came in, a
/// disc's folder taken as its album's, so one rip's songs share it; none
/// for what a site served.
#[must_use]
pub fn origin_of(key: &SourceKey) -> Option<String> {
    let (scheme, path) = match key {
        SourceKey::Torrent(t) => (format!("torrent:{}", t.hash()), t.path()),
        SourceKey::Manual(m) => ("manual:".to_string(), m.path()),
        SourceKey::Remote { .. } => return None,
    };
    let mut folder = path.parent()?;
    if folder
        .file_name()
        .is_some_and(|n| is_disc_folder(&n.to_string_lossy()))
    {
        folder = folder.parent()?;
    }
    let parts: Vec<String> = folder
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(format!("{scheme}/{}", parts.join("/")))
}

/// What opens the album unit of an `[[album]]`.
const LISTED: &str = "album:";

/// The album units a song's files came in: each file's folder
/// ([`origin_of`]) with the album its tags name, a disc closing it left
/// out, and the `[[album]]` the song list holds it under. Songs of one
/// unit came together, one rip of one album.
#[must_use]
pub fn units_of(song: &Song, facts: &BTreeMap<SourceKey, Facts>) -> BTreeSet<String> {
    let mut units = BTreeSet::new();
    if let Some(album) = &song.album {
        units.insert(format!("{LISTED}{album}"));
    }
    for key in &song.sources {
        let Some(origin) = origin_of(key) else {
            continue;
        };
        let named = facts
            .get(key)
            .and_then(|f| f.tags.get(&Field::Album))
            .and_then(|o| o.values.first());
        let album = named.map_or_else(String::new, |a| similar::fold(crate::parts::album(a).name));
        units.insert(format!("{origin}\u{1f}{album}"));
    }
    units
}

/// The release group each album unit is on by its songs' MusicBrainz
/// records: of an `[[album]]`, the one most of them are on, two or more
/// and no other as many; of a folder, only one more than half of them
/// are on, since a folder of singles is no album. A record of another
/// release group in the unit is held to it ([`crate::vet`]).
#[must_use]
pub fn unit_groups(songs: &[Song], facts: &BTreeMap<SourceKey, Facts>) -> BTreeMap<String, String> {
    let mut tally: BTreeMap<String, (Vec<Option<String>>, usize)> = BTreeMap::new();
    for song in songs {
        let records = song.sources.iter().filter(
            |k| matches!(k, SourceKey::Remote { site, .. } if site == crate::provider::MUSICBRAINZ),
        );
        let groups: BTreeSet<&str> = records
            .filter_map(|k| facts.get(k)?.tags.get(&Field::MusicBrainzReleaseGroupId))
            .filter_map(|o| o.values.first())
            .map(String::as_str)
            .collect();
        if groups.is_empty() {
            continue;
        }
        for unit in units_of(song, facts) {
            let (votes, songs) = tally.entry(unit).or_default();
            votes.extend(groups.iter().map(|g| Some((*g).to_string())));
            *songs += 1;
        }
    }
    tally
        .into_iter()
        .filter_map(|(unit, (votes, songs))| {
            let group = most(&votes)?;
            let on = votes.iter().flatten().filter(|g| **g == group).count();
            (unit.starts_with(LISTED) || on * 2 > songs).then_some((unit, group))
        })
        .collect()
}

/// Whether a folder's name names one disc of a set: `CD1`, `Disc 2`.
fn is_disc_folder(name: &str) -> bool {
    let lower = name.to_lowercase();
    let rest = ["cd", "disc", "disk"]
        .iter()
        .find_map(|p| lower.strip_prefix(p));
    rest.is_some_and(|r| {
        let digits = r.trim_start_matches([' ', '_', '-', '.']);
        !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
    })
}

/// The songs on albums its sources name, album by album: a MusicBrainz
/// release group, else the one release group of its folded album artist
/// and album, else that name in one year.
fn albums<'m>(all: &'m [Member<'m>]) -> Vec<Vec<&'m Member<'m>>> {
    let mut named: BTreeMap<(String, String), BTreeSet<&str>> = BTreeMap::new();
    for m in all {
        if let Some(group) = m.first(Field::MusicBrainzReleaseGroupId) {
            named.entry(m.name()).or_default().insert(group);
        }
    }
    let mut albums: BTreeMap<(String, String, String), Vec<&Member<'_>>> = BTreeMap::new();
    for m in all {
        let name = m.name();
        let key = match m.first(Field::MusicBrainzReleaseGroupId) {
            Some(group) => (group.to_string(), String::new(), String::new()),
            None => match named.get(&name).filter(|g| g.len() == 1) {
                Some(groups) => (
                    groups
                        .iter()
                        .next()
                        .copied()
                        .unwrap_or_default()
                        .to_string(),
                    String::new(),
                    String::new(),
                ),
                None => (
                    String::new(),
                    format!("{}\u{1f}{}", name.0, name.1),
                    m.year(),
                ),
            },
        };
        albums.entry(key).or_default().push(m);
    }
    albums.into_values().collect()
}

/// The values the songs of `album` agree on, each song's into `agreed`.
fn agree_on(album: &[&Member<'_>], agreed: &mut BTreeMap<usize, Vec<Agreed>>) {
    for field in [Field::Album, Field::AlbumArtist] {
        let mut spellings: BTreeMap<String, Vec<&Member<'_>>> = BTreeMap::new();
        for m in album.iter().filter(|m| !m.by_hand(field)) {
            if let Some((v, _)) = m.tag(field) {
                spellings
                    .entry(similar::fold(&v.join(" ")))
                    .or_default()
                    .push(m);
            }
        }
        for alike in spellings.values() {
            settle(field, alike, alike, album.len(), agreed);
        }
    }
    let releases: BTreeSet<&str> = album
        .iter()
        .filter_map(|m| m.first(Field::MusicBrainzAlbumId))
        .collect();
    for release in &releases {
        let on: Vec<&Member<'_>> = album
            .iter()
            .copied()
            .filter(|m| m.first(Field::MusicBrainzAlbumId) == Some(release))
            .collect();
        for field in WHOLE.into_iter().skip(2) {
            let free: Vec<&Member<'_>> = on.iter().copied().filter(|m| !m.by_hand(field)).collect();
            settle(field, &free, &free, album.len(), agreed);
        }
    }
    if releases.len() <= 1 {
        let free: Vec<&Member<'_>> = album
            .iter()
            .copied()
            .filter(|m| !m.by_hand(Field::Date))
            .collect();
        let off: Vec<&Member<'_>> = free
            .iter()
            .copied()
            .filter(|m| m.first(Field::MusicBrainzAlbumId).is_none())
            .collect();
        settle(Field::Date, &free, &off, album.len(), agreed);
    }
}

/// `field` given the value most of `voters` have, the first breaking a
/// tie, to each of `songs` that has another or none: `of` songs being on
/// the album.
fn settle(
    field: Field,
    voters: &[&Member<'_>],
    songs: &[&Member<'_>],
    of: usize,
    agreed: &mut BTreeMap<usize, Vec<Agreed>>,
) {
    let mut tally: Vec<(&[String], &str, usize)> = Vec::new();
    for m in voters {
        let Some((values, from)) = m.tag(field).filter(|(v, _)| !v.is_empty()) else {
            continue;
        };
        match tally.iter_mut().find(|(v, _, _)| *v == values) {
            Some(vote) => vote.2 += 1,
            None => tally.push((values, from, 1)),
        }
    }
    let Some(&(values, from, count)) = tally.iter().min_by_key(|v| std::cmp::Reverse(v.2)) else {
        return;
    };
    for m in songs {
        if m.tag(field).is_some_and(|(v, _)| v == values) {
            continue;
        }
        agreed.entry(m.n).or_default().push(Agreed {
            field,
            values: values.to_vec(),
            from: format!("{from}, as {count} of its album's {of} songs have it"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Codec;
    use crate::resolve::{AudioRef, Format, Plan, TAGGING, TagWhy, Why, render_version};
    use crate::tags::{Offer, Offers};

    const GROUP: &str = "00000000-0000-0000-0000-0000000000a0";
    const FIRST: &str = "00000000-0000-0000-0000-0000000000a1";
    const LATER: &str = "00000000-0000-0000-0000-0000000000a2";

    /// A song of `tags`, each a field, its value and where it came from.
    fn song(n: usize, tags: &[(Field, &str, &str)]) -> (usize, Resolved) {
        let (tags, why) = tags
            .iter()
            .map(|(f, v, from)| {
                let why = TagWhy {
                    key: f.vorbis().to_string(),
                    from: (*from).to_string(),
                    cleaned: Vec::new(),
                };
                ((f.vorbis().to_string(), vec![(*v).to_string()]), why)
            })
            .unzip();
        let plan = Plan {
            folder: None,
            version: render_version(Codec::Opus),
            format: Format::Copy { codec: Codec::Opus },
            audio: AudioRef {
                key: SourceKey::youtube("vid00000001"),
                rev: "r".into(),
                index: 0,
            },
            cover: None,
            lyrics: None,
            tags,
            loudness: None,
            tagging: TAGGING,
            chapters: Vec::new(),
        };
        let why = Why {
            tags: why,
            ..Why::default()
        };
        let stem = std::path::PathBuf::from(format!("song {n}"));
        (n, Resolved { plan, stem, why })
    }

    /// A song on `release` of the album, its fields from the record `from`.
    fn on(n: usize, release: &str, date: &str, from: &str) -> (usize, Resolved) {
        song(
            n,
            &[
                (Field::Album, "Rooms of Salt", from),
                (Field::AlbumArtist, "Marlo Venn", from),
                (Field::Date, date, from),
                (Field::ReleaseStatus, "official", from),
                (Field::ReleaseType, "album", from),
                (Field::MusicBrainzAlbumId, release, from),
                (Field::MusicBrainzReleaseGroupId, GROUP, from),
            ],
        )
    }

    fn offers(fields: &[(Field, &str)]) -> Offers {
        fields
            .iter()
            .map(|(f, v)| {
                let offer = Offer {
                    values: vec![(*v).to_string()],
                    structured: true,
                    also: Vec::new(),
                };
                (*f, offer)
            })
            .collect()
    }

    fn agreed(planned: &[(usize, Resolved)], n: usize, field: Field) -> Option<Vec<String>> {
        agree(planned)
            .get(&n)?
            .iter()
            .find(|a| a.field == field)
            .map(|a| a.values.clone())
    }

    #[test]
    fn a_song_on_another_edition_takes_the_one_most_of_its_album_is_on() {
        let record = SourceKey::parse(&format!("musicbrainz:{FIRST}")).unwrap();
        let mut facts = Facts::unreadable("1".into());
        facts.editions = vec![offers(&[
            (Field::MusicBrainzAlbumId, FIRST),
            (Field::MusicBrainzReleaseGroupId, GROUP),
            (Field::ReleaseStatus, "official"),
            (Field::ReleaseType, "album"),
            (Field::Date, "1971-03-02"),
        ])];
        let facts = BTreeMap::from([(record.clone(), facts)]);
        let songs = vec![
            Song::default(),
            Song::default(),
            Song {
                sources: vec![record.clone()],
                ..Song::default()
            },
        ];
        let from = record.to_string();
        let planned = vec![
            on(0, FIRST, "1971-03-02", "musicbrainz:x"),
            on(1, FIRST, "1971-03-02", "musicbrainz:y"),
            on(2, LATER, "1972", &from),
        ];
        assert_eq!(
            editions(&planned, &songs, &facts),
            BTreeMap::from([(2, (record, 0))])
        );
    }

    #[test]
    fn an_album_on_one_release_already_or_none_offered_moves_nothing() {
        let planned = vec![
            on(0, FIRST, "1971", "musicbrainz:x"),
            on(1, LATER, "1972", "musicbrainz:y"),
        ];
        let songs = vec![Song::default(), Song::default()];
        assert!(editions(&planned, &songs, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn names_alike_once_folded_are_spelled_as_most_songs_spell_them() {
        let named = |n, album| {
            song(
                n,
                &[
                    (Field::Album, album, "torrent:t"),
                    (Field::AlbumArtist, "Marlo Venn", "torrent:t"),
                    (Field::Date, "1974", "torrent:t"),
                ],
            )
        };
        let planned = vec![
            named(0, "Over The Border"),
            named(1, "Over the Border"),
            named(2, "Over the Border"),
            named(3, "Under the Border"),
        ];
        assert_eq!(
            agreed(&planned, 0, Field::Album),
            Some(vec!["Over the Border".to_string()])
        );
        assert_eq!(agreed(&planned, 1, Field::Album), None);
        assert_eq!(agreed(&planned, 3, Field::Album), None);
    }

    #[test]
    fn songs_on_one_release_agree_on_its_date_and_give_what_one_lacks() {
        let mut planned = vec![
            on(0, FIRST, "2018", "musicbrainz:x"),
            on(1, FIRST, "2018", "musicbrainz:y"),
            on(2, FIRST, "1976", "torrent:t"),
        ];
        planned[0]
            .1
            .plan
            .tags
            .push(("LABEL".into(), vec!["Harbor Lights".into()]));
        planned[0].1.why.tags.push(TagWhy {
            key: "LABEL".into(),
            from: "musicbrainz:x".into(),
            cleaned: Vec::new(),
        });
        assert_eq!(
            agreed(&planned, 2, Field::Date),
            Some(vec!["2018".to_string()])
        );
        assert_eq!(
            agreed(&planned, 1, Field::Label),
            Some(vec!["Harbor Lights".to_string()])
        );
        assert_eq!(agreed(&planned, 0, Field::Date), None);
    }

    #[test]
    fn a_song_on_no_release_group_joins_the_one_of_its_name_and_takes_its_date() {
        let upload = song(
            2,
            &[
                (Field::Album, "Rooms of Salt", "youtube.com:vid00000001"),
                (Field::AlbumArtist, "Marlo Venn", "youtube.com:vid00000001"),
                (Field::Date, "2025-05-27", "youtube.com:vid00000001"),
            ],
        );
        let planned = vec![
            on(0, FIRST, "2024-11-08", "musicbrainz:x"),
            on(1, FIRST, "2024-11-08", "musicbrainz:y"),
            upload,
        ];
        assert_eq!(
            agreed(&planned, 2, Field::Date),
            Some(vec!["2024-11-08".to_string()])
        );
    }

    #[test]
    fn two_release_groups_of_one_name_stay_apart() {
        let mut planned = vec![
            on(0, FIRST, "1973", "musicbrainz:x"),
            on(1, LATER, "2007", "musicbrainz:y"),
        ];
        let other = "00000000-0000-0000-0000-0000000000b0";
        for (k, v) in &mut planned[1].1.plan.tags {
            if k == Field::MusicBrainzReleaseGroupId.vorbis() {
                *v = vec![other.to_string()];
            }
        }
        assert!(agree(&planned).is_empty());
    }

    #[test]
    fn a_tag_set_by_hand_is_never_changed_nor_counted() {
        let mut planned = vec![
            on(0, FIRST, "1976", "musicbrainz:x"),
            on(1, FIRST, "1976", "musicbrainz:y"),
            on(2, FIRST, "2018", "musicbrainz:z"),
        ];
        for song in &mut planned[..2] {
            for why in &mut song.1.why.tags {
                if why.key == Field::Date.vorbis() {
                    why.from = "song.tags".into();
                }
            }
        }
        assert!(agree(&planned).is_empty());
    }

    fn look(seed: u64) -> Look {
        Look {
            content: picture::Hashes { p: seed, d: seed },
            square: None,
            frame: None,
            flat: false,
        }
    }

    #[test]
    fn a_disc_s_folder_is_its_album_s() {
        let key = |p: &str| SourceKey::parse(&format!("torrent:{}/{p}", "a".repeat(40))).unwrap();
        let disc_1 = origin_of(&key("Harbor Nights/CD1/01 Song.flac"));
        assert_eq!(disc_1, origin_of(&key("Harbor Nights/Disc 2/01 Song.flac")));
        assert_eq!(disc_1, origin_of(&key("Harbor Nights/03 Song.flac")));
        assert_ne!(disc_1, origin_of(&key("Glass Weather/01 Song.flac")));
        assert_eq!(origin_of(&SourceKey::youtube("vid00000001")), None);
        assert!(!is_disc_folder("Discotheque"));
    }

    #[test]
    fn songs_of_one_folder_and_album_are_one_unit_on_the_release_most_are_on() {
        let (songs, facts) = albums_of(&[
            ("Harbor Nights/CD1/01.flac", "Harbor Nights CD1", GROUP),
            ("Harbor Nights/CD2/01.flac", "Harbor Nights CD 2", GROUP),
            ("Harbor Nights/CD2/02.flac", "harbor nights", LATER),
            ("Glass Weather/01.flac", "Glass Weather", LATER),
        ]);
        let units: Vec<_> = songs.iter().map(|s| units_of(s, &facts)).collect();
        assert_eq!(
            units[0], units[2],
            "a disc's folder and name are its album's"
        );
        assert_eq!(units[0].len(), 1);
        assert_ne!(units[0], units[3]);
        let groups = unit_groups(&songs, &facts);
        let unit = units[0].first().unwrap();
        assert_eq!(groups.get(unit).map(String::as_str), Some(GROUP));
        assert_eq!(groups.len(), 1, "a release one song is on leads no unit");
    }

    #[test]
    fn a_folder_is_on_a_release_more_than_half_its_songs_are_on_and_a_listed_album_on_the_most() {
        let (mut songs, facts) = albums_of(&[
            ("Singles/01.flac", "", GROUP),
            ("Singles/02.flac", "", GROUP),
            ("Singles/03.flac", "", FIRST),
            ("Singles/04.flac", "", LATER),
        ]);
        assert!(unit_groups(&songs, &facts).is_empty(), "two of four");
        let listed = file("Singles/album.cue");
        for song in &mut songs {
            song.album = Some(listed.clone());
        }
        let groups = unit_groups(&songs, &facts);
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups.get(&format!("{LISTED}{listed}")).map(String::as_str),
            Some(GROUP)
        );
    }

    fn file(path: &str) -> SourceKey {
        SourceKey::parse(&format!("torrent:{}/{path}", "a".repeat(40))).unwrap()
    }

    /// A song for each file at a path, with the album its tags name, and
    /// a MusicBrainz record on a release group.
    fn albums_of(files: &[(&str, &str, &str)]) -> (Vec<Song>, BTreeMap<SourceKey, Facts>) {
        let offering = |field: Field, value: &str| {
            let mut f = Facts::unreadable("r".into());
            let offer = Offer {
                values: vec![value.to_string()],
                structured: true,
                also: Vec::new(),
            };
            f.tags.insert(field, offer);
            f
        };
        let mut facts = BTreeMap::new();
        let mut songs = Vec::new();
        for (n, (path, album, group)) in files.iter().enumerate() {
            let record = format!("musicbrainz:00000000-0000-0000-0000-00000000000{n}");
            let record = SourceKey::parse(&record).unwrap();
            if !album.is_empty() {
                facts.insert(file(path), offering(Field::Album, album));
            }
            facts.insert(
                record.clone(),
                offering(Field::MusicBrainzReleaseGroupId, group),
            );
            songs.push(Song {
                sources: vec![file(path), record],
                ..Song::default()
            });
        }
        (songs, facts)
    }

    #[test]
    fn the_most_common_value_leads_only_alone_and_twice_or_more() {
        let of = |v: &[&str]| most(&v.iter().map(|s| Some((*s).to_string())).collect::<Vec<_>>());
        assert_eq!(of(&["a", "a", "b"]).as_deref(), Some("a"));
        assert_eq!(of(&["a", "b"]), None, "no value twice");
        assert_eq!(of(&["a", "a", "b", "b"]), None, "a tie");
    }

    #[test]
    fn a_pinned_cover_is_its_album_s_design() {
        use crate::facts::{CoverAt, CoverFacts};
        let key = |n: usize| SourceKey::parse(&format!("manual:a{n}.flac")).unwrap();
        let (a, b) = (look(0), look(u64::MAX));
        let (mut facts, mut songs, mut planned) = (BTreeMap::new(), Vec::new(), Vec::new());
        for n in 0..3 {
            let mut f = Facts::unreadable("r".into());
            for (index, look) in [(1, a), (2, b)] {
                f.covers.push(CoverFacts {
                    at: CoverAt::Picture { index },
                    mimetype: "image/jpeg".into(),
                    quality: None,
                    digest: None,
                    look: Some(look),
                    other: false,
                });
            }
            facts.insert(key(n), f);
            let mut song = on(n, FIRST, "2001", "musicbrainz:x");
            song.1.plan.cover = Some(crate::resolve::CoverRef {
                key: key(n),
                rev: "r".into(),
                at: CoverAt::Picture {
                    index: if n == 0 { 1 } else { 2 },
                },
                mimetype: "image/jpeg".into(),
                crop: None,
                made: None,
                out: None,
            });
            planned.push(song);
            songs.push(Song {
                sources: vec![key(n)],
                cover: (n == 0).then(|| key(n)),
                ..Song::default()
            });
        }
        let picked = consistent(&planned, &songs, &facts);
        assert_eq!(picked.get(&1).and_then(|p| p.0), Some(a));
        assert_eq!(picked.get(&2).and_then(|p| p.0), Some(a));
        songs[0].cover = None;
        let picked = consistent(&planned, &songs, &facts);
        assert_eq!(
            picked.get(&0).and_then(|p| p.0),
            Some(b),
            "unpinned, the most"
        );
    }

    #[test]
    fn an_album_s_design_is_the_one_most_songs_show() {
        let (a, b) = (look(0), look(u64::MAX));
        assert_eq!(
            design(&[Some((a, 600)), Some((b, 1400)), Some((a, 300))]),
            Some(a)
        );
        assert_eq!(
            design(&[Some((a, 600)), Some((b, 1400))]),
            Some(b),
            "of groups alike in size, the sharpest"
        );
        assert_eq!(design(&[None, None]), None);
    }
}
