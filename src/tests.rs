use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use clap::Parser;

use super::*;
use crate::lrclib::testing::Server;
use crate::source::SourceKey;
use crate::testing::{FLAC, Fake, words};

fn cli(args: &[&str]) -> Cli {
    Cli::try_parse_from(std::iter::once("muman").chain(args.iter().copied())).unwrap()
}

fn defaults(root: &Path) -> Defaults {
    Defaults {
        home: root.join("data"),
        library: root.join("music"),
        cache: None,
    }
}

#[test]
fn folders_default_to_the_platform_and_flags_override_them() {
    let root = Path::new("root");
    let job = Job::from_cli(cli(&["status"]), defaults(root));
    assert_eq!(job.dirs.home, root.join("data"));
    assert_eq!(job.dirs.library, root.join("music"));
    let job = Job::from_cli(
        cli(&["--home", "h", "--library", "l", "status"]),
        defaults(root),
    );
    assert_eq!(job.dirs.home, PathBuf::from("h"));
    assert_eq!(job.dirs.library, PathBuf::from("l"));
}

struct Setup {
    dir: tempfile::TempDir,
}

impl Setup {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn job(&self, args: &[&str]) -> Job {
        let mut job = Job::from_cli(cli(args), defaults(self.dir.path()));
        job.settling = std::time::Duration::ZERO;
        job.throttles = crate::lookup::Throttles::none();
        job.dirs = Dirs {
            home: self.dir.path().join("home"),
            library: self.dir.path().join("lib"),
            locks: Some(self.dir.path().to_path_buf()),
        };
        job
    }

    /// A file written a minute ago, long enough to be done copying in.
    fn file(&self, rel: &str) -> String {
        let path = self.dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, rel.as_bytes()).unwrap();
        let earlier = SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(earlier)
            .unwrap();
        path.to_string_lossy().into_owned()
    }

    /// A file holding `text`, written as [`Setup::file`] writes one.
    fn written(&self, rel: &str, text: &str) -> String {
        let path = self.file(rel);
        std::fs::write(&path, text).unwrap();
        path
    }

    fn run<R: Runner>(&self, fake: &R, args: &[&str]) -> (bool, String) {
        let mut out = Vec::new();
        let _guard = crate::kill::guard_library(&self.job(args).dirs.library);
        let ok = run_with(
            &self.job(args),
            fake,
            &Server::default(),
            None,
            &mut out,
            &mut Vec::new(),
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        let dirs = self.job(args).dirs;
        if crate::state::exists(&dirs.home)
            && let Err(e) = crate::reconcile::invariant(&dirs)
        {
            panic!("{args:?}: {e}: {text}");
        }
        (ok, text)
    }

    fn songs(&self) -> Vec<Vec<SourceKey>> {
        Manifest::load(&self.dir.path().join("home"))
            .unwrap()
            .songs
            .into_iter()
            .map(|s| s.sources)
            .collect()
    }
}

/// `from` moved to `to`, as the user moves a file.
fn moved(from: &Path, to: &Path) {
    std::fs::rename(from, to).unwrap();
}

fn flacs() -> Fake {
    Fake::default().probe(".flac", FLAC)
}

#[test]
fn an_added_file_with_a_decomposed_name_is_listed_composed_with_its_tags() {
    let s = Setup::new();
    let song = s.file("rips/Noe\u{308}l.flac");
    let (ok, text) = s.run(&flacs(), &["add", "--artist", "Marlo Venn", &song]);
    assert!(ok, "{text}");
    let songs = Manifest::load(&s.dir.path().join("home")).unwrap().songs;
    assert_eq!(songs.len(), 1, "{text}");
    assert_eq!(
        songs[0].sources,
        [SourceKey::Manual("No\u{eb}l.flac".into())]
    );
    assert_eq!(
        songs[0].tags,
        [("artist".to_string(), vec!["Marlo Venn".to_string()])]
    );
}

#[test]
fn an_added_file_is_copied_with_its_lyrics_listed_and_written() {
    let s = Setup::new();
    let song = s.file("rips/Song.flac");
    s.file("rips/Song.lrc");
    s.file("rips/song.JPG");
    let (ok, text) = s.run(&flacs(), &["add", &song]);
    assert!(ok, "{text}");
    assert!(
        s.dir.path().join("rips/Song.flac").exists(),
        "the user's file stays"
    );
    assert!(s.dir.path().join("home/sources/manual/Song.lrc").exists());
    assert!(s.dir.path().join("home/sources/manual/song.JPG").exists());
    assert_eq!(s.songs(), [vec![SourceKey::Manual("Song.flac".into())]]);
    assert!(
        s.dir.path().join("lib/Artist/Record/02 Song.flac").exists(),
        "{text}"
    );
    assert!(
        s.dir.path().join("lib/Artist/Record/02 Song.lrc").exists(),
        "{text}"
    );

    let mut out = Vec::new();
    let e = run_with(
        &s.job(&["add", &song]),
        &flacs(),
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("is a source already"), "{e:#}");
    std::fs::write(&song, "other bytes").unwrap();
    let e = run_with(
        &s.job(&["add", &song]),
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("exists already; rename"), "{e:#}");
}

#[test]
fn lyrics_an_earlier_writing_wrote_are_written_again() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written("home/sources/manual/Song.lrc", "[00:01.00]Paper comets\n");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let home = s.dir.path().join("home");
    let mut state = State::load(&home).unwrap();
    for written in state.outputs.values_mut() {
        if let Some(lyrics) = written.plan.as_mut().and_then(|p| p.lyrics.as_mut()) {
            lyrics.writing = 0;
        }
    }
    state.save(&home).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok && text.contains("Updated (muman's rendering)"), "{text}");
    assert_eq!(
        std::fs::read_to_string(s.dir.path().join("lib/Artist/Record/02 Song.lrc")).unwrap(),
        "[00:01.00]Paper comets\n"
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok && !text.contains("Updated"), "{text}");
}

#[test]
fn lyrics_inside_a_song_cleaned_anew_retag_it_and_encode_nothing() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written(
        "home/sources/manual/Song.lrc",
        "[00:01.00]Paper comets\n[00:02.00][Chorus]\n",
    );
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    std::fs::write(&list, "version = 1\n[[lyrics.out]]\nto = \"tag\"\n").unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let song = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    let inside = || crate::tags::lyrics_in(&song).map(|(text, ..)| text);
    assert_eq!(inside().as_deref(), Some("[00:01.00]Paper comets\n"));

    let t = std::fs::read_to_string(&list).unwrap();
    let at = t.find("[lyrics.clean.packaging]").expect(&t);
    let off = format!(
        "{}{}",
        &t[..at],
        t[at..].replacen("cues = true", "cues = false", 1)
    );
    std::fs::write(&list, off).unwrap();
    let fake = flacs();
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(text.contains("Updated (lyric cleaning)"), "{text}");
    assert_eq!(
        inside().as_deref(),
        Some("[00:01.00]Paper comets\n[00:02.00][Chorus]\n")
    );
    assert!(
        !fake.calls().iter().any(|c| c.iter().any(|a| a == "-c:a")),
        "the song is tagged again, not encoded: {:?}",
        fake.calls()
    );
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
}

#[test]
fn a_lyric_cleaning_switch_flipped_writes_the_lyrics_again() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written(
        "home/sources/manual/Song.lrc",
        "[00:01.00]Paper comets\n[00:02.00][Chorus]\n",
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lrc = s.dir.path().join("lib/Artist/Record/02 Song.lrc");
    assert_eq!(
        std::fs::read_to_string(&lrc).unwrap(),
        "[00:01.00]Paper comets\n"
    );

    let list = s.dir.path().join("home/songs.toml");
    let t = std::fs::read_to_string(&list).unwrap();
    let at = t.find("[lyrics.clean.packaging]").expect(&t);
    let off = format!(
        "{}{}",
        &t[..at],
        t[at..].replacen("cues = true", "cues = false", 1)
    );
    std::fs::write(&list, off).unwrap();
    let song = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    let before = std::fs::read(&song).unwrap();
    let fake = flacs();
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(text.contains("Updated (lyric cleaning)"), "{text}");
    assert_eq!(
        std::fs::read_to_string(&lrc).unwrap(),
        "[00:01.00]Paper comets\n[00:02.00][Chorus]\n"
    );
    assert!(
        !fake.calls().iter().any(|c| c.iter().any(|a| a == "-c:a")),
        "lyrics beside the song alone are written again: {:?}",
        fake.calls()
    );
    assert_eq!(std::fs::read(&song).unwrap(), before, "its file as it was");
}

#[test]
fn a_song_retitled_cleans_its_lyrics_by_its_new_title() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written(
        "home/sources/manual/Song.lrc",
        "[00:01.00]Artist - Song\n[00:02.00]Paper comets\n",
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lrc = |at: &str| std::fs::read_to_string(s.dir.path().join(at)).unwrap();
    assert_eq!(
        lrc("lib/Artist/Record/02 Song.lrc"),
        "[00:02.00]Paper comets\n",
        "the title restated goes"
    );
    let (ok, text) = s.run(
        &flacs(),
        &["set", "-y", "manual:Song.flac", "title=Lantern"],
    );
    assert!(ok, "{text}");
    let retitled = lrc("lib/Artist/Record/02 Lantern.lrc");
    assert_eq!(
        retitled, "[00:01.00]Artist - Song\n[00:02.00]Paper comets\n",
        "{text}"
    );

    let fresh = Setup::new();
    fresh.file("home/sources/manual/Song.flac");
    fresh.written(
        "home/sources/manual/Song.lrc",
        "[00:01.00]Artist - Song\n[00:02.00]Paper comets\n",
    );
    std::fs::write(
        fresh.dir.path().join("home/songs.toml"),
        "version = 1\n\n[[song]]\nsources = [\"manual:Song.flac\"]\ntags.title = \"Lantern\"\n",
    )
    .unwrap();
    let (ok, text) = fresh.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(
        std::fs::read_to_string(fresh.dir.path().join("lib/Artist/Record/02 Lantern.lrc")).unwrap(),
        retitled,
        "as made from the start"
    );
    let (ok, text) = s.run(
        &flacs(),
        &[
            "set",
            "-y",
            "manual:Song.flac",
            "artist=Rin Hollow",
            "album_artist=Artist",
        ],
    );
    assert!(ok, "{text}");
    assert!(text.contains("Updated (lyric cleaning, tags)"), "{text}");
}

#[test]
fn a_song_retitled_cleans_its_credits_by_its_new_title_with_the_title_rule_off() {
    let list = "version = 1\n\n[lyrics.clean.packaging]\ntitle = false\n\n\
                [[song]]\nsources = [\"manual:Song.flac\"]\n";
    let lrc = "[00:01.00]Song\n[00:02.00]Mix by Rin Hollow\n[00:03.00]Paper comets\n";
    let home = |list: &str| {
        let s = Setup::new();
        s.file("home/sources/manual/Song.flac");
        s.written("home/sources/manual/Song.lrc", lrc);
        s.written("home/songs.toml", list);
        let (ok, text) = s.run(&flacs(), &["sync"]);
        assert!(ok, "{text}");
        s
    };
    let read = |s: &Setup, at: &str| std::fs::read_to_string(s.dir.path().join(at)).unwrap();
    let s = home(list);
    assert_eq!(
        read(&s, "lib/Artist/Record/02 Song.lrc"),
        "[00:01.00]Song\n[00:03.00]Paper comets\n",
        "the credit after the title goes"
    );
    let (ok, text) = s.run(
        &flacs(),
        &["set", "-y", "manual:Song.flac", "title=Lantern"],
    );
    assert!(ok, "{text}");
    let fresh = home(&list.replace("\"]\n", "\"]\ntags.title = \"Lantern\"\n"));
    assert_eq!(
        read(&s, "lib/Artist/Record/02 Lantern.lrc"),
        read(&fresh, "lib/Artist/Record/02 Lantern.lrc"),
        "as made from the start: {text}"
    );
}

#[test]
fn a_song_list_naming_the_old_tables_is_moved_and_writes_no_song_again() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written("home/sources/manual/Song.lrc", "[00:01.00]Paper comets\n");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let list = s.dir.path().join("home/songs.toml");
    let now = std::fs::read_to_string(&list).unwrap();
    // Edition 5 knew no [audio.pick.recording] nor [audio.pick.dynamics];
    // a sync adds them back.
    let table = |at: &str, end: &str| {
        let at = now.find(at).unwrap();
        now[at..at + now[at..].find(end).unwrap() + end.len()].to_string()
    };
    let recording = table("[audio.pick.recording]", "enabled = true\n");
    let dynamics = table("[audio.pick.dynamics]", "step = \"1 dB\"\n");
    let old = now
        .replace(&recording, "")
        .replace(&dynamics, "")
        .replace(
            &format!("edition = {}", crate::migrate::EDITION),
            "edition = 5",
        )
        .replace("[audio.pick.", "[quality.")
        .replace("[cover.pick.", "[quality.")
        .replace("[lyrics.clean.", "[clean.lyrics.")
        .replace("[tags.clean.", "[clean.");
    assert!(old.contains("[clean.lyrics.tidy]"), "{old}");
    std::fs::write(&list, &old).unwrap();

    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Renamed in the song list: [quality.purity] is [audio.pick.purity] now"),
        "{text}"
    );
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
    let filled = std::fs::read_to_string(&list).unwrap();
    assert!(
        filled.contains(&recording) && filled.contains(&dynamics),
        "{filled}"
    );
    let without = |text: &str| text.replace(&recording, "").replace(&dynamics, "");
    assert_eq!(without(&filled), without(&now));
}

#[test]
fn a_song_list_an_early_edition_wrote_syncs_and_undoes_to_its_own_values() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    let old = format!(
        "{}\n[[song]]\nsources = [\"manual:Song.flac\"]\n",
        include_str!("../testdata/editions/2.toml")
    );
    let list = s.dir.path().join("home/songs.toml");
    s.written("home/songs.toml", &old);
    let (ok, text) = s.run(&flacs(), &["sync", "--update-defaults"]);
    assert!(ok, "{text}");
    assert!(text.contains("Added"), "{text}");
    let now = std::fs::read_to_string(&list).unwrap();
    let doc: toml_edit::DocumentMut = now.parse().unwrap();
    assert_eq!(
        doc["version"].as_integer(),
        Some(1),
        "a list of one song of one's own is written in the format every muman reads"
    );
    assert_eq!(
        doc["edition"].as_integer(),
        Some(crate::migrate::EDITION),
        "{now}"
    );
    assert_eq!(
        Manifest::load(&s.dir.path().join("home"))
            .unwrap()
            .songs
            .len(),
        1
    );

    let (ok, again) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{again}");
    assert!(!again.contains("Renamed in the song list"), "{again}");
    for verb in ["Added", "Updated", "Written again", "Moved", "Removed"] {
        assert!(!again.contains(verb), "a second sync changed: {again}");
    }
    assert_eq!(std::fs::read_to_string(&list).unwrap(), now);

    // The list goes back to edition 2's values; the sync after the undo
    // gives its settings their new names, as every write of the list does,
    // and keeps the values, which only --update-defaults raises.
    let (ok, undone) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{undone}");
    let back: toml_edit::DocumentMut = std::fs::read_to_string(&list).unwrap().parse().unwrap();
    assert_eq!(back["edition"].as_integer(), Some(2), "{undone}");
    assert_eq!(
        back["audio"]["codecs"].to_string().trim(),
        r#"["opus", "flac"]"#
    );
    assert!(back.get("quality").is_none(), "renamed: {back}");
}

#[test]
fn lyrics_changed_by_hand_are_neither_written_over_nor_removed() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written(
        "home/sources/manual/Song.lrc",
        "[00:01.00]Paper comets\n[00:02.00][Chorus]\n",
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lrc = s.dir.path().join("lib/Artist/Record/02 Song.lrc");
    let mended = "[00:01.00]Paper comets, mended\n";
    std::fs::write(&lrc, mended).unwrap();
    let (_, report) = s.report(&flacs(), &["check"]);
    assert!(
        report.contains("Changed since muman wrote it, so left alone: Artist/Record/02 Song.lrc"),
        "{report}"
    );

    let list = s.dir.path().join("home/songs.toml");
    let t = std::fs::read_to_string(&list).unwrap();
    let at = t.find("[lyrics.clean.packaging]").expect(&t);
    let off = format!(
        "{}{}",
        &t[..at],
        t[at..].replacen("cues = true", "cues = false", 1)
    );
    std::fs::write(&list, off).unwrap();
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(
        text.contains("Artist/Record/02 Song.lrc changed since muman wrote it"),
        "{text}"
    );
    assert_eq!(std::fs::read_to_string(&lrc).unwrap(), mended);

    let (ok, text) = s.run(&flacs(), &["remove", "-y", "manual:Song.flac"]);
    assert!(ok, "{text}");
    assert!(!s.dir.path().join("lib/Artist/Record/02 Song.flac").exists());
    assert_eq!(
        std::fs::read_to_string(&lrc).unwrap(),
        mended,
        "the user's now: {text}"
    );
}

#[test]
fn lyrics_copied_with_new_times_are_still_muman_s() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written(
        "home/sources/manual/Song.lrc",
        "[00:01.00]Paper comets\n[00:02.00][Chorus]\n",
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lrc = s.dir.path().join("lib/Artist/Record/02 Song.lrc");
    filetime::set_file_mtime(&lrc, filetime::FileTime::from_unix_time(86_400, 0)).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    let t = std::fs::read_to_string(&list).unwrap();
    let at = t.find("[lyrics.clean.packaging]").expect(&t);
    let off = format!(
        "{}{}",
        &t[..at],
        t[at..].replacen("cues = true", "cues = false", 1)
    );
    std::fs::write(&list, off).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(text.contains("Updated (lyric cleaning)"), "{text}");
    assert_eq!(
        std::fs::read_to_string(&lrc).unwrap(),
        "[00:01.00]Paper comets\n[00:02.00][Chorus]\n"
    );
}

#[test]
fn where_lyrics_went_becomes_their_outputs_and_writes_no_song_again() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written("home/sources/manual/Song.lrc", "[00:01.00]Paper comets\n");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let list = s.dir.path().join("home/songs.toml");
    let now = std::fs::read_to_string(&list).unwrap();
    let start = now.find("\n# Where the lyrics a song takes").expect(&now);
    let end = now[start..].find("words = true\n").unwrap() + start + "words = true\n".len();
    let old = format!("{}\n{}", &now[..start], &now[end..])
        .replace(
            &format!("edition = {}", crate::migrate::EDITION),
            "edition = 6",
        )
        .replacen("[library]\n", "[library]\nlyrics = \"sidecar\"\n", 1);
    std::fs::write(&list, &old).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Renamed in the song list: [library] `lyrics = \"sidecar\"`"),
        "{text}"
    );
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
    let upgraded = std::fs::read_to_string(&list).unwrap();
    assert!(
        upgraded.contains("[[lyrics.out]]\nto = \"file\""),
        "{upgraded}"
    );
    assert!(!upgraded.contains("lyrics = \"sidecar\""), "{upgraded}");
}

#[test]
fn each_lyrics_output_writes_its_file_and_one_taken_out_takes_only_its_own() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written("home/sources/manual/Song.lrc", "[00:01.00]Paper comets\n");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    let two = "version = 1\n[[lyrics.out]]\nto = \"file\"\n\
               [[lyrics.out]]\nto = \"file\"\nsuffix = \".en\"\n";
    std::fs::write(&list, two).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib/Artist/Record");
    for name in ["02 Song.lrc", "02 Song.en.lrc"] {
        assert_eq!(
            std::fs::read_to_string(lib.join(name)).unwrap(),
            "[00:01.00]Paper comets\n",
            "{name}: {text}"
        );
    }
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");

    let listed = std::fs::read_to_string(&list).unwrap();
    let at = listed
        .find("[[lyrics.out]]\nto = \"file\"\nsuffix = \".en\"")
        .expect(&listed);
    std::fs::write(&list, &listed[..at]).unwrap();
    let before = std::fs::read(lib.join("02 Song.flac")).unwrap();
    let fake = flacs();
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(!lib.join("02 Song.en.lrc").exists(), "{text}");
    assert!(lib.join("02 Song.lrc").exists(), "{text}");
    assert_eq!(std::fs::read(lib.join("02 Song.flac")).unwrap(), before);
    assert!(
        !fake.calls().iter().any(|c| c.iter().any(|a| a == "-c:a")),
        "{text}"
    );
}

#[test]
fn a_file_beside_added_is_written_alone_and_the_rest_left_as_they_are() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written("home/sources/manual/Song.lrc", "[00:01.00]Paper comets\n");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    std::fs::write(&list, "version = 1\n[[lyrics.out]]\nto = \"file\"\n").unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib/Artist/Record");
    let modified = |name: &str| {
        std::fs::metadata(lib.join(name))
            .unwrap()
            .modified()
            .unwrap()
    };
    let (song, lrc) = (modified("02 Song.flac"), modified("02 Song.lrc"));

    let listed = std::fs::read_to_string(&list).unwrap();
    std::fs::write(
        &list,
        format!("{listed}\n[[lyrics.out]]\nto = \"file\"\nformat = \"txt\"\n"),
    )
    .unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(
        std::fs::read_to_string(lib.join("02 Song.txt")).unwrap(),
        "Paper comets\n",
        "{text}"
    );
    assert_eq!(modified("02 Song.flac"), song, "the song's file left alone");
    assert_eq!(modified("02 Song.lrc"), lrc, "its lyrics file left alone");
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
}

#[test]
fn a_cover_file_added_or_taken_out_is_written_or_removed_alone() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    let jpg = s.file("home/sources/manual/Song.jpg");
    std::fs::write(&jpg, crate::testing::PIXEL_JPEG).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib/Artist/Record");
    let song = std::fs::read(lib.join("02 Song.flac")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&list).unwrap();
    let files = "[[cover.out]]\nto = \"file\"\n\
                 [[cover.out]]\nto = \"file\"\nsuffix = \".small\"\nformat = \"webp\"\n\
                 max_side = 300\n";
    std::fs::write(&list, format!("{listed}\n{files}")).unwrap();
    let fake = flacs();
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(
        std::fs::read(lib.join("02 Song.jpg")).unwrap(),
        crate::testing::PIXEL_JPEG,
        "its own bytes: {text}"
    );
    assert_eq!(
        std::fs::read(lib.join("02 Song.small.webp")).unwrap(),
        crate::testing::PIXEL_WEBP,
        "{text}"
    );
    assert_eq!(std::fs::read(lib.join("02 Song.flac")).unwrap(), song);
    assert!(!fake.ran("-c:a"), "{text}");
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");

    let listed = std::fs::read_to_string(&list).unwrap();
    let at = listed
        .find("[[cover.out]]\nto = \"file\"\nsuffix")
        .expect(&listed);
    std::fs::write(&list, &listed[..at]).unwrap();
    let fake = flacs();
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(!lib.join("02 Song.small.webp").exists(), "{text}");
    assert!(lib.join("02 Song.jpg").exists(), "{text}");
    assert_eq!(std::fs::read(lib.join("02 Song.flac")).unwrap(), song);
    assert!(!fake.ran("-c:a"), "{text}");
}

const FOLDER_COVER: &str = "[[cover.out]]\nto = \"tag\"\n\
                            [[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\n";

/// The song the state records as holding `file` beside it, by its first
/// source.
fn holder_of(s: &Setup, file: &str) -> Option<SourceKey> {
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    let mut holders = state
        .outputs
        .values()
        .filter(|w| w.sidecar_paths().any(|p| p.ends_with(file)));
    let holder = holders.next()?.sources.first().cloned();
    assert!(holders.next().is_none(), "one song holds {file}");
    holder
}

#[test]
fn a_song_that_fails_keeps_its_file_and_its_name_from_a_song_of_its_name() {
    let s = Setup::new();
    let a = s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let before = library_of(&s);
    assert_eq!(before.len(), 2, "{text}");
    std::fs::remove_file(&a).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(!ok && text.contains("Failed"), "{text}");
    assert_eq!(library_of(&s), before, "neither file changes: {text}");
    assert!(
        !text.contains("Updated") && !text.contains("Removed"),
        "{text}"
    );
}

#[test]
fn a_folder_s_cover_is_written_once_and_handed_on_when_its_song_goes() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    std::fs::write(&list, format!("version = 1\n{FOLDER_COVER}")).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let cover = s.dir.path().join("lib/Artist/Record/cover.jpg");
    assert_eq!(
        std::fs::read(&cover).unwrap(),
        crate::testing::PIXEL_JPEG,
        "{text}"
    );
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Up to date: 2 song(s)"), "{text}");

    let holder = holder_of(&s, "cover.jpg").expect("a song holds it");
    let (ok, text) = s.run(&flacs(), &["remove", "-y", &holder.to_string()]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(
        std::fs::read(&cover).unwrap(),
        crate::testing::PIXEL_JPEG,
        "handed on: {text}"
    );
    let now = holder_of(&s, "cover.jpg").expect("the song left holds it");
    assert_ne!(now, holder);
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
}

#[test]
fn a_song_that_fails_keeps_the_folder_s_cover_it_holds() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    std::fs::write(&list, format!("version = 1\n{FOLDER_COVER}")).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let before = library_of(&s);
    let holder = holder_of(&s, "cover.jpg").expect("a song holds it");
    let SourceKey::Manual(file) = &holder else {
        panic!("{holder}");
    };
    std::fs::remove_file(s.dir.path().join("home/sources/manual").join(file.path())).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(!ok && text.contains("Failed"), "{text}");
    assert_eq!(library_of(&s), before, "nothing moves for its sake: {text}");
    assert_eq!(holder_of(&s, "cover.jpg"), Some(holder), "{text}");
}

#[test]
fn a_folder_s_own_cover_is_left_alone_and_blocks_no_song() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        format!("version = 1\n{FOLDER_COVER}"),
    )
    .unwrap();
    let mine = s.dir.path().join("lib/Artist/Record/cover.jpg");
    std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
    std::fs::write(&mine, "mine").unwrap();
    let (ok, text) = s.run(&flacs(), &["sync", "--force"]);
    assert!(ok, "{text}");
    assert!(text.contains("cover.jpg left alone"), "{text}");
    assert_eq!(std::fs::read_to_string(&mine).unwrap(), "mine");
    assert!(s.dir.path().join("lib/Artist/Record/02 Song.flac").exists());
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");

    std::fs::remove_file(&mine).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(
        std::fs::read(&mine).unwrap(),
        crate::testing::PIXEL_JPEG,
        "written once theirs is gone: {text}"
    );
}

#[test]
fn a_folder_s_cover_the_user_changed_stays_in_its_folder_when_its_song_moves() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    std::fs::write(&list, format!("version = 1\n{FOLDER_COVER}")).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let cover = s.dir.path().join("lib/Artist/Record/cover.jpg");
    std::fs::write(&cover, "mine").unwrap();
    let before = library_of(&s);
    let holder = holder_of(&s, "cover.jpg").expect("a song holds it");

    let (ok, text) = s.run(
        &flacs(),
        &["set", "-y", &holder.to_string(), "album=Copper Moth"],
    );
    assert!(ok, "{text}");
    assert!(text.contains("Moved:"), "{text}");
    assert_eq!(std::fs::read_to_string(&cover).unwrap(), "mine", "{text}");
    let moved = s.dir.path().join("lib/Artist/Copper Moth/cover.jpg");
    assert_eq!(
        std::fs::read(&moved).unwrap(),
        crate::testing::PIXEL_JPEG,
        "the song's new folder has muman's: {text}"
    );
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Up to date: 2 song(s)"), "{text}");

    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(library_of(&s), before, "{text}");
    let (_, report) = s.report(&flacs(), &["check"]);
    assert!(report.contains("cover.jpg"), "{report}");
}

#[test]
fn a_folder_s_cover_the_user_changed_stays_with_a_song_held_back_from_its_move() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    std::fs::write(&list, format!("version = 1\n{FOLDER_COVER}")).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let cover = s.dir.path().join("lib/Artist/Record/cover.jpg");
    std::fs::write(&cover, "mine").unwrap();
    let holder = holder_of(&s, "cover.jpg").expect("a song holds it");
    assert_eq!(holder.to_string(), "manual:b.flac");
    let lib = s.dir.path().join("lib");
    let mine = lib.join("Record/Song.flac");
    std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
    std::fs::write(&mine, "my own rip").unwrap();
    let before = library_of(&s);
    let listed = std::fs::read_to_string(&list).unwrap();
    let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
    let renamed = listed.replace(&default, "template = \"{{ album }}/{{ title }}\"");
    std::fs::write(&list, &renamed).unwrap();

    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Moved:"), "{text}");
    assert!(
        text.contains("Left at Artist/Record/02 Song.flac"),
        "{text}"
    );
    assert_eq!(std::fs::read_to_string(&cover).unwrap(), "mine", "{text}");
    assert_eq!(std::fs::read_to_string(&mine).unwrap(), "my own rip");

    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(library_of(&s), before, "{text}");
}

#[test]
fn a_folder_s_cover_the_user_changed_moves_with_its_whole_album() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    std::fs::write(&list, format!("version = 1\n{FOLDER_COVER}")).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    std::fs::write(s.dir.path().join("lib/Artist/Record/cover.jpg"), "mine").unwrap();
    let before = library_of(&s);
    let listed = std::fs::read_to_string(&list).unwrap();
    let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
    let renamed = listed.replace(&default, "template = \"{{ album }}/{{ title }}\"");
    std::fs::write(&list, &renamed).unwrap();

    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib");
    assert_eq!(
        std::fs::read_to_string(lib.join("Record/cover.jpg")).unwrap(),
        "mine",
        "{text}"
    );
    assert!(!lib.join("Artist").exists(), "{text}");

    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(library_of(&s), before, "{text}");
}

#[test]
fn undo_takes_a_folder_s_cover_back_out() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let list = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&list).unwrap();
    let folder = "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"folder\"\n";
    std::fs::write(&list, format!("{listed}\n{folder}")).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let cover = s.dir.path().join("lib/Artist/Record/folder.jpg");
    assert!(cover.exists(), "{text}");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert!(!cover.exists(), "{text}");
}

#[test]
fn a_folder_s_cover_moves_with_its_album() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    std::fs::write(&list, format!("version = 1\n{FOLDER_COVER}")).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let listed = std::fs::read_to_string(&list).unwrap();
    let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
    std::fs::write(
        &list,
        listed.replace(&default, "template = \"{{ album }}/{{ title }}\""),
    )
    .unwrap();
    let fake = flacs();
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(text.contains("Moved:"), "{text}");
    let lib = s.dir.path().join("lib");
    assert_eq!(
        std::fs::read(lib.join("Record/cover.jpg")).unwrap(),
        crate::testing::PIXEL_JPEG,
        "{text}"
    );
    assert!(!lib.join("Artist").exists(), "{text}");
    assert!(!fake.ran("-c:a"), "moved, not written: {text}");
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Up to date: 2 song(s)"), "{text}");
}

#[test]
fn songs_of_two_folders_moved_into_the_library_folder_move_and_take_no_cover() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let other = FLAC
        .replace("\"Artist\"", "\"Kiri Hoshino\"")
        .replace("\"Record\"", "\"Shoals\"")
        .replace("\"Song\"", "\"Two\"");
    let fake = || Fake::default().probe("b.flac", &other).probe(".flac", FLAC);
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    std::fs::write(&list, format!("version = 1\n{FOLDER_COVER}")).unwrap();
    let (ok, text) = s.run(&fake(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib");
    assert!(lib.join("Artist/Record/cover.jpg").exists(), "{text}");
    assert!(lib.join("Kiri Hoshino/Shoals/cover.jpg").exists(), "{text}");
    let listed = std::fs::read_to_string(&list).unwrap();
    let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
    let flat = listed.replace(&default, "template = \"{{ artist }} - {{ title }}\"");
    std::fs::write(&list, flat).unwrap();
    let fake = fake();
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(text.matches("Moved:").count(), 2, "{text}");
    assert!(!text.contains("Added:"), "{text}");
    assert!(!fake.ran("-c:a"), "moved, not written: {text}");
    assert!(
        text.contains("No folder cover in the library folder itself"),
        "{text}"
    );
    assert!(!text.contains("share a folder"), "{text}");
    assert!(!lib.join("cover.jpg").exists(), "{text}");
    assert!(!lib.join("Artist").exists(), "{text}");
    assert!(!lib.join("Kiri Hoshino").exists(), "{text}");
    let (_, text) = s.run(&fake, &["sync"]);
    assert!(text.contains("Up to date: 2 song(s)"), "{text}");
}

#[test]
fn a_folder_cover_the_user_changed_is_left_by_a_template_that_drops_its_folder() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    std::fs::write(&list, format!("version = 1\n{FOLDER_COVER}")).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let cover = s.dir.path().join("lib/Artist/Record/cover.jpg");
    std::fs::write(&cover, "a cover of my own").unwrap();
    let before = library_of(&s);
    let listed = std::fs::read_to_string(&list).unwrap();
    let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
    std::fs::write(
        &list,
        listed.replace(&default, "template = \"{{ artist }} - {{ title }}\""),
    )
    .unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(
        std::fs::read(&cover).unwrap(),
        b"a cover of my own",
        "{text}"
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(library_of(&s), before, "{text}");
}

#[test]
fn a_song_that_fails_still_counts_toward_its_folder_s_majority() {
    let s = Setup::new();
    for name in ["a.flac", "b.flac", "other.flac"] {
        s.file(&format!("home/sources/manual/{name}"));
    }
    let other = crate::testing::FLAC.replace("\"Record\"", "\"Harbor\"");
    let fake = || {
        Fake::default()
            .probe("other.flac", &other)
            .probe(".flac", FLAC)
    };
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    let flat = "[library]\ntemplate = \"{{ artist }}/{{ title }}\"\n";
    let majority =
        FOLDER_COVER.replace("name = \"cover\"", "name = \"cover\"\nmixed = \"majority\"");
    std::fs::write(&list, format!("version = 1\n{flat}{majority}")).unwrap();
    let (ok, text) = s.run(&fake(), &["sync"]);
    assert!(ok, "{text}");
    let before = library_of(&s);
    let holder = holder_of(&s, "cover.jpg").expect("a song holds it");
    let other_of_record = if holder.to_string() == "manual:a.flac" {
        "b.flac"
    } else {
        "a.flac"
    };
    std::fs::remove_file(
        s.dir
            .path()
            .join("home/sources/manual")
            .join(other_of_record),
    )
    .unwrap();
    let (ok, text) = s.run(&fake(), &["sync"]);
    assert!(!ok && text.contains("Failed"), "{text}");
    assert!(
        !text.contains("No folder cover"),
        "two songs of Record still: {text}"
    );
    assert_eq!(library_of(&s), before, "{text}");
}

#[test]
fn a_folder_of_several_albums_takes_a_cover_only_by_a_majority() {
    let s = Setup::new();
    for name in ["a.flac", "b.flac", "other.flac"] {
        s.file(&format!("home/sources/manual/{name}"));
    }
    let other = crate::testing::FLAC.replace("\"Record\"", "\"Harbor\"");
    let fake = || {
        Fake::default()
            .probe("other.flac", &other)
            .probe(".flac", FLAC)
    };
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    let flat = "[library]\ntemplate = \"{{ artist }}/{{ title }}\"\n";
    std::fs::write(&list, format!("version = 1\n{flat}{FOLDER_COVER}")).unwrap();
    let (ok, text) = s.run(&fake(), &["sync"]);
    assert!(ok, "{text}");
    let cover = s.dir.path().join("lib/Artist/cover.jpg");
    assert!(!cover.exists(), "{text}");
    assert!(
        text.contains("No folder cover where songs of several albums share a folder: Artist"),
        "{text}"
    );

    let listed = std::fs::read_to_string(&list).unwrap();
    let majority = listed.replace("name = \"cover\"", "name = \"cover\"\nmixed = \"majority\"");
    std::fs::write(&list, &majority).unwrap();
    let (ok, text) = s.run(&fake(), &["sync"]);
    assert!(ok, "{text}");
    assert!(
        cover.exists(),
        "two songs of Record to one of Harbor: {text}"
    );
    assert!(!text.contains("No folder cover"), "{text}");

    let (ok, text) = s.run(&fake(), &["remove", "-y", "manual:b.flac"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&fake(), &["sync"]);
    assert!(ok, "{text}");
    assert!(!cover.exists(), "one song each: {text}");
    assert!(
        text.contains("No folder cover where no album has the most of a folder's songs"),
        "{text}"
    );
}

#[test]
fn lyrics_go_where_their_timing_takes_them() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written("home/sources/manual/Song.lrc", "Paper comets\n");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n[[lyrics.out]]\nto = \"file\"\nwhen = \"timed\"\n\
         [[lyrics.out]]\nto = \"file\"\nsuffix = \".plain\"\nwhen = \"untimed\"\n",
    )
    .unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib/Artist/Record");
    assert!(!lib.join("02 Song.lrc").exists(), "untimed: {text}");
    assert_eq!(
        std::fs::read_to_string(lib.join("02 Song.plain.lrc")).unwrap(),
        "Paper comets\n"
    );
}

#[test]
fn lyrics_are_written_as_cues_and_plain_text_each_in_its_file() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written(
        "home/sources/manual/Song.lrc",
        "[00:01.00]Paper comets\n[00:03.50]Lantern weather\n",
    );
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n[[lyrics.out]]\nto = \"file\"\nformat = \"vtt\"\n\
         [[lyrics.out]]\nto = \"file\"\nformat = \"srt\"\n\
         [[lyrics.out]]\nto = \"file\"\nformat = \"txt\"\n",
    )
    .unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib/Artist/Record");
    let vtt = std::fs::read_to_string(lib.join("02 Song.vtt")).unwrap();
    assert!(
        vtt.starts_with("WEBVTT\n\n00:00:01.000 --> 00:00:03.500\nPaper comets\n"),
        "{vtt}"
    );
    let srt = std::fs::read_to_string(lib.join("02 Song.srt")).unwrap();
    assert!(
        srt.starts_with("1\n00:00:01,000 --> 00:00:03,500\n"),
        "{srt}"
    );
    assert_eq!(
        std::fs::read_to_string(lib.join("02 Song.txt")).unwrap(),
        "Paper comets\nLantern weather\n"
    );
    assert!(!lib.join("02 Song.lrc").exists());
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
}

#[test]
fn a_file_of_cues_added_writes_no_song_whose_lyrics_are_inside_again() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written("home/sources/manual/Song.lrc", "[00:01.00]Paper comets\n");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    let outs = "version = 1\n[[lyrics.out]]\nto = \"tag\"\n[[lyrics.out]]\nto = \"file\"\n";
    std::fs::write(&list, outs).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let listed = std::fs::read_to_string(&list).unwrap();
    std::fs::write(
        &list,
        format!("{listed}\n[[lyrics.out]]\nto = \"file\"\nformat = \"srt\"\n"),
    )
    .unwrap();
    let fake = flacs();
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib/Artist/Record");
    let srt = std::fs::read_to_string(lib.join("02 Song.srt")).unwrap();
    assert!(srt.starts_with("1\n00:00:01,000 --> "), "{srt}");
    assert!(
        !fake.calls().iter().any(|c| c.iter().any(|a| a == "-c:a")),
        "{text}"
    );
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
}

#[test]
fn a_song_file_s_own_lyrics_tag_is_its_lyrics() {
    use lofty::config::WriteOptions;
    use lofty::tag::{ItemKey, ItemValue, Tag, TagExt, TagItem, TagType};

    let s = Setup::new();
    let song = s.file("rips/Song.flac");
    std::fs::write(&song, crate::testing::SILENCE_FLAC).unwrap();
    let lrc = "[00:01.00]first invented line\n[00:03.00]second invented line\n";
    let mut tag = Tag::new(TagType::VorbisComments);
    tag.push(TagItem::new(
        ItemKey::TrackTitle,
        ItemValue::Text("Song".into()),
    ));
    tag.push(TagItem::new(ItemKey::Lyrics, ItemValue::Text(lrc.into())));
    tag.save_to_path(Path::new(&song), WriteOptions::default())
        .unwrap();
    let (ok, text) = s.run(&flacs(), &["add", &song]);
    assert!(ok, "{text}");
    let written = s.dir.path().join("lib/Unknown Artist/Song/Song.lrc");
    assert_eq!(
        std::fs::read_to_string(&written).ok().as_deref(),
        Some(lrc),
        "{text}"
    );

    let home = s.dir.path().join("home");
    let key = SourceKey::Manual("Song.flac".into());
    let own = |state: &State| {
        state.facts[&key]
            .lyrics
            .iter()
            .any(|l| l.at == crate::facts::LyricsAt::Tag)
    };
    let mut state = State::load(&home).unwrap();
    let facts = state.facts.get_mut(&key).unwrap();
    facts.lyrics.clear();
    facts.tags_method = String::new();
    state.save(&home).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(
        ok && text.contains("Reading the tags of 1 source(s)"),
        "{text}"
    );
    assert!(
        own(&State::load(&home).unwrap()),
        "facts made before learn them"
    );
}

#[test]
fn tags_given_to_add_are_set_on_the_song_and_written() {
    let s = Setup::new();
    let song = s.file("rips/Song.flac");
    let (ok, text) = s.run(
        &flacs(),
        &[
            "add",
            "--artist",
            "Marlo Venn",
            "--album",
            "Low Orchard",
            &song,
        ],
    );
    assert!(ok, "{text}");
    let m = Manifest::load(&s.dir.path().join("home")).unwrap();
    let tags: BTreeMap<String, Vec<String>> = m.songs[0].tags.iter().cloned().collect();
    assert_eq!(tags["artist"], ["Marlo Venn"]);
    assert_eq!(tags["album"], ["Low Orchard"]);
    assert!(
        s.dir
            .path()
            .join("lib/Marlo Venn/Low Orchard/02 Song.flac")
            .exists(),
        "{text}"
    );
}

#[test]
fn a_dropped_in_file_is_listed_and_a_moved_one_followed() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs(), [vec![SourceKey::Manual("a.flac".into())]]);

    let from = s.dir.path().join("home/sources/manual/a.flac");
    let to = s.dir.path().join("home/sources/manual/Artist/a.flac");
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::rename(&from, &to).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Moved: manual:a.flac is now manual:Artist/a.flac"),
        "{text}"
    );
    assert_eq!(s.songs(), [vec![SourceKey::Manual("Artist/a.flac".into())]]);
    assert!(!text.contains("Updated"), "a file that only moved: {text}");
}

/// Run `args` in `s`, a dropped-in file settling for an hour.
fn run_settling(s: &Setup, fake: &Fake, args: &[&str]) -> (bool, String) {
    let mut job = s.job(args);
    job.settling = std::time::Duration::from_secs(3600);
    let mut out = Vec::new();
    let ok = run_with(
        &job,
        fake,
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    (ok, String::from_utf8(out).unwrap())
}

#[test]
fn a_file_renamed_in_the_manual_folder_is_followed_without_settling() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");

    let from = s.dir.path().join("home/sources/manual/a.flac");
    let to = s.dir.path().join("home/sources/manual/Artist/drop.flac");
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::rename(&from, &to).unwrap();
    let (ok, report) = s.report(&flacs(), &["check"]);
    assert!(ok, "{report}");
    assert!(report.contains("followed by the next sync"), "{report}");
    let (ok, text) = run_settling(&s, &flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Moved: manual:a.flac is now manual:Artist/drop.flac"),
        "{text}"
    );
    assert!(!text.contains("Still being copied in"), "{text}");
    assert_eq!(
        s.songs(),
        [vec![SourceKey::Manual("Artist/drop.flac".into())]]
    );
}

#[test]
fn a_listed_file_gone_while_another_is_copied_in_waits_rather_than_fails() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let built: Vec<PathBuf> =
        crate::store::files_below(&s.dir.path().join("lib"), 9, |_| true).unwrap();
    assert_eq!(built.len(), 1, "{text}");

    std::fs::remove_file(s.dir.path().join("home/sources/manual/a.flac")).unwrap();
    std::fs::write(
        s.dir.path().join("home/sources/manual/b.flac"),
        b"half of a.flac",
    )
    .unwrap();
    let (ok, text) = run_settling(&s, &flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(text.contains("Still being copied in"), "{text}");
    assert!(text.contains("Made once its file is copied in"), "{text}");
    assert!(!text.contains("Failed"), "{text}");
    assert!(built[0].exists(), "its file is kept: {text}");
    assert_eq!(s.songs(), [vec![SourceKey::Manual("a.flac".into())]]);
}

/// Move the manual file `from` of `s` to `to`.
fn move_manual(s: &Setup, from: &str, to: &str) {
    let manual = s.dir.path().join("home/sources/manual");
    let to = manual.join(to);
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::rename(manual.join(from), to).unwrap();
}

#[test]
fn a_moved_file_is_followed_by_every_command_that_syncs() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");

    move_manual(&s, "a.flac", "Artist/a.flac");
    let (ok, text) = s.run(&flacs(), &["remove", "-y", "manual:b.flac"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Moved: manual:a.flac is now manual:Artist/a.flac"),
        "{text}"
    );
    assert_eq!(s.songs(), [vec![SourceKey::Manual("Artist/a.flac".into())]]);
}

#[test]
fn undoing_the_sync_that_followed_a_move_follows_it_again() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let built: Vec<PathBuf> =
        crate::store::files_below(&s.dir.path().join("lib"), 9, |_| true).unwrap();
    assert_eq!(built.len(), 1, "{text}");

    move_manual(&s, "a.flac", "Artist/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Moved: manual:a.flac is now manual:Artist/a.flac"),
        "{text}"
    );
    assert!(!text.contains("Removed"), "{text}");
    assert!(built[0].exists(), "its file is kept: {text}");
    assert_eq!(s.songs(), [vec![SourceKey::Manual("Artist/a.flac".into())]]);

    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(!text.contains("Added"), "listed once: {text}");
    assert_eq!(s.songs(), [vec![SourceKey::Manual("Artist/a.flac".into())]]);
}

/// A song of `in/a.flac` and a copy of it, `in/copy.flac`, synced, by
/// `fake`; the library file built.
fn copies_listed(s: &Setup, fake: &Fake) -> PathBuf {
    s.written("home/sources/manual/in/a.flac", "one recording");
    assert!(s.run(fake, &["sync"]).0);
    s.written("home/sources/manual/in/copy.flac", "one recording");
    let (ok, text) = s.run(fake, &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(
        s.songs(),
        [vec![
            SourceKey::Manual("in/a.flac".into()),
            SourceKey::Manual("in/copy.flac".into())
        ]],
        "{text}"
    );
    let built = crate::store::files_below(&s.dir.path().join("lib"), 9, |_| true).unwrap();
    assert_eq!(built.len(), 1, "{text}");
    built[0].clone()
}

fn copies() -> Fake {
    flacs()
        .print("a.flac", words(1600, 1))
        .print("copy.flac", words(1600, 1))
}

#[test]
fn two_copies_of_one_song_moved_together_are_followed_and_undone() {
    let s = Setup::new();
    let built = copies_listed(&s, &copies());
    move_manual(&s, "in/a.flac", "x/p.flac");
    move_manual(&s, "in/copy.flac", "x/q.flac");
    let moved = [vec![
        SourceKey::Manual("x/p.flac".into()),
        SourceKey::Manual("x/q.flac".into()),
    ]];
    let (ok, text) = s.run(&copies(), &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs(), moved, "{text}");

    let (ok, text) = s.run(&copies(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert!(!text.contains("Not followed"), "{text}");
    assert!(!text.contains("Removed"), "{text}");
    assert!(built.exists(), "{text}");
    assert_eq!(s.songs(), moved, "{text}");
    let (ok, report) = s.report(&copies(), &["check"]);
    assert!(ok, "{report}");
}

#[test]
fn a_file_of_yours_deleted_beside_another_of_its_song_is_taken_out_of_it() {
    let s = Setup::new();
    let fake = || {
        flacs()
            .print("old.flac", words(1600, 1))
            .print("new.flac", words(1600, 1))
    };
    let (old, new) = (
        SourceKey::Manual("in/old.flac".into()),
        SourceKey::Manual("in/new.flac".into()),
    );
    let gone = s.file("home/sources/manual/in/old.flac");
    assert!(s.run(&fake(), &["sync"]).0);
    s.file("home/sources/manual/in/new.flac");
    let (ok, text) = s.run(&fake(), &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs(), [vec![old.clone(), new.clone()]], "{text}");

    std::fs::remove_file(gone).unwrap();
    let (_, report) = s.report(&fake(), &["status"]);
    assert!(
        report.contains(
            "Deleted from sources/manual by hand, taken out of its song by the next sync: \
             manual:in/old.flac"
        ),
        "{report}"
    );
    let (ok, text) = s.run(&fake(), &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Deleted from sources/manual by hand, so taken out of its song"),
        "{text}"
    );
    assert_eq!(s.songs(), [vec![new.clone()]]);
    let (ok, report) = s.report(&fake(), &["check"]);
    assert!(ok, "{report}");

    let (ok, text) = s.run(&fake(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs(), [vec![old, new]]);
}

/// `a.flac` and `b.flac` synced by `fake`, the song of `b.flac` removed,
/// and `b.flac` written anew with other bytes.
fn removed_and_written_over(s: &Setup, fake: &Fake) {
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    assert!(s.run(fake, &["sync"]).0);
    let (ok, text) = s.run(fake, &["remove", "-y", "manual:b.flac"]);
    assert!(ok, "{text}");
    s.written(
        "home/sources/manual/b.flac",
        "another recording at its name",
    );
}

#[test]
fn other_audio_at_a_removed_song_s_name_is_listed_as_a_new_file() {
    let s = Setup::new();
    removed_and_written_over(&s, &flacs());
    let (_, report) = s.run(&flacs(), &["status"]);
    assert!(
        report.contains(
            "Other audio than its removed song held, listed by the next sync as a new file: \
             manual:b.flac"
        ),
        "{report}"
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Other audio than its removed song held, so listed as a new file"),
        "{text}"
    );
    assert!(!text.contains("kept out"), "{text}");
    assert_eq!(
        s.songs(),
        [
            vec![SourceKey::Manual("a.flac".into())],
            vec![SourceKey::Manual("b.flac".into())]
        ]
    );
}

#[test]
fn a_removed_song_s_file_retagged_in_place_stays_out() {
    let s = Setup::new();
    let fake = Fake {
        hashes: vec![("b.flac".into(), 7)],
        ..flacs()
    };
    removed_and_written_over(&s, &fake);
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(!text.contains("Other audio"), "{text}");
    assert_eq!(s.songs(), [vec![SourceKey::Manual("a.flac".into())]]);
}

#[test]
fn a_song_whose_copies_cannot_be_told_apart_keeps_its_file() {
    let s = Setup::new();
    let built = copies_listed(&s, &copies());
    let (ok, text) = s.run(
        &copies(),
        &["set", "-y", "manual:in/a.flac", "audio=manual:in/copy.flac"],
    );
    assert!(ok, "{text}");
    move_manual(&s, "in/a.flac", "x/p.flac");
    move_manual(&s, "in/copy.flac", "x/q.flac");
    assert!(s.run(&copies(), &["sync"]).0, "told apart by their stamps");
    for args in [&["undo", "-y"][..], &["sync"]] {
        let (ok, text) = s.run(&copies(), args);
        assert!(!ok, "which is pinned cannot be told: {text}");
        assert!(text.contains("Not followed"), "{text}");
        assert!(!text.contains("Removed"), "{text}");
        assert!(built.exists(), "held as built: {text}");
    }
    let (_, report) = s.report(&copies(), &["check"]);
    assert!(!report.contains("cannot be written"), "{report}");
}

#[test]
fn a_file_renamed_and_retagged_at_once_is_followed_by_its_audio() {
    let s = Setup::new();
    let fake = Fake {
        hashes: vec![("a.flac".into(), 7), ("drop.flac".into(), 7)],
        ..flacs()
    };
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");

    std::fs::remove_file(s.dir.path().join("home/sources/manual/a.flac")).unwrap();
    s.written(
        "home/sources/manual/Artist/drop.flac",
        "a.flac, tagged anew",
    );
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Moved: manual:a.flac is now manual:Artist/drop.flac"),
        "{text}"
    );
    assert_eq!(
        s.songs(),
        [vec![SourceKey::Manual("Artist/drop.flac".into())]]
    );
}

/// The two manual files `s` lists, `a.flac` and `b.flac`, synced; the
/// first song given a genre by hand.
fn two_listed(s: &Setup) {
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&flacs(), &["set", "-y", "manual:a.flac", "genre=Folk"]);
    assert!(ok, "{text}");
}

/// The genre each song of `s` was given by hand, by its sources.
fn genres(s: &Setup) -> Vec<(Vec<SourceKey>, Option<String>)> {
    let manifest = Manifest::load(&s.dir.path().join("home")).unwrap();
    (manifest.songs.into_iter())
        .map(|song| {
            let genre = (song.tags.iter())
                .find(|(k, _)| k == "genre")
                .and_then(|(_, v)| v.first().cloned());
            (song.sources, genre)
        })
        .collect()
}

#[test]
fn two_files_that_trade_names_are_each_followed_to_the_other_s() {
    let s = Setup::new();
    two_listed(&s);

    move_manual(&s, "a.flac", "x.flac");
    move_manual(&s, "b.flac", "a.flac");
    move_manual(&s, "x.flac", "b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Moved: manual:a.flac is now manual:b.flac"),
        "{text}"
    );
    assert!(
        text.contains("Moved: manual:b.flac is now manual:a.flac"),
        "{text}"
    );
    assert!(!text.contains("has changed since"), "{text}");
    let (a, b) = (
        SourceKey::Manual("a.flac".into()),
        SourceKey::Manual("b.flac".into()),
    );
    assert_eq!(
        genres(&s),
        [(vec![b], Some("Folk".to_string())), (vec![a], None)],
        "each song keeps its own tags"
    );
}

#[test]
fn a_file_moved_over_another_listed_one_takes_its_song_there() {
    let s = Setup::new();
    two_listed(&s);

    move_manual(&s, "b.flac", "a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(!ok, "the song of the file replaced has none: {text}");
    assert!(
        text.contains("Moved: manual:b.flac is now manual:a.flac"),
        "{text}"
    );
    assert!(
        text.contains("Replaced by manual:b.flac, moved over it: manual:a.flac"),
        "{text}"
    );
    let (a, b) = (
        SourceKey::Manual("a.flac".into()),
        SourceKey::Manual("b.flac".into()),
    );
    assert_eq!(
        genres(&s),
        [(vec![b], Some("Folk".to_string())), (vec![a], None)],
        "the Folk song waits for its own file"
    );
}

#[test]
fn files_edited_in_place_are_followed_as_changed_not_as_moved() {
    let s = Setup::new();
    two_listed(&s);

    s.written("home/sources/manual/a.flac", "other audio of a");
    s.written("home/sources/manual/b.flac", "other audio of b");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(!text.contains("Moved"), "{text}");
    assert_eq!(
        s.songs(),
        [
            vec![SourceKey::Manual("a.flac".into())],
            vec![SourceKey::Manual("b.flac".into())]
        ]
    );
}

#[test]
fn a_gone_file_two_files_could_be_is_not_guessed_at() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");

    let manual = s.dir.path().join("home/sources/manual");
    for copy in ["x", "y"] {
        std::fs::create_dir_all(manual.join(copy)).unwrap();
        std::fs::copy(manual.join("a.flac"), manual.join(copy).join("a.flac")).unwrap();
    }
    std::fs::remove_file(manual.join("a.flac")).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(!ok, "{text}");
    assert!(
        text.contains(
            "Not followed, as which file is which cannot be told: manual:a.flac held what \
             manual:x/a.flac, manual:y/a.flac hold"
        ),
        "{text}"
    );
    assert!(!text.contains("Added"), "neither is listed anew: {text}");
    assert_eq!(s.songs(), [vec![SourceKey::Manual("a.flac".into())]]);
}

#[test]
fn a_removed_song_s_file_moved_stays_out_and_is_restored_where_it_is() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");

    move_manual(&s, "b.flac", "sorted/b.flac");
    let (ok, text) = s.run(&flacs(), &["remove", "-y", "manual:b.flac"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Moved: manual:b.flac is now manual:sorted/b.flac"),
        "{text}"
    );
    let removed = |s: &Setup| {
        (Manifest::load(&s.dir.path().join("home")).unwrap().removed)
            .into_iter()
            .flat_map(|r| r.sources)
            .collect::<Vec<_>>()
    };
    assert_eq!(removed(&s), [SourceKey::Manual("sorted/b.flac".into())]);
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(!text.contains("Added"), "{text}");

    move_manual(&s, "sorted/b.flac", "kept/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(
        !text.contains("Added"),
        "a file moved after its removal: {text}"
    );
    assert_eq!(removed(&s), [SourceKey::Manual("kept/b.flac".into())]);

    move_manual(&s, "kept/b.flac", "b.flac");
    let (ok, text) = s.run(&flacs(), &["restore", "-y", "manual:kept/b.flac"]);
    assert!(ok, "{text}");
    assert_eq!(
        s.songs(),
        [
            vec![SourceKey::Manual("a.flac".into())],
            vec![SourceKey::Manual("b.flac".into())]
        ]
    );
}

#[test]
fn undoing_an_add_whose_copy_was_moved_takes_the_copy_out() {
    let s = Setup::new();
    let a = s.file("rips/a.flac");
    let b = s.file("rips/b.flac");
    assert!(s.run(&flacs(), &["add", &a]).0);
    assert!(s.run(&flacs(), &["add", &b]).0);

    move_manual(&s, "b.flac", "sorted/b.flac");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert!(text.contains("Deleted the copy the run made"), "{text}");
    assert!(!text.contains("gone already"), "{text}");
    let moved = s.dir.path().join("home/sources/manual/sorted/b.flac");
    assert!(!moved.exists(), "{text}");
    assert!(
        s.dir.path().join("rips/b.flac").exists(),
        "the user's own stays"
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(!text.contains("Added"), "{text}");
    assert_eq!(s.songs(), [vec![SourceKey::Manual("a.flac".into())]]);
}

#[test]
fn an_original_yt_dlp_fetched_goes_into_the_store_under_its_key() {
    let s = Setup::new();
    let original = s.file("Downloads/YouTube/chan/Song [aaaaaaaaaaa].mkv");
    let fake = Fake::default().info(
        "aaaaaaaaaaa",
        r#"{"id": "aaaaaaaaaaa", "extractor_key": "Youtube", "title": "Song", "uploader": "Chan"}"#,
    );
    let (ok, text) = s.run(&fake, &["add", &original]);
    assert!(ok, "{text}");
    let store = s.dir.path().join("home/sources/yt-dlp");
    assert!(store.join("youtube.com/aaaaaaaaaaa.mkv").exists(), "{text}");
    assert!(!store.join("chan").exists(), "{text}");
    assert_eq!(s.songs(), [vec![SourceKey::youtube("aaaaaaaaaaa")]]);
}

#[test]
fn an_original_its_info_json_cannot_be_cleared_of_is_not_added() {
    let s = Setup::new();
    let original = s.file("Downloads/YouTube/chan/Song [aaaaaaaaaaa].mkv");
    let fake = Fake::default()
        .info(
            "aaaaaaaaaaa",
            r#"{"id": "aaaaaaaaaaa", "extractor_key": "Youtube", "title": "Song",
                "url": "https://media.video.test/a?ip=203.0.113.7"}"#,
        )
        .probe("aaaaaaaaaaa.mkv", "not what ffprobe writes");
    let e = run_with(
        &s.job(&["add", &original]),
        &fake,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("not added"), "{e:#}");
    let store = s.dir.path().join("home/sources/yt-dlp");
    assert!(!store.join("youtube.com/aaaaaaaaaaa.mkv").exists());
    assert!(Path::new(&original).exists(), "the file given is left");
}

#[test]
fn an_original_whose_id_holds_a_space_goes_into_the_store_under_its_key() {
    let s = Setup::new();
    let original = s.file("Downloads/01 Lantern #2.mp3 [item-1⧸01 Lantern #2.mp3].mkv");
    let fake = Fake::default().info(
        "Lantern",
        r#"{"id": "item-1/01 Lantern #2.mp3", "extractor_key": "ArchiveOrg",
            "title": "Lantern", "uploader": "Marlo Venn"}"#,
    );
    let (ok, text) = s.run(&fake, &["add", &original]);
    assert!(ok, "{text}");
    let key = SourceKey::parse("archive.org:item-1/01 Lantern #2.mp3").unwrap();
    assert_eq!(s.songs(), [vec![key]], "{text}");
    assert!(!s.dir.path().join("home/sources/manual").exists(), "{text}");
}

#[test]
fn an_original_in_a_folder_goes_to_its_key_once_and_undo_takes_it_out() {
    let s = Setup::new();
    let original = s.file("Downloads/batch/chan/Song [aaaaaaaaaaa].mkv");
    s.file("Downloads/batch/b.flac");
    let fake = Fake::default().probe("b.flac", FLAC).info(
        "aaaaaaaaaaa",
        r#"{"id": "aaaaaaaaaaa", "extractor_key": "Youtube", "title": "Song", "uploader": "Chan"}"#,
    );
    let folder = s.dir.path().join("Downloads/batch");
    let (ok, text) = s.run(&fake, &["add", "-y", folder.to_str().unwrap()]);
    assert!(ok, "{text}");
    let home = s.dir.path().join("home/sources");
    let placed = home.join("yt-dlp/youtube.com/aaaaaaaaaaa.mkv");
    assert!(placed.exists(), "{text}");
    assert!(home.join("manual/batch/b.flac").exists(), "{text}");
    assert!(
        !home.join("manual/batch/chan").exists(),
        "no original in the manual folder: {text}"
    );
    let mut songs = s.songs();
    songs.sort();
    assert!(
        songs.contains(&vec![SourceKey::youtube("aaaaaaaaaaa")]),
        "{songs:?}"
    );

    let (ok, text) = s.run(&fake, &["add", &original]);
    assert!(ok, "{text}");
    assert!(
        text.contains("which sources/yt-dlp holds already"),
        "{text}"
    );
    assert_eq!(s.songs().len(), 2, "{text}");

    // The add that changed nothing is no run; undo takes the first back.
    let (ok, text) = s.run(&fake, &["undo", "-y"]);
    assert!(ok, "{text}");
    assert!(!placed.exists(), "{text}");
}

/// [`Fake`], but for yt-dlp's listing, which names one upload, and a
/// YouTube Music search, which finds nothing and is counted.
struct Listing(Fake, std::sync::atomic::AtomicUsize);

impl Runner for Listing {
    fn run(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<()> {
        self.0.run(cmd)
    }

    fn output(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<Vec<u8>> {
        let last = cmd.last().map(|a| a.to_string_lossy()).unwrap_or_default();
        if last.starts_with("https://music.youtube.com/search") {
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Ok(Vec::new());
        }
        if cmd.iter().any(|a| a == "--dump-single-json") {
            return Ok(br#"{"_type": "video", "id": "aaaaaaaaaaa", "extractor_key": "Youtube",
                "title": "Marlo Venn - Lantern Weather", "uploader": "Marlo Venn", "duration": 66}"#
                .to_vec());
        }
        self.0.output(cmd)
    }

    fn stream(
        &self,
        cmd: &[std::ffi::OsString],
        on_line: &mut dyn FnMut(crate::runner::Line<'_>),
    ) -> anyhow::Result<bool> {
        self.0.stream(cmd, on_line)
    }
}

#[test]
fn add_looks_no_track_up_on_youtube_music_while_it_is_off() {
    let s = Setup::new();
    let searched = |list: &str| {
        let home = s.dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(crate::dirs::MANIFEST), list).unwrap();
        let listing = Listing(Fake::default(), std::sync::atomic::AtomicUsize::new(0));
        let args = ["add", "https://www.youtube.com/watch?v=aaaaaaaaaaa"];
        let mut out = Vec::new();
        let _ = run_with(
            &s.job(&args),
            &listing,
            &Server::default(),
            None,
            &mut out,
            &mut Vec::new(),
        );
        listing.1.into_inner()
    };
    assert_eq!(searched("version = 1\n"), 1);
    assert_eq!(
        searched("version = 1\n[providers.youtube-music]\nenabled = false\n"),
        0
    );
}

#[test]
fn a_url_is_fetched_into_the_store_and_yt_dlp_is_never_shown_raw() {
    let s = Setup::new();
    let store = s.dir.path().join("home/sources/yt-dlp");
    let fake = Fake {
        lines: vec!["[youtube] aaaaaaaaaaa: Downloading webpage".into()],
        on_stream: Some(Box::new(move |args: &[String]| {
            let done = &args[args.iter().position(|a| a == "--print-to-file").unwrap() + 2];
            let path = store.join("chan/Song [aaaaaaaaaaa].mkv");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"mkv").unwrap();
            std::fs::write(
                done,
                format!(
                    "Youtube \"aaaaaaaaaaa\" {}\n",
                    serde_json::to_string(&path).unwrap()
                ),
            )
            .unwrap();
            true
        })),
        ..Fake::default()
    };
    let (ok, text) = s.run(
        &fake,
        &[
            "add",
            "--no-match",
            "https://www.youtube.com/watch?v=aaaaaaaaaaa",
        ],
    );
    assert!(ok, "{text}");
    assert_eq!(s.songs(), [vec![SourceKey::youtube("aaaaaaaaaaa")]]);
    assert!(
        text.contains("yt-dlp: [youtube] aaaaaaaaaaa: Downloading webpage"),
        "{text}"
    );
    assert!(text.contains("Added:"), "{text}");
}

#[test]
fn a_title_or_a_torrent_s_name_cannot_drive_the_terminal() {
    let s = Setup::new();
    let said = |fake: &Fake, args: &[&str]| {
        let mut out = anstream::AutoStream::new(Vec::new(), anstream::ColorChoice::Always);
        run_with(
            &s.job(args),
            fake,
            &Server::default(),
            None,
            &mut out,
            &mut Vec::new(),
        )
        .unwrap();
        String::from_utf8(out.into_inner()).unwrap()
    };
    let fetched = Fake {
        lines: vec![
            "[download] \x1b]0;X\x07Paper Comets".into(),
            "ERROR: [youtube] aaaaaaaaaaa: \x1b]0;X\x07Gone".into(),
        ],
        on_stream: Some(Box::new(|_: &[String]| false)),
        ..Fake::default()
    };
    let url = "https://www.youtube.com/watch?v=aaaaaaaaaaa";
    let ytdlp = said(&fetched, &["add", "--no-match", url]);
    let file = s.dir.path().join("a.torrent");
    let bytes = crate::testing::metainfo("Lantern\x1b]0;X\x07Weather", &["a.flac"]);
    std::fs::write(&file, bytes).unwrap();
    let seeded = Fake {
        lines: vec!["Download Results:".into()],
        on_stream: Some(Box::new(|_: &[String]| false)),
        ..Fake::default()
    };
    let aria2c = said(&seeded, &["add", "--no-match", file.to_str().unwrap()]);
    for text in [&ytdlp, &aria2c] {
        assert!(
            !text.contains("\x1b]") && !text.contains('\x07'),
            "{text:?}"
        );
        assert!(
            text.contains("\x1b[31m"),
            "muman's own colours stay: {text:?}"
        );
    }
    let ytdlp = ui::plain(&ytdlp);
    assert!(
        ytdlp.contains("yt-dlp: [download] �]0;X�Paper Comets"),
        "{ytdlp}"
    );
    assert!(
        ytdlp.contains("yt-dlp: [youtube] aaaaaaaaaaa: �]0;X�Gone"),
        "{ytdlp}"
    );
    assert!(ui::plain(&aria2c).contains("Weather"), "{aria2c:?}");
}

/// The downloads of the input file an aria2c run of `args` reads, each
/// its address, then its options as its command line would take them.
fn input_of(args: &[String]) -> Vec<Vec<String>> {
    let Some(path) = args.iter().find_map(|a| a.strip_prefix("--input-file=")) else {
        return Vec::new();
    };
    let mut entries: Vec<Vec<String>> = Vec::new();
    for line in std::fs::read_to_string(path).unwrap().lines() {
        match (line.strip_prefix("  "), entries.last_mut()) {
            (Some(option), Some(entry)) => entry.push(format!("--{option}")),
            _ => entries.push(vec![line.to_string()]),
        }
    }
    entries
}

/// A fake aria2c: a metainfo run saves the metainfo of each torrent of
/// `torrents`, by hash, its magnet link names, as its peers would send
/// it, and a fetch writes each file it is told to write, `runs` each
/// command line recorded with its input file's downloads after it.
fn aria2c_of(torrents: Vec<(String, Vec<u8>)>, runs: Arc<Mutex<Vec<Vec<String>>>>) -> Fake {
    Fake {
        on_stream: Some(Box::new(move |args: &[String]| {
            let entries = input_of(args);
            let mut seen = args.to_vec();
            seen.extend(entries.iter().flatten().cloned());
            runs.lock().unwrap().push(seen);
            let option = |of: &[String], name: &str| {
                of.iter()
                    .find_map(|a| a.strip_prefix(&format!("--{name}=")))
                    .map(PathBuf::from)
            };
            if args.iter().any(|a| a == "--bt-metadata-only=true") {
                let dir = option(args, "dir").unwrap();
                std::fs::create_dir_all(&dir).unwrap();
                for entry in &entries {
                    for (hash, metainfo) in &torrents {
                        if entry[0].contains(hash.as_str()) {
                            std::fs::write(dir.join(format!("{hash}.torrent")), metainfo).unwrap();
                        }
                    }
                }
                return true;
            }
            for entry in &entries {
                let dir = option(entry, "dir").unwrap();
                for out in entry.iter().filter_map(|a| a.strip_prefix("--index-out=")) {
                    let (_, path) = out.split_once('=').unwrap();
                    let path = dir.join(path);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, b"x").unwrap();
                }
            }
            true
        })),
        ..Fake::default()
    }
    .probe(".flac", FLAC)
}

/// [`aria2c_of`] the one torrent of `hash`.
fn aria2c(metainfo: Vec<u8>, hash: String, runs: Arc<Mutex<Vec<Vec<String>>>>) -> Fake {
    aria2c_of(vec![(hash, metainfo)], runs)
}

#[test]
fn a_torrents_song_files_are_fetched_and_listed_and_the_rest_left() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo(
        "Lantern Weather",
        &[
            "01 Paper Comets.flac",
            "02 Rooms of Salt.flac",
            "cover.jpg",
            "notes.nfo",
        ],
    );
    let meta = crate::torrent::Metainfo::read(&bytes).unwrap();
    let hash = meta.hash().to_string();
    let file = s.dir.path().join("Lantern Weather.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let runs = Arc::new(Mutex::new(Vec::new()));
    let fake = aria2c(bytes, hash.clone(), runs.clone());
    let (ok, text) = s.run(&fake, &["add", "--no-match", file.to_str().unwrap()]);
    assert!(ok, "{text}");
    assert!(text.contains("Fetching 3 of 4 file(s)"), "{text}");
    let key = |p: &str| SourceKey::parse(&format!("torrent:{hash}/{p}")).unwrap();
    let mut songs = s.songs();
    songs.sort();
    assert_eq!(
        songs,
        [
            vec![key("01 Paper Comets.flac")],
            vec![key("02 Rooms of Salt.flac")]
        ]
    );
    let runs = runs.lock().unwrap();
    assert_eq!(runs.len(), 1);
    assert!(
        runs[0].contains(&"--select-file=1,2,3".to_string()),
        "{:?}",
        runs[0]
    );
    let torrents = s.dir.path().join("home/sources/torrent");
    assert!(torrents.join(format!("{hash}.torrent")).is_file());
    assert!(torrents.join(&hash).join("cover.jpg").is_file());
    assert!(!torrents.join(&hash).join("notes.nfo").exists());
    assert!(
        !s.dir
            .path()
            .join("home/partial/torrent")
            .join(&hash)
            .exists()
    );
    let manifest = Manifest::load(&s.dir.path().join("home")).unwrap();
    assert_eq!(
        manifest.torrents[&hash].name.as_deref(),
        Some("Lantern Weather")
    );
    assert!(
        std::fs::read_to_string(s.dir.path().join("home/songs.toml"))
            .unwrap()
            .contains("version = 4")
    );
}

#[test]
fn a_torrents_file_kept_at_another_files_place_is_left_and_said() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo(
        "Lantern Weather",
        &["01 Paper Comets?.flac", "01 Paper Comets？.flac"],
    );
    let meta = crate::torrent::Metainfo::read(&bytes).unwrap();
    let hash = meta.hash().to_string();
    let file = s.dir.path().join("Lantern Weather.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let runs = Arc::new(Mutex::new(Vec::new()));
    let fake = aria2c(bytes, hash.clone(), runs.clone());
    let (ok, text) = s.run(&fake, &["add", "--no-match", file.to_str().unwrap()]);
    assert!(ok, "{text}");
    assert!(
        text.contains("01 Paper Comets？.flac is not fetched: some systems keep it at the place of 01 Paper Comets?.flac"),
        "{text}"
    );
    let key = SourceKey::parse(&format!("torrent:{hash}/01 Paper Comets?.flac")).unwrap();
    assert_eq!(s.songs(), [vec![key]]);
    assert!(
        runs.lock().unwrap()[0].contains(&"--select-file=1".to_string()),
        "{:?}",
        runs.lock().unwrap()
    );
}

#[test]
fn a_torrents_file_keyed_as_another_is_left_and_the_other_written() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("Lantern Weather", &["a:b.flac", "a：b.flac"]);
    let meta = crate::torrent::Metainfo::read(&bytes).unwrap();
    let hash = meta.hash().to_string();
    let file = s.dir.path().join("Lantern Weather.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let runs = Arc::new(Mutex::new(Vec::new()));
    let fake = aria2c(bytes, hash.clone(), runs.clone());
    let (ok, text) = s.run(&fake, &["add", "--no-match", file.to_str().unwrap()]);
    assert!(ok, "{text}");
    assert!(
        text.contains("a：b.flac is not fetched: its key is spelled as that of a:b.flac"),
        "{text}"
    );
    let key = SourceKey::parse(&format!("torrent:{hash}/a：b.flac")).unwrap();
    assert_eq!(s.songs(), [vec![key]]);
    let built = crate::store::files_below(&s.dir.path().join("lib"), 9, |_| true).unwrap();
    assert_eq!(built.len(), 1, "{text}");
    let (ok, text) = s.run(&fake, &["sync", "--offline"]);
    assert!(ok, "{text}");
}

#[test]
fn a_torrents_files_stay_sources_when_its_metainfo_is_gone() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("Lantern Weather", &["01 Paper Comets.flac", "cover.jpg"]);
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let file = s.dir.path().join("Lantern Weather.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let fake = aria2c(bytes, hash.clone(), Arc::default());
    let (ok, text) = s.run(&fake, &["add", "--no-match", file.to_str().unwrap()]);
    assert!(ok, "{text}");
    let torrents = s.dir.path().join("home/sources/torrent");
    std::fs::remove_file(torrents.join(format!("{hash}.torrent"))).unwrap();

    let (ok, text) = s.run(&fake, &["status"]);
    assert!(ok, "{text}");
    let (ok, report) = s.report(&fake, &["check"]);
    assert!(!ok, "{report}");
    assert!(
        report.contains(&format!("The metainfo of torrent {hash} is gone")),
        "{report}"
    );
    assert!(!report.contains("Missing from sources"), "{report}");
    let (_, text) = s.run(&fake, &["purge", "-y"]);
    assert!(
        torrents.join(&hash).join("01 Paper Comets.flac").is_file(),
        "{text}"
    );
    assert!(torrents.join(&hash).join("cover.jpg").is_file(), "{text}");
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
}

#[test]
fn a_torrents_file_named_as_a_drive_is_kept_by_a_lookalike_not_refused() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("X: The Remixes", &["a:b.flac", "Plain.flac"]);
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let file = s.dir.path().join("a.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let fake = aria2c(bytes, hash.clone(), Arc::new(Mutex::new(Vec::new())));
    let (ok, text) = s.run(&fake, &["add", "--no-match", file.to_str().unwrap()]);
    assert!(ok, "{text}");
    let key = |p: &str| SourceKey::parse(&format!("torrent:{hash}/{p}")).unwrap();
    let mut songs = s.songs();
    songs.sort();
    assert_eq!(songs, [vec![key("Plain.flac")], vec![key("a：b.flac")]]);
    let torrents = s.dir.path().join("home/sources/torrent");
    assert!(torrents.join(&hash).join("a：b.flac").is_file());
}

#[test]
fn a_torrent_downloaded_already_is_taken_up_from_its_folder() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("Lantern Weather", &["Paper Comets.flac", "scan.png"]);
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let file = s.dir.path().join("a.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let downloads = s.dir.path().join("downloads");
    s.written("downloads/Lantern Weather/Paper Comets.flac", "x");
    s.written("downloads/Lantern Weather/scan.png", "y");
    let partial = s.dir.path().join("home/partial/torrent").join(&hash);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let saw = seen.clone();
    let fake = Fake {
        on_stream: Some(Box::new(move |_: &[String]| {
            saw.lock().unwrap().extend([
                std::fs::read(partial.join("files").join("1.flac")).ok(),
                std::fs::read(partial.join("files").join("2.png")).ok(),
            ]);
            true
        })),
        ..Fake::default()
    }
    .probe(".flac", FLAC);
    let (ok, text) = s.run(
        &fake,
        &[
            "add",
            "--no-match",
            "--from",
            downloads.to_str().unwrap(),
            file.to_str().unwrap(),
        ],
    );
    assert!(ok, "{text}");
    assert!(
        text.contains("Taking up 2 file(s) of Lantern Weather"),
        "{text}"
    );
    assert_eq!(
        *seen.lock().unwrap(),
        [Some(b"x".to_vec()), Some(b"y".to_vec())],
        "the song, and the scan sharing its piece, copied in before aria2c checks them"
    );
    assert_eq!(s.songs().len(), 1, "{text}");
}

#[test]
fn a_magnet_links_metainfo_comes_from_its_peers() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("The Glass Orchards", &["Rooms of Salt.flac"]);
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let runs = Arc::new(Mutex::new(Vec::new()));
    let fake = aria2c(bytes, hash.clone(), runs.clone());
    let link = format!("magnet:?xt=urn:btih:{hash}&tr=udp%3A%2F%2Ftracker.example%3A1337");
    let (ok, text) = s.run(&fake, &["add", "--no-match", &link]);
    assert!(ok, "{text}");
    assert_eq!(
        runs.lock().unwrap().len(),
        2,
        "the metainfo, then the files"
    );
    assert_eq!(s.songs().len(), 1, "{text}");
    let manifest = Manifest::load(&s.dir.path().join("home")).unwrap();
    assert_eq!(
        manifest.torrents[&hash].trackers,
        ["udp://tracker.example:1337"]
    );
    let (ok, text) = s.run(&fake, &["add", "--no-match", &link]);
    assert!(ok, "{text}");
    assert_eq!(
        runs.lock().unwrap().len(),
        2,
        "its metainfo is kept: {text}"
    );

    let e = run_with(
        &s.job(&["add", "--offline", &link]),
        &fake,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e}").contains("--offline"), "{e:#}");
    assert_eq!(runs.lock().unwrap().len(), 2, "nothing run offline");
}

#[test]
fn sync_fetches_a_torrents_file_gone_from_the_store_again() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo(
        "Lantern Weather",
        &["01 Paper Comets.flac", "02 Rooms of Salt.flac", "cover.jpg"],
    );
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let link = format!("magnet:?xt=urn:btih:{hash}&tr=udp%3A%2F%2Ftracker.example%3A1337");
    let runs = Arc::new(Mutex::new(Vec::new()));
    let fake = aria2c(bytes, hash.clone(), runs.clone());
    let (ok, text) = s.run(&fake, &["add", "--no-match", &link]);
    assert!(ok, "{text}");
    let mut manifest = Manifest::load(&s.dir.path().join("home")).unwrap();
    manifest.edit(crate::manifest::Edit::Remove {
        key: SourceKey::parse(&format!("torrent:{hash}/02 Rooms of Salt.flac")).unwrap(),
        note: "Rooms of Salt".into(),
    });
    manifest.save().unwrap();
    let torrents = s.dir.path().join("home/sources/torrent");
    let comets = torrents.join(&hash).join("01 Paper Comets.flac");
    let salt = torrents.join(&hash).join("02 Rooms of Salt.flac");
    std::fs::remove_file(&comets).unwrap();
    std::fs::remove_file(torrents.join(&hash).join("cover.jpg")).unwrap();
    std::fs::remove_file(&salt).unwrap();
    std::fs::remove_file(torrents.join(format!("{hash}.torrent"))).unwrap();
    runs.lock().unwrap().clear();

    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Fetching 1 missing torrent file(s) again"),
        "{text}"
    );
    let runs = runs.lock().unwrap();
    assert_eq!(runs.len(), 2, "its metainfo from its peers, then the file");
    assert!(
        runs[0].iter().any(|a| a.contains("tracker.example")),
        "{:?}",
        runs[0]
    );
    assert!(
        runs[1].contains(&"--select-file=1,3".to_string()),
        "{:?}",
        runs[1]
    );
    assert!(comets.is_file() && torrents.join(format!("{hash}.torrent")).is_file());
    assert!(!salt.exists(), "a file no song lists is not fetched again");
}

/// Two torrents of one file each, as `add` is given them: each its
/// hash and metainfo, and the `.torrent` file written for it.
fn two_torrents(s: &Setup) -> [(String, Vec<u8>, PathBuf); 2] {
    ["Lantern Weather", "The Glass Orchards"].map(|name| {
        let bytes = crate::testing::metainfo(name, &["a.flac"]);
        let hash = crate::torrent::Metainfo::read(&bytes)
            .unwrap()
            .hash()
            .to_string();
        let file = s.dir.path().join(format!("{name}.torrent"));
        std::fs::write(&file, &bytes).unwrap();
        (hash, bytes, file)
    })
}

#[test]
fn sync_asks_for_every_lost_metainfo_in_one_run_and_every_file_in_another() {
    let s = Setup::new();
    let [a, b] = two_torrents(&s);
    let runs = Arc::new(Mutex::new(Vec::new()));
    let fake = aria2c_of(
        vec![(a.0.clone(), a.1.clone()), (b.0.clone(), b.1.clone())],
        runs.clone(),
    );
    let (ok, text) = s.run(
        &fake,
        &[
            "add",
            "--no-match",
            a.2.to_str().unwrap(),
            b.2.to_str().unwrap(),
        ],
    );
    assert!(ok, "{text}");
    let torrents = s.dir.path().join("home/sources/torrent");
    for hash in [&a.0, &b.0] {
        std::fs::remove_file(torrents.join(hash).join("a.flac")).unwrap();
        std::fs::remove_file(torrents.join(format!("{hash}.torrent"))).unwrap();
    }
    runs.lock().unwrap().clear();

    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Fetching 2 missing torrent file(s) again"),
        "{text}"
    );
    let runs = runs.lock().unwrap();
    assert_eq!(
        runs.len(),
        2,
        "the metainfo of both, then the files of both"
    );
    assert!(runs[0].contains(&"--bt-metadata-only=true".to_string()));
    for hash in [&a.0, &b.0] {
        assert!(runs[0].iter().any(|r| r.contains(&format!("btih:{hash}"))));
        assert!(
            runs[1]
                .iter()
                .any(|r| r.ends_with(&format!("{hash}.torrent")))
        );
        assert!(torrents.join(hash).join("a.flac").is_file());
    }
}

#[test]
fn torrents_given_together_are_fetched_by_one_aria2c() {
    let s = Setup::new();
    let [a, b] = two_torrents(&s);
    let runs = Arc::new(Mutex::new(Vec::new()));
    let fake = aria2c_of(
        vec![(a.0.clone(), a.1.clone()), (b.0.clone(), b.1.clone())],
        runs.clone(),
    );
    let (ok, text) = s.run(
        &fake,
        &[
            "add",
            "--no-match",
            a.2.to_str().unwrap(),
            b.2.to_str().unwrap(),
        ],
    );
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 2, "{text}");
    let runs = runs.lock().unwrap();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert!(runs[0].contains(&"--max-concurrent-downloads=2".to_string()));
    for hash in [&a.0, &b.0] {
        assert!(
            runs[0]
                .iter()
                .any(|r| r.ends_with(&format!("{hash}.torrent"))),
            "{runs:?}"
        );
    }
}

#[test]
fn a_torrent_no_peer_seeds_fails_alone_and_the_one_beside_it_arrives() {
    let s = Setup::new();
    let [a, b] = two_torrents(&s);
    let fetched = aria2c_of(
        vec![(a.0.clone(), a.1.clone()), (b.0.clone(), b.1.clone())],
        Arc::default(),
    );
    let fake = Fake {
        lines: vec![
            "Download Results:".into(),
            "000001|INPR|       0B/s|  0|partial/files/1.flac".into(),
            "000002|OK  |   1.0MiB/s|100|partial/files/1.flac".into(),
        ],
        on_stream: Some(Box::new(move |args: &[String]| {
            (fetched.on_stream.as_ref().unwrap())(args);
            false
        })),
        ..Fake::default()
    }
    .probe(".flac", FLAC);
    let (ok, text) = s.run(
        &fake,
        &[
            "add",
            "--no-match",
            a.2.to_str().unwrap(),
            b.2.to_str().unwrap(),
        ],
    );
    assert!(!ok, "{text}");
    assert!(
        text.contains("Lantern Weather: no peer sent anything for 10 min; what arrived waits to be resumed; giving it to `add` again goes on"),
        "{text}"
    );
    let key = SourceKey::parse(&format!("torrent:{}/a.flac", b.0)).unwrap();
    assert_eq!(s.songs(), [vec![key]], "{text}");
}

#[test]
fn a_torrent_unfinished_keeps_each_file_whose_pieces_check() {
    use sha1::Digest as _;
    let s = Setup::new();
    let pieces: Vec<u8> = [b"x", b"y"]
        .iter()
        .flat_map(|p| sha1::Sha1::digest(p).to_vec())
        .collect();
    let file = |name: &str| format!("d6:lengthi1e4:pathl{}:{name}ee", name.len());
    let mut bytes = format!(
        "d4:infod5:filesl{}{}e4:name15:Lantern Weather12:piece lengthi1e6:pieces40:",
        file("a.flac"),
        file("b.flac")
    )
    .into_bytes();
    bytes.extend(pieces);
    bytes.extend(b"ee");
    let meta = crate::torrent::Metainfo::read(&bytes).unwrap();
    let hash = meta.hash().to_string();
    let path = s.dir.path().join("Lantern Weather.torrent");
    std::fs::write(&path, &bytes).unwrap();
    let fetched = aria2c(bytes, hash.clone(), Arc::default());
    let fake = Fake {
        lines: vec![
            "Download Results:".into(),
            "000001|INPR|       0B/s| 50|partial/files/1.flac".into(),
        ],
        on_stream: Some(Box::new(move |args: &[String]| {
            (fetched.on_stream.as_ref().unwrap())(args);
            false
        })),
        ..Fake::default()
    }
    .probe(".flac", FLAC);
    let (ok, text) = s.run(&fake, &["add", "--no-match", path.to_str().unwrap()]);
    assert!(!ok, "{text}");
    assert!(
        text.contains("Lantern Weather: taking the 1 file(s) that arrived whole"),
        "{text}"
    );
    let key = SourceKey::parse(&format!("torrent:{hash}/a.flac")).unwrap();
    assert_eq!(s.songs(), [vec![key]], "{text}");
}

/// A runner as its fake, with no aria2c to start.
struct NoAria2c(Fake);

impl crate::runner::Runner for NoAria2c {
    fn run(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<()> {
        self.0.run(cmd)
    }

    fn output(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<Vec<u8>> {
        self.0.output(cmd)
    }

    fn pipe(
        &self,
        cmd: &[std::ffi::OsString],
        on_bytes: &mut dyn FnMut(&[u8]),
    ) -> anyhow::Result<()> {
        self.0.pipe(cmd, on_bytes)
    }

    fn fingerprint(&self, pcm: &std::path::Path) -> anyhow::Result<Vec<u32>> {
        self.0.fingerprint(pcm)
    }

    fn stream(
        &self,
        cmd: &[std::ffi::OsString],
        on_line: &mut dyn FnMut(crate::runner::Line<'_>),
    ) -> anyhow::Result<bool> {
        if cmd[0] == "aria2c" {
            return Err(crate::tools::ToolError::Missing {
                tool: &crate::tools::TOOLS[3],
                named: None,
            }
            .into());
        }
        self.0.stream(cmd, on_line)
    }
}

#[test]
fn an_add_of_a_torrent_with_no_aria2c_stops_as_a_missing_tool() {
    let s = Setup::new();
    let [a, _] = two_torrents(&s);
    let fake = NoAria2c(aria2c_of(vec![(a.0.clone(), a.1.clone())], Arc::default()));
    let mut out = Vec::new();
    let ran = run_with(
        &s.job(&["add", "--no-match", a.2.to_str().unwrap()]),
        &fake,
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    );
    let e = ran.unwrap_err();
    assert!(
        e.downcast_ref::<crate::tools::ToolError>().is_some(),
        "{e:#}"
    );
    assert_eq!(s.songs(), Vec::<Vec<SourceKey>>::new());
}

#[test]
fn a_sync_with_no_aria2c_fails_each_torrent_file_and_goes_on() {
    let s = Setup::new();
    let [a, b] = two_torrents(&s);
    let fake = aria2c_of(
        vec![(a.0.clone(), a.1.clone()), (b.0.clone(), b.1.clone())],
        Arc::default(),
    );
    let given = [a.2.to_str().unwrap(), b.2.to_str().unwrap()];
    let (ok, text) = s.run(&fake, &["add", "--no-match", given[0], given[1]]);
    assert!(ok, "{text}");
    let torrents = s.dir.path().join("home/sources/torrent");
    std::fs::remove_file(torrents.join(&a.0).join("a.flac")).unwrap();
    std::fs::remove_file(torrents.join(&b.0).join("a.flac")).unwrap();
    std::fs::remove_file(torrents.join(format!("{}.torrent", b.0))).unwrap();

    let mut out = Vec::new();
    let ok = run_with(
        &s.job(&["sync"]),
        &NoAria2c(Fake::default().probe(".flac", FLAC)),
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!ok, "{text}");
    for hash in [&a.0, &b.0] {
        assert!(
            text.contains(&format!("Could not fetch torrent:{hash}/a.flac again:")),
            "{text}"
        );
    }
    assert_eq!(text.matches("aria2c not found").count(), 2, "{text}");
    let (_, text) = s.run(&fake, &["sync"]);
    assert!(
        text.contains("Not fetching 2 missing torrent file(s) that failed lately"),
        "{text}"
    );
}

#[test]
fn a_sync_fetches_torrents_beside_yt_dlp() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("Lantern Weather", &["a.flac"]);
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let file = s.dir.path().join("a.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let fetch = aria2c(bytes, hash.clone(), Arc::default());
    let (ok, text) = s.run(&fetch, &["add", "--no-match", file.to_str().unwrap()]);
    assert!(ok, "{text}");
    let song = s
        .dir
        .path()
        .join("home/sources/torrent")
        .join(&hash)
        .join("a.flac");
    std::fs::remove_file(&song).unwrap();
    let list = s.dir.path().join("home/songs.toml");
    let mut text = std::fs::read_to_string(&list).unwrap();
    text.push_str("\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    std::fs::write(&list, text).unwrap();

    // Each run waits for the other to start; one after the other, the
    // first would wait in vain.
    let started = Arc::new((Mutex::new([false; 2]), std::sync::Condvar::new()));
    let overlapped = Arc::new(Mutex::new(Vec::new()));
    let saw = overlapped.clone();
    let fake = Fake {
        on_stream: Some(Box::new(move |args: &[String]| {
            let me = usize::from(args[0] == "yt-dlp");
            let (lock, arrived) = &*started;
            let mut seen = lock.lock().unwrap();
            seen[me] = true;
            arrived.notify_all();
            let (seen, wait) = arrived
                .wait_timeout_while(seen, std::time::Duration::from_secs(10), |s| !s[1 - me])
                .unwrap();
            drop(seen);
            saw.lock()
                .unwrap()
                .push((args[0].clone(), !wait.timed_out()));
            me == 0 && (fetch.on_stream.as_ref().unwrap())(args)
        })),
        ..Fake::default()
    }
    .probe(".flac", FLAC);
    let (_, text) = s.run(&fake, &["sync"]);
    let mut overlapped = overlapped.lock().unwrap().clone();
    overlapped.sort();
    assert_eq!(
        overlapped,
        [("aria2c".to_string(), true), ("yt-dlp".to_string(), true)],
        "{text}"
    );
    assert!(song.is_file(), "{text}");
}

#[test]
fn a_torrents_file_that_failed_to_come_again_waits() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("Lantern Weather", &["a.flac"]);
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let file = s.dir.path().join("a.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let fake = aria2c(bytes, hash.clone(), Arc::default());
    let (ok, text) = s.run(&fake, &["add", "--no-match", file.to_str().unwrap()]);
    assert!(ok, "{text}");
    std::fs::remove_file(
        s.dir
            .path()
            .join("home/sources/torrent")
            .join(&hash)
            .join("a.flac"),
    )
    .unwrap();
    let stalled = Fake {
        lines: vec!["Download Results:".into(), "000001|INPR|0B/s|-|a".into()],
        on_stream: Some(Box::new(|_: &[String]| false)),
        ..Fake::default()
    };
    let (ok, text) = s.run(&stalled, &["sync"]);
    assert!(!ok, "{text}");
    assert!(text.contains("no peer sent anything"), "{text}");
    let (_, text) = s.run(&stalled, &["sync"]);
    assert!(
        text.contains("Not fetching 1 missing torrent file(s)"),
        "{text}"
    );
}

#[test]
fn a_torrent_no_peer_seeds_fails_and_lists_nothing() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("Lantern Weather", &["a.flac"]);
    let file = s.dir.path().join("a.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let fake = Fake {
        lines: vec![
            "Download Results:".into(),
            "000001|INPR|       0B/s|  -|partial/files/1.flac".into(),
        ],
        on_stream: Some(Box::new(|_: &[String]| false)),
        ..Fake::default()
    };
    let (ok, text) = s.run(&fake, &["add", "--no-match", file.to_str().unwrap()]);
    assert!(!ok, "{text}");
    assert!(text.contains("no peer sent anything for 10 min"), "{text}");
    assert!(s.songs().is_empty(), "{text}");
}

#[test]
fn a_fetched_source_keeps_none_of_the_cookies_it_was_fetched_with() {
    let s = Setup::new();
    let fake = Fake {
        on_stream: Some(Box::new(move |args: &[String]| {
            let done = &args[args.iter().position(|a| a == "--print-to-file").unwrap() + 2];
            let staging = &args[args.iter().position(|a| a == "--paths").unwrap() + 1];
            let path = Path::new(staging).join("Youtube/aaaaaaaaaaa.mkv");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"mkv").unwrap();
            std::fs::write(
                done,
                format!(
                    "Youtube \"aaaaaaaaaaa\" {}\n",
                    serde_json::to_string(&path).unwrap()
                ),
            )
            .unwrap();
            true
        })),
        ..Fake::default()
    }
    .info(
        "aaaaaaaaaaa",
        r#"{"id": "aaaaaaaaaaa", "extractor_key": "Youtube", "title": "Song",
            "cookies": "SID=TOKEN-0001", "http_headers": {"User-Agent": "Agent/1"}}"#,
    );
    let (ok, text) = s.run(
        &fake,
        &[
            "add",
            "--no-match",
            "https://www.youtube.com/watch?v=aaaaaaaaaaa",
        ],
    );
    assert!(ok, "{text}");
    let attached = fake.attached.lock().unwrap();
    assert_eq!(attached.len(), 1, "{text}");
    assert!(
        !attached[0].contains("TOKEN") && !attached[0].contains("Agent/1"),
        "{attached:?}"
    );
    assert!(
        s.dir
            .path()
            .join("home/sources/yt-dlp/youtube.com/aaaaaaaaaaa.mkv")
            .exists(),
        "{text}"
    );
}

#[test]
fn a_download_that_does_not_read_is_fetched_again() {
    let s = Setup::new();
    s.written(
        "home/songs.toml",
        "version = 1\n\
         [providers.youtube-music]\nenabled = false\n\
         [providers.youtube]\nenabled = false\n\
         [providers.lrclib]\nenabled = false\n\
         [providers.acoustid]\nenabled = false\n\
         [providers.musicbrainz]\nenabled = false\n\
         [providers.coverart]\nenabled = false\n\
         [[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n",
    );
    s.written("home/sources/yt-dlp/youtube.com/aaaaaaaaaaa.mkv", "garbage");
    let info =
        r#"{"id": "aaaaaaaaaaa", "extractor_key": "Youtube", "title": "Song", "uploader": "Chan"}"#;
    let _ = run_with(
        &s.job(&["sync", "--offline"]),
        &Unprobed(Fake::default().info("aaaaaaaaaaa", info)),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    let key = SourceKey::youtube("aaaaaaaaaaa");
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    assert_eq!(
        state.failures.get(&key).map(|f| f.step),
        Some(crate::state::Step::Measure)
    );
    let mut out = Vec::new();
    let _ = run_with(
        &s.job(&["sync"]),
        &Fake::default().info("aaaaaaaaaaa", info),
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    );
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("damaged source(s) again"), "{text}");
}

#[test]
fn a_damaged_download_is_fetched_again_and_its_whole_copy_taken() {
    let s = Setup::new();
    s.written(
        "home/songs.toml",
        "version = 1\n\
         [providers.youtube-music]\nenabled = false\n\
         [providers.youtube]\nenabled = false\n\
         [providers.lrclib]\nenabled = false\n\
         [providers.acoustid]\nenabled = false\n\
         [providers.musicbrainz]\nenabled = false\n\
         [providers.coverart]\nenabled = false\n\
         [[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n",
    );
    s.written(
        "home/sources/yt-dlp/youtube.com/aaaaaaaaaaa.mkv",
        "a copy with holes",
    );
    let info =
        r#"{"id": "aaaaaaaaaaa", "extractor_key": "Youtube", "title": "Song", "uploader": "Chan"}"#;
    // Three seconds lost from its middle, as a download that lost pieces.
    let mut holed = "#tb 0: 1/1000\n".to_string();
    for n in (0..100).filter(|n| !(20..70).contains(n)) {
        let pts = n * 60;
        let _ = writeln!(holed, "0, {pts}, {pts}, 60, 300, 0x0");
    }
    let damaged = Fake {
        timelines: vec![("aaaaaaaaaaa".into(), holed)],
        hashes: vec![("aaaaaaaaaaa".into(), 1)],
        ..Fake::default()
    }
    .info("aaaaaaaaaaa", info);
    let (_, text) = s.run(&damaged, &["sync", "--offline"]);
    assert!(text.contains("lacks"), "{text}");
    let key = SourceKey::youtube("aaaaaaaaaaa");
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    assert_eq!(
        state.failures.get(&key).map(|f| f.step),
        Some(crate::state::Step::Refetch),
        "{text}"
    );
    assert!(state.damaged.contains_key(&key));

    // Fetched again, whole: taken without `--accept`.
    let whole = Fake {
        hashes: vec![("aaaaaaaaaaa".into(), 2)],
        on_stream: Some(Box::new(move |args: &[String]| {
            let done = &args[args.iter().position(|a| a == "--print-to-file").unwrap() + 2];
            let staging = &args[args.iter().position(|a| a == "--paths").unwrap() + 1];
            let staged = Path::new(staging).join("Youtube/aaaaaaaaaaa.mkv");
            std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
            std::fs::write(&staged, b"a whole copy").unwrap();
            let line = format!(
                "Youtube \"aaaaaaaaaaa\" {}\n",
                serde_json::to_string(&staged).unwrap()
            );
            std::fs::write(done, line).unwrap();
            true
        })),
        ..Fake::default()
    }
    .info("aaaaaaaaaaa", info);
    let (ok, text) = s.run(&whole, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Fetching 1 damaged source(s) again"),
        "{text}"
    );
    assert!(
        text.contains("taken, a whole copy in place of a damaged one"),
        "{text}"
    );
    assert!(text.contains("Updated (audio"), "{text}");
    let (ok, again) = s.run(&whole, &["sync"]);
    assert!(ok && !again.contains("Fetching"), "{again}");
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    assert!(!state.failures.contains_key(&key) && state.damaged.is_empty());
}

#[test]
fn a_torrent_file_holding_other_bytes_is_damaged() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("Lantern Weather", &["Paper Comets.flac"]);
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let key = SourceKey::parse(&format!("torrent:{hash}/Paper Comets.flac")).unwrap();
    s.written(
        "home/songs.toml",
        &format!(
            "version = 4\n\
             [providers.lrclib]\nenabled = false\n\
             [providers.acoustid]\nenabled = false\n\
             [providers.musicbrainz]\nenabled = false\n\
             [providers.coverart]\nenabled = false\n\
             [[song]]\nsources = [\"{key}\"]\n"
        ),
    );
    let torrents = s.dir.path().join("home/sources/torrent");
    std::fs::create_dir_all(&torrents).unwrap();
    std::fs::write(torrents.join(format!("{hash}.torrent")), &bytes).unwrap();
    let at = format!("home/sources/torrent/{hash}/Paper Comets.flac");
    s.written(&at, "as fetched");
    let hashed = |n| {
        Fake {
            hashes: vec![("Paper Comets".into(), n)],
            ..Fake::default()
        }
        .probe(".flac", FLAC)
    };
    let (ok, text) = s.run(&hashed(1), &["sync", "--offline"]);
    assert!(ok, "{text}");
    s.written(&at, "eight bytes overwritten");
    let (_, text) = s.run(&hashed(3), &["sync", "--offline"]);
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    assert_eq!(
        state.failures.get(&key).map(|f| f.step),
        Some(crate::state::Step::Refetch),
        "{text}"
    );
    assert!(!text.contains("Updated"), "{text}");
}

#[test]
fn the_sync_that_finds_a_torrent_file_damaged_fetches_it_again() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("Lantern Weather", &["Paper Comets.flac"]);
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let file = s.dir.path().join("Lantern Weather.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let fake = aria2c(bytes, hash.clone(), Arc::default());
    let (ok, text) = s.run(&fake, &["add", "--no-match", file.to_str().unwrap()]);
    assert!(ok, "{text}");
    let stored = s
        .dir
        .path()
        .join("home/sources/torrent")
        .join(&hash)
        .join("Paper Comets.flac");
    std::fs::write(&stored, b"y").unwrap();

    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Fetching 1 damaged torrent file(s) again"),
        "{text}"
    );
    assert_eq!(std::fs::read(&stored).unwrap(), b"x", "{text}");
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    assert!(state.failures.is_empty(), "{text}");
}

#[test]
fn a_damaged_torrent_file_is_repaired_from_its_peers_and_taken() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("Lantern Weather", &["Paper Comets.flac"]);
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let key = SourceKey::parse(&format!("torrent:{hash}/Paper Comets.flac")).unwrap();
    s.written(
        "home/songs.toml",
        &format!(
            "version = 4\n\
             [providers.lrclib]\nenabled = false\n\
             [providers.acoustid]\nenabled = false\n\
             [providers.musicbrainz]\nenabled = false\n\
             [providers.coverart]\nenabled = false\n\
             [[song]]\nsources = [\"{key}\"]\n"
        ),
    );
    let torrents = s.dir.path().join("home/sources/torrent");
    std::fs::create_dir_all(&torrents).unwrap();
    std::fs::write(torrents.join(format!("{hash}.torrent")), &bytes).unwrap();
    let file = torrents.join(&hash).join("Paper Comets.flac");
    s.written(
        &format!("home/sources/torrent/{hash}/Paper Comets.flac"),
        "a copy with holes",
    );
    // Three seconds lost from its middle, as a disk that lost sectors.
    let mut holed = "#tb 0: 1/1000\n".to_string();
    for n in (0..100).filter(|n| !(20..70).contains(n)) {
        let pts = n * 60;
        let _ = writeln!(holed, "0, {pts}, {pts}, 60, 300, 0x0");
    }
    let damaged = Fake {
        timelines: vec![("Paper Comets".into(), holed)],
        hashes: vec![("Paper Comets".into(), 1)],
        ..Fake::default()
    }
    .probe(".flac", FLAC);
    let (_, text) = s.run(&damaged, &["sync", "--offline"]);
    assert!(text.contains("lacks"), "{text}");
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    assert_eq!(
        state.failures.get(&key).map(|f| f.step),
        Some(crate::state::Step::Refetch),
        "{text}"
    );

    let copied = Arc::new(Mutex::new(Vec::new()));
    let seen = copied.clone();
    let partial = s.dir.path().join("home/partial/torrent").join(&hash);
    let whole = Fake {
        hashes: vec![("Paper Comets".into(), 2)],
        on_stream: Some(Box::new(move |_: &[String]| {
            let at = partial.join("files").join("1.flac");
            seen.lock()
                .unwrap()
                .push(std::fs::read(&at).unwrap_or_default());
            std::fs::write(&at, b"a whole copy").unwrap();
            true
        })),
        ..Fake::default()
    }
    .probe(".flac", FLAC);
    let (ok, text) = s.run(&whole, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Fetching 1 damaged torrent file(s) again"),
        "{text}"
    );
    assert_eq!(
        *copied.lock().unwrap(),
        [b"a copy with holes".to_vec()],
        "aria2c starts from the damaged copy"
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"a whole copy");
    assert!(
        text.contains("taken, a whole copy in place of a damaged one"),
        "{text}"
    );
    let (ok, again) = s.run(&whole, &["sync"]);
    assert!(ok && !again.contains("Fetching"), "{again}");
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    assert!(!state.failures.contains_key(&key) && state.damaged.is_empty());
}

/// A home whose one song is `Paper Comets.flac` of the torrent of
/// `files`, each named with its bytes, in one piece hashed as its peers
/// hold it; the store keeps the song file alone, and a fake measures it
/// missing audio. Its key, its file, and the fake.
fn damaged_at_source(s: &Setup, files: &[(&str, &[u8])]) -> (SourceKey, PathBuf, Fake) {
    use sha1::Digest as _;
    let mut list = String::new();
    for (name, bytes) in files {
        let _ = write!(
            list,
            "d6:lengthi{}e4:pathl{}:{name}ee",
            bytes.len(),
            name.len()
        );
    }
    let mut bytes =
        format!("d4:infod5:filesl{list}e4:name15:Lantern Weather12:piece lengthi16384e6:pieces20:")
            .into_bytes();
    let all: Vec<u8> = files.iter().flat_map(|(_, b)| b.iter().copied()).collect();
    bytes.extend(sha1::Sha1::digest(all));
    bytes.extend(b"ee");
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let key = SourceKey::parse(&format!("torrent:{hash}/Paper Comets.flac")).unwrap();
    s.written(
        "home/songs.toml",
        &format!(
            "version = 4\n\
             [providers.lrclib]\nenabled = false\n\
             [providers.acoustid]\nenabled = false\n\
             [providers.musicbrainz]\nenabled = false\n\
             [providers.coverart]\nenabled = false\n\
             [[song]]\nsources = [\"{key}\"]\n"
        ),
    );
    let torrents = s.dir.path().join("home/sources/torrent");
    std::fs::create_dir_all(torrents.join(&hash)).unwrap();
    std::fs::write(torrents.join(format!("{hash}.torrent")), &bytes).unwrap();
    let file = torrents.join(&hash).join("Paper Comets.flac");
    std::fs::write(&file, files[0].1).unwrap();
    // Three seconds lost from its middle, as a rip of a broken file holds.
    let mut holed = "#tb 0: 1/1000\n".to_string();
    for n in (0..100).filter(|n| !(20..70).contains(n)) {
        let pts = n * 60;
        let _ = writeln!(holed, "0, {pts}, {pts}, 60, 300, 0x0");
    }
    let fake = Fake {
        timelines: vec![("Paper Comets".into(), holed)],
        hashes: vec![("Paper Comets".into(), 1)],
        ..Fake::default()
    }
    .probe(".flac", FLAC);
    (key, file, fake)
}

fn aria2c_runs(fake: &Fake) -> usize {
    fake.calls().iter().filter(|c| c[0] == "aria2c").count()
}

#[test]
fn a_damaged_torrent_file_its_torrent_holds_so_is_not_fetched_even_on_retry() {
    let s = Setup::new();
    let (key, _, fake) = damaged_at_source(&s, &[("Paper Comets.flac", b"a copy with holes")]);
    let (_, text) = s.run(&fake, &["sync"]);
    assert!(text.contains("lacks"), "{text}");
    assert_eq!(
        text.matches("holds its torrent's own bytes, every piece checking")
            .count(),
        1,
        "{text}"
    );
    assert!(!text.contains("Fetching"), "{text}");
    let step = || {
        let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
        state.failures.get(&key).map(|f| f.step)
    };
    assert_eq!(step(), Some(crate::state::Step::Mend), "{text}");

    for retry in [&["sync", "--retry"][..], &["sync", "--until-done"]] {
        let (_, text) = s.run(&fake, retry);
        assert!(!text.contains("Fetching"), "{text}");
        assert!(!text.contains("own bytes"), "said once: {text}");
    }
    assert_eq!(aria2c_runs(&fake), 0);
    assert_eq!(step(), Some(crate::state::Step::Mend));
}

#[test]
fn a_torrent_file_measured_whole_since_it_was_not_mended_is_no_problem() {
    let s = Setup::new();
    let (key, file, fake) = damaged_at_source(&s, &[("Paper Comets.flac", b"a copy with holes")]);
    let (_, text) = s.run(&fake, &["sync"]);
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    assert_eq!(
        state.failures.get(&key).map(|f| f.step),
        Some(crate::state::Step::Mend),
        "{text}"
    );
    let (ok, report) = s.report(&fake, &["check"]);
    assert!(!ok && report.contains("to mend"), "{report}");

    // Measured anew, as by a measure that judges it whole since.
    let home = s.dir.path().join("home");
    let mut state = crate::state::State::load(&home).unwrap();
    let audio = state.facts.get_mut(&key).unwrap().audio.as_mut().unwrap();
    audio.analysis.integrity = None;
    state.save(&home).unwrap();
    let whole = Fake {
        hashes: vec![("Paper Comets".into(), 1)],
        ..Fake::default()
    }
    .probe(".flac", FLAC);
    let (_, text) = s.run(&whole, &["sync"]);
    assert!(file.exists());
    let (_, report) = s.report(&whole, &["check"]);
    assert!(!report.contains("to mend"), "{text}\n{report}");
    assert!(!report.contains("Damaged audio"), "{report}");
}

#[test]
fn a_damaged_torrent_file_fetched_back_as_it_was_is_not_fetched_again() {
    let s = Setup::new();
    // The piece reaches into the notes, which no song brings, so the
    // store alone cannot check it.
    let (key, file, fake) = damaged_at_source(
        &s,
        &[
            ("Paper Comets.flac", b"a copy with holes"),
            ("notes.nfo", b"notes"),
        ],
    );
    let (_, text) = s.run(&fake, &["sync", "--offline"]);
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    assert_eq!(
        state.failures.get(&key).map(|f| f.step),
        Some(crate::state::Step::Refetch),
        "{text}"
    );
    let before = std::fs::metadata(&file).unwrap().modified().unwrap();

    let (_, text) = s.run(&fake, &["sync"]);
    assert!(
        text.contains("Fetching 1 damaged torrent file(s) again"),
        "{text}"
    );
    assert_eq!(
        text.matches("holds its torrent's own bytes, every piece checking")
            .count(),
        1,
        "{text}"
    );
    assert_eq!(aria2c_runs(&fake), 1);
    assert_eq!(std::fs::read(&file).unwrap(), b"a copy with holes");
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        before,
        "left as it was, so not measured again"
    );

    let (_, text) = s.run(&fake, &["sync", "--retry"]);
    assert!(!text.contains("Fetching"), "{text}");
    assert_eq!(aria2c_runs(&fake), 1);
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    assert_eq!(
        state.failures.get(&key).map(|f| f.step),
        Some(crate::state::Step::Mend)
    );
}

#[test]
fn status_changes_nothing() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (_, report) = s.report(&flacs(), &["status"]);
    assert!(report.contains("Not listed yet"), "{report}");
    assert!(!s.dir.path().join("home/songs.toml").exists());
    assert!(!s.dir.path().join("lib").exists());
}

#[test]
fn status_info_and_check_report_on_stdout() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    for (args, said) in [
        (&["status"][..], "Songs: 1 up to date"),
        (&["status", "--all"], "(up to date)"),
        (&["info"], "Up to date"),
        (&["check"], "No problems found"),
    ] {
        let (_, report) = s.report(&flacs(), args);
        assert!(report.contains(said), "{args:?}: {report}");
    }
}

#[test]
fn status_of_a_song_by_name_says_why_each_lookup_is_made_or_not() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let list = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&list).unwrap();
    std::fs::write(
        &list,
        listed.replace(
            "until = [\"lyrics:timed\"]\nskip = []\nrecheck = \"7 days\"",
            "until = [\"lyrics:timed\"]\nskip = [\"from:manual\"]\nrecheck = \"7 days\"",
        ),
    )
    .unwrap();
    let (_, report) = s.report(&flacs(), &["status", "a.flac"]);
    assert!(report.contains("  lookups\n"), "{report}");
    assert!(
        report.contains("lrclib        skipped: [providers] skip 'from:manual'"),
        "{report}"
    );
    assert!(
        report.contains("youtube-music cannot ask: no YouTube upload to search from"),
        "{report}"
    );
}

impl Setup {
    /// What a command writes to stdout.
    fn report(&self, fake: &Fake, args: &[&str]) -> (bool, String) {
        let mut data = Vec::new();
        let ok = run_with(
            &self.job(args),
            fake,
            &Server::default(),
            None,
            &mut Vec::new(),
            &mut data,
        )
        .unwrap();
        (ok, String::from_utf8(data).unwrap())
    }
}

impl Setup {
    fn listed(&self, args: &[&str]) -> String {
        let mut data = Vec::new();
        run_with(
            &self.job(args),
            &flacs(),
            &Server::default(),
            None,
            &mut Vec::new(),
            &mut data,
        )
        .unwrap();
        String::from_utf8(data).unwrap()
    }

    fn refused(&self, args: &[&str]) -> String {
        let e = run_with(
            &self.job(args),
            &flacs(),
            &Server::default(),
            None,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
        format!("{e:#}")
    }
}

#[test]
fn a_misspelled_song_table_takes_no_song_out_of_the_library() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib/Artist/Record");
    assert_eq!(std::fs::read_dir(&lib).unwrap().count(), 2, "{text}");
    let list = s.dir.path().join("home/songs.toml");
    let text = std::fs::read_to_string(&list).unwrap();
    std::fs::write(&list, text.replace("[[song]]", "[[songs]]")).unwrap();
    let e = run_with(
        &s.job(&["sync"]),
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    let text = format!("{e:#}");
    assert!(text.contains("did you mean `song`?"), "{text}");
    assert_eq!(
        std::fs::read_dir(&lib).map_or(0, Iterator::count),
        2,
        "{text}"
    );
}

#[test]
fn a_removed_song_goes_and_stays_gone_until_restored() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib/Artist/Record");
    assert_eq!(std::fs::read_dir(&lib).unwrap().count(), 2, "{text}");

    s.refused(&["remove", "manual:a.flac"]);
    assert!(
        s.refused(&["remove", "-y", "song"])
            .contains("2 songs match")
    );
    let (ok, text) = s.run(&flacs(), &["remove", "-y", "manual:a.flac"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs(), [vec![SourceKey::Manual("b.flac".into())]]);
    assert!(
        s.dir.path().join("home/sources/manual/a.flac").exists(),
        "a removed song keeps its source"
    );
    assert_eq!(std::fs::read_dir(&lib).unwrap().count(), 1, "{text}");

    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(
        s.songs().len(),
        1,
        "a removed file is not listed again: {text}"
    );
    assert_eq!(
        s.listed(&["list", "--removed", "--keys"]),
        "manual:a.flac\n"
    );

    let (ok, text) = s.run(&flacs(), &["restore", "-y", "manual:a.flac"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 2);
    assert_eq!(std::fs::read_dir(&lib).unwrap().count(), 2, "{text}");

    let (ok, text) = s.run(&flacs(), &["remove", "-y", "--all", "album:record"]);
    assert!(ok, "{text}");
    assert!(s.songs().is_empty(), "{text}");
    assert_eq!(
        s.listed(&["list", "--removed", "--keys", "album:record"]),
        "manual:a.flac\nmanual:b.flac\n",
        "a, restored where it stood, is first again"
    );
    let (ok, text) = s.run(&flacs(), &["restore", "-y", "--all", "album:record"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 2, "{text}");
}

#[test]
fn a_change_on_a_terminal_names_what_it_does_and_asks_by_name() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");

    let mut p = crate::ui::MockPrompter::answering([false]);
    p.push_choice([0, 1]);
    let text = s
        .trusting(&flacs(), &["remove", "song"], Some(&mut p))
        .unwrap();
    assert!(p.asked[0].starts_with("2 songs match"), "{:?}", p.asked);
    assert_eq!(
        p.ticked,
        [Vec::<usize>::new()],
        "no song is picked to begin with"
    );
    assert_eq!(p.asked[1], "Remove 2 songs?");
    let text = crate::ui::plain(&text);
    assert!(text.contains("  − "), "{text}");
    assert!(text.contains("`muman undo` puts them back"), "{text}");
    assert_eq!(s.songs().len(), 2, "answering no changes nothing: {text}");

    let mut p = crate::ui::MockPrompter::new();
    p.push_text(Some("2"));
    let text = s
        .trusting(
            &flacs(),
            &["remove", "--purge", "manual:a.flac"],
            Some(&mut p),
        )
        .unwrap();
    assert_eq!(
        p.asked,
        ["Type 1 to remove 1 song and trash 1 file of yours:"],
        "{text}"
    );
    assert_eq!(
        s.songs().len(),
        2,
        "a count mistyped changes nothing: {text}"
    );
    assert!(s.dir.path().join("home/sources/manual/a.flac").exists());
}

#[test]
fn edit_shows_where_each_tag_comes_from_and_hides_what_was_held() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let dirs = crate::dirs::Dirs {
        home: s.dir.path().join("home"),
        library: s.dir.path().join("lib"),
        locks: Some(s.dir.path().to_path_buf()),
    };
    let seen = std::cell::RefCell::new(String::new());
    let mut editor = |path: &std::path::Path| -> anyhow::Result<()> {
        let text = std::fs::read_to_string(path)?;
        *seen.borrow_mut() = text.clone();
        std::fs::write(
            path,
            text.replace("tags.genre = \"\"", "tags.genre = \"Folk\""),
        )?;
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(1);
        filetime::set_file_mtime(path, filetime::FileTime::from_system_time(later))?;
        Ok(())
    };
    let mut p = crate::ui::MockPrompter::new();
    let changed = crate::editor::edit(
        &dirs,
        &[],
        false,
        Some(&mut p),
        &mut editor,
        &mut crate::history::Later::new(&dirs.home).unwrap(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(changed);
    let shown = seen.into_inner();
    assert!(shown.contains("# TITLE"), "{shown}");
    assert!(shown.contains("← manual:a.flac"), "{shown}");
    assert!(
        !shown.contains("held."),
        "what was held is muman's: {shown}"
    );
    let list = std::fs::read_to_string(dirs.manifest()).unwrap();
    assert!(!list.contains("← "), "{list}");
    assert!(list.contains("tags.genre = \"Folk\""), "{list}");
    assert!(list.contains("held.\"manual:a.flac\""), "{list}");
}

#[test]
fn set_with_a_query_alone_changes_a_song_step_by_step() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(s.refused(&["set", "song"]).contains("step by step"));

    // The menu: the six tags a song list starts with, the track number
    // the source offers, the three pins, the lyrics offset, another tag,
    // then the review.
    let (title, date, audio, review) = (0, 5, 7, 12);
    let mut p = crate::ui::MockPrompter::answering([true]);
    p.push_select(Some(title)).push_select(Some(1));
    p.push_text(Some("Tide"));
    p.push_select(Some(date)).push_select(Some(1));
    p.push_text(Some("spring 2019")).push_text(Some("2019"));
    p.push_select(Some(audio)).push_select(Some(1));
    p.push_select(Some(review));
    let text = s
        .trusting(&flacs(), &["set", "song"], Some(&mut p))
        .unwrap();
    assert!(p.asked[0].starts_with("What to change?"), "{:?}", p.asked);
    assert!(p.asked[0].contains("\n  title "), "{:?}", p.asked);
    assert!(p.asked[0].ends_with("\n  Review and save"), "{:?}", p.asked);
    assert!(
        p.asked.contains(&"title: [Song]".to_string()),
        "{:?}",
        p.asked
    );
    assert!(
        p.asked
            .iter()
            .any(|a| a.starts_with("refused: date = \"spring 2019\"")),
        "{:?}",
        p.asked
    );
    assert_eq!(
        p.asked.last().map(String::as_str),
        Some("Save 3 changes to Song — Artist?"),
        "{text}"
    );
    let list = std::fs::read_to_string(s.dir.path().join("home/songs.toml")).unwrap();
    assert!(list.contains("tags.title = \"Tide\""), "{list}");
    assert!(list.contains("tags.date = \"2019\""), "{list}");
    assert!(list.contains("audio = \"manual:a.flac\""), "{list}");

    let mut p = crate::ui::MockPrompter::answering([true]);
    p.push_select(Some(title)).push_select(Some(1));
    p.push_text(Some("Ember Road"));
    p.push_select(None);
    s.trusting(&flacs(), &["set", "manual:a.flac"], Some(&mut p))
        .unwrap();
    assert_eq!(
        p.asked.last().map(String::as_str),
        Some("Leave without saving 1 change?")
    );
    let after = std::fs::read_to_string(s.dir.path().join("home/songs.toml")).unwrap();
    assert!(
        after.contains("tags.title = \"Tide\""),
        "leaving saves nothing: {after}"
    );
}

#[test]
fn set_step_by_step_tags_an_album_types_an_offset_and_names_another_tag() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let list = s.dir.path().join("home/songs.toml");

    // A tag named is listed once chosen, so the review moves down one.
    let (offset, another, review) = (10, 11, 13);
    let mut p = crate::ui::MockPrompter::answering([true]);
    p.push_select(Some(offset));
    p.push_text(Some("soon")).push_text(Some("120 ms"));
    p.push_select(Some(another));
    p.push_text(Some("composer"));
    p.push_select(Some(1));
    p.push_text(Some("Ada Quill"));
    p.push_select(Some(review));
    let text = s
        .trusting(&flacs(), &["set", "manual:a.flac"], Some(&mut p))
        .unwrap();
    assert!(
        p.asked.iter().any(|a| a.starts_with("refused: ")),
        "{:?}",
        p.asked
    );
    let after = std::fs::read_to_string(&list).unwrap();
    assert!(
        after.contains("lyrics_offset = \"120 ms\""),
        "{after}\n{text}"
    );
    assert!(
        after.contains("composer = \"Ada Quill\""),
        "{after}\n{text}"
    );

    let album = "youtube.com:playlist/PLvid0000001";
    let text = std::fs::read_to_string(&list)
        .unwrap()
        .replace("[[song]]\n", &format!("[[song]]\nalbum = \"{album}\"\n"));
    std::fs::write(
        &list,
        format!(
            "{text}\n[[album]]\nsource = \"{album}\"\ntags = {{ album = \"Lantern Weather\" }}\n"
        ),
    )
    .unwrap();

    // The rows of the menu for two songs on an album, read from a look
    // that leaves at once.
    let mut look = crate::ui::MockPrompter::new();
    look.push_select(None);
    s.trusting(&flacs(), &["set", "--all", "song"], Some(&mut look))
        .unwrap();
    let rows: Vec<&str> = look.asked[0].lines().skip(1).collect();
    let row = |start: &str| rows.iter().position(|r| r.trim_start().starts_with(start));
    let (genre, review) = (row("genre ").unwrap(), row("Review and save").unwrap());
    let mut p = crate::ui::MockPrompter::answering([true]);
    p.push_select(Some(genre)).push_select(Some(1));
    p.push_text(Some("Folk"));
    p.push_select(Some(1));
    p.push_select(Some(review));
    let text = s
        .trusting(&flacs(), &["set", "--all", "song"], Some(&mut p))
        .unwrap();
    assert!(
        p.asked
            .iter()
            .any(|a| a.starts_with("Set genre to Folk where?")),
        "{:?}",
        p.asked
    );
    let after = std::fs::read_to_string(&list).unwrap();
    let album_table = &after[after.find("[[album]]").unwrap()..];
    assert!(album_table.contains("genre = \"Folk\""), "{after}\n{text}");
}

#[test]
fn sync_on_a_new_home_writes_the_song_list_to_edit() {
    let s = Setup::new();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let list = std::fs::read_to_string(s.dir.path().join("home/songs.toml")).unwrap();
    assert!(
        list.contains("[library]") && list.contains("[audio]"),
        "{list}"
    );
}

#[test]
fn audio_that_does_not_read_is_not_copied_in_or_listed() {
    let s = Setup::new();
    let good = s.file("rips/One/a.flac");
    s.file("rips/One/broken.flac");
    let lone = s.file("rips/lone.flac");
    let fake = Fake::default()
        .probe("broken.flac", "not json")
        .probe("lone.flac", "not json")
        .probe(".flac", FLAC);
    let e = run_with(
        &s.job(&[
            "add",
            &s.dir.path().join("rips/One").to_string_lossy(),
            &lone,
        ]),
        &fake,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    let said = format!("{e:#}");
    assert!(said.starts_with("2 file(s) could not be added"), "{said}");
    assert!(
        said.contains("broken.flac, which reads as no audio"),
        "{said}"
    );
    assert_eq!(s.songs(), [vec![SourceKey::Manual("One/a.flac".into())]]);
    let manual = s.dir.path().join("home/sources/manual");
    assert!(!manual.join("One/broken.flac").exists() && !manual.join("lone.flac").exists());
    assert!(Path::new(&good).exists());
}

/// The files `sources/manual` holds, by their names on disk.
fn manual_files(s: &Setup) -> Vec<String> {
    let manual = s.dir.path().join("home/sources/manual");
    let mut names: Vec<String> = crate::store::files_below(&manual, usize::MAX, |_| true)
        .unwrap_or_default()
        .iter()
        .map(|f| {
            f.strip_prefix(&manual)
                .unwrap_or(f)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn an_add_stopped_at_any_step_leaves_no_copy_undo_cannot_take_out() {
    let whole = Setup::new();
    let given = whole.file("rips/a.flac");
    let counted = crate::kill::kill_at(whole.dir.path(), usize::MAX);
    whole.run(&flacs(), &["add", &given]);
    let steps = counted.seen();
    drop(counted);
    assert!(steps > 1, "{steps}");
    for at in 1..=steps {
        let s = Setup::new();
        let given = s.file("rips/a.flac");
        let kill = crate::kill::kill_at(s.dir.path(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_with(
                &s.job(&["add", &given]),
                &flacs(),
                &Server::default(),
                None,
                &mut Vec::new(),
                &mut Vec::new(),
            )
        }));
        drop(kill);
        assert!(stopped.is_err(), "not stopped at step {at} of {steps}");
        let (ok, text) = s.run(&flacs(), &["sync"]);
        assert!(ok, "step {at}: {text}");
        let undone = s.trusting(&flacs(), &["undo", "-y"], None);
        assert_eq!(
            manual_files(&s),
            Vec::<String>::new(),
            "step {at}: {undone:?}"
        );
        let m = Manifest::load(&s.dir.path().join("home")).unwrap();
        assert!(m.songs.is_empty(), "step {at}: {undone:?}");
    }
}

#[test]
fn an_add_stopped_at_any_step_is_undone_at_once() {
    let whole = Setup::new();
    let given = whole.file("rips/a.flac");
    let counted = crate::kill::kill_at(whole.dir.path(), usize::MAX);
    whole.run(&flacs(), &["add", &given]);
    let steps = counted.seen();
    drop(counted);
    for at in 1..=steps {
        let s = Setup::new();
        let given = s.file("rips/a.flac");
        let kill = crate::kill::kill_at(s.dir.path(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_with(
                &s.job(&["add", &given]),
                &flacs(),
                &Server::default(),
                None,
                &mut Vec::new(),
                &mut Vec::new(),
            )
        }));
        drop(kill);
        assert!(stopped.is_err(), "not stopped at step {at} of {steps}");
        let undone = s.trusting(&flacs(), &["undo", "-y"], None);
        if let Err(e) = &undone {
            assert!(
                format!("{e:#}").contains("no run to undo"),
                "step {at}: {e:#}"
            );
        }
        let m = Manifest::load(&s.dir.path().join("home")).map(|m| m.songs.len());
        assert!(m.is_err() || m.unwrap() == 0, "step {at}: {undone:?}");
        assert!(library_of(&s).is_empty(), "step {at}: {undone:?}");
    }
}

#[test]
fn a_set_stopped_at_any_step_is_undone_at_once() {
    let prepare = |s: &Setup| {
        s.file("home/sources/manual/a.flac");
        let (ok, text) = s.run(&flacs(), &["sync"]);
        assert!(ok, "{text}");
    };
    let args = ["set", "-y", "manual:a.flac", "title=Rooms of Salt"];
    let whole = Setup::new();
    prepare(&whole);
    let counted = crate::kill::kill_at(whole.dir.path(), usize::MAX);
    whole.run(&flacs(), &args);
    let steps = counted.seen();
    drop(counted);
    for at in 1..=steps {
        let s = Setup::new();
        prepare(&s);
        let songs = s.dir.path().join("home/songs.toml");
        let (listed, library) = (std::fs::read_to_string(&songs).unwrap(), library_of(&s));
        let kill = crate::kill::kill_at(s.dir.path(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_with(
                &s.job(&args),
                &flacs(),
                &Server::default(),
                None,
                &mut Vec::new(),
                &mut Vec::new(),
            )
        }));
        drop(kill);
        assert!(stopped.is_err(), "not stopped at step {at} of {steps}");
        // Stopped before it changed anything, the run before is the last.
        if std::fs::read_to_string(&songs).unwrap() == listed && library_of(&s) == library {
            continue;
        }
        let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
        assert!(ok, "step {at}: {text}");
        assert_eq!(
            std::fs::read_to_string(&songs).unwrap(),
            listed,
            "step {at}"
        );
        assert_eq!(library_of(&s), library, "step {at}: {text}");
    }
}

#[test]
fn undo_is_refused_where_a_file_of_your_own_holds_where_a_song_moves_back() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
    edit_list(&s, &default, "template = \"{{ title }}\"");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok && text.contains("Moved"), "{text}");
    let old = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, "a file of my own").unwrap();
    let library = library_of(&s);
    let e = s.refused(&["undo", "-y"]);
    assert!(e.contains("is taken"), "{e}");
    assert_eq!(library_of(&s), library);
}

#[test]
fn a_change_with_no_run_kept_before_it_is_undone() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    std::fs::remove_dir_all(s.dir.path().join("home/history")).unwrap();
    let songs = s.dir.path().join("home/songs.toml");
    let (listed, library) = (std::fs::read_to_string(&songs).unwrap(), library_of(&s));
    for args in [
        &["set", "-y", "manual:a.flac", "genre=Folk"][..],
        &["remove", "-y", "manual:a.flac"],
    ] {
        let (ok, text) = s.run(&flacs(), args);
        assert!(ok, "{args:?}: {text}");
        let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
        assert!(ok, "{args:?}: {text}");
        assert_eq!(std::fs::read_to_string(&songs).unwrap(), listed, "{args:?}");
        assert_eq!(library_of(&s), library, "{args:?}: {text}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn add_refuses_two_files_named_alike_once_normalized_and_copies_neither() {
    let s = Setup::new();
    let nfc = s.file("rips/caf\u{e9}.flac");
    let nfd = s.written("rips/other/cafe\u{301}.flac", "another recording");
    let e = s
        .trusting(&flacs(), &["add", &nfc, &nfd], None)
        .unwrap_err();
    assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
    assert!(format!("{e:#}").contains("another file given"), "{e:#}");
    assert!(manual_files(&s).is_empty(), "{:?}", manual_files(&s));
}

#[cfg(target_os = "linux")]
#[test]
fn add_refuses_another_file_under_another_spelling_of_a_name_held() {
    let s = Setup::new();
    let first = s.file("one/caf\u{e9}.flac");
    let (ok, text) = s.run(&flacs(), &["add", &first]);
    assert!(ok, "{text}");
    let before = library_of(&s);
    let second = s.written("two/cafe\u{301}.flac", "another recording");
    let e = s.trusting(&flacs(), &["add", &second], None).unwrap_err();
    assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
    assert!(format!("{e:#}").contains("exists already"), "{e:#}");
    assert_eq!(library_of(&s), before);
    assert_eq!(manual_files(&s), ["caf\u{e9}.flac"]);
}

#[cfg(target_os = "linux")]
#[test]
fn add_refuses_a_name_that_is_no_text() {
    use std::os::unix::ffi::OsStrExt;
    let s = Setup::new();
    let folder = s.dir.path().join("rips/Album");
    std::fs::create_dir_all(&folder).unwrap();
    let odd = folder.join(std::ffi::OsStr::from_bytes(b"caf\xe9.flac"));
    std::fs::write(&odd, "one").unwrap();
    s.file("rips/Album/b.flac");
    let e = s
        .trusting(&flacs(), &["add", &folder.to_string_lossy()], None)
        .unwrap_err();
    assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
    assert!(format!("{e:#}").contains("UTF-8"), "{e:#}");
    assert!(manual_files(&s).is_empty(), "{:?}", manual_files(&s));
}

#[test]
fn a_file_given_a_decomposed_name_is_copied_composed_and_undone() {
    let s = Setup::new();
    let given = s.file("rips/cafe\u{301}.flac");
    let (ok, text) = s.run(&flacs(), &["add", &given]);
    assert!(ok, "{text}");
    assert_eq!(manual_files(&s), ["caf\u{e9}.flac"]);
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert!(manual_files(&s).is_empty(), "{:?}", manual_files(&s));
}

#[cfg(target_os = "linux")]
#[test]
fn lyrics_named_decomposed_come_with_a_song_named_composed() {
    let s = Setup::new();
    let song = s.file("rips/caf\u{e9}.flac");
    s.written("rips/cafe\u{301}.lrc", "[00:01.00]Paper comets\n");
    let (ok, text) = s.run(&flacs(), &["add", &song]);
    assert!(ok, "{text}");
    assert_eq!(manual_files(&s), ["caf\u{e9}.flac", "caf\u{e9}.lrc"]);
}

#[test]
fn lyrics_lines_left_out_as_damaged_are_said_when_the_song_is_written() {
    let s = Setup::new();
    let song = s.file("rips/a.flac");
    s.written(
        "rips/a.lrc",
        "[00:01.00]Paper comets\n[00:-3.00]Lantern\n[00:02.00]Kessler Row\n",
    );
    let (ok, text) = s.run(&flacs(), &["add", &song]);
    assert!(ok, "{text}");
    assert!(
        text.contains("lyrics of manual:a.flac: 1 line(s) left out"),
        "{text}"
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(!text.contains("left out"), "said once, as written: {text}");
}

#[test]
fn a_folder_named_by_where_it_ends_is_copied_under_its_own_name() {
    let s = Setup::new();
    s.file("rips/One/Disc/a.flac");
    let fake = Fake::default().probe(".flac", FLAC);
    let up = s
        .dir
        .path()
        .join("rips")
        .join("One")
        .join("Disc")
        .join("..");
    let up = up.to_string_lossy().into_owned();
    let (ok, text) = s.run(&fake, &["add", &up]);
    assert!(ok, "{text}");
    assert_eq!(
        s.songs(),
        [vec![SourceKey::Manual("One/Disc/a.flac".into())]]
    );
}

#[test]
fn a_hidden_file_is_refused_and_one_in_a_folder_left_behind() {
    let s = Setup::new();
    let hidden = s.file("in/.Song.flac");
    let fake = Fake::default().probe(".flac", FLAC);
    let add = |args: &[&str]| {
        run_with(
            &s.job(args),
            &fake,
            &Server::default(),
            None,
            &mut Vec::new(),
            &mut Vec::new(),
        )
    };
    let e = add(&["add", &hidden]).unwrap_err();
    assert!(
        format!("{e:#}").contains("rename it without the dot"),
        "{e:#}"
    );
    let manual = s.dir.path().join("home/sources/manual");
    assert!(!manual.join(".Song.flac").exists());

    s.file("rips/One/a.flac");
    s.file("rips/One/._a.flac");
    std::fs::create_dir_all(s.dir.path().join("rips/Empty")).unwrap();
    let folder = |n: &str| {
        s.dir
            .path()
            .join("rips")
            .join(n)
            .to_string_lossy()
            .into_owned()
    };
    add(&["add", &folder("One")]).unwrap();
    assert!(!manual.join("One/._a.flac").exists());
    assert_eq!(s.songs(), [vec![SourceKey::Manual("One/a.flac".into())]]);
    let e = add(&["add", &folder("Empty")]).unwrap_err();
    assert!(
        format!("{e:#}").contains("holds no song muman reads"),
        "{e:#}"
    );
}

#[test]
fn a_song_taken_out_of_the_list_by_hand_stays_out_until_restored() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 2, "{text}");
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    let (head, rest) = listed.split_once("\n[[song]]\n").unwrap();
    let kept: Vec<&str> = rest
        .split("\n[[song]]\n")
        .filter(|b| !b.contains("manual:a.flac"))
        .collect();
    std::fs::write(
        &songs,
        format!("{head}\n[[song]]\n{}", kept.join("\n[[song]]\n")),
    )
    .unwrap();

    let text = s.listed(&["status"]);
    assert!(
        text.contains("kept out by the next sync: manual:a.flac"),
        "{text}"
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Taken out of the song list by hand, so kept out"),
        "{text}"
    );
    assert_eq!(
        s.songs(),
        [vec![SourceKey::Manual("b.flac".into())]],
        "{text}"
    );
    let lib = s.dir.path().join("lib");
    let files = |dir: &Path| {
        crate::store::files_below(dir, usize::MAX, |_| true)
            .unwrap()
            .len()
    };
    assert_eq!(files(&lib), 1, "the other song alone: {text}");
    let (_, again) = s.run(&flacs(), &["sync"]);
    assert_eq!(s.songs().len(), 1, "{again}");

    let (ok, text) = s.run(&flacs(), &["restore", "-y", "manual:a.flac"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 2, "{text}");
}

#[test]
fn sync_fills_in_every_setting_and_updates_none_already_current() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let songs = s.dir.path().join("home/songs.toml");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    std::fs::write(&songs, "version = 1\n\n[audio]\nopus_kbps = 192\n").unwrap();
    let (ok, text) = s.run(&flacs(), &["sync", "--update-defaults"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Renamed in the song list: [audio] `opus_kbps = 192`"),
        "{text}"
    );
    assert!(
        text.contains("No setting is at an older edition's default"),
        "{text}"
    );
    let listed = std::fs::read_to_string(&songs).unwrap();
    assert!(
        listed.contains(&format!("\nedition = {}\n", crate::migrate::EDITION)),
        "{listed}"
    );
    assert!(listed.contains("opus_bitrate = \"192 kb/s\""), "{listed}");
    assert!(!listed.contains("opus_kbps"), "{listed}");
    assert!(listed.contains("[ytdlp]"), "{listed}");
}

#[test]
fn info_counts_no_notice_of_the_song_list_as_a_song() {
    let s = Setup::new();
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\nedition = 3\n\n[ytdlp]\nsub_langs = \"all,-live_chat\"\n\n[audoi]\n",
    )
    .unwrap();
    let (ok, text) = s.report(&flacs(), &["info", "-v"]);
    assert!(ok, "{text}");
    let unmade = text.lines().find(|l| l.contains("Cannot be made"));
    assert_eq!(
        unmade.map(|l| l.split_whitespace().last()),
        Some(Some("0")),
        "{text}"
    );
}

#[test]
fn info_changes_names_each_value_a_sync_would_change_and_why() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let changes = |fields: &[&str]| {
        let args: Vec<&str> = ["info", "--changes"]
            .iter()
            .chain(fields)
            .copied()
            .collect();
        let (ok, report) = s.report(&flacs(), &args);
        assert!(ok, "{report}");
        crate::ui::plain(&report)
    };
    assert_eq!(changes(&[]), "No song would change\n");

    let list = s.dir.path().join("home/songs.toml");
    let text = std::fs::read_to_string(&list).unwrap();
    assert!(text.contains("tags.title = \"\""), "{text}");
    std::fs::write(
        &list,
        text.replace("tags.title = \"\"", "tags.title = \"Lantern\""),
    )
    .unwrap();
    assert_eq!(
        changes(&[]),
        "manual:Song.flac — Lantern — Artist\n  \
         path  Artist/Record/02 Song.flac → Artist/Record/02 Lantern.flac\n  \
         TITLE Song → Lantern (song.tags)\n\
         1 song(s) would change: TITLE 1, path 1\n"
    );
    assert_eq!(
        changes(&["title"]),
        "manual:Song.flac — Lantern — Artist\n  TITLE Song → Lantern (song.tags)\n\
         1 song(s) would change: TITLE 1\n",
        "a tag named as the song list names it"
    );
    assert!(
        s.refused(&["info", "--changes", "colour"])
            .contains("No song is written with `colour`")
    );
}

#[test]
fn status_of_a_song_a_sync_would_fetch_says_so_and_fails_nothing() {
    let s = Setup::new();
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n\n[[song]]\nsources = [\"jamendo.com:1234567\"]\n",
    )
    .unwrap();
    let (ok, text) = s.run(&flacs(), &["status"]);
    assert!(ok, "{text}");
    assert!(!text.contains("Failed"), "{text}");
    assert!(text.contains("Made once a sync fetches it"), "{text}");
    let (_, report) = s.report(&flacs(), &["status"]);
    assert!(report.contains("Songs: 1 made once fetched"), "{report}");

    let home = s.dir.path().join("home");
    let mut state = crate::state::State::load(&home).unwrap();
    let key = SourceKey::parse("jamendo.com:1234567").unwrap();
    state.record_failure(&key, crate::state::Step::Fetch, None, "Not found".into());
    state.save(&home).unwrap();
    let (ok, text) = s.run(&flacs(), &["status"]);
    assert!(!ok, "a source that failed lately is not fetched: {text}");
    assert!(text.contains("Failed"), "{text}");
}

#[test]
fn a_sync_fetches_what_is_missing_or_damaged_and_waits_on_what_failed_lately() {
    let s = Setup::new();
    let hash = "0123456789abcdef0123456789abcdef01234567";
    s.file("home/sources/manual/gone.flac");
    std::fs::remove_file(s.dir.path().join("home/sources/manual/gone.flac")).unwrap();
    s.written(
        "home/songs.toml",
        &format!(
            "version = 4\n\
             [[song]]\nsources = [\"jamendo.com:1234567\"]\n\
             [[song]]\nsources = [\"jamendo.com:7654321\"]\n\
             [[song]]\nsources = [\"torrent:{hash}/a.flac\"]\n\
             [[song]]\nsources = [\"manual:gone.flac\"]\n\
             [[song]]\nsources = [\"tunes.example:item0001\"]\n"
        ),
    );
    let home = s.dir.path().join("home");
    let manifest = Manifest::load(&home).unwrap();
    let store = crate::store::Store::scan(&s.job(&["sync"]).dirs).unwrap();
    let key = |k: &str| SourceKey::parse(k).unwrap();
    let mut state = crate::state::State::default();
    state.record_failure(
        &key("jamendo.com:7654321"),
        crate::state::Step::Fetch,
        None,
        "gone".into(),
    );
    state.record_failure(
        &key("jamendo.com:7654321"),
        crate::state::Step::Fetch,
        None,
        "gone".into(),
    );
    let torrent = key(&format!("torrent:{hash}/a.flac"));
    state.record_failure(&torrent, crate::state::Step::Refetch, None, "cut".into());
    let now = crate::state::now_secs();
    let due = crate::due::due(&manifest, &store, &state.failures, false, now);
    let missing = crate::due::Fetch::Missing;
    assert_eq!(
        due.now,
        BTreeMap::from([
            (key("jamendo.com:1234567"), missing),
            (torrent.clone(), crate::due::Fetch::Damaged),
        ]),
        "a manual file and a key of no address to fetch from are no fetch"
    );
    assert_eq!(
        due.waiting,
        std::collections::BTreeSet::from([key("jamendo.com:7654321")])
    );
    let retried = crate::due::due(&manifest, &store, &state.failures, true, now);
    assert!(retried.now.contains_key(&key("jamendo.com:7654321")));
    assert!(retried.waiting.is_empty());
}

#[test]
fn status_names_each_old_setting_and_changes_nothing() {
    let s = Setup::new();
    let songs = s.dir.path().join("home/songs.toml");
    std::fs::create_dir_all(s.dir.path().join("home")).unwrap();
    let old = "version = 1\n\n[history]\nmax_mib = 1024\n";
    std::fs::write(&songs, old).unwrap();
    let (ok, text) = s.run(&flacs(), &["status"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("[history] `max_mib = 1024` is `max_size = \"1 GiB\"` now"),
        "{text}"
    );
    assert_eq!(std::fs::read_to_string(&songs).unwrap(), old);
}

#[test]
fn undo_moves_songs_back_after_a_template_edited_by_hand() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
    assert!(listed.contains(&default), "{listed}");
    std::fs::write(
        &songs,
        listed.replace(&default, "template = \"{{ album }}/{{ title }}\""),
    )
    .unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(text.contains("Moved:"), "{text}");
    assert!(s.dir.path().join("lib/Record/Song.flac").exists());

    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Moved back: Artist/Record/02 Song.flac"),
        "{text}"
    );
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
    assert!(s.dir.path().join("lib/Artist/Record/02 Song.flac").exists());
    assert!(!s.dir.path().join("lib/Record").exists());
    assert_eq!(std::fs::read_to_string(&songs).unwrap(), listed);
}

#[test]
fn a_state_damaged_or_emptied_is_named_and_not_read_as_new() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let home = s.dir.path().join("home");
    for beside in ["state.db-wal", "state.db-shm"] {
        let _ = std::fs::remove_file(home.join(beside));
    }
    let db = home.join("state.db");
    let mut bytes = std::fs::read(&db).unwrap();
    bytes[..100].fill(b'x');
    std::fs::write(&db, &bytes).unwrap();
    let e = s.trusting(&flacs(), &["list"], None).unwrap_err();
    assert!(format!("{e:#}").contains(&*db.to_string_lossy()), "{e:#}");
    std::fs::write(&db, b"").unwrap();
    let e = s.trusting(&flacs(), &["check"], None).unwrap_err();
    assert!(format!("{e:#}").contains("is empty"), "{e:#}");
}

#[test]
fn what_undo_puts_back_is_counted_and_promised_only_while_runs_are_kept() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let (_, text) = s.run(&flacs(), &["remove", "-y", "manual:a.flac"]);
    assert!(text.contains("`muman undo` puts it back"), "{text}");
    let (_, text) = s.run(&flacs(), &["restore", "-y", "manual:a.flac"]);
    assert!(text.contains("`muman undo` takes it out again"), "{text}");
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    let at = listed.find("[history]").unwrap();
    let runs = at + listed[at..].find("runs = ").unwrap();
    let end = runs + listed[runs..].find('\n').unwrap();
    std::fs::write(
        &songs,
        format!("{}runs = 0{}", &listed[..runs], &listed[end..]),
    )
    .unwrap();
    s.run(&flacs(), &["sync"]);
    for args in [
        &["set", "-y", "manual:a.flac", "genre=Folk"][..],
        &["remove", "-y", "manual:a.flac"],
        &["restore", "-y", "manual:a.flac"],
    ] {
        let (_, text) = s.run(&flacs(), args);
        assert!(!text.contains("`muman undo` puts"), "{args:?}: {text}");
        assert!(!text.contains("`muman undo` takes"), "{args:?}: {text}");
        assert!(text.contains("runs = 0"), "{args:?}: {text}");
    }
}

#[test]
fn lyrics_muman_wrote_and_the_user_changed_are_said_to_be_so() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.written("home/sources/manual/a.lrc", "[00:01.00]Paper comets\n");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lrc = s.dir.path().join("lib/Artist/Record/02 Song.lrc");
    assert!(lrc.exists(), "{text}");
    std::fs::write(&lrc, "[00:01.00]Paper comets, by hand\n").unwrap();
    let (_, text) = s.run(
        &flacs(),
        &["set", "-y", "manual:a.flac", "lyrics_offset=1s"],
    );
    assert!(text.contains("changed since muman wrote it"), "{text}");
    assert!(!text.contains("not muman's"), "{text}");
}

#[test]
fn a_song_renamed_beside_lyrics_the_user_changed_stays_where_it_is_and_undoes() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.written("home/sources/manual/a.lrc", "[00:01.00]Paper comets\n");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let song = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    let lrc = song.with_extension("lrc");
    std::fs::write(&lrc, "[00:01.00]Paper comets, by hand\n").unwrap();
    let built = std::fs::read(&song).unwrap();

    let (ok, text) = s.run(&flacs(), &["set", "-y", "manual:a.flac", "title=Lantern"]);
    assert!(ok, "{text}");
    assert!(
        text.contains(
            "Left at Artist/Record/02 Song.flac: Artist/Record/02 Song.lrc changed since muman \
             wrote it"
        ),
        "{text}"
    );
    assert_eq!(std::fs::read(&song).unwrap(), built, "{text}");
    assert_eq!(
        std::fs::read_to_string(&lrc).unwrap(),
        "[00:01.00]Paper comets, by hand\n"
    );
    assert!(
        !s.dir
            .path()
            .join("lib/Artist/Record/02 Lantern.flac")
            .exists()
    );
    let (_, report) = s.report(&flacs(), &["check"]);
    assert!(!report.contains(".flac (`sync --force"), "{report}");

    let (ok, text) = s.run(&flacs(), &["sync", "--force"]);
    assert!(ok, "{text}");
    let renamed = s.dir.path().join("lib/Artist/Record/02 Lantern.flac");
    assert!(renamed.exists(), "{text}");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(std::fs::read(&song).unwrap(), built, "{text}");
    assert_eq!(
        std::fs::read_to_string(&lrc).unwrap(),
        "[00:01.00]Paper comets, by hand\n",
        "{text}"
    );
}

#[test]
fn a_song_whose_source_is_gone_says_which() {
    let s = Setup::new();
    let a = s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    std::fs::remove_file(&a).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(!ok, "{text}");
    assert!(
        text.contains("manual:a.flac is not in the sources"),
        "{text}"
    );
}

#[test]
fn a_lyrics_offset_past_an_hour_is_refused_before_it_is_shown() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let songs = s.dir.path().join("home/songs.toml");
    let before = std::fs::read_to_string(&songs).unwrap();
    for term in ["lyrics_offset=-2h", "lyrics_offset_ms=-7200000"] {
        let mut out = Vec::new();
        let e = run_with(
            &s.job(&["set", "-y", "manual:a.flac", term]),
            &flacs(),
            &Server::default(),
            None,
            &mut out,
            &mut Vec::new(),
        )
        .unwrap_err();
        let text = String::from_utf8(out).unwrap();
        assert!(
            e.downcast_ref::<change::Refused>().is_some(),
            "{term}: {e:#}"
        );
        assert!(format!("{e:#}").contains("more than an hour"), "{e:#}");
        assert!(!text.contains("changes"), "{term}: {text}");
        assert_eq!(std::fs::read_to_string(&songs).unwrap(), before);
    }
}

#[test]
fn check_during_a_run_reports_no_file_it_is_moving() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let lib = s.dir.path().join("lib");
    let from = lib.join("Artist/Record/02 Song.flac");
    let to = lib.join("Elsewhere/02 Song.flac");
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    let held = Lock::runs(&s.dir.path().join("home")).unwrap();
    std::fs::rename(&from, &to).unwrap();
    let (_, report) = s.report(&flacs(), &["check"]);
    drop(held);
    assert!(report.contains("run is under way"), "{report}");
    assert!(!report.contains("Missing"), "{report}");
    assert!(!report.contains("Not muman's"), "{report}");
}

/// The fake, but ffprobe reads nothing, as of a file damaged past reading.
struct Unprobed(Fake);

impl Runner for Unprobed {
    fn run(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<()> {
        self.0.run(cmd)
    }

    fn output(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<Vec<u8>> {
        if cmd
            .first()
            .is_some_and(|p| p.to_string_lossy().contains("ffprobe"))
        {
            anyhow::bail!("Invalid data found when processing input");
        }
        self.0.output(cmd)
    }

    fn stream(
        &self,
        cmd: &[std::ffi::OsString],
        on_line: &mut dyn FnMut(crate::runner::Line<'_>),
    ) -> anyhow::Result<bool> {
        self.0.stream(cmd, on_line)
    }
}

#[test]
fn a_source_that_reads_as_none_is_written_from_by_no_sync() {
    let s = Setup::new();
    let a = s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let before = library_of(&s);
    std::fs::write(&a, "garbage").unwrap();
    let _ = run_with(
        &s.job(&["sync"]),
        &Unprobed(flacs()),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    let mut out = Vec::new();
    let ok = run_with(
        &s.job(&["sync", "--force"]),
        &flacs(),
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!ok && text.contains("manual:a.flac"), "{text}");
    assert_eq!(library_of(&s), before, "held as it was: {text}");
}

/// The fake, but every ffmpeg run fails, as on a disk gone full.
struct NoFfmpeg(Fake);

impl Runner for NoFfmpeg {
    fn run(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<()> {
        if cmd
            .first()
            .is_some_and(|p| p.to_string_lossy().contains("ffmpeg"))
        {
            anyhow::bail!("No space left on device");
        }
        self.0.run(cmd)
    }

    fn output(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<Vec<u8>> {
        self.0.output(cmd)
    }

    fn stream(
        &self,
        cmd: &[std::ffi::OsString],
        on_line: &mut dyn FnMut(crate::runner::Line<'_>),
    ) -> anyhow::Result<bool> {
        if cmd
            .first()
            .is_some_and(|p| p.to_string_lossy().contains("ffmpeg"))
        {
            anyhow::bail!("No space left on device");
        }
        self.0.stream(cmd, on_line)
    }
}

#[test]
fn check_names_a_song_the_last_sync_could_not_write_until_one_writes_it() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let mut out = Vec::new();
    let ok = run_with(
        &s.job(&["sync"]),
        &NoFfmpeg(flacs()),
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!ok && text.contains("Failed"), "{text}");
    let (ok, report) = s.report(&flacs(), &["check"]);
    assert!(!ok, "{report}");
    assert!(
        report.contains("The last 1 sync(s) could not write it: manual:a.flac"),
        "{report}"
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, report) = s.report(&flacs(), &["check"]);
    assert!(ok && !report.contains("could not write"), "{report}");
}

#[test]
fn check_and_sync_agree_a_song_whose_source_reads_as_none_is_not_built_and_status_counts_it() {
    let s = Setup::new();
    let a = s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    std::fs::write(&a, "garbage").unwrap();
    let mut out = Vec::new();
    let ok = run_with(
        &s.job(&["sync"]),
        &Unprobed(flacs()),
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    assert!(!ok, "{}", String::from_utf8_lossy(&out));
    let (ok, report) = s.report(&flacs(), &["check"]);
    assert!(!ok, "{report}");
    assert!(
        report.contains("No sync can build it: manual:a.flac"),
        "{report}"
    );
    let (_, text) = s.run(&flacs(), &["status"]);
    assert!(
        text.contains("problem(s) found; `muman check` lists them"),
        "{text}"
    );
}

/// The fake, but ffprobe is stopped by a signal once.
struct StoppedOnce(Fake, std::sync::atomic::AtomicBool);

impl Runner for StoppedOnce {
    fn run(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<()> {
        self.0.run(cmd)
    }

    fn output(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<Vec<u8>> {
        let probe = cmd
            .first()
            .is_some_and(|p| p.to_string_lossy().contains("ffprobe"));
        if probe && !self.1.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(anyhow::Error::new(crate::runner::Exited {
                program: "ffprobe".into(),
                code: None,
                said: String::new(),
            }));
        }
        self.0.output(cmd)
    }

    fn stream(
        &self,
        cmd: &[std::ffi::OsString],
        on_line: &mut dyn FnMut(crate::runner::Line<'_>),
    ) -> anyhow::Result<bool> {
        self.0.stream(cmd, on_line)
    }
}

#[test]
fn a_source_a_signal_stopped_the_reading_of_is_read_by_the_next_sync() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let mut out = Vec::new();
    let _ = run_with(
        &s.job(&["sync"]),
        &StoppedOnce(flacs(), std::sync::atomic::AtomicBool::new(false)),
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    );
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("this time"), "{text}");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "no --retry needed: {text}");
    assert_eq!(library_of(&s).len(), 1, "{text}");
    let (ok, report) = s.report(&flacs(), &["check"]);
    assert!(
        ok && !report.contains("Failed"),
        "nothing kept failed: {report}"
    );
}

#[test]
fn a_source_that_reads_again_is_no_longer_reported_failed() {
    let s = Setup::new();
    let a = s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let whole = std::fs::read(&a).unwrap();
    std::fs::write(&a, "garbage").unwrap();
    let mut out = Vec::new();
    let _ = run_with(
        &s.job(&["sync"]),
        &Unprobed(flacs()),
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    );
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Could not read manual:a.flac"), "{text}");
    std::fs::write(&a, whole).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, report) = s.report(&flacs(), &["check"]);
    assert!(ok && !report.contains("Failed"), "{report}");
}

#[test]
fn check_reports_a_song_no_sync_can_build() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
    std::fs::write(
        &songs,
        listed.replace(&default, "template = \"{{ title | truncate('x') }}\""),
    )
    .unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(!ok && text.contains("Failed"), "{text}");
    let (ok, report) = s.report(&flacs(), &["check"]);
    assert!(!ok, "{report}");
    assert!(report.contains("No sync can build it"), "{report}");
    assert!(!report.contains("No problems found"), "{report}");
}

#[test]
fn a_template_naming_no_field_is_refused_before_the_sync_while_songs_still_list() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let before = library_of(&s);
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
    std::fs::write(
        &songs,
        listed.replace(&default, "template = \"{{ albm }}/{{ title }}\""),
    )
    .unwrap();
    s.file("home/sources/manual/b.flac");
    let e = s.refused(&["sync"]);
    assert!(e.contains("names albm"), "{e}");
    assert_eq!(library_of(&s), before);
    assert!(!std::fs::read_to_string(&songs).unwrap().contains("b.flac"));
    let edited = std::fs::read_to_string(&songs).unwrap();
    for args in [
        &["set", "-y", "manual:a.flac", "genre=Folk"][..],
        &["remove", "-y", "manual:a.flac"],
    ] {
        let e = s.refused(args);
        assert!(e.contains("names albm"), "{args:?}: {e}");
        assert_eq!(std::fs::read_to_string(&songs).unwrap(), edited, "{args:?}");
    }
    assert!(s.listed(&["list"]).contains("manual:a.flac"));
    let (ok, report) = s.report(&flacs(), &["check"]);
    assert!(!ok && report.contains("names albm"), "{report}");
}

#[test]
fn undo_waits_for_a_run_under_way_before_it_plans() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    s.run(&flacs(), &["set", "-y", "manual:a.flac", "genre=Folk"]);
    let held = Lock::runs(&s.dir.path().join("home")).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut out = Vec::new();
            let done = run_with(
                &s.job(&["undo", "--dry-run"]),
                &flacs(),
                &Server::default(),
                None,
                &mut out,
                &mut Vec::new(),
            );
            let done = done.map_err(|e| format!("{e:#}"));
            tx.send((done, String::from_utf8(out).unwrap())).unwrap();
        });
        let early = rx.recv_timeout(Duration::from_millis(500));
        assert!(
            early.is_err(),
            "planned while a run held the home: {early:?}"
        );
        drop(held);
        let (done, text) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(done.unwrap(), "{text}");
        assert!(text.contains("Undoing the last run"), "{text}");
        assert!(!text.contains("stopped before it ended"), "{text}");
    });
}

#[test]
fn undo_is_refused_before_saying_what_it_would_do() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    std::fs::write(&songs, format!("{listed}\n# edited by hand\n")).unwrap();
    for args in [&["undo", "-n"][..], &["undo", "-y"]] {
        let mut out = Vec::new();
        let e = run_with(
            &s.job(args),
            &flacs(),
            &Server::default(),
            None,
            &mut out,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
        assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
    }
}

#[test]
fn a_file_of_your_own_at_a_songs_path_is_written_over_only_when_forced() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let mine = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
    std::fs::write(&mine, "my own rip").unwrap();
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("Left alone, not muman's"), "{text}");
    assert_eq!(std::fs::read_to_string(&mine).unwrap(), "my own rip");

    let (ok, text) = s.run(&flacs(), &["sync", "--force"]);
    assert!(ok, "{text}");
    assert_ne!(std::fs::read(&mine).unwrap(), b"my own rip");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(
        std::fs::read_to_string(&mine).unwrap(),
        "my own rip",
        "{text}"
    );
    assert!(text.contains("Left alone, not muman's"), "{text}");
}

#[test]
fn lyrics_of_your_own_beside_a_song_are_written_over_only_when_forced_and_undone() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let song = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    let mine = song.with_extension("lrc");
    std::fs::write(&mine, "[00:01.00]my own words").unwrap();
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    std::fs::write(
        &songs,
        listed.replace("mode = \"tags\"", "mode = \"audio\""),
    )
    .unwrap();
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "a song with no lyrics leaves yours be: {text}");
    assert!(text.contains("Updated"), "{text}");
    let built = std::fs::read(&song).unwrap();

    s.written("home/sources/manual/a.lrc", "[00:01.00]Paper comets\n");
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(
        text.contains(
            "Left at Artist/Record/02 Song.flac: Artist/Record/02 Song.lrc is not muman's"
        ),
        "{text}"
    );
    assert_eq!(std::fs::read(&song).unwrap(), built, "{text}");
    let (_, report) = s.report(&flacs(), &["status", "--all"]);
    assert!(
        report.contains("left where it is, Artist/Record/02 Song.lrc not muman's"),
        "{report}"
    );

    let (ok, text) = s.run(&flacs(), &["sync", "--force"]);
    assert!(ok, "{text}");
    assert_eq!(
        std::fs::read_to_string(&mine).unwrap(),
        "[00:01.00]Paper comets\n",
        "{text}"
    );
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(
        std::fs::read_to_string(&mine).unwrap(),
        "[00:01.00]my own words",
        "{text}"
    );
    assert_eq!(std::fs::read(&song).unwrap(), built, "{text}");
}

#[test]
fn undo_of_a_forced_sync_leaves_a_file_retagged_by_hand_changed() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let file = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    let mut retagged = std::fs::read(&file).unwrap();
    retagged.extend_from_slice(b"a comment of my own");
    std::fs::write(&file, &retagged).unwrap();
    let (ok, text) = s.run(&flacs(), &["sync", "--force"]);
    assert!(ok, "{text}");
    assert_ne!(std::fs::read(&file).unwrap(), retagged);
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(std::fs::read(&file).unwrap(), retagged, "{text}");
    let mut report = Vec::new();
    run_with(
        &s.job(&["check"]),
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut report,
    )
    .unwrap();
    let report = String::from_utf8(report).unwrap();
    assert!(report.contains("Changed since muman wrote it"), "{report}");
    let (_, text) = s.run(&flacs(), &["set", "-y", "manual:a.flac", "genre=Folk"]);
    assert!(
        text.contains("Left alone, changed since muman wrote it"),
        "{text}"
    );
    assert_eq!(std::fs::read(&file).unwrap(), retagged, "{text}");
}

#[test]
fn a_source_cut_off_since_its_song_was_built_leaves_its_file_as_built() {
    let s = Setup::new();
    let source = s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let file = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    let built = std::fs::read(&file).unwrap();
    std::fs::write(&source, "a copy stopped short").unwrap();
    let mut cut = Fake {
        held: vec![("a.flac".into(), 3)],
        ..flacs()
    };
    let said = crate::testing::FLAC.replace(
        r#""sample_rate": "44100"}"#,
        r#""sample_rate": "44100", "duration": "200.0"}"#,
    );
    cut.probes.insert(0, ("a.flac".into(), said));
    let mut out = Vec::new();
    let ok = run_with(
        &s.job(&["sync"]),
        &cut,
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!ok, "a failure: {text}");
    assert!(
        text.contains("cut off") && text.contains("since its song was built"),
        "{text}"
    );
    assert!(!text.contains("Updated"), "{text}");
    assert_eq!(std::fs::read(&file).unwrap(), built, "{text}");
    let mut report = Vec::new();
    let clean = run_with(
        &s.job(&["check"]),
        &cut,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut report,
    )
    .unwrap();
    let report = String::from_utf8(report).unwrap();
    assert!(
        !clean && report.contains("Damaged audio") && report.contains("cut off"),
        "{report}"
    );
    let mut out = Vec::new();
    run_with(
        &s.job(&["sync", "--force"]),
        &cut,
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Updated (audio"), "{text}");
}

#[test]
fn a_source_cut_off_since_its_song_was_built_is_not_followed_though_its_file_is_gone() {
    let s = Setup::new();
    let source = s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let file = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    let held = std::fs::read_to_string(s.dir.path().join("home/songs.toml")).unwrap();
    std::fs::remove_file(&file).unwrap();
    std::fs::write(&source, "a copy stopped short").unwrap();
    let mut cut = Fake {
        held: vec![("a.flac".into(), 3)],
        ..flacs()
    };
    let said = crate::testing::FLAC.replace(
        r#""sample_rate": "44100"}"#,
        r#""sample_rate": "44100", "duration": "200.0"}"#,
    );
    cut.probes.insert(0, ("a.flac".into(), said));
    for run in ["the sync that finds it", "the sync after"] {
        let (ok, text) = s.run(&cut, &["sync"]);
        assert!(!ok, "{run} fails: {text}");
        assert!(
            text.contains("no file built before is left"),
            "{run}: {text}"
        );
        assert!(!text.contains("Removed"), "{run}: {text}");
        assert!(!file.exists(), "{run} writes nothing: {text}");
    }
    assert_eq!(
        std::fs::read_to_string(s.dir.path().join("home/songs.toml")).unwrap(),
        held,
        "the record keeps naming the audio the song was built from"
    );
    let (_, text) = s.run(&cut, &["sync", "--force"]);
    assert!(file.exists(), "{text}");
}

#[test]
fn check_decode_measures_each_source_afresh_and_names_its_damage() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let check = |fake: &Fake, args: &[&str]| {
        let mut report = Vec::new();
        let clean = run_with(
            &s.job(args),
            fake,
            &Server::default(),
            None,
            &mut Vec::new(),
            &mut report,
        )
        .unwrap();
        (clean, String::from_utf8(report).unwrap())
    };
    let (clean, report) = check(&flacs(), &["check", "--decode"]);
    assert!(clean, "{report}");
    // The file lost three seconds since it was measured.
    let mut list = "#tb 0: 1/1000\n".to_string();
    for n in (0..100).filter(|n| !(20..70).contains(n)) {
        let pts = n * 60;
        let _ = writeln!(list, "0, {pts}, {pts}, 60, 300, 0x0");
    }
    let holed = Fake {
        timelines: vec![("a.flac".into(), list)],
        ..flacs()
    };
    let (clean, report) = check(&holed, &["check"]);
    assert!(clean, "measured before: {report}");
    let (clean, report) = check(&holed, &["check", "--decode"]);
    assert!(!clean, "{report}");
    assert!(
        report.contains("Damaged audio: manual:a.flac lacks"),
        "{report}"
    );
}

#[test]
fn restore_puts_removed_songs_back_where_they_were() {
    let s = Setup::new();
    for n in 1..=4 {
        s.file(&format!("home/sources/manual/t{n}.flac"));
    }
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (order, before) = (s.songs(), library_of(&s));
    let go = |args: &[&str]| {
        let (ok, text) = s.run(&flacs(), args);
        assert!(ok, "{args:?}: {text}");
    };
    go(&["remove", "-y", "manual:t1.flac"]);
    go(&["restore", "-y", "manual:t1.flac"]);
    assert_eq!(s.songs(), order, "one alone");
    assert_eq!(library_of(&s), before, "one alone");
    for (removed, restored) in [(["t2", "t3"], ["t3", "t2"]), (["t3", "t2"], ["t3", "t2"])] {
        for t in removed {
            go(&["remove", "-y", &format!("manual:{t}.flac")]);
        }
        for t in restored {
            go(&["restore", "-y", &format!("manual:{t}.flac")]);
        }
        assert_eq!(s.songs(), order, "{removed:?} then {restored:?}");
        assert_eq!(library_of(&s), before, "{removed:?} then {restored:?}");
    }
}

/// Edit the song list `s` holds, `from` as `to`, returning it as it was.
fn edit_list(s: &Setup, from: &str, to: &str) -> String {
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    assert!(listed.contains(from), "{listed}");
    std::fs::write(&songs, listed.replacen(from, to, 1)).unwrap();
    listed
}

#[test]
fn one_undo_takes_back_a_run_killed_partway_and_the_run_that_finished_it() {
    let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
    one_undo_takes_back_a_sync_killed_at_any_step(
        &|s| {
            s.file("home/sources/manual/a.flac");
            s.file("home/sources/manual/b.flac");
        },
        &|s| edit_list(s, &default, "template = \"{{ title }} {{ id }}\""),
    );
}

#[test]
fn one_undo_takes_back_a_sync_killed_as_it_kept_files_it_writes_over() {
    one_undo_takes_back_a_sync_killed_at_any_step(
        &|s| {
            s.file("home/sources/manual/a.flac");
            s.written("home/sources/manual/a.lrc", "[00:01.00]Paper comets\n");
        },
        &|s| {
            edit_list(
                s,
                "format = \"lrc\"\nwhen = \"any\"",
                "format = \"txt\"\nwhen = \"any\"",
            )
        },
    );
}

#[test]
fn one_undo_takes_back_a_sync_killed_as_it_moved_and_retagged_a_song() {
    one_undo_takes_back_a_sync_killed_at_any_step(
        &|s| {
            s.file("home/sources/manual/a.flac");
            s.file("home/sources/manual/b.flac");
        },
        &|s| edit_list(s, "tags.title = \"\"", "tags.title = \"Rooms of Salt\""),
    );
}

#[test]
fn undo_of_a_run_that_moved_a_song_and_wrote_beside_it_takes_that_file_out() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let library = library_of(&s);
    s.written("home/sources/manual/a.lrc", "[00:01.00]Paper comets\n");
    edit_list(&s, "tags.title = \"\"", "tags.title = \"Rooms of Salt\"");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(text.contains("Moved"), "{text}");
    std::fs::remove_file(s.dir.path().join("home/sources/manual/a.lrc")).unwrap();
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(library_of(&s), library, "{text}");
}

#[test]
fn undo_of_a_forced_sync_that_wrote_a_held_song_anew_leaves_it_held() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let file = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    let mut retagged = std::fs::read(&file).unwrap();
    retagged.extend_from_slice(b"a comment of my own");
    std::fs::write(&file, &retagged).unwrap();
    edit_list(&s, "tags.title = \"\"", "tags.title = \"Rooms of Salt\"");
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("changed since muman wrote it"), "{text}");
    let held = library_of(&s);
    let (ok, text) = s.run(&flacs(), &["sync", "--force"]);
    assert!(ok, "{text}");
    assert_ne!(library_of(&s), held, "{text}");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(library_of(&s), held, "{text}");
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(text.contains("changed since muman wrote it"), "{text}");
    assert_eq!(library_of(&s), held, "{text}");
}

/// A sync after `edit`, of a home `prepare` fills and a sync builds,
/// killed at each step, then synced to its end: one undo puts back the
/// song list and the library as they were before it.
fn one_undo_takes_back_a_sync_killed_at_any_step(
    sources: &dyn Fn(&Setup),
    edit: &dyn Fn(&Setup) -> String,
) {
    let prepare = |s: &Setup| {
        sources(s);
        let (ok, text) = s.run(&flacs(), &["sync"]);
        assert!(ok, "{text}");
    };
    let whole = Setup::new();
    prepare(&whole);
    edit(&whole);
    let counted = crate::kill::kill_at(whole.dir.path(), usize::MAX);
    whole.run(&flacs(), &["sync"]);
    let steps = counted.seen();
    drop(counted);
    assert!(steps > 1, "{steps}");
    for at in 1..=steps {
        let s = Setup::new();
        prepare(&s);
        let (library, before) = (library_of(&s), edit(&s));
        let kill = crate::kill::kill_at(s.dir.path(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_with(
                &s.job(&["sync"]),
                &flacs(),
                &Server::default(),
                None,
                &mut Vec::new(),
                &mut Vec::new(),
            )
        }));
        drop(kill);
        assert!(stopped.is_err(), "not stopped at step {at} of {steps}");
        let (ok, text) = s.run(&flacs(), &["sync"]);
        assert!(ok, "step {at}: {text}");
        let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
        assert!(ok, "step {at}: {text}");
        let songs = s.dir.path().join("home/songs.toml");
        assert_eq!(
            std::fs::read_to_string(&songs).unwrap(),
            before,
            "step {at}: {text}"
        );
        assert_eq!(library_of(&s), library, "step {at}: {text}");
    }
}

#[test]
fn undo_after_undoing_a_sync_that_followed_a_source_goes_on() {
    let s = Setup::new();
    let a = s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let songs = s.dir.path().join("home/songs.toml");
    let first = std::fs::read_to_string(&songs).unwrap();
    s.run(&flacs(), &["set", "-y", "manual:a.flac", "genre=Folk"]);
    std::fs::write(&a, "retagged").unwrap();
    let retagged = || {
        Fake::default().probe(
            ".flac",
            &FLAC.replace("\"TITLE\": \"Song\"", "\"TITLE\": \"Song Two\""),
        )
    };
    let (ok, text) = s.run(&retagged(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&retagged(), &["undo", "-y"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&retagged(), &["undo", "-y"]);
    assert!(ok, "the set is undone too: {text}");
    let back = std::fs::read_to_string(&songs).unwrap();
    assert!(!back.contains("Folk"), "{back}");
    assert_ne!(back, first, "the source's change is held: {back}");
}

#[test]
fn undo_puts_a_removed_song_back() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let file = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    let before = std::fs::read(&file).unwrap();
    let (ok, text) = s.run(&flacs(), &["remove", "-y", "manual:a.flac"]);
    assert!(ok && !file.exists(), "{text}");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert!(text.contains("Put back"), "{text}");
    assert_eq!(std::fs::read(&file).unwrap(), before);
    assert_eq!(s.songs().len(), 1);
    assert!(
        text.contains("Up to date: 1 song(s)"),
        "nothing written again: {text}"
    );
}

/// The record of the latest run, as `run.json` holds it, and its path.
fn latest_record(s: &Setup) -> (PathBuf, serde_json::Value) {
    let history = s.dir.path().join("home/history");
    let last = std::fs::read_dir(&history)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("run.json").exists())
        .max()
        .unwrap();
    let file = last.join("run.json");
    let text = std::fs::read(&file).unwrap();
    (file, serde_json::from_slice(&text).unwrap())
}

#[test]
fn a_run_records_the_songs_it_changed_alone_and_undoes_them() {
    let s = Setup::new();
    for name in ["a", "b", "c"] {
        s.file(&format!("home/sources/manual/{name}.flac"));
    }
    s.run(&flacs(), &["sync"]);
    let synced = library_of(&s);
    let (ok, text) = s.run(
        &flacs(),
        &["set", "-y", "manual:b.flac", "album=Lanternfall"],
    );
    assert!(ok, "{text}");
    let (_, record) = latest_record(&s);
    assert_eq!(record["version"], 2);
    assert!(record.get("outputs").is_none(), "{record}");
    let before = record["before"].as_object().unwrap();
    assert_eq!(
        before.len(),
        2,
        "b where it was and where it goes: {record}"
    );
    assert_eq!(
        before.values().filter(|w| w.is_null()).count(),
        1,
        "the new path held nothing: {record}"
    );
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Up to date: 3 song(s)") && !text.contains("not muman's"),
        "each song is muman's as before: {text}"
    );
    assert_eq!(library_of(&s), synced);
}

#[test]
fn a_run_an_earlier_muman_recorded_whole_is_undone() {
    let s = Setup::new();
    for name in ["a", "b"] {
        s.file(&format!("home/sources/manual/{name}.flac"));
    }
    s.run(&flacs(), &["sync"]);
    let synced = library_of(&s);
    let state =
        serde_json::to_value(crate::state::State::load(&s.dir.path().join("home")).unwrap())
            .unwrap();
    let (ok, text) = s.run(&flacs(), &["remove", "-y", "manual:a.flac"]);
    assert!(ok, "{text}");
    // As version 1 wrote it: every output as the run began.
    let (file, mut record) = latest_record(&s);
    let fields = record.as_object_mut().unwrap();
    fields.remove("before");
    fields.insert("version".into(), 1.into());
    fields.insert("outputs".into(), state["outputs"].clone());
    std::fs::write(&file, serde_json::to_vec(&record).unwrap()).unwrap();
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok && text.contains("Put back"), "{text}");
    assert!(
        text.contains("Up to date: 2 song(s)"),
        "the song put back is muman's again: {text}"
    );
    assert_eq!(library_of(&s), synced);
}

#[test]
fn a_home_an_earlier_muman_left_syncs_as_it_was_and_undoes_its_run() {
    let s = Setup::new();
    for name in ["a", "b"] {
        s.file(&format!("home/sources/manual/{name}.flac"));
    }
    s.written("home/sources/manual/b.lrc", "[00:01.00]Paper comets\n");
    s.run(&flacs(), &["sync"]);
    let synced = library_of(&s);
    let home = s.dir.path().join("home");
    let began = serde_json::to_value(crate::state::State::load(&home).unwrap()).unwrap();
    let (ok, text) = s.run(&flacs(), &["remove", "-y", "manual:a.flac"]);
    assert!(ok, "{text}");
    let removed = library_of(&s);

    // As the last release left the home: its state in `state.json`, and
    // the run recorded whole, as version 1 of the record holds it.
    let left = crate::state::State::load(&home).unwrap();
    crate::state::put_old_state(&home, serde_json::to_string(&left).unwrap().as_bytes());
    let (file, mut record) = latest_record(&s);
    let fields = record.as_object_mut().unwrap();
    fields.remove("before");
    fields.insert("version".into(), 1.into());
    fields.insert("outputs".into(), began["outputs"].clone());
    std::fs::write(&file, serde_json::to_vec(&record).unwrap()).unwrap();

    let fake = flacs();
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
    assert_eq!(library_of(&s), removed, "nothing written again");
    assert!(
        !fake.calls().iter().any(|c| c.iter().any(|a| a == "-c:a")),
        "{text}"
    );
    assert!(crate::state::exists(&home), "brought into state.db");
    assert!(home.join("state.json.bak").exists());
    let stub = std::fs::read_to_string(home.join("state.json")).unwrap();
    assert!(
        stub.contains("moved_to"),
        "an earlier muman refuses the home: {stub}"
    );

    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok && text.contains("Put back"), "{text}");
    assert_eq!(
        library_of(&s),
        synced,
        "the run the last release made, undone"
    );
}

#[test]
fn info_prints_the_state_as_json_and_changes_nothing() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let home = s.dir.path().join("home");
    let written = crate::state::written_at(&home);
    let (ok, text) = s.report(&flacs(), &["info", "--state"]);
    assert!(ok, "{text}");
    let dumped: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        dumped["outputs"]
            .as_object()
            .unwrap()
            .contains_key("Artist/Record/02 Song.flac"),
        "{text}"
    );
    assert_eq!(crate::state::written_at(&home), written);

    // A home an earlier muman left is read as it is.
    let state = crate::state::State::load(&home).unwrap();
    crate::state::put_old_state(&home, serde_json::to_string(&state).unwrap().as_bytes());
    let mut again = Vec::new();
    crate::state::State::dump(&home, &mut again).unwrap();
    assert_eq!(String::from_utf8(again).unwrap(), text);
    assert!(!crate::state::exists(&home), "nothing brought over");
}

#[test]
fn set_refuses_a_number_or_date_that_is_none_and_a_measured_gain() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    for (term, said) in [
        ("track=abc", "no number"),
        ("disc=2/x", "no number"),
        ("date=yesterday", "no date"),
        ("replaygain_track_gain=loud", "[loudness]"),
    ] {
        let e = s.refused(&["set", "-y", "manual:a.flac", term]);
        assert!(e.contains(said), "{term}: {e}");
    }
    let (ok, text) = s.run(
        &flacs(),
        &["set", "-y", "manual:a.flac", "track=3/12", "date=2024-05"],
    );
    assert!(ok, "{text}");
}

#[test]
fn set_shows_and_stores_each_value_as_it_takes_effect() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    let tags = || -> BTreeMap<String, Vec<String>> {
        let m = Manifest::load(&s.dir.path().join("home")).unwrap();
        m.songs[0].tags.iter().cloned().collect()
    };
    let (ok, text) = s.run(
        &flacs(),
        &[
            "set",
            "-y",
            "manual:a.flac",
            "tracknumber=07",
            "date=2019-07-04 10:00",
        ],
    );
    assert!(ok, "{text}");
    assert!(text.contains("→ 7\n"), "{text}");
    assert!(text.contains("→ 2019-07-04\n"), "{text}");
    let songs = std::fs::read_to_string(s.dir.path().join("home/songs.toml")).unwrap();
    assert!(songs.contains("tracknumber = \"7\""), "{songs}");
    let (ok, text) = s.run(&flacs(), &["set", "-y", "manual:a.flac", "tracknumber=07"]);
    assert!(ok, "{text}");
    assert!(text.contains("Nothing changed"), "{text}");
    let (ok, text) = s.run(&flacs(), &["set", "-y", "manual:a.flac", "date=\u{200f}"]);
    assert!(ok, "{text}");
    assert!(text.contains("→ what the sources offer"), "{text}");
    assert!(!tags().contains_key("date"), "{:?}", tags());
}

#[test]
fn set_tags_a_song_and_writes_it_again() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.run(&flacs(), &["sync"]);
    s.refused(&["set", "genre=House"]);
    let (ok, text) = s.run(
        &flacs(),
        &["set", "-y", "manual:a.flac", "album=Lanternfall", "date!"],
    );
    assert!(ok, "{text}");
    assert!(text.contains("album: Record → Lanternfall"), "{text}");
    let m = Manifest::load(&s.dir.path().join("home")).unwrap();
    let tags: BTreeMap<String, Vec<String>> = m.songs[0].tags.iter().cloned().collect();
    assert_eq!(tags["album"], ["Lanternfall"]);
    assert!(
        s.dir
            .path()
            .join("lib/Artist/Lanternfall/02 Song.flac")
            .exists(),
        "{text}"
    );
    assert!(!s.dir.path().join("lib/Artist/Record").exists(), "{text}");
}

#[test]
fn list_writes_each_matching_song_to_its_own_stream() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    s.run(&flacs(), &["sync"]);
    assert_eq!(
        s.listed(&["list"]),
        "manual:a.flac\tArtist\tSong\tRecord\nmanual:b.flac\tArtist\tSong\tRecord\n"
    );
    assert_eq!(
        s.listed(&["list", "--keys", "manual:b.flac"]),
        "manual:b.flac\n"
    );
    assert_eq!(
        s.listed(&["list", "-f", "{path}", "^key:a.flac"]),
        "Artist/Record/02 Song [b].flac\n"
    );
    assert_eq!(s.listed(&["list", "nothing-like-it"]), "");
}

impl Setup {
    fn run_against(&self, server: &Server, args: &[&str]) -> (bool, String) {
        let mut out = Vec::new();
        let ok = run_with(
            &self.job(args),
            &flacs(),
            server,
            None,
            &mut out,
            &mut Vec::new(),
        )
        .unwrap();
        (ok, String::from_utf8(out).unwrap())
    }
}

const RECORD: &str = r#"{"id": 7, "trackName": "Song", "artistName": "Artist", "albumName": "Record",
    "duration": 200.5, "syncedLyrics": "[00:01.00] la la\n[00:30.00] la"}"#;

#[test]
fn an_untimed_record_is_asked_about_again_and_kept_anew_once_timed() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let untimed = r#"{"id": 7, "trackName": "Song", "artistName": "Artist", "albumName": "Record",
        "duration": 200.5, "plainLyrics": "la la\nla"}"#;
    let first = Server::default().answer("/api/get?", untimed);
    let (ok, text) = s.run_against(&first, &["sync"]);
    assert!(
        ok && text.contains("untimed lyrics from LRCLIB, lrclib:7"),
        "{text}"
    );
    let home = s.dir.path().join("home");
    let looked = |state: &State| {
        state
            .lookups
            .iter()
            .find(|l| l.find == "lrclib")
            .cloned()
            .unwrap()
    };
    assert_eq!(looked(&State::load(&home).unwrap()).unchanged, 0);

    let same = Server::default().answer("/api/get?", untimed);
    let (ok, text) = s.run_against(&same, &["sync", "--rematch"]);
    assert!(ok, "{text}");
    assert_eq!(
        looked(&State::load(&home).unwrap()).unchanged,
        1,
        "found again unchanged: it waits longer"
    );

    let timed = Server::default().answer("/api/get?", RECORD);
    let (ok, text) = s.run_against(&timed, &["sync", "--rematch"]);
    assert!(ok, "{text}");
    assert_eq!(looked(&State::load(&home).unwrap()).unchanged, 0);
    let lrc = std::fs::read_to_string(s.dir.path().join("lib/Artist/Record/02 Song.lrc")).unwrap();
    assert!(
        lrc.contains("[00:01.00]"),
        "the stored copy took its timing: {lrc}"
    );
}

#[test]
fn a_song_of_your_own_finds_its_lyrics_on_lrclib_once() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let server = Server::default().answer("/api/get?", RECORD);
    let (ok, text) = s.run_against(&server, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("timed lyrics from LRCLIB, lrclib:7"),
        "{text}"
    );
    assert_eq!(
        s.songs(),
        [vec![
            SourceKey::Manual("a.flac".into()),
            SourceKey::parse("lrclib:7").unwrap()
        ]]
    );
    let lrc = std::fs::read_to_string(s.dir.path().join("lib/Artist/Record/02 Song.lrc")).unwrap();
    assert!(lrc.contains("la la"), "{lrc}");
    assert!(
        !server
            .asked
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.contains("youtube")),
        "nothing is asked of YouTube"
    );

    let again = Server::default();
    let (ok, text) = s.run_against(&again, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        again.asked.lock().unwrap().is_empty(),
        "found once, not asked again"
    );

    std::fs::remove_file(s.dir.path().join("home/sources/lrclib/7.lrc")).unwrap();
    let list = s.dir.path().join("home").join(crate::dirs::MANIFEST);
    let listed = std::fs::read_to_string(&list).unwrap();
    let off = "[providers.lrclib]\nenabled = false";
    std::fs::write(
        &list,
        listed.replace("[providers.lrclib]\nenabled = true", off),
    )
    .unwrap();
    let off = Server::default().answer("/api/get/7", RECORD);
    let (_, text) = s.run_against(&off, &["sync"]);
    assert!(off.asked.lock().unwrap().is_empty(), "{text}");
    assert!(
        text.contains("1 missing record(s) whose provider is off"),
        "{text}"
    );
    std::fs::write(&list, listed).unwrap();
    let refetch = Server::default().answer("/api/get/7", RECORD);
    let (ok, text) = s.run_against(&refetch, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        s.dir.path().join("home/sources/lrclib/7.lrc").exists(),
        "{text}"
    );
}

#[test]
fn two_songs_of_one_recording_share_its_lyrics_and_a_purge_spares_them() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let server = Server::default().answer("/api/get?", RECORD);
    let (ok, text) = s.run_against(&server, &["sync", "--new"]);
    assert!(ok, "{text}");
    let record = SourceKey::parse("lrclib:7").unwrap();
    let songs = s.songs();
    assert_eq!(songs.len(), 2, "{songs:?}");
    assert!(songs.iter().all(|k| k.contains(&record)), "{songs:?}");
    let state = State::load(&s.dir.path().join("home")).unwrap();
    assert!(
        state
            .lookups
            .iter()
            .filter(|l| l.find == "lrclib")
            .all(|l| l.outcome == crate::state::Outcome::Found(record.clone())),
        "{:?}",
        state.lookups
    );

    let (ok, text) = s.run_against(
        &Server::default(),
        &["remove", "-y", "--purge", "manual:a.flac"],
    );
    assert!(ok, "{text}");
    assert!(
        s.dir.path().join("home/sources/lrclib/7.lrc").exists(),
        "the other song still lists it: {text}"
    );
}

#[test]
fn offline_a_sync_asks_no_service_and_an_add_fetches_no_url() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let server = Server::default().answer("/api/get?", RECORD);
    let (ok, text) = s.run_against(&server, &["sync", "--offline"]);
    assert!(ok, "{text}");
    assert!(server.asked.lock().unwrap().is_empty(), "{text}");
    assert_eq!(s.songs().len(), 1);
    let e = run_with(
        &s.job(&[
            "add",
            "--offline",
            "https://youtube.com/watch?v=aaaaaaaaaaa",
        ]),
        &flacs(),
        &server,
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e}").contains("--offline"), "{e:#}");
}

#[test]
fn a_purge_spares_what_a_removed_song_lists_so_that_restore_gets_it_whole() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let server = Server::default().answer("/api/get?", RECORD);
    let (ok, text) = s.run_against(&server, &["sync", "--new"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run_against(&Server::default(), &["remove", "-y", "manual:b.flac"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run_against(
        &Server::default(),
        &["remove", "-y", "--purge", "manual:a.flac"],
    );
    assert!(ok, "{text}");
    let lrc = s.dir.path().join("home/sources/lrclib/7.lrc");
    assert!(lrc.exists(), "{text}");
    assert!(text.contains("which a removed song lists"), "{text}");
    let (ok, text) = s.run_against(&Server::default(), &["restore", "-y", "manual:b.flac"]);
    assert!(ok, "{text}");
    let record = SourceKey::parse("lrclib:7").unwrap();
    assert!(s.songs().iter().any(|k| k.contains(&record)), "{text}");
}

#[test]
fn undo_of_a_purge_waits_for_the_file_of_your_own_back_from_the_trash() {
    let s = Setup::new();
    let a = s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&flacs(), &["remove", "-y", "--purge", "manual:a.flac"]);
    assert!(ok && !Path::new(&a).exists(), "{text}");
    let said = s.refused(&["undo", "-y"]);
    assert!(
        said.contains("moved") && said.contains("to the trash"),
        "{said}"
    );
    assert_eq!(s.songs().len(), 0);
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 1, "{text}");
}

#[test]
fn a_purge_stopped_at_any_step_still_asks_for_its_file_back_from_the_trash() {
    let prepare = |s: &Setup| {
        let a = s.file("home/sources/manual/a.flac");
        let (ok, text) = s.run(&flacs(), &["sync"]);
        assert!(ok, "{text}");
        a
    };
    let purge = ["remove", "-y", "--purge", "manual:a.flac"];
    let whole = Setup::new();
    prepare(&whole);
    let counted = crate::kill::kill_at(whole.dir.path(), usize::MAX);
    whole.run(&flacs(), &purge);
    let steps = counted.seen();
    drop(counted);
    for at in 1..=steps {
        let s = Setup::new();
        let a = prepare(&s);
        let kill = crate::kill::kill_at(s.dir.path(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_with(
                &s.job(&purge),
                &flacs(),
                &Server::default(),
                None,
                &mut Vec::new(),
                &mut Vec::new(),
            )
        }));
        drop(kill);
        assert!(stopped.is_err(), "not stopped at step {at} of {steps}");
        let (ok, text) = s.run(&flacs(), &["sync"]);
        assert!(ok, "step {at}: {text}");
        if s.songs().len() == 1 {
            assert!(Path::new(&a).exists(), "step {at}: {text}");
            continue;
        }
        if !Path::new(&a).exists() {
            let said = s.refused(&["undo", "-y"]);
            assert!(said.contains("to the trash"), "step {at}: {said}");
            s.file("home/sources/manual/a.flac");
        }
        let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
        assert!(ok, "step {at}: {text}");
        assert_eq!(s.songs().len(), 1, "step {at}: {text}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn undo_of_a_purge_finds_a_file_of_a_decomposed_name_put_back() {
    let s = Setup::new();
    let given = s.file("home/sources/manual/cafe\u{301}.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(
        &flacs(),
        &["remove", "-y", "--purge", "manual:caf\u{e9}.flac"],
    );
    assert!(ok && !Path::new(&given).exists(), "{text}");
    s.file("home/sources/manual/cafe\u{301}.flac");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 1, "{text}");
}

#[test]
fn restore_of_a_purged_song_waits_for_its_files_back_from_the_trash() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&flacs(), &["remove", "-y", "--purge", "manual:a.flac"]);
    assert!(ok, "{text}");
    let said = s.refused(&["restore", "-y", "manual:a.flac"]);
    assert!(
        said.contains("a.flac") && said.contains("put them back"),
        "{said}"
    );
    assert_eq!(s.songs().len(), 1);
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["restore", "-y", "manual:a.flac"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 2, "{text}");
}

const MBID: &str = "00000000-0000-4000-8000-000000000001";

/// A recording of `Song` by `Artist` on the album `Glass Orchards`, as a
/// search (`track`) or a lookup (`tracks`) lists its track.
fn recording(tracks: &str) -> String {
    format!(
        r#"{{"id": "{MBID}", "score": 100, "title": "Song", "length": 200400,
            "artist-credit": [{{"name": "Artist"}}],
            "releases": [{{"id": "00000000-0000-4000-8000-00000000000a", "title": "Glass Orchards",
                "status": "Official", "date": "2011-03-04", "artist-credit": [{{"name": "Artist"}}],
                "release-group": {{"primary-type": "Album", "secondary-types": []}},
                "media": [{{"position": 1, "track-offset": 3, "{tracks}": [{{"number": "4"}}]}}]}}]}}"#
    )
}

#[test]
fn a_record_of_another_song_is_set_aside_and_status_says_why() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.written(
        &format!("home/sources/musicbrainz/{MBID}.json"),
        &format!(
            r#"{{"id": "{MBID}", "title": "Night Ferry", "artists": ["Kiri Hoshino"],
                "release": {{"id": "00000000-0000-4000-8000-00000000000a",
                "title": "Harbor Lights", "artists": ["Kiri Hoshino"], "track": 3}}}}"#
        ),
    );
    s.written(
        "home/songs.toml",
        &format!(
            "version = 1\n\n[[song]]\nsources = [\"manual:a.flac\", \"musicbrainz:{MBID}\"]\n"
        ),
    );
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(
        s.dir.path().join("lib/Artist/Record/02 Song.flac").exists(),
        "the file's own album: {text}"
    );
    let (_, report) = s.report(&flacs(), &["status", "a.flac"]);
    assert!(
        report.contains(&format!(
            "  set aside  musicbrainz:{MBID}: titled \"Night Ferry\" by Kiri Hoshino on \
             \"Harbor Lights\": no title or credit in common"
        )),
        "{report}"
    );
}

#[test]
fn status_names_a_source_of_another_recording_than_the_song_s() {
    let s = Setup::new();
    for name in ["a", "b"] {
        s.file(&format!("home/sources/manual/{name}.flac"));
    }
    s.written(
        &format!("home/sources/musicbrainz/{MBID}.json"),
        &format!(
            r#"{{"id": "{MBID}", "title": "Song", "artists": ["Artist"], "length_ms": 200500}}"#
        ),
    );
    s.written(
        "home/songs.toml",
        &format!(
            "version = 1\n\n[[song]]\nsources = [\"manual:b.flac\", \"manual:a.flac\", \
             \"musicbrainz:{MBID}\"]\n"
        ),
    );
    let named = FLAC.replace(
        r#""track": "2""#,
        &format!(r#""MUSICBRAINZ_TRACKID": "{MBID}""#),
    );
    let fake = || {
        let mut fake = Fake::default()
            .probe("b.flac", &FLAC.replace("200.0", "231.0"))
            .probe(".flac", &named);
        fake.pcm = vec![
            ("a.flac".into(), pcm(&words(4000, 2))),
            ("b.flac".into(), pcm(&words(4000, 3))),
        ];
        fake
    };
    let (_, report) = s.report(&fake(), &["status", "--all"]);
    assert!(
        report.contains(
            "audio   manual:a.flac: the song's recording by its record: aligns with none"
        ),
        "{report}"
    );
    assert!(
        report.contains(
            "recording  manual:b.flac: another recording than the song's 1 other: aligns with \
             none, 31 s longer than its record"
        ),
        "{report}"
    );
}

#[test]
fn a_song_makes_one_lookup_a_provider_a_run_whatever_its_recheck() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n[providers.lrclib]\nrecheck = \"0 days\"\n",
    )
    .unwrap();
    let fake = Fake::default().probe(".flac", &FLAC.replace(r#""ALBUM": "Record", "#, ""));
    let search = format!(r#"{{"recordings": [{}]}}"#, recording("track"));
    let server = Server::default().answer("/ws/2/recording?query=", &search);
    let mut out = Vec::new();
    let ok = run_with(
        &s.job(&["sync"]),
        &fake,
        &server,
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(ok, "{text}");
    assert!(
        text.contains("tags from MusicBrainz"),
        "a second round: {text}"
    );
    let asked = server.asked.lock().unwrap();
    let lrclib = asked
        .iter()
        .filter(|u| u.contains("lrclib.net/api/"))
        .count();
    assert_eq!(lrclib, 1, "{asked:?}");
}

#[test]
fn a_song_on_no_album_finds_its_album_on_musicbrainz_once() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let fake = Fake::default().probe(".flac", &FLAC.replace(r#""ALBUM": "Record", "#, ""));
    let search = format!(r#"{{"recordings": [{}]}}"#, recording("track"));
    let run = |server: &Server| {
        let mut out = Vec::new();
        let ok = run_with(
            &s.job(&["sync"]),
            &fake,
            server,
            None,
            &mut out,
            &mut Vec::new(),
        )
        .unwrap();
        (ok, String::from_utf8(out).unwrap())
    };
    let server = Server::default().answer("/ws/2/recording?query=", &search);
    let (ok, text) = run(&server);
    assert!(ok, "{text}");
    assert!(
        text.contains(&format!(
            "tags from MusicBrainz, musicbrainz:{MBID}, on Glass Orchards"
        )),
        "{text}"
    );
    assert_eq!(
        s.songs(),
        [vec![
            SourceKey::Manual("a.flac".into()),
            SourceKey::parse(&format!("musicbrainz:{MBID}")).unwrap()
        ]]
    );
    assert!(
        s.dir
            .path()
            .join("lib/Artist/Glass Orchards/04 Song.flac")
            .exists(),
        "{text}"
    );

    let again = Server::default();
    let (ok, text) = run(&again);
    assert!(ok, "{text}");
    assert!(
        !again
            .asked
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.contains("/ws/2/")),
        "found once, not asked again"
    );

    let kept = s
        .dir
        .path()
        .join(format!("home/sources/musicbrainz/{MBID}.json"));
    std::fs::remove_file(&kept).unwrap();
    let refetch =
        Server::default().answer(&format!("/ws/2/recording/{MBID}?"), &recording("tracks"));
    let (ok, text) = run(&refetch);
    assert!(ok, "{text}");
    assert!(kept.exists(), "{text}");
    assert!(
        s.dir
            .path()
            .join("lib/Artist/Glass Orchards/04 Song.flac")
            .exists(),
        "{text}"
    );
}

#[test]
fn an_acoustid_lookup_keeps_every_recording_it_named_with_its_releases() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let fake = flacs().print("a.flac", words(1600, 1));
    let other = "00000000-0000-4000-8000-000000000002";
    let answer = format!(
        r#"{{"status": "ok", "results": [{{"id": "x", "score": 0.95, "recordings": [
            {{"id": "{MBID}", "title": "Song", "duration": 200, "sources": 9,
              "artists": [{{"name": "Artist"}}],
              "releasegroups": [{{"id": "00000000-0000-4000-8000-00000000000b", "type": "Album",
                "releases": [{{"id": "00000000-0000-4000-8000-00000000000a",
                  "title": "Glass Orchards", "date": {{"year": 2011}},
                  "mediums": [{{"position": 1, "track_count": 9, "tracks": [{{"position": 4}}]}}]}}]}}]}},
            {{"id": "{other}", "title": "Song", "duration": 201, "sources": 2,
              "artists": [{{"name": "Artist"}}]}}]}}]}}"#
    );
    let server = Server::default()
        .answer("/v2/lookup?", &answer)
        .answer(&format!("/ws/2/recording/{MBID}?"), &recording("tracks"));
    let mut out = Vec::new();
    let ok = run_with(
        &s.job(&["sync"]),
        &fake,
        &server,
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(ok, "{text}");
    assert!(text.contains("by its fingerprint on AcoustID"), "{text}");
    let state = State::load(&s.dir.path().join("home")).unwrap();
    let record = SourceKey::parse(&format!("musicbrainz:{MBID}")).unwrap();
    let candidates = state.candidates(&[SourceKey::Manual("a.flac".into())], &record);
    let ids: Vec<String> = candidates.iter().map(|c| c.id.to_string()).collect();
    assert_eq!(ids, [MBID, other], "{text}");
    let release = &candidates[0].releases[0];
    assert_eq!(
        (release.title.as_str(), release.track, release.tracks),
        ("Glass Orchards", Some(4), Some(9))
    );
}

#[test]
fn a_song_that_has_what_until_names_asks_nothing_of_musicbrainz() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n[providers.acoustid]\nuntil = \"has:album\"\n\
         [providers.musicbrainz]\nuntil = [\"has:album\"]\n",
    )
    .unwrap();
    let server = Server::default();
    let (ok, text) = s.run_against(&server, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        !server
            .asked
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.contains("/ws/2/recording")),
        "{:?}",
        server.asked
    );
}

#[test]
fn a_song_on_an_album_lacking_its_date_and_track_asks_musicbrainz() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let server = Server::default();
    let (ok, text) = s.run_against(&server, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        server
            .asked
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.contains("/ws/2/recording")),
        "{:?}",
        server.asked
    );
}

#[test]
fn lookups_a_refusing_service_left_wait_for_the_next_run_unrecorded() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let refusing = Server {
        refusing: true,
        ..Server::default()
    };
    let (_, text) = s.run_against(&refusing, &["sync"]);
    assert!(
        text.contains("LRCLIB refuses requests for going too fast: 2 lookup(s) on lrclib wait"),
        "{text}"
    );
    let asked = refusing
        .asked
        .lock()
        .unwrap()
        .iter()
        .filter(|u| u.contains("/api/"))
        .count();
    assert!(
        asked <= 8,
        "a request and three retries per lookup under way: {asked}"
    );
    let state = State::load(&s.dir.path().join("home")).unwrap();
    assert!(state.lookups.is_empty(), "{:?}", state.lookups);
    let server = Server::default().answer("/api/get?", RECORD);
    let (ok, text) = s.run_against(&server, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("lyrics from LRCLIB"),
        "asked again at once: {text}"
    );
}

#[test]
fn lookups_on_a_network_that_is_down_wait_for_the_next_run_unrecorded() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let down = Server {
        unreachable: true,
        ..Server::default()
    };
    let (_, text) = s.run_against(&down, &["sync"]);
    assert!(
        text.contains(
            "LRCLIB cannot be reached (connection refused): 2 lookup(s) on lrclib wait for the next run"
        ),
        "{text}"
    );
    assert!(!text.contains("lookup failed"), "{text}");
    let asked = down
        .asked
        .lock()
        .unwrap()
        .iter()
        .filter(|u| u.contains("/api/"))
        .count();
    assert!(
        asked <= 2,
        "one request per lookup under way at most: {asked}"
    );
    let state = State::load(&s.dir.path().join("home")).unwrap();
    assert!(state.lookups.is_empty(), "{:?}", state.lookups);
    let server = Server::default().answer("/api/get?", RECORD);
    let (ok, text) = s.run_against(&server, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("lyrics from LRCLIB"),
        "asked again at once: {text}"
    );
}

#[test]
fn a_listed_record_no_longer_there_is_asked_for_again_only_when_due() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let server = Server::default().answer("/api/get?", RECORD);
    let (ok, text) = s.run_against(&server, &["sync"]);
    assert!(ok, "{text}");
    let kept = s.dir.path().join("home/sources/lrclib/7.lrc");
    std::fs::remove_file(&kept).unwrap();
    let key = SourceKey::parse("lrclib:7").unwrap();
    let failure = || {
        State::load(&s.dir.path().join("home"))
            .unwrap()
            .failures
            .get(&key)
            .map(|f| f.count)
    };
    let asked_again = |server: &Server| {
        server
            .asked
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.contains("/api/get/7"))
    };

    let down = Server {
        unreachable: true,
        ..Server::default()
    };
    let (_, text) = s.run_against(&down, &["sync"]);
    assert!(
        text.contains("1 missing record(s) wait for the next run"),
        "{text}"
    );
    assert_eq!(failure(), None, "a network that is down fails nothing");

    let gone = Server::default();
    let (_, text) = s.run_against(&gone, &["sync"]);
    assert!(asked_again(&gone), "{text}");
    assert!(
        text.contains("Could not fetch lrclib:7 again: no such record"),
        "{text}"
    );
    assert_eq!(failure(), Some(1));

    let gone = Server::default();
    let (_, text) = s.run_against(&gone, &["sync"]);
    assert!(!asked_again(&gone), "it waits an hour: {text}");
    assert!(
        text.contains("Not fetching 1 missing record(s) that failed lately"),
        "{text}"
    );
    assert_eq!(failure(), Some(1));

    let back = Server::default().answer("/api/get/7", RECORD);
    let (ok, text) = s.run_against(&back, &["sync", "--retry"]);
    assert!(ok, "{text}");
    assert!(asked_again(&back) && kept.exists(), "{text}");
    assert_eq!(failure(), None, "found again, it is cleared");
}

#[test]
fn a_song_lrclib_lacks_is_not_asked_again_for_a_week() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let none = Server::default();
    s.run_against(&none, &["sync"]);
    assert!(!none.asked.lock().unwrap().is_empty());
    let none = Server::default();
    s.run_against(&none, &["sync"]);
    assert!(none.asked.lock().unwrap().is_empty());
    let none = Server::default();
    s.run_against(&none, &["sync", "--rematch"]);
    assert!(
        !none.asked.lock().unwrap().is_empty(),
        "--rematch asks at once"
    );
}

#[test]
fn the_hook_runs_once_a_run_changed_anything() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n[[hook]]\non = \"changed\"\nrun = [\"mpc\", \"update\", \"{written}\"]\n",
    )
    .unwrap();
    let fake = flacs();
    let (ok, text) = s.run(&fake, &["--trust", "sync"]);
    assert!(ok, "{text}");
    assert!(
        fake.calls().iter().any(|c| c == &["mpc", "update", "1"]),
        "{:?}",
        fake.calls()
    );
    let fake = flacs();
    s.run(&fake, &["sync"]);
    assert!(!fake.ran("mpc"), "nothing changed, no hook");
}

#[test]
fn the_changed_hook_runs_after_a_run_that_only_put_back_or_moved_files() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let songs = s.dir.path().join("home/songs.toml");
    let hook = "[[hook]]\non = \"changed\"\n\
                run = [\"mpc\", \"update\", \"{written}\", \"{moved}\", \"{removed}\"]\n";
    std::fs::write(&songs, format!("version = 1\n{hook}")).unwrap();
    let hooked = |args: &[&str]| {
        let fake = flacs();
        let mut out = Vec::new();
        run_with(
            &s.job(args),
            &fake,
            &Server::default(),
            None,
            &mut out,
            &mut Vec::new(),
        )
        .unwrap();
        let ran: Vec<Vec<String>> = fake
            .calls()
            .into_iter()
            .filter(|c| c.first().is_some_and(|p| p == "mpc"))
            .collect();
        (ran, String::from_utf8(out).unwrap())
    };
    hooked(&["--trust", "sync"]);
    hooked(&["set", "-y", "manual:a.flac", "genre=Folk"]);
    let (ran, text) = hooked(&["undo", "-y"]);
    assert!(text.contains("Put back:"), "{text}");
    assert_eq!(ran, [["mpc", "update", "1", "0", "0"]], "{text}");
    let listed = std::fs::read_to_string(&songs).unwrap();
    let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
    assert!(listed.contains(&default), "{listed}");
    std::fs::write(
        &songs,
        listed.replace(&default, "template = \"{{ title }}\""),
    )
    .unwrap();
    let (ran, text) = hooked(&["sync"]);
    assert!(text.contains("Moved:"), "{text}");
    assert_eq!(ran, [["mpc", "update", "0", "1", "0"]], "{text}");
}

#[test]
fn a_hook_refused_disables_itself_alone() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n[[hook]]\non = \"written\"\nrun = [\"tagger\", \"{path}\"]\n\
         [[hook]]\non = \"changed\"\nrun = \"mpc update\"\n\
         [[hook]]\non = \"changed\"\nrun = [\"mpc\", \"update\"]\n",
    )
    .unwrap();
    let fake = flacs();
    let (ok, text) = s.run(&fake, &["--trust", "sync"]);
    assert!(ok, "{text}");
    assert!(!fake.ran("tagger"), "{:?}", fake.calls());
    assert!(fake.ran("mpc"), "{:?}", fake.calls());
    assert!(text.contains("Hook 1 not run"), "{text}");
    assert!(text.contains("Hook 2 not run"), "{text}");
    for args in [&["list"][..], &["set", "-y", "manual:a.flac", "genre=Folk"]] {
        let (ok, text) = s.run(&flacs(), args);
        assert!(ok, "{args:?}: {text}");
    }
    let mut report = Vec::new();
    let ok = run_with(
        &s.job(&["check"]),
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut report,
    )
    .unwrap();
    let report = String::from_utf8(report).unwrap();
    assert!(!ok && report.contains("Hook 1 is never run"), "{report}");
}

/// Says, when the hook's program runs, whether a run held the home.
struct Watching<'a> {
    fake: Fake,
    home: &'a Path,
    busy: std::sync::Mutex<Vec<bool>>,
}

impl Runner for Watching<'_> {
    fn run(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<()> {
        self.fake.run(cmd)
    }

    fn output(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<Vec<u8>> {
        self.fake.output(cmd)
    }

    fn stream(
        &self,
        cmd: &[std::ffi::OsString],
        on_line: &mut dyn FnMut(crate::runner::Line<'_>),
    ) -> anyhow::Result<bool> {
        self.fake.stream(cmd, on_line)
    }

    fn stream_env(
        &self,
        cmd: &[std::ffi::OsString],
        env: &[(String, String)],
        on_line: &mut dyn FnMut(crate::runner::Line<'_>),
    ) -> anyhow::Result<bool> {
        if cmd.first().is_some_and(|p| p == "mpc") {
            self.busy.lock().unwrap().push(Lock::busy(self.home));
        }
        self.fake.stream_env(cmd, env, on_line)
    }
}

#[test]
fn the_hook_runs_once_its_run_has_let_go_of_the_home() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n[[hook]]\non = \"changed\"\nrun = [\"mpc\", \"update\"]\n",
    )
    .unwrap();
    let home = s.dir.path().join("home");
    let watching = Watching {
        fake: flacs(),
        home: &home,
        busy: std::sync::Mutex::default(),
    };
    let mut out = Vec::new();
    let done = run_with(
        &s.job(&["--trust", "sync"]),
        &watching,
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    );
    let text = String::from_utf8(out).unwrap();
    assert!(done.unwrap(), "{text}");
    assert_eq!(*watching.busy.lock().unwrap(), [false], "{text}");
}

#[test]
fn a_muman_a_hook_starts_runs_no_hook_and_may_use_its_home() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n[[hook]]\non = \"changed\"\nrun = [\"mpc\", \"update\"]\n",
    )
    .unwrap();
    let mut job = s.job(&["--trust", "sync"]);
    job.hooked = Some(Hooked {
        home: s.dir.path().join("home"),
        library: Some(s.dir.path().join("lib")),
    });
    let fake = flacs();
    let mut out = Vec::new();
    let done = run_with(
        &job,
        &fake,
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    );
    let text = String::from_utf8(out).unwrap();
    assert!(done.unwrap(), "{text}");
    assert!(text.contains("Added:"), "{text}");
    assert!(!fake.ran("mpc"), "{:?}", fake.calls());
}

impl Setup {
    /// Run `args` with `prompter` answering, if any; the error, if any.
    fn trusting(
        &self,
        fake: &Fake,
        args: &[&str],
        prompter: Option<&mut crate::ui::MockPrompter>,
    ) -> Result<String> {
        let mut out = Vec::new();
        let prompter = prompter.map(|p| p as &mut dyn Prompter);
        run_with(
            &self.job(args),
            fake,
            &Server::default(),
            prompter,
            &mut out,
            &mut Vec::new(),
        )?;
        Ok(String::from_utf8(out).unwrap())
    }
}

const HOOKED: &str = "version = 1\n[[hook]]\non = \"changed\"\nrun = [\"mpc\", \"update\"]\n";

#[test]
fn a_song_list_s_hook_runs_only_once_trusted_and_y_trusts_nothing() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    std::fs::write(s.dir.path().join("home/songs.toml"), HOOKED).unwrap();
    for args in [
        &["sync"][..],
        &["set", "-y", "manual:a.flac", "genre=Folk"],
        &["info"],
    ] {
        let fake = flacs();
        let e = s.trusting(&fake, args, None).unwrap_err();
        assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
        assert!(format!("{e}").contains("--trust"), "{e}");
        assert_eq!(fake.calls(), Vec::<Vec<String>>::new(), "{args:?}");
    }
    assert!(s.trusting(&flacs(), &["list"], None).is_ok());
    let fake = flacs();
    let mut no = crate::ui::MockPrompter::answering([false]);
    let text = s.trusting(&fake, &["sync"], Some(&mut no)).unwrap();
    assert!(
        text.contains("[[hook]] 1 (new): on = \"changed\""),
        "{text}"
    );
    assert!(fake.calls().is_empty(), "declined, nothing runs");
    let fake = flacs();
    let mut yes = crate::ui::MockPrompter::answering([true]);
    s.trusting(&fake, &["sync"], Some(&mut yes)).unwrap();
    assert!(fake.ran("mpc"), "{:?}", fake.calls());
    let mut unasked = crate::ui::MockPrompter::new();
    s.trusting(&flacs(), &["sync", "--force"], Some(&mut unasked))
        .unwrap();
    assert!(unasked.asked.is_empty(), "trusted once");
}

#[test]
fn undo_putting_back_a_hook_taken_out_asks_again() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let songs = s.dir.path().join("home/songs.toml");
    std::fs::write(&songs, HOOKED).unwrap();
    s.trusting(&flacs(), &["--trust", "sync"], None).unwrap();
    let listed = std::fs::read_to_string(&songs).unwrap();
    let hook = "[[hook]]\non = \"changed\"\nrun = [\"mpc\", \"update\"]\n";
    assert!(listed.contains(hook), "{listed}");
    std::fs::write(&songs, listed.replace(hook, "")).unwrap();
    s.trusting(
        &flacs(),
        &["set", "-y", "manual:a.flac", "genre=Folk"],
        None,
    )
    .unwrap();
    let e = s.trusting(&flacs(), &["undo", "-y"], None).unwrap_err();
    assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
    assert!(!std::fs::read_to_string(&songs).unwrap().contains(hook));
    s.trusting(&flacs(), &["--trust", "undo", "-y"], None)
        .unwrap();
    assert!(std::fs::read_to_string(&songs).unwrap().contains(hook));
}

#[test]
fn an_export_asks_no_trust_to_copy_and_asks_it_to_encode() {
    let (s, fake) = exported_home("200.0");
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    let url = "url = \"https://api.acoustid.org\"\n";
    let own = format!("{url}key = \"k0000000\"\n");
    std::fs::write(&songs, listed.replace(url, &own)).unwrap();
    s.trusting(&fake, &["--trust", "sync"], None).unwrap();
    State::keep_trusted(&s.dir.path().join("home"), &BTreeMap::new()).unwrap();
    let zip = s.dir.path().join("out.zip").to_string_lossy().into_owned();
    s.trusting(&fake, &["export", "-o", &zip], None).unwrap();
    let encoded = ["export", "-o", &zip, "--codec", "mp3"];
    let e = s.trusting(&fake, &encoded, None).unwrap_err();
    assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
}

#[test]
fn until_done_makes_every_lookup_due_in_one_run() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n[providers.lrclib]\nper_run = 1\n",
    )
    .unwrap();
    let server = Server::default();
    let (ok, text) = s.run_against(&server, &["sync", "--until-done"]);
    assert!(ok, "{text}");
    assert!(!text.contains("wait for the next run"), "{text}");
    let asked = server
        .asked
        .lock()
        .unwrap()
        .iter()
        .filter(|u| u.contains("/api/search"))
        .count();
    assert_eq!(asked, 2, "both songs, past per_run");
}

#[test]
fn a_backlog_of_lookups_drains_over_runs() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n[providers.lrclib]\nper_run = 1\n",
    )
    .unwrap();
    let first = Server::default();
    let (_, text) = s.run_against(&first, &["sync"]);
    assert!(
        text.contains("1 more on lrclib wait for the next run"),
        "{text}"
    );
    let asked = |server: &Server| {
        server
            .asked
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.contains("/api/search"))
            .count()
    };
    assert_eq!(asked(&first), 1);
    let second = Server::default();
    s.run_against(&second, &["sync"]);
    assert_eq!(asked(&second), 1, "the other song's turn");
    let third = Server::default();
    s.run_against(&third, &["sync"]);
    assert_eq!(asked(&third), 0, "both asked lately");
}

/// The entries of the zip at `path`, by name, with their bytes.
fn unzip(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    (0..zip.len())
        .map(|n| {
            let mut entry = zip.by_index(n).unwrap();
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
            (entry.name().unwrap().into_owned(), bytes)
        })
        .collect()
}

/// A home with one FLAC song, written; `seconds` long.
fn exported_home(seconds: &str) -> (Setup, Fake) {
    let s = Setup::new();
    let song = s.file("rips/Song.flac");
    s.file("rips/Song.lrc");
    let fake = Fake::default().probe(".flac", &FLAC.replace("200.0", seconds));
    let (ok, text) = s.run(&fake, &["add", &song]);
    assert!(ok, "{text}");
    (s, fake)
}

#[test]
fn an_export_writes_over_no_file_of_the_home_and_another_s_only_when_forced() {
    let (s, fake) = exported_home("200.0");
    let stranger = PathBuf::from(s.written("theirs.flac", "their own"));
    let home = s.dir.path().join("home");
    let targets = [
        home.join("songs.toml"),
        home.join("state.db"),
        home.join("sources/manual/Song.flac"),
        s.dir.path().join("lib/Artist/Record/02 Song.flac"),
        stranger.clone(),
    ];
    for target in &targets {
        let before = std::fs::read(target).unwrap();
        let e = s
            .trusting(&fake, &["export", "-o", &target.to_string_lossy()], None)
            .unwrap_err();
        assert!(
            e.downcast_ref::<change::Refused>().is_some(),
            "{}: {e:#}",
            target.display()
        );
        assert_eq!(
            std::fs::read(target).unwrap(),
            before,
            "{}",
            target.display()
        );
    }
    let (ok, text) = s.run(
        &fake,
        &["export", "--force", "-o", &stranger.to_string_lossy()],
    );
    assert!(ok, "{text}");
    assert!(std::fs::read(&stranger).unwrap().starts_with(b"PK"));
    let songs = home.join("songs.toml");
    let e = s
        .trusting(
            &fake,
            &["export", "--force", "-o", &songs.to_string_lossy()],
            None,
        )
        .unwrap_err();
    assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
}

#[test]
fn an_export_into_a_folder_not_there_is_refused_before_it_is_made() {
    let (s, fake) = exported_home("0.05");
    let nowhere = s.dir.path().join("nowhere");
    let named = format!("{}{}", nowhere.display(), std::path::MAIN_SEPARATOR);
    let e = s
        .trusting(&fake, &["export", "-o", &named], None)
        .unwrap_err();
    assert!(format!("{e:#}").contains("no folder"), "{e:#}");
    assert!(!nowhere.exists());
    let scratch: Vec<_> = std::fs::read_dir(s.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".muman-scratch-"))
        .collect();
    assert!(scratch.is_empty(), "{scratch:?}");
}

#[cfg(unix)]
#[test]
fn an_export_fills_the_empty_folder_a_link_names_and_keeps_the_link() {
    let (s, fake) = exported_home("0.05");
    for (n, slash) in ["", "/"].into_iter().enumerate() {
        let card = s.dir.path().join(format!("card{n}"));
        std::fs::create_dir(&card).unwrap();
        let link = s.dir.path().join(format!("link{n}"));
        std::os::unix::fs::symlink(&card, &link).unwrap();
        let named = format!("{}{slash}", link.display());
        let (ok, text) = s.run(&fake, &["export", "--folder", "-o", &named]);
        assert!(ok, "{named}: {text}");
        let kept = std::fs::symlink_metadata(&link).unwrap();
        assert!(kept.file_type().is_symlink(), "{named}");
        assert!(card.join("songs.toml").is_file(), "{named}: {text}");
        assert!(card.join("library").is_dir(), "{named}: {text}");
        let left: Vec<_> = std::fs::read_dir(&card)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 2, "{named}: {left:?}");
    }
}

#[test]
fn an_export_short_of_a_song_fails() {
    let (s, fake) = exported_home("200.0");
    std::fs::remove_file(s.dir.path().join("lib/Artist/Record/02 Song.flac")).unwrap();
    let zip = s.dir.path().join("out.zip");
    let mut out = Vec::new();
    let ok = run_with(
        &s.job(&["export", "-o", &zip.to_string_lossy()]),
        &fake,
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!ok && text.contains("left out"), "{text}");
    assert!(zip.exists(), "{text}");
}

#[test]
fn offline_a_url_is_refused_before_any_file_is_copied() {
    let s = Setup::new();
    let song = s.file("rips/Song.flac");
    let mut job = s.job(&["add", &song, "https://www.youtube.com/watch?v=aaaaaaaaaaa"]);
    job.offline = true;
    let e = run_with(
        &job,
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
    assert!(!s.dir.path().join("home/sources/manual/Song.flac").exists());
}

#[test]
fn export_writes_the_song_list_and_the_library_into_a_zip() {
    let (s, fake) = exported_home("200.0");
    let zip = s.dir.path().join("out");
    let typo = zip.join("nowhere").join("x.zip");
    let e = run_with(
        &s.job(&["export", "-o", &typo.to_string_lossy()]),
        &fake,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e}").contains("is no folder"), "{e:#}");
    assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
    assert!(!zip.exists(), "no folder made for a typo");
    std::fs::create_dir_all(&zip).unwrap();
    let (ok, text) = s.run(&fake, &["export", "-o", &zip.to_string_lossy()]);
    assert!(ok, "{text}");
    let entries = unzip(&zip.join("muman.zip"));
    assert_eq!(
        entries.keys().collect::<Vec<_>>(),
        [
            "library/Artist/Record/02 Song.flac",
            "library/Artist/Record/02 Song.lrc",
            "songs.toml"
        ]
    );
    let lib = s.dir.path().join("lib/Artist/Record");
    assert_eq!(
        entries["library/Artist/Record/02 Song.flac"],
        std::fs::read(lib.join("02 Song.flac")).unwrap()
    );
    assert_eq!(
        entries["songs.toml"],
        std::fs::read(s.dir.path().join("home/songs.toml")).unwrap()
    );
}

#[test]
fn export_redacts_the_song_list_when_asked_and_leaves_it_as_it_is() {
    let (s, fake) = exported_home("200.0");
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    let url = "url = \"https://api.acoustid.org\"\n";
    let own = format!("{url}key = \"k0000000\"\n");
    std::fs::write(&songs, listed.replace(url, &own)).unwrap();
    assert!(s.run(&fake, &["--trust", "sync"]).0);
    let zip = s.dir.path().join("out.zip").to_string_lossy().into_owned();
    let (ok, text) = s.run(&fake, &["export", "--redact", "-o", &zip]);
    assert!(ok && text.contains("1 secret(s) redacted"), "{text}");
    let exported = String::from_utf8(unzip(Path::new(&zip))["songs.toml"].clone()).unwrap();
    let kept = std::fs::read_to_string(&songs).unwrap();
    assert_eq!(exported, kept.replace("k0000000", "<redacted>"));
}

#[test]
fn export_encodes_songs_lower_to_fit_and_leaves_the_library_alone() {
    let (s, fake) = exported_home("0.05");
    let whole = s.dir.path().join("whole.zip");
    s.run(&fake, &["export", "-o", &whole.to_string_lossy()]);
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    let max = std::fs::metadata(&whole).unwrap().len() - 2000;
    let fitted = s.dir.path().join("fitted.zip");
    let (ok, text) = s.run(
        &fake,
        &[
            "export",
            "-o",
            &fitted.to_string_lossy(),
            "--max-size",
            &max.to_string(),
        ],
    );
    assert!(ok, "{text}");
    assert!(fake.ran("libopus"), "{text}");
    assert!(std::fs::metadata(&fitted).unwrap().len() <= max);
    let entries = unzip(&fitted);
    assert!(
        entries.contains_key("library/Artist/Record/02 Song.opus"),
        "{:?}",
        entries.keys()
    );
    assert!(entries.contains_key("library/Artist/Record/02 Song.lrc"));
    assert!(s.dir.path().join("lib/Artist/Record/02 Song.flac").exists());
    assert_eq!(
        crate::state::State::load(&s.dir.path().join("home")).unwrap(),
        state
    );
}

#[test]
fn export_into_too_little_room_says_what_it_needs() {
    let (s, fake) = exported_home("0.05");
    let mut out = Vec::new();
    let zip = s.dir.path().join("small.zip");
    let e = run_with(
        &s.job(&["export", "-o", &zip.to_string_lossy(), "--max-size", "1KiB"]),
        &fake,
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("cannot hold"), "{e:#}");
    assert!(!zip.exists());
}

#[test]
fn export_writes_songs_in_the_codec_and_shape_given_and_leaves_the_library_alone() {
    let (s, fake) = exported_home("0.05");
    let state = crate::state::State::load(&s.dir.path().join("home")).unwrap();
    let zip = s.dir.path().join("out.zip");
    let (ok, text) = s.run(
        &fake,
        &[
            "export",
            "-o",
            &zip.to_string_lossy(),
            "--codec",
            "opus",
            "--template",
            "{{ album }}/{{ title }}",
        ],
    );
    assert!(ok, "{text}");
    assert!(fake.ran("libopus"), "{text}");
    assert_eq!(
        unzip(&zip).keys().collect::<Vec<_>>(),
        [
            "library/Record/Song.lrc",
            "library/Record/Song.opus",
            "songs.toml"
        ]
    );
    assert!(s.dir.path().join("lib/Artist/Record/02 Song.flac").exists());
    assert_eq!(
        crate::state::State::load(&s.dir.path().join("home")).unwrap(),
        state
    );
}

#[test]
fn export_copies_each_song_whose_plan_the_settings_given_leave_alone() {
    let (s, _) = exported_home("200.0");
    let fake = Fake::default().probe(".flac", FLAC);
    let zip = s.dir.path().join("out.zip");
    let (ok, text) = s.run(
        &fake,
        &[
            "export",
            "-o",
            &zip.to_string_lossy(),
            "--template",
            "{{ title }}",
        ],
    );
    assert!(ok, "{text}");
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
    let entries = unzip(&zip);
    assert_eq!(
        entries["library/Song.flac"],
        std::fs::read(s.dir.path().join("lib/Artist/Record/02 Song.flac")).unwrap()
    );
    assert!(entries.contains_key("library/Song.lrc"));
}

#[test]
fn export_writes_the_covers_and_cleans_as_set() {
    let (s, _) = exported_home("200.0");
    let zip = s.dir.path().join("out.zip");
    let fake = Fake::default().probe(".flac", FLAC);
    let folder = "cover.out=[{ to = \"file\", scope = \"folder\", name = \"cover\" }]";
    let (ok, text) = s.run(
        &fake,
        &["export", "-o", &zip.to_string_lossy(), "--set", folder],
    );
    assert!(ok, "{text}");
    let entries = unzip(&zip);
    assert_eq!(
        entries["library/Artist/Record/cover.jpg"],
        crate::testing::PIXEL_JPEG,
        "{text}"
    );
    let song = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    assert_ne!(
        entries["library/Artist/Record/02 Song.flac"],
        std::fs::read(&song).unwrap(),
        "no cover inside: {text}"
    );

    let zip = s.dir.path().join("cleaned.zip");
    let fake = Fake::default().probe(".flac", FLAC);
    let (ok, text) = s.run(
        &fake,
        &[
            "export",
            "-o",
            &zip.to_string_lossy(),
            "--set",
            "lyrics.clean.tidy.markup=false",
        ],
    );
    assert!(ok, "{text}");
    assert!(!fake.calls().is_empty(), "made anew, not copied: {text}");
}

#[test]
fn export_embeds_lyrics_when_asked() {
    let (s, fake) = exported_home("0.05");
    let zip = s.dir.path().join("out.zip");
    let (ok, text) = s.run(
        &fake,
        &[
            "export",
            "-o",
            &zip.to_string_lossy(),
            "--lyrics",
            "embedded",
        ],
    );
    assert!(ok, "{text}");
    assert_eq!(
        unzip(&zip).keys().collect::<Vec<_>>(),
        ["library/Artist/Record/02 Song.flac", "songs.toml"]
    );
}

#[test]
fn export_keeps_a_song_changed_by_hand_as_the_library_holds_it() {
    let (s, fake) = exported_home("0.05");
    let file = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    let mut edited = std::fs::read(&file).unwrap();
    edited.extend(b"edited");
    std::fs::write(&file, &edited).unwrap();
    let zip = s.dir.path().join("out.zip");
    let (ok, text) = s.run(
        &fake,
        &["export", "-o", &zip.to_string_lossy(), "--codec", "opus"],
    );
    assert!(
        !ok && text.contains("changed since muman wrote them"),
        "{text}"
    );
    assert_eq!(unzip(&zip)["library/Artist/Record/02 Song.flac"], edited);
}

#[test]
fn export_keeps_a_song_whose_sources_are_gone_as_the_library_holds_it() {
    let (s, fake) = exported_home("0.05");
    std::fs::remove_dir_all(s.dir.path().join("home/sources/manual")).unwrap();
    let zip = s.dir.path().join("out.zip");
    let (ok, text) = s.run(
        &fake,
        &["export", "-o", &zip.to_string_lossy(), "--codec", "opus"],
    );
    assert!(!ok && text.contains("whose sources are gone"), "{text}");
    assert!(unzip(&zip).contains_key("library/Artist/Record/02 Song.flac"));
}

#[test]
fn export_fits_songs_made_anew_into_its_size() {
    let (s, fake) = exported_home("0.05");
    let zip = s.dir.path().join("out.zip");
    let (ok, text) = s.run(
        &fake,
        &[
            "export",
            "-o",
            &zip.to_string_lossy(),
            "--codec",
            "opus",
            "--max-size",
            "1GiB",
        ],
    );
    assert!(ok, "{text}");
    assert!(unzip(&zip).contains_key("library/Artist/Record/02 Song.opus"));
}

/// A home holding one 5.1 FLAC song, `seconds` long, synced once each
/// `(line, replacement)` of `settings` is made in its song list, and a
/// fake whose stereo mix of it is far quieter than its six channels.
fn surround_home(seconds: &str, settings: &[(&str, &str)]) -> (Setup, Fake) {
    let s = Setup::new();
    let tone = |amplitude| {
        crate::testing::wav(
            8000,
            2,
            Some(0x3),
            &crate::testing::sine(8000, 2, 1000.0, amplitude, 2.0),
        )
    };
    let surround = FLAC.replace("200.0", seconds).replace(
        r#""channels": 2, "sample_rate": "44100""#,
        r#""channels": 6, "channel_layout": "5.1", "sample_rate": "48000""#,
    );
    let fake = Fake {
        wavs: vec![("Song.flac".into(), tone(0.4))],
        mixed: vec![("Song.flac".into(), tone(0.05))],
        ..Fake::default()
    }
    .probe(".flac", &surround);
    let song = s.file("rips/Song.flac");
    let (ok, text) = s.run(&fake, &["add", &song]);
    assert!(ok, "{text}");
    if !settings.is_empty() {
        let list = s.dir.path().join("home/songs.toml");
        let mut text = std::fs::read_to_string(&list).unwrap();
        for (line, replacement) in settings {
            assert!(text.contains(line), "{line}");
            text = text.replacen(line, replacement, 1);
        }
        s.written("home/songs.toml", &text);
        let (ok, text) = s.run(&fake, &["sync"]);
        assert!(ok, "{text}");
    }
    (s, fake)
}

/// The gain the last MP3 `fake` encoded was given in its samples.
fn mp3_volume(fake: &Fake) -> Option<String> {
    let calls = fake.calls();
    let encode = calls
        .iter()
        .rfind(|c| c.iter().any(|a| a == "libmp3lame"))?;
    encode.iter().find_map(|a| {
        let at = a.find("volume=")?;
        a[at..].split([',', ' ', ';']).next().map(str::to_owned)
    })
}

/// The mixes into stereo `fake` was asked to measure.
fn stereo_mixes(fake: &Fake) -> usize {
    (fake.calls().iter())
        .filter(|c| {
            c.iter().any(|a| a == crate::ffmpeg::PIPE)
                && c.iter().any(|a| a.starts_with("aresample=ochl=stereo"))
        })
        .count()
}

/// The settings a sync writes the song of [`surround_home`] in MP3 under,
/// levelled in its samples.
const IN_MP3: [(&str, &str); 4] = [
    (
        r#"codecs = ["opus", "vorbis", "aac", "mp3", "flac"]"#,
        r#"codecs = ["mp3"]"#,
    ),
    (r#"lossy = "opus""#, r#"lossy = "mp3""#),
    (r#"lossless = "flac""#, r#"lossless = "mp3""#),
    (r#"mode = "tags""#, r#"mode = "audio""#),
];

#[test]
fn an_export_mixing_a_song_levels_it_by_its_mix_as_a_sync_would() {
    let (_synced, by_sync) = surround_home("200.0", &IN_MP3);
    let written = mp3_volume(&by_sync);
    assert!(written.is_some(), "{:?}", by_sync.calls());

    let (s, fake) = surround_home("200.0", &[]);
    let before = stereo_mixes(&fake);
    let zip = s.dir.path().join("out.zip");
    let (ok, text) = s.run(
        &fake,
        &[
            "export",
            "-o",
            &zip.to_string_lossy(),
            "--codec",
            "mp3",
            "--set",
            "loudness.mode=audio",
        ],
    );
    assert!(ok, "{text}");
    assert!(stereo_mixes(&fake) > before, "the mix is measured: {text}");
    assert_eq!(mp3_volume(&fake), written, "{text}");
}

#[test]
fn an_export_mixing_a_song_to_fit_levels_it_by_its_mix_as_a_sync_would() {
    let (_synced, by_sync) = surround_home("0.05", &IN_MP3);
    let written = mp3_volume(&by_sync);
    assert!(written.is_some(), "{:?}", by_sync.calls());

    let (s, fake) = surround_home("0.05", &[]);
    let whole = s.dir.path().join("whole.zip");
    let (ok, text) = s.run(&fake, &["export", "-o", &whole.to_string_lossy()]);
    assert!(ok, "{text}");
    let max = std::fs::metadata(&whole).unwrap().len() - 2000;
    let fitted = s.dir.path().join("fitted.zip");
    let (ok, text) = s.run(
        &fake,
        &[
            "export",
            "-o",
            &fitted.to_string_lossy(),
            "--max-size",
            &max.to_string(),
            "--set",
            "audio.lossy=\"mp3\"",
            "--set",
            "loudness.mode=audio",
        ],
    );
    assert!(ok, "{text}");
    assert!(text.contains("at a lower bitrate to fit"), "{text}");
    assert_eq!(mp3_volume(&fake), written, "{text}");
}

/// Every file under `dir`, by its path there with `/`, with its bytes.
fn files_under(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                let rel = path.strip_prefix(dir).unwrap();
                files.insert(
                    crate::relpath::to_portable(rel),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    files
}

#[test]
fn export_writes_into_a_folder_what_the_zip_holds() {
    let (s, fake) = exported_home("200.0");
    let zip = s.dir.path().join("out.zip");
    let (ok, text) = s.run(&fake, &["export", "-o", &zip.to_string_lossy()]);
    assert!(ok, "{text}");
    let folder = s.dir.path().join("out");
    let (ok, text) = s.run(
        &fake,
        &["export", "--folder", "-o", &folder.to_string_lossy()],
    );
    assert!(ok, "{text}");
    assert_eq!(files_under(&folder), unzip(&zip));
    let time = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
    assert_eq!(
        time(&folder.join("library/Artist/Record/02 Song.flac")),
        time(&s.dir.path().join("lib/Artist/Record/02 Song.flac"))
    );
    let beside: Vec<_> = std::fs::read_dir(s.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".muman-scratch-"))
        .collect();
    assert!(beside.is_empty(), "{beside:?}");
}

#[test]
fn export_fills_an_empty_folder_and_refuses_one_holding_anything() {
    let (s, fake) = exported_home("0.05");
    let folder = s.dir.path().join("card");
    std::fs::create_dir_all(&folder).unwrap();
    let args = ["export", "--folder", "--codec", "opus", "-o"];
    let (ok, text) = s.run(&fake, &[&args[..], &[&*folder.to_string_lossy()]].concat());
    assert!(ok, "{text}");
    assert!(folder.join("library/Artist/Record/02 Song.opus").is_file());
    let before = files_under(&folder);
    let e = run_with(
        &s.job(&[&args[..], &[&*folder.to_string_lossy()]].concat()),
        &fake,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("is not empty"), "{e:#}");
    assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
    assert_eq!(files_under(&folder), before);
}

#[test]
fn a_folder_s_files_count_in_whole_blocks_and_a_zip_s_in_none() {
    let (s, fake) = exported_home("0.05");
    let folder = s.dir.path().join("card");
    let block = ["--set", "library.block_size=1MiB"];
    let e = run_with(
        &s.job(
            &[
                &["export", "--folder", "--max-size", "2MiB", "-o"],
                &[&*folder.to_string_lossy()][..],
                &block[..],
            ]
            .concat(),
        ),
        &fake,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("cannot hold"), "{e:#}");
    assert!(!folder.exists());
    let zip = s.dir.path().join("out.zip");
    let e = run_with(
        &s.job(&[&["export", "-o", &*zip.to_string_lossy()][..], &block[..]].concat()),
        &fake,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(e.downcast_ref::<crate::cli::Invalid>().is_some(), "{e:#}");
    let (ok, text) = s.run(
        &fake,
        &["export", "--max-size", "2MiB", "-o", &zip.to_string_lossy()],
    );
    assert!(ok, "{text}");
}

#[test]
fn an_export_given_a_setting_that_does_not_read_writes_nothing() {
    let (s, fake) = exported_home("0.05");
    let zip = s.dir.path().join("out.zip");
    let zip = zip.to_string_lossy();
    for given in [["--set", "audio.lossy=flac"], ["--template", "{{ nope("]] {
        let e = run_with(
            &s.job(&["export", "-o", &zip, given[0], given[1]]),
            &fake,
            &Server::default(),
            None,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(e.downcast_ref::<crate::cli::Invalid>().is_some(), "{e:#}");
    }
    assert!(!s.dir.path().join("out.zip").exists());
}

#[test]
fn an_export_that_levels_songs_the_song_list_does_not_measure_is_refused() {
    let (s, fake) = exported_home("0.05");
    let songs = s.dir.path().join("home/songs.toml");
    let text = std::fs::read_to_string(&songs).unwrap();
    std::fs::write(&songs, text.replace("mode = \"tags\"", "mode = \"off\"")).unwrap();
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    let zip = s.dir.path().join("out.zip");
    let said = s.refused(&[
        "export",
        "-o",
        zip.to_str().unwrap(),
        "--set",
        "loudness.mode=tags",
    ]);
    assert!(said.contains("measures none"), "{said}");
    assert!(!zip.exists());
}

#[test]
fn stale_partial_downloads_go_from_every_folder_and_fresh_ones_stay() {
    let dir = tempfile::tempdir().unwrap();
    let partial = dir.path().join("partial");
    let stale = partial.join("Chan/Song [vid00000001].f251.webm.part");
    let fresh = partial.join("Other/Song [vid00000002].f251.webm.part");
    for file in [&stale, &fresh] {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "half").unwrap();
    }
    std::fs::create_dir_all(partial.join("Empty")).unwrap();
    let month_ago = SystemTime::now() - std::time::Duration::from_secs(30 * 24 * 3600);
    std::fs::File::options()
        .write(true)
        .open(&stale)
        .unwrap()
        .set_modified(month_ago)
        .unwrap();
    sync::clear_stale(
        &partial,
        std::time::Duration::from_secs(14 * 24 * 3600),
        None,
    );
    assert!(!stale.exists());
    assert!(
        !partial.join("Chan").exists(),
        "the folder it leaves empty goes"
    );
    assert!(!partial.join("Empty").exists());
    assert!(fresh.exists(), "a later run resumes it");
}

#[test]
fn a_stale_clear_leaves_the_folder_it_keeps_apart_even_empty() {
    let dir = tempfile::tempdir().unwrap();
    let partial = dir.path().join("partial");
    let apart = partial.join(crate::source::TORRENT);
    std::fs::create_dir_all(&apart).unwrap();
    std::fs::create_dir_all(partial.join("Empty")).unwrap();
    sync::clear_stale(
        &partial,
        std::time::Duration::from_secs(14 * 24 * 3600),
        Some(&apart),
    );
    assert!(
        apart.is_dir(),
        "a torrent fetched beside it makes its folder in there"
    );
    assert!(!partial.join("Empty").exists());
}

#[test]
fn duplicates_are_the_songs_listed_apart_that_are_one_recording() {
    let s = Setup::new();
    for name in ["a", "b", "c"] {
        s.file(&format!("home/sources/manual/{name}.flac"));
    }
    let song = words(1600, 1);
    let prints = || {
        flacs()
            .print("a.flac", song.clone())
            .print("b.flac", song.clone())
            .print("c.flac", words(1600, 2))
    };
    let (ok, text) = s.run(&prints(), &["sync", "--new"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 3);
    let (ok, report) = s.report(&prints(), &["duplicates"]);
    assert!(ok);
    assert!(
        report.contains("2 songs, one recording, on one album"),
        "{report}"
    );
    assert!(
        report.contains("manual:a.flac") && report.contains("manual:b.flac"),
        "{report}"
    );
    assert!(!report.contains("manual:c.flac"), "{report}");
    let (_, report) = s.report(&prints(), &["duplicates", "c.flac"]);
    assert!(
        report.contains("No group of one recording holds a song the query matches"),
        "{report}"
    );
}

#[test]
fn duplicates_resolved_become_one_song_and_undo_puts_them_back() {
    let s = Setup::new();
    for name in ["a", "b", "c"] {
        s.file(&format!("home/sources/manual/{name}.flac"));
    }
    let song = words(1600, 1);
    let prints = || {
        flacs()
            .print("a.flac", song.clone())
            .print("b.flac", song.clone())
            .print("c.flac", words(1600, 2))
    };
    let (ok, text) = s.run(&prints(), &["sync", "--new"]);
    assert!(ok, "{text}");
    s.refused(&["duplicates", "--resolve"]);

    let mut p = crate::ui::MockPrompter::answering([true]);
    p.push_select(Some(0));
    let text = s
        .trusting(&prints(), &["duplicates", "--resolve"], Some(&mut p))
        .unwrap();
    assert!(p.asked[0].starts_with("Group 1:"), "{:?}", p.asked);
    assert!(
        p.asked[0].contains("(suggested: an album holds it twice)"),
        "{:?}",
        p.asked
    );
    assert_eq!(
        p.asked.last().map(String::as_str),
        Some("Merge 1 group?"),
        "{text}"
    );
    let text = crate::ui::plain(&text);
    assert!(text.contains("Group 1 of 1"), "{text}");
    assert!(text.contains("% alike"), "{text}");
    let songs = s.songs();
    assert_eq!(songs.len(), 2, "{text}");
    assert!(
        songs.iter().any(|keys| keys.len() == 2),
        "the copies are one song: {songs:?}"
    );

    let (ok, text) = s.run(&prints(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 3, "{text}");
}

#[test]
fn merge_twice_merges_the_copies_an_album_holds_and_leaves_tags_set_apart() {
    let s = Setup::new();
    for name in ["a", "b", "c", "d"] {
        s.file(&format!("home/sources/manual/{name}.flac"));
    }
    let (one, two) = (words(1600, 1), words(1600, 2));
    let prints = || {
        flacs()
            .print("a.flac", one.clone())
            .print("b.flac", one.clone())
            .print("c.flac", two.clone())
            .print("d.flac", two.clone())
    };
    let (ok, text) = s.run(&prints(), &["sync", "--new"]);
    assert!(ok, "{text}");
    for (key, genre) in [
        ("manual:c.flac", "genre=Folk"),
        ("manual:d.flac", "genre=Ambient"),
    ] {
        let (ok, text) = s.run(&prints(), &["set", "-y", key, genre]);
        assert!(ok, "{text}");
    }
    s.refused(&["duplicates", "--merge", "twice"]);
    let (ok, text) = s.run(&prints(), &["duplicates", "--merge", "twice", "-n"]);
    assert!(ok, "{text}");
    assert!(text.contains("Dry run: nothing changed"), "{text}");
    assert_eq!(s.songs().len(), 4, "a dry run changes nothing");

    let (ok, text) = s.run(&prints(), &["duplicates", "--merge", "twice", "-y"]);
    assert!(ok, "{text}");
    assert!(text.contains("GENRE set by hand to two values"), "{text}");
    let songs = s.songs();
    assert_eq!(songs.len(), 3, "{text}");
    assert!(
        songs.contains(&vec![
            SourceKey::Manual("a.flac".into()),
            SourceKey::Manual("b.flac".into())
        ]),
        "{songs:?}"
    );
}

#[test]
fn a_copy_of_a_track_too_short_to_agree_on_six_seconds_is_one_recording() {
    let s = Setup::new();
    let track = s.file("rips/Interlude.flac");
    let copy = s.file("rips/Interlude (copy).flac");
    // An interlude of 8 s: 40 words.
    let print = words(40, 3);
    let fake = || flacs().print("Interlude", print.clone());
    let (ok, text) = s.run(&fake(), &["add", &track]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&fake(), &["add", &copy]);
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 1, "{text}");
    for name in ["a", "b"] {
        s.file(&format!("home/sources/manual/{name}.flac"));
    }
    let prints = || {
        fake()
            .print("a.flac", print.clone())
            .print("b.flac", print.clone())
    };
    let (ok, text) = s.run(&prints(), &["sync", "--new"]);
    assert!(ok, "{text}");
    let (_, report) = s.report(&prints(), &["duplicates"]);
    assert!(report.contains("3 songs, one recording"), "{report}");
}

/// 8 kHz samples, a square wave at each of `levels` for 10 ms.
fn pcm(levels: &[u32]) -> Vec<u8> {
    levels
        .iter()
        .flat_map(|level| {
            let level = i16::try_from(level % 10_000 + 2000).unwrap();
            [level, -level].repeat(40)
        })
        .flat_map(i16::to_le_bytes)
        .collect()
}

#[test]
fn an_excerpt_added_to_its_song_leaves_the_whole_song_chosen() {
    let s = Setup::new();
    let whole = s.file("rips/Ember Road.flac");
    let edit = s.file("rips/Ember Road (Edit).flac");
    let print = words(320, 1);
    let levels = words(4000, 2);
    let fake = || {
        let mut fake = flacs()
            .print("Road.flac", print.clone())
            .print("Edit).flac", print[..160].to_vec());
        fake.pcm = vec![
            ("Road.flac".into(), pcm(&levels)),
            ("Edit).flac".into(), pcm(&levels[..2000])),
        ];
        fake
    };
    let (ok, text) = s.run(&fake(), &["add", &whole]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&fake(), &["add", "-y", &edit]);
    assert!(ok, "{text}");
    assert_eq!(s.songs().len(), 1, "{text}");
    let (_, report) = s.report(&fake(), &["status", "--all"]);
    assert!(
        report.contains("audio   manual:Ember Road.flac: 0.0 s of sound beyond the song"),
        "{report}"
    );
}

#[test]
fn an_album_with_a_soft_cover_finds_one_on_the_cover_art_archive_once() {
    const GROUP: &str = "00000000-0000-4000-8000-0000000000aa";
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let search = format!(
        r#"{{"release-groups": [{{"id": "{GROUP}", "title": "Record", "primary-type": "Album",
            "score": 100, "artist-credit": [{{"name": "Artist"}}]}}]}}"#
    );
    let server = Server::default()
        .answer("/ws/2/release-group?query=", &search)
        .answer_bytes(
            &format!("/release-group/{GROUP}/front-1200"),
            b"\xFF\xD8\xFF\xE0",
        );
    let (ok, text) = s.run_against(&server, &["sync", "--new"]);
    assert!(ok, "{text}");
    let cover = SourceKey::parse(&format!("coverart:{GROUP}")).unwrap();
    let songs = s.songs();
    assert!(songs.iter().all(|k| k.contains(&cover)), "{songs:?}");
    assert!(
        s.dir
            .path()
            .join(format!("home/sources/coverart/{GROUP}.jpg"))
            .exists()
    );
    let asked = server.asked.lock().unwrap();
    let count = |part: &str| asked.iter().filter(|u| u.contains(part)).count();
    assert_eq!(
        count("/ws/2/release-group"),
        1,
        "one search an album: {asked:?}"
    );
    assert_eq!(count("/front-1200"), 1, "{asked:?}");
}

#[test]
fn a_song_list_gone_missing_removes_nothing() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib/Artist/Record");
    std::fs::remove_file(s.dir.path().join("home/songs.toml")).unwrap();
    assert!(s.refused(&["sync"]).contains("songs.toml is missing"));
    let song = s.file("rips/c.flac");
    assert!(s.refused(&["add", &song]).contains("2 song(s)"));
    assert!(!s.dir.path().join("home/sources/manual/c.flac").exists());
    assert_eq!(std::fs::read_dir(&lib).unwrap().count(), 2);
    assert!(!s.dir.path().join("home/songs.toml").exists());
}

#[test]
fn a_dry_run_or_a_refused_change_keeps_no_run_to_undo() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let runs = || {
        std::fs::read_dir(s.dir.path().join("home/history"))
            .unwrap()
            .count()
    };
    let before = runs();
    let (ok, text) = s.run(&flacs(), &["remove", "-n", "manual:a.flac"]);
    assert!(ok, "{text}");
    s.refused(&["set", "-y", "nothing-matches", "genre=Folk"]);
    assert_eq!(runs(), before);
}

#[test]
fn has_holds_for_tags_a_source_gives_and_not_for_loudness_muman_measured() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let has = s.listed(&["list", "-f", "{has}"]);
    assert!(has.contains("title"), "{has}");
    assert!(!has.contains("replaygain"), "{has}");
    assert!(s.listed(&["list", "has:replaygain_track_gain"]).is_empty());
}

#[test]
fn has_holds_for_a_gain_a_source_gives_though_muman_writes_its_own() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let gained = crate::testing::FLAC.replace(
        r#""track": "2"}"#,
        r#""track": "2", "REPLAYGAIN_TRACK_GAIN": "-3.21 dB"}"#,
    );
    let fake = Fake::default().probe(".flac", &gained);
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    let mut data = Vec::new();
    run_with(
        &s.job(&["list", "has:replaygain_track_gain"]),
        &fake,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut data,
    )
    .unwrap();
    assert!(!data.is_empty());
}

#[test]
fn a_command_stopped_on_a_song_list_that_does_not_read_keeps_no_run() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let history = s.dir.path().join("home/history");
    let runs = || std::fs::read_dir(&history).unwrap().count();
    let before = runs();
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    let version = listed.lines().find(|l| l.starts_with("version")).unwrap();
    let broken = listed.replacen(version, "", 1);
    std::fs::write(&songs, &broken).unwrap();
    let e = run_with(
        &s.job(&["set", "-y", "manual:a.flac", "genre=Folk"]),
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert!(e.is_err());
    assert_eq!(runs(), before);
    assert_eq!(std::fs::read_to_string(&songs).unwrap(), broken);
}

#[test]
fn a_sync_whose_library_cannot_be_locked_writes_nothing() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let blocker = s.file("not-a-folder");
    let mut job = s.job(&["sync"]);
    job.dirs.locks = Some(Path::new(&blocker).join("locks"));
    let e = run_with(
        &job,
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert!(e.is_err());
    assert!(library_of(&s).is_empty());
}

/// Answers yes, noting whether a run held the home as it was asked.
struct Asked {
    home: PathBuf,
    held: bool,
}

impl crate::ui::Prompter for Asked {
    fn ask(&mut self, _: &str) -> std::io::Result<bool> {
        self.held |= Lock::busy(&self.home);
        Ok(true)
    }

    fn choose(&mut self, _: &str, _: &[String], picked: &[usize]) -> std::io::Result<Vec<usize>> {
        Ok(picked.to_vec())
    }

    fn select(&mut self, _: &str, _: &[String]) -> std::io::Result<Option<usize>> {
        Ok(None)
    }

    fn text(&mut self, _: &str, _: &str) -> std::io::Result<Option<String>> {
        Ok(None)
    }

    fn text_with(
        &mut self,
        _: &str,
        _: &str,
        _: &str,
        _: crate::ui::Check,
    ) -> std::io::Result<Option<String>> {
        Ok(None)
    }
}

#[test]
fn a_change_asking_its_question_holds_no_other_run_back() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    for args in [
        &["set", "manual:a.flac", "genre=Folk"][..],
        &["remove", "manual:a.flac"],
        &["restore", "manual:a.flac"],
    ] {
        let mut asked = Asked {
            home: s.dir.path().join("home"),
            held: false,
        };
        let mut out = Vec::new();
        let ok = run_with(
            &s.job(args),
            &flacs(),
            &Server::default(),
            Some(&mut asked),
            &mut out,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(ok, "{args:?}: {}", String::from_utf8_lossy(&out));
        assert!(!asked.held, "{args:?} held the home as it asked");
    }
    assert!(s.listed(&["list"]).contains("manual:a.flac"));
}

#[test]
fn a_change_to_a_home_not_there_is_refused_and_makes_nothing() {
    let s = Setup::new();
    for args in [
        &["undo", "-n"][..],
        &["set", "-n", "title:x", "genre=Folk"],
        &["remove", "-y", "title:x"],
        &["purge", "-n"],
    ] {
        let e = s.refused(args);
        assert!(e.contains("nothing listed yet"), "{args:?}: {e}");
        assert!(!s.dir.path().join("home").exists(), "{args:?}");
    }
}

#[test]
fn a_source_touched_but_not_changed_is_not_measured_again() {
    let s = Setup::new();
    let song = s.file("home/sources/manual/Song.flac");
    s.written("home/sources/manual/Song.lrc", "[00:01.00]Paper comets\n");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let later = SystemTime::now() + std::time::Duration::from_secs(3600);
    filetime::set_file_mtime(&song, filetime::FileTime::from_system_time(later)).unwrap();
    let fake = flacs();
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(!text.contains("Measuring"), "{text}");
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());

    std::fs::write(&song, b"other bytes").unwrap();
    let (_, text) = s.run(&flacs(), &["sync"]);
    assert!(
        text.contains("Measuring"),
        "changed, measured again: {text}"
    );
}

#[test]
fn a_song_taken_out_by_hand_stays_out_though_its_file_was_just_touched() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let songs = s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(&songs).unwrap();
    let (head, rest) = listed.split_once("\n[[song]]\n").unwrap();
    let kept: Vec<&str> = rest
        .split("\n[[song]]\n")
        .filter(|b| !b.contains("manual:a.flac"))
        .collect();
    std::fs::write(
        &songs,
        format!("{head}\n[[song]]\n{}", kept.join("\n[[song]]\n")),
    )
    .unwrap();

    let mut job = s.job(&["sync"]);
    job.settling = std::time::Duration::from_secs(3600);
    let mut out = Vec::new();
    run_with(
        &job,
        &flacs(),
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("so kept out: manual:a.flac"), "{text}");
    assert_eq!(
        s.listed(&["list", "--removed", "--keys"]),
        "manual:a.flac\n"
    );
}

#[test]
fn a_set_that_changes_nothing_writes_nothing() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&flacs(), &["set", "-y", "manual:a.flac", "genre=Folk"]);
    assert!(ok, "{text}");
    let songs = s.dir.path().join("home/songs.toml");
    let written = std::fs::metadata(&songs).unwrap().modified().unwrap();
    let (ok, text) = s.run(&flacs(), &["set", "-y", "manual:a.flac", "GENRE=Folk"]);
    assert!(ok, "{text}");
    assert!(text.contains("Nothing changed"), "{text}");
    assert_eq!(
        std::fs::metadata(&songs).unwrap().modified().unwrap(),
        written
    );
}

#[test]
fn a_library_put_back_from_a_copy_with_new_times_is_still_muman_s() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib");
    for file in crate::store::files_below(&lib, usize::MAX, |_| true).unwrap() {
        let bytes = std::fs::read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, bytes).unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        filetime::set_file_mtime(&file, filetime::FileTime::from_system_time(later)).unwrap();
    }
    let (ok, text) = s.run(&flacs(), &["set", "-y", "manual:a.flac", "genre=Folk"]);
    assert!(ok, "{text}");
    assert!(!text.contains("changed since muman wrote it"), "{text}");
    assert!(text.contains("Updated"), "{text}");
}

#[test]
fn a_library_moved_or_copied_elsewhere_keeps_the_files_muman_wrote() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let lib = s.dir.path().join("lib");
    // Copied elsewhere with new times, as a copy without times leaves it,
    // and copied again with the song's file changed since.
    let copy = |to: &str, edit: bool| {
        let to = s.dir.path().join(to);
        for file in crate::store::files_below(&lib, usize::MAX, |_| true).unwrap() {
            let at = to.join(file.strip_prefix(&lib).unwrap());
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            let mut bytes = std::fs::read(&file).unwrap();
            if edit && at.extension().is_some_and(|e| e == "flac") {
                bytes.extend(b"retagged");
            }
            std::fs::write(&at, bytes).unwrap();
        }
        to
    };
    let run_in = |library: &Path, args: &[&str]| {
        let mut job = s.job(args);
        job.dirs.library = library.to_path_buf();
        let mut out = Vec::new();
        let ok = run_with(
            &job,
            &flacs(),
            &Server::default(),
            None,
            &mut out,
            &mut Vec::new(),
        )
        .unwrap();
        (ok, String::from_utf8(out).unwrap())
    };
    let copied = copy("elsewhere", false);
    let (ok, text) = run_in(&copied, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("1 file(s) here as muman wrote them stay its"),
        "{text}"
    );
    assert!(text.contains("Up to date: 1"), "{text}");
    assert!(!text.contains("not muman's"), "{text}");
    let (ok, text) = run_in(&copied, &["set", "-y", "manual:a.flac", "genre=Folk"]);
    assert!(ok && text.contains("Updated"), "{text}");
    assert!(!text.contains("not muman's"), "{text}");

    let edited = copy("edited", true);
    let (_, text) = run_in(&edited, &["sync"]);
    assert!(
        text.contains("0 file(s) here as muman wrote them stay its"),
        "{text}"
    );
    assert!(text.contains("Left alone, not muman's"), "{text}");
}

#[test]
fn an_export_of_a_song_list_edited_since_the_last_run_is_refused() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let songs = s.dir.path().join("home/songs.toml");
    let text = std::fs::read_to_string(&songs).unwrap();
    std::fs::write(
        &songs,
        text.replace("tags.genre = \"\"", "tags.genre = \"Folk\""),
    )
    .unwrap();
    let zip = s.dir.path().join("out.zip");
    let said = s.refused(&["export", "-o", zip.to_str().unwrap()]);
    assert!(said.contains("changed since the last run"), "{said}");
    assert!(!zip.exists());
}

#[test]
fn a_home_in_the_library_is_refused_before_add_copies_anything() {
    let s = Setup::new();
    let file = s.file("in/a.flac");
    let mut job = s.job(&["add", &file]);
    job.dirs.home = s.dir.path().join("lib/.muman");
    let e = run_with(
        &job,
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("is in the library"), "{e:#}");
    assert!(!job.dirs.home.exists());
}

#[test]
fn a_library_in_the_home_s_sources_is_refused() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let mut job = s.job(&["sync"]);
    job.dirs.library = s.dir.path().join("home/sources/manual/lib");
    let e = run_with(
        &job,
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("in the home's sources"), "{e:#}");
    assert!(!job.dirs.library.exists());
}

#[test]
fn a_library_in_a_folder_muman_keeps_in_the_home_is_refused() {
    let s = Setup::new();
    for kept in ["history", "partial"] {
        let mut job = s.job(&["sync"]);
        job.dirs.library = s.dir.path().join("home").join(kept).join("music");
        let e = run_with(
            &job,
            &flacs(),
            &Server::default(),
            None,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(
            format!("{e:#}").contains(&format!("in the home's {kept}")),
            "{e:#}"
        );
        assert!(!job.dirs.library.exists());
    }
}

#[test]
fn undo_and_purge_refuse_a_library_in_the_home_s_sources() {
    let s = Setup::new();
    for args in [["undo", "-y"], ["purge", "-y"]] {
        let mut job = s.job(&args);
        job.dirs.library = s.dir.path().join("home/sources/manual/lib");
        let e = run_with(
            &job,
            &flacs(),
            &Server::default(),
            None,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(
            format!("{e:#}").contains("in the home's sources"),
            "{args:?}: {e:#}"
        );
    }
}

#[test]
fn each_command_is_classified_as_it_runs() {
    const SYNC: &[&str] = &[
        "Fetching",
        "Measuring",
        "Looking up",
        "Comparing",
        "Writing",
    ];
    const CHANGE: &[&str] = &["Measuring", "Comparing", "Writing"];
    const RESTORE: &[&str] = &["Fetching", "Measuring", "Comparing", "Writing"];
    const MERGE: &[&str] = &["Comparing prints", "Comparing", "Measuring", "Writing"];
    const ZIP: &[&str] = &["Encoding", "Writing the zip"];
    let is = |apart, edits_list, gated, ffmpeg, steps| Traits {
        apart,
        edits_list,
        gated,
        ffmpeg,
        locks: true,
        steps,
    };
    let free = |t: Traits| Traits { locks: false, ..t };
    let table: [(&[&str], Traits); 22] = [
        (&["add", "manual:a.flac"], is(true, true, true, true, SYNC)),
        (&["sync"], is(true, true, true, true, SYNC)),
        (&["list"], free(is(false, false, false, false, &[]))),
        (&["remove", "a"], is(true, true, true, true, CHANGE)),
        (&["restore", "a"], is(true, true, true, true, RESTORE)),
        (&["set", "a", "title=b"], is(true, true, true, true, CHANGE)),
        (&["edit"], is(true, true, true, true, CHANGE)),
        (&["review"], is(true, true, true, true, CHANGE)),
        (&["undo"], is(true, false, false, true, CHANGE)),
        (&["purge"], is(true, false, false, false, &[])),
        (&["check"], is(false, false, false, false, &["Decoding"])),
        (
            &["check", "--decode"],
            is(false, false, true, true, &["Decoding"]),
        ),
        (
            &["check", "--upstream"],
            is(false, false, true, false, &["Decoding"]),
        ),
        (
            &["status"],
            is(false, false, true, true, &["Measuring", "Comparing"]),
        ),
        (&["export"], is(false, false, false, false, ZIP)),
        (
            &["export", "--set", "audio.codecs=[\"opus\"]"],
            is(false, false, true, true, ZIP),
        ),
        (
            &["export", "--codec", "opus", "--folder"],
            is(
                false,
                false,
                true,
                true,
                &["Encoding", "Writing the folder"],
            ),
        ),
        (
            &["duplicates"],
            free(is(false, false, false, false, &["Comparing prints"])),
        ),
        (
            &["duplicates", "--resolve"],
            is(true, true, true, true, MERGE),
        ),
        (
            &["duplicates", "--merge", "twice"],
            is(true, true, true, true, MERGE),
        ),
        (&["info"], is(false, false, true, false, &[])),
        (
            &["completions", "bash"],
            free(is(false, false, false, false, &[])),
        ),
    ];
    for (args, expected) in table {
        let t = traits(&cli(args).command);
        assert_eq!(t, expected, "{args:?}");
        assert!(
            !t.edits_list || t.apart,
            "{args:?}: what edits the list keeps apart"
        );
        assert!(
            !(t.gated || t.apart) || t.locks,
            "{args:?}: what runs or writes locks"
        );
    }
    let named: std::collections::BTreeSet<&str> = table.iter().map(|(a, _)| a[0]).collect();
    for sub in cli::command().get_subcommands() {
        assert!(
            named.contains(sub.get_name()),
            "{} is in the table",
            sub.get_name()
        );
    }
}

#[test]
fn a_hook_may_run_muman_on_its_home_but_not_on_its_library_elsewhere() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let go = |args: &[&str], hooked: Hooked| {
        let mut job = s.job(args);
        job.hooked = Some(hooked);
        let mut out = Vec::new();
        let done = run_with(
            &job,
            &flacs(),
            &Server::default(),
            None,
            &mut out,
            &mut Vec::new(),
        );
        (done, String::from_utf8(out).unwrap())
    };
    let own = Hooked {
        home: s.dir.path().join("home"),
        library: Some(s.dir.path().join("lib")),
    };
    for args in [&["sync"][..], &["status"], &["info"], &["list"]] {
        let (done, text) = go(args, own.clone());
        assert!(done.unwrap(), "{args:?}: {text}");
    }
    let elsewhere = Hooked {
        home: s.dir.path().join("other"),
        ..own.clone()
    };
    let e = go(&["sync"], elsewhere).0.unwrap_err();
    assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
    assert!(format!("{e:#}").contains("--library"), "{e:#}");
    let apart = Hooked {
        home: s.dir.path().join("other"),
        library: Some(s.dir.path().join("other-lib")),
    };
    let (synced, text) = go(&["sync"], apart);
    assert!(synced.unwrap(), "{text}");
}

#[test]
fn a_hooked_run_is_read_from_its_variables_alone() {
    let vars = |set: &'static [(&str, &str)]| {
        move |name: &str| {
            set.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| std::ffi::OsString::from(v))
        }
    };
    assert_eq!(hooked_from(&vars(&[("MUMAN_LIBRARY", "lib")])), None);
    assert_eq!(
        hooked_from(&vars(&[
            ("MUMAN_HOOK_HOME", "home"),
            ("MUMAN_LIBRARY", "lib")
        ])),
        Some(Hooked {
            home: "home".into(),
            library: Some("lib".into()),
        })
    );
}

#[test]
fn a_song_whose_new_path_holds_a_file_of_your_own_stays_where_it_was() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let was = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    assert!(was.exists(), "{text}");
    let mine = s.dir.path().join("lib/Artist/Other/02 Song.flac");
    std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
    std::fs::write(&mine, "my own rip").unwrap();
    let (_, text) = s.run(&flacs(), &["set", "-y", "manual:a.flac", "album=Other"]);
    assert!(
        text.contains("Left at Artist/Record/02 Song.flac"),
        "{text}"
    );
    assert!(!text.contains("Removed"), "{text}");
    assert!(was.exists(), "the song keeps its file: {text}");
    assert_eq!(std::fs::read_to_string(&mine).unwrap(), "my own rip");
}

#[test]
fn a_title_changing_only_its_case_moves_its_file() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&flacs(), &["set", "-y", "manual:a.flac", "title=SONG"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Moved: Artist/Record/02 Song.flac → Artist/Record/02 SONG.flac"),
        "{text}"
    );
    assert!(!text.contains("Added"), "{text}");
    // A filesystem that ignores case finds either spelling; the folder
    // lists the one on disk.
    let names: Vec<String> = std::fs::read_dir(s.dir.path().join("lib/Artist/Record"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["02 SONG.flac"]);
}

#[test]
fn a_path_that_is_not_there_is_told_from_an_address() {
    for path in [
        "/music/Marlo Venn/Tide.flac",
        "./Tide.flac",
        "Tide.flac",
        "C:\\Music\\Tide.flac",
    ] {
        assert!(add::names_a_path(path), "{path}");
    }
    for address in [
        "https://archive.org/details/item0001",
        "ytsearch:lantern weather",
        "youtube.com/watch?v=vid00000001",
    ] {
        assert!(!add::names_a_path(address), "{address}");
    }
}

#[test]
fn a_link_in_a_folder_added_is_followed_only_to_a_place_inside_it() {
    let s = Setup::new();
    s.file("rips/Album/Song.flac");
    s.file("rips/Album/art/front.jpg");
    s.file("rips/Other/Glass Orchards.flac");
    let secret = s.written("secret/creds", "[00:01.00]a secret");
    let album = s.dir.path().join("rips").join("Album");
    let linked = crate::testing::link(
        &album.join("art").join("front.jpg"),
        &album.join("cover.jpg"),
    ) && crate::testing::link(Path::new(&secret), &album.join("Song.lrc"))
        && crate::testing::link(
            &s.dir.path().join("rips").join("Other"),
            &album.join("extras"),
        );
    if !linked {
        return;
    }
    let (ok, text) = s.run(&flacs(), &["add", &album.to_string_lossy()]);
    assert!(ok, "{text}");
    assert!(
        text.contains("links leading out of it") && text.contains("Song.lrc"),
        "{text}"
    );
    assert!(text.contains("extras"), "{text}");
    let manual = s.dir.path().join("home/sources/manual/Album");
    assert!(manual.join("Song.flac").exists());
    assert!(
        manual.join("cover.jpg").exists(),
        "a link inside is followed"
    );
    assert!(!manual.join("Song.lrc").exists());
    assert!(!manual.join("extras").exists());
    assert_eq!(
        s.songs(),
        [vec![
            SourceKey::Manual("Album/Song.flac".into()),
            SourceKey::Manual("added/front.jpg".into())
        ]],
        "art/front.jpg, which no song file beside it brings, given as a named picture is"
    );
    assert!(s.dir.path().join("lib/Artist/Record/02 Song.flac").exists());
    assert!(!s.dir.path().join("lib/Artist/Record/02 Song.lrc").exists());
}

#[test]
fn a_file_added_by_its_link_is_followed_and_its_sidecars_bounded_by_its_folder() {
    let s = Setup::new();
    let real = s.file("elsewhere/Song.flac");
    let secret = s.written("secret/creds", "[00:01.00]a secret");
    let rips = s.dir.path().join("rips");
    std::fs::create_dir_all(&rips).unwrap();
    let song = rips.join("Song.flac");
    let linked = crate::testing::link(Path::new(&real), &song)
        && crate::testing::link(Path::new(&secret), &rips.join("Song.lrc"));
    if !linked {
        return;
    }
    let (ok, text) = s.run(&flacs(), &["add", &song.to_string_lossy()]);
    assert!(ok, "{text}");
    assert!(text.contains("links leading out of it: Song.lrc"), "{text}");
    let manual = s.dir.path().join("home/sources/manual");
    assert_eq!(
        std::fs::read(manual.join("Song.flac")).unwrap(),
        b"elsewhere/Song.flac",
        "the path given is followed"
    );
    assert!(!manual.join("Song.lrc").exists());
    assert!(!s.dir.path().join("lib/Artist/Record/02 Song.lrc").exists());
}

#[test]
fn a_torrent_is_taken_up_through_no_link_out_of_its_folder() {
    let s = Setup::new();
    let bytes = crate::testing::metainfo("Lantern Weather", &["Paper Comets.flac", "scan.png"]);
    let hash = crate::torrent::Metainfo::read(&bytes)
        .unwrap()
        .hash()
        .to_string();
    let file = s.dir.path().join("a.torrent");
    std::fs::write(&file, &bytes).unwrap();
    let downloads = s.dir.path().join("downloads");
    s.written("downloads/Lantern Weather/Paper Comets.flac", "x");
    let secret = s.written("secret/key", "y");
    let scan = downloads.join("Lantern Weather").join("scan.png");
    if !crate::testing::link(Path::new(&secret), &scan) {
        return;
    }
    let partial = s.dir.path().join("home/partial/torrent").join(&hash);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let saw = seen.clone();
    let fake = Fake {
        on_stream: Some(Box::new(move |_: &[String]| {
            saw.lock().unwrap().extend([
                std::fs::read(partial.join("files").join("1.flac")).ok(),
                std::fs::read(partial.join("files").join("2.png")).ok(),
            ]);
            true
        })),
        ..Fake::default()
    }
    .probe(".flac", FLAC);
    let (ok, text) = s.run(
        &fake,
        &[
            "add",
            "--no-match",
            "--from",
            downloads.to_str().unwrap(),
            file.to_str().unwrap(),
        ],
    );
    assert!(ok, "{text}");
    assert!(
        text.contains("links leading out of it") && text.contains("scan.png"),
        "{text}"
    );
    assert!(
        text.contains("Taking up 1 file(s) of Lantern Weather"),
        "{text}"
    );
    assert_eq!(*seen.lock().unwrap(), [Some(b"x".to_vec()), None]);
}

#[test]
fn a_file_muman_does_not_read_is_not_added_and_the_rest_is() {
    let s = Setup::new();
    let notes = s.file("rips/notes.docx");
    let song = s.file("rips/Song.flac");
    let e = run_with(
        &s.job(&["add", &notes, &song]),
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        format!("{e:#}").contains("notes.docx, which is no song, lyrics, picture or tags"),
        "{e:#}"
    );
    let manual = s.dir.path().join("home/sources/manual");
    assert!(!manual.join("notes.docx").exists());
    assert_eq!(s.songs(), [vec![SourceKey::Manual("Song.flac".into())]]);
    assert!(s.dir.path().join("lib/Artist/Record/02 Song.flac").exists());

    s.file("rips/Album/a.flac");
    s.file("rips/Album/booklet.pdf");
    s.file("rips/Album/Scans/rip.xyz");
    let album = s.dir.path().join("rips").join("Album");
    let (ok, text) = s.run(&flacs(), &["add", &album.to_string_lossy()]);
    assert!(ok, "{text}");
    assert!(
        text.contains("Left out of") && text.contains("booklet.pdf") && text.contains("rip.xyz"),
        "{text}"
    );
    assert!(manual.join("Album").join("a.flac").exists());
    assert!(!manual.join("Album").join("booklet.pdf").exists());
    assert!(!manual.join("Album").join("Scans").exists());
}

#[test]
fn a_history_that_cannot_be_written_costs_a_run_only_its_undo() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let history = s.dir.path().join("home/history");
    std::fs::remove_dir_all(&history).unwrap();
    std::fs::write(&history, "not a folder").unwrap();
    let (ok, text) = s.run(&flacs(), &["set", "-y", "manual:a.flac", "genre=Folk"]);
    assert!(ok, "{text}");
    assert!(text.contains("Updated"), "{text}");
    assert!(
        s.listed(&["list", "genre:=folk", "--keys"])
            .contains("manual:a.flac")
    );
}

#[test]
fn status_follows_a_dropped_file_that_moved_as_the_next_sync_will() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let to = s.dir.path().join("home/sources/manual/Artist/a.flac");
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::rename(s.dir.path().join("home/sources/manual/a.flac"), &to).unwrap();
    let (_, said) = s.run(&flacs(), &["status"]);
    let (_, text) = s.report(&flacs(), &["status"]);
    assert!(
        said.contains(
            "Moved, followed by the next sync: manual:a.flac is now manual:Artist/a.flac"
        ),
        "{said}"
    );
    let both = format!("{said}{text}");
    assert!(
        !both.contains("Failed") && !both.contains("Not listed yet"),
        "{both}"
    );
    assert_eq!(
        s.songs(),
        [vec![SourceKey::Manual("a.flac".into())]],
        "status changes no song"
    );
}

#[test]
fn status_of_a_home_not_made_yet_makes_none() {
    let s = Setup::new();
    let (ok, text) = s.run(&flacs(), &["status"]);
    assert!(ok, "{text}");
    assert!(!s.dir.path().join("home").exists(), "{text}");
}

#[test]
fn a_cover_the_tags_cannot_hold_costs_its_song_only_the_cover() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let jpg = s.file("home/sources/manual/a.jpg");
    std::fs::write(&jpg, "jpeg").unwrap();
    let fake = Fake {
        pictures: vec![("a.jpg".into(), (1200, 1200))],
        ..flacs()
    };
    let (ok, text) = s.run(&fake, &["sync"]);
    assert!(ok, "{text}");
    assert!(text.contains("no cover"), "{text}");
    assert!(
        s.dir.path().join("lib/Artist/Record/02 Song.flac").exists(),
        "{text}"
    );
}

#[test]
fn undo_is_refused_over_a_file_of_your_own_where_a_removed_song_was() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&flacs(), &["remove", "-y", "manual:a.flac"]);
    assert!(ok, "{text}");
    let mine = s.dir.path().join("lib/Artist/Record/02 Song.flac");
    std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
    std::fs::write(&mine, "my own rip").unwrap();
    assert!(s.refused(&["undo", "-y"]).contains("is not muman's"));
    assert_eq!(std::fs::read_to_string(&mine).unwrap(), "my own rip");
}

#[test]
fn undo_of_a_home_s_first_add_leaves_a_home_that_syncs() {
    let s = Setup::new();
    let song = s.file("rips/a.flac");
    let (ok, text) = s.run(&flacs(), &["add", &song]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert!(s.dir.path().join("home/songs.toml").exists(), "{text}");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert!(s.songs().is_empty(), "{text}");
}

#[test]
fn undo_of_an_add_takes_out_the_copy_it_made() {
    let s = Setup::new();
    let a = s.file("rips/a.flac");
    let b = s.file("rips/b.flac");
    let (ok, text) = s.run(&flacs(), &["add", &a]);
    assert!(ok, "{text}");
    let (ok, text) = s.run(&flacs(), &["add", &b]);
    assert!(ok, "{text}");
    let copy = s.dir.path().join("home/sources/manual/b.flac");
    assert!(copy.exists());
    let (ok, text) = s.run(&flacs(), &["undo", "-n"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("sources/manual/b.flac: the copy the run made, deleted"),
        "{text}"
    );
    assert!(copy.exists(), "-n changes nothing");
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert!(!copy.exists(), "{text}");
    assert!(Path::new(&b).exists(), "the file it was copied from stays");
    assert!(s.dir.path().join("home/sources/manual/a.flac").exists());
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    assert_eq!(
        s.songs(),
        [vec![SourceKey::Manual("a.flac".into())]],
        "{text}"
    );
}

/// The library's files, by their paths in it, with their bytes.
fn library_of(s: &Setup) -> BTreeMap<PathBuf, Vec<u8>> {
    let lib = s.dir.path().join("lib");
    crate::store::files_below(&lib, usize::MAX, |_| true)
        .unwrap_or_default()
        .into_iter()
        .map(|f| {
            let bytes = std::fs::read(&f).unwrap();
            (f.strip_prefix(&lib).unwrap().to_path_buf(), bytes)
        })
        .collect()
}

/// The files a write stopped before its rename left in the home.
fn parts_in(s: &Setup) -> Vec<PathBuf> {
    crate::store::files_below(&s.dir.path().join("home"), usize::MAX, |_| true)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.extension().is_some_and(|e| e == "part"))
        .collect()
}

/// What a run killed partway may leave, once recovered.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ends {
    /// The change made whole, as a run that ended makes it.
    Whole,
    /// The change made whole, or not made at all.
    WholeOrNone,
}

/// Run `command` on a home `prepare` makes, killed at each change of a
/// file under the test's folder in turn, as a crash or a Ctrl-C would stop
/// it; `recover` then brings the home back. After each, every file the
/// state records is there and none is half written, the library is as
/// `ends` allows, and one more sync changes nothing.
fn survives_a_kill_at_every_step(
    prepare: &dyn Fn(&Setup),
    command: &[&str],
    recover: &dyn Fn(&Setup),
    ends: Ends,
) {
    let whole = Setup::new();
    prepare(&whole);
    let before = library_of(&whole);
    let counted = crate::kill::kill_at(whole.dir.path(), usize::MAX);
    whole.run(&flacs(), command);
    let steps = counted.seen();
    drop(counted);
    recover(&whole);
    let after = library_of(&whole);
    assert_ne!(before, after, "{command:?} changes the library");
    assert!(steps > 0);
    for at in 1..=steps {
        let s = Setup::new();
        prepare(&s);
        let kill = crate::kill::kill_at(s.dir.path(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_with(
                &s.job(command),
                &flacs(),
                &Server::default(),
                None,
                &mut Vec::new(),
                &mut Vec::new(),
            )
        }));
        drop(kill);
        assert!(
            stopped.is_err(),
            "{command:?} not stopped at step {at} of {steps}"
        );
        let stopped_as = library_of(&s);
        let guard = crate::kill::guard_library(&s.dir.path().join("lib"));
        let _ = run_with(
            &s.job(&["status"]),
            &flacs(),
            &Server::default(),
            None,
            &mut Vec::new(),
            &mut Vec::new(),
        );
        drop(guard);
        assert_eq!(
            library_of(&s),
            stopped_as,
            "status after step {at} moved nothing"
        );
        // As old as a part a stopped run left long ago.
        let earlier = SystemTime::now() - std::time::Duration::from_secs(3600);
        for part in parts_in(&s) {
            filetime::set_file_mtime(&part, filetime::FileTime::from_system_time(earlier)).unwrap();
        }
        recover(&s);
        assert_eq!(parts_in(&s), Vec::<PathBuf>::new(), "step {at}");
        let now = library_of(&s);
        let fine = now == after || (ends == Ends::WholeOrNone && now == before);
        let show = |lib: &BTreeMap<PathBuf, Vec<u8>>| {
            lib.iter()
                .map(|(p, b)| format!("{} {}", p.display(), crate::hash::digest(b)))
                .collect::<Vec<_>>()
        };
        assert!(
            fine,
            "{command:?} killed at step {at} of {steps}:\nnow {:?}\nwhole {:?}\nbefore {:?}",
            show(&now),
            show(&after),
            show(&before)
        );
        let (_, text) = s.run(&flacs(), &["sync"]);
        for verb in ["Added", "Updated", "Written again", "Moved", "Removed"] {
            assert!(
                !text.contains(verb),
                "step {at}: a sync after changed: {text}"
            );
        }
    }
}

/// Two songs of one's own, synced.
fn two_songs(s: &Setup) {
    s.file("home/sources/manual/a.flac");
    s.file("home/sources/manual/b.flac");
    s.run(&flacs(), &["sync"]);
}

fn sync_again(s: &Setup) {
    s.run(&flacs(), &["sync"]);
}

#[test]
fn a_sync_moving_songs_to_a_new_template_survives_a_kill_anywhere() {
    let retemplated = |s: &Setup| {
        two_songs(s);
        let songs = s.dir.path().join("home/songs.toml");
        let listed = std::fs::read_to_string(&songs).unwrap();
        let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
        std::fs::write(
            &songs,
            listed.replace(&default, "template = \"{{ album }}/{{ title }}\""),
        )
        .unwrap();
    };
    survives_a_kill_at_every_step(&retemplated, &["sync"], &sync_again, Ends::Whole);
}

#[test]
fn a_sync_moving_songs_and_their_lyrics_to_a_new_template_survives_a_kill_anywhere() {
    let retemplated = |s: &Setup| {
        s.file("home/sources/manual/a.flac");
        s.written("home/sources/manual/a.lrc", "[00:01.00]Paper comets\n");
        s.run(&flacs(), &["sync"]);
        let songs = s.dir.path().join("home/songs.toml");
        let listed = std::fs::read_to_string(&songs).unwrap();
        let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
        std::fs::write(
            &songs,
            listed.replace(&default, "template = \"{{ album }}/{{ title }}\""),
        )
        .unwrap();
    };
    survives_a_kill_at_every_step(&retemplated, &["sync"], &sync_again, Ends::Whole);
}

#[test]
fn a_remove_survives_a_kill_anywhere() {
    survives_a_kill_at_every_step(
        &two_songs,
        &["remove", "-y", "manual:a.flac"],
        &sync_again,
        Ends::WholeOrNone,
    );
}

#[test]
fn a_set_writing_a_song_again_survives_a_kill_anywhere() {
    survives_a_kill_at_every_step(
        &two_songs,
        &["set", "-y", "manual:a.flac", "album=Lanternfall"],
        &sync_again,
        Ends::WholeOrNone,
    );
}

#[test]
fn an_undo_survives_a_kill_anywhere_and_finishes_when_run_again() {
    let set = |s: &Setup| {
        two_songs(s);
        s.run(
            &flacs(),
            &["set", "-y", "manual:a.flac", "album=Lanternfall"],
        );
    };
    // An undo stopped with its record still undoing is run again; one
    // stopped after, in its sync, is synced.
    let finish = |s: &Setup| {
        let history = s.dir.path().join("home/history");
        let undoing = std::fs::read_dir(&history)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                std::fs::read_to_string(e.path().join("run.json"))
                    .is_ok_and(|t| t.contains("\"undoing\":true"))
            });
        if undoing {
            s.run(&flacs(), &["undo", "-y"]);
        } else {
            s.run(&flacs(), &["sync"]);
        }
    };
    survives_a_kill_at_every_step(&set, &["undo", "-y"], &finish, Ends::WholeOrNone);
}

#[test]
fn an_undo_killed_anywhere_is_finished_by_the_next_and_goes_no_further() {
    let set = |s: &Setup| {
        two_songs(s);
        s.run(
            &flacs(),
            &["set", "-y", "manual:a.flac", "album=Lanternfall"],
        );
    };
    let whole = Setup::new();
    two_songs(&whole);
    let listed = std::fs::read_to_string(whole.dir.path().join("home/songs.toml")).unwrap();
    let synced = library_of(&whole);
    let whole = Setup::new();
    set(&whole);
    let counted = crate::kill::kill_at(whole.dir.path(), usize::MAX);
    whole.run(&flacs(), &["undo", "-y"]);
    let steps = counted.seen();
    drop(counted);
    assert_eq!(library_of(&whole), synced);
    for at in 1..=steps {
        let s = Setup::new();
        set(&s);
        let kill = crate::kill::kill_at(s.dir.path(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_with(
                &s.job(&["undo", "-y"]),
                &flacs(),
                &Server::default(),
                None,
                &mut Vec::new(),
                &mut Vec::new(),
            )
        }));
        drop(kill);
        assert!(stopped.is_err(), "not stopped at step {at} of {steps}");
        let (_, text) = s.run(&flacs(), &["undo", "-y"]);
        assert_eq!(library_of(&s), synced, "step {at} of {steps}: {text}");
        assert_eq!(
            std::fs::read_to_string(s.dir.path().join("home/songs.toml")).unwrap(),
            listed,
            "step {at}: {text}"
        );
    }
}

#[test]
fn a_change_waits_for_an_undo_stopped_before_it_put_the_library_back() {
    let s = Setup::new();
    two_songs(&s);
    let synced = library_of(&s);
    s.run(
        &flacs(),
        &["set", "-y", "manual:a.flac", "album=Lanternfall"],
    );
    let home = s.dir.path().join("home");
    // Stopped once its record says it is undoing, before it puts back a file.
    let kill = crate::kill::kill_at(s.dir.path(), 2);
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_with(
            &s.job(&["undo", "-y"]),
            &flacs(),
            &Server::default(),
            None,
            &mut Vec::new(),
            &mut Vec::new(),
        )
    }));
    drop(kill);
    assert!(stopped.is_err());
    assert!(crate::history::undo_unfinished(&home));
    for args in [&["sync"][..], &["remove", "-y", "manual:b.flac"]] {
        let e = s.refused(args);
        assert!(e.contains("run `muman undo` to finish it"), "{args:?}: {e}");
    }
    let (ok, text) = s.run(&flacs(), &["undo", "-y"]);
    assert!(ok, "{text}");
    assert_eq!(library_of(&s), synced);
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
}

#[test]
fn check_names_an_undo_stopped_before_its_sync_ended() {
    let s = Setup::new();
    two_songs(&s);
    s.run(
        &flacs(),
        &["set", "-y", "manual:a.flac", "album=Lanternfall"],
    );
    let dirs = s.job(&["undo"]).dirs;
    let planned = crate::history::plan(&dirs).unwrap();
    drop(crate::history::undo(&dirs, planned, &mut Vec::new()).unwrap());
    let mut report = Vec::new();
    let clean = run_with(
        &s.job(&["check"]),
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut report,
    )
    .unwrap();
    let report = String::from_utf8(report).unwrap();
    assert!(!clean, "{report}");
    assert!(report.contains("an undo stopped partway"), "{report}");
    let (ok, text) = s.run(&flacs(), &["undo", "-n"]);
    assert!(ok, "{text}");
    assert!(text.contains("Finish the undo stopped partway"), "{text}");
}

#[test]
fn a_sync_killed_after_a_hand_edit_is_undone_with_the_edit() {
    let synced = Setup::new();
    two_songs(&synced);
    let songs = |s: &Setup| s.dir.path().join("home/songs.toml");
    let listed = std::fs::read_to_string(songs(&synced)).unwrap();
    let edited = listed
        .replace(
            "codecs = [\"opus\", \"vorbis\", \"aac\", \"mp3\", \"flac\"]",
            "codecs = [\"opus\"]",
        )
        .replace("lossless = \"flac\"", "lossless = \"opus\"");
    assert_ne!(edited, listed);
    let prepare = |s: &Setup| {
        two_songs(s);
        std::fs::write(songs(s), &edited).unwrap();
    };
    let whole = Setup::new();
    prepare(&whole);
    let before = library_of(&whole);
    let counted = crate::kill::kill_at(whole.dir.path(), usize::MAX);
    whole.run(&flacs(), &["sync"]);
    let steps = counted.seen();
    drop(counted);
    assert_ne!(
        library_of(&whole),
        before,
        "the edit writes every song again"
    );
    for at in 1..=steps {
        let s = Setup::new();
        prepare(&s);
        let kill = crate::kill::kill_at(s.dir.path(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_with(
                &s.job(&["sync"]),
                &flacs(),
                &Server::default(),
                None,
                &mut Vec::new(),
                &mut Vec::new(),
            )
        }));
        drop(kill);
        assert!(stopped.is_err(), "not stopped at step {at} of {steps}");
        if library_of(&s) == before {
            continue;
        }
        let mut out = Vec::new();
        let undone = run_with(
            &s.job(&["undo", "-y"]),
            &flacs(),
            &Server::default(),
            None,
            &mut out,
            &mut Vec::new(),
        );
        let text = String::from_utf8_lossy(&out);
        assert!(undone.is_ok(), "step {at} of {steps}: {undone:?}: {text}");
        assert_eq!(library_of(&s), before, "step {at} of {steps}: {text}");
        assert_eq!(std::fs::read_to_string(songs(&s)).unwrap(), listed);
    }
}

#[test]
fn songs_moving_with_their_lyrics_survive_a_kill_anywhere() {
    let retemplated = |s: &Setup| {
        for name in ["a", "b"] {
            s.file(&format!("home/sources/manual/{name}.flac"));
            let lrc = s.dir.path().join(format!("home/sources/manual/{name}.lrc"));
            std::fs::write(&lrc, format!("[00:01.00]sung by {name}\n")).unwrap();
        }
        s.run(&flacs(), &["sync"]);
        assert!(
            library_of(s)
                .keys()
                .any(|p| p.extension().is_some_and(|e| e == "lrc"))
        );
        let songs = s.dir.path().join("home/songs.toml");
        let listed = std::fs::read_to_string(&songs).unwrap();
        let default = format!("template = \"{}\"", crate::settings::DEFAULT_TEMPLATE);
        std::fs::write(
            &songs,
            listed.replace(&default, "template = \"{{ title }}\""),
        )
        .unwrap();
    };
    survives_a_kill_at_every_step(&retemplated, &["sync"], &sync_again, Ends::Whole);
}

mod attaching;
mod convergence;
