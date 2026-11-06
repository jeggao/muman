//! Random runs of commands, some killed partway, each leaving the library
//! one sync of the song list would build.
//!
//! A case is 8 to 12 steps over four songs of one's own: add one, retitle
//! one, remove or restore one, undo the last run, edit a setting a build
//! reads (the template, the codec lossless audio is written in, the
//! lyrics' format, the loudness mode), break a song's source so it reads
//! as none or mend it, or sync. A step may be killed at one of its first
//! changes to a file, as a crash would stop it, then an undo stopped
//! partway is run again and a sync recovers.
//! After each step and a sync, which lists again the sources of one's own
//! an undo left unlisted, one more sync changes nothing, and the library
//! is the one a new home holding the same sources, mended, and song list
//! builds in one sync, but for the files of the songs broken, which stay
//! byte for byte as they were while they are; every file recorded has
//! the plan it was made from, and [`crate::reconcile::invariant`] holds
//! after every run.
//!
//! The runner reads no `PROPTEST_*` variable and writes no file: its
//! config is built whole here, never from `Config::default`, which reads
//! the environment, failures are not persisted, and the seed is fixed,
//! so every run tries the same cases. Without proptest's `fork` feature
//! the runner reads no variable of its own.
//!
//! Songs edited by hand, and two songs at one path, are left out: each
//! song has its own title, and nothing writes into the library but
//! muman.

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, RngSeed, TestRng, TestRunner};

use super::*;

const SONGS: [&str; 4] = ["Ash", "Birch", "Cedar", "Damson"];

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

#[derive(Debug, Clone)]
enum Op {
    Add(usize),
    Retitle(usize, bool),
    Remove(usize),
    Restore(usize),
    Undo,
    Edit(usize),
    /// The song's source made to read as none.
    Break(usize),
    Mend(usize),
    Sync,
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
        song.prop_map(Op::Mend),
        Just(Op::Undo),
        (0..EDITS.len()).prop_map(Op::Edit),
        Just(Op::Sync),
    ]
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec((op(), prop::option::weighted(0.25, 1..30_usize)), 8..=12)
}

/// What a broken source holds.
const GARBAGE: &[u8] = b"garbage";

fn source(n: usize) -> String {
    format!("home/sources/manual/{}.flac", SONGS[n])
}

/// The fake, but a tool given a file that holds [`GARBAGE`] fails, as
/// ffprobe fails on bytes that are no audio.
struct Breakable(Fake);

impl Breakable {
    fn refuses(cmd: &[std::ffi::OsString]) -> anyhow::Result<()> {
        if cmd
            .iter()
            .any(|a| std::fs::read(a).is_ok_and(|b| b == GARBAGE))
        {
            anyhow::bail!("Invalid data found when processing input");
        }
        Ok(())
    }
}

impl Runner for Breakable {
    fn run(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<()> {
        Self::refuses(cmd)?;
        self.0.run(cmd)
    }

    fn output(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<Vec<u8>> {
        Self::refuses(cmd)?;
        self.0.output(cmd)
    }

    fn stream(
        &self,
        cmd: &[std::ffi::OsString],
        on_line: &mut dyn FnMut(crate::runner::Line<'_>),
    ) -> anyhow::Result<bool> {
        Self::refuses(cmd)?;
        self.0.stream(cmd, on_line)
    }
}

/// Each song's probe: its own title and track, so no two share a path.
fn fake() -> Breakable {
    Breakable(
        SONGS
            .iter()
            .enumerate()
            .fold(Fake::default(), |fake, (n, name)| {
                let probe = FLAC
                    .replace("\"TITLE\": \"Song\"", &format!("\"TITLE\": \"{name}\""))
                    .replace("\"track\": \"2\"", &format!("\"track\": \"{}\"", n + 1));
                fake.probe(&format!("{name}.flac"), &probe)
            }),
    )
}

/// Whether song `n`'s source is broken.
fn broken(s: &Setup, n: usize) -> bool {
    std::fs::read(s.dir.path().join(source(n))).is_ok_and(|b| b == GARBAGE)
}

/// Each file of song `n` the state records, and what it holds.
fn files_of(s: &Setup, n: usize) -> BTreeMap<PathBuf, Vec<u8>> {
    let key = SourceKey::parse(&key(n)).unwrap();
    let Ok(state) = crate::state::State::load(&s.dir.path().join("home")) else {
        return BTreeMap::new();
    };
    let lib = s.dir.path().join("lib");
    (state.outputs.iter())
        .filter(|(_, w)| w.sources.contains(&key))
        .flat_map(|(p, w)| w.files(p).map(Path::to_path_buf).collect::<Vec<_>>())
        .filter_map(|f| Some((f.clone(), std::fs::read(lib.join(&f)).ok()?)))
        .collect()
}

fn key(n: usize) -> String {
    format!("manual:{}.flac", SONGS[n])
}

/// The command line `op` runs, once it has made what it needs.
fn command(s: &Setup, op: &Op) -> Vec<String> {
    let words = |w: &[&str]| w.iter().map(ToString::to_string).collect();
    match op {
        Op::Add(n) => {
            let source = format!("home/sources/manual/{}.flac", SONGS[*n]);
            if !s.dir.path().join(&source).exists() {
                s.file(&source);
            }
            words(&["sync"])
        }
        Op::Retitle(n, other) => {
            let title = format!(
                "{} {}",
                SONGS[*n],
                if *other { "Harbor" } else { "Lantern" }
            );
            words(&["set", "-y", &key(*n), &format!("title={title}")])
        }
        Op::Break(n) => {
            let path = s.dir.path().join(source(*n));
            if path.exists() {
                std::fs::write(path, GARBAGE).unwrap();
            }
            words(&["sync"])
        }
        Op::Mend(n) => {
            if broken(s, *n) {
                s.file(&source(*n));
            }
            words(&["sync"])
        }
        Op::Remove(n) => words(&["remove", "-y", &key(*n)]),
        Op::Restore(n) => words(&["restore", "-y", &key(*n)]),
        Op::Undo => words(&["undo", "-y"]),
        Op::Edit(n) => {
            let list = s.dir.path().join("home/songs.toml");
            let text = std::fs::read_to_string(&list).unwrap();
            let (from, to) = EDITS[*n];
            let edited = if text.contains(from) {
                text.replacen(from, to, 1)
            } else {
                text.replacen(to, from, 1)
            };
            std::fs::write(&list, edited).unwrap();
            words(&["sync"])
        }
        Op::Sync => words(&["sync"]),
    }
}

/// Run `args` on `s` as [`Setup::run`] does, a command refused, such
/// as a restore of a song listed, being one outcome among others.
fn attempt(s: &Setup, fake: &Breakable, args: &[&str]) {
    let _guard = crate::kill::guard_library(&s.job(args).dirs.library);
    let _ = run_with(
        &s.job(args),
        fake,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    let dirs = s.job(args).dirs;
    if crate::state::exists(&dirs.home)
        && let Err(e) = crate::reconcile::invariant(&dirs)
    {
        panic!("{args:?}: {e}");
    }
}

/// Run `step` on `s`, recovering where it was killed. An undo stopped
/// with its record still undoing is finished by the next undo, as
/// `an_undo_survives_a_kill_anywhere_and_finishes_when_run_again` has it,
/// since a sync alone leaves what it put back beside what it had not yet
/// removed; a sync then recovers.
fn take(s: &Setup, fake: &Breakable, (op, kill): &Step) {
    let args = command(s, op);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let Some(at) = kill else {
        attempt(s, fake, &args);
        return;
    };
    let killing = crate::kill::kill_at(s.dir.path(), *at);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_with(
            &s.job(&args),
            fake,
            &Server::default(),
            None,
            &mut Vec::new(),
            &mut Vec::new(),
        )
    }));
    drop(killing);
    // As old as a part a stopped run left long ago.
    let earlier = SystemTime::now() - std::time::Duration::from_secs(3600);
    for part in parts_in(s) {
        filetime::set_file_mtime(&part, filetime::FileTime::from_system_time(earlier)).unwrap();
    }
    if undoing(s) {
        attempt(s, fake, &["undo", "-y"]);
    }
    attempt(s, fake, &["sync"]);
}

/// Whether a run's record says an undo of it was stopped partway.
fn undoing(s: &Setup) -> bool {
    std::fs::read_dir(s.dir.path().join("home/history"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            std::fs::read_to_string(e.path().join("run.json"))
                .is_ok_and(|t| t.contains("\"undoing\":true"))
        })
}

/// The library one sync of `s`'s sources, mended, and song list builds in
/// a new home, and `s`'s, both without the files of the songs broken.
type Libraries = (BTreeMap<PathBuf, Vec<u8>>, BTreeMap<PathBuf, Vec<u8>>);

fn built_at_once(s: &Setup, fake: &Breakable) -> Libraries {
    let fresh = Setup::new();
    let sources = s.dir.path().join("home/sources/manual");
    for source in crate::store::files_below(&sources, usize::MAX, |_| true).unwrap() {
        let rel = source.strip_prefix(s.dir.path()).unwrap();
        let to = fresh.dir.path().join(rel);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        if std::fs::read(&source).is_ok_and(|b| b == GARBAGE) {
            fresh.file(&rel.to_string_lossy());
        } else {
            std::fs::copy(&source, &to).unwrap();
        }
    }
    std::fs::copy(
        s.dir.path().join("home/songs.toml"),
        fresh.dir.path().join("home/songs.toml"),
    )
    .unwrap();
    fresh.run(fake, &["sync"]);
    let without = |home: &Setup| {
        let mut library = library_of(home);
        for n in (0..SONGS.len()).filter(|&n| broken(s, n)) {
            for file in files_of(home, n).keys() {
                library.remove(file);
            }
        }
        library
    };
    (without(&fresh), without(s))
}

/// Fails unless every step of `steps`, from two songs synced, leaves the
/// library one sync of its song list builds.
fn converges(steps: &[Step]) {
    let s = Setup::new();
    let fake = fake();
    s.file("home/sources/manual/Ash.flac");
    s.written("home/sources/manual/Ash.lrc", "[00:01.00]Paper comets\n");
    s.file("home/sources/manual/Birch.flac");
    s.run(&fake, &["sync"]);
    for (n, step) in steps.iter().enumerate() {
        let held: Vec<(usize, BTreeMap<PathBuf, Vec<u8>>)> = (0..SONGS.len())
            .filter(|&n| broken(&s, n))
            .map(|n| (n, files_of(&s, n)))
            .collect();
        take(&s, &fake, step);
        // A song list put back by undo leaves sources of one's own unlisted,
        // which the next sync lists again.
        s.run(&fake, &["sync"]);
        let (_, text) = s.run(&fake, &["sync"]);
        for verb in ["Added", "Updated", "Written again", "Moved", "Removed"] {
            assert!(
                !text.contains(verb),
                "step {n}, {step:?}: a sync after changed: {text}"
            );
        }
        let (fresh, now) = built_at_once(&s, &fake);
        assert!(
            fresh == now,
            "step {n}, {step:?}: the library is not the one its song list builds"
        );
        // An undo puts back, a remove takes away and a restore brings back
        // a broken song's files; nothing else changes them.
        if !matches!(step.0, Op::Undo | Op::Remove(_) | Op::Restore(_)) {
            for (song, files) in &held {
                if broken(&s, *song) {
                    assert_eq!(
                        &files_of(&s, *song),
                        files,
                        "step {n}, {step:?}: a broken song's files changed"
                    );
                }
            }
        }
        let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
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

#[test]
fn random_runs_leave_the_library_one_sync_builds() {
    run_cases(3);
}

#[test]
#[ignore = "slow: more cases of the test above, run on each push to main"]
fn many_random_runs_leave_the_library_one_sync_builds() {
    run_cases(64);
}
