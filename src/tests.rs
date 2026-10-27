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

    fn run(&self, fake: &Fake, args: &[&str]) -> (bool, String) {
        let mut out = Vec::new();
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
        if dirs.home.join(crate::dirs::STATE).exists()
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
    assert!(format!("{e:#}").contains("exists already"), "{e:#}");
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
fn a_song_list_naming_the_old_tables_is_moved_and_writes_no_song_again() {
    let s = Setup::new();
    s.file("home/sources/manual/Song.flac");
    s.written("home/sources/manual/Song.lrc", "[00:01.00]Paper comets\n");
    let (ok, text) = s.run(&flacs(), &["sync"]);
    assert!(ok, "{text}");
    let list = s.dir.path().join("home/songs.toml");
    let now = std::fs::read_to_string(&list).unwrap();
    let old = now
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
    assert_eq!(std::fs::read_to_string(&list).unwrap(), now);
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
        text.contains("Artist/Record/02 Song.lrc is not muman's"),
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
        "manual:b.flac\nmanual:a.flac\n"
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
        &mut || Ok(()),
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
    let due = crate::sync::due(&manifest, &store, &state.failures, false, now);
    let missing = crate::sync::Fetch::Missing;
    assert_eq!(
        due.now,
        BTreeMap::from([
            (key("jamendo.com:1234567"), missing),
            (torrent.clone(), crate::sync::Fetch::Damaged),
        ]),
        "a manual file and a key of no address to fetch from are no fetch"
    );
    assert_eq!(
        due.waiting,
        std::collections::BTreeSet::from([key("jamendo.com:7654321")])
    );
    let retried = crate::sync::due(&manifest, &store, &state.failures, true, now);
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
    std::fs::write(
        &list,
        format!("{listed}\n[providers.lrclib]\nenabled = false\n"),
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
fn a_song_on_an_album_asks_nothing_of_musicbrainz() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
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
fn hooks_run_for_each_file_written_and_once_a_run_changed_anything() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    std::fs::write(
        s.dir.path().join("home/songs.toml"),
        "version = 1\n[[hook]]\non = \"written\"\nrun = [\"tagger\", \"{rel}\"]\n\
         [[hook]]\non = \"changed\"\nrun = [\"mpc\", \"update\", \"{written}\"]\n",
    )
    .unwrap();
    let fake = flacs();
    let mut out = Vec::new();
    run_with(
        &s.job(&["--trust", "sync"]),
        &fake,
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(fake.ran("./Artist/Record/02 Song.flac"), "{text}");
    assert!(
        fake.calls().iter().any(|c| c == &["mpc", "update", "1"]),
        "{:?}",
        fake.calls()
    );
    let fake = flacs();
    run_with(
        &s.job(&["sync"]),
        &fake,
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap();
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

const HOOKED: &str = "version = 1\n[[hook]]\non = \"written\"\nrun = [\"tagger\", \"{rel}\"]\n";

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
        text.contains("[[hook]] 1 (new): on = \"written\""),
        "{text}"
    );
    assert!(fake.calls().is_empty(), "declined, nothing runs");
    let fake = flacs();
    let mut yes = crate::ui::MockPrompter::answering([true]);
    s.trusting(&fake, &["sync"], Some(&mut yes)).unwrap();
    assert!(fake.ran("tagger"), "{:?}", fake.calls());
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
    let hook = "[[hook]]\non = \"written\"\nrun = [\"tagger\", \"{rel}\"]\n";
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
    let own = "\n[providers.acoustid]\nkey = \"k0000000\"\n";
    std::fs::write(&songs, format!("{listed}{own}")).unwrap();
    s.trusting(&fake, &["--trust", "sync"], None).unwrap();
    State::keep_trusted(&s.dir.path().join("home"), &BTreeMap::new()).unwrap();
    let zip = s.dir.path().join("out.zip").to_string_lossy().into_owned();
    s.trusting(&fake, &["export", "-o", &zip], None).unwrap();
    let encoded = ["export", "-o", &zip, "--codec", "mp3"];
    let e = s.trusting(&fake, &encoded, None).unwrap_err();
    assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
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
    let own = "\n[providers.acoustid]\nkey = \"k0000000\"\n";
    std::fs::write(&songs, format!("{listed}{own}")).unwrap();
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
    let state = std::fs::read(s.dir.path().join("home/state.json")).unwrap();
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
        std::fs::read(s.dir.path().join("home/state.json")).unwrap(),
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
    let state = std::fs::read(s.dir.path().join("home/state.json")).unwrap();
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
        std::fs::read(s.dir.path().join("home/state.json")).unwrap(),
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
        [vec![SourceKey::Manual("Album/Song.flac".into())]]
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
    let counted = crate::testing::kill_at(whole.dir.path(), usize::MAX);
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
        let kill = crate::testing::kill_at(s.dir.path(), at);
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
        recover(&s);
        let now = library_of(&s);
        let fine = now == after || (ends == Ends::WholeOrNone && now == before);
        let show = |lib: &BTreeMap<PathBuf, Vec<u8>>| {
            lib.iter()
                .map(|(p, b)| format!("{} {}", p.display(), crate::facts::digest(b)))
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
    let counted = crate::testing::kill_at(whole.dir.path(), usize::MAX);
    whole.run(&flacs(), &["undo", "-y"]);
    let steps = counted.seen();
    drop(counted);
    assert_eq!(library_of(&whole), synced);
    for at in 1..=steps {
        let s = Setup::new();
        set(&s);
        let kill = crate::testing::kill_at(s.dir.path(), at);
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
    let counted = crate::testing::kill_at(whole.dir.path(), usize::MAX);
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
        let kill = crate::testing::kill_at(s.dir.path(), at);
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
