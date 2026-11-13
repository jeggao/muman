//! Sources whose keys change before a sync plans: keys renamed by the
//! site they came from, and manual files followed to where they moved.
//!
//! A key a site renames keeps its fetched file through the rename: the
//! file takes a second name at the new key's place, then the state and
//! then the song list name the new key, and only then does the old name
//! go. A run stopped between any two finds the file under the key the
//! list names, so it is not fetched again; stopped after the state, it
//! measures that one source again, and after the list, it leaves the
//! old name, which `purge` lists. A filesystem that makes no second
//! name, as FAT, moves the file after the state instead.
//!
//! A manual file a song or a removed song names, gone from its path, is
//! followed by every run that brings the library in line, so no command
//! fails its song for a file the user only moved, a removed song's file
//! moved stays out rather than come back as a new song. The run records
//! the moves it follows, and its undo puts the song list back with them
//! followed, as the state and the user's file stay ([`crate::history`]);
//! the undo of a run an earlier muman recorded puts back the keys before,
//! and the sync after has them followed again. Song files, `.lrc` files,
//! pictures and tag files alike.
//!
//! A file is known by what it held, never by its name alone: the name
//! is what a move changes, and two files that trade names, as a tagger
//! fixing two track numbers leaves them, each hold every byte the other
//! held. What a named file held is what measuring last read of it, its
//! size and modification time, which a move keeps, and the digest of its
//! bytes, and what its song, or removed song, records it held, which the
//! undo of an earlier muman's run puts back with the song list while the
//! state keeps the file's record under the key it moved to: for a song
//! file the digest of its audio's packets, which no retag touches, so a
//! file a tagger renamed as it retagged it is still found; for a lyrics
//! file, a picture or a tag file its bytes. One recording in two files
//! of another encoding is two audios.
//!
//! The files gone are matched to those that may be them: the files no
//! song or removed song names, and the named files whose size or time
//! changed, which may be another named file moved over them. The match
//! goes by the strongest evidence first, for all of them at once: the
//! size and time, then the bytes, then the audio. Of the files arrived
//! too lately to be read yet ([`Unlisted`]) only those holding every
//! byte of a gone one are taken: a rename places a file anew, so it
//! looks as freshly arrived as a copy under way, but a copy under way
//! does not yet hold every byte. The digests need nothing beyond what
//! measuring keeps, on every system, where a file's ID would have to be
//! recorded before it went.
//!
//! A file gone that more than one file could be, or a file more than one
//! gone file could be, is not guessed at where the guess decides what a
//! song is made from: it is named, and those files are not listed as new
//! songs either. Its song is held as built, as one waiting for a file
//! copied in, its library file kept, though the state, which an undo
//! leaves as it is, names that file's sources by the files it cannot
//! tell; the run fails until one such file is left. Where the guess
//! decides nothing it is made: keys gone that one song, or one removed
//! song, alone names among its sources, pinned nowhere and recorded as
//! holding the same, each of which as many files could be, as two
//! copies of one file moved together, are matched to those files in the
//! order of their names, since any order makes one song of one set of
//! files and records. The undo of an earlier muman's run, which puts back
//! the keys but not the state's stamps that told them apart, finds such
//! copies so.
//!
//! A named file whose own content moved elsewhere is gone too, but only
//! once another named file is matched to its path: one that holds other
//! audio and that no gone file is, a file of the user's own edited in
//! place, is followed as changed, its song with it. So two files that
//! traded names, or any longer round of them, are each followed to the
//! other's name, each song keeping its own tags, pins and offsets, and a
//! file moved over another named one takes its song there while the
//! song of the file it replaced, whose content is nowhere, names the key
//! the moved file left until its file is back: no two songs name one
//! file, and the file put back is followed as any other. All the moves
//! are written at once, so a key one move frees and another takes is
//! never named twice.
//!
//! A named file is changed, too, when what measuring read at its name,
//! as it is now, is other than what its record names it held: the undo
//! of an earlier muman's run puts back the song list's keys and records,
//! but the state keeps each file's record under the name it moved to, so
//! after a swap was followed each name holds what the other's record
//! names, though no stamp changed. Such files are matched as any changed
//! one, and the state, which names each where it is already, is not
//! moved again ([`Moves::drifted`]).
//!
//! A removed song keeps out what it held, by the same rule, not a name:
//! a song file only a removed song names, settled and not as measuring
//! last read it, whose audio is other than the song recorded, is a new
//! file the user put at that name, listed as any dropped in
//! ([`Moves::anew`]); one retagged in place holds the same audio and
//! stays out. Its audio unknown, as to `check`, which hears nothing, it
//! stays out too. A copy of such a file dropped in under another name,
//! the file still at its own, holds what the removed song held, its bytes
//! or its audio, and joins that removed song's sources, kept out with it
//! as a copy of a listed song joins that song ([`Moves::copies`]), and
//! listed with it again by `restore`; other audio, another encoding of
//! the recording among it, is a new file, as at its name. Only the sync
//! that lists the files dropped in keeps it out.
//!
//! A song file of the user's that is followed nowhere, while another
//! song file of the user's its song lists is still there and nothing is
//! arriving, was deleted by hand ([`deleted`]), as encoding a file anew
//! and deleting the old one leaves it: `sync` takes it out of its song,
//! as it keeps out a song taken out of the list by hand, saying so, and
//! `undo` puts it back. Its song would otherwise be made from the rest
//! with `check` naming the file gone for good. A file gone with no other
//! of the user's left beside it is not taken out: its folder may be
//! moved away for a while, and its song waits for it, failing, its file
//! kept; one beside a fetched source only is named by `check` with how
//! to take it out by hand.

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Result;

use crate::held::{Aspect, Held};
use crate::manifest::{Edit, Manifest};
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::State;
use crate::store::{self, Kind, Located, Store};

/// The manual files neither a song nor a removed song lists, as
/// [`Store::unlisted`] splits them, but that a file once listed is no new
/// one still copying in, however lately it was touched.
#[derive(Debug, Clone, Default)]
pub struct Unlisted {
    /// Those settled, read as they are.
    pub ready: Vec<SourceKey>,
    /// Those arrived too lately to be read yet.
    pub settling: Vec<PathBuf>,
    /// The song files only a removed song names, settled, that are not as
    /// measuring last read them: each may hold other audio than the song
    /// held, a new file at its name, which [`moved`] tells.
    pub tombed: Vec<SourceKey>,
}

impl Unlisted {
    #[must_use]
    pub fn of(manifest: &Manifest, state: &State, store: &Store, wait: Duration) -> Self {
        let removed = manifest.removed_keys();
        let (ready, mut settling) = store.unlisted(&manifest.keys(), SystemTime::now(), wait);
        let (tombed, mut ready): (Vec<SourceKey>, Vec<SourceKey>) =
            ready.into_iter().partition(|k| removed.contains(k));
        let tombed = (tombed.into_iter())
            .filter(|k| {
                let was = state.facts.get(k).and_then(|f| store::stamp_of_rev(&f.rev));
                let now = store.locate(k).and_then(|l| store::stamp(&l.path));
                was.is_some() && was != now
            })
            .collect();
        settling.retain(|path| !removed.contains(&SourceKey::Manual(path.into())));
        settling.retain(|path| {
            let key = SourceKey::Manual(path.into());
            let was_listed = state.listed.contains(&key);
            if was_listed {
                ready.push(key);
            }
            !was_listed
        });
        Self {
            ready,
            settling,
            tombed,
        }
    }
}

/// The manual files the song list names that [`moved`] found elsewhere.
#[derive(Debug, Clone, Default)]
pub struct Moves {
    /// Each key, by the key of the file that is it now.
    pub moved: BTreeMap<SourceKey, SourceKey>,
    /// Of those, each whose file another named file was moved over, by
    /// that other: its song names the key the other left until its own
    /// file is back.
    pub over: BTreeMap<SourceKey, SourceKey>,
    /// Keys gone, each set beside the files that may be them, of which
    /// none can be told to be one; no file of these is followed or listed.
    pub unsure: Vec<(BTreeSet<SourceKey>, BTreeSet<SourceKey>)>,
    /// Keys only a removed song names whose file holds other audio than
    /// the song held: new files, listed as any dropped in.
    pub anew: Vec<SourceKey>,
    /// Keys moved whose record the state already keeps where the file
    /// is, as after an undo: the state follows none of them.
    pub drifted: BTreeSet<SourceKey>,
    /// Files no song names that hold what a song file only a removed song
    /// names, still at its name, holds, each by that file's key: kept out
    /// with its removed song.
    pub copies: BTreeMap<SourceKey, SourceKey>,
}

impl Moves {
    /// The moves the state follows: those not [`Moves::drifted`].
    #[must_use]
    pub fn of_state(&self) -> BTreeMap<SourceKey, SourceKey> {
        (self.moved.iter())
            .filter(|(from, _)| !self.drifted.contains(*from))
            .map(|(from, to)| (from.clone(), to.clone()))
            .collect()
    }

    /// What is said of each file of [`Moves::anew`]; `ahead` of the sync
    /// that lists it.
    pub fn anew_lines(&self, ahead: bool) -> impl Iterator<Item = String> + '_ {
        let what = if ahead {
            "listed by the next sync"
        } else {
            "so listed"
        };
        (self.anew.iter()).map(move |key| {
            format!("Other audio than its removed song held, {what} as a new file: {key}")
        })
    }

    /// What is said of each file of [`Moves::copies`]; `ahead` of the
    /// sync that keeps it out.
    pub fn copy_lines(&self, ahead: bool) -> impl Iterator<Item = String> + '_ {
        (self.copies.iter()).map(move |(copy, of)| {
            if ahead {
                format!("Holds what removed {of} held, kept out with it by the next sync: {copy}")
            } else {
                format!(
                    "Holds what removed {of} held, so kept out with it: {copy}; `muman restore` \
                     lists it again"
                )
            }
        })
    }

    /// What following `from` says; `ahead` of the sync that follows it.
    #[must_use]
    pub fn line(&self, from: &SourceKey, ahead: bool) -> Option<String> {
        let to = self.moved.get(from)?;
        let when = if ahead {
            ", followed by the next sync"
        } else {
            ""
        };
        Some(match self.over.get(from) {
            Some(by) => format!(
                "Replaced by {by}, moved over it{when}: {from}; its song waits for its own \
                 file as {to}"
            ),
            None => format!("Moved{when}: {from} is now {to}"),
        })
    }

    /// What is said of each file not followed for being unsure.
    pub fn unsure_lines(&self) -> impl Iterator<Item = String> + '_ {
        self.unsure.iter().map(|(gone, files)| {
            let names = |keys: &BTreeSet<SourceKey>| {
                keys.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "Not followed, as which file is which cannot be told: {} held what {} hold; \
                 leave one such file in sources/manual",
                names(gone),
                names(files)
            )
        })
    }
}

/// The evidence two files are one, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum By {
    /// Size and modification time.
    Stamp,
    /// Every byte.
    Bytes,
    /// The audio's packets.
    Audio,
}

/// What a named file held, as the module says.
struct Was {
    kind: Kind,
    stamp: Option<(u64, u128)>,
    bytes: Option<String>,
    /// The digest its record names of the part that is the file: its
    /// audio, or its bytes for a file that is one part whole.
    part: Option<String>,
}

impl Was {
    /// What `key` held; what measuring read at its name is left out when
    /// that is `drifted` from what its record names.
    fn of(manifest: &Manifest, state: &State, key: &SourceKey, kind: Kind, drifted: bool) -> Self {
        let facts = state.facts.get(key);
        let part = (recorded(manifest, key).and_then(|h| h.digest(aspect(kind))))
            .map(str::to_string)
            .or_else(|| now_held(manifest, key, facts?, kind));
        let facts = facts.filter(|_| !drifted);
        Self {
            kind,
            stamp: facts.and_then(|f| store::stamp_of_rev(&f.rev)),
            bytes: facts.and_then(|f| Some(f.content.as_deref()?.split(';').next()?.to_string())),
            part,
        }
    }
}

/// What the song, or removed song, naming `key` records it held.
fn recorded<'m>(manifest: &'m Manifest, key: &SourceKey) -> Option<&'m Held> {
    (manifest.song_with(key).and_then(|s| s.held.get(key)))
        .or_else(|| manifest.removed.iter().find_map(|r| r.held.get(key)))
}

/// The digest of the part of a file of `kind` that `facts` read.
fn now_held(
    manifest: &Manifest,
    key: &SourceKey,
    facts: &crate::facts::Facts,
    kind: Kind,
) -> Option<String> {
    let now = Held::now(key, facts, &manifest.settings.sites)?;
    now.digest(aspect(kind)).map(str::to_string)
}

/// Whether what measuring read at `key`'s name, as it is now, is other
/// than what its record names it held: another named file's, as an undo,
/// which puts back the records but not the state's keys, leaves it.
fn drifted(manifest: &Manifest, state: &State, store: &Store, key: &SourceKey, kind: Kind) -> bool {
    let Some(at) = store.locate(key) else {
        return false;
    };
    let Some(facts) = state.facts.get(key).filter(|f| f.holds_for(&at.rev())) else {
        return false;
    };
    let was = recorded(manifest, key).and_then(|h| h.digest(aspect(kind)));
    was.zip(now_held(manifest, key, facts, kind))
        .is_some_and(|(was, now)| !same(was, &now))
}

/// The part a record holds of a file of `kind`.
fn aspect(kind: Kind) -> Aspect {
    match kind {
        Kind::Media => Aspect::Audio,
        Kind::Lyrics => Aspect::Lyrics,
        Kind::Image => Aspect::Cover,
        Kind::Tags => Aspect::Tags,
    }
}

/// A file that may be a named one, what it holds read once, when asked.
struct File {
    key: SourceKey,
    kind: Kind,
    settling: bool,
    at: Option<Located>,
    stamp: Option<(u64, u128)>,
    bytes: OnceCell<Option<String>>,
    audio: OnceCell<Option<String>>,
}

impl File {
    fn new(store: &Store, key: SourceKey, kind: Kind, settling: bool) -> Self {
        let at = store.locate(&key);
        Self {
            stamp: at.as_ref().and_then(|l| store::stamp(&l.path)),
            key,
            kind,
            settling,
            at,
            bytes: OnceCell::new(),
            audio: OnceCell::new(),
        }
    }

    /// What measuring kept of it at its revision.
    fn facts<'s>(&self, state: &'s State) -> Option<&'s crate::facts::Facts> {
        let at = self.at.as_ref()?;
        (state.facts.get(&self.key)).filter(|f| f.holds_for(&at.rev()))
    }

    fn bytes(&self, state: &State) -> Option<&str> {
        self.bytes
            .get_or_init(|| {
                (self.facts(state))
                    .and_then(|f| Some(f.content.as_deref()?.split(';').next()?.to_string()))
                    .or_else(|| crate::hash::digest_file(&self.at.as_ref()?.path))
            })
            .as_deref()
    }

    fn audio(
        &self,
        state: &State,
        hear: &mut impl FnMut(&Located) -> Option<String>,
    ) -> Option<&str> {
        self.audio
            .get_or_init(|| {
                (self.facts(state))
                    .filter(|f| f.hashed())
                    .and_then(|f| f.audio.as_ref()?.digest.clone())
                    .or_else(|| hear(self.at.as_ref()?))
            })
            .as_deref()
    }

    /// Whether its audio is known to be other than `was` records.
    fn holds_other(
        &self,
        was: &Was,
        state: &State,
        hear: &mut impl FnMut(&Located) -> Option<String>,
    ) -> bool {
        (was.part.as_deref())
            .zip(self.audio(state, hear))
            .is_some_and(|(was, now)| !same(now, was))
    }

    /// Whether it is the file `was` names, by `by`.
    fn is(
        &self,
        was: &Was,
        by: By,
        state: &State,
        hear: &mut impl FnMut(&Located) -> Option<String>,
    ) -> bool {
        if self.kind != was.kind || (self.settling && by != By::Bytes) {
            return false;
        }
        match by {
            By::Stamp => was.stamp.is_some() && self.stamp == was.stamp,
            By::Bytes => {
                let sized = was
                    .stamp
                    .is_none_or(|(size, _)| self.stamp.is_some_and(|(have, _)| have == size));
                let whole = was
                    .bytes
                    .as_deref()
                    .is_some_and(|b| sized && self.bytes(state).is_some_and(|have| have == b));
                let part = was.part.as_deref().is_some_and(|p| {
                    (self.bytes(state))
                        .and_then(|b| crate::held::of_file(aspect(self.kind), b))
                        .is_some_and(|have| same(&have, p))
                });
                whole || part
            }
            By::Audio => was
                .part
                .as_deref()
                .is_some_and(|p| (self.audio(state, hear)).is_some_and(|have| same(have, p))),
        }
    }
}

/// Whether two digests, cut to the song list's length, agree.
fn same(a: &str, b: &str) -> bool {
    let n = crate::held::DIGITS;
    a.len() >= n && b.len() >= n && crate::held::agree(a, b)
}

/// A named file gone, or changed, looking for the file that is it.
struct Seeker<'m> {
    key: SourceKey,
    was: Was,
    /// The file at its own path, for one changed.
    own: Option<usize>,
    role: Option<Role<'m>>,
}

/// What a gone key is to the song list but its name, for a key that one
/// song or one removed song names among its sources and nowhere else:
/// two keys of one role, given each other's files, make what they made.
#[derive(Debug, PartialEq, Eq)]
struct Role<'m> {
    /// The song's place, or the removed song's after every song.
    owner: usize,
    held: Option<&'m Held>,
}

impl<'m> Role<'m> {
    fn of(manifest: &'m Manifest, key: &SourceKey) -> Option<Self> {
        let pinned = |s: &crate::manifest::Song| {
            [&s.album, &s.audio, &s.cover].contains(&&Some(key.clone()))
                || matches!(&s.lyrics, Some(crate::manifest::LyricsPin::From(k)) if k == key)
        };
        let songs = (manifest.songs.iter().enumerate()).filter(|(_, s)| s.has(key) || pinned(s));
        let removed =
            (manifest.removed.iter().enumerate()).filter(|(_, r)| r.sources.contains(key));
        let mut roles = (songs.map(|(n, s)| (!pinned(s)).then(|| (n, s.held.get(key)))))
            .chain(removed.map(|(n, r)| Some((manifest.songs.len() + n, r.held.get(key)))));
        let (owner, held) = roles.next()??;
        roles.next().is_none().then_some(Self { owner, held })
    }
}

/// Each manual file a song or a removed song names that is gone, or that
/// another named file was moved over, by the file that is it now, as the
/// module says; the files of `unlisted` found to be one, or unsure, taken
/// out of it. What a song file holds is read from what measuring kept of
/// it, else by `hear`.
pub fn moved(
    manifest: &Manifest,
    state: &State,
    store: &Store,
    unlisted: &mut Unlisted,
    mut hear: impl FnMut(&Located) -> Option<String>,
) -> Moves {
    let mut named = manifest.keys();
    named.extend(manifest.removed_keys());
    let named: Vec<(SourceKey, Kind)> = (named.into_iter())
        .filter_map(|k| {
            let SourceKey::Manual(m) = &k else {
                return None;
            };
            let kind = store::kind_of(m.path())?;
            Some((k, kind))
        })
        .collect();
    let restamped = |k: &SourceKey| {
        let was = state.facts.get(k).and_then(|f| store::stamp_of_rev(&f.rev));
        let now = store.locate(k).and_then(|l| store::stamp(&l.path));
        was.is_some() && was != now
    };
    let gone: Vec<&(SourceKey, Kind)> = named.iter().filter(|(k, _)| !store.has(k)).collect();
    // Each named file there, changed since measured or drifted from its
    // record, and whether drifted.
    let changed: Vec<(&SourceKey, Kind, bool)> = (named.iter())
        .filter(|(k, _)| store.has(k))
        .map(|(k, kind)| (k, *kind, drifted(manifest, state, store, k, *kind)))
        .filter(|(k, _, drifted)| *drifted || restamped(k))
        .collect();
    let drifted: BTreeSet<SourceKey> = (changed.iter())
        .filter(|(_, _, drifted)| *drifted)
        .map(|(k, _, _)| (*k).clone())
        .collect();
    // With none gone, a file changed alone was changed in place: no other
    // named file can be the one moved over it.
    if gone.is_empty() && changed.len() < 2 && unlisted.tombed.is_empty() {
        return Moves {
            copies: copies(manifest, state, store, unlisted, &mut hear),
            ..Moves::default()
        };
    }
    let named: BTreeSet<&SourceKey> = named.iter().map(|(k, _)| k).collect();
    let mut files: Vec<File> = (unlisted.ready.iter())
        .map(|k| File::new(store, k.clone(), Kind::Media, false))
        .chain(
            (unlisted.settling.iter())
                .map(|rel| File::new(store, SourceKey::Manual(rel.into()), Kind::Media, true)),
        )
        .chain(
            (store.manual_files())
                .filter(|(k, kind)| *kind != Kind::Media && !named.contains(k))
                .map(|(k, kind)| File::new(store, k, kind, false)),
        )
        .collect();
    let mut seekers: Vec<Seeker> = (gone.iter())
        .map(|(k, kind)| Seeker {
            key: k.clone(),
            was: Was::of(manifest, state, k, *kind, false),
            own: None,
            role: Role::of(manifest, k),
        })
        .collect();
    for (k, kind, drifted) in changed {
        files.push(File::new(store, k.clone(), kind, false));
        seekers.push(Seeker {
            key: k.clone(),
            was: Was::of(manifest, state, k, kind, drifted),
            own: Some(files.len() - 1),
            role: None,
        });
    }
    let (pairs, unsure) = pair(&seekers, &files, state, &mut hear);
    let taken: BTreeMap<usize, usize> = pairs.iter().map(|(s, f)| (*f, *s)).collect();
    let mut moves = Moves::default();
    for (s, f) in &pairs {
        moves
            .moved
            .insert(seekers[*s].key.clone(), files[*f].key.clone());
    }
    moved_over(&seekers, &pairs, &taken, &mut moves);
    moves.drifted = (drifted.into_iter())
        .filter(|k| moves.moved.contains_key(k))
        .collect();
    let mut out: BTreeSet<&SourceKey> = pairs.values().map(|f| &files[*f].key).collect();
    for (gone, these) in &unsure {
        out.extend(these.iter().map(|f| &files[*f].key));
        moves.unsure.push((
            gone.iter().map(|s| seekers[*s].key.clone()).collect(),
            these.iter().map(|f| files[*f].key.clone()).collect(),
        ));
    }
    moves.anew = (seekers.iter())
        .filter(|s| unlisted.tombed.contains(&s.key) && !out.contains(&s.key))
        .filter(|s| s.own.is_some_and(|f| !taken.contains_key(&f)))
        .filter(|s| {
            s.own
                .is_some_and(|f| files[f].holds_other(&s.was, state, &mut hear))
        })
        .map(|s| s.key.clone())
        .collect();
    unlisted.ready.extend(moves.anew.iter().cloned());
    unlisted.ready.retain(|k| !out.contains(k));
    unlisted
        .settling
        .retain(|rel| !out.contains(&SourceKey::Manual(rel.into())));
    moves.copies = copies(manifest, state, store, unlisted, &mut hear);
    moves
}

/// Each settled file of `unlisted` holding what a song file only a
/// removed song names, still at its name, holds, its bytes or its audio,
/// by that file's key, taken out of `unlisted`, as the module says.
fn copies(
    manifest: &Manifest,
    state: &State,
    store: &Store,
    unlisted: &mut Unlisted,
    hear: &mut impl FnMut(&Located) -> Option<String>,
) -> BTreeMap<SourceKey, SourceKey> {
    if manifest.removed.is_empty() || unlisted.ready.is_empty() {
        return BTreeMap::new();
    }
    let kept: Vec<(SourceKey, Was)> = (manifest.removed_keys().into_iter())
        .filter(|k| !unlisted.ready.contains(k))
        .filter(|k| store.locate(k).is_some_and(|l| l.kind == Kind::Media))
        .map(|k| {
            let was = Was::of(manifest, state, &k, Kind::Media, false);
            (k, was)
        })
        .collect();
    if kept.is_empty() {
        return BTreeMap::new();
    }
    let mut copies = BTreeMap::new();
    for key in &unlisted.ready {
        let file = File::new(store, key.clone(), Kind::Media, false);
        let of = [By::Bytes, By::Audio].into_iter().find_map(|by| {
            (kept.iter())
                .find(|(_, was)| file.is(was, by, state, hear))
                .map(|(k, _)| k.clone())
        });
        if let Some(of) = of {
            copies.insert(key.clone(), of);
        }
    }
    unlisted.ready.retain(|k| !copies.contains_key(k));
    copies
}

/// Each changed seeker whose file another took, not matched itself, put
/// in `moves` as moved over, by `pairs` and the files `taken` by seeker.
fn moved_over(
    seekers: &[Seeker],
    pairs: &BTreeMap<usize, usize>,
    taken: &BTreeMap<usize, usize>,
    moves: &mut Moves,
) {
    for (s, seeker) in seekers.iter().enumerate() {
        let Some(&by) = seeker.own.and_then(|f| taken.get(&f)) else {
            continue;
        };
        if pairs.contains_key(&s) {
            continue;
        }
        // Back along the files moved over one another to the key whose
        // file went, which no song names once they are followed.
        let mut freed = by;
        for _ in 0..seekers.len() {
            match seekers[freed].own.and_then(|f| taken.get(&f)) {
                Some(&next) => freed = next,
                None => break,
            }
        }
        moves
            .moved
            .insert(seeker.key.clone(), seekers[freed].key.clone());
        moves
            .over
            .insert(seeker.key.clone(), seekers[by].key.clone());
    }
}

/// Seekers by the file each is matched to, and sets of seekers beside
/// the files none of which can be told to be one of them.
type Pairs = (
    BTreeMap<usize, usize>,
    Vec<(BTreeSet<usize>, BTreeSet<usize>)>,
);

/// Each seeker matched to the one file that is it, by the strongest
/// evidence first, and the sets that cannot be told apart; a changed
/// file's seeker is kept matched only while another's match takes its
/// own path, as the module says.
fn pair(
    seekers: &[Seeker],
    files: &[File],
    state: &State,
    hear: &mut impl FnMut(&Located) -> Option<String>,
) -> Pairs {
    let mut open_seekers: BTreeSet<usize> = (0..seekers.len()).collect();
    let mut open_files: BTreeSet<usize> = (0..files.len()).collect();
    let mut pairs = BTreeMap::new();
    let mut unsure: Vec<(BTreeSet<usize>, BTreeSet<usize>)> = Vec::new();
    for by in [By::Stamp, By::Bytes, By::Audio] {
        // A changed file still itself, retagged or touched, is no other's.
        for (s, seeker) in seekers.iter().enumerate() {
            if let Some(f) = seeker.own.filter(|f| open_files.contains(f))
                && open_seekers.contains(&s)
                && files[f].is(&seeker.was, by, state, hear)
            {
                open_seekers.remove(&s);
                open_files.remove(&f);
            }
        }
        let mut found: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &s in &open_seekers {
            let matches: Vec<usize> = (open_files.iter().copied())
                .filter(|f| Some(*f) != seekers[s].own)
                .filter(|f| files[*f].is(&seekers[s].was, by, state, hear))
                .collect();
            if !matches.is_empty() {
                found.insert(s, matches);
            }
        }
        let mut claims: BTreeMap<usize, usize> = BTreeMap::new();
        for f in found.values().flatten() {
            *claims.entry(*f).or_default() += 1;
        }
        let mut groups: Vec<(BTreeSet<usize>, BTreeSet<usize>)> = Vec::new();
        for (&s, matches) in &found {
            open_seekers.remove(&s);
            if let [f] = matches[..]
                && claims[&f] == 1
            {
                open_files.remove(&f);
                pairs.insert(s, f);
                continue;
            }
            let mut group = (BTreeSet::from([s]), BTreeSet::from_iter(matches.clone()));
            groups.retain(|(their_seekers, their_files)| {
                let shared = their_files.intersection(&group.1).next().is_some();
                if shared {
                    group.0.extend(their_seekers);
                    group.1.extend(their_files);
                }
                !shared
            });
            groups.push(group);
        }
        for group in groups {
            for f in &group.1 {
                open_files.remove(f);
            }
            match interchangeable(&group, &found, seekers, files) {
                Some(matched) => pairs.extend(matched),
                None => unsure.push(group),
            }
        }
    }
    loop {
        let taken: BTreeSet<usize> = pairs.values().copied().collect();
        let left: Vec<usize> = (pairs.keys().copied())
            .filter(|s| seekers[*s].own.is_some_and(|f| !taken.contains(&f)))
            .collect();
        if left.is_empty() {
            break;
        }
        for s in left {
            pairs.remove(&s);
        }
    }
    (pairs, unsure)
}

/// The seekers of `group` each matched to one of its files, in the order
/// of their keys, where which is which changes nothing: as many files as
/// seekers, each seeker gone, of one [`Role`], and `found` to be any of
/// the files.
fn interchangeable(
    (group_seekers, group_files): &(BTreeSet<usize>, BTreeSet<usize>),
    found: &BTreeMap<usize, Vec<usize>>,
    seekers: &[Seeker],
    files: &[File],
) -> Option<Vec<(usize, usize)>> {
    let role = seekers[*group_seekers.first()?].role.as_ref()?;
    let alike = group_seekers.len() == group_files.len()
        && group_seekers.iter().all(|s| {
            seekers[*s].own.is_none()
                && seekers[*s].role.as_ref() == Some(role)
                && found.get(s).is_some_and(|m| m.len() == group_files.len())
        });
    if !alike {
        return None;
    }
    let mut by_seeker: Vec<usize> = group_seekers.iter().copied().collect();
    by_seeker.sort_by(|a, b| seekers[*a].key.cmp(&seekers[*b].key));
    let mut by_file: Vec<usize> = group_files.iter().copied().collect();
    by_file.sort_by(|a, b| files[*a].key.cmp(&files[*b].key));
    Some(by_seeker.into_iter().zip(by_file).collect())
}

/// The named manual files the next sync follows to where they moved, as
/// [`moved`] finds them among the files no song names, those arrived
/// within `settling` still being copied in.
pub fn found_moves(
    manifest: &Manifest,
    state: &State,
    store: &Store,
    settling: Duration,
    hear: impl FnMut(&Located) -> Option<String>,
) -> Moves {
    let mut unlisted = Unlisted::of(manifest, state, store, settling);
    moved(manifest, state, store, &mut unlisted, hear)
}

/// Each manual file of `sources`, a removed song's, and of `trashed`, the
/// files of the user's own `remove --purge` moved to the trash, that is
/// not back, by the key it is waited for at: where it was, or where
/// `moved` follows it, and a file beside a song file, its `.lrc` or a
/// picture, beside where that went ([`crate::manifest::moved_with`]).
/// `restore` and the `undo` of the purge wait for the same files, found
/// the same way ([`found_moves`]), as the sync after either lists them.
#[must_use]
pub fn not_back(
    manual: &Path,
    store: &Store,
    moved: &BTreeMap<SourceKey, SourceKey>,
    (sources, trashed): (&[SourceKey], &[SourceKey]),
) -> Vec<SourceKey> {
    let with = crate::manifest::moved_with(sources, trashed, moved);
    let on_disk = |k: &SourceKey| match k {
        SourceKey::Manual(m) => crate::relpath::on_disk(manual, m.path()).is_some(),
        _ => false,
    };
    let gone: BTreeSet<SourceKey> = (sources.iter().chain(trashed))
        .filter(|k| matches!(k, SourceKey::Manual(_)))
        .filter(|k| store.locate(k).is_none() && !on_disk(k))
        .filter(|k| !with.get(*k).is_some_and(on_disk))
        .map(|k| with.get(k).unwrap_or(k).clone())
        .collect();
    gone.into_iter().collect()
}

/// [`moved`]'s `hear`: the audio a song file holds, read by `runner`,
/// its files written below `scratch`.
pub fn hearing<'a, R: Runner>(
    runner: &'a R,
    scratch: &'a Path,
) -> impl FnMut(&Located) -> Option<String> + 'a {
    move |at| {
        let sub = super::Sub::new(scratch, "heard-").ok()?;
        crate::facts::hash(runner, at, sub.path()).ok()?.0.audio
    }
}

/// How [`Following::follow`] keeps what it follows.
#[derive(Debug)]
pub struct Following<'a> {
    pub home: &'a Path,
    pub lock: &'a crate::atomic::Lock,
    pub rec: crate::history::Recording<'a>,
    pub dry_run: bool,
    pub settling: Duration,
    /// Whether the caller lists the files dropped in next, as `sync` does,
    /// which takes a file deleted by hand out of its song too
    /// ([`deleted`]).
    pub listing: bool,
}

impl Following<'_> {
    /// Follow each named manual file to where it moved ([`moved`]), and
    /// when listing take each file deleted by hand out of its song and
    /// keep each copy of a removed song's file out with it
    /// ([`Moves::copies`]), in the song list and the state, saved, the
    /// list noted in `rec` and the moves in its run; a dry run does so in
    /// memory alone, so it reports on the songs as the next sync will
    /// find them. Returns the moves, with what it cannot tell
    /// ([`Moves::unsure`]), and the files no song names that are none of
    /// those followed.
    pub fn follow<W: Write>(
        mut self,
        manifest: &mut Manifest,
        state: &mut State,
        store: &Store,
        hear: impl FnMut(&Located) -> Option<String>,
        out: &mut W,
    ) -> Result<(Moves, Unlisted)> {
        let mut unlisted = Unlisted::of(manifest, state, store, self.settling);
        let moves = moved(manifest, state, store, &mut unlisted, hear);
        let deleted = if self.listing {
            deleted(manifest, store, &moves, &unlisted)
        } else {
            Vec::new()
        };
        for line in moves.unsure_lines() {
            crate::ui::warning(out, &line)?;
        }
        for line in moves.anew_lines(!self.listing) {
            crate::ui::info(out, &line)?;
        }
        for line in moves.copy_lines(!self.listing || self.dry_run) {
            crate::ui::info(out, &line)?;
        }
        if self.dry_run {
            for (copy, of) in &moves.copies {
                if let Some(removed) =
                    (manifest.removed.iter_mut()).find(|r| r.sources.contains(of))
                {
                    removed.sources.push(copy.clone());
                }
            }
        }
        let keep_out = self.listing && !self.dry_run && !moves.copies.is_empty();
        if moves.moved.is_empty() && deleted.is_empty() && !keep_out {
            return Ok((moves, unlisted));
        }
        for line in moves
            .moved
            .keys()
            .filter_map(|k| moves.line(k, self.dry_run))
        {
            crate::ui::info(out, &line)?;
        }
        for key in deleted {
            crate::ui::info(
                out,
                &format!(
                    "Deleted from sources/manual by hand, so taken out of its song: {key}; \
                     `muman undo` puts it back"
                ),
            )?;
            manifest.edit(Edit::Drop(key));
        }
        if keep_out {
            for (copy, of) in &moves.copies {
                manifest.edit(Edit::KeptOut {
                    with: of.clone(),
                    key: copy.clone(),
                });
            }
        }
        if self.dry_run {
            manifest.rekey(&moves.moved);
        } else {
            if !moves.moved.is_empty() {
                manifest.edit(Edit::Moved(moves.moved.clone()));
            }
            // Recorded ahead, so the undo of a run stopped once the list is
            // saved puts it back with the files followed.
            if let Some(run) = self.rec.run() {
                run.followed(&moves.moved);
                run.checkpoint(self.home)?;
            }
            // The list before the state: stopped between, the next run
            // measures the file again, where the other way it would find
            // no record of the file gone to follow it by.
            manifest.save_locked(self.lock, &mut self.rec)?;
        }
        state.rekey(&moves.of_state())?;
        if !self.dry_run && !moves.moved.is_empty() {
            state.save(self.home)?;
        }
        Ok((moves, unlisted))
    }
}

/// Each song file of the user's a song lists that is gone from
/// `sources/manual`, followed nowhere by `moves`, while another song file
/// of the user's the song lists is there and no file is arriving
/// ([`Unlisted::settling`]): deleted by hand, as a file encoded anew
/// leaves the old one, as the module says.
#[must_use]
pub fn deleted(
    manifest: &Manifest,
    store: &Store,
    moves: &Moves,
    unlisted: &Unlisted,
) -> Vec<SourceKey> {
    if !unlisted.settling.is_empty() {
        return Vec::new();
    }
    let unsure: BTreeSet<&SourceKey> = moves.unsure.iter().flat_map(|(gone, _)| gone).collect();
    let song_file = |k: &SourceKey| match k {
        SourceKey::Manual(m) => store::kind_of(m.path()) == Some(Kind::Media),
        _ => false,
    };
    let mut deleted = Vec::new();
    for song in &manifest.songs {
        if !(song.sources.iter()).any(|k| song_file(k) && store.has(k)) {
            continue;
        }
        deleted.extend(
            (song.sources.iter())
                .filter(|k| song_file(k) && !store.has(k))
                .filter(|k| !moves.moved.contains_key(*k) && !unsure.contains(k))
                .cloned(),
        );
    }
    deleted
}

/// Each listed manual file gone while a file no song lists is still
/// arriving, which may be it, moved by a copy that has not ended: its
/// song waits for it, as one waits for a fetch, rather than fail.
pub(crate) fn arriving(
    manifest: &Manifest,
    state: &State,
    store: &Store,
    settling: Duration,
) -> BTreeSet<SourceKey> {
    if Unlisted::of(manifest, state, store, settling)
        .settling
        .is_empty()
    {
        return BTreeSet::new();
    }
    manifest
        .keys()
        .into_iter()
        .filter(|k| matches!(k, SourceKey::Manual(_)) && !store.has(k))
        .collect()
}

/// Each listed source the song list's `[sites]` names otherwise, by the
/// key a fetch now names it: one of an extractor's name, as keys were
/// once named, by the site listing it, else by the site of the page its
/// facts or its song's record recorded; one of a domain a site lists as
/// an alias by that site. One no other song lists nor another source
/// takes. A source of yt-dlp's generic extractor keeps its name: what
/// it recorded is the mirror a redirect ended at, not the address it was
/// fetched by.
pub(super) fn sites_found(manifest: &Manifest, state: &State) -> BTreeMap<SourceKey, SourceKey> {
    let listed = manifest.keys();
    let sites = &manifest.settings.sites;
    let mut renames = BTreeMap::new();
    for song in &manifest.songs {
        for key in song.sources.iter().filter(|k| k.site() != Some("generic")) {
            let page = state
                .facts
                .get(key)
                .and_then(|f| f.served.as_ref()?.url.clone())
                .or_else(|| song.held.get(key)?.url.clone());
            let by_page = || {
                let (extractor, id) = key.site().zip(key.id())?;
                sites.fetched(extractor, page.as_deref(), id)
            };
            let to = sites
                .renamed(key)
                .or_else(|| key.is_old().then(by_page).flatten());
            if let Some(to) =
                to.filter(|t| t != key && !listed.contains(t) && !renames.values().any(|v| v == t))
            {
                renames.insert(key.clone(), to);
            }
        }
    }
    renames
}
