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
    h.fetched("ccccccccccc");
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
    assert!(
        text.contains("Unused:") && text.contains("[ccccccccccc]"),
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
    assert!(!again.ran("chromaprint"), "nothing is measured again");
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
    assert!(!broken.calls().is_empty());
    std::fs::write(&path, "changed").unwrap();
    let fixed = infos(&["aaaaaaaaaaa"]);
    let (ok, text) = h.run(&fixed, Options::default());
    assert!(ok, "{text}");
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
    let lines = crate::history::describe(&h.dirs.home).unwrap().join("\n");
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
