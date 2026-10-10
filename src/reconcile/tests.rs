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
        let text = String::from_utf8(out).unwrap();
        if !opts.dry_run {
            self.holds(&text);
        }
        (ok, text)
    }

    /// What `status` with `query` and `--all` when `all` reports.
    fn status(&self, fake: &Fake, query: &[&str], all: bool) -> String {
        let query: Vec<String> = query.iter().map(ToString::to_string).collect();
        let mut report = Vec::new();
        let opts = Options {
            dry_run: true,
            ..Options::default()
        };
        let shown = Report {
            out: &mut report,
            query: &query,
            all,
        };
        reconcile_into(
            fake,
            &self.dirs,
            opts,
            None,
            &mut std::io::sink(),
            Some(shown),
        )
        .unwrap();
        crate::ui::plain(&String::from_utf8(report).unwrap())
    }

    fn holds(&self, text: &str) {
        if let Err(e) = invariant(&self.dirs) {
            panic!("{e}: {text}");
        }
    }

    /// The library holds the files the state records and `yours`, which
    /// muman does not own, and nothing else.
    fn audit(&self, yours: &[&str], text: &str) {
        let state = State::load(&self.dirs.home).unwrap();
        let mut recorded: BTreeSet<PathBuf> = yours.iter().map(PathBuf::from).collect();
        for (path, written) in &state.outputs {
            recorded.insert(path.clone());
            recorded.extend(written.lyrics.clone());
        }
        let on_disk: BTreeSet<PathBuf> = files(&self.dirs.library)
            .into_iter()
            .map(|p| p.strip_prefix(&self.dirs.library).unwrap().to_path_buf())
            .collect();
        assert_eq!(on_disk, recorded, "{text}");
    }

    fn lib(&self, rel: &str) -> PathBuf {
        self.dirs.library.join(rel)
    }
}

/// Every file below `dir`.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut left = vec![dir.to_path_buf()];
    while let Some(d) = left.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                left.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    found
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
        .filter(|c| c.iter().any(|a| a == "opus") && !c.iter().any(|a| a == crate::ffmpeg::PIPE))
        .count()
}

const TWO: &str = "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\n";

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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { genre = \"Ambient\" }\n");
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let (_, text) = h.run(&fake, Options::default());
    assert_eq!(
        renders(&fake),
        0,
        "its tags are written into its file: {text}"
    );
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { title = \"New\" }\n");
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let (ok, _) = h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    assert!(ok && old.exists());
}

#[test]
fn a_dry_run_writes_nothing_and_says_why() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    let unused = h.fetched("ccccccccccc");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
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
    assert!(text.contains("audio   youtube.com:aaaaaaaaaaa"), "{text}");
    assert!(text.contains("TITLE"), "{text}");
    let shown = crate::ui::plain(&text)
        .lines()
        .find_map(|l| l.strip_prefix("Unused: ").map(PathBuf::from));
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
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
    h.songs("[[song]]\nsources = [\"youtube.com:vvvvvvvvvvv\", \"youtube.com:rrrrrrrrrrr\"]\n");
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { genre = \"Pop\" }\n");
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    let rel = PathBuf::from("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus");
    std::fs::write(h.lib(&rel.to_string_lossy()), "").unwrap();
    let fake = infos(&["aaaaaaaaaaa"]);
    let (_, text) = h.run(&fake, Options::default());
    assert_eq!(renders(&fake), 1, "emptied since written: {text}");

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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let broken = infos(&["aaaaaaaaaaa"]).probe("aaaaaaaaaaa", "not json");
    let (ok, text) = h.run(&broken, Options::default());
    assert!(!ok);
    assert!(
        text.contains("Could not read youtube.com:aaaaaaaaaaa"),
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
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
    let both = "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\", \"youtube.com:bbbbbbbbbbb\"]\n";
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { genre = \"Pop\" }\n");
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    let before = renders(&fake);
    h.songs(
        "[library]\ntemplate = \"{{ artist }} - {{ title }}\"\n\
         [[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n",
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
fn a_new_template_moves_a_file_changed_since_it_was_written() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    let before = renders(&fake);
    std::fs::write(
        h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus"),
        "replay gain",
    )
    .unwrap();
    h.songs(
        "[library]\ntemplate = \"{{ artist }} - {{ title }}\"\n\
         [[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n",
    );
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert_eq!(renders(&fake), before, "{text}");
    let moved = h.lib("Chan - Title aaaaaaaaaaa.opus");
    assert_eq!(std::fs::read_to_string(&moved).unwrap(), "replay gain");
    assert!(!h.lib("Chan").exists(), "no copy is left behind: {text}");
}

#[test]
fn a_changed_file_whose_song_changes_and_moves_waits_for_force() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    let before = renders(&fake);
    let old = h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus");
    std::fs::write(&old, "replay gain").unwrap();
    h.songs(
        "[library]\ntemplate = \"{{ artist }} - {{ title }}\"\n\
         [[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { genre = \"Pop\" }\n",
    );
    let (_, text) = h.run(&fake, Options::default());
    assert_eq!(renders(&fake), before, "{text}");
    assert!(
        text.contains("Left alone, changed since muman wrote it: Chan/Title aaaaaaaaaaa/"),
        "{text}"
    );
    assert!(old.exists(), "{text}");
    assert!(!h.lib("Chan - Title aaaaaaaaaaa.opus").exists(), "{text}");

    let forced = Options {
        force: true,
        ..Options::default()
    };
    let (ok, text) = h.run(&fake, forced);
    assert!(ok, "{text}");
    assert!(h.lib("Chan - Title aaaaaaaaaaa.opus").exists(), "{text}");
    assert!(
        !old.exists(),
        "written again elsewhere, the old file goes: {text}"
    );
}

#[test]
fn a_move_lands_where_another_song_leaves_and_never_on_its_file() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    assert!(h.run(&fake, Options::default()).0);
    let (a, b) = (
        "Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus",
        "Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.opus",
    );
    std::fs::write(h.lib(b), "edited by a tagger").unwrap();
    let written = std::fs::read(h.lib(a)).unwrap();
    h.songs(&format!(
        "[library]\ntemplate = \"{{% if id == 'aaaaaaaaaaa' %}}Chan/Title bbbbbbbbbbb/Title \
         bbbbbbbbbbb{{% else %}}moved/{{{{ title }}}}{{% endif %}}\"\n{TWO}"
    ));
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert_eq!(
        std::fs::read_to_string(h.lib("moved/Title bbbbbbbbbbb.opus")).unwrap(),
        "edited by a tagger",
        "the edited file moves first: {text}"
    );
    assert_eq!(std::fs::read(h.lib(b)).unwrap(), written, "{text}");
    assert!(!text.contains("Wrote"), "{text}");
    // Two songs trading paths have nowhere to go first.
    h.songs(&format!(
        "[library]\ntemplate = \"{{% if id == 'aaaaaaaaaaa' %}}moved/Title bbbbbbbbbbb\
         {{% else %}}Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb{{% endif %}}\"\n{TWO}"
    ));
    let (_, text) = h.run(&fake, Options::default());
    assert!(!text.contains("Moved"), "{text}");
    assert_eq!(
        std::fs::read_to_string(h.lib("moved/Title bbbbbbbbbbb.opus")).unwrap(),
        "edited by a tagger",
        "{text}"
    );
}

#[test]
fn a_song_that_loses_its_told_apart_name_moves_to_the_plain_one() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let same = "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { title = \"Same\" }\n\
                [[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { title = \"Same\" }\n";
    h.songs(same);
    assert!(h.run(&fake, Options::default()).0);
    let before = files(&h.lib(""));
    h.songs(&same.replacen("Same", "Other", 1));
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(
        !text.contains("Wrote"),
        "both move, neither is written: {text}"
    );
    let after = files(&h.lib(""));
    assert_eq!(after.len(), before.len(), "{before:?} → {after:?}");
}

#[test]
fn a_rename_in_case_alone_leaves_no_old_file_behind() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    let fake = infos(&["aaaaaaaaaaa"]);
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { title = \"lantern\" }\n");
    assert!(h.run(&fake, Options::default()).0);
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { title = \"Lantern\" }\n");
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
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    h.songs(
        "[library]\ntemplate = \"{{ artist }} - {{ title }}\"\n\
         [[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n",
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
        digest: None,
    };
    let old = BTreeMap::from([(PathBuf::from("A/x.opus"), written("A/x.lrc"))]);
    let now = BTreeMap::from([(PathBuf::from("A/X.flac"), written("A/X.lrc"))]);
    let plan = prune_plan(&h.dirs.library, &old, &now, &BTreeSet::new(), false);
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
         [[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n\
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

/// What happens to the library between two syncs.
#[derive(Debug, Clone, Copy)]
enum Disturbance {
    Nothing,
    /// A copy or a restore set every file's time.
    Touched,
    /// A tagger rewrote one file.
    Retagged,
    /// A file was deleted.
    Deleted,
    /// A file of the user's own sits where a song is going.
    Theirs,
}

/// What the song list changes between the two syncs.
#[derive(Debug, Clone, Copy)]
enum Change {
    Nothing,
    Template,
    Tag,
    Codec,
    TemplateAndTag,
}

const PAPER: &str = "Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus";

fn disturb(h: &Home, d: Disturbance) -> Vec<&'static str> {
    let song = h.lib(PAPER);
    match d {
        Disturbance::Nothing => Vec::new(),
        Disturbance::Touched => {
            for file in files(&h.dirs.library) {
                let later = std::time::SystemTime::now() + Duration::from_secs(5);
                std::fs::File::options()
                    .write(true)
                    .open(&file)
                    .unwrap()
                    .set_modified(later)
                    .unwrap();
            }
            Vec::new()
        }
        Disturbance::Retagged => {
            std::fs::write(&song, "replay gain").unwrap();
            Vec::new()
        }
        Disturbance::Deleted => {
            std::fs::remove_file(&song).unwrap();
            Vec::new()
        }
        Disturbance::Theirs => {
            let theirs = h.lib("Chan - Title aaaaaaaaaaa.opus");
            std::fs::write(theirs, "mine").unwrap();
            vec!["Chan - Title aaaaaaaaaaa.opus"]
        }
    }
}

fn changed(c: Change) -> String {
    let template = "[library]\ntemplate = \"{{ artist }} - {{ title }}\"\n";
    let tagged = "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags.genre = \"Folk\"\n\
                  [[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\n";
    match c {
        Change::Nothing => TWO.to_string(),
        Change::Template => format!("{template}{TWO}"),
        Change::Tag => tagged.to_string(),
        Change::Codec => format!("[audio]\ncodecs = [\"flac\"]\nlossy = \"vorbis\"\n{TWO}"),
        Change::TemplateAndTag => format!("{template}{tagged}"),
    }
}

#[test]
fn no_disturbance_and_no_change_leaves_a_second_copy_or_a_missing_one() {
    use Change as C;
    use Disturbance as D;
    let disturbances = [D::Nothing, D::Touched, D::Retagged, D::Deleted, D::Theirs];
    let changes = [C::Nothing, C::Template, C::Tag, C::Codec, C::TemplateAndTag];
    for d in disturbances {
        for c in changes {
            let h = home();
            h.fetched("aaaaaaaaaaa");
            h.fetched("bbbbbbbbbbb");
            h.songs(TWO);
            let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
            assert!(h.run(&fake, Options::default()).0);
            let yours = disturb(&h, d);
            h.songs(&changed(c));
            let (_, text) = h.run(&fake, Options::default());
            let case = format!("{d:?} then {c:?}: {text}");
            h.audit(&yours, &case);
            let before = fake.calls().len();
            let (_, again) = h.run(&fake, Options::default());
            h.audit(&yours, &format!("{case}\nagain: {again}"));
            let rendered = fake.calls()[before..]
                .iter()
                .any(|c| c.iter().any(|a| a == "opus" || a == "ogg"));
            assert!(
                !rendered,
                "the next sync writes nothing: {case}\nagain: {again}"
            );
        }
    }
}

#[test]
fn a_source_whose_file_times_change_writes_nothing_again() {
    let h = home();
    let source = h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    let song = h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus");
    let written = std::fs::metadata(&song).unwrap().modified().unwrap();
    filetime::set_file_mtime(&source, filetime::FileTime::from_unix_time(1_000_000, 0)).unwrap();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(text.contains("Measuring 1 source(s)"), "{text}");
    assert!(
        !text.contains("Added") && !text.contains("Updated"),
        "{text}"
    );
    assert_eq!(
        std::fs::metadata(&song).unwrap().modified().unwrap(),
        written
    );
}

/// `state` as muman wrote it before it hashed sources: no digests, and
/// every plan naming its sources by their files' revisions.
fn unhashed(mut state: State) -> State {
    for f in state.facts.values_mut() {
        f.hash_method.clear();
        f.tags_digest = None;
        f.served = None;
        if let Some(a) = &mut f.audio {
            a.digest = None;
        }
        f.covers.iter_mut().for_each(|c| c.digest = None);
        f.lyrics.iter_mut().for_each(|l| l.digest = None);
    }
    let rev = |facts: &BTreeMap<SourceKey, Facts>, k: &SourceKey| facts[k].rev.clone();
    let tools = "ffmpeg version fake";
    let mut sizes = BTreeMap::new();
    for w in state.outputs.values_mut() {
        let plan = w.plan.as_mut().unwrap();
        let named = limit::plan_key(tools, plan);
        plan.audio.rev = rev(&state.facts, &plan.audio.key);
        if let Some(c) = &mut plan.cover {
            c.rev = rev(&state.facts, &c.key);
        }
        if let Some(l) = &mut plan.lyrics {
            l.rev = rev(&state.facts, &l.key);
        }
        if let Some(m) = state.sizes.get(&named) {
            sizes.insert(limit::plan_key(tools, plan), m.clone());
        }
    }
    state.sizes = sizes;
    state
}

#[test]
fn sources_measured_before_hashing_are_hashed_and_write_nothing_again() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    let now = State::load(&h.dirs.home).unwrap();
    let key = SourceKey::youtube("aaaaaaaaaaa");
    let digest = now.facts[&key].audio_rev();
    assert!(!digest.contains(':'), "named by its content: {digest}");
    unhashed(now).save(&h.dirs.home).unwrap();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(text.contains("Hashing 1 source(s)"), "{text}");
    assert!(!text.contains("Measuring"), "nothing decoded: {text}");
    assert!(
        !text.contains("Added") && !text.contains("Updated"),
        "{text}"
    );
    let after = State::load(&h.dirs.home).unwrap();
    assert_eq!(after.facts[&key].audio_rev(), digest);
    let plan = after
        .outputs
        .values()
        .next()
        .unwrap()
        .plan
        .as_ref()
        .unwrap();
    assert_eq!(plan.audio.rev, digest);
    assert_ne!(plan.cover.as_ref().unwrap().rev, after.facts[&key].rev);
}

#[test]
fn a_size_measured_before_hashing_is_not_measured_again() {
    let h = flac_home("[library]\nmax_size = \"10 MB\"\n");
    assert!(h.run(&big_flac(), Options::default()).0);
    let state = State::load(&h.dirs.home).unwrap();
    assert!(!state.sizes.is_empty(), "a real size is kept");
    let state_sizes = state.sizes.clone();
    unhashed(state).save(&h.dirs.home).unwrap();
    let fake = big_flac();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(text.contains("Hashing 1 source(s)"), "{text}");
    assert_eq!(renders(&fake), 0, "{text}");
    let kept = State::load(&h.dirs.home).unwrap().sizes;
    assert_eq!(
        kept, state_sizes,
        "kept under the names plans have now: {text}"
    );
}

/// The `held` line the song list holds for `key`, if any.
fn held_line(h: &Home, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(h.dirs.manifest()).unwrap();
    let start = format!("held.\"{key}\" = ");
    text.lines()
        .find(|l| l.starts_with(&start))
        .map(str::to_string)
}

const ONE: &str = "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n";
const FORMAT: &str = r#"{"id": "aaaaaaaaaaa", "title": "Title aaaaaaaaaaa", "uploader": "Chan",
    "format_id": "399+251", "formats": [
        {"format_id": "251", "filesize": 3456789, "acodec": "opus", "vcodec": "none"},
        {"format_id": "399", "filesize": 9999999, "acodec": "none", "vcodec": "av01"}]}"#;

#[test]
fn a_sync_records_what_each_source_held_and_the_format_served() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs(ONE);
    let fake = Fake::default().info("aaaaaaaaaaa", FORMAT);
    assert!(h.run(&fake, Options::default()).0);
    let line = held_line(&h, "youtube.com:aaaaaaaaaaa").expect("recorded");
    assert!(
        line.contains("format = \"399+251\", size = 3456789 }"),
        "{line}"
    );
    let (_, text) = h.run(&fake, Options::default());
    assert_eq!(
        held_line(&h, "youtube.com:aaaaaaaaaaa").unwrap(),
        line,
        "{text}"
    );
}

#[test]
fn a_source_fetched_again_with_other_audio_leaves_its_song_as_built_until_accepted() {
    let h = home();
    let source = h.fetched("aaaaaaaaaaa");
    h.songs(ONE);
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    let song = h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus");
    let built = std::fs::read(&song).unwrap();
    let recorded = held_line(&h, "youtube.com:aaaaaaaaaaa").unwrap();
    std::fs::write(&source, "re-encoded by the site").unwrap();

    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    let plain = crate::ui::plain(&text);
    assert!(
        plain.contains("youtube.com:aaaaaaaaaaa has changed since its song was built: other audio"),
        "{plain}"
    );
    assert!(plain.contains("left as built; `sync --accept`"), "{plain}");
    assert!(!plain.contains("Updated"), "{plain}");
    assert_eq!(std::fs::read(&song).unwrap(), built);
    assert_eq!(held_line(&h, "youtube.com:aaaaaaaaaaa").unwrap(), recorded);
    let (_, again) = h.run(&fake, Options::default());
    assert!(
        again.contains("left as built"),
        "said until accepted: {again}"
    );

    let (ok, text) = h.run(
        &fake,
        Options {
            accept: true,
            ..Options::default()
        },
    );
    assert!(ok, "{text}");
    assert!(
        crate::ui::plain(&text).contains("taken, as `--accept` asks"),
        "{text}"
    );
    assert!(text.contains("Updated"), "{text}");
    assert_ne!(held_line(&h, "youtube.com:aaaaaaaaaaa").unwrap(), recorded);
    let (_, after) = h.run(&fake, Options::default());
    assert!(!after.contains("has changed"), "{after}");
}

#[test]
fn a_song_with_no_file_left_is_built_from_what_its_source_holds_and_says_so() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs(&format!(
        "{ONE}held.\"youtube.com:aaaaaaaaaaa\" = {{ audio = \"0123456789abcdef\" }}\n"
    ));
    let fake = infos(&["aaaaaaaaaaa"]);
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    let plain = crate::ui::plain(&text);
    assert!(plain.contains("no file built before being left"), "{plain}");
    assert!(
        h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus")
            .exists()
    );
    assert!(
        held_line(&h, "youtube.com:aaaaaaaaaaa")
            .unwrap()
            .contains("0123456789abcdef"),
        "kept until accepted"
    );
}

#[test]
fn a_file_of_your_own_holding_other_audio_is_followed_and_recorded() {
    let h = flac_home("");
    let fake = || Fake::default().probe(".flac", crate::testing::FLAC);
    assert!(h.run(&fake(), Options::default()).0);
    let recorded = held_line(&h, "manual:Song.flac").unwrap();
    let path = h.dirs.manual().join("Song.flac");
    std::fs::write(&path, "a better rip").unwrap();
    let (ok, text) = h.run(
        &fake(),
        Options {
            settling: Duration::ZERO,
            ..Options::default()
        },
    );
    assert!(ok, "{text}");
    assert!(
        crate::ui::plain(&text).contains("its song follows it, a file of your own"),
        "{text}"
    );
    assert!(text.contains("Updated"), "{text}");
    assert_ne!(held_line(&h, "manual:Song.flac").unwrap(), recorded);
}

#[test]
fn a_source_fetched_again_with_other_tags_alone_leaves_its_song_as_built() {
    let h = home();
    let source = h.fetched("aaaaaaaaaaa");
    h.songs(ONE);
    let titled = |title: &str| Fake {
        hashes: vec![("aaaaaaaaaaa".into(), 7)],
        ..Fake::default().info(
            "aaaaaaaaaaa",
            &format!(r#"{{"id": "aaaaaaaaaaa", "title": "{title}", "uploader": "Chan"}}"#),
        )
    };
    assert!(h.run(&titled("Lantern Weather"), Options::default()).0);
    let line = held_line(&h, "youtube.com:aaaaaaaaaaa").unwrap();
    for part in ["audio = ", "cover = ", "lyrics = ", "tags = "] {
        assert!(line.contains(part), "{line}");
    }
    let song = h.lib("Chan/Lantern Weather/Lantern Weather.opus");
    let built = std::fs::read(&song).unwrap();
    std::fs::write(&source, "fetched again").unwrap();

    let (ok, text) = h.run(&titled("Paper Comets"), Options::default());
    assert!(ok, "{text}");
    let plain = crate::ui::plain(&text);
    assert!(
        plain.contains("youtube.com:aaaaaaaaaaa has changed since its song was built: other tags;"),
        "only the tags: {plain}"
    );
    assert_eq!(std::fs::read(&song).unwrap(), built, "{plain}");
    assert_eq!(held_line(&h, "youtube.com:aaaaaaaaaaa").unwrap(), line);

    let (ok, text) = h.run(
        &titled("Paper Comets"),
        Options {
            accept: true,
            ..Options::default()
        },
    );
    assert!(ok, "{text}");
    assert!(
        h.lib("Chan/Paper Comets/Paper Comets.opus").exists(),
        "{text}"
    );
    assert_ne!(held_line(&h, "youtube.com:aaaaaaaaaaa").unwrap(), line);
}

#[test]
fn status_counts_the_songs_up_to_date_and_shows_those_a_sync_changes() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(ONE);
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    assert!(h.run(&fake, Options::default()).0);
    h.songs(TWO);
    let text = h.status(&fake, &[], false);
    assert!(!text.contains("Title aaaaaaaaaaa"), "{text}");
    assert!(text.contains("Title bbbbbbbbbbb.opus (new)"), "{text}");
    assert!(
        text.contains("Songs: 1 new, 1 up to date; `--all` or a query shows those up to date"),
        "{text}"
    );
    let all = h.status(&fake, &[], true);
    assert!(all.contains("Title aaaaaaaaaaa.opus (up to date)"), "{all}");
    assert!(all.contains("Songs: 1 new, 1 up to date\n"), "{all}");
}

#[test]
fn status_shows_each_song_a_query_matches_and_nothing_of_the_others() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.fetched("ccccccccccc");
    h.songs(ONE);
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc"]);
    assert!(h.run(&fake, Options::default()).0);
    h.songs(TWO);
    let text = h.status(&fake, &["title:aaaaaaaaaaa"], false);
    assert!(
        text.contains("Title aaaaaaaaaaa.opus (up to date)"),
        "{text}"
    );
    assert!(text.contains("audio   youtube.com:aaaaaaaaaaa"), "{text}");
    assert!(!text.contains("bbbbbbbbbbb"), "{text}");
    assert!(!text.contains("Unused"), "files are no song: {text}");
    assert!(text.ends_with("Songs: 1 up to date\n"), "{text}");
    let none = h.status(&fake, &["title:zzz"], false);
    assert_eq!(none, "No song matches the query\n");
}

#[test]
fn status_refuses_a_field_no_song_has() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs(ONE);
    let query = vec!["colour:red".to_string()];
    let shown = Report {
        out: &mut Vec::new(),
        query: &query,
        all: false,
    };
    let opts = Options {
        dry_run: true,
        ..Options::default()
    };
    let e = reconcile_into(
        &infos(&["aaaaaaaaaaa"]),
        &h.dirs,
        opts,
        None,
        &mut std::io::sink(),
        Some(shown),
    )
    .unwrap_err();
    assert!(
        format!("{e:#}").contains("No song has a field `colour`"),
        "{e:#}"
    );
}

/// `state` as a muman that keyed sources by yt-dlp's extractor wrote
/// it, each of `sites` named `<extractor>:<id>`, its sizes kept by the
/// plans so named.
fn by_extractor(mut state: State, sites: &[(&str, &str)]) -> String {
    let tools = "ffmpeg version fake";
    let old = |k: &SourceKey| {
        let site = k.site()?;
        let (_, extractor) = sites.iter().find(|(s, _)| *s == site)?;
        SourceKey::parse(&format!("{extractor}:{}", k.id()?)).ok()
    };
    let mut kept = BTreeMap::new();
    for w in state.outputs.values() {
        let plan = w.plan.as_ref().unwrap();
        if let Some(m) = state.sizes.get(&limit::plan_key(tools, plan)) {
            kept.insert(limit::plan_key(tools, &plan.respelled(&old)), m.clone());
        }
    }
    state.sizes = kept;
    state.version = 1;
    let mut text = serde_json::to_string_pretty(&state).unwrap();
    for (site, extractor) in sites {
        text = text.replace(&format!("\"{site}:"), &format!("\"{extractor}:"));
    }
    text
}

/// A fetched FLAC whose audio takes 30 MB.
fn big_fetched() -> Fake {
    Fake {
        packets: vec![("aaaaaaaaaaa".into(), 30_000_000)],
        ..infos(&["aaaaaaaaaaa"])
    }
    .probe(".mkv", crate::testing::FLAC)
}

#[test]
fn a_home_keyed_by_extractor_is_keyed_by_site_and_writes_no_song_again() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs(&format!("[library]\nmax_size = \"100 kB\"\n{ONE}"));
    let (ok, text) = h.run(&big_fetched(), Options::default());
    assert!(
        ok && text.contains("1 song(s) below their best format"),
        "{text}"
    );
    let state = State::load(&h.dirs.home).unwrap();
    assert!(!state.sizes.is_empty(), "a real size is kept");
    let (sizes, outputs) = (state.sizes.clone(), state.outputs.clone());
    let recorded = held_line(&h, "youtube.com:aaaaaaaaaaa").unwrap();
    let songs = std::fs::read_to_string(h.dirs.manifest()).unwrap();
    std::fs::write(h.dirs.manifest(), songs.replace("youtube.com:", "youtube:")).unwrap();
    let old = by_extractor(state, &[("youtube.com", "youtube")]);
    std::fs::write(h.dirs.home.join(crate::dirs::STATE), old).unwrap();

    let fake = big_fetched();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert_eq!(renders(&fake), 0, "{text}");
    assert!(
        !text.contains("Added") && !text.contains("Updated"),
        "{text}"
    );
    let after = State::load(&h.dirs.home).unwrap();
    assert_eq!(after.version, crate::state::VERSION);
    assert_eq!(after.outputs, outputs);
    assert_eq!(after.sizes, sizes, "kept under the names plans have now");
    let songs = std::fs::read_to_string(h.dirs.manifest()).unwrap();
    assert!(!songs.contains("\"youtube:"), "{songs}");
    assert_eq!(held_line(&h, "youtube.com:aaaaaaaaaaa").unwrap(), recorded);
}

#[test]
fn a_source_of_a_site_no_table_names_is_renamed_by_its_page() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"tunes.example:aaaaaaaaaaa\"]\n");
    let fake = || {
        Fake::default().info(
            "aaaaaaaaaaa",
            r#"{"id": "aaaaaaaaaaa", "title": "Title aaaaaaaaaaa", "uploader": "Chan",
                "format_id": "mp3", "webpage_url": "https://www.tunes.example/t/aaaaaaaaaaa"}"#,
        )
    };
    assert!(h.run(&fake(), Options::default()).0);
    let recorded = held_line(&h, "tunes.example:aaaaaaaaaaa").unwrap();
    assert!(
        recorded.contains("url = \"https://www.tunes.example/t/aaaaaaaaaaa\""),
        "{recorded}"
    );
    let outputs = State::load(&h.dirs.home).unwrap().outputs;
    let songs = std::fs::read_to_string(h.dirs.manifest()).unwrap();
    std::fs::write(
        h.dirs.manifest(),
        songs.replace("tunes.example:", "funkwhale:"),
    )
    .unwrap();
    let old = by_extractor(
        State::load(&h.dirs.home).unwrap(),
        &[("tunes.example", "funkwhale")],
    );
    std::fs::write(h.dirs.home.join(crate::dirs::STATE), old).unwrap();

    let dry = h.status(&fake(), &[], false);
    assert!(dry.contains("Songs: 1 up to date"), "{dry}");
    let fake = fake();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(
        text.contains(
            "Renamed by the site it came from: funkwhale:aaaaaaaaaaa → tunes.example:aaaaaaaaaaa"
        ),
        "{text}"
    );
    assert_eq!(renders(&fake), 0, "{text}");
    assert!(
        !text.contains("Added") && !text.contains("Updated"),
        "{text}"
    );
    assert_eq!(State::load(&h.dirs.home).unwrap().outputs, outputs);
    assert_eq!(
        held_line(&h, "tunes.example:aaaaaaaaaaa").unwrap(),
        recorded
    );
    let (_, again) = h.run(&infos(&[]), Options::default());
    assert!(!again.contains("Renamed"), "{again}");
}

#[test]
fn a_library_that_is_a_file_is_refused_before_the_old_one_is_forgotten() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lib");
    std::fs::write(&file, "not a folder").unwrap();
    let mut state = State {
        library: Some(dir.path().join("music")),
        ..State::default()
    };
    state.outputs.insert(
        "A/x.flac".into(),
        Written {
            sources: vec![],
            lyrics: None,
            plan: None,
            stamp: None,
            digest: None,
        },
    );
    assert!(adopt(&mut state, &file, &mut Vec::new()).is_err());
    assert_eq!(state.outputs.len(), 1);
    assert_eq!(state.library, Some(dir.path().join("music")));
}

#[test]
fn a_full_disk_is_told_from_a_source_that_does_not_read() {
    let full = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::StorageFull));
    assert!(out_of_space(&full.context("creating facts-1")));
    assert!(out_of_space(&anyhow::anyhow!(
        "ffmpeg failed (exit 1): Error writing trailer: No space left on device"
    )));
    assert!(!out_of_space(&anyhow::anyhow!(
        "ffprobe failed (exit 1): Invalid data"
    )));
}

/// Two seconds of a tone at 8 kHz, `amplitude` of full scale, as a fake
/// analysis decodes it.
fn tone(amplitude: f32) -> Vec<u8> {
    crate::testing::wav(
        8000,
        2,
        Some(0x3),
        &crate::testing::sine(8000, 2, 1000.0, amplitude, 2.0),
    )
}

fn tag<'a>(plan: &'a Plan, key: &str) -> Option<&'a str> {
    plan.tags
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.first())
        .map(String::as_str)
}

const ALBUM: &str = "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { album = \"Harbor Lights\", albumartist = \"Paper Comets\" }\n[[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { album = \"Harbor Lights\", albumartist = \"Paper Comets\" }\n";

fn levelled() -> Fake {
    let mut fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    fake.wavs = vec![
        ("aaaaaaaaaaa".into(), tone(0.1)),
        ("bbbbbbbbbbb".into(), tone(0.4)),
    ];
    fake
}

fn plans(h: &Home) -> Vec<Plan> {
    State::load(&h.dirs.home)
        .unwrap()
        .outputs
        .into_values()
        .filter_map(|w| w.plan)
        .collect()
}

#[test]
fn songs_are_levelled_by_what_they_measured_and_an_album_by_all_its_songs() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(ALBUM);
    let fake = levelled();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    let plans = plans(&h);
    let tracks: BTreeSet<&str> = plans
        .iter()
        .filter_map(|p| tag(p, "REPLAYGAIN_TRACK_GAIN"))
        .collect();
    let albums: BTreeSet<&str> = plans
        .iter()
        .filter_map(|p| tag(p, "REPLAYGAIN_ALBUM_GAIN"))
        .collect();
    assert_eq!(tracks.len(), 2, "{plans:?}");
    assert_eq!(albums.len(), 1, "{plans:?}");
    assert!(plans.iter().all(|p| {
        p.loudness
            .is_some_and(|g| g.apply == crate::loudness::Apply::Tags)
    }));
    let status = h.status(&fake, &[], true);
    assert!(
        status.contains("measured over its album's 2 song(s)"),
        "{status}"
    );
    let (_, again) = h.run(&fake, Options::default());
    assert!(again.contains("Up to date: 2"), "{again}");
}

#[test]
fn a_gain_put_in_the_opus_header_is_a_retag_and_a_gain_in_the_samples_an_encode() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(ALBUM);
    let fake = levelled();
    h.run(&fake, Options::default());
    let written = renders(&fake);
    h.songs(&format!("[loudness]\nmode = \"header\"\n{ALBUM}"));
    let (_, text) = h.run(&fake, Options::default());
    assert_eq!(renders(&fake), written, "{text}");
    assert!(text.contains("(loudness)"), "{text}");
    let gains: Vec<i16> = files(&h.dirs.library)
        .iter()
        .filter_map(|f| crate::ogg::output_gain(&std::fs::read(f).unwrap()))
        .collect();
    assert_eq!(gains.len(), 2);
    assert!(
        gains.iter().all(|&g| g != 0 && g == gains[0]),
        "one album, one gain: {gains:?}"
    );
    h.songs(&format!("[loudness]\nmode = \"audio\"\n{ALBUM}"));
    let (_, text) = h.run(&fake, Options::default());
    assert_eq!(renders(&fake), written + 2, "{text}");
    assert!(
        fake.calls()
            .iter()
            .any(|c| c.iter().any(|a| a.contains("volume=")))
    );
    assert!(
        plans(&h)
            .iter()
            .all(|p| p.format.is_encoded() && p.chain().gain != 0)
    );
}

#[test]
fn loudness_off_measures_nothing_and_keeps_what_sources_carry() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[loudness]\nmode = \"off\"\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(!fake.ran(crate::ffmpeg::PIPE));
    assert!(
        plans(&h)
            .iter()
            .all(|p| p.loudness.is_none() && tag(p, "REPLAYGAIN_TRACK_GAIN").is_none())
    );
}

#[test]
fn facts_measured_before_loudness_are_analyzed_alone() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[loudness]\nmode = \"off\"\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    h.run(&fake, Options::default());
    let before = fake.calls().len();
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let (_, text) = h.run(&fake, Options::default());
    assert!(text.contains("Analyzing 1 source(s)"), "{text}");
    let calls = fake.calls();
    let new = &calls[before..];
    assert_eq!(
        new.iter()
            .filter(|c| c.iter().any(|a| a == crate::ffmpeg::PIPE))
            .count(),
        1
    );
    assert!(
        !new.iter().any(|c| c.iter().any(|a| a == "s16le")),
        "no print again"
    );
    assert!(
        plans(&h)
            .iter()
            .all(|p| tag(p, "REPLAYGAIN_TRACK_GAIN").is_some())
    );
}

#[test]
fn a_song_mixed_is_levelled_by_its_mix() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[audio]\nlayouts = [\"stereo\"]\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let surround = crate::testing::ORIGINAL.replace(
        r#""channels": 2, "sample_rate": "48000""#,
        r#""channels": 6, "channel_layout": "5.1", "sample_rate": "48000""#,
    );
    let mut fake = infos(&["aaaaaaaaaaa"]).probe("aaaaaaaaaaa", &surround);
    fake.wavs = vec![("aaaaaaaaaaa".into(), tone(0.4))];
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(fake.calls().iter().any(|c| {
        c.iter().any(|a| a == crate::ffmpeg::PIPE)
            && c.iter().any(|a| a.starts_with("aresample=ochl=stereo"))
    }));
    let state = State::load(&h.dirs.home).unwrap();
    assert_eq!(state.mixes.len(), 1);
    let plans = plans(&h);
    assert!(plans[0].format.mix().is_some());
    assert!(
        tag(&plans[0], "REPLAYGAIN_TRACK_GAIN").is_some(),
        "{plans:?}"
    );
}
