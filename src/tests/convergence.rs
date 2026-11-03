//! Random runs of commands, some killed partway, each leaving the library
//! one sync of the song list would build.
//!
//! A case is 8 to 12 steps over four songs of one's own: add one, retitle
//! one, remove or restore one, undo the last run, edit a setting a build
//! reads (the template, the codec lossless audio is written in, the
//! lyrics' format, the loudness mode), or sync. A step may be killed at one of its first
//! changes to a file, as a crash would stop it, then an undo stopped
//! partway is run again and a sync recovers.
//! After each step and a sync, which lists again the sources of one's own
//! an undo left unlisted, one more sync changes nothing, and the library
//! is the one a new home holding the same sources and song list builds in
//! one sync; [`crate::reconcile::invariant`] holds after every run.
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
        song.prop_map(Op::Restore),
        Just(Op::Undo),
        (0..EDITS.len()).prop_map(Op::Edit),
        Just(Op::Sync),
    ]
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec((op(), prop::option::weighted(0.25, 1..30_usize)), 8..=12)
}

/// Each song's probe: its own title and track, so no two share a path.
fn fake() -> Fake {
    SONGS
        .iter()
        .enumerate()
        .fold(Fake::default(), |fake, (n, name)| {
            let probe = FLAC
                .replace("\"TITLE\": \"Song\"", &format!("\"TITLE\": \"{name}\""))
                .replace("\"track\": \"2\"", &format!("\"track\": \"{}\"", n + 1));
            fake.probe(&format!("{name}.flac"), &probe)
        })
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
fn attempt(s: &Setup, fake: &Fake, args: &[&str]) {
    let _guard = crate::testing::guard_library(&s.job(args).dirs.library);
    let _ = run_with(
        &s.job(args),
        fake,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    let dirs = s.job(args).dirs;
    if dirs.home.join(crate::dirs::STATE).exists()
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
fn take(s: &Setup, fake: &Fake, (op, kill): &Step) {
    let args = command(s, op);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let Some(at) = kill else {
        attempt(s, fake, &args);
        return;
    };
    let killing = crate::testing::kill_at(s.dir.path(), *at);
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

/// The library one sync of `s`'s sources and song list builds in a new
/// home.
fn built_at_once(s: &Setup, fake: &Fake) -> BTreeMap<PathBuf, Vec<u8>> {
    let fresh = Setup::new();
    let sources = s.dir.path().join("home/sources/manual");
    for source in crate::store::files_below(&sources, usize::MAX, |_| true).unwrap() {
        let rel = source.strip_prefix(s.dir.path()).unwrap();
        let to = fresh.dir.path().join(rel);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(&source, &to).unwrap();
    }
    std::fs::copy(
        s.dir.path().join("home/songs.toml"),
        fresh.dir.path().join("home/songs.toml"),
    )
    .unwrap();
    fresh.run(fake, &["sync"]);
    library_of(&fresh)
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
        assert!(
            library_of(&s) == built_at_once(&s, &fake),
            "step {n}, {step:?}: the library is not the one its song list builds"
        );
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
#[ignore = "slow: more cases of the test above"]
fn many_random_runs_leave_the_library_one_sync_builds() {
    run_cases(64);
}
