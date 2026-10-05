//! A song looking other sources up: each trigger whose condition holds
//! for a song with a source from one of its providers, and none from the
//! one it finds, is a lookup, made unless one made the same way found
//! something, or found nothing lately. What a lookup finds joins the
//! song, and may trigger lookups of its own, for a few rounds.
//!
//! Each lookup is recorded in the state by the source it was made from,
//! the provider looked up and that provider's `method`, with when and
//! what came of it. It is made again:
//!
//! | Recorded | Made again |
//! |---|---|
//! | Nothing, or another `method` | At once |
//! | Found nothing | After the provider's `recheck_days` |
//! | Failed | After an hour, doubling with each failure, up to a week |
//! | Found a source, an instrumental, or declined | Never |
//!
//! A song makes at most one lookup per provider a run, so a release and
//! its upload never look each other up in turn. Each round measures what
//! the triggers read, makes its lookups and fetches what they found,
//! without holding the run's lock; what one round adds may trigger the
//! next. A provider's `per_run` cap lets a backlog, as the first sync of
//! a library never looked up has, drain over several runs rather than all
//! at once on a free service.
//!
//! A YouTube Music track found later is fetched beside its upload and
//! joins the song; the upload stays a source when the two are one
//! recording, else the track takes its key. An LRCLIB or MusicBrainz
//! record found is kept in the store and joins the song. A source the run
//! already knows, listed, removed or replaced, is recorded found but never
//! added again.
//!
//! LRCLIB and MusicBrainz are asked for the song as it resolves: its
//! title, first artist, album and length; a song lacking any but the
//! album asks neither.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::Path;

use crate::http::{HttpTransport, Throttle};
use anyhow::{Context, Result, anyhow};

use crate::acquire::{self, Acquire};
use crate::align::Alignment;
use crate::dirs::Dirs;
use crate::download;
use crate::facts::Facts;
use crate::lrclib::{self, Found, Query, Record};
use crate::manifest::{Edit, Manifest};
use crate::music::{self, Entry};
use crate::musicbrainz;
use crate::parallel;
use crate::provider::{LRCLIB, MUSICBRAINZ, Provider, When};
use crate::reconcile::{self, Measuring, Planned};
use crate::resolve::{self, Resolved};
use crate::runner::Runner;
use crate::source::{SourceKey, watch_url};
use crate::state::{self, Looked, Outcome, State};
use crate::store::{Located, Store};
use crate::tags::Field;

/// Rounds of lookups one run makes: what one round finds triggers the
/// next, as an upload's release then finds the release's lyrics.
pub const ROUNDS: usize = 3;

/// The providers looked up, in the order a round makes their lookups.
pub const ORDER: [Provider; 4] = [
    Provider::YouTubeMusic,
    Provider::YouTube,
    Provider::Lrclib,
    Provider::MusicBrainz,
];

/// How a provider's lookups are made; a lookup made another way is due
/// again.
#[must_use]
pub fn method(p: Provider) -> &'static str {
    match p {
        Provider::YouTubeMusic => "youtube-music/1",
        Provider::YouTube => "youtube/1",
        Provider::Lrclib => "lrclib/1",
        Provider::MusicBrainz => "musicbrainz/1",
        Provider::Manual => "manual",
    }
}

/// A lookup due: the song, the source it is made from, and what it finds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub song: usize,
    pub from: SourceKey,
    pub find: Provider,
}

/// Whether a song resolves to lyrics, and to timed ones.
fn lyrics_of(r: Option<&Resolved>, facts: &BTreeMap<SourceKey, Facts>) -> (bool, bool) {
    let Some(l) = r.and_then(|r| r.plan.lyrics.as_ref()) else {
        return (false, false);
    };
    let timed = facts
        .get(&l.key)
        .and_then(|f| f.lyrics.iter().find(|x| x.at == l.at))
        .is_some_and(|x| x.timing.is_some());
    (true, timed)
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
/// for its title, which no source names.
#[must_use]
pub fn query_of(r: &Resolved, facts: &BTreeMap<SourceKey, Facts>) -> Option<Query> {
    let tag = |f: Field| {
        r.plan
            .tags
            .iter()
            .find(|(k, _)| k == f.vorbis())
            .and_then(|(_, v)| v.first().cloned())
    };
    Some(Query {
        title: tag(Field::Title)?,
        artist: tag(Field::Artist)?,
        album: tag(Field::Album).filter(|_| album_of(Some(r))),
        seconds: facts.get(&r.plan.audio.key)?.duration?,
    })
}

/// Every lookup due, a song's for each provider at most once; with
/// `force`, whatever was found or not before.
#[must_use]
pub fn due(
    manifest: &Manifest,
    state: &State,
    planned: &Planned,
    now: u64,
    force: bool,
) -> Vec<Due> {
    let config = &manifest.providers;
    let resolved: HashMap<usize, &Resolved> = planned.iter().map(|(n, r)| (*n, r)).collect();
    let mut found = Vec::new();
    for (n, song) in manifest.songs.iter().enumerate() {
        let kinds: Vec<(&SourceKey, Provider)> = song
            .sources
            .iter()
            .filter_map(|k| Some((k, Provider::of(k, state.facts.get(k))?)))
            .collect();
        let r = resolved.get(&n).copied();
        let (has, timed) = lyrics_of(r, &state.facts);
        let album = album_of(r);
        let mut taken = BTreeSet::new();
        for t in config.active() {
            if kinds.iter().any(|(_, p)| *p == t.find) || taken.contains(&t.find) {
                continue;
            }
            let wanted = match t.when {
                When::Always => true,
                When::NoLyrics => !has,
                When::NoTimedLyrics => !timed,
                When::NoAlbum => !album,
            };
            let Some((from, _)) = kinds.iter().find(|(_, p)| t.from.contains(p)) else {
                continue;
            };
            let askable = !t.find.kept() || r.and_then(|r| query_of(r, &state.facts)).is_some();
            let days = config.settings(t.find).recheck_days;
            let open = force || state.looked(from, t.find).is_none_or(|l| l.due(now, days));
            if wanted && askable && open {
                taken.insert(t.find);
                found.push(Due {
                    song: n,
                    from: (*from).clone(),
                    find: t.find,
                });
            }
        }
    }
    found
}

/// What one lookup found; short-lived, so its size matters little.
#[allow(clippy::large_enum_variant)]
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
    Nothing,
}

fn entry_of<R: Runner>(runner: &R, key: &SourceKey) -> Result<Entry> {
    let id = key.id().context("not a video")?;
    music::entries(&runner.output(&music::video_command(id))?)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("yt-dlp listed nothing for {key}"))
}

/// The services a run asks, at the addresses the song list sets.
struct Clients<'a> {
    lrclib: lrclib::Client<'a>,
    musicbrainz: musicbrainz::Client<'a>,
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

fn look<R: Runner>(
    runner: &R,
    d: &Due,
    located: Option<&Located>,
    query: Option<&Query>,
    clients: &Clients<'_>,
    audio: &Path,
) -> Result<Hit> {
    let query = || query.context("the song has no title, artist or length");
    Ok(match d.find {
        Provider::YouTubeMusic => {
            let entry = entry_of(runner, &d.from)?;
            match acquire::lookup(runner, &entry, audio)? {
                Some((track, a)) => Hit::Track(entry, track, a),
                None => Hit::Nothing,
            }
        }
        Provider::YouTube => {
            let entry = entry_of(runner, &d.from)?;
            let path = &located.context("its file is not in the store")?.path;
            let own = acquire::local_envelope(runner, path)?;
            match acquire::reverse_lookup(runner, &entry, &own, audio)? {
                Some((upload, a)) => Hit::Upload(upload, a),
                None => Hit::Nothing,
            }
        }
        Provider::Lrclib => match clients.lrclib.find(query()?)? {
            Found::Lyrics(r) => Hit::Lyrics(Box::new(r)),
            Found::Instrumental(_) => Hit::Instrumental,
            Found::Nothing => Hit::Nothing,
        },
        Provider::MusicBrainz => match clients.musicbrainz.find(query()?)? {
            Some(r) => Hit::Tags(Box::new(r)),
            None => Hit::Nothing,
        },
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

/// The yt-dlp template that fetches beside the file of `from`.
fn template_beside(store: &Store, from: &SourceKey) -> String {
    let folder = store
        .locate(from)
        .and_then(|l| {
            l.path
                .parent()
                .and_then(Path::file_name)
                .map(|f| f.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "unknown".to_string());
    download::template_in(&folder)
}

/// Make every lookup due, joining what is found to its song, for up to
/// [`ROUNDS`] rounds, asking MusicBrainz at the pace of `throttle`.
/// `force` makes the first round's lookups whatever was found before; a
/// YouTube Music lookup from a key in `declined` is recorded declined
/// instead. Returns whether every fetch succeeded.
#[allow(clippy::too_many_lines)]
pub fn run<R: Runner, W: Write>(
    acquire: &mut Acquire<'_, R, W>,
    dirs: &Dirs,
    http: &(dyn HttpTransport + Sync),
    throttle: &Throttle,
    force: bool,
    declined: &BTreeSet<SourceKey>,
) -> Result<bool> {
    let home = &dirs.home;
    let runner = acquire.runner;
    let mut ok = true;
    let mut force = force;
    let mut made: BTreeMap<Provider, usize> = BTreeMap::new();
    let mut held = BTreeMap::new();
    for round in 0..ROUNDS {
        let mut manifest = Manifest::load(home)?;
        let mut state = State::load(home)?;
        let store = Store::scan(dirs)?;
        let scratch = acquire.temp().join(format!("lookup-{round}"));
        let mut how = Measuring {
            retry: false,
            checkpoint: &mut |s: &State| State::keep_measures(home, s),
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
        let (planned, _) = reconcile::plan(&manifest, &state, &dirs.library, &mut std::io::sink())?;
        let (refused, due): (Vec<Due>, Vec<Due>) =
            due(&manifest, &state, &planned, state::now_secs(), force)
                .into_iter()
                .partition(|d| d.find == Provider::YouTubeMusic && declined.contains(&d.from));
        let mut records: Vec<Looked> = refused
            .into_iter()
            .map(|d| Looked::now(d.from, d.find, Outcome::Declined))
            .collect();
        let mut waiting = BTreeMap::new();
        let due: Vec<Due> = due
            .into_iter()
            .filter(|d| {
                let limit = manifest.providers.settings(d.find).per_run;
                let n = made.entry(d.find).or_insert(0);
                if limit > 0 && *n >= limit {
                    *waiting.entry(d.find).or_insert(0_usize) += 1;
                    return false;
                }
                *n += 1;
                true
            })
            .collect();
        held = waiting;
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
        );
        let clients = Clients {
            lrclib: lrclib::Client {
                base: &bases.0,
                transport: http,
            },
            musicbrainz: musicbrainz::Client {
                base: &bases.1,
                transport: http,
                throttle,
            },
        };
        let audio = acquire.temp().join("audio");
        let resolved: HashMap<usize, &Resolved> = planned.iter().map(|(n, r)| (*n, r)).collect();
        let mut hits: Vec<(Due, Result<Hit>)> = Vec::new();
        for p in ORDER {
            let group: Vec<&Due> = due.iter().filter(|d| d.find == p).collect();
            if group.is_empty() {
                continue;
            }
            let workers = manifest.providers.settings(p).concurrency;
            let found = parallel::map(&group, workers, |d| {
                let query = resolved
                    .get(&d.song)
                    .and_then(|r| query_of(r, &state.facts));
                look(
                    runner,
                    d,
                    store.locate(&d.from).as_ref(),
                    query.as_ref(),
                    &clients,
                    &audio,
                )
            });
            hits.extend(group.into_iter().cloned().zip(found));
        }

        let mut replaced = Vec::new();
        let mut joined = 0_usize;
        for (d, hit) in hits {
            let name = reconcile::name_of(
                &manifest.songs[d.song],
                resolved.get(&d.song).copied(),
                &state.facts,
            );
            let outcome = match hit {
                // No yt-dlp fails every lookup alike; the run stops and
                // says so, rather than putting each song off for a week.
                Err(e) if e.downcast_ref::<crate::runner::MissingTool>().is_some() => {
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
                        extractor: LRCLIB.to_string(),
                        id: record.id.to_string(),
                    };
                    if !acquire.known.contains(&key) {
                        lrclib::keep(&dirs.lrclib(), &record)?;
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
                        extractor: MUSICBRAINZ.to_string(),
                        id: record.id.clone(),
                    };
                    if !acquire.known.contains(&key) {
                        musicbrainz::keep(&dirs.musicbrainz(), &record)?;
                        let on = record
                            .release
                            .as_ref()
                            .map_or_else(String::new, |r| format!(", on {}", r.title));
                        crate::ui::info(
                            acquire.out,
                            &format!("{name}: tags from MusicBrainz, {key}{on}"),
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
                        let template = template_beside(&store, &d.from);
                        if !store.has(&key) && !acquire.fetch_one(&template, &key)? {
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
                        let template = template_beside(&store, &d.from);
                        if acquire.fetch_one(&template, &key)? {
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
            records.push(Looked::now(d.from.clone(), d.find, outcome));
        }
        manifest.save()?;
        State::keep_lookups(home, &records, &replaced)?;
        if joined == 0 {
            break;
        }
        force = false;
    }
    for (p, n) in held {
        crate::ui::info(
            acquire.out,
            &format!("{n} more on {p} wait for the next run, past `per_run`"),
        )?;
    }
    Ok(ok)
}

/// Fetch again every listed LRCLIB or MusicBrainz record gone from the
/// store, by its ID, asking MusicBrainz at the pace of `throttle`.
/// Returns each that could not be, with why.
pub fn refetch(
    dirs: &Dirs,
    manifest: &Manifest,
    http: &(dyn HttpTransport + Sync),
    throttle: &Throttle,
) -> Result<Vec<(SourceKey, String)>> {
    let store = Store::scan(dirs)?;
    let bases = (
        base(manifest, Provider::Lrclib),
        base(manifest, Provider::MusicBrainz),
    );
    let lrclib = lrclib::Client {
        base: &bases.0,
        transport: http,
    };
    let musicbrainz = musicbrainz::Client {
        base: &bases.1,
        transport: http,
        throttle,
    };
    let mut failed = Vec::new();
    for key in manifest.keys() {
        let SourceKey::Remote { extractor, id } = &key else {
            continue;
        };
        if store.has(&key) {
            continue;
        }
        let kept = match extractor.as_str() {
            LRCLIB => id
                .parse::<u64>()
                .context("not a record ID")
                .and_then(|id| lrclib.by_id(id))
                .and_then(|r| r.map(|r| lrclib::keep(&dirs.lrclib(), &r)).transpose()),
            MUSICBRAINZ => musicbrainz.by_id(id).and_then(|r| {
                r.map(|r| musicbrainz::keep(&dirs.musicbrainz(), &r))
                    .transpose()
            }),
            _ => continue,
        };
        match kept {
            Ok(Some(_)) => {}
            Ok(None) => failed.push((key.clone(), "no such record".to_string())),
            Err(e) => failed.push((key.clone(), format!("{e:#}"))),
        }
    }
    Ok(failed)
}

#[cfg(test)]
mod tests;
