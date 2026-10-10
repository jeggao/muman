//! Lyrics, pictures and tag files given to `add`, each matched to the
//! songs it belongs to, as [`crate::identify`] matches audio.
//!
//! A loose file carries no fingerprint, so it is matched by what it says
//! and what it holds, the strongest evidence deciding:
//!
//! 1. **An ID**: a video ID in its name that one of the song's sources
//!    has, or an ISRC or MusicBrainz recording ID its tags share with the
//!    song's. A file whose bytes a source of the song already holds is
//!    the song's already, and nothing is added.
//! 1. **Its name**: the name of one of the song's files, as a sidecar is
//!    named after its song; two songs so named are each only maybe.
//! 1. **Its fields**: title, artist, album, track and length, from an
//!    `.lrc`'s ID tags, a tag file, or its file name read every way beets'
//!    `fromfilename` reads one; compared by [`crate::similar`] against
//!    every name each song is known by, its tags and every source's.
//! 1. **Its content**: lyrics against the words the song's own lyrics
//!    sing; a picture against the song's covers, by [`crate::picture`].
//!
//! Fields are weighed as beets weighs them, title and artist 3, album 2,
//! length 2, track 1, and their distance is `Σ weight × distance / Σ
//! weight` over the fields both sides have; a length counts no distance
//! within 10 s and all of it from 30 s. The bands are beets' own:
//!
//! | Distance | And | Verdict |
//! |---|---|---|
//! | 0.04 or less | 5 of weight compared, 0.10 nearer than the next song, no other version | The song's |
//! | 0.25 or less | | Maybe the song's |
//! | More | | Not the song's |
//!
//! A title alone weighs 3, so it never decides: a hundred songs are
//! called `Intro`. A version word on one side only (live, instrumental,
//! remix…) or another number keeps a match at maybe.
//!
//! Lyrics are refused by a song they cannot be the words of: one whose
//! last line comes more than 2 s after the song ends, or whose `[length:]`
//! is 4 s or more off it, which `resolve` would never pick. Their words,
//! as runs of three [`crate::similar::tokens`], are measured by the share
//! the smaller set's runs the other holds: half or more is the song's,
//! a fifth or more maybe; under a tenth, both long enough and in one
//! script, keeps even a match by name at maybe, and two scripts, a
//! translation, count neither way.
//!
//! A picture goes to every song whose cover looks like it, besides the
//! song matched another way. Added without asking only while that is at
//! most [`MANY`] songs or one album's: a channel that gives every upload
//! one still picture would otherwise lend it to every song it uploaded.
//!
//! A cue sheet, or a JSON file of several songs, is matched track by
//! track, each track's songs costed as above, and assigned so the sum is
//! least ([`crate::similar::assign`]): two tracks never go to one song,
//! and a track worse than maybe for every song goes to none.
//!
//! What is not the song's for sure is asked about on a terminal: the
//! likeliest songs, a search of the song list, or leaving the file out.
//! With `-y` the likeliest is taken when it leads the next by 0.10.
//! Without a terminal it is left out, and the command refuses at its
//! end, so a script hears that a file it named was not added. A file
//! that cannot be used, as one that does not read or a cue sheet that
//! splits one song's file into tracks, is not added either, and the
//! command fails at its end; the songs given with it are written first. Distances
//! are rounded to thousandths before any comparison, and ties go to the
//! song listed first, so the same files go to the same songs every run.
//!
//! An accepted lyrics file or picture is copied into the manual folder's
//! `added/`, where no song file is, so it becomes no song's sidecar or
//! folder cover, and listed among the song's sources, where `resolve`
//! ranks it as any other; one already there with the same bytes is
//! listed again rather than copied, but for lyrics another song lists,
//! which are copied anew, a song's lyrics being its own. A tag file is written into the song's
//! `tags` and not kept.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::dirs::Dirs;
use crate::facts::{self, LyricsAt};
use crate::lyrics;
use crate::manifest::{self, Edit, Manifest};
use crate::picture::{self, Alike, Look};
use crate::query::{self, Query, View};
use crate::reconcile;
use crate::runner::Runner;
use crate::similar::{self, Name};
use crate::source::SourceKey;
use crate::state::State;
use crate::store::{self, Kind, Store};
use crate::tagfile::{self, Sheet, Tags};
use crate::tags::Field;
use crate::ui::{self, Prompter};

const STRONG: i64 = 40;
const ASK: i64 = 250;
const LEAD: i64 = 100;
const MIN_WEIGHT: f64 = 5.0;
const GRACE_MS: f64 = 10_000.0;
const SPAN_MS: f64 = 30_000.0;
const CUE_GRACE_MS: f64 = 2_000.0;
const CUE_SPAN_MS: f64 = 15_000.0;
const LAST_LINE_SLACK_MS: i64 = 2_000;
const STATED_AGREES_MS: i64 = 2_000;
const SAME_WORDS: f64 = 0.5;
const MAYBE_WORDS: f64 = 0.2;
const OTHER_WORDS: f64 = 0.1;
const FEW_RUNS: usize = 20;
/// The songs a picture is added to unasked, but for one album's.
pub const MANY: usize = 3;
/// The likeliest songs offered when asking.
const OFFERED: usize = 5;
/// The songs a search lists.
const LISTED: usize = 50;
/// The songs each track of a sheet is costed against.
const PER_TRACK: usize = 8;
/// What leaving a track of a sheet unassigned costs: worse than maybe.
const UNMATCHED: i64 = 500;
const NO_MATCH: i64 = 10_000;
/// Where accepted files are copied, inside the manual folder.
pub const ADDED: &str = "added";

/// Files given to songs, each with what it holds.
pub type Loose = Vec<(PathBuf, What)>;

/// What a loose file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    Lyrics,
    Picture,
    Tags,
}

impl What {
    fn of(self) -> &'static str {
        match self {
            Self::Lyrics => "the lyrics of",
            Self::Picture => "the cover of",
            Self::Tags => "the tags of",
        }
    }

    fn whose(self) -> &'static str {
        match self {
            Self::Lyrics => "whose lyrics are these?",
            Self::Picture => "whose cover is this?",
            Self::Tags => "whose tags are these?",
        }
    }
}

/// The extensions a loose file may have, besides those the store reads.
pub const LOOSE: [&str; 10] = [
    "cue",
    "json",
    "jsonl",
    "ffmeta",
    "ffmetadata",
    "txt",
    "tags",
    "vc",
    "srt",
    "vtt",
];

/// What `path` holds, if `add` matches it to songs rather than listing
/// it: none for a song file or a folder.
pub fn sort(path: &Path) -> Result<Option<What>> {
    if path.is_dir() {
        return Ok(None);
    }
    match store::kind_of(path) {
        Some(Kind::Media) => return Ok(None),
        Some(Kind::Lyrics) => return Ok(Some(What::Lyrics)),
        Some(Kind::Image) => return Ok(Some(What::Picture)),
        _ => {}
    }
    let ext = extension(path);
    if ext == "srt" || ext == "vtt" {
        return Ok(Some(What::Lyrics));
    }
    if LOOSE.contains(&ext.as_str()) {
        let text = read_text(path)?;
        if tagfile::format_of(path, &text).is_some() {
            return Ok(Some(What::Tags));
        }
        if ext == "txt" && !text.trim().is_empty() && lyrics::is_text(&text) {
            return Ok(Some(What::Lyrics));
        }
    }
    bail!(
        "{} is no song, lyrics, picture or tags muman reads",
        path.display()
    )
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(lyrics::decode(&bytes))
}

/// How loose files are matched.
#[derive(Debug, Clone, Default)]
pub struct How {
    /// Take a song that is only maybe the file's when it leads.
    pub yes: bool,
    /// The query naming the songs every loose file goes to.
    pub to: Vec<String>,
    pub verbose: bool,
}

/// What attaching found: the songs it gave something, and how many files
/// it left out for want of an answer.
#[derive(Debug, Default)]
pub struct Attached {
    pub songs: Vec<SourceKey>,
    pub left_out: usize,
    /// The files that could not be read or used, each with why.
    pub unread: Vec<String>,
}

/// One song as a loose file is matched against it.
#[derive(Debug)]
struct Candidate {
    id: SourceKey,
    label: String,
    album: String,
    album_key: String,
    titles: Vec<Name>,
    artists: Vec<Name>,
    albums: Vec<Name>,
    track: Option<u32>,
    length_ms: Option<i64>,
    stems: BTreeSet<String>,
    ids: BTreeSet<String>,
    looks: Vec<Look>,
    digests: BTreeSet<String>,
    lyrics: Vec<PathBuf>,
    /// The subtitle streams its sources hold, read only when it is among
    /// the likeliest songs for lyrics.
    streams: Vec<(PathBuf, u32)>,
    view: View,
}

/// What a file or a track says of its song.
#[derive(Debug, Default, Clone)]
struct Fields {
    title: Option<Name>,
    artist: Option<Name>,
    album: Option<Name>,
    track: Option<u32>,
}

#[derive(Debug, Default)]
struct Clues {
    readings: Vec<Fields>,
    length_ms: Option<i64>,
    grace: (f64, f64),
    ids: BTreeSet<String>,
    stem: String,
    file: Option<String>,
    /// The songs given in the same command from its folder, by stem.
    with: BTreeSet<String>,
    digest: Option<String>,
    look: Option<Look>,
    words: Option<(BTreeSet<String>, Option<icu_properties::props::Script>)>,
    last_ms: Option<i64>,
    stated_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Verdict {
    Different,
    Unsure,
    Same,
}

/// How one song stands against one file.
#[derive(Debug, Clone)]
struct Judged {
    song: usize,
    verdict: Verdict,
    /// Field distance in thousandths; [`NO_MATCH`] when no field compared.
    distance: i64,
    /// Whether the verdict rests on the fields alone, so needs its lead.
    by_fields: bool,
    looks: Alike,
    already: bool,
    reasons: Vec<String>,
}

impl Judged {
    /// How its names compare, in words: a percentage would read as more
    /// sure than a name ever makes a song.
    fn names(&self) -> Option<&'static str> {
        match self.distance {
            d if d <= STRONG => Some("same name"),
            d if d <= ASK => Some("close name"),
            d if d < NO_MATCH => Some("other name"),
            _ => None,
        }
    }
}

fn minutes(ms: i64) -> String {
    let s = (ms + 500) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

#[allow(clippy::cast_possible_truncation)]
fn thousandths(d: f64) -> i64 {
    (d * 1000.0).round() as i64
}

#[allow(clippy::cast_precision_loss)]
fn ms_f(ms: i64) -> f64 {
    ms as f64
}

/// The tag file name a song list writes a Vorbis comment as.
fn song_list_name(vorbis: &str) -> String {
    match Field::named(vorbis) {
        Some(Field::AlbumArtist) => "album_artist".into(),
        Some(Field::Track) => "track".into(),
        Some(Field::Disc) => "disc".into(),
        Some(Field::TrackTotal) => "track_total".into(),
        Some(Field::DiscTotal) => "disc_total".into(),
        Some(Field::ReleaseCountry) => "release_country".into(),
        _ => vorbis.to_ascii_lowercase(),
    }
}

fn values<'a>(tags: &'a [(String, Vec<String>)], name: &str) -> impl Iterator<Item = &'a String> {
    tags.iter()
        .filter(move |(k, _)| *k == name)
        .flat_map(|(_, v)| v.iter())
}

fn names<'a>(values: impl Iterator<Item = &'a String>) -> Vec<Name> {
    let mut seen = BTreeSet::new();
    values
        .filter(|v| seen.insert((*v).clone()))
        .map(|v| Name::new(v))
        .filter(|n| !n.is_empty())
        .collect()
}

fn folded_stem(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let bare = match stem.rfind(" [") {
        Some(at) if stem.ends_with(']') => &stem[..at],
        _ => stem,
    };
    similar::fold(bare)
}

/// Every song of the list as loose files are matched against it.
fn candidates(
    manifest: &Manifest,
    state: &State,
    store: &Store,
    dirs: &Dirs,
) -> Result<Vec<Candidate>> {
    let views = query::resolved_views(manifest, state, &dirs.library)?;
    let mut out = Vec::new();
    for (song, (view, resolved)) in manifest.songs.iter().zip(views) {
        let Some(id) = song.id().cloned() else {
            continue;
        };
        let facts: Vec<&facts::Facts> = song
            .sources
            .iter()
            .filter_map(|k| state.facts.get(k))
            .collect();
        let offered = |field: Field| {
            facts
                .iter()
                .filter_map(move |f| f.tags.get(&field))
                .flat_map(|o| o.values.iter())
        };
        let tags = &view.tags;
        let titles = names(values(tags, "TITLE").chain(offered(Field::Title)));
        let artists = names(
            values(tags, "ARTIST")
                .chain(values(tags, "ALBUMARTIST"))
                .chain(offered(Field::Artist))
                .chain(offered(Field::AlbumArtist)),
        );
        let albums = names(values(tags, "ALBUM").chain(offered(Field::Album)));
        let audio = resolved.as_ref().map(|r| r.plan.audio.key.clone());
        #[allow(clippy::cast_possible_truncation)]
        let length_ms = audio
            .as_ref()
            .and_then(|k| state.facts.get(k))
            .or_else(|| facts.iter().copied().find(|f| f.audio.is_some()))
            .and_then(|f| f.duration)
            .map(|s| (s * 1000.0).round() as i64);
        let mut stems = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut looks = Vec::new();
        let mut digests = BTreeSet::new();
        let mut lyrics = Vec::new();
        let mut streams = Vec::new();
        for key in &song.sources {
            if let SourceKey::Remote { id, .. } = key {
                ids.insert(id.to_lowercase());
            }
            let located = store.locate(key);
            if let Some(l) = &located
                && l.kind == Kind::Media
            {
                stems.insert(folded_stem(&l.path));
            }
            let Some(f) = state.facts.get(key) else {
                continue;
            };
            for c in &f.covers {
                looks.extend(c.look);
                digests.extend(c.digest.clone());
            }
            for l in &f.lyrics {
                digests.extend(l.digest.clone());
                match (&l.at, &located) {
                    (LyricsAt::Stream { index }, Some(s)) => streams.push((s.path.clone(), *index)),
                    (at, Some(s)) => lyrics.extend(crate::render::lyrics_file(at, s)),
                    (_, None) => {}
                }
            }
        }
        if let Some(r) = &resolved {
            stems.insert(folded_stem(&r.stem.with_extension("x")));
        }
        for field in ["ISRC", "MUSICBRAINZ_TRACKID"] {
            ids.extend(values(tags, field).map(|v| v.to_lowercase()));
        }
        stems.remove("");
        let album = view.first("album");
        let album_key = format!("{}\u{1f}{album}", view.first("albumartist"));
        out.push(Candidate {
            label: view.name(),
            album_key: similar::fold(&album_key),
            album,
            titles,
            artists,
            albums,
            track: view.first("tracknumber").parse().ok(),
            length_ms,
            stems,
            ids,
            looks,
            digests,
            lyrics,
            streams,
            id,
            view,
        });
    }
    Ok(out)
}

/// The distance between the song and what `fields` say, with the weight
/// compared and any conflict found.
fn score(c: &Candidate, f: &Fields, clues: &Clues) -> (f64, f64, Vec<&'static str>) {
    let mut sum = 0.0;
    let mut weight = 0.0;
    let mut conflicts = Vec::new();
    let mut field =
        |ours: &Option<Name>, theirs: &[Name], w: f64, conflicts: &mut Vec<&'static str>| {
            let Some(ours) = ours else { return };
            let best = theirs
                .iter()
                .filter_map(|t| similar::compare(ours, t))
                .min_by(|a, b| a.distance.total_cmp(&b.distance));
            if let Some(best) = best {
                sum += w * best.distance;
                weight += w;
                conflicts.extend(best.conflicts);
            }
        };
    field(&f.title, &c.titles, 3.0, &mut conflicts);
    field(&f.artist, &c.artists, 3.0, &mut Vec::new());
    field(&f.album, &c.albums, 2.0, &mut Vec::new());
    if let (Some(a), Some(b)) = (f.track, c.track) {
        sum += if a == b { 0.0 } else { 1.0 };
        weight += 1.0;
    }
    if let (Some(a), Some(b)) = (clues.length_ms, c.length_ms) {
        let (grace, span) = clues.grace;
        let off = ms_f((a - b).abs());
        sum += 2.0 * ((off - grace) / (span - grace)).clamp(0.0, 1.0);
        weight += 2.0;
    }
    conflicts.sort_unstable();
    conflicts.dedup();
    (
        if weight > 0.0 { sum / weight } else { 1.0 },
        weight,
        conflicts,
    )
}

/// How every song stands against `clues`, likeliest first.
fn judge(cands: &[Candidate], clues: &Clues, what: What) -> Vec<Judged> {
    let mut judged: Vec<Judged> = cands
        .iter()
        .enumerate()
        .map(|(n, c)| judge_one(n, c, clues, what))
        .collect();
    let named = |j: &Judged| j.reasons.iter().any(|r| r == "named as its file");
    let stem_shared = judged.iter().filter(|j| named(j)).count();
    for j in &mut judged {
        if stem_shared > 1 && named(j) {
            for r in &mut j.reasons {
                if r == "named as its file" {
                    *r = format!("named as its file, as {stem_shared} songs are");
                }
            }
            if j.verdict == Verdict::Same {
                j.verdict = Verdict::Unsure;
            }
        }
    }
    // A match by fields alone must lead every other song: two equally
    // near are each only maybe.
    let distances: Vec<(usize, i64)> = judged.iter().map(|j| (j.song, j.distance)).collect();
    for j in &mut judged {
        if !(j.by_fields && j.verdict == Verdict::Same) {
            continue;
        }
        let nearest = distances
            .iter()
            .filter(|(song, _)| *song != j.song)
            .min_by_key(|(song, d)| (*d, *song));
        if let Some(&(song, d)) = nearest
            && d - j.distance < LEAD
        {
            j.verdict = Verdict::Unsure;
            j.reasons
                .push(format!("as near as {}, so asked", cands[song].label));
        }
    }
    order(&mut judged);
    judged
}

fn order(judged: &mut [Judged]) {
    judged.sort_by(|a, b| {
        b.verdict
            .cmp(&a.verdict)
            .then(b.looks.cmp(&a.looks))
            .then(a.distance.cmp(&b.distance))
            .then(a.song.cmp(&b.song))
    });
}

#[allow(clippy::too_many_lines)]
fn judge_one(n: usize, c: &Candidate, clues: &Clues, what: What) -> Judged {
    let mut reasons = Vec::new();
    let mut verdict = Verdict::Different;
    let mut by_fields = false;
    let mut cap = Verdict::Same;
    let mut already = false;
    if clues.digest.as_ref().is_some_and(|d| c.digests.contains(d)) {
        already = true;
        verdict = Verdict::Same;
        reasons.push("holds it already".to_string());
    }
    if !clues.ids.is_disjoint(&c.ids) {
        verdict = Verdict::Same;
        reasons.push("named by its ID".to_string());
    }
    let named_as = (!clues.stem.is_empty() && c.stems.contains(&clues.stem))
        || clues.file.as_ref().is_some_and(|f| c.stems.contains(f));
    if named_as {
        verdict = verdict.max(Verdict::Same);
        reasons.push("named as its file".to_string());
    }
    if !clues.with.is_disjoint(&c.stems) {
        verdict = Verdict::Same;
        reasons.push("came with it".to_string());
    }
    let mut distance = NO_MATCH;
    let mut conflicts = Vec::new();
    for reading in &clues.readings {
        let (d, weight, found) = score(c, reading, clues);
        if weight == 0.0 {
            continue;
        }
        let d = thousandths(d);
        if d < distance {
            distance = d;
            conflicts = found;
            let fields = if d <= STRONG && weight >= MIN_WEIGHT {
                Verdict::Same
            } else if d <= ASK {
                Verdict::Unsure
            } else {
                Verdict::Different
            };
            by_fields = fields > verdict;
            verdict = verdict.max(fields);
        }
    }
    if let (Some(a), Some(b)) = (clues.length_ms, c.length_ms)
        && distance < NO_MATCH
        && ms_f((a - b).abs()) <= clues.grace.0
    {
        reasons.push("length agrees".to_string());
    }
    for conflict in &conflicts {
        reasons.push((*conflict).to_string());
        cap = Verdict::Unsure;
    }
    let mut looks = Alike::Different;
    if let Some(look) = &clues.look {
        looks = c
            .looks
            .iter()
            .map(|l| picture::alike(look, l))
            .max()
            .unwrap_or(Alike::Different);
        match looks {
            Alike::Same => {
                verdict = Verdict::Same;
                by_fields = false;
                reasons.push("its cover looks alike".to_string());
            }
            Alike::Unsure => {
                verdict = verdict.max(Verdict::Unsure);
                reasons.push("its cover looks somewhat alike".to_string());
            }
            Alike::Different => {}
        }
    }
    if what == What::Lyrics {
        if let (Some(last), Some(length)) = (clues.last_ms, c.length_ms)
            && last > length + LAST_LINE_SLACK_MS
        {
            return Judged {
                song: n,
                verdict: Verdict::Different,
                distance,
                by_fields: false,
                looks,
                already: false,
                reasons: vec![format!(
                    "its last line is at {}, past the song's {}",
                    minutes(last),
                    minutes(length)
                )],
            };
        }
        if let (Some(stated), Some(length)) = (clues.stated_ms, c.length_ms) {
            let off = (stated - length).abs();
            if ms_f(off) >= crate::resolve::STATED_SPAN_MS {
                return Judged {
                    song: n,
                    verdict: Verdict::Different,
                    distance,
                    by_fields: false,
                    looks,
                    already: false,
                    reasons: vec![format!(
                        "it says it is {} long, the song {}",
                        minutes(stated),
                        minutes(length)
                    )],
                };
            }
            if off <= STATED_AGREES_MS && !reasons.iter().any(|r| r == "length agrees") {
                reasons.push("length agrees".to_string());
            }
        }
    }
    if let (Some((ours, script)), false) = (&clues.words, c.lyrics.is_empty()) {
        let mut best: Option<f64> = None;
        let mut contradicted = false;
        for path in &c.lyrics {
            let Ok(text) = read_text(path) else { continue };
            let sung = lyrics::sung(&text);
            let theirs = similar::shingles(&similar::tokens(&sung), 3);
            let share = similar::containment(ours, &theirs);
            if similar::script_of(&sung) == *script || script.is_none() {
                best = Some(best.map_or(share, |b: f64| b.max(share)));
                contradicted |=
                    share < OTHER_WORDS && ours.len() >= FEW_RUNS && theirs.len() >= FEW_RUNS;
            }
        }
        match best {
            Some(s) if s >= SAME_WORDS => {
                verdict = Verdict::Same;
                by_fields = false;
                reasons.push("its words are the song's".to_string());
            }
            Some(s) if s >= MAYBE_WORDS => {
                verdict = verdict.max(Verdict::Unsure);
                reasons.push("its words are much like the song's".to_string());
            }
            _ if contradicted => {
                cap = Verdict::Unsure;
                reasons.push("its words are not the song's".to_string());
            }
            _ => {}
        }
    }
    if looks != Alike::Same && clues.look.as_ref().is_some_and(|l| l.flat) {
        cap = cap.min(Verdict::Unsure);
    }
    Judged {
        song: n,
        verdict: verdict.min(cap),
        distance,
        by_fields,
        looks,
        already,
        reasons,
    }
}

/// What a file asks of the song list.
struct Item {
    path: PathBuf,
    label: String,
    what: What,
    /// The file as it is copied, where it is converted first.
    bytes: Option<Vec<u8>>,
    clues: Clues,
    sheet: Option<Sheet>,
}

/// The readings of a file's name, as fields.
fn name_readings(path: &Path, what: What) -> Vec<Fields> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if what == What::Picture
        && store::FOLDER_COVERS
            .iter()
            .any(|c| stem.eq_ignore_ascii_case(c))
    {
        return Vec::new();
    }
    let bare = match stem.rfind(" [") {
        Some(at) if stem.ends_with(']') => &stem[..at],
        _ => stem,
    };
    let mut out = Vec::new();
    for g in similar::guesses(bare) {
        let fields = Fields {
            title: Some(Name::new(&g.title)),
            artist: g.artist.as_deref().map(Name::new),
            album: None,
            track: g.track,
        };
        if what == What::Picture {
            out.push(Fields {
                title: None,
                album: Some(Name::new(&g.title)),
                ..fields.clone()
            });
        }
        out.push(fields);
    }
    out
}

fn fields_of(tags: &Tags) -> Fields {
    let first = |name: &str| values(tags, name).next().map(|v| Name::new(v));
    Fields {
        title: first("TITLE"),
        artist: first("ARTIST"),
        album: first("ALBUM"),
        track: values(tags, "TRACKNUMBER")
            .next()
            .and_then(|v| v.parse().ok()),
    }
}

fn ids_of(tags: &Tags) -> BTreeSet<String> {
    values(tags, "ISRC")
        .chain(values(tags, "MUSICBRAINZ_TRACKID"))
        .map(|v| v.to_lowercase())
        .collect()
}

/// Read `path` into what it asks, converting subtitles to LRC in
/// `scratch`.
fn read_item<R: Runner>(runner: &R, path: &Path, what: What, scratch: &Path) -> Result<Item> {
    let label = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let mut clues = Clues {
        stem: folded_stem(path),
        grace: (GRACE_MS, SPAN_MS),
        digest: facts::digest_file(path),
        ..Clues::default()
    };
    if let Some(id) = crate::source::id_of(path) {
        clues.ids.insert(id.to_lowercase());
    }
    let mut bytes = None;
    let mut sheet = None;
    match what {
        What::Lyrics => {
            let text = if matches!(extension(path).as_str(), "srt" | "vtt") {
                let lrc = scratch.join(format!("{}.lrc", clues.stem.len()));
                let args: Vec<std::ffi::OsString> =
                    facts::LRC_ARGS.iter().map(Into::into).collect();
                let output = crate::ffmpeg::Output::new(args, &lrc);
                let ran =
                    crate::ffmpeg::run_outputs(runner, &[path], std::slice::from_ref(&output));
                if !ran.into_iter().all(|r| r.is_ok()) {
                    bail!("{} reads as no subtitles", path.display());
                }
                let converted =
                    std::fs::read(&lrc).with_context(|| format!("reading {}", lrc.display()))?;
                clues.digest = Some(facts::digest(&converted));
                let text = lyrics::decode(&converted);
                bytes = Some(converted);
                text
            } else {
                read_text(path)?
            };
            if text.trim().is_empty() || !lyrics::is_text(&text) {
                bail!("{} holds no lyrics", path.display());
            }
            let h = lyrics::headers(&text);
            if h.title.is_some() {
                clues.readings.push(Fields {
                    title: h.title.as_deref().map(Name::new),
                    artist: h.artist.as_deref().map(Name::new),
                    album: h.album.as_deref().map(Name::new),
                    track: None,
                });
            }
            clues.stated_ms = h.length_ms;
            clues.length_ms = h.length_ms;
            clues.last_ms = lyrics::timing(&lyrics::clean_lrc(&text))
                .map(|t| t.last_ms.saturating_sub(h.offset_ms));
            let sung = lyrics::sung(&text);
            clues.words = Some((
                similar::shingles(&similar::tokens(&sung), 3),
                similar::script_of(&sung),
            ));
            if h.title.is_none() {
                clues.readings.extend(name_readings(path, what));
            }
        }
        What::Picture => {
            clues.look = look_of(runner, path, scratch);
            if clues.look.is_none() {
                bail!("{} reads as no picture", path.display());
            }
            clues.readings.extend(name_readings(path, what));
        }
        What::Tags => {
            let text = read_text(path)?;
            let read = tagfile::read(path, &text)?;
            if read.tracks.len() == 1 {
                let tags = read.tags_of(0);
                clues.readings.push(fields_of(&tags));
                clues.ids = ids_of(&tags);
                clues.length_ms = read.tracks[0].length_ms;
                clues.readings.extend(
                    name_readings(path, what)
                        .into_iter()
                        .filter(|_| fields_of(&tags).title.is_none()),
                );
            }
            sheet = Some(read);
        }
    }
    Ok(Item {
        path: path.to_path_buf(),
        label,
        what,
        bytes,
        clues,
        sheet,
    })
}

fn look_of<R: Runner>(runner: &R, path: &Path, scratch: &Path) -> Option<Look> {
    let located = store::Located {
        key: SourceKey::Manual(Path::new(path.file_name()?).into()),
        path: path.to_path_buf(),
        kind: Kind::Image,
        lyrics: None,
        covers: Vec::new(),
    };
    facts::look(runner, &located, scratch)
        .ok()?
        .into_iter()
        .find_map(|(_, look)| look)
}

/// The likeliest songs whose lyrics are read from their streams.
const STREAMED: usize = 3;

/// Read the subtitle streams of the likeliest songs for each lyrics
/// item, so their words can be compared; one ffmpeg run a source.
fn read_streams<R: Runner>(runner: &R, cands: &mut [Candidate], items: &[Item], scratch: &Path) {
    for item in items
        .iter()
        .filter(|i| i.what == What::Lyrics && i.clues.words.is_some())
    {
        let likeliest: Vec<usize> = judge(cands, &item.clues, What::Lyrics)
            .iter()
            .take(STREAMED)
            .map(|j| j.song)
            .collect();
        for n in likeliest {
            let c = &mut cands[n];
            if !c.lyrics.is_empty() {
                continue;
            }
            for (k, (path, index)) in std::mem::take(&mut c.streams).into_iter().enumerate() {
                let lrc = scratch.join(format!("sung-{n}-{k}.lrc"));
                let mut args: Vec<std::ffi::OsString> =
                    vec!["-map".into(), format!("0:{index}").into()];
                args.extend(facts::LRC_ARGS.iter().map(Into::into));
                let output = crate::ffmpeg::Output::new(args, &lrc);
                let ran = crate::ffmpeg::run_outputs(
                    runner,
                    &[path.as_path()],
                    std::slice::from_ref(&output),
                );
                if ran.into_iter().all(|r| r.is_ok()) {
                    c.lyrics.push(lrc);
                }
            }
        }
    }
}

/// What became of one file.
enum Decision {
    /// Copied and listed by these songs.
    Listed(Vec<usize>),
    /// Its tags set on these songs, each with its own; `left_out` when
    /// some of its tracks were left for want of an answer.
    Tagged {
        songs: Vec<(usize, Tags)>,
        left_out: bool,
    },
    Already,
    Declined,
    LeftOut,
}

/// The asking side: the songs, how they are named, and the prompter.
struct Asking<'a, 'p, W: Write> {
    cands: &'a [Candidate],
    extractors: &'a BTreeSet<String>,
    yes: bool,
    prompter: Option<&'p mut dyn Prompter>,
    out: &'a mut W,
}

impl<W: Write> Asking<'_, '_, W> {
    fn info(&mut self, msg: &str) -> Result<()> {
        ui::info(self.out, msg)?;
        Ok(())
    }

    fn warn(&mut self, msg: &str) -> Result<()> {
        ui::warning(self.out, msg)?;
        Ok(())
    }

    fn line(&self, j: &Judged) -> String {
        let c = &self.cands[j.song];
        let mut line = c.label.clone();
        if !c.album.is_empty() {
            let _ = write!(line, " · {}", c.album);
        }
        if let Some(ms) = c.length_ms {
            let _ = write!(line, " · {}", minutes(ms));
        }
        let mut why: Vec<String> = j.names().map(String::from).into_iter().collect();
        why.extend(j.reasons.iter().cloned());
        if !why.is_empty() {
            let _ = write!(line, "   {}", why.join(", "));
        }
        line
    }

    /// The song `item` goes to: the one sure, or one picked; none when
    /// the user declines, `Err` of a refusal when none can be asked.
    fn one(
        &mut self,
        item_label: &str,
        what: What,
        judged: &[Judged],
        question: &str,
    ) -> Result<Option<usize>> {
        let top = judged.first();
        if let Some(j) = top.filter(|j| j.verdict == Verdict::Same) {
            let why = j.reasons.join(", ");
            let shown = if why.is_empty() {
                String::new()
            } else {
                format!(" ({why})")
            };
            self.info(&format!(
                "{item_label}: {} {}{shown}; adding to that song",
                what.of(),
                self.cands[j.song].label
            ))?;
            return Ok(Some(j.song));
        }
        let offered: Vec<&Judged> = judged
            .iter()
            .filter(|j| j.verdict == Verdict::Unsure)
            .chain(
                judged
                    .iter()
                    .filter(|j| j.verdict == Verdict::Different && j.distance <= ASK * 2),
            )
            .take(OFFERED)
            .collect();
        // The likeliest leads when no other song may be it, or when it is
        // clearly the nearest by name.
        let leads = |j: &&Judged| {
            j.verdict == Verdict::Unsure
                && judged.get(1).is_none_or(|next| {
                    next.verdict == Verdict::Different || next.distance - j.distance >= LEAD
                })
        };
        if self.yes
            && let Some(j) = top.filter(leads)
        {
            self.info(&format!(
                "{item_label}: likely {} {}; adding to that song",
                what.of(),
                self.cands[j.song].label
            ))?;
            return Ok(Some(j.song));
        }
        if self.prompter.is_none() {
            let maybe: Vec<String> = offered
                .iter()
                .filter(|j| j.verdict == Verdict::Unsure)
                .map(|j| self.cands[j.song].label.clone())
                .collect();
            let said = if maybe.is_empty() {
                format!("{item_label} matches no song; not added. Name its song with --to")
            } else {
                format!(
                    "{item_label} may be {} {}; not added. Name its song with --to{}",
                    what.of(),
                    maybe.join(" or "),
                    if self.yes {
                        ", or run on a terminal"
                    } else {
                        ", add it again with -y, or run on a terminal"
                    }
                )
            };
            self.warn(&said)?;
            return Err(LeftOut.into());
        }
        let lines: Vec<String> = offered.iter().map(|j| self.line(j)).collect();
        let songs: Vec<usize> = offered.iter().map(|j| j.song).collect();
        let asked = if songs.is_empty() {
            format!("{item_label} matches no song. Search for it?")
        } else {
            format!("{item_label}: {question}")
        };
        self.pick(&asked, &lines, &songs, question)
    }

    /// Ask among `songs`, named by `lines`, or a search; `None` when the
    /// user leaves the file out.
    fn pick(
        &mut self,
        asked: &str,
        lines: &[String],
        songs: &[usize],
        question: &str,
    ) -> Result<Option<usize>> {
        loop {
            let mut items = lines.to_vec();
            items.push("Search the song list…".to_string());
            items.push("Don't add it".to_string());
            let Some(p) = self.prompter.as_deref_mut() else {
                return Ok(None);
            };
            match p.select(asked, &items)? {
                Some(i) if i < songs.len() => return Ok(Some(songs[i])),
                Some(i) if i == songs.len() => match self.search(question)? {
                    Searched::Picked(n) => return Ok(Some(n)),
                    Searched::LeftOut => return Ok(None),
                    Searched::Back => {}
                },
                _ => return Ok(None),
            }
        }
    }

    /// Search the song list until a song is picked, the file is left
    /// out, or the user goes back to the question.
    fn search(&mut self, question: &str) -> Result<Searched> {
        loop {
            let Some(p) = self.prompter.as_deref_mut() else {
                return Ok(Searched::LeftOut);
            };
            let Some(typed) = p.text(
                "Search the song list:",
                "a query as `muman list` reads it, as artist:venn lantern; esc goes back",
            )?
            else {
                return Ok(Searched::Back);
            };
            let terms = match shell_words::split(&typed) {
                Ok(t) => t,
                Err(e) => {
                    self.warn(&format!("{typed}: {e}"))?;
                    continue;
                }
            };
            let query = match Query::parse(&terms, self.extractors) {
                Ok(q) => q,
                Err(e) => {
                    self.warn(&format!("{e:#}"))?;
                    continue;
                }
            };
            let found: Vec<usize> = (0..self.cands.len())
                .filter(|&n| query.matches(&self.cands[n].view))
                .collect();
            if found.is_empty() {
                self.info(&format!("No song matches `{typed}`"))?;
                continue;
            }
            let mut items: Vec<String> = found
                .iter()
                .take(LISTED)
                .map(|&n| self.cands[n].label.clone())
                .collect();
            let shown = items.len();
            items.push("Search again…".to_string());
            items.push("Don't add it".to_string());
            let Some(p) = self.prompter.as_deref_mut() else {
                return Ok(Searched::LeftOut);
            };
            match p.select(
                &format!("{} song(s) match; {question}", found.len()),
                &items,
            )? {
                Some(i) if i < shown => return Ok(Searched::Picked(found[i])),
                Some(i) if i == shown => {}
                _ => return Ok(Searched::LeftOut),
            }
        }
    }
}

/// How a search of the song list ended.
enum Searched {
    Picked(usize),
    LeftOut,
    Back,
}

/// A file left out for want of a terminal to ask on.
#[derive(Debug)]
struct LeftOut;

impl std::fmt::Display for LeftOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("left out")
    }
}

impl std::error::Error for LeftOut {}

/// Match each of `paths`, files `sort` found loose, to its songs, ask
/// where needed, copy what is accepted and record the edits. `beside` are
/// the song files the same command added, by where they came from, for a
/// folder's cover.
#[allow(clippy::too_many_lines)]
pub fn attach<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    paths: &[(PathBuf, What)],
    beside: &[(PathBuf, SourceKey)],
    how: &How,
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<Attached> {
    if paths.is_empty() {
        return Ok(Attached::default());
    }
    let mut manifest = Manifest::load(&dirs.home)?;
    let mut state = State::load(&dirs.home)?;
    let store = Store::scan(dirs)?;
    let listed = manifest.keys();
    let temp = crate::atomic::Scratch::new()?;
    {
        let home = &dirs.home;
        let mut measuring = reconcile::Measuring {
            retry: false,
            say_unread: false,
            checkpoint: &mut |s: &State| State::keep_measures(home, s),
            analyses: crate::analysis::wanted(&manifest.settings.loudness),
        };
        reconcile::measure(
            runner,
            &store,
            &listed,
            &mut state,
            temp.path(),
            &mut measuring,
            out,
        )?;
    }
    if paths.iter().any(|(_, w)| *w == What::Picture)
        && reconcile::look(runner, &store, &listed, &mut state, temp.path(), out)?
    {
        State::keep_measures(&dirs.home, &state)?;
    }
    let mut cands = candidates(&manifest, &state, &store, dirs)?;
    let extractors = query::extractors(&manifest);
    let pool: Option<Vec<usize>> = if how.to.is_empty() {
        None
    } else {
        let mut terms = Vec::new();
        for value in &how.to {
            terms.extend(
                shell_words::split(value)
                    .map_err(|e| crate::change::Refused(format!("--to {value}: {e}")))?,
            );
        }
        let q = Query::parse(&terms, &extractors)?;
        let found: Vec<usize> = (0..cands.len())
            .filter(|&n| q.matches(&cands[n].view))
            .collect();
        if found.is_empty() {
            return Err(
                crate::change::Refused(format!("No song matches `{}`", how.to.join(" "))).into(),
            );
        }
        Some(found)
    };
    let mut attached = Attached::default();
    let mut items = Vec::new();
    for (n, (path, what)) in paths.iter().enumerate() {
        let scratch = temp.path().join(format!("loose-{n}"));
        std::fs::create_dir_all(&scratch)?;
        match read_item(runner, path, *what, &scratch) {
            Ok(item) => items.push(item),
            Err(e) => {
                ui::warning(out, &format!("{e:#}; not added"))?;
                attached.unread.push(path.display().to_string());
            }
        }
    }
    read_streams(runner, &mut cands, &items, temp.path());
    let mut ask = Asking {
        cands: &cands,
        extractors: &extractors,
        yes: how.yes,
        prompter,
        out,
    };
    let mut decisions = Vec::new();
    for item in &mut items {
        // A folder's cover given with that folder's songs is theirs.
        if item.what == What::Picture {
            let folder = folder_of(&item.path);
            for (origin, key) in beside {
                if folder.is_some()
                    && folder_of(origin) == folder
                    && let SourceKey::Manual(m) = key
                {
                    item.clues.with.insert(folded_stem(m.path()));
                }
            }
        }
        let decision = match decide(&mut ask, item, pool.as_deref()) {
            Ok(d) => d,
            Err(e) if e.is::<LeftOut>() => Decision::LeftOut,
            Err(e) => {
                ui::warning(ask.out, &format!("{e:#}"))?;
                attached.unread.push(item.path.display().to_string());
                Decision::Declined
            }
        };
        decisions.push(decision);
    }
    let manual = dirs.manual();
    // The files of one song's own, which another song may not be given.
    let mut taken: BTreeSet<SourceKey> = manifest.keys();
    for (item, decision) in items.iter().zip(decisions) {
        match decision {
            Decision::Listed(songs) => {
                // A picture is one file the songs share; lyrics are one
                // song's own, so another song's copy is not reused.
                let key = if item.what == What::Picture {
                    copy_in(&manual, item, &mut BTreeSet::new())?
                } else {
                    copy_in(&manual, item, &mut taken)?
                };
                for n in songs {
                    let id = cands[n].id.clone();
                    manifest.edit(Edit::Add {
                        sources: vec![id.clone(), key.clone()],
                        album: None,
                    });
                    attached.songs.push(id);
                }
            }
            Decision::Tagged { songs, left_out } => {
                attached.left_out += usize::from(left_out);
                for (n, tags) in songs {
                    let id = cands[n].id.clone();
                    let named: Vec<String> = tags
                        .iter()
                        .map(|(k, _)| song_list_name(k).replace('_', " "))
                        .collect();
                    let had = manifest
                        .song_with(&id)
                        .map(|s| s.tags.clone())
                        .unwrap_or_default();
                    let replaced: Vec<String> = tags
                        .iter()
                        .filter(|(k, v)| {
                            had.iter().any(|(h, was)| {
                                crate::tags::vorbis_key(h) == *k && !was.is_empty() && was != v
                            })
                        })
                        .map(|(k, _)| song_list_name(k).replace('_', " "))
                        .collect();
                    let replacing = if replaced.is_empty() {
                        String::new()
                    } else {
                        format!(", replacing the {} set before", replaced.join(", "))
                    };
                    ui::info(
                        ask.out,
                        &format!(
                            "{}: setting {} of {} by hand{replacing}",
                            item.label,
                            named.join(", "),
                            cands[n].label
                        ),
                    )?;
                    manifest.edit(Edit::Tag {
                        key: id.clone(),
                        tags: tags
                            .iter()
                            .map(|(k, v)| (song_list_name(k), v.clone()))
                            .collect(),
                    });
                    attached.songs.push(id);
                }
                if let Some(sheet) = &item.sheet
                    && how.verbose
                {
                    for skipped in &sheet.skipped {
                        ui::info(ask.out, &format!("{}: skipped {skipped}", item.label))?;
                    }
                }
            }
            Decision::Already => {}
            Decision::Declined if attached.unread.contains(&item.path.display().to_string()) => {}
            Decision::Declined => ui::info(ask.out, &format!("{}: not added", item.label))?,
            Decision::LeftOut => attached.left_out += 1,
        }
    }
    manifest.save()?;
    Ok(attached)
}

/// The folder `path` is in, as the file system names it, so two
/// spellings of one folder are one.
fn folder_of(path: &Path) -> Option<PathBuf> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    dunce::canonicalize(parent).ok()
}

/// Whether `n` may be given a file: a song with a source only it lists.
fn ownable(c: &Candidate) -> bool {
    !manifest::shareable(&c.id)
}

#[allow(clippy::too_many_lines)]
fn decide<W: Write>(
    ask: &mut Asking<'_, '_, W>,
    item: &Item,
    pool: Option<&[usize]>,
) -> Result<Decision> {
    let cands = ask.cands;
    if let Some(sheet) = &item.sheet
        && sheet.tracks.len() > 1
    {
        return decide_sheet(ask, item, sheet, pool);
    }
    let judged: Vec<Judged> = judge(cands, &item.clues, item.what)
        .into_iter()
        .filter(|j| ownable(&cands[j.song]))
        .filter(|j| pool.is_none_or(|p| p.contains(&j.song)))
        .collect();
    // A song that holds it already is told so; a picture still goes to
    // the other songs it is meant for.
    let holders: Vec<usize> = judged
        .iter()
        .filter(|j| j.already)
        .map(|j| j.song)
        .collect();
    let held = |n: usize| holders.contains(&n);
    if let Some(&n) = holders.first()
        && (item.what != What::Picture && pool.is_none_or(|p| p.iter().all(|m| held(*m))))
    {
        ask.info(&format!(
            "{}: {} already holds it",
            item.label, cands[n].label
        ))?;
        return Ok(Decision::Already);
    }
    let tagged = |n: usize| -> Decision {
        let tags = item
            .sheet
            .as_ref()
            .map(|s| s.tags_of(0))
            .unwrap_or_default();
        Decision::Tagged {
            songs: vec![(n, tags)],
            left_out: false,
        }
    };
    if let Some(pool) = pool {
        if item.what == What::Picture {
            let wanted: Vec<usize> = pool.iter().copied().filter(|n| !held(*n)).collect();
            return Ok(if wanted.is_empty() {
                Decision::Already
            } else {
                Decision::Listed(wanted)
            });
        }
        let pool: Vec<usize> = pool.iter().copied().filter(|n| !held(*n)).collect();
        let chosen = match pool.as_slice() {
            [one] => Some(*one),
            many => {
                let lines: Vec<String> = many.iter().map(|&n| cands[n].label.clone()).collect();
                if ask.prompter.is_none() {
                    ask.warn(&format!(
                        "{} songs match --to; {} not added. Name one of them:\n  {}",
                        many.len(),
                        item.label,
                        lines.join("\n  ")
                    ))?;
                    return Ok(Decision::LeftOut);
                }
                ask.pick(
                    &format!("{}: {}", item.label, item.what.whose()),
                    &lines,
                    many,
                    item.what.whose(),
                )?
            }
        };
        for j in judged
            .iter()
            .filter(|j| Some(j.song) == chosen && j.verdict == Verdict::Different)
        {
            if let Some(why) = j.reasons.first() {
                ask.warn(&format!("{}: {why}", item.label))?;
            }
        }
        return Ok(match chosen {
            Some(n) if item.what == What::Tags => tagged(n),
            Some(n) => Decision::Listed(vec![n]),
            None => Decision::Declined,
        });
    }
    if item.what == What::Picture {
        let alike: Vec<&Judged> = judged.iter().filter(|j| j.looks == Alike::Same).collect();
        let somewhat: Vec<&Judged> = judged.iter().filter(|j| j.looks == Alike::Unsure).collect();
        let others = judged
            .iter()
            .filter(|j| j.verdict == Verdict::Same && j.reasons.iter().any(|r| r == "came with it"))
            .count();
        if !alike.is_empty() || !somewhat.is_empty() || others > 0 {
            let one_album = alike.iter().all(|j| {
                !cands[j.song].album.is_empty()
                    && cands[j.song].album_key == cands[alike[0].song].album_key
            });
            let (mut sure, mut asked): (Vec<usize>, Vec<usize>) =
                if alike.len() <= MANY || one_album {
                    (alike.iter().map(|j| j.song).collect(), Vec::new())
                } else {
                    (Vec::new(), alike.iter().map(|j| j.song).collect())
                };
            asked.extend(somewhat.iter().map(|j| j.song));
            sure.extend(
                judged
                    .iter()
                    .filter(|j| j.looks != Alike::Same && j.verdict == Verdict::Same)
                    .map(|j| j.song),
            );
            sure.retain(|n| !held(*n));
            asked.retain(|n| !held(*n));
            if sure.is_empty() && asked.is_empty() && !holders.is_empty() {
                ask.info(&format!(
                    "{}: {} already holds it",
                    item.label, cands[holders[0]].label
                ))?;
                return Ok(Decision::Already);
            }
            if !sure.is_empty() {
                let album = &cands[sure[0]].album;
                let whose = if sure.len() > 1
                    && !album.is_empty()
                    && sure.iter().all(|&n| cands[n].album == *album)
                {
                    format!("{} songs of {album}", sure.len())
                } else {
                    sure.iter()
                        .map(|&n| cands[n].label.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                ask.info(&format!(
                    "{}: alike to the covers of {whose}; adding it to each",
                    item.label
                ))?;
            }
            if !asked.is_empty() {
                let lines: Vec<String> = asked
                    .iter()
                    .map(|&n| {
                        let j = judged.iter().find(|j| j.song == n).expect("judged");
                        ask.line(j)
                    })
                    .collect();
                match ask.prompter.as_deref_mut() {
                    Some(p) => {
                        let picked = p.choose(
                            &format!(
                                "{} also looks like these songs' covers; add it to which?",
                                item.label
                            ),
                            &lines,
                            &[],
                        )?;
                        sure.extend(picked.into_iter().filter_map(|i| asked.get(i).copied()));
                    }
                    None if sure.is_empty() => {
                        ask.warn(&format!(
                            "{} looks somewhat like the covers of {} song(s); not added. Name its songs \
                             with --to, or run on a terminal",
                            item.label,
                            asked.len()
                        ))?;
                        return Ok(Decision::LeftOut);
                    }
                    None => ask.warn(&format!(
                        "{}: {} more song(s) have a cover somewhat like it; not added to those",
                        item.label,
                        asked.len()
                    ))?,
                }
            }
            sure.sort_unstable();
            sure.dedup();
            if !sure.is_empty() {
                return Ok(Decision::Listed(sure));
            }
            return Ok(Decision::Declined);
        }
    }
    let question = item.what.whose();
    Ok(match ask.one(&item.label, item.what, &judged, question)? {
        Some(n) if item.what == What::Tags => tagged(n),
        Some(n) => Decision::Listed(vec![n]),
        None => Decision::Declined,
    })
}

/// A sheet of several tracks, each matched to a song of its own.
#[allow(clippy::too_many_lines)]
fn decide_sheet<W: Write>(
    ask: &mut Asking<'_, '_, W>,
    item: &Item,
    sheet: &Sheet,
    pool: Option<&[usize]>,
) -> Result<Decision> {
    let cands = ask.cands;
    let named = |file: &str| {
        sheet
            .tracks
            .iter()
            .filter(|t| t.file.as_deref() == Some(file))
            .count()
    };
    // A file several tracks are in is a whole album's rip, which muman
    // keeps as one song; a song named as it is no track of it.
    if let Some(file) = sheet
        .tracks
        .iter()
        .filter_map(|t| t.file.as_deref())
        .find(|f| named(f) > 1)
        && cands
            .iter()
            .any(|c| c.stems.contains(&folded_stem(Path::new(file))))
    {
        bail!(
            "{} splits {file} into tracks, which muman keeps as one song; \
             its tags are not set",
            item.label
        );
    }
    let mut per_track: Vec<Vec<Judged>> = Vec::new();
    for (n, track) in sheet.tracks.iter().enumerate() {
        let tags = sheet.tags_of(n);
        let clues = Clues {
            readings: vec![fields_of(&tags)],
            length_ms: track.length_ms,
            grace: (CUE_GRACE_MS, CUE_SPAN_MS),
            ids: ids_of(&tags),
            file: track
                .file
                .as_deref()
                .filter(|f| named(f) == 1)
                .map(|f| folded_stem(Path::new(f))),
            ..Clues::default()
        };
        let judged: Vec<Judged> = judge(cands, &clues, What::Tags)
            .into_iter()
            .filter(|j| ownable(&cands[j.song]))
            .filter(|j| pool.is_none_or(|p| p.contains(&j.song)))
            .take(PER_TRACK)
            .collect();
        per_track.push(judged);
    }
    let mut columns: Vec<usize> = per_track.iter().flatten().map(|j| j.song).collect();
    columns.sort_unstable();
    columns.dedup();
    let rows = sheet.tracks.len();
    let cost: Vec<Vec<i64>> = per_track
        .iter()
        .enumerate()
        .map(|(r, judged)| {
            let mut row: Vec<i64> = columns
                .iter()
                .map(|song| {
                    judged
                        .iter()
                        .find(|j| j.song == *song)
                        .map_or(NO_MATCH, |j| match j.verdict {
                            Verdict::Different => NO_MATCH,
                            _ if j.distance >= NO_MATCH => 0,
                            _ => j.distance,
                        })
                })
                .collect();
            row.extend((0..rows).map(|d| if d == r { UNMATCHED } else { NO_MATCH * 10 }));
            row
        })
        .collect();
    let assigned = similar::assign(&cost);
    let mut fits: Vec<(usize, Option<Judged>)> = Vec::new();
    for (r, col) in assigned.into_iter().enumerate() {
        let judged = columns.get(col).and_then(|song| {
            let mut j = per_track[r].iter().find(|j| j.song == *song)?.clone();
            if j.verdict == Verdict::Different {
                return None;
            }
            let next = per_track[r].iter().find(|o| o.song != j.song);
            if j.by_fields
                && j.verdict == Verdict::Same
                && next.is_some_and(|o| o.distance - j.distance < LEAD)
            {
                j.verdict = Verdict::Unsure;
            }
            Some(j)
        });
        fits.push((r, judged));
    }
    let title_of = |r: usize| {
        values(&sheet.tracks[r].tags, "TITLE")
            .next()
            .cloned()
            .unwrap_or_default()
    };
    let all_sure = fits
        .iter()
        .all(|(_, j)| j.as_ref().is_some_and(|j| j.verdict == Verdict::Same));
    let tagged = |chosen: &[(usize, usize)], left_out: bool| {
        if chosen.is_empty() && left_out {
            return Decision::LeftOut;
        }
        Decision::Tagged {
            songs: chosen.iter().map(|&(r, n)| (n, sheet.tags_of(r))).collect(),
            left_out,
        }
    };
    let sure: Vec<(usize, usize)> = fits
        .iter()
        .filter_map(|(r, j)| {
            j.as_ref()
                .filter(|j| j.verdict == Verdict::Same)
                .map(|j| (*r, j.song))
        })
        .collect();
    if all_sure {
        return Ok(tagged(&sure, false));
    }
    for (r, j) in &fits {
        let to = j.as_ref().map_or_else(
            || "(no song)".to_string(),
            |j| {
                format!(
                    "{} ({}{})",
                    cands[j.song].label,
                    j.names().unwrap_or("named as its file"),
                    if j.verdict == Verdict::Same {
                        ""
                    } else {
                        ", asked"
                    }
                )
            },
        );
        ask.info(&format!(
            "{} track {}: “{}” → {to}",
            item.label,
            r + 1,
            title_of(*r)
        ))?;
    }
    let fitting: Vec<(usize, usize)> = fits
        .iter()
        .filter_map(|(r, j)| j.as_ref().map(|j| (*r, j.song)))
        .collect();
    if ask.yes {
        let led: Vec<(usize, usize)> = fits
            .iter()
            .filter_map(|(r, j)| {
                let j = j.as_ref()?;
                let next = per_track[*r].iter().find(|o| o.song != j.song);
                let leads = next.is_none_or(|o| o.distance - j.distance >= LEAD);
                (j.verdict == Verdict::Same || leads).then_some((*r, j.song))
            })
            .collect();
        let unsure = fitting.len() - led.len();
        if unsure > 0 {
            ask.warn(&format!(
                "{}: {unsure} track(s) may be either of two songs; their tags are not set. \
                 Run on a terminal to choose",
                item.label
            ))?;
        }
        return Ok(tagged(&led, unsure > 0));
    }
    if fitting.is_empty() && ask.prompter.is_none() {
        ask.warn(&format!(
            "{}: no track fits a song; not added. Name the songs it is of with --to",
            item.label
        ))?;
        return Ok(Decision::LeftOut);
    }
    let Some(p) = ask.prompter.as_deref_mut() else {
        let unsure = fitting.len() - sure.len();
        if unsure > 0 {
            ask.warn(&format!(
                "{}: {unsure} track(s) are not surely a song's; their tags are not set. \
                 Run on a terminal, or give -y",
                item.label
            ))?;
        }
        return Ok(tagged(&sure, unsure > 0));
    };
    let album = values(&sheet.album, "ALBUM")
        .next()
        .cloned()
        .unwrap_or_default();
    let picked = if fitting.is_empty() {
        Some(1)
    } else {
        p.select(
            &format!(
                "{}: {} of {rows} tracks fit songs{}; set their tags?",
                item.label,
                fitting.len(),
                if album.is_empty() {
                    String::new()
                } else {
                    format!(" of {album}")
                }
            ),
            &[
                format!("Set all {}", fitting.len()),
                "Choose song by song".to_string(),
                "Don't add it".to_string(),
            ],
        )?
    };
    match picked {
        Some(0) => Ok(tagged(&fitting, false)),
        Some(1) => {
            let mut chosen: Vec<(usize, usize)> = Vec::new();
            for (r, judged) in per_track.iter().enumerate() {
                let question = format!("track {} “{}”: which song is it?", r + 1, title_of(r));
                // A song is one track's at most, and only one that may be it.
                let offered: Vec<&Judged> = judged
                    .iter()
                    .filter(|j| j.verdict != Verdict::Different)
                    .filter(|j| !chosen.iter().any(|(_, n)| *n == j.song))
                    .take(OFFERED)
                    .collect();
                let lines: Vec<String> = offered.iter().map(|j| ask.line(j)).collect();
                let songs: Vec<usize> = offered.iter().map(|j| j.song).collect();
                if let Some(n) = ask.pick(
                    &format!("{}, {question}", item.label),
                    &lines,
                    &songs,
                    &question,
                )? && !chosen.iter().any(|(_, m)| *m == n)
                {
                    chosen.push((r, n));
                }
            }
            Ok(tagged(&chosen, false))
        }
        _ => Ok(Decision::Declined),
    }
}

/// Copy `item` into the manual folder's `added/`, or find it there: its
/// key. A copy already there with its bytes is listed again, but for one
/// in `taken`, the keys of other songs' own files, which is copied anew.
fn copy_in(manual: &Path, item: &Item, taken: &mut BTreeSet<SourceKey>) -> Result<SourceKey> {
    let bytes = match &item.bytes {
        Some(b) => b.clone(),
        None => {
            std::fs::read(&item.path).with_context(|| format!("reading {}", item.path.display()))?
        }
    };
    let digest = facts::digest(&bytes);
    let added = manual.join(ADDED);
    std::fs::create_dir_all(&added).with_context(|| format!("creating {}", added.display()))?;
    let stem = item
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("added")
        .to_string();
    let ext = match (item.what, extension(&item.path).as_str()) {
        (What::Lyrics, _) => "lrc".to_string(),
        (_, ext) => ext.to_string(),
    };
    for n in 1.. {
        let name = if n == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let to = added.join(&name);
        let key = SourceKey::Manual(PathBuf::from(ADDED).join(&name).into());
        if to.exists() {
            if !taken.contains(&key) && facts::digest_file(&to).as_deref() == Some(digest.as_str())
            {
                taken.insert(key.clone());
                return Ok(key);
            }
            continue;
        }
        crate::atomic::write(&added, &crate::atomic::Name::new(&name)?, &bytes)?;
        taken.insert(key.clone());
        return Ok(key);
    }
    unreachable!("some name is free")
}

#[cfg(test)]
mod tests;
