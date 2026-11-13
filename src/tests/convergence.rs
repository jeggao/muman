//! Random runs of commands, and of what a user does between them, some
//! killed partway, each leaving the library one sync of the song list
//! would build.
//!
//! A case is 8 to 12 steps over four songs of one's own. A step is a
//! command: add one, retitle one, remove or restore one, undo the last
//! run, edit a setting a build reads (the template, the codec lossless
//! audio is written in, the lyrics' format, the loudness mode), or sync.
//! Or it is what a user does by hand, then a sync: break a song's file so
//! it reads as none or mend it, move it to another folder of
//! `sources/manual` or back, trade two songs' files' names, rename and
//! retag one at once, drop in an encoding of one beside it, delete one,
//! put other audio at one's name, or change a file beside a song in the
//! library, its lyrics or the folder's cover. A broken file is neither
//! moved nor traded: what it held is gone, so nothing could tell it. A
//! step may be killed at one of its first changes to a file, as a crash
//! would stop it, then an undo stopped partway is run again and a sync
//! recovers.
//!
//! After each step and a sync, which lists again the sources of one's own
//! an undo left unlisted, one more sync changes nothing, and the library
//! is the one a new home builds in one sync from the same sources,
//! mended, and the same song list, its songs in the other order, so a
//! library that hangs on their order fails. The files of a song whose
//! files are broken or gone are left out of that, and stay byte for byte
//! as they were while none of its files reads. So are the files the user
//! changed, and the songs they hold where they are, as the guide says,
//! but for a folder's cover, which only stays in its folder: muman
//! neither writes over nor removes such a file, so what the user wrote
//! stays in the library. Every file recorded has the plan it was made
//! from, and [`crate::reconcile::invariant`] holds after every run.
//! Every sync, the one that recovers from a stopped run among them, does
//! with each library file what `status` foretold of it just before
//! ([`crate::reconcile::as_foretold`]): left as it was, moved, written or
//! gone. A sync that first lists a file no song names, which `status`
//! does not plan, and one whose move or removal fails, which departs from
//! what it meant, are not held to it.
//! An undo is refused, saying why, or puts the song list back as the run
//! it undoes found it, its keys following the files the user moved since,
//! and every library file but those the user changed with it, wherever
//! each source is where it was and holds what it held: a file renamed,
//! as a song file and its `.lrc` parted, changes what a song is made of.
//!
//! A song file is read for what it holds, not by its name, as a real file
//! is: it names its song and its recording, which [`Model`] hands the
//! fake as a file named for them, so tags go with a file renamed and an
//! encoding prints as its recording. Each recording has a title of its
//! own and each song a track, so no two songs share a path.
//!
//! The runner reads no `PROPTEST_*` variable and writes no file: its
//! config is built whole here, never from `Config::default`, which reads
//! the environment, failures are not persisted, and the seed is fixed,
//! so every run tries the same cases. Without proptest's `fork` feature
//! the runner reads no variable of its own.

use std::collections::BTreeSet;
use std::ffi::OsString;

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, RngSeed, TestRng, TestRunner};

use super::*;
use crate::manifest::{Removed, Song};

const SONGS: [&str; 4] = ["Ash", "Birch", "Cedar", "Damson"];

/// The recordings a song's files may hold, one put at a file's name after
/// another.
const RECORDINGS: u32 = 3;

const SEED: [u8; 32] = *b"muman converges on one sync seed";

/// A setting a build reads, as the song list writes it and as it is
/// changed to.
const EDITS: [(&str, &str); 4] = [
    (
        "template = \"{{ album_artist }}/{{ album }}/{{ disc_track }}{{ title }}\"",
        "template = \"{{ album }}/{{ title }}\"",
    ),
    (
        "codecs = [\"opus\", \"vorbis\", \"aac\", \"mp3\", \"flac\"]\nlossy = \"opus\"\nlossless = \"flac\"",
        "codecs = [\"opus\", \"vorbis\", \"aac\", \"mp3\"]\nlossy = \"opus\"\nlossless = \"wavpack\"",
    ),
    (
        "format = \"lrc\"\nwhen = \"any\"",
        "format = \"txt\"\nwhen = \"any\"",
    ),
    ("mode = \"tags\"", "mode = \"off\""),
];

/// The folder's cover written as a file of its own, after the tag every
/// song takes.
const FOLDER_FILE: (&str, &str) = (
    "mixed = \"none\"\n",
    "mixed = \"none\"\n\n[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\n",
);

#[derive(Debug, Clone)]
enum Op {
    Add(usize),
    Retitle(usize, bool),
    Remove(usize),
    Restore(usize),
    Undo,
    Edit(usize),
    /// The song's file made to read as none.
    Break(usize),
    Mend(usize),
    Sync,
    /// The song's file moved into a folder of `sources/manual`, or out.
    Move(usize),
    /// Two songs' files trading names.
    Swap(usize, usize),
    /// The song's file renamed and retagged at once, as a tagger does.
    Retag(usize),
    /// An encoding of the song's recording dropped in beside its file.
    Encode(usize),
    /// The song's file deleted by hand.
    Delete(usize),
    /// Another recording put at the song's file's name.
    Replace(usize),
    /// A library file beside a song changed by hand.
    Touch(usize),
}

/// A step, and the change to a file it is killed at, if any.
type Step = (Op, Option<usize>);

fn op() -> impl Strategy<Value = Op> {
    let song = 0..SONGS.len();
    prop_oneof![
        song.clone().prop_map(Op::Add),
        (song.clone(), any::<bool>()).prop_map(|(n, b)| Op::Retitle(n, b)),
        song.clone().prop_map(Op::Remove),
        song.clone().prop_map(Op::Restore),
        song.clone().prop_map(Op::Break),
        song.clone().prop_map(Op::Mend),
        Just(Op::Undo),
        (0..EDITS.len()).prop_map(Op::Edit),
        Just(Op::Sync),
        song.clone().prop_map(Op::Move),
        (song.clone(), song.clone()).prop_map(|(a, b)| Op::Swap(a, b)),
        song.clone().prop_map(Op::Retag),
        song.clone().prop_map(Op::Encode),
        song.clone().prop_map(Op::Delete),
        song.clone().prop_map(Op::Replace),
        song.prop_map(Op::Touch),
    ]
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec((op(), prop::option::weighted(0.25, 1..30_usize)), 8..=12)
}

/// What a broken file holds.
const GARBAGE: &[u8] = b"garbage";

/// What a file of song `n` holding `recording`, tagged as `tags` says,
/// holds.
fn holding(n: usize, recording: u32, tags: &str) -> String {
    format!("song:{}\naudio:{recording}\ntags:{tags}\n", SONGS[n])
}

/// The song and recording a song file holds.
fn held_in(bytes: &[u8]) -> Option<(&str, &str)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.lines();
    let song = lines.next()?.strip_prefix("song:")?;
    let audio = lines.next()?.strip_prefix("audio:")?;
    Some((song, audio))
}

/// The fake, given each song file as a file named for its song and
/// recording, holding that recording alone, as the module says; a tool
/// given a file that holds [`GARBAGE`] fails, as ffprobe fails on bytes
/// that are no audio.
struct Model(Fake);

impl Model {
    /// `cmd` with each song file it names in place, in `scratch`.
    fn read(cmd: &[OsString], scratch: &Path) -> anyhow::Result<Vec<OsString>> {
        cmd.iter()
            .map(|arg| {
                let Ok(bytes) = std::fs::read(arg) else {
                    return Ok(arg.clone());
                };
                if bytes == GARBAGE {
                    anyhow::bail!("Invalid data found when processing input");
                }
                let Some((song, audio)) = held_in(&bytes) else {
                    return Ok(arg.clone());
                };
                let path = scratch.join(format!("={song}={audio}=.flac"));
                std::fs::write(&path, format!("audio:{song}:{audio}"))?;
                Ok(path.into_os_string())
            })
            .collect()
    }
}

impl Runner for Model {
    fn run(&self, cmd: &[OsString]) -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        self.0.run(&Self::read(cmd, scratch.path())?)
    }

    fn output(&self, cmd: &[OsString]) -> anyhow::Result<Vec<u8>> {
        let scratch = tempfile::tempdir()?;
        self.0.output(&Self::read(cmd, scratch.path())?)
    }

    fn stream(
        &self,
        cmd: &[OsString],
        on_line: &mut dyn FnMut(crate::runner::Line<'_>),
    ) -> anyhow::Result<bool> {
        let scratch = tempfile::tempdir()?;
        self.0.stream(&Self::read(cmd, scratch.path())?, on_line)
    }

    fn pipe(&self, cmd: &[OsString], on_bytes: &mut dyn FnMut(&[u8])) -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        self.0.pipe(&Self::read(cmd, scratch.path())?, on_bytes)
    }

    fn fingerprint(&self, pcm: &Path) -> anyhow::Result<Vec<u32>> {
        self.0.fingerprint(pcm)
    }
}

/// Each recording's probe, its own title and its song's track, and its
/// print, so an encoding of it is one recording with it.
fn fake() -> Model {
    let mut fake = Fake::default();
    for (n, name) in SONGS.iter().enumerate() {
        for recording in 0..RECORDINGS {
            let title = match recording {
                0 => (*name).to_string(),
                r => format!("{name} Take {r}"),
            };
            let probe = FLAC
                .replace("\"TITLE\": \"Song\"", &format!("\"TITLE\": \"{title}\""))
                .replace("\"track\": \"2\"", &format!("\"track\": \"{}\"", n + 1));
            let at = format!("={name}={recording}=");
            let seed = (n as u64) * u64::from(RECORDINGS) + u64::from(recording) + 1;
            fake = fake.probe(&at, &probe).print(&at, words(1600, seed));
        }
    }
    Model(fake)
}

fn key_of(rel: &str) -> SourceKey {
    SourceKey::Manual(rel.into())
}

/// What the home and the library held as a run began.
#[derive(Debug, Clone)]
struct Found {
    songs: Vec<Song>,
    removed: Vec<Removed>,
    library: Files,
    /// Each file under `sources/manual`, and what it holds: a song's
    /// lyrics file is the one of its name, so a file renamed may change
    /// what a song is made of as well as its key.
    sources: BTreeMap<PathBuf, Vec<u8>>,
    /// How many of [`World::moves`] were made by then.
    moves: usize,
}

/// A home, what the user did in it, and what each run kept found.
struct World {
    s: Setup,
    fake: Model,
    /// Each song's files under `sources/manual`, the first the one a step
    /// acts on.
    files: [Vec<String>; 4],
    /// The recording each song's files are put with.
    recording: [u32; 4],
    /// How many files the user retagged or changed, so each change writes
    /// other bytes.
    edits: usize,
    /// What each path under `sources/manual` was last put as: what a
    /// broken or gone one is mended to.
    put: BTreeMap<String, Vec<u8>>,
    /// What the user wrote into library files.
    changed: Vec<Vec<u8>>,
    /// What each run kept found as it began, by its folder's name.
    found: BTreeMap<OsString, Found>,
    /// The files the user moved, as keys, a set of moves at once each.
    moves: Vec<BTreeMap<SourceKey, SourceKey>>,
}

impl World {
    fn new() -> Self {
        Self {
            s: Setup::new(),
            fake: fake(),
            files: Default::default(),
            recording: [0; 4],
            edits: 0,
            put: BTreeMap::new(),
            changed: Vec::new(),
            found: BTreeMap::new(),
            moves: Vec::new(),
        }
    }

    fn home(&self) -> PathBuf {
        self.s.dir.path().join("home")
    }

    fn manual(&self) -> PathBuf {
        self.home().join("sources").join("manual")
    }

    /// Put song `n`'s recording at `rel`, tagged as `tags` says, written a
    /// minute ago as [`Setup::file`] writes a file.
    fn put_at(&mut self, n: usize, rel: &str, tags: &str) {
        let path = self.manual().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = holding(n, self.recording[n], tags).into_bytes();
        std::fs::write(&path, &bytes).unwrap();
        let earlier = SystemTime::now() - std::time::Duration::from_secs(60);
        filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(earlier)).unwrap();
        self.put.insert(rel.to_string(), bytes);
    }

    /// The first of the names song `n`'s file may take that no file has.
    fn free_name(&self, n: usize) -> String {
        let name = SONGS[n];
        let stems = [name.to_string(), format!("{name}-re"), format!("{name}-2")];
        stems
            .iter()
            .flat_map(|stem| [format!("{stem}.flac"), format!("sorted/{stem}.flac")])
            .find(|rel| !self.manual().join(rel).exists())
            .unwrap()
    }

    /// The key of song `n`'s file, or the name its file takes.
    fn key(&self, n: usize) -> String {
        let rel = (self.files[n].first().cloned()).unwrap_or_else(|| format!("{}.flac", SONGS[n]));
        format!("manual:{rel}")
    }

    /// Song `n`'s first file renamed to `to`, unless a file is there.
    fn rename(&mut self, n: usize, to: String) {
        let Some(from) = self.files[n].first().cloned() else {
            return;
        };
        let target = self.manual().join(&to);
        if target.exists() {
            return;
        }
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::rename(self.manual().join(&from), &target).unwrap();
        if let Some(bytes) = self.put.get(&from).cloned() {
            self.put.insert(to.clone(), bytes);
        }
        self.moves
            .push(BTreeMap::from([(key_of(&from), key_of(&to))]));
        self.files[n][0] = to;
    }

    /// The command line `op` runs, once what the user does is done.
    fn command(&mut self, op: &Op) -> Vec<String> {
        let words = |w: &[&str]| w.iter().map(ToString::to_string).collect();
        match *op {
            Op::Retitle(n, other) => {
                let title = format!("{} {}", SONGS[n], if other { "Harbor" } else { "Lantern" });
                return words(&["set", "-y", &self.key(n), &format!("title={title}")]);
            }
            Op::Remove(n) => return words(&["remove", "-y", &self.key(n)]),
            Op::Restore(n) => return words(&["restore", "-y", &self.key(n)]),
            Op::Undo => return words(&["undo", "-y"]),
            Op::Move(_) | Op::Swap(..) | Op::Retag(_) => self.move_by_hand(op),
            _ => self.by_hand(op),
        }
        words(&["sync"])
    }

    /// What the user does by hand for `op`, but move a file.
    fn by_hand(&mut self, op: &Op) {
        let first = |w: &Self, n: usize| w.files[n].first().cloned();
        match *op {
            Op::Add(n) => {
                if self.files[n].is_empty() {
                    let rel = self.free_name(n);
                    self.put_at(n, &rel, "0");
                    self.files[n].push(rel);
                }
            }
            Op::Edit(n) => {
                let list = self.home().join("songs.toml");
                let text = std::fs::read_to_string(&list).unwrap();
                let (from, to) = EDITS[n];
                let edited = if text.contains(from) {
                    text.replacen(from, to, 1)
                } else {
                    text.replacen(to, from, 1)
                };
                std::fs::write(&list, edited).unwrap();
            }
            Op::Break(n) => {
                if let Some(rel) = first(self, n) {
                    std::fs::write(self.manual().join(rel), GARBAGE).unwrap();
                }
            }
            Op::Mend(n) => {
                for rel in self.files[n].clone() {
                    let path = self.manual().join(&rel);
                    if std::fs::read(&path).is_ok_and(|b| b == GARBAGE) {
                        std::fs::write(&path, &self.put[&rel]).unwrap();
                    }
                }
            }
            Op::Encode(n) => {
                if self.files[n].len() == 1 {
                    let rel = self.free_name(n);
                    self.put_at(n, &rel, "encoded");
                    self.files[n].push(rel);
                }
            }
            Op::Delete(n) => {
                if let Some(rel) = first(self, n) {
                    std::fs::remove_file(self.manual().join(&rel)).unwrap();
                    self.files[n].remove(0);
                }
            }
            Op::Replace(n) => {
                if let Some(rel) = first(self, n) {
                    self.recording[n] = (self.recording[n] + 1) % RECORDINGS;
                    self.put_at(n, &rel, "0");
                }
            }
            Op::Touch(n) => {
                let beside: Vec<PathBuf> = (library_of(&self.s).into_keys())
                    .filter(|p| {
                        p.extension()
                            .is_some_and(|e| e == "lrc" || e == "txt" || e == "jpg")
                    })
                    .collect();
                if !beside.is_empty() {
                    let file = self
                        .s
                        .dir
                        .path()
                        .join("lib")
                        .join(&beside[n % beside.len()]);
                    // What the user wrote before is theirs to write over.
                    let was = std::fs::read(&file).unwrap();
                    self.changed.retain(|b| *b != was);
                    self.edits += 1;
                    let bytes = format!("changed by hand, {} times\n", self.edits);
                    std::fs::write(&file, &bytes).unwrap();
                    self.changed.push(bytes.into_bytes());
                }
            }
            _ => {}
        }
    }

    /// The user's files moved for `op`.
    fn move_by_hand(&mut self, op: &Op) {
        let first = |w: &Self, n: usize| w.files[n].first().cloned();
        match *op {
            Op::Move(n) => {
                // A broken file is told by what it held no more than in a
                // swap: its song waits at its name.
                if let Some(rel) = first(self, n).filter(|rel| self.reads(&key_of(rel))) {
                    let to = match rel.strip_prefix("sorted/") {
                        Some(out) => out.to_string(),
                        None => format!("sorted/{rel}"),
                    };
                    self.rename(n, to);
                }
            }
            Op::Swap(a, b) => {
                // Each is told by what it held, which a broken file no
                // longer holds: one would be taken as changed in place.
                if let (Some(ra), Some(rb)) = (first(self, a), first(self, b))
                    && a != b
                    && self.reads(&key_of(&ra))
                    && self.reads(&key_of(&rb))
                {
                    let aside = self.s.dir.path().join("aside.flac");
                    std::fs::rename(self.manual().join(&ra), &aside).unwrap();
                    std::fs::rename(self.manual().join(&rb), self.manual().join(&ra)).unwrap();
                    std::fs::rename(&aside, self.manual().join(&rb)).unwrap();
                    let (pa, pb) = (self.put.get(&ra).cloned(), self.put.get(&rb).cloned());
                    pb.map(|p| self.put.insert(ra.clone(), p));
                    pa.map(|p| self.put.insert(rb.clone(), p));
                    self.moves.push(BTreeMap::from([
                        (key_of(&ra), key_of(&rb)),
                        (key_of(&rb), key_of(&ra)),
                    ]));
                    self.files[a][0] = rb;
                    self.files[b][0] = ra;
                }
            }
            Op::Retag(n) => {
                if let Some(rel) = first(self, n) {
                    let to = match rel.strip_suffix("-re.flac") {
                        Some(stem) => format!("{stem}.flac"),
                        None => rel.replacen(".flac", "-re.flac", 1),
                    };
                    self.rename(n, to);
                    if let Some(rel) = first(self, n) {
                        self.edits += 1;
                        let tags = format!("retagged {}", self.edits);
                        self.put_at(n, &rel, &tags);
                    }
                }
            }
            _ => {}
        }
    }

    /// The folders of the runs kept.
    fn runs(&self) -> BTreeSet<OsString> {
        std::fs::read_dir(self.home().join("history"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join("run.json").exists())
            .map(|e| e.file_name())
            .collect()
    }

    /// The latest run kept, ended and not being undone, and what it found.
    fn latest_ended(&self) -> Option<Found> {
        let last = self.runs().pop_last()?;
        let path = self.home().join("history").join(&last).join("run.json");
        let record: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        let flag = |name: &str| record[name].as_bool();
        let ended = flag("complete") != Some(false)
            && flag("undoing") != Some(true)
            && flag("undone") != Some(true);
        ended.then(|| self.found.get(&last).cloned()).flatten()
    }

    fn snapshot(&self) -> Found {
        let manifest = Manifest::load(&self.home()).unwrap();
        let sources = crate::store::files_below(&self.manual(), usize::MAX, |_| true)
            .unwrap_or_default()
            .into_iter()
            .map(|f| (f.clone(), std::fs::read(&f).unwrap()))
            .collect();
        Found {
            songs: manifest.songs,
            removed: manifest.removed,
            library: library_of(&self.s),
            sources,
            moves: self.moves.len(),
        }
    }

    /// Note `found` as what each run kept that `before` did not list found.
    fn began(&mut self, before: &BTreeSet<OsString>, found: &Found) {
        for run in self.runs().difference(before) {
            self.found.insert(run.clone(), found.clone());
        }
    }

    /// Run `args` as [`Setup::run`] does, a command refused, such as a
    /// restore of a song listed, being one outcome among others, with what
    /// it said; a sync does to each library file what `status` foretold
    /// just before it ([`crate::reconcile::as_foretold`]).
    fn attempt(&self, args: &[&str]) -> (anyhow::Result<bool>, String) {
        let dirs = self.s.job(args).dirs;
        let foretold = (args == ["sync"])
            .then(|| self.foretold(&dirs))
            .flatten()
            .map(|(f, said)| (f, said, crate::reconcile::stamped(&dirs.library)));
        let mut out = Vec::new();
        let done = {
            let _guard = crate::kill::guard_library(&dirs.library);
            run_with(
                &self.s.job(args),
                &self.fake,
                &Server::default(),
                None,
                &mut out,
                &mut Vec::new(),
            )
        };
        let text = String::from_utf8_lossy(&out).into_owned();
        if crate::state::exists(&dirs.home)
            && let Err(e) = crate::reconcile::invariant(&dirs)
        {
            panic!("{args:?}: {e}: {text}");
        }
        // A move or a removal that fails departs from the intent, as it says.
        if let Some((foretold, said, before)) = foretold.filter(|_| !text.contains("Could not")) {
            let after = crate::reconcile::stamped(&dirs.library);
            if let Err(e) = crate::reconcile::as_foretold(&foretold, &before, &after) {
                panic!("the sync did otherwise than status foretold: {e}\n{said}\n{text}");
            }
        }
        (done, text)
    }

    /// What a dry run of a sync now foretells it does with each library
    /// file; none when the sync lists a song first, which a dry run does
    /// not plan, or there is nothing to sync yet.
    fn foretold(&self, dirs: &Dirs) -> Option<(crate::reconcile::Foretold, String)> {
        let mut said = Vec::new();
        let opts = crate::reconcile::Options {
            dry_run: true,
            ..crate::reconcile::Options::default()
        };
        let rec = crate::history::Recording::DryRun;
        let synced = crate::reconcile::reconcile(&self.fake, dirs, opts, rec, &mut said).ok()?;
        let said = String::from_utf8_lossy(&said).into_owned();
        let lists = ["added by the next sync", "listed by the next sync"];
        let listing = lists.iter().any(|l| said.contains(l));
        Some((synced.foretold.filter(|_| !listing)?, said))
    }

    /// [`Setup::run`], what a run it begins found noted, a sync held to
    /// what `status` foretold of it.
    fn run(&mut self, args: &[&str]) -> (bool, String) {
        let (before, found) = (self.runs(), self.snapshot());
        let done = if args == ["sync"] {
            let (done, text) = self.attempt(args);
            (done.unwrap(), text)
        } else {
            self.s.run(&self.fake, args)
        };
        self.began(&before, &found);
        done
    }

    /// Take `step`, recovering where it was killed. An undo stopped with
    /// its record still undoing is finished by the next undo, as
    /// `an_undo_survives_a_kill_anywhere_and_finishes_when_run_again` has
    /// it, since a sync alone leaves what it put back beside what it had
    /// not yet removed; a sync then recovers.
    fn take(&mut self, (op, kill): &Step, n: usize) {
        let (before, found) = (self.runs(), self.snapshot());
        let undone = self.latest_ended();
        let args = self.command(op);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let Some(at) = kill else {
            let (done, _) = self.attempt(&args);
            self.began(&before, &found);
            if matches!(op, Op::Undo) {
                match done {
                    Ok(_) => {
                        if let Some(undone) = undone {
                            self.back_as_found(&undone, n);
                        }
                    }
                    Err(e) => assert!(
                        e.downcast_ref::<crate::errors::Refused>().is_some(),
                        "step {n}: undo failed rather than refused: {e:#}"
                    ),
                }
            }
            return;
        };
        let killing = crate::kill::kill_at(self.s.dir.path(), *at);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_with(
                &self.s.job(&args),
                &self.fake,
                &Server::default(),
                None,
                &mut Vec::new(),
                &mut Vec::new(),
            )
        }));
        drop(killing);
        // As old as a part a stopped run left long ago.
        let earlier = SystemTime::now() - std::time::Duration::from_secs(3600);
        for part in parts_in(&self.s) {
            filetime::set_file_mtime(&part, filetime::FileTime::from_system_time(earlier)).unwrap();
        }
        if self.undoing() {
            let _ = self.attempt(&["undo", "-y"]);
        }
        let _ = self.attempt(&["sync"]);
        self.began(&before, &found);
    }

    /// Whether a run's record says an undo of it was stopped partway.
    fn undoing(&self) -> bool {
        std::fs::read_dir(self.home().join("history"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                std::fs::read_to_string(e.path().join("run.json"))
                    .is_ok_and(|t| t.contains("\"undoing\":true"))
            })
    }

    /// Each key the user's moves since the first `from` took a file named
    /// then to, by that key.
    fn moved_since(&self, from: usize) -> BTreeMap<SourceKey, SourceKey> {
        let mut to: BTreeMap<SourceKey, SourceKey> = BTreeMap::new();
        for moves in &self.moves[from..] {
            let arrived: BTreeSet<SourceKey> = to.values().cloned().collect();
            for now in to.values_mut() {
                if let Some(next) = moves.get(now) {
                    now.clone_from(next);
                }
            }
            for (key, next) in moves {
                // A file at a name another left since is no file of then.
                if !arrived.contains(key) && !to.contains_key(key) {
                    to.insert(key.clone(), next.clone());
                }
            }
        }
        to.retain(|k, v| k != v);
        to
    }

    /// Fails unless the song list is the one `found` holds, its keys
    /// moved as the user moved their files, and the library too where the
    /// sources are as they were, as the module says.
    fn back_as_found(&self, found: &Found, n: usize) {
        let moved = self.moved_since(found.moves);
        let now = Manifest::load(&self.home()).unwrap();
        let songs = |songs: Vec<Song>, moved: &BTreeMap<SourceKey, SourceKey>| -> Vec<Song> {
            (songs.into_iter())
                .map(|mut s| {
                    s.rekey(moved);
                    s.held.clear();
                    s
                })
                .collect()
        };
        let removed = |removed: Vec<Removed>, moved: &BTreeMap<SourceKey, SourceKey>| {
            (removed.into_iter())
                .map(|mut r| {
                    for k in &mut r.sources {
                        if let Some(to) = moved.get(k) {
                            k.clone_from(to);
                        }
                    }
                    r.held.clear();
                    r
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            songs(now.songs, &BTreeMap::new()),
            songs(found.songs.clone(), &moved),
            "step {n}: undo put back other songs than the run found"
        );
        assert_eq!(
            removed(now.removed, &BTreeMap::new()),
            removed(found.removed.clone(), &moved),
            "step {n}: undo put back other removed songs than the run found"
        );
        let again = self.snapshot();
        if again.sources != found.sources {
            return;
        }
        let mine = |library: &Files| -> Files {
            (library.iter())
                .filter(|(_, b)| !self.changed.contains(b))
                .map(|(p, b)| (p.clone(), b.clone()))
                .collect()
        };
        let (now, then) = (mine(&again.library), mine(&found.library));
        let differ: Vec<&PathBuf> = (now.keys().chain(then.keys()))
            .filter(|p| now.get(*p) != then.get(*p))
            .collect();
        assert!(
            differ.is_empty(),
            "step {n}: undo left the library otherwise than the run found it: {differ:?}"
        );
    }

    /// Whether the manual file `key` names is there and reads.
    fn reads(&self, key: &SourceKey) -> bool {
        let SourceKey::Manual(m) = key else {
            return true;
        };
        std::fs::read(self.manual().join(m.path())).is_ok_and(|b| b != GARBAGE)
    }

    /// The sources of each listed song any of whose files of one's own is
    /// broken or gone, and whether none of them reads.
    fn troubled(&self) -> Vec<(Vec<SourceKey>, bool)> {
        let manifest = Manifest::load(&self.home()).unwrap();
        (manifest.songs.into_iter())
            .filter(|s| s.sources.iter().any(|k| !self.reads(k)))
            .map(|s| {
                let none = !s.sources.iter().any(|k| self.reads(k));
                (s.sources, none)
            })
            .collect()
    }

    /// The library one sync of the sources, mended, and the song list,
    /// its songs in the other order, builds in a new home, and this one's,
    /// both without the files of troubled songs and those the user changed,
    /// and of the songs left as built for them.
    fn built_at_once(&self) -> Libraries {
        let fresh = Setup::new();
        let manual = fresh.dir.path().join("home").join("sources").join("manual");
        for source in crate::store::files_below(&self.manual(), usize::MAX, |_| true).unwrap() {
            let rel = source.strip_prefix(self.manual()).unwrap();
            let to = manual.join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(&source, &to).unwrap();
        }
        let troubled = self.troubled();
        for key in troubled.iter().flat_map(|(keys, _)| keys) {
            if let SourceKey::Manual(m) = key
                && !self.reads(key)
                && let Some(bytes) = self.put.get(&m.path().to_string_lossy().into_owned())
            {
                let to = manual.join(m.path());
                std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                std::fs::write(to, bytes).unwrap();
            }
        }
        let list = std::fs::read_to_string(self.home().join("songs.toml")).unwrap();
        std::fs::write(fresh.dir.path().join("home/songs.toml"), reversed(&list)).unwrap();
        fresh.run(&self.fake, &["sync"]);
        let changed: Vec<PathBuf> = (library_of(&self.s).into_iter())
            .filter(|(_, b)| self.changed.contains(b))
            .map(|(p, _)| p)
            .collect();
        // A song a file of which the user changed, or that would write one,
        // is left as it is, but for the folder's cover, which stays in its
        // folder as the song moves and keeps no song from writing.
        let left: Vec<Vec<SourceKey>> = [&self.s, &fresh]
            .iter()
            .flat_map(|home| {
                let state = crate::state::State::load(&home.dir.path().join("home")).unwrap();
                (state.outputs.into_iter())
                    .filter(|(p, w)| {
                        w.files(p).any(|f| {
                            changed.iter().any(|c| c == f)
                                && f.file_stem().is_none_or(|stem| stem != "cover")
                        })
                    })
                    .map(|(_, w)| w.sources)
                    .collect::<Vec<_>>()
            })
            .collect();
        let without = |home: &Setup| {
            let mut library = library_of(home);
            let mut out: BTreeSet<PathBuf> = BTreeSet::new();
            for keys in troubled.iter().map(|(keys, _)| keys).chain(&left) {
                out.extend(files_of(home, keys).into_keys());
            }
            // Which of a folder's songs holds its cover is no matter: one
            // stays while another song's file is beside it.
            let kept: BTreeSet<PathBuf> = (library.keys())
                .filter(|p| !out.contains(*p) && p.file_stem().is_none_or(|s| s != "cover"))
                .filter_map(|p| p.parent().map(Path::to_path_buf))
                .collect();
            out.retain(|p| {
                p.file_stem().is_none_or(|s| s != "cover")
                    || !p.parent().is_some_and(|f| kept.contains(f))
            });
            for file in &out {
                library.remove(file);
            }
            for file in &changed {
                library.remove(file);
            }
            library
        };
        (without(&fresh), without(&self.s))
    }
}

/// The song list `text` with its songs in the other order.
fn reversed(text: &str) -> String {
    let mut doc: toml_edit::DocumentMut = text.parse().unwrap();
    if let Some(songs) = doc
        .get_mut("song")
        .and_then(toml_edit::Item::as_array_of_tables_mut)
    {
        let mut tables: Vec<toml_edit::Table> = songs.iter().cloned().collect();
        tables.reverse();
        songs.clear();
        for mut table in tables {
            unplace(&mut table);
            songs.push(table);
        }
    }
    doc.to_string()
}

/// `table` and the tables in it printed where the array they are in is.
fn unplace(table: &mut toml_edit::Table) {
    table.set_position(None);
    for (_, item) in table.iter_mut() {
        if let Some(t) = item.as_table_mut() {
            unplace(t);
        }
    }
}

/// Each library file the state records for the song of `keys`, and what
/// it holds.
fn files_of(s: &Setup, keys: &[SourceKey]) -> Files {
    let Ok(state) = crate::state::State::load(&s.dir.path().join("home")) else {
        return BTreeMap::new();
    };
    let lib = s.dir.path().join("lib");
    (state.outputs.iter())
        .filter(|(_, w)| w.sources.iter().any(|k| keys.contains(k)))
        .flat_map(|(p, w)| w.files(p).map(Path::to_path_buf).collect::<Vec<_>>())
        .filter_map(|f| Some((f.clone(), std::fs::read(lib.join(&f)).ok()?)))
        .collect()
}

/// Library files, by their paths, and what each holds.
type Files = BTreeMap<PathBuf, Vec<u8>>;

/// The library one sync builds in a new home, and the home's own.
type Libraries = (Files, Files);

/// Fails unless every step of `steps`, from two songs synced, leaves the
/// library one sync of its song list builds, as the module says.
fn converges(steps: &[Step]) {
    let mut w = World::new();
    std::fs::create_dir_all(w.home()).unwrap();
    std::fs::write(
        w.home().join("songs.toml"),
        crate::manifest::NEW.replacen(FOLDER_FILE.0, FOLDER_FILE.1, 1),
    )
    .unwrap();
    for (n, name) in SONGS.iter().enumerate().take(2) {
        let rel = format!("{name}.flac");
        w.put_at(n, &rel, "0");
        w.files[n].push(rel);
    }
    w.s.written("home/sources/manual/Ash.lrc", "[00:01.00]Paper comets\n");
    w.run(&["sync"]);
    for (n, step) in steps.iter().enumerate() {
        let held: Vec<(Vec<SourceKey>, Files)> = (w.troubled().into_iter())
            .filter(|(_, none)| *none)
            .map(|(keys, _)| {
                let files = files_of(&w.s, &keys);
                (keys, files)
            })
            .collect();
        w.take(step, n);
        // A song list put back by undo leaves sources of one's own unlisted,
        // which the next sync lists again.
        w.run(&["sync"]);
        let (_, text) = w.run(&["sync"]);
        for verb in ["Added", "Updated", "Written again", "Moved", "Removed"] {
            assert!(
                !text.contains(verb),
                "step {n}, {step:?}: a sync after changed: {text}"
            );
        }
        let (fresh, now) = w.built_at_once();
        let differ: Vec<&PathBuf> = (fresh.keys().chain(now.keys()))
            .filter(|p| fresh.get(*p) != now.get(*p))
            .collect();
        assert!(
            differ.is_empty(),
            "step {n}, {step:?}: the library is not the one its song list builds: {differ:?}"
        );
        let library = library_of(&w.s);
        for bytes in &w.changed {
            assert!(
                library.values().any(|b| b == bytes),
                "step {n}, {step:?}: a file the user changed was written over or removed"
            );
        }
        // An undo puts back, a remove takes away and a restore brings back
        // a troubled song's files; nothing else changes them.
        if !matches!(step.0, Op::Undo | Op::Remove(_) | Op::Restore(_)) {
            let still: Vec<Vec<SourceKey>> = (w.troubled().into_iter())
                .filter(|(_, none)| *none)
                .map(|(keys, _)| keys)
                .collect();
            for (keys, files) in held.iter().filter(|(k, _)| still.contains(k)) {
                let mut now = files_of(&w.s, keys);
                let changed: BTreeSet<PathBuf> = (now.iter())
                    .filter(|(_, b)| w.changed.contains(b))
                    .map(|(p, _)| p.clone())
                    .collect();
                now.retain(|p, _| !changed.contains(p));
                let mut files = files.clone();
                files.retain(|p, _| !changed.contains(p));
                assert_eq!(
                    now, files,
                    "step {n}, {step:?}: the files of a song none of whose files reads changed"
                );
            }
        }
        let state = crate::state::State::load(&w.home()).unwrap();
        for (path, written) in &state.outputs {
            assert!(
                written.plan.is_some(),
                "step {n}, {step:?}: {} has no plan",
                path.display()
            );
        }
    }
}

fn run_cases(cases: u32) {
    let config = Config {
        cases,
        max_local_rejects: 0,
        max_global_rejects: 0,
        max_flat_map_regens: 0,
        failure_persistence: None,
        source_file: None,
        test_name: None,
        max_shrink_time: 0,
        max_shrink_iters: 16,
        max_default_size_range: 100,
        result_cache: proptest::test_runner::noop_result_cache,
        verbose: 0,
        rng_algorithm: RngAlgorithm::ChaCha,
        rng_seed: RngSeed::Fixed(0),
        _non_exhaustive: (),
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &SEED));
    if let Err(e) = runner.run(&steps(), |steps| {
        converges(&steps);
        Ok(())
    }) {
        panic!("{e}");
    }
}

/// A dry run before the sync follows the two files in memory alone, and
/// what it measured of each is never kept under the other's name.
#[test]
fn two_files_trading_names_are_synced_as_status_foretold() {
    converges(&[(Op::Swap(1, 0), None)]);
}

#[test]
fn random_runs_leave_the_library_one_sync_builds() {
    run_cases(3);
}

#[test]
#[ignore = "slow: more cases of the test above, run on each push to main"]
fn many_random_runs_leave_the_library_one_sync_builds() {
    run_cases(64);
}
