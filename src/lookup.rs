//! A song looking other sources up: each provider a song asks until it
//! has what that provider's `until` names, unless a `skip` query or the
//! song's own `lookups` leaves it out, made unless one made the same way
//! found something, or found nothing lately. What a lookup finds joins
//! the song, and may start lookups of its own in the next round.
//!
//! Each lookup is recorded in the state by the source it was made from,
//! the provider looked up and that provider's `method`, with when and
//! what came of it. A song's record of a provider is the newest made from
//! any of its sources. It is made again:
//!
//! | Recorded | Made again |
//! |---|---|
//! | Nothing | At once |
//! | Made another way | At once, unless it found a source the song lists |
//! | Found nothing, or an instrumental | After the provider's `recheck` |
//! | Failed | After an hour, doubling with each failure, up to a week |
//! | Found a source the song lists that falls short | After `recheck`, doubling each time it is found again unchanged, up to 90 days |
//! | Found a source, or declined | Never |
//!
//! A source the song lists from the provider is judged by what it gives
//! itself, not by what the song resolves to: it falls short when it
//! lacks a condition of `until` the song lacks too. A cover that loses to
//! a square one, or lyrics a pin passes over, never makes a lookup that
//! would find the same record again. A record that falls short is asked
//! about again in its provider's own way: LRCLIB is searched again, as a
//! better record may have joined it; a MusicBrainz record or a Cover Art
//! Archive cover is fetched again by its ID, which a search would find
//! again; a YouTube video is never looked for again, a search costing a
//! listing and downloads that find the same upload. A record found again
//! replaces the stored copy when it differs.
//!
//! A lookup a service refused for going too fast, after the retries
//! [`crate::http::Service`] makes, or that reached no server, as when the
//! network is down ([`crate::http::Unreachable`]), is recorded as nothing
//! at all: it and the provider's other lookups that run, in its later
//! rounds too, are put off to the next run, rather than each waiting out
//! its timeout and then a failure's backoff. A service that answered with
//! an error, or with what does not read, failed that lookup alone.
//!
//! A listed record the store no longer holds is fetched again by its ID
//! ([`refetch`]); one that fails, or that its service no longer has,
//! waits as a source yt-dlp fetches does, an hour doubling, recorded in
//! the state's `failures`.
//!
//! A song makes at most one lookup per provider a run, so a release and
//! its upload never look each other up in turn. A MusicBrainz record the
//! song lists counts for AcoustID too, so a search by name never follows
//! a fingerprint's find. Its MusicBrainz lookup waits while its AcoustID
//! lookup is due, since both find a MusicBrainz record and the
//! fingerprint names the recording surer than its names do; its Cover Art
//! Archive lookup waits while either is due, so it asks for the release
//! group the record names rather than searching for the album by its
//! names. A round AcoustID or MusicBrainz answered is followed by another
//! even when nothing joined, so a song AcoustID could not place is
//! searched for by name, and its cover looked up, in the same run. Each
//! round measures what the conditions read, makes its lookups and fetches
//! what they found, without holding the run's lock; what one round adds
//! may make the next due. A provider's `per_run` cap lets a backlog, as
//! the first sync of a library never looked up has, drain over several
//! runs rather than all at once on a free service. `sync --until-done`
//! lifts the cap and the number of rounds, and asks again at once what
//! failed, so one run leaves nothing due; it still makes each lookup once
//! a run, at each service's pace.
//!
//! A YouTube Music track found later is fetched beside its upload and
//! joins the song; the upload stays a source when the two are one
//! recording, else the track takes its key. An LRCLIB or MusicBrainz
//! record found is kept in the store and joins the song, even when
//! another song lists it already, as another release of one recording
//! does: a record is data songs share. A video the run already knows,
//! listed, removed or replaced, is recorded found but never added again.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::io::Write;
use std::path::Path;

use std::sync::OnceLock;

use crate::http::{self, HttpTransport, Throttle};
use anyhow::{Context, Result, anyhow};

use crate::acoustid;
use crate::acquire::{self, Acquire};
use crate::align::Alignment;
use crate::coverart;
use crate::dirs::Dirs;
use crate::facts::Facts;
use crate::lrclib::{self, Found, Query, Record};
use crate::manifest::LookupsOff;
use crate::manifest::{Edit, Manifest};
use crate::music::watch_url;
use crate::music::{self, Entry};
use crate::musicbrainz::{self, Mbid};
use crate::parallel;
use crate::provider::{COVERART, LRCLIB, MUSICBRAINZ, Provider};
use crate::query::{self as songs, View};
use crate::reconcile::{self, Measuring};
use crate::resolve::{self, Resolved};
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::{self, Looked, Outcome, State};
use crate::store::{Located, Store};
use crate::tags::Field;

/// Rounds of lookups one run makes: what one round finds starts lookups
/// of the next, as an upload's release then finds the release's lyrics.
/// `sync --until-done` goes on until a round has nothing left to ask.
pub const ROUNDS: usize = 3;

/// The providers looked up, in the order a round makes their lookups.
pub const ORDER: [Provider; 6] = [
    Provider::YouTubeMusic,
    Provider::YouTube,
    Provider::Lrclib,
    Provider::AcoustId,
    Provider::MusicBrainz,
    Provider::CoverArt,
];

/// How a provider's lookups are made; a lookup made another way is due
/// again.
#[must_use]
pub fn method(p: Provider) -> &'static str {
    match p {
        Provider::YouTubeMusic => "youtube-music/2",
        Provider::YouTube => "youtube/1",
        Provider::Lrclib => "lrclib/2",
        Provider::MusicBrainz => "musicbrainz/1",
        Provider::AcoustId => "acoustid/1",
        Provider::CoverArt => "coverart/2",
        Provider::Manual => "manual",
    }
}

/// A lookup due: the song, the source it is made from, and what it finds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub song: usize,
    pub from: SourceKey,
    pub find: Provider,
    /// The record the song lists from the provider, which falls short,
    /// with how many times it was found again unchanged: fetched again by
    /// its ID, or for LRCLIB searched for again.
    pub again: Option<(SourceKey, u32)>,
}

/// How a run asks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ask {
    /// Make every lookup now, whatever its record says: `sync --rematch`.
    pub rematch: bool,
    /// Ask until nothing is due: no `per_run` cap, no limit of rounds,
    /// and what failed asked again at once: `sync --until-done`.
    pub persist: bool,
}

/// What a song's lookup of one provider comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Due(Due),
    /// Waits while the song's lookup of this provider is due.
    Waits(Provider),
    /// Asked again then, in seconds since the Unix epoch.
    Later(u64),
    /// Asked no more: declined, or found a source the song took out.
    Never,
    /// Nothing left to ask: the song has what `until` names (`None`), or
    /// the source it lists from the provider gives all it can.
    Done(Option<SourceKey>),
    /// Left out by this `skip` query, or the song's own `lookups`.
    Skipped(String),
    /// The provider is turned off.
    Off,
    /// No output writes what it finds, or the song pins its own.
    Unwritten,
    /// Why the song cannot ask it.
    CannotAsk(&'static str),
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Due(d) => match &d.again {
                Some((key, _)) => write!(f, "asks now about {key}, which falls short"),
                None => f.write_str("asks now"),
            },
            Self::Waits(p) => write!(f, "waits for {p}"),
            Self::Later(at) => write!(f, "again on {}", date_of(*at)),
            Self::Never => f.write_str("never again: declined, or what it found was taken out"),
            Self::Done(None) => f.write_str("done: the song has what `until` names"),
            Self::Done(Some(key)) => write!(f, "done: {key} gives what it can"),
            Self::Skipped(why) => write!(f, "skipped: {why}"),
            Self::Off => f.write_str("off"),
            Self::Unwritten => f.write_str("not sought: no output writes it, or the song pins it"),
            Self::CannotAsk(why) => write!(f, "cannot ask: {why}"),
        }
    }
}

/// What a Cover Art Archive lookup asks for a resolved song: its album
/// and album artist, and the MusicBrainz IDs it names of them.
#[must_use]
pub fn cover_query_of(r: &Resolved) -> Option<coverart::Query> {
    let tag = |f: Field| {
        r.plan
            .tags
            .iter()
            .find(|(k, _)| k == f.vorbis())
            .and_then(|(_, v)| v.first().cloned())
            .filter(|v| !v.trim().is_empty())
    };
    Some(coverart::Query {
        album: tag(Field::Album)?,
        album_artist: tag(Field::AlbumArtist).or_else(|| tag(Field::Artist))?,
        release_group: tag(Field::MusicBrainzReleaseGroupId),
        release: tag(Field::MusicBrainzAlbumId),
    })
}

/// Whether a song resolves to an album a source or the song list names,
/// rather than a single named for its title.
fn album_of(r: Option<&Resolved>) -> bool {
    r.is_some_and(|r| {
        r.why
            .tags
            .iter()
            .any(|t| t.key == Field::Album.vorbis() && t.from != resolve::SINGLE)
    })
}

/// What an LRCLIB or MusicBrainz lookup asks for a resolved song: its
/// title, first artist, album and length; no album for a single named
/// for its title, which no source names. A song lacking any but the
/// album is asked of neither.
#[must_use]
pub fn query_of(r: &Resolved, facts: &BTreeMap<SourceKey, Facts>) -> Option<Query> {
    Some(Query {
        title: tag_of(r, Field::Title)?,
        artist: tag_of(r, Field::Artist)?,
        album: tag_of(r, Field::Album).filter(|_| album_of(Some(r))),
        seconds: facts.get(&r.plan.audio.key)?.length()?,
    })
}

/// What an AcoustID lookup asks for a resolved song: the print and length
/// of its audio, with what names and album it has. A song whose audio
/// lacks either is not asked.
#[must_use]
pub fn print_query_of(r: &Resolved, facts: &BTreeMap<SourceKey, Facts>) -> Option<acoustid::Query> {
    let audio = facts.get(&r.plan.audio.key)?;
    Some(acoustid::Query {
        print: audio.print.clone()?,
        seconds: audio.length()?,
        title: tag_of(r, Field::Title),
        artist: tag_of(r, Field::Artist),
        album: tag_of(r, Field::Album).filter(|_| album_of(Some(r))),
    })
}

/// The first value a resolved song writes for `f`.
fn tag_of(r: &Resolved, f: Field) -> Option<String> {
    r.plan
        .tags
        .iter()
        .find(|(k, _)| k == f.vorbis())
        .and_then(|(_, v)| v.first().cloned())
}

/// The day `secs` after the Unix epoch falls on, in UTC, as `2026-11-04`.
fn date_of(secs: u64) -> String {
    // Civil from days, after Howard Hinnant's `civil_from_days`.
    let z = i64::try_from(secs / 86_400).unwrap_or(i64::MAX / 2) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Every lookup due, a song's for each provider at most once.
pub fn due(
    manifest: &Manifest,
    state: &State,
    planned: &[(usize, Resolved)],
    now: u64,
    ask: Ask,
) -> Result<Vec<Due>> {
    Ok(verdicts(manifest, state, planned, now, ask)?
        .into_iter()
        .filter_map(|(_, _, v)| match v {
            Verdict::Due(d) => Some(d),
            _ => None,
        })
        .collect())
}

/// What each song's lookup of each provider comes to, by song, in
/// [`ORDER`].
pub fn verdicts(
    manifest: &Manifest,
    state: &State,
    planned: &[(usize, Resolved)],
    now: u64,
    ask: Ask,
) -> Result<Vec<(usize, Provider, Verdict)>> {
    let config = &manifest.providers;
    let resolved: HashMap<usize, &Resolved> = planned.iter().map(|(n, r)| (*n, r)).collect();
    let views: Vec<View> = manifest
        .songs
        .iter()
        .enumerate()
        .map(|(n, song)| songs::view_of(song, resolved.get(&n).copied(), &state.facts))
        .collect();
    let extractors = songs::extractors(manifest);
    let mut skips: BTreeMap<Provider, Vec<(&str, songs::Query)>> = BTreeMap::new();
    for p in ORDER {
        for text in config.skips(p) {
            let q = songs::parse_text(text, &extractors)
                .and_then(|q| q.check_fields(&views).map(|()| q))
                .with_context(|| format!("[providers] skip: `{text}`"))?;
            skips.entry(p).or_default().push((text, q));
        }
    }
    let out = Outputs {
        lyrics: !manifest.settings.lyrics.out.is_empty(),
        cover: !manifest.settings.cover.out.is_empty(),
    };
    let mut found = Vec::new();
    for (n, song) in manifest.songs.iter().enumerate() {
        let asking = Asking {
            n,
            song,
            view: &views[n],
            resolved: resolved.get(&n).copied(),
            state,
            now,
            ask,
        };
        let mut mine: Vec<(Provider, Verdict)> = ORDER
            .into_iter()
            .map(|p| {
                let skipped = skips
                    .get(&p)
                    .and_then(|s| s.iter().find(|(_, q)| q.matches(&views[n])))
                    .map(|(text, _)| (*text).to_string());
                (p, asking.verdict(p, config, skipped, out))
            })
            .collect();
        let is_due = |mine: &[(Provider, Verdict)], p: Provider| {
            mine.iter()
                .any(|(q, v)| *q == p && matches!(v, Verdict::Due(_)))
        };
        let waits = [
            (Provider::MusicBrainz, vec![Provider::AcoustId]),
            (
                Provider::CoverArt,
                vec![Provider::AcoustId, Provider::MusicBrainz],
            ),
        ];
        for (p, after) in waits {
            if let Some(first) = after.into_iter().find(|a| is_due(&mine, *a))
                && let Some((_, v)) = mine
                    .iter_mut()
                    .find(|(q, v)| *q == p && matches!(v, Verdict::Due(_)))
            {
                *v = Verdict::Waits(first);
            }
        }
        found.extend(mine.into_iter().map(|(p, v)| (n, p, v)));
    }
    Ok(found)
}

/// Whether any output writes lyrics, and covers.
#[derive(Debug, Clone, Copy)]
struct Outputs {
    lyrics: bool,
    cover: bool,
}

/// One song as its lookups are decided.
struct Asking<'a> {
    n: usize,
    song: &'a crate::manifest::Song,
    view: &'a View,
    resolved: Option<&'a Resolved>,
    state: &'a State,
    now: u64,
    ask: Ask,
}

impl Asking<'_> {
    /// What the song's lookup of `p` comes to, `skipped` naming the
    /// `skip` query that leaves it out, if any.
    fn verdict(
        &self,
        p: Provider,
        config: &crate::provider::Config,
        skipped: Option<String>,
        out: Outputs,
    ) -> Verdict {
        let song = self.song;
        let facts = &self.state.facts;
        let settings = config.settings(p);
        if !config.on(p) {
            return Verdict::Off;
        }
        match &song.lookups_off {
            LookupsOff::All => return Verdict::Skipped("the song sets lookups = false".into()),
            off if off.covers(p) => {
                return Verdict::Skipped(format!("the song sets lookups.{p} = false"));
            }
            _ => {}
        }
        if let Some(query) = skipped {
            return Verdict::Skipped(format!("[providers] skip '{query}'"));
        }
        let lyrics = matches!(p, Provider::YouTube | Provider::Lrclib);
        if (lyrics && (!out.lyrics || song.lyrics.is_some()))
            || (p == Provider::CoverArt && (!out.cover || song.cover.is_some()))
        {
            return Verdict::Unwritten;
        }
        if settings.until.iter().all(|c| c.holds(self.view)) {
            return Verdict::Done(None);
        }
        let kinds: Vec<(&SourceKey, Provider)> = song
            .sources
            .iter()
            .filter_map(|k| Some((k, Provider::of(k, facts.get(k))?)))
            .collect();
        let of = |wanted: &[Provider]| {
            kinds
                .iter()
                .find(|(_, q)| wanted.contains(q))
                .map(|(k, _)| (*k).clone())
        };
        let from = match p {
            Provider::YouTubeMusic => of(&[Provider::YouTube]),
            Provider::YouTube => of(&[Provider::YouTubeMusic]),
            _ => of(&[Provider::Manual, Provider::YouTube, Provider::YouTubeMusic]),
        };
        let Some(from) = from else {
            return Verdict::CannotAsk(match p {
                Provider::YouTubeMusic => "no YouTube upload to search from",
                Provider::YouTube => "no YouTube Music release to search from",
                _ => "no source of its own to ask from",
            });
        };
        let recheck = settings.recheck;
        if let Some(record) = of(&[p.yields()]) {
            return self.again(p, settings, from, record);
        }
        let r = self.resolved;
        let askable = match p {
            Provider::CoverArt => r.and_then(cover_query_of).is_some(),
            Provider::AcoustId => r
                .and_then(|r| print_query_of(r, &self.state.facts))
                .is_some(),
            p if p.kept() => self.query().is_some(),
            _ => true,
        };
        if !askable {
            return Verdict::CannotAsk(match p {
                Provider::CoverArt => "the song names no album and album artist",
                Provider::AcoustId => "its audio has no fingerprint or length",
                _ => "the song has no title, artist or length",
            });
        }
        let last = self.state.newest(&song.sources, &[p]);
        if !self.ask.rematch
            && let Some(l) = last
        {
            let failed = matches!(l.outcome, Outcome::Failed { .. });
            match l.again_at(recheck, &|k| song.has(k)) {
                _ if failed && self.ask.persist => {}
                None => return Verdict::Never,
                Some(at) if self.now < at => return Verdict::Later(at),
                Some(_) => {}
            }
        }
        Verdict::Due(Due {
            song: self.n,
            from,
            find: p,
            again: None,
        })
    }

    /// What the song's lookup of `p` comes to when it lists `record`, a
    /// source from it, asked from `from`: done when the record gives what
    /// it can, else asked about again in the provider's own way.
    fn again(
        &self,
        p: Provider,
        settings: &crate::provider::Settings,
        from: SourceKey,
        record: SourceKey,
    ) -> Verdict {
        if matches!(
            p,
            Provider::YouTube | Provider::YouTubeMusic | Provider::AcoustId
        ) {
            return Verdict::Done(Some(record));
        }
        // A record not measured yet is judged once the round measures it.
        let Some(kept) = self.state.facts.get(&record) else {
            return Verdict::Done(Some(record));
        };
        let own = songs::source_view(&record, Some(kept));
        let short = settings
            .until
            .iter()
            .any(|c| !c.holds(self.view) && !c.holds(&own));
        if !short {
            return Verdict::Done(Some(record));
        }
        if p == Provider::Lrclib && self.query().is_none() {
            return Verdict::CannotAsk("the song has no title, artist or length");
        }
        let finds: &[Provider] = if p == Provider::MusicBrainz {
            &[Provider::MusicBrainz, Provider::AcoustId]
        } else {
            &[p]
        };
        let last = self
            .state
            .newest(&self.song.sources, finds)
            .filter(|l| l.outcome == Outcome::Found(record.clone()));
        let at = last.map_or(0, |l| l.short_again_at(settings.recheck));
        if !self.ask.rematch && self.now < at {
            return Verdict::Later(at);
        }
        Verdict::Due(Due {
            song: self.n,
            from,
            find: p,
            again: Some((record, last.map_or(0, |l| l.unchanged))),
        })
    }

    fn query(&self) -> Option<Query> {
        self.resolved.and_then(|r| query_of(r, &self.state.facts))
    }
}

/// What one lookup found.
// Short-lived, so its size matters little.
#[expect(clippy::large_enum_variant)]
#[derive(Clone)]
enum Hit {
    /// The release of an upload: the upload's entry, the release's, and
    /// how their audio compared.
    Track(Entry, Entry, Option<Alignment>),
    /// An upload of a release with subtitles, the same recording.
    Upload(Entry, Alignment),
    Lyrics(Box<Record>),
    Instrumental,
    /// A MusicBrainz recording.
    Tags(Box<musicbrainz::Record>),
    /// The front of the song's album.
    Cover(Box<coverart::Cover>),
    Nothing,
}

/// A copy of `hit` for another song that asked the same: a song of the
/// same album.
fn again(hit: &Result<Hit>) -> Result<Hit> {
    match hit {
        Ok(h) => Ok(h.clone()),
        Err(e) => Err(http::put_off(e).unwrap_or_else(|| anyhow!("{e:#}"))),
    }
}

fn entry_of<R: Runner>(runner: &R, clients: &Clients<'_>, key: &SourceKey) -> Result<Entry> {
    let id = key.id().context("not a video")?;
    music::entries(
        &runner.output(&music::video_command(id, clients.ytdlp))?,
        clients.sites,
    )
    .into_iter()
    .next()
    .ok_or_else(|| anyhow!("yt-dlp listed nothing for {key}"))
}

/// The pace of a run's requests to each service, shared by every
/// thread, and the releases the run has looked up, so an album's songs
/// ask for theirs once a run, whichever round or refetch asks.
#[derive(Debug)]
pub struct Throttles {
    pub lrclib: Throttle,
    pub musicbrainz: Throttle,
    pub acoustid: Throttle,
    pub coverart: Throttle,
    pub releases: musicbrainz::Releases,
}

impl Throttles {
    /// The pace each service asks of a client.
    #[must_use]
    pub fn polite() -> Self {
        Self {
            lrclib: lrclib::throttle(),
            musicbrainz: musicbrainz::throttle(),
            acoustid: acoustid::throttle(),
            coverart: coverart::throttle(),
            releases: musicbrainz::Releases::default(),
        }
    }

    /// No spacing, for tests.
    #[must_use]
    pub fn none() -> Self {
        Self {
            lrclib: Throttle::none(),
            musicbrainz: Throttle::none(),
            acoustid: Throttle::none(),
            coverart: Throttle::none(),
            releases: musicbrainz::Releases::default(),
        }
    }
}

/// The services a run asks, at the addresses the song list sets.
struct Clients<'a> {
    lrclib: lrclib::Client<'a>,
    musicbrainz: musicbrainz::Client<'a>,
    /// Whether `[providers.musicbrainz]` is on.
    musicbrainz_on: bool,
    acoustid: acoustid::Client<'a>,
    coverart: &'a str,
    transport: &'a (dyn HttpTransport + Sync),
    throttle: &'a Throttle,
    sites: &'a crate::sites::Sites,
    ytdlp: &'a crate::download::Passed,
}

impl Clients<'_> {
    /// The Cover Art Archive, searching MusicBrainz for what a song's
    /// tags do not name while it is on.
    fn coverart(&self) -> coverart::Client<'_> {
        coverart::Client {
            base: self.coverart,
            transport: self.transport,
            throttle: self.throttle,
            musicbrainz: self.musicbrainz_on.then_some(&self.musicbrainz),
        }
    }
}

/// The base address the song list sets for a provider.
fn base(manifest: &Manifest, p: Provider) -> String {
    manifest
        .providers
        .settings(p)
        .url
        .clone()
        .unwrap_or_default()
}

/// What a song's lookups ask, of each kind it can be asked.
struct Asked<'a> {
    query: Option<&'a Query>,
    cover: Option<&'a coverart::Query>,
    print: Option<&'a acoustid::Query>,
    /// The album a MusicBrainz record asked about again named when kept,
    /// so it is fetched on the same release.
    album: Option<&'a str>,
}

fn look<R: Runner>(
    runner: &R,
    d: &Due,
    located: Option<&Located>,
    asked: &Asked<'_>,
    clients: &Clients<'_>,
    audio: &Path,
) -> Result<Hit> {
    let query = || {
        asked
            .query
            .context("the song has no title, artist or length")
    };
    Ok(match d.find {
        Provider::YouTubeMusic => {
            let entry = entry_of(runner, clients, &d.from)?;
            match acquire::lookup(runner, clients.sites, clients.ytdlp, &entry, audio)? {
                Some((track, a)) => Hit::Track(entry, track, a),
                None => Hit::Nothing,
            }
        }
        Provider::YouTube => {
            let entry = entry_of(runner, clients, &d.from)?;
            let path = &located.context("its file is not in the store")?.path;
            let own = acquire::local_envelope(runner, path)?;
            match acquire::reverse_lookup(
                runner,
                clients.sites,
                clients.ytdlp,
                &entry,
                &own,
                audio,
            )? {
                Some((upload, a)) => Hit::Upload(upload, a),
                None => Hit::Nothing,
            }
        }
        Provider::Lrclib => match clients.lrclib.find(query()?)? {
            Found::Lyrics(r) => Hit::Lyrics(Box::new(r)),
            Found::Instrumental(_) => Hit::Instrumental,
            Found::Nothing => Hit::Nothing,
        },
        Provider::MusicBrainz => {
            let found = match &d.again {
                Some((key, _)) => {
                    let id = key.id().and_then(Mbid::parse).context("not a record ID")?;
                    clients.musicbrainz.by_id(&id, asked.album)?
                }
                None => clients.musicbrainz.find(query()?)?,
            };
            found.map_or(Hit::Nothing, |r| Hit::Tags(Box::new(r)))
        }
        Provider::AcoustId => {
            let q = asked
                .print
                .context("the song's audio has no print or length")?;
            let found = match clients.acoustid.find(q)? {
                Some(id) => clients.musicbrainz.by_id(&id, q.album.as_deref())?,
                None => None,
            };
            found.map_or(Hit::Nothing, |r| Hit::Tags(Box::new(r)))
        }
        Provider::CoverArt => {
            let found = match &d.again {
                Some((key, _)) => {
                    let id = key.id().and_then(Mbid::parse).context("not a record ID")?;
                    clients.coverart().by_id(&id)?
                }
                None => clients
                    .coverart()
                    .find(asked.cover.context("the song names no album")?)?,
            };
            found.map_or(Hit::Nothing, |c| Hit::Cover(Box::new(c)))
        }
        Provider::Manual => Hit::Nothing,
    })
}

/// `key` added to the song listing `from`.
fn join(
    manifest: &mut Manifest,
    known: &mut BTreeSet<SourceKey>,
    from: &SourceKey,
    key: &SourceKey,
) {
    manifest.edit(Edit::Add {
        sources: vec![from.clone(), key.clone()],
        album: None,
    });
    known.insert(key.clone());
}

/// What a run has made of its lookups across its rounds, and so what a
/// round may still make: each provider's lookups against its `per_run`,
/// the lookups each source has made, and the providers whose service
/// refused or could not be reached. A round asks [`ThisRun::admit`] alone,
/// so a song makes at most one lookup per provider a run, such a provider
/// is asked nothing more, and `per_run` counts the whole run.
#[derive(Debug, Default)]
struct ThisRun {
    made: BTreeMap<Provider, usize>,
    asked: BTreeSet<(SourceKey, Provider)>,
    refusing: BTreeSet<Provider>,
    /// Lookups the last round left past `per_run`, by provider.
    waiting: BTreeMap<Provider, usize>,
}

impl ThisRun {
    /// The lookups of `due` this run may still make, each counted as
    /// made; `per_run` gives each provider's cap, 0 for none.
    fn admit(&mut self, due: Vec<Due>, per_run: impl Fn(Provider) -> usize) -> Vec<Due> {
        self.waiting.clear();
        let mut admitted = Vec::new();
        for d in due {
            if self.refusing.contains(&d.find) || self.asked.contains(&(d.from.clone(), d.find)) {
                continue;
            }
            let limit = per_run(d.find);
            let n = self.made.entry(d.find).or_insert(0);
            if limit > 0 && *n >= limit {
                *self.waiting.entry(d.find).or_insert(0) += 1;
                continue;
            }
            *n += 1;
            self.asked.insert((d.from.clone(), d.find));
            admitted.push(d);
        }
        admitted
    }

    /// `provider`'s service refused for going too fast, or could not be
    /// reached: nothing more is asked of it.
    fn refused(&mut self, provider: Provider) {
        self.refusing.insert(provider);
    }
}

/// Make every lookup due, joining what is found to its song, for up to
/// [`ROUNDS`] rounds, or until none is due when `ask.persist`.
/// `ask.rematch` makes the first round's lookups whatever was found
/// before; a YouTube Music lookup from a key in `declined` is recorded
/// declined instead. Returns whether every fetch succeeded.
// The rounds and each round's steps, in the order they run.
#[expect(clippy::too_many_lines)]
pub fn run<R: Runner, W: Write>(
    acquire: &mut Acquire<'_, R, W>,
    dirs: &Dirs,
    http: &(dyn HttpTransport + Sync),
    throttles: &Throttles,
    ask: Ask,
    declined: &BTreeSet<SourceKey>,
) -> Result<bool> {
    let home = &dirs.home;
    let runner = acquire.runner;
    let mut ok = true;
    let mut ask = ask;
    let mut this_run = ThisRun::default();
    for round in 0.. {
        if round >= ROUNDS && !ask.persist {
            break;
        }
        let mut manifest = Manifest::load(home)?;
        let mut state = State::load(home)?;
        let store = Store::scan(dirs)?;
        let scratch = acquire.temp().join(format!("lookup-{round}"));
        let mut how = Measuring {
            retry: false,
            say_unread: false,
            checkpoint: &mut |s: &State| State::keep_measures(home, s),
            analyses: crate::analysis::wanted(&manifest.settings.loudness),
        };
        reconcile::measure(
            runner,
            &store,
            &manifest.keys(),
            &mut state,
            &scratch,
            &mut how,
            acquire.out,
        )?;
        let on_disk: BTreeSet<SourceKey> = manifest
            .keys()
            .into_iter()
            .filter(|k| store.has(k))
            .collect();
        let (planned, _) = reconcile::plan(
            &manifest,
            &state,
            &dirs.library,
            Some(&on_disk),
            &mut std::io::sink(),
        )?;
        let (refused, due): (Vec<Due>, Vec<Due>) =
            due(&manifest, &state, &planned, state::now_secs(), ask)?
                .into_iter()
                .partition(|d| d.find == Provider::YouTubeMusic && declined.contains(&d.from));
        let mut records: Vec<Looked> = refused
            .into_iter()
            .map(|d| Looked::now(d.from, d.find, Outcome::Declined))
            .collect();
        let due = this_run.admit(due, |p| {
            if ask.persist {
                0
            } else {
                manifest.providers.settings(p).per_run
            }
        });
        if due.is_empty() {
            State::keep_lookups(home, &records, &[])?;
            break;
        }
        let counts: Vec<String> = ORDER
            .into_iter()
            .filter_map(|p| {
                let n = due.iter().filter(|d| d.find == p).count();
                (n > 0).then(|| format!("{n} on {p}"))
            })
            .collect();
        crate::ui::info(
            acquire.out,
            &format!("Looking songs up: {}", counts.join(", ")),
        )?;

        let bases = (
            base(&manifest, Provider::Lrclib),
            base(&manifest, Provider::MusicBrainz),
            base(&manifest, Provider::CoverArt),
            base(&manifest, Provider::AcoustId),
        );
        let key = manifest
            .providers
            .settings(Provider::AcoustId)
            .key
            .clone()
            .unwrap_or_default();
        let clients = Clients {
            lrclib: lrclib::Client {
                base: &bases.0,
                transport: http,
                throttle: &throttles.lrclib,
            },
            musicbrainz: musicbrainz::Client {
                base: &bases.1,
                transport: http,
                throttle: &throttles.musicbrainz,
                releases: &throttles.releases,
            },
            musicbrainz_on: manifest.providers.on(Provider::MusicBrainz),
            acoustid: acoustid::Client {
                base: &bases.3,
                key: &key,
                transport: http,
                throttle: &throttles.acoustid,
            },
            coverart: &bases.2,
            transport: http,
            throttle: &throttles.coverart,
            sites: &manifest.settings.sites,
            ytdlp: acquire.fetcher.passed,
        };
        let audio = acquire.temp().join("audio");
        let resolved: HashMap<usize, &Resolved> = planned.iter().map(|(n, r)| (*n, r)).collect();
        let mut hits: Vec<(Due, Result<Hit>)> = Vec::new();
        let step = crate::progress::step("Looking up", Some(due.len() as u64));
        for p in ORDER {
            let group: Vec<&Due> = due.iter().filter(|d| d.find == p).collect();
            if group.is_empty() {
                continue;
            }
            let workers = manifest.providers.settings(p).concurrency;
            let covers: Vec<Option<coverart::Query>> = group
                .iter()
                .map(|d| {
                    resolved
                        .get(&d.song)
                        .filter(|_| p == Provider::CoverArt && d.again.is_none())
                        .and_then(|r| cover_query_of(r))
                })
                .collect();
            // One lookup an album: a later song of it takes the first's.
            let first: Vec<usize> = (0..group.len())
                .map(|i| {
                    covers[..i]
                        .iter()
                        .position(|q| q.is_some() && *q == covers[i])
                        .unwrap_or(i)
                })
                .collect();
            let asking: Vec<usize> = (0..group.len()).filter(|i| first[*i] == *i).collect();
            let stopped: OnceLock<anyhow::Error> = OnceLock::new();
            let asked = parallel::map(&asking, workers, |&i| {
                let d = group[i];
                if let Some(e) = stopped.get().and_then(http::put_off) {
                    return Err(e);
                }
                let _working = step.working(&reconcile::name_of(
                    &manifest.songs[d.song],
                    resolved.get(&d.song).copied(),
                    &state.facts,
                ));
                let r = resolved.get(&d.song);
                let query = r.and_then(|r| query_of(r, &state.facts));
                let print = r
                    .filter(|_| p == Provider::AcoustId)
                    .and_then(|r| print_query_of(r, &state.facts));
                let album = d
                    .again
                    .as_ref()
                    .and_then(|(key, _)| state.facts.get(key))
                    .and_then(|f| f.tags.get(&Field::Album))
                    .and_then(|o| o.values.first())
                    .map(String::as_str);
                let asked = Asked {
                    query: query.as_ref(),
                    cover: covers[i].as_ref(),
                    print: print.as_ref(),
                    album,
                };
                let hit = look(
                    runner,
                    d,
                    store.locate(&d.from).as_ref(),
                    &asked,
                    &clients,
                    // A folder each: two lookups downloading one candidate
                    // at once would write one file.
                    &audio.join(format!("{}-{}", d.song, d.find)),
                );
                if let Some(e) = hit.as_ref().err().and_then(http::put_off) {
                    let _ = stopped.set(e);
                }
                hit
            });
            // The songs that took another's lookup are done with it.
            step.advance((group.len() - asking.len()) as u64);
            let found: Vec<Result<Hit>> = (0..group.len())
                .map(|i| {
                    let at = asking.binary_search(&first[i]).unwrap_or_default();
                    again(&asked[at])
                })
                .collect();
            hits.extend(group.into_iter().cloned().zip(found));
        }

        let printed = hits.iter().any(|(d, hit)| {
            matches!(d.find, Provider::AcoustId | Provider::MusicBrainz)
                && !hit.as_ref().is_err_and(|e| http::put_off(e).is_some())
        });
        let mut replaced = Vec::new();
        // Records kept anew: the songs listing them follow what they hold now.
        let mut anew: Vec<SourceKey> = Vec::new();
        let mut joined = 0_usize;
        let mut put_off: BTreeMap<Provider, (usize, String)> = BTreeMap::new();
        for (d, hit) in hits {
            let name = reconcile::name_of(
                &manifest.songs[d.song],
                resolved.get(&d.song).copied(),
                &state.facts,
            );
            if let Err(e) = &hit
                && let Some(why) = http::put_off(e)
            {
                put_off.entry(d.find).or_insert((0, why.to_string())).0 += 1;
                continue;
            }
            let mut unchanged = 0;
            let mut again = |key: &SourceKey, differs: bool| {
                if let Some((k, n)) = &d.again
                    && k == key
                    && !differs
                {
                    unchanged = n + 1;
                }
            };
            let outcome = match hit {
                // No yt-dlp fails every lookup alike; the run stops and
                // says so, rather than putting each song off for a week.
                Err(e) if e.downcast_ref::<crate::tools::ToolError>().is_some() => {
                    return Err(e);
                }
                Err(e) => {
                    crate::ui::warning(
                        acquire.out,
                        &format!("{name}: {} lookup failed: {e:#}", d.find),
                    )?;
                    Outcome::Failed {
                        count: 1,
                        error: format!("{e:#}"),
                    }
                }
                Ok(Hit::Nothing) => Outcome::Nothing,
                Ok(Hit::Instrumental) => {
                    crate::ui::info(acquire.out, &format!("{name}: an instrumental, by LRCLIB"))?;
                    Outcome::Instrumental
                }
                Ok(Hit::Lyrics(record)) => {
                    let key = SourceKey::Remote {
                        site: LRCLIB.to_string(),
                        id: record.id.to_string(),
                    };
                    let json = serde_json::to_vec_pretty(&record)?;
                    let differs =
                        differs(&dirs.lrclib().join(format!("{}.json", record.id)), &json);
                    if differs {
                        lrclib::keep(&dirs.lrclib(), &record)?;
                        anew.push(key.clone());
                    }
                    again(&key, differs);
                    if !manifest.songs[d.song].has(&key) {
                        let timed = if record.synced_lyrics.is_some() {
                            "timed"
                        } else {
                            "untimed"
                        };
                        crate::ui::info(
                            acquire.out,
                            &format!("{name}: {timed} lyrics from LRCLIB, {key}"),
                        )?;
                        join(&mut manifest, &mut acquire.known, &d.from, &key);
                        joined += 1;
                    }
                    Outcome::Found(key)
                }
                Ok(Hit::Tags(record)) => {
                    let key = SourceKey::Remote {
                        site: MUSICBRAINZ.to_string(),
                        id: record.id.to_string(),
                    };
                    let json = serde_json::to_vec_pretty(&record)?;
                    let differs = differs(
                        &musicbrainz::path_of(&dirs.musicbrainz(), &record.id),
                        &json,
                    );
                    if differs {
                        musicbrainz::keep(&dirs.musicbrainz(), &record)?;
                        anew.push(key.clone());
                    }
                    again(&key, differs);
                    if !manifest.songs[d.song].has(&key) {
                        let on = record
                            .release
                            .as_ref()
                            .map_or_else(String::new, |r| format!(", on {}", r.title));
                        let by = if d.find == Provider::AcoustId {
                            " by its fingerprint on AcoustID"
                        } else {
                            ""
                        };
                        crate::ui::info(
                            acquire.out,
                            &format!("{name}: tags from MusicBrainz{by}, {key}{on}"),
                        )?;
                        join(&mut manifest, &mut acquire.known, &d.from, &key);
                        joined += 1;
                    }
                    Outcome::Found(key)
                }
                Ok(Hit::Cover(cover)) => {
                    let key = SourceKey::Remote {
                        site: COVERART.to_string(),
                        id: cover.id().to_string(),
                    };
                    let folder = dirs.kept(COVERART);
                    let differs = differs(&coverart::path_of(&folder, &cover), cover.bytes());
                    if differs {
                        coverart::keep(&folder, &cover)?;
                        anew.push(key.clone());
                    }
                    again(&key, differs);
                    if !manifest.songs[d.song].has(&key) {
                        crate::ui::info(
                            acquire.out,
                            &format!("{name}: a cover from the Cover Art Archive, {key}"),
                        )?;
                        join(&mut manifest, &mut acquire.known, &d.from, &key);
                        joined += 1;
                    }
                    Outcome::Found(key)
                }
                Ok(Hit::Track(entry, track, alignment)) => {
                    let key = SourceKey::youtube(&track.id);
                    if acquire.known.contains(&key) {
                        Outcome::Found(key)
                    } else {
                        crate::ui::info(
                            acquire.out,
                            &format!(
                                "{name}: its YouTube Music track is {}",
                                watch_url(&track.id)
                            ),
                        )?;
                        if !store.has(&key) && !acquire.fetch_one(&key)? {
                            ok = false;
                            crate::ui::warning(
                                acquire.out,
                                "  the track failed; keeping the upload",
                            )?;
                            Outcome::Failed {
                                count: 1,
                                error: "the track did not arrive".into(),
                            }
                        } else {
                            let fits = alignment
                                .map_or_else(|| music::same_length(&entry, &track), |a| a.fits());
                            if fits && entry.has_subtitles() != Some(false) {
                                join(&mut manifest, &mut acquire.known, &d.from, &key);
                                joined += 1;
                            } else {
                                crate::ui::info(acquire.out, "  kept in place of the upload")?;
                                manifest.edit(Edit::Rename {
                                    from: d.from.clone(),
                                    to: key.clone(),
                                });
                                replaced.push((d.from.clone(), key.clone()));
                                acquire.known.insert(key.clone());
                                joined += 1;
                            }
                            Outcome::Found(key)
                        }
                    }
                }
                Ok(Hit::Upload(upload, alignment)) => {
                    let key = SourceKey::youtube(&upload.id);
                    if acquire.known.contains(&key) {
                        Outcome::Found(key)
                    } else {
                        crate::ui::info(
                            acquire.out,
                            &format!(
                                "{name}: {} is the same recording, {} ms apart; keeping it for its lyrics",
                                watch_url(&upload.id),
                                alignment.offset_ms
                            ),
                        )?;
                        if acquire.fetch_one(&key)? {
                            join(&mut manifest, &mut acquire.known, &d.from, &key);
                            joined += 1;
                            Outcome::Found(key)
                        } else {
                            ok = false;
                            crate::ui::warning(
                                acquire.out,
                                "  the upload failed; no lyrics for now",
                            )?;
                            Outcome::Failed {
                                count: 1,
                                error: "the upload did not arrive".into(),
                            }
                        }
                    }
                }
            };
            let mut looked = Looked::now(d.from.clone(), d.find, outcome);
            looked.unchanged = unchanged;
            records.push(looked);
        }
        for key in anew {
            manifest.edit(Edit::Unhold(key));
        }
        manifest.save()?;
        State::keep_lookups(home, &records, &replaced)?;
        let uploads: Vec<SourceKey> = replaced.iter().map(|(upload, _)| upload.clone()).collect();
        for file in store.discard(&uploads, &manifest.keys())? {
            crate::ui::info(
                acquire.out,
                &format!(
                    "Deleted an upload its release took the place of: {}",
                    file.display()
                ),
            )?;
        }
        for (p, (n, why)) in put_off {
            this_run.refused(p);
            crate::ui::warning(
                acquire.out,
                &format!("{why}: {n} lookup(s) on {p} wait for the next run"),
            )?;
        }
        if joined == 0 && !printed {
            break;
        }
        ask.rematch = false;
    }
    for (p, n) in this_run.waiting {
        crate::ui::info(
            acquire.out,
            &format!("{n} more on {p} wait for the next run, past `per_run`"),
        )?;
    }
    Ok(ok)
}

/// Whether the file at `path` holds other than `bytes`, or is missing: a
/// record found again that changed since it was kept.
fn differs(path: &Path, bytes: &[u8]) -> bool {
    std::fs::read(path).map_or(true, |kept| kept != bytes)
}

/// What [`refetch`] made of the records gone from the store.
#[derive(Debug, Default)]
pub struct Refetched {
    /// Each record that could not be fetched again, with why.
    pub failed: Vec<(SourceKey, String)>,
    /// How many records wait for the next run, their service refusing or
    /// unreachable, by why.
    pub put_off: BTreeMap<String, usize>,
    /// How many records were not asked for, as their fetch failed lately.
    pub waiting: usize,
    /// How many records were not asked for, as their provider is off.
    pub off: usize,
}

/// Fetch again every listed LRCLIB, MusicBrainz or Cover Art Archive
/// record gone from the store, by its ID, asking each at the pace of
/// `throttles`; one whose fetch `failures` records waits as a fetch
/// does, unless `retry`, and one whose provider is off is not asked for.
pub fn refetch(
    dirs: &Dirs,
    manifest: &Manifest,
    http: &(dyn HttpTransport + Sync),
    throttles: &Throttles,
    failures: &BTreeMap<SourceKey, state::Failure>,
    retry: bool,
) -> Result<Refetched> {
    let store = Store::scan(dirs)?;
    let bases = (
        base(manifest, Provider::Lrclib),
        base(manifest, Provider::MusicBrainz),
    );
    let lrclib = lrclib::Client {
        base: &bases.0,
        transport: http,
        throttle: &throttles.lrclib,
    };
    let musicbrainz = musicbrainz::Client {
        base: &bases.1,
        transport: http,
        throttle: &throttles.musicbrainz,
        releases: &throttles.releases,
    };
    let coverart_base = base(manifest, Provider::CoverArt);
    let covers = coverart::Client {
        base: &coverart_base,
        transport: http,
        throttle: &throttles.coverart,
        musicbrainz: manifest
            .providers
            .on(Provider::MusicBrainz)
            .then_some(&musicbrainz),
    };
    let state = State::load(&dirs.home)?;
    // The album the record named when kept, so it is fetched on the
    // same release.
    let album_of_record = |key: &SourceKey| {
        state
            .facts
            .get(key)
            .and_then(|f| f.tags.get(&Field::Album))
            .and_then(|o| o.values.first().cloned())
    };
    let now = state::now_secs();
    let mut refetched = Refetched::default();
    for key in manifest.keys() {
        let SourceKey::Remote { site, id } = &key else {
            continue;
        };
        if store.has(&key) || ![LRCLIB, MUSICBRAINZ, COVERART].contains(&site.as_str()) {
            continue;
        }
        if Provider::of(&key, None).is_some_and(|p| !manifest.providers.on(p)) {
            refetched.off += 1;
            continue;
        }
        let failed_lately = failures
            .get(&key)
            .is_some_and(|f| f.step == state::Step::Fetch && !f.due(None, now));
        if failed_lately && !retry {
            refetched.waiting += 1;
            continue;
        }
        let kept = match site.as_str() {
            LRCLIB => id
                .parse::<u64>()
                .context("not a record ID")
                .and_then(|id| lrclib.by_id(id))
                .and_then(|r| r.map(|r| lrclib::keep(&dirs.lrclib(), &r)).transpose()),
            MUSICBRAINZ => Mbid::parse(id)
                .context("not a record ID")
                .and_then(|id| musicbrainz.by_id(&id, album_of_record(&key).as_deref()))
                .and_then(|r| {
                    r.map(|r| musicbrainz::keep(&dirs.musicbrainz(), &r))
                        .transpose()
                }),
            COVERART => Mbid::parse(id)
                .context("not a record ID")
                .and_then(|id| covers.by_id(&id))
                .and_then(|c| {
                    c.map(|c| coverart::keep(&dirs.kept(COVERART), &c))
                        .transpose()
                }),
            _ => continue,
        };
        match kept {
            Ok(Some(_)) => {}
            Ok(None) => refetched
                .failed
                .push((key.clone(), "no such record".to_string())),
            Err(e) => match http::put_off(&e) {
                Some(why) => *refetched.put_off.entry(why.to_string()).or_default() += 1,
                None => refetched.failed.push((key.clone(), format!("{e:#}"))),
            },
        }
    }
    Ok(refetched)
}

#[cfg(test)]
mod tests;
