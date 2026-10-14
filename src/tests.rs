use std::collections::BTreeMap;
use std::fmt::Write as _;

use clap::Parser;

use super::*;
use crate::lrclib::testing::Server;
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
    assert!(listed.contains("\nedition = 2\n"), "{listed}");
    assert!(listed.contains("opus_bitrate = \"192 kb/s\""), "{listed}");
    assert!(!listed.contains("opus_kbps"), "{listed}");
    assert!(listed.contains("[ytdlp]"), "{listed}");
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
    hooked(&["sync"]);
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
            (entry.name().to_string(), bytes)
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
    clear_stale(&partial, std::time::Duration::from_secs(14 * 24 * 3600));
    assert!(!stale.exists());
    assert!(
        !partial.join("Chan").exists(),
        "the folder it leaves empty goes"
    );
    assert!(!partial.join("Empty").exists());
    assert!(fresh.exists(), "a later run resumes it");
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
        assert!(names_a_path(path), "{path}");
    }
    for address in [
        "https://archive.org/details/item0001",
        "ytsearch:lantern weather",
        "youtube.com/watch?v=vid00000001",
    ] {
        assert!(!names_a_path(address), "{address}");
    }
}

#[test]
fn a_file_muman_does_not_read_is_not_added() {
    let s = Setup::new();
    let notes = s.file("rips/notes.docx");
    let e = run_with(
        &s.job(&["add", &notes]),
        &flacs(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        format!("{e:#}").contains("is no song, lyrics, picture or tags"),
        "{e:#}"
    );
    assert!(!s.dir.path().join("home/sources/manual/notes.docx").exists());
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
        .replace("codecs = [\"opus\", \"flac\"]", "codecs = [\"opus\"]")
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
