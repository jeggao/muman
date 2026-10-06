use super::*;
use crate::testing::{Fake, words};

struct Home {
    _dir: tempfile::TempDir,
    dirs: Dirs,
}

fn home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        home: dir.path().join("home"),
        library: dir.path().join("lib"),
    };
    std::fs::create_dir_all(&dirs.home).unwrap();
    std::fs::create_dir_all(&dirs.library).unwrap();
    Home { _dir: dir, dirs }
}

impl Home {
    fn fetched(&self, id: &str) -> PathBuf {
        let path = self.dirs.ytdlp().join(format!("chan/Song {id} [{id}].mkv"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, id.as_bytes()).unwrap();
        path
    }

    fn songs(&self, body: &str) {
        std::fs::write(self.dirs.manifest(), format!("version = 1\n{body}")).unwrap();
    }

    fn run(&self, fake: &Fake, opts: Options) -> (bool, String) {
        let mut out = Vec::new();
        let ok = reconcile(fake, &self.dirs, opts, None, &mut out).unwrap();
        (ok, String::from_utf8(out).unwrap())
    }

    fn lib(&self, rel: &str) -> PathBuf {
        self.dirs.library.join(rel)
    }
}

/// An info JSON naming each upload by its ID.
fn infos(ids: &[&str]) -> Fake {
    ids.iter().fold(Fake::default(), |f, id| {
        f.info(
            id,
            &format!(r#"{{"id": "{id}", "title": "Title {id}", "uploader": "Chan", "subtitles": {{"en": [{{"name": "English"}}]}}}}"#),
        )
    })
}

fn renders(fake: &Fake) -> usize {
    fake.calls()
        .iter()
        .filter(|c| c.iter().any(|a| a == "opus"))
        .count()
}

const TWO: &str = "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"youtube:bbbbbbbbbbb\"]\n";

#[test]
fn a_sync_writes_every_song_once_and_then_nothing() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(
        text.contains("Added: Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus"),
        "{text}"
    );
    assert!(
        h.lib("Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.opus")
            .exists()
    );
    assert!(
        h.lib("Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.lrc")
            .exists()
    );
    let state = State::load(&h.dirs.home).unwrap();
    assert_eq!(state.outputs.len(), 2);
    assert_eq!(state.facts.len(), 2);

    let again = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let (ok, text) = h.run(&again, Options::default());
    assert!(ok, "{text}");
    assert!(
        again.calls().is_empty(),
        "facts are cached and nothing changed: {:?}",
        again.calls()
    );
    assert!(text.contains("Up to date: 2 song(s)"), "{text}");
}

#[test]
fn a_changed_tag_rewrites_only_its_song() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    h.run(&infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]), Options::default());
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"youtube:bbbbbbbbbbb\"]\ntags = { genre = \"Ambient\" }\n");
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let (_, text) = h.run(&fake, Options::default());
    assert_eq!(renders(&fake), 1, "{text}");
    assert!(
        text.contains("Updated (tags): Chan/Title bbbbbbbbbbb"),
        "{text}"
    );
}

#[test]
fn a_dropped_song_is_removed_with_its_folders_and_nothing_else() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    h.run(&infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]), Options::default());
    let mine = h.lib("Chan/notes.txt");
    std::fs::write(&mine, "mine").unwrap();
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    let (ok, text) = h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    assert!(ok, "{text}");
    assert!(
        text.contains("Removed: Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.opus"),
        "{text}"
    );
    assert!(
        !h.lib("Chan/Title bbbbbbbbbbb").exists(),
        "its emptied folder goes"
    );
    assert!(
        mine.exists()
            && h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus")
                .exists()
    );
}

#[test]
fn a_song_that_fails_keeps_its_last_file() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    let gone = h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    h.run(&infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]), Options::default());
    std::fs::remove_file(gone).unwrap();
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"youtube:bbbbbbbbbbb\"]\ntags = { title = \"New\" }\n");
    let (ok, text) = h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    assert!(!ok);
    assert!(text.contains("Failed:"), "{text}");
    assert!(
        h.lib("Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.opus")
            .exists()
    );
    assert!(
        State::load(&h.dirs.home)
            .unwrap()
            .outputs
            .contains_key(Path::new("Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.opus"))
    );
}

#[test]
fn a_lost_state_deletes_nothing_it_did_not_write() {
    let h = home();
    let old = h.lib("Chan/Old [ooooooooooo].opus");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, "old").unwrap();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    let (ok, _) = h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    assert!(ok && old.exists());
}

#[test]
fn a_dry_run_writes_nothing_and_says_why() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    let unused = h.fetched("ccccccccccc");
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    let (_, text) = h.run(
        &fake,
        Options {
            dry_run: true,
            ..Options::default()
        },
    );
    assert_eq!(renders(&fake), 0);
    assert!(!h.lib("Chan").exists());
    assert!(text.contains("(new)"), "{text}");
    assert!(text.contains("audio   youtube:aaaaaaaaaaa"), "{text}");
    assert!(text.contains("TITLE"), "{text}");
    let shown = text
        .lines()
        .find_map(|l| l.strip_prefix("Unused: "))
        .map(PathBuf::from);
    assert_eq!(
        shown.as_deref(),
        Some(unused.as_path()),
        "an unused file is shown by its whole path: {text}"
    );
}

#[test]
fn a_dry_run_elsewhere_keeps_what_it_measured_and_nothing_of_the_library() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    let before = State::load(&h.dirs.home).unwrap();
    h.fetched("bbbbbbbbbbb");
    h.songs(&format!("[library]\nmax_size = \"1 GiB\"\n{TWO}"));
    let elsewhere = Dirs {
        library: h.dirs.home.join("phone"),
        ..h.dirs.clone()
    };
    let mut out = Vec::new();
    let dry = Options {
        dry_run: true,
        ..Options::default()
    };
    reconcile(
        &infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]),
        &elsewhere,
        dry,
        None,
        &mut out,
    )
    .unwrap();
    let after = State::load(&h.dirs.home).unwrap();
    assert_eq!(after.outputs, before.outputs);
    assert_eq!(after.library, before.library);
    assert!(after.facts.contains_key(&SourceKey::youtube("bbbbbbbbbbb")));
}

#[test]
fn a_dry_run_says_what_a_sync_would_remove_keep_or_write_again() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    h.run(&infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]), Options::default());
    std::fs::write(
        h.lib("Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.opus"),
        "mine",
    )
    .unwrap();
    std::fs::remove_file(h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.lrc")).unwrap();
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    let dry = Options {
        dry_run: true,
        ..Options::default()
    };
    let (_, text) = h.run(&infos(&["aaaaaaaaaaa"]), dry);
    assert!(text.contains("(written again)"), "lyrics gone: {text}");
    assert!(!text.contains("Would remove"), "{text}");
    assert!(
        text.contains("Would leave in place, changed since muman wrote it"),
        "{text}"
    );
}

/// 8 kHz samples whose loudness follows `levels`, one per 10 ms.
fn pcm(levels: &[f64]) -> Vec<u8> {
    levels
        .iter()
        .flat_map(|l| {
            #[allow(clippy::cast_possible_truncation)]
            let s = (l * 20000.0) as i16;
            (0..80).flat_map(move |i| if i % 2 == 0 { s } else { -s }.to_le_bytes())
        })
        .collect()
}

/// A loudness curve unique to its seed, smooth as music's is.
fn levels(frames: usize, seed: u64) -> Vec<f64> {
    let mut level = 0.5;
    words(frames, seed)
        .into_iter()
        .map(|w| {
            level = 0.8 * level + 0.2 * (f64::from(w) / f64::from(u32::MAX));
            level
        })
        .collect()
}

#[test]
fn the_release_wins_over_its_video_by_measure_and_lends_it_lyrics() {
    let h = home();
    h.fetched("rrrrrrrrrrr");
    h.fetched("vvvvvvvvvvv");
    h.songs("[[song]]\nsources = [\"youtube:vvvvvvvvvvv\", \"youtube:rrrrrrrrrrr\"]\n");
    let song = levels(6000, 7);
    let mut video = levels(1000, 99);
    video.extend(&song);
    let mut fake = Fake::default()
        .info("rrrrrrrrrrr", r#"{"id": "rrrrrrrrrrr", "track": "Song", "artists": ["A"], "album": "Record"}"#)
        .info("vvvvvvvvvvv", r#"{"id": "vvvvvvvvvvv", "title": "Song (MV)", "uploader": "A", "subtitles": {"en": [{"name": "English"}]}}"#);
    fake.pcm = vec![
        ("rrrrrrrrrrr".into(), pcm(&song)),
        ("vvvvvvvvvvv".into(), pcm(&video)),
    ];
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    let state = State::load(&h.dirs.home).unwrap();
    let (path, written) = state.outputs.iter().next().unwrap();
    assert_eq!(path, Path::new("A/Record/Song.opus"));
    let plan = written.plan.as_ref().unwrap();
    assert_eq!(plan.audio.key, SourceKey::youtube("rrrrrrrrrrr"));
    let lyrics = plan.lyrics.as_ref().unwrap();
    assert_eq!(lyrics.key, SourceKey::youtube("vvvvvvvvvvv"));
    assert_eq!(lyrics.shift_ms, 10_000, "the video's 10 s intro");
    assert_eq!(state.alignments.len(), 2);
}

#[test]
fn songs_that_name_the_same_path_are_kept_apart() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    let same = r#"{"title": "Same", "uploader": "Chan"}"#;
    let fake = Fake::default()
        .info("aaaaaaaaaaa", same)
        .info("bbbbbbbbbbb", same);
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(h.lib("Chan/Same/Same.opus").exists());
    assert!(
        h.lib("Chan/Same/Same [bbbbbbbbbbb].opus").exists(),
        "{text}"
    );
}

#[test]
fn manual_files_of_one_name_keep_paths_of_their_own() {
    let h = home();
    let mut body = String::new();
    for folder in ["a", "b", "c"] {
        let path = h.dirs.manual().join(folder).join("01. Song.opus");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, folder).unwrap();
        body += "[[song]]\nsources = [\"manual:";
        body += folder;
        body += "/01. Song.opus\"]\n";
    }
    h.songs(&body);
    let untagged = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "opus", "channels": 2}]}"#;
    let fake = Fake::default().probe(".opus", untagged);
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    for name in ["Untitled", "Untitled [01. Song]", "Untitled [01. Song 2]"] {
        assert!(
            h.lib(&format!("Unknown Artist/Unknown Album/{name}.opus"))
                .exists(),
            "{name}: {text}"
        );
    }

    let again = Fake::default().probe(".opus", untagged);
    let (ok, text) = h.run(&again, Options::default());
    assert!(ok, "{text}");
    assert!(text.contains("Up to date: 3 song(s)"), "{text}");
}

#[test]
fn tags_read_another_way_are_read_again_without_measuring() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    let (ok, text) = h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    assert!(ok, "{text}");
    let key = SourceKey::youtube("aaaaaaaaaaa");
    let mut state = State::load(&h.dirs.home).unwrap();
    let facts = state.facts.get_mut(&key).unwrap();
    let tags = std::mem::take(&mut facts.tags);
    facts.tags_method = String::new();
    state.save(&h.dirs.home).unwrap();

    let again = infos(&["aaaaaaaaaaa"]);
    let (ok, text) = h.run(&again, Options::default());
    assert!(ok, "{text}");
    assert!(text.contains("Reading the tags of 1 source(s)"), "{text}");
    assert!(!again.ran("s16le"), "nothing is measured again");
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
    let state = State::load(&h.dirs.home).unwrap();
    assert_eq!(state.facts[&key].tags, tags);
    assert!(state.facts[&key].tags_hold());
}

#[test]
fn a_dry_run_keeps_what_it_measured() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    let dry = Options {
        dry_run: true,
        ..Options::default()
    };
    h.run(&infos(&["aaaaaaaaaaa"]), dry);
    assert_eq!(State::load(&h.dirs.home).unwrap().facts.len(), 1);
    let again = infos(&["aaaaaaaaaaa"]);
    h.run(&again, dry);
    assert!(again.calls().is_empty(), "{:?}", again.calls());
    assert!(State::load(&h.dirs.home).unwrap().outputs.is_empty());
}

#[test]
fn a_file_changed_since_it_was_written_is_left_alone() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    h.run(&infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]), Options::default());
    let a = h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus");
    let b = h.lib("Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.opus");
    std::fs::write(&a, "retagged by hand").unwrap();
    std::fs::write(&b, "retagged by hand").unwrap();
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\ntags = { genre = \"Pop\" }\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    let (_, text) = h.run(&fake, Options::default());
    assert_eq!(renders(&fake), 0, "{text}");
    assert!(
        text.contains("Left alone, changed since muman wrote it"),
        "{text}"
    );
    assert!(
        text.contains("Left in place, changed since muman wrote it"),
        "{text}"
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "retagged by hand");
    assert!(b.exists(), "a changed file no song makes is kept");
    let state = State::load(&h.dirs.home).unwrap();
    assert!(
        state
            .outputs
            .contains_key(Path::new("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus"))
    );
    assert_eq!(state.outputs.len(), 1, "the kept file is no longer muman's");

    let fake = infos(&["aaaaaaaaaaa"]);
    let forced = Options {
        force: true,
        ..Options::default()
    };
    h.run(&fake, forced);
    assert_eq!(renders(&fake), 1);
    assert_ne!(std::fs::read(&a).unwrap(), b"retagged by hand");
}

#[test]
fn a_run_stopped_while_writing_leaves_each_file_vouched_for_or_claimed() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    h.run(&infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]), Options::default());
    h.songs(&format!(
        "[[hook]]\non = \"written\"\nrun = [\"tagger\", \"{{rel}}\"]\n{TWO}"
    ));
    let mut stopping = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    stopping.on_stream = Some(Box::new(|args: &[String]| {
        assert!(
            !args.iter().any(|a| a.contains("bbbbbbbbbbb")),
            "the run stops while writing the second song"
        );
        true
    }));
    let every_song = Options {
        force: true,
        checkpoint: Some(Duration::ZERO),
        ..Options::default()
    };
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.run(&stopping, every_song)
    }));
    assert!(stopped.is_err());

    let state = State::load(&h.dirs.home).unwrap();
    let a = Path::new("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus");
    let b = Path::new("Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.opus");
    assert!(state.outputs[a].plan.is_some(), "the first is vouched for");
    assert!(!changed_since_written(&h.dirs.library, &state.outputs, a));
    assert_eq!(
        (&state.outputs[b].plan, &state.outputs[b].stamp),
        (&None, &None),
        "the second is claimed, not taken for changed by someone else"
    );
    h.songs(TWO);
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert_eq!(renders(&fake), 1, "only the claimed song: {text}");
    assert!(!text.contains("changed since muman wrote it"), "{text}");
    assert!(
        text.contains("Written again: Chan/Title bbbbbbbbbbb"),
        "{text}"
    );
}

#[test]
fn an_empty_or_unfinished_file_is_written_again() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    let rel = PathBuf::from("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus");
    let mut state = State::load(&h.dirs.home).unwrap();
    state.outputs.get_mut(&rel).unwrap().stamp = None;
    state.save(&h.dirs.home).unwrap();
    std::fs::write(h.lib(&rel.to_string_lossy()), "").unwrap();
    let fake = infos(&["aaaaaaaaaaa"]);
    h.run(&fake, Options::default());
    assert_eq!(renders(&fake), 1, "an empty file is not current");

    state = State::load(&h.dirs.home).unwrap();
    state.outputs.get_mut(&rel).unwrap().plan = None;
    state.save(&h.dirs.home).unwrap();
    let fake = infos(&["aaaaaaaaaaa"]);
    h.run(&fake, Options::default());
    assert_eq!(
        renders(&fake),
        1,
        "a file a run was about to write is written again"
    );
}

#[test]
fn a_source_that_cannot_be_read_waits_until_it_changes() {
    let h = home();
    let path = h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    let broken = infos(&["aaaaaaaaaaa"]).probe("aaaaaaaaaaa", "not json");
    let (ok, text) = h.run(&broken, Options::default());
    assert!(!ok);
    assert!(
        text.contains("Could not read youtube:aaaaaaaaaaa"),
        "{text}"
    );
    let broken = infos(&["aaaaaaaaaaa"]).probe("aaaaaaaaaaa", "not json");
    let (_, text) = h.run(&broken, Options::default());
    assert!(text.contains("Not reading 1 source(s)"), "{text}");
    assert!(broken.calls().is_empty(), "{:?}", broken.calls());
    let retry = Options {
        retry: true,
        ..Options::default()
    };
    let broken = infos(&["aaaaaaaaaaa"]).probe("aaaaaaaaaaa", "not json");
    h.run(&broken, retry);
    assert_ne!(broken.calls(), Vec::<Vec<String>>::new());
    std::fs::write(&path, "changed").unwrap();
    let fixed = infos(&["aaaaaaaaaaa"]);
    let (ok, text) = h.run(&fixed, Options::default());
    assert!(ok, "{text}");
}

#[test]
fn a_fetch_that_failed_before_does_not_keep_a_file_now_there_from_being_read() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    let mut state = State::default();
    state.record_failure(
        &SourceKey::youtube("aaaaaaaaaaa"),
        Step::Fetch,
        None,
        "the upload did not arrive".into(),
    );
    state.save(&h.dirs.home).unwrap();
    let (ok, text) = h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    assert!(ok, "{text}");
    assert!(!text.contains("Not reading"), "{text}");
    let state = State::load(&h.dirs.home).unwrap();
    assert!(state.failures.is_empty(), "{:?}", state.failures);
}

#[test]
fn a_song_whose_chosen_source_is_gone_is_written_from_another() {
    let h = home();
    let both = "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\", \"youtube:bbbbbbbbbbb\"]\n";
    let paths = [h.fetched("aaaaaaaaaaa"), h.fetched("bbbbbbbbbbb")];
    h.songs(both);
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    let chosen = |h: &Home| {
        let state = State::load(&h.dirs.home).unwrap();
        state
            .outputs
            .values()
            .next()
            .unwrap()
            .plan
            .clone()
            .unwrap()
            .audio
            .key
    };
    let first = chosen(&h);
    let gone = usize::from(first != SourceKey::youtube("aaaaaaaaaaa"));
    std::fs::remove_file(&paths[gone]).unwrap();
    h.songs(&format!("{both}tags = {{ genre = \"Folk\" }}\n"));
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert_ne!(chosen(&h), first, "{text}");
    assert!(
        !text.contains("Comparing"),
        "nothing to compare with a file gone: {text}"
    );
}

#[test]
fn a_run_keeps_what_it_replaces_and_removes() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    h.run(&infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]), Options::default());
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\ntags = { genre = \"Pop\" }\n");
    let mut run = Run::begin(&h.dirs.home).unwrap();
    let mut out = Vec::new();
    reconcile(
        &infos(&["aaaaaaaaaaa"]),
        &h.dirs,
        Options::default(),
        Some(&mut run),
        &mut out,
    )
    .unwrap();
    run.finish(&h.dirs.home).unwrap();
    let lines = crate::history::plan(&h.dirs).unwrap().lines().join("\n");
    assert!(
        lines.contains("Title aaaaaaaaaaa.opus: put back"),
        "{lines}"
    );
    assert!(
        lines.contains("Title bbbbbbbbbbb.opus: put back"),
        "{lines}"
    );
    assert!(lines.contains("song list"), "{lines}");
}

#[test]
fn a_new_template_moves_songs_without_writing_them_again() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    let before = renders(&fake);
    h.songs(
        "[library]\ntemplate = \"{{ artist }} - {{ title }}\"\n\
         [[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n",
    );
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert_eq!(renders(&fake), before, "{text}");
    assert!(h.lib("Chan - Title aaaaaaaaaaa.opus").exists(), "{text}");
    assert!(h.lib("Chan - Title aaaaaaaaaaa.lrc").exists(), "{text}");
    assert!(!h.lib("Chan").exists(), "the emptied folders go: {text}");
    let (_, again) = h.run(&fake, Options::default());
    assert!(!again.contains("Moved"), "{again}");
}

#[test]
fn a_move_never_lands_on_a_file_another_song_holds() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    assert!(h.run(&fake, Options::default()).0);
    let theirs = h.lib("Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.opus");
    std::fs::write(&theirs, "edited by a tagger").unwrap();
    h.songs(&format!(
        "[library]\ntemplate = \"{{% if id == 'aaaaaaaaaaa' %}}Chan/Title bbbbbbbbbbb/Title \
         bbbbbbbbbbb{{% else %}}moved/{{{{ title }}}}{{% endif %}}\"\n{TWO}"
    ));
    let (_, text) = h.run(&fake, Options::default());
    assert!(!text.contains("Moved: Chan/Title aaaaaaaaaaa"), "{text}");
    assert_eq!(
        std::fs::read_to_string(&theirs).unwrap(),
        "edited by a tagger",
        "{text}"
    );
}

#[test]
fn a_rename_in_case_alone_leaves_no_old_file_behind() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    let fake = infos(&["aaaaaaaaaaa"]);
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\ntags = { title = \"lantern\" }\n");
    assert!(h.run(&fake, Options::default()).0);
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\ntags = { title = \"Lantern\" }\n");
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    let chan = h.lib("Chan");
    let mut names: Vec<String> = std::fs::read_dir(&chan)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_lowercase())
        .collect();
    names.sort();
    assert_eq!(names, ["lantern"], "{text}");
    let files = std::fs::read_dir(chan.join("Lantern")).unwrap().count();
    assert_eq!(files, 2, "the song and its lyrics, once: {text}");
}

#[test]
fn status_says_a_song_would_move() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    h.songs(
        "[library]\ntemplate = \"{{ artist }} - {{ title }}\"\n\
         [[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n",
    );
    let (_, text) = h.run(
        &fake,
        Options {
            dry_run: true,
            ..Options::default()
        },
    );
    let text = crate::ui::plain(&text);
    assert!(text.contains("Would move"), "{text}");
    assert!(text.contains("(moved)"), "{text}");
    assert!(
        h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus")
            .exists()
    );
}

/// A home with one manual FLAC whose audio takes 30 MB, and `settings`.
fn flac_home(settings: &str) -> Home {
    let h = home();
    let path = h.dirs.manual().join("Song.flac");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "flac").unwrap();
    h.songs(&format!(
        "{settings}\n[[song]]\nsources = [\"manual:Song.flac\"]\n"
    ));
    h
}

fn big_flac() -> Fake {
    Fake {
        packets: vec![("Song.flac".into(), 30_000_000)],
        ..Fake::default()
    }
    .probe(".flac", crate::testing::FLAC)
}

#[test]
fn a_song_over_the_limit_is_written_lower_and_stays_so() {
    let h = flac_home("[library]\nmax_size = \"10 MB\"\n");
    let status = h.run(
        &big_flac(),
        Options {
            dry_run: true,
            ..Options::default()
        },
    );
    assert!(
        status
            .1
            .contains("Opus 192 kbit/s to fit max_size; at best FLAC, copied"),
        "{}",
        status.1
    );
    assert!(status.1.contains("(max_size)"), "{}", status.1);
    let fake = big_flac();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(fake.ran("libopus") && fake.ran("192k"), "{text}");
    assert!(h.lib("Artist/Record/02 Song.opus").exists(), "{text}");
    assert!(!h.lib("Artist/Record/02 Song.flac").exists());

    let again = big_flac();
    let (ok, text) = h.run(&again, Options::default());
    assert!(ok, "{text}");
    assert_eq!(renders(&again), 0, "a fixed point: {text}");
    assert!(!again.ran("framecrc"), "the size is measured once");
}

#[test]
fn without_a_limit_a_lowered_song_is_written_at_its_best_again() {
    let h = flac_home("[library]\nmax_size = \"10 MB\"\n");
    h.run(&big_flac(), Options::default());
    assert!(h.lib("Artist/Record/02 Song.opus").exists());
    h.songs("[[song]]\nsources = [\"manual:Song.flac\"]\n");
    let (ok, text) = h.run(&big_flac(), Options::default());
    assert!(ok, "{text}");
    assert!(
        text.contains("Updated (format): Artist/Record/02 Song.flac"),
        "{text}"
    );
    assert!(h.lib("Artist/Record/02 Song.flac").exists(), "{text}");
    assert!(!h.lib("Artist/Record/02 Song.opus").exists(), "{text}");
}

#[test]
fn a_limit_too_small_for_any_bitrate_leaves_songs_out() {
    let h = flac_home("[library]\nmax_size = \"2 KiB\"\n");
    let (ok, text) = h.run(&big_flac(), Options::default());
    assert!(!ok);
    assert!(
        text.contains("cannot hold the library") && text.contains("left out: Song"),
        "{text}"
    );
    assert!(!h.lib("Artist/Record/02 Song.opus").exists());
    assert!(!h.lib("Artist/Record/02 Song.flac").exists());
}

/// Three manual FLACs a second long with no cover, whose audio takes 30,
/// 20 and 10 kB: estimates the size of what the fake writes.
fn three_flacs() -> Fake {
    let short = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "flac",
        "channels": 2, "sample_rate": "44100"}], "format": {"duration": "1.0",
        "tags": {"TITLE": "Song", "ARTIST": "Artist", "ALBUM": "Record", "track": "2"}}}"#;
    Fake {
        packets: vec![
            ("a.flac".into(), 30_000),
            ("b.flac".into(), 20_000),
            ("c.flac".into(), 10_000),
        ],
        ..Fake::default()
    }
    .probe(".flac", short)
}

fn limit(bytes: u64) -> String {
    format!("[library]\nmax_size = {bytes}\nblock_size = 1\n")
}

/// A home holding `names` as manual files, listing those in `listed`.
fn listing(h: &Home, settings: &str, listed: &[&str]) {
    for name in ["a", "b", "c"] {
        let path = h.dirs.manual().join(format!("{name}.flac"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, name).unwrap();
    }
    let songs = listed
        .iter()
        .map(|n| format!("[[song]]\nsources = [\"manual:{n}.flac\"]\n"))
        .collect::<Vec<_>>()
        .concat();
    h.songs(&format!("{settings}\n{songs}"));
}

/// Each library file and the format it was written in.
fn formats(h: &Home) -> Vec<(PathBuf, crate::resolve::Format)> {
    State::load(&h.dirs.home)
        .unwrap()
        .outputs
        .into_iter()
        .filter_map(|(p, w)| Some((p, w.plan?.format)))
        .collect()
}

#[test]
fn a_library_fitted_song_by_song_ends_as_one_fitted_at_once() {
    let at_once = home();
    listing(&at_once, &limit(20_000), &["a", "b", "c"]);
    let (ok, text) = at_once.run(&three_flacs(), Options::default());
    assert!(ok, "{text}");
    let lowered = formats(&at_once)
        .iter()
        .filter(|(_, f)| f.is_encoded())
        .count();
    assert!(lowered > 0, "the limit binds: {text}");

    let by_song = home();
    for listed in [&["c"][..], &["c", "a"], &["a", "b", "c"]] {
        listing(&by_song, &limit(20_000), listed);
        let (ok, text) = by_song.run(&three_flacs(), Options::default());
        assert!(ok, "{text}");
    }
    assert_eq!(formats(&by_song), formats(&at_once));
}

#[test]
fn sizes_measured_under_another_limit_change_no_choice() {
    let fresh = home();
    listing(&fresh, &limit(20_000), &["a", "b", "c"]);
    fresh.run(&three_flacs(), Options::default());
    assert!(formats(&fresh).iter().any(|(_, f)| f.is_encoded()));

    let measured = home();
    for max in [9_000, 200_000, 14_000] {
        listing(&measured, &limit(max), &["a", "b", "c"]);
        measured.run(&three_flacs(), Options::default());
    }
    listing(&measured, &limit(20_000), &["a", "b", "c"]);
    let (ok, text) = measured.run(&three_flacs(), Options::default());
    assert!(ok, "{text}");
    assert_eq!(formats(&measured), formats(&fresh));
}

#[test]
fn a_song_rendered_to_be_measured_is_moved_into_place_not_rendered_again() {
    let h = flac_home("[library]\nmax_size = \"10 MB\"\n");
    let fake = big_flac();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    let encodes = fake
        .calls()
        .iter()
        .filter(|c| c.iter().any(|a| a == "libopus"))
        .count();
    assert_eq!(encodes, 1, "{text}");
    assert!(h.lib("Artist/Record/02 Song.opus").exists());
    let state = State::load(&h.dirs.home).unwrap();
    assert_eq!(state.sizes.len(), 1, "{:?}", state.sizes);
}

#[test]
fn pruning_keeps_lyrics_a_case_blind_filesystem_takes_for_a_new_songs() {
    let h = home();
    std::fs::create_dir_all(h.lib("A")).unwrap();
    std::fs::write(h.lib("A/x.opus"), "old").unwrap();
    std::fs::write(h.lib("A/X.lrc"), "new lyrics").unwrap();
    // One file by both names: so on NTFS and APFS already, made so here.
    if !h.lib("A/x.lrc").exists() {
        std::fs::hard_link(h.lib("A/X.lrc"), h.lib("A/x.lrc")).unwrap();
    }
    let written = |lyrics: &str| Written {
        sources: Vec::new(),
        lyrics: Some(lyrics.into()),
        plan: None,
        stamp: None,
    };
    let old = BTreeMap::from([(PathBuf::from("A/x.opus"), written("A/x.lrc"))]);
    let now = BTreeMap::from([(PathBuf::from("A/X.flac"), written("A/X.lrc"))]);
    let plan = prune_plan(&h.dirs.library, &old, &now, &BTreeSet::new());
    assert_eq!(
        plan,
        [(
            PathBuf::from("A/x.opus"),
            Pruned::Removed(vec![PathBuf::from("A/x.opus")])
        )]
    );
}

#[test]
fn a_template_takes_the_first_artist_and_every_artist_as_a_list() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs(
        "[library]\ntemplate = \"{{ artist }}/{{ artists | length }}/{{ title }}\"\n\
         [[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n\
         [song.tags]\nartist = [\"Ada Quill\", \"Marlo Venn\"]\n",
    );
    let (ok, text) = h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    assert!(ok, "{text}");
    assert!(
        h.lib("Ada Quill/2/Title aaaaaaaaaaa.opus").exists(),
        "{text}"
    );
}

#[test]
fn a_name_telling_two_songs_apart_is_made_safe_as_any_tag() {
    let h = home();
    // A replacement of the user's own, as every platform allows `#` in a
    // file's name where it does not allow what the default ones replace.
    let mut body = String::from("[library]\nreplace = { \"#\" = \"-\" }\n");
    for folder in ["a", "b"] {
        let path = h.dirs.manual().join(folder).join("Song#.opus");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, folder).unwrap();
        body += "[[song]]\nsources = [\"manual:";
        body += folder;
        body += "/Song#.opus\"]\n";
    }
    h.songs(&body);
    let untagged = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "opus", "channels": 2}]}"#;
    let (ok, text) = h.run(
        &Fake::default().probe(".opus", untagged),
        Options::default(),
    );
    assert!(ok, "{text}");
    let state = State::load(&h.dirs.home).unwrap();
    let paths: Vec<String> = state
        .outputs
        .keys()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    assert!(paths.iter().any(|p| p.contains("[Song-]")), "{paths:?}");
    assert!(paths.iter().all(|p| !p.contains('#')), "{paths:?}");
}
