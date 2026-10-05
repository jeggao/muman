use std::collections::BTreeMap;

use super::*;
use crate::lrclib::testing::Server;
use crate::testing::{FLAC, Fake};

fn cli(args: &[&str]) -> Cli {
    Cli::try_parse_from(std::iter::once("muman").chain(args.iter().copied())).unwrap()
}

fn defaults(root: &Path) -> Defaults {
    Defaults {
        home: root.join("data"),
        library: root.join("music"),
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
        (ok, String::from_utf8(out).unwrap())
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

fn flacs() -> Fake {
    Fake::default().probe(".flac", FLAC)
}

#[test]
fn an_added_file_is_copied_with_its_lyrics_listed_and_written() {
    let s = Setup::new();
    let song = s.file("rips/Song.flac");
    s.file("rips/Song.lrc");
    let (ok, text) = s.run(&flacs(), &["add", &song]);
    assert!(ok, "{text}");
    assert!(
        s.dir.path().join("rips/Song.flac").exists(),
        "the user's file stays"
    );
    assert!(s.dir.path().join("home/sources/manual/Song.lrc").exists());
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
    assert!(
        s.dir
            .path()
            .join("home/sources/yt-dlp/chan/Song [aaaaaaaaaaa].mkv")
            .exists()
    );
    assert_eq!(s.songs(), [vec![SourceKey::youtube("aaaaaaaaaaa")]]);
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
                    "Youtube aaaaaaaaaaa {}\n",
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
fn status_changes_nothing() {
    let s = Setup::new();
    s.file("home/sources/manual/a.flac");
    let (_, text) = s.run(&flacs(), &["status"]);
    assert!(text.contains("Not listed yet"), "{text}");
    assert!(!s.dir.path().join("home/songs.toml").exists());
    assert!(!s.dir.path().join("lib").exists());
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
    let refetch = Server::default().answer("/api/get/7", RECORD);
    let (ok, text) = s.run_against(&refetch, &["sync"]);
    assert!(ok, "{text}");
    assert!(
        s.dir.path().join("home/sources/lrclib/7.lrc").exists(),
        "{text}"
    );
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
        &s.job(&["sync"]),
        &fake,
        &Server::default(),
        None,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(fake.ran("Artist/Record/02 Song.flac"), "{text}");
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
