use std::fmt::Write as _;
use std::path::PathBuf;

use super::measure::out_of_space;
use super::prune::{Pruned, prune_plan};
use super::*;
use crate::facts::Facts;
use crate::history::Run;
use crate::resolve::Plan;
use crate::state::{Step, Written};
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
        locks: Some(dir.path().to_path_buf()),
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
        if !opts.dry_run && State::load(&self.dirs.home).is_ok() {
            return self.foretold(fake, opts);
        }
        self.run_plain(fake, opts)
    }

    fn run_plain(&self, fake: &Fake, opts: Options) -> (bool, String) {
        let mut out = Vec::new();
        let ok = reconcile(fake, &self.dirs, opts, Recording::Test, &mut out)
            .unwrap()
            .ok;
        let text = String::from_utf8(out).unwrap();
        if !opts.dry_run {
            self.holds(&text);
        }
        (ok, text)
    }

    /// [`Home::run`], once a dry run has foretold what the sync does with
    /// each library file, which it then does ([`as_foretold`]).
    fn foretold(&self, fake: &Fake, opts: Options) -> (bool, String) {
        // As `check` and `info` foresee it, before `status` measures anything.
        let forecast = (!opts.force && !opts.accept)
            .then(|| super::forecast(fake, &self.dirs, opts.settling).unwrap())
            .flatten();
        let dry = Options {
            dry_run: true,
            ..opts
        };
        let sink = &mut std::io::sink();
        let synced = reconcile(fake, &self.dirs, dry, Recording::DryRun, sink).unwrap();
        let before = stamped(&self.dirs.library);
        let (ok, text) = self.run_plain(fake, opts);
        let after = stamped(&self.dirs.library);
        let foretold = synced.foretold.expect("a dry run foretells");
        if let Err(e) = as_foretold(&foretold, &before, &after) {
            panic!("the sync did otherwise than status foretold: {e}: {text}");
        }
        if let Some(Err(e)) = forecast.map(|f| as_foretold(&f, &before, &after)) {
            panic!("the sync did otherwise than check and info foresaw: {e}: {text}");
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
        let _ = reconcile_into(
            fake,
            &self.dirs,
            opts,
            Recording::DryRun,
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
            recorded.extend(written.sidecar_paths().cloned());
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
    h.run_plain(&infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]), Options::default());
    std::fs::remove_file(gone).unwrap();
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { title = \"New\" }\n");
    let (ok, text) = h.run_plain(&infos(&["aaaaaaaaaaa"]), Options::default());
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
    let _ = reconcile(
        &infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]),
        &elsewhere,
        dry,
        Recording::DryRun,
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
    assert!(
        text.contains("(a file beside it written again)"),
        "lyrics gone: {text}"
    );
    assert!(!text.contains("Would remove"), "{text}");
    assert!(
        text.contains("Would leave in place, changed since muman wrote it"),
        "{text}"
    );
}

#[test]
fn a_file_beside_a_song_deleted_is_written_again_alone_and_said_so() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    let song = h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus");
    let lrc = h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.lrc");
    let stamp = crate::store::stamp_text(&song);
    std::fs::remove_file(&lrc).unwrap();
    let before = renders(&fake);
    let mut run = Run::begin(&h.dirs.home).unwrap();
    let mut out = Vec::new();
    let synced = reconcile(
        &fake,
        &h.dirs,
        Options::default(),
        Recording::Run(&mut run),
        &mut out,
    )
    .unwrap();
    run.finish(&h.dirs.home).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(synced.ok, "{text}");
    assert_eq!(renders(&fake), before, "nothing encoded again: {text}");
    assert!(lrc.exists(), "{text}");
    assert_eq!(crate::store::stamp_text(&song), stamp, "its file as it was");
    assert!(
        text.contains("Written again: Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.lrc"),
        "{text}"
    );
    assert!(
        !text.contains(".opus"),
        "its file is not said written: {text}"
    );
    let undo = crate::history::plan(&h.dirs)
        .map(|p| p.lines().join("\n"))
        .unwrap_or_default();
    assert!(!undo.contains(".opus"), "nothing of its file kept: {undo}");
}

#[test]
fn lyrics_taken_off_a_song_are_said_removed() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\nlyrics = false\n");
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(
        text.contains("Removed: Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.lrc"),
        "{text}"
    );
    assert!(
        !h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.lrc")
            .exists()
    );
}

#[test]
fn a_changed_lrc_of_a_song_taking_no_lyrics_is_said_removed_by_force() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    let lrc = "Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.lrc";
    std::fs::write(h.lib(lrc), "[00:09.00]my own line\n").unwrap();
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\nlyrics = false\n");
    let said = crate::diagnose::diagnose(
        &crate::testing::Fake::default(),
        &h.dirs,
        std::time::Duration::ZERO,
    )
    .unwrap()
    .said;
    let forced = format!("{lrc} (`sync --force` removes it)");
    assert!(
        (said.iter())
            .any(|s| matches!(s, crate::diagnose::Said::Problem(l) if l.ends_with(&forced))),
        "{said:?}"
    );
    let (_, text) = h.run(&fake, Options::default());
    let text = crate::ui::plain(&text);
    assert!(
        text.contains(&format!(
            "{lrc} changed since muman wrote it (`sync --force` removes it)"
        )),
        "{text}"
    );
    assert!(h.lib(lrc).exists(), "{text}");
    let force = Options {
        force: true,
        ..Options::default()
    };
    let (ok, text) = h.run(&fake, force);
    assert!(ok, "{text}");
    assert!(text.contains(&format!("Removed: {lrc}")), "{text}");
    assert!(!h.lib(lrc).exists(), "{text}");
}

#[test]
fn lyrics_taken_off_a_song_whose_file_holds_none_name_its_lrc_alone() {
    let h = home();
    listing(&h, "", &["a"]);
    std::fs::write(h.dirs.manual().join("a.lrc"), "[00:01.00]la la\n").unwrap();
    let fake = Fake::default().probe("a.flac", &tagged_flac("Kite", 1, "Record", false));
    assert!(h.run(&fake, Options::default()).0);
    let song = h.lib("Artist/Record/01 Kite.flac");
    assert!(h.lib("Artist/Record/01 Kite.lrc").exists());
    let stamp = crate::store::stamp_text(&song);
    h.songs("[[song]]\nsources = [\"manual:a.flac\"]\nlyrics = false\n");
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert_eq!(crate::store::stamp_text(&song), stamp, "{text}");
    assert!(
        text.contains("Removed: Artist/Record/01 Kite.lrc"),
        "{text}"
    );
    assert!(
        !text.contains(".flac"),
        "its file, untouched, is not said written: {text}"
    );
}

#[test]
fn a_changed_lrc_of_a_song_dropped_is_said_left_by_status_and_sync_alike() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    assert!(h.run(&fake, Options::default()).0);
    let lrc = "Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.lrc";
    std::fs::write(h.lib(lrc), "[00:01.00]my own words\n").unwrap();
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let said = h.status(&fake, &[], false);
    let (ok, done) = h.run(&fake, Options::default());
    assert!(ok, "{done}");
    let done = crate::ui::plain(&done);
    let after = |text: &str, lead: &str| -> BTreeSet<String> {
        text.lines()
            .filter_map(|l| Some(l.split_once(lead)?.1.trim().to_string()))
            .collect()
    };
    assert_eq!(
        after(&said, "Would remove: "),
        after(&done, "Removed: "),
        "{said}\n{done}"
    );
    assert!(
        said.contains(&format!(
            "Would leave in place, changed since muman wrote it: {lrc}"
        )),
        "{said}"
    );
    assert!(
        done.contains(&format!(
            "Left in place, changed since muman wrote it: {lrc}; it is yours now"
        )),
        "{done}"
    );
    assert_eq!(
        std::fs::read_to_string(h.lib(lrc)).unwrap(),
        "[00:01.00]my own words\n"
    );
    h.audit(&[lrc], &done);
}

#[test]
fn a_folder_s_cover_freed_for_a_move_is_written_over_by_its_new_holder() {
    let h = home();
    let folder = "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\n";
    listing(&h, folder, &["a", "b"]);
    let fake = Fake::default()
        .probe("a.flac", &tagged_flac("Kite", 1, "Record", false))
        .probe("b.flac", &tagged_flac("Tide", 2, "Record", false));
    assert!(h.run(&fake, Options::default()).0);
    let cover = "Artist/Record/cover.jpg";
    assert!(h.lib(cover).exists());
    // The song holding the cover is dropped, and the other takes its path.
    h.songs(&format!(
        "{folder}[[song]]\nsources = [\"manual:b.flac\"]\ntags = {{ title = \"Kite\", track = \"1\" }}\n"
    ));
    let said = h.status(&fake, &[], false);
    let (ok, done) = h.run(&fake, Options::default());
    assert!(ok, "{done}");
    assert!(said.contains("Would move: "), "{said}");
    for text in [&said, &done] {
        assert!(!text.contains(&format!("Removed: {cover}")), "{text}");
        assert!(!text.contains(&format!("remove: {cover}")), "{text}");
        assert!(!text.contains("not muman's"), "{text}");
    }
    let state = State::load(&h.dirs.home).unwrap();
    let holders: Vec<&PathBuf> = (state.outputs.iter())
        .filter(|(_, w)| w.sidecar_paths().any(|s| s == Path::new(cover)))
        .map(|(p, _)| p)
        .collect();
    assert_eq!(
        holders,
        [&PathBuf::from("Artist/Record/01 Kite.flac")],
        "{done}"
    );
    h.audit(&[], &done);
}

#[test]
fn a_song_moved_within_its_folder_moves_not_the_folder_s_cover_and_is_undone() {
    let h = home();
    let folder = "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\n";
    listing(&h, folder, &["a"]);
    let fake = Fake::default().probe("a.flac", &tagged_flac("Kite", 1, "Record", false));
    assert!(h.run(&fake, Options::default()).0);
    let cover = "Artist/Record/cover.jpg";
    assert!(h.lib(cover).exists());
    h.songs(&format!(
        "{folder}[[song]]\nsources = [\"manual:a.flac\"]\ntags = {{ title = \"Kite Rooms\" }}\n"
    ));
    let said = h.status(&fake, &[], false);
    let mut run = Run::begin(&h.dirs.home).unwrap();
    let mut out = Vec::new();
    let synced = reconcile(
        &fake,
        &h.dirs,
        Options::default(),
        Recording::Run(&mut run),
        &mut out,
    );
    run.finish(&h.dirs.home).unwrap();
    let done = String::from_utf8(out).unwrap();
    assert!(synced.unwrap().ok, "{done}");
    assert!(
        said.contains("Would move: Artist/Record/01 Kite.flac → "),
        "{said}"
    );
    assert!(
        done.contains("Moved: Artist/Record/01 Kite.flac → "),
        "{done}"
    );
    for text in [&said, &done] {
        assert!(!text.contains(&format!("{cover} → ")), "{text}");
    }
    assert!(h.lib(cover).exists(), "{done}");
    h.audit(&[], &done);
    let plan = crate::history::plan(&h.dirs).unwrap();
    let lines = plan.lines().to_vec();
    crate::history::undo(&h.dirs, plan, &mut Vec::new())
        .unwrap()
        .finish()
        .unwrap();
    assert!(h.lib("Artist/Record/01 Kite.flac").exists(), "{lines:?}");
    assert!(h.lib(cover).exists(), "{lines:?}");
}

#[test]
fn a_forced_sync_writes_again_a_folder_s_cover_changed_by_hand() {
    let h = home();
    let folder = "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\n";
    listing(&h, folder, &["a"]);
    let fake = Fake::default().probe("a.flac", &tagged_flac("Kite", 1, "Record", false));
    assert!(h.run(&fake, Options::default()).0);
    let cover = h.lib("Artist/Record/cover.jpg");
    let written = std::fs::read(&cover).unwrap();
    std::fs::write(&cover, "a picture of my own").unwrap();
    let forced = Options {
        force: true,
        ..Options::default()
    };
    let (ok, text) = h.run(&fake, forced);
    assert!(ok, "{text}");
    assert!(!text.contains("not muman's"), "{text}");
    assert_eq!(std::fs::read(&cover).unwrap(), written, "{text}");
    h.audit(&[], &text);
}

/// 8 kHz samples whose loudness follows `levels`, one per 10 ms.
fn pcm(levels: &[f64]) -> Vec<u8> {
    levels
        .iter()
        .flat_map(|l| {
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
    // A release rises from quiet and falls back to it; one at full sound on
    // its first frame, the intro running into it, would read as cut off.
    let mut song = vec![0.0; 10];
    song.extend(levels(6000, 7));
    song.extend([0.0; 10]);
    let mut video = levels(1000, 99);
    video.extend([0.0; 10]);
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
    assert_eq!(
        lyrics.shift_ms, 10_100,
        "the video's 10 s intro and its rest"
    );
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
    let (ok, text) = h.run_plain(&infos(&["aaaaaaaaaaa"]), Options::default());
    assert!(ok, "{text}");
    let key = SourceKey::youtube("aaaaaaaaaaa");
    let mut state = State::load(&h.dirs.home).unwrap();
    let facts = state.facts.get_mut(&key).unwrap();
    let tags = std::mem::take(&mut facts.tags);
    facts.tags_method = String::new();
    state.save(&h.dirs.home).unwrap();

    let again = infos(&["aaaaaaaaaaa"]);
    let (ok, text) = h.run_plain(&again, Options::default());
    assert!(ok, "{text}");
    assert!(text.contains("Reading the tags of 1 source(s)"), "{text}");
    assert!(!again.ran("s16le"), "nothing is measured again");
    assert!(text.contains("Up to date: 1 song(s)"), "{text}");
    let state = State::load(&h.dirs.home).unwrap();
    assert_eq!(state.facts[&key].tags, tags);
    assert!(state.facts[&key].tags_hold(crate::store::Kind::Media));
}

#[test]
fn a_record_read_before_its_length_was_kept_is_read_again_alone() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    let id = "00000000-0000-0000-0000-000000000001";
    std::fs::create_dir_all(h.dirs.musicbrainz()).unwrap();
    std::fs::write(
        h.dirs.musicbrainz().join(format!("{id}.json")),
        format!(
            r#"{{"id": "{id}", "title": "Song", "artists": ["Hoshi7ne"], "length_ms": 241000}}"#
        ),
    )
    .unwrap();
    h.songs(&format!(
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\", \"musicbrainz:{id}\"]\n"
    ));
    let (ok, text) = h.run_plain(&infos(&["aaaaaaaaaaa"]), Options::default());
    assert!(ok, "{text}");
    let key = SourceKey::parse(&format!("musicbrainz:{id}")).unwrap();
    let mut state = State::load(&h.dirs.home).unwrap();
    assert_eq!(state.facts[&key].length_ms, Some(241_000));
    let facts = state.facts.get_mut(&key).unwrap();
    facts.tags_method = crate::tags::METHOD.to_string();
    facts.length_ms = None;
    state.save(&h.dirs.home).unwrap();

    let again = infos(&["aaaaaaaaaaa"]);
    let (ok, text) = h.run_plain(&again, Options::default());
    assert!(ok, "{text}");
    assert!(text.contains("Reading the tags of 1 source(s)"), "{text}");
    let state = State::load(&h.dirs.home).unwrap();
    assert_eq!(state.facts[&key].length_ms, Some(241_000));
    assert!(state.facts[&key].tags_hold(crate::store::Kind::Tags));
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
fn a_file_retagged_by_hand_keeping_its_size_and_time_is_left_alone() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    let a = h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus");
    // A tagger set to keep files' times, its edit inside the tag's padding.
    let mut edited = std::fs::read(&a).unwrap();
    let at = edited.len() - 1;
    edited[at] ^= 0xff;
    let time = std::fs::metadata(&a).unwrap().modified().unwrap();
    std::fs::write(&a, &edited).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&a)
        .unwrap()
        .set_modified(time)
        .unwrap();
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { genre = \"Pop\" }\n");
    let (_, text) = h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    assert_eq!(std::fs::read(&a).unwrap(), edited, "{text}");
}

#[test]
fn a_run_stopped_while_writing_leaves_each_file_vouched_for_or_claimed() {
    let prepare = || {
        let h = home();
        h.fetched("aaaaaaaaaaa");
        h.fetched("bbbbbbbbbbb");
        h.songs(TWO);
        h.run(&infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]), Options::default());
        h
    };
    let every_song = Options {
        force: true,
        ..Options::default()
    };
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let whole = prepare();
    let counted = crate::kill::kill_at(whole.dirs.home.parent().unwrap(), usize::MAX);
    whole.run(&fake, every_song);
    let steps = counted.seen();
    drop(counted);
    assert!(steps > 2, "{steps}");
    for at in 1..=steps {
        let h = prepare();
        let kill = crate::kill::kill_at(h.dirs.home.parent().unwrap(), at);
        let stopped =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.run(&fake, every_song)));
        drop(kill);
        assert!(stopped.is_err(), "not stopped at step {at} of {steps}");
        let state = State::load(&h.dirs.home).unwrap();
        for (path, written) in &state.outputs {
            assert!(
                (written.plan.is_some()
                    && !changed_since_written(&h.dirs.library, &state.outputs, path))
                    || (written.plan.is_none() && written.stamp.is_none()),
                "step {at}: {} is neither vouched for nor claimed",
                path.display()
            );
        }
        let (ok, text) = h.run(&fake, Options::default());
        assert!(ok, "step {at}: {text}");
        assert!(
            !text.contains("changed since muman wrote it"),
            "step {at}: {text}"
        );
    }
}

#[test]
fn a_song_whose_write_failed_keeps_its_record_as_it_was() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    h.run_plain(&infos(&["aaaaaaaaaaa"]), Options::default());
    let before = State::load(&h.dirs.home).unwrap().outputs;
    let mut failing = infos(&["aaaaaaaaaaa"]);
    failing.failing = vec!["opus".into()];
    let forced = Options {
        force: true,
        ..Options::default()
    };
    let (ok, text) = h.run_plain(&failing, forced);
    assert!(!ok && text.contains("Failed"), "{text}");
    let after = State::load(&h.dirs.home).unwrap().outputs;
    for (path, written) in &after {
        assert!(
            written.plan.is_some(),
            "{} lost its plan: {text}",
            path.display()
        );
    }
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>()
    );
}

/// A runner that panics building the song at a path holding a fragment,
/// as a crate's bug would, and runs every other command as `Fake` does.
struct Panicking<'a>(&'a Fake, &'a str);

impl crate::runner::Runner for Panicking<'_> {
    fn run(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<()> {
        let args: Vec<String> = cmd
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let building = |a: &String| a.contains(&format!("{}.opus.part", self.1));
        assert!(!args.iter().any(building), "a width past every name");
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
        self.0.stream(cmd, on_line)
    }

    fn pipe(
        &self,
        cmd: &[std::ffi::OsString],
        on_bytes: &mut dyn FnMut(&[u8]),
    ) -> anyhow::Result<()> {
        self.0.pipe(cmd, on_bytes)
    }

    fn fingerprint(&self, pcm: &Path) -> anyhow::Result<Vec<u32>> {
        self.0.fingerprint(pcm)
    }
}

#[test]
fn a_panic_building_one_song_fails_that_song_alone() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let mut out = Vec::new();
    let runner = Panicking(&fake, "bbbbbbbbbbb");
    let ok = reconcile(
        &runner,
        &h.dirs,
        Options::default(),
        Recording::Test,
        &mut out,
    )
    .unwrap()
    .ok;
    let text = String::from_utf8(out).unwrap();
    h.holds(&text);
    assert!(!ok, "{text}");
    assert!(
        text.contains("Failed: ") && text.contains("a width past every name (a panic at "),
        "the panic is said in the song's failure: {text}"
    );
    assert!(
        h.lib("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus")
            .exists(),
        "{text}"
    );
    h.audit(&[], &text);
    let failures = State::load(&h.dirs.home).unwrap().song_failures;
    assert_eq!(failures.len(), 1, "check names the song: {failures:?}");
    assert!(
        failures
            .values()
            .all(|f| f.error.contains("a width past every name")),
        "{failures:?}"
    );
    let (ok, text) = h.run(&fake, Options::default());
    assert!(
        ok && text.contains("Added: "),
        "the next sync writes it: {text}"
    );
}

/// A runner that, once it has made the file of the song at a path holding
/// a fragment, puts a folder in the way of that song's file, as a player
/// holding the file open on Windows keeps it from being replaced; and
/// runs every command as `Fake` does.
struct Holding<'a>(&'a Fake, &'a str);

impl crate::runner::Runner for Holding<'_> {
    fn run(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<()> {
        self.0.run(cmd)?;
        for arg in cmd.iter().map(|a| a.to_string_lossy()) {
            let Some(song) = arg.strip_suffix(".opus.part") else {
                continue;
            };
            let song = PathBuf::from(format!("{song}.opus"));
            if song.is_file() && arg.contains(self.1) {
                std::fs::remove_file(&song).unwrap();
                std::fs::create_dir_all(song.join("held")).unwrap();
            }
        }
        Ok(())
    }

    fn output(&self, cmd: &[std::ffi::OsString]) -> anyhow::Result<Vec<u8>> {
        self.0.output(cmd)
    }

    fn stream(
        &self,
        cmd: &[std::ffi::OsString],
        on_line: &mut dyn FnMut(crate::runner::Line<'_>),
    ) -> anyhow::Result<bool> {
        self.0.stream(cmd, on_line)
    }

    fn pipe(
        &self,
        cmd: &[std::ffi::OsString],
        on_bytes: &mut dyn FnMut(&[u8]),
    ) -> anyhow::Result<()> {
        self.0.pipe(cmd, on_bytes)
    }

    fn fingerprint(&self, pcm: &Path) -> anyhow::Result<Vec<u32>> {
        self.0.fingerprint(pcm)
    }
}

#[test]
fn a_song_written_in_part_stays_muman_s_and_is_written_whole_next() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    let rel = Path::new("Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus");
    let (song, lrc) = (
        h.dirs.library.join(rel),
        h.dirs.library.join(rel.with_extension("lrc")),
    );
    let lyrics = std::fs::read(&lrc).unwrap();
    std::fs::write(&lrc, b"[00:01.00]Older words\n").unwrap();
    let forced = Options {
        force: true,
        ..Options::default()
    };
    let mut out = Vec::new();
    let runner = Holding(&fake, "aaaaaaaaaaa");
    let ok = reconcile(&runner, &h.dirs, forced, Recording::Test, &mut out)
        .unwrap()
        .ok;
    let text = String::from_utf8(out).unwrap();
    h.holds(&text);
    assert!(!ok && text.contains("left half written"), "{text}");
    assert_eq!(std::fs::read(&lrc).unwrap(), lyrics, "its lyrics in place");
    let written = State::load(&h.dirs.home).unwrap().outputs[rel].clone();
    assert!(
        written.plan.is_none() && written.stamp.is_none(),
        "recorded as the run recorded it before writing any: {written:?}"
    );
    let said = crate::diagnose::diagnose(
        &crate::testing::Fake::default(),
        &h.dirs,
        std::time::Duration::ZERO,
    )
    .unwrap()
    .said;
    assert!(
        !said
            .iter()
            .any(|s| format!("{s:?}").contains("Changed since")),
        "{said:?}"
    );
    std::fs::remove_dir_all(&song).unwrap();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(
        !text.contains("changed since muman wrote it") && !text.contains("not muman's"),
        "{text}"
    );
    assert!(song.is_file() && lrc.is_file(), "{text}");
}

#[test]
fn a_song_moving_where_a_file_cannot_be_freed_stays_where_it_is() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    h.songs(
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { title = \"Same\" }\n\
         [[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { title = \"Same\" }\n",
    );
    assert!(h.run_plain(&fake, Options::default()).0);
    // A file that will not go, as one Windows holds open: a folder at the
    // name, recorded as the file muman wrote there.
    let plain = Path::new("Chan/Same/Same.opus");
    std::fs::remove_file(h.dirs.library.join(plain)).unwrap();
    std::fs::create_dir_all(h.dirs.library.join(plain).join("held")).unwrap();
    let mut state = State::load(&h.dirs.home).unwrap();
    let written = state.outputs.get_mut(plain).unwrap();
    written.stamp = crate::store::stamp_text(&h.dirs.library.join(plain));
    written.digest = None;
    state.save(&h.dirs.home).unwrap();
    h.songs(
        "[[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { title = \"Same\" }\n\
         [[removed]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\nnote = \"Same\"\n",
    );
    let told_apart = h.lib("Chan/Same/Same [bbbbbbbbbbb].opus");
    let bytes = std::fs::read(&told_apart).unwrap();
    let (_, text) = h.run_plain(&fake, Options::default());
    assert_eq!(
        text.matches("Could not remove").count(),
        1,
        "said once: {text}"
    );
    assert!(
        !text.contains("Failed"),
        "nothing written where it cannot be: {text}"
    );
    assert_eq!(std::fs::read(&told_apart).unwrap(), bytes, "{text}");
    assert!(told_apart.with_extension("lrc").is_file(), "{text}");
    assert!(!h.lib("Chan/Same/Same.lrc").exists(), "{text}");

    std::fs::remove_dir_all(h.dirs.library.join(plain)).unwrap();
    let (ok, text) = h.run_plain(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(
        text.contains("Moved: Chan/Same/Same [bbbbbbbbbbb].opus → Chan/Same/Same.opus"),
        "{text}"
    );
}

#[test]
fn a_move_in_case_alone_stopped_halfway_is_muman_s_and_finished() {
    let fake = infos(&["aaaaaaaaaaa"]);
    let titled = |title: &str| {
        format!(
            "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = {{ title = \"{title}\" }}\n"
        )
    };
    let prepare = || {
        let h = home();
        h.fetched("aaaaaaaaaaa");
        h.songs(&titled("lantern"));
        assert!(h.run(&fake, Options::default()).0);
        h.songs(&titled("Lantern"));
        h
    };
    // As a sync runs, recording what it does: the record names each move.
    let recorded = |h: &Home| {
        let mut run = Run::begin(&h.dirs.home).unwrap();
        let mut out = Vec::new();
        let synced = reconcile(
            &fake,
            &h.dirs,
            Options::default(),
            Recording::Run(&mut run),
            &mut out,
        );
        run.finish(&h.dirs.home).unwrap();
        let text = String::from_utf8(out).unwrap();
        h.holds(&text);
        (synced.unwrap().ok, text)
    };
    let whole = prepare();
    let counted = crate::kill::kill_at(whole.dirs.home.parent().unwrap(), usize::MAX);
    recorded(&whole);
    let steps = counted.seen();
    drop(counted);
    let mut halfway = 0;
    for at in 1..=steps {
        let h = prepare();
        let kill = crate::kill::kill_at(h.dirs.home.parent().unwrap(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| recorded(&h)));
        drop(kill);
        assert!(stopped.is_err(), "not stopped at step {at} of {steps}");
        let left: Vec<PathBuf> = files(&h.dirs.library)
            .into_iter()
            .filter(|f| f.to_string_lossy().ends_with(".moving"))
            .map(|f| f.strip_prefix(&h.dirs.library).unwrap().to_path_buf())
            .collect();
        let said: Vec<String> = (crate::diagnose::diagnose(
            &crate::testing::Fake::default(),
            &h.dirs,
            std::time::Duration::ZERO,
        )
        .unwrap()
        .said
        .iter())
        .map(|s| format!("{s:?}"))
        .collect();
        assert!(
            !said
                .iter()
                .any(|s| s.contains("Not muman's") && s.contains(".moving")),
            "step {at}: {said:?}"
        );
        for moving in &left {
            halfway += 1;
            let line = format!(
                "Left by an interrupted run: {}",
                crate::relpath::show(moving)
            );
            assert!(
                said.iter().any(|s| s.contains(&line)),
                "step {at}: {said:?}"
            );
        }
        // The next sync finishes the move first: nothing is missing, and
        // `status` says what it does with each file.
        assert!(
            !said.iter().any(|s| s.contains("Missing")),
            "step {at}: {said:?}"
        );
        let dry = Options {
            dry_run: true,
            ..Options::default()
        };
        let sink = &mut std::io::sink();
        let foretold = (reconcile(&fake, &h.dirs, dry, Recording::DryRun, sink).unwrap())
            .foretold
            .unwrap();
        let before = stamped(&h.dirs.library);
        let (ok, text) = recorded(&h);
        if let Err(e) = as_foretold(&foretold, &before, &stamped(&h.dirs.library)) {
            panic!("step {at}: {e}: {text}");
        }
        assert!(ok, "step {at}: {text}");
        assert!(
            !text.contains("Added: "),
            "step {at}: the move is finished: {text}"
        );
        h.audit(&[], &text);
    }
    assert!(halfway > 0, "no step of {steps} stopped a move halfway");
}

#[test]
fn a_move_stopped_anywhere_after_a_copy_gave_the_library_new_times_is_finished() {
    a_move_stopped_anywhere_is_finished(&|h: &mut Home| copied_without_times(h));
}

#[test]
fn a_move_stopped_anywhere_going_on_in_a_run_stopped_before_it_is_finished() {
    a_move_stopped_anywhere_is_finished(&|h: &mut Home| {
        copied_without_times(h);
        // Stopped before it changed anything, its record the next run's to go on in.
        drop(Run::begin(&h.dirs.home).unwrap());
    });
}

#[test]
fn a_move_stopped_anywhere_in_a_library_folder_moved_is_finished() {
    a_move_stopped_anywhere_is_finished(&|h: &mut Home| {
        let moved = h.dirs.library.with_file_name("moved");
        std::fs::rename(&h.dirs.library, &moved).unwrap();
        h.dirs.library = moved;
    });
}

/// Give every file in the library a new time, as a copy made without
/// its times, or a backup put back, does.
fn copied_without_times(h: &Home) {
    let then = filetime::FileTime::from_unix_time(1_000_000_000, 0);
    for file in files(&h.dirs.library) {
        filetime::set_file_mtime(file, then).unwrap();
    }
}

/// A song retitled after `found` changed how its library is found, its
/// file and lyrics moved by a sync killed at each step: the next sync
/// follows each move the stopped one made, and finishes the song where
/// it went, every file muman's.
fn a_move_stopped_anywhere_is_finished(found: &dyn Fn(&mut Home)) {
    let fake = infos(&["aaaaaaaaaaa"]);
    let titled = |title: &str| {
        format!(
            "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = {{ title = \"{title}\" }}\n"
        )
    };
    let prepare = || {
        let mut h = home();
        h.fetched("aaaaaaaaaaa");
        h.songs(&titled("Lantern"));
        assert!(h.run(&fake, Options::default()).0);
        found(&mut h);
        h.songs(&titled("Lantern Rooms"));
        h
    };
    let recorded = |h: &Home| {
        let mut run = Run::begin(&h.dirs.home).unwrap();
        let mut out = Vec::new();
        let synced = reconcile(
            &fake,
            &h.dirs,
            Options::default(),
            Recording::Run(&mut run),
            &mut out,
        );
        run.finish(&h.dirs.home).unwrap();
        let text = String::from_utf8(out).unwrap();
        h.holds(&text);
        (synced.unwrap().ok, text)
    };
    let whole = prepare();
    let counted = crate::kill::kill_at(whole.dirs.home.parent().unwrap(), usize::MAX);
    let (_, text) = recorded(&whole);
    let steps = counted.seen();
    drop(counted);
    assert!(text.contains("Moved: "), "{text}");
    for at in 1..=steps {
        let h = prepare();
        let kill = crate::kill::kill_at(h.dirs.home.parent().unwrap(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| recorded(&h)));
        drop(kill);
        assert!(stopped.is_err(), "not stopped at step {at} of {steps}");
        let said: Vec<String> =
            (crate::diagnose::diagnose(&Fake::default(), &h.dirs, Duration::ZERO)
                .unwrap()
                .said
                .iter())
            .map(|s| format!("{s:?}"))
            .collect();
        assert!(
            !said.iter().any(|s| s.contains("Not muman's")),
            "step {at}: {said:?}"
        );
        let (ok, text) = recorded(&h);
        assert!(ok && !text.contains("not muman's"), "step {at}: {text}");
        for file in ["Lantern Rooms.opus", "Lantern Rooms.lrc"] {
            let at_last = h.lib(&format!("Chan/Lantern Rooms/{file}"));
            assert!(at_last.exists(), "step {at}: {file}: {text}");
        }
        h.audit(&[], &text);
        let (ok, again) = recorded(&h);
        assert!(ok && again.contains("Up to date: 1"), "step {at}: {again}");
        let plan = crate::history::plan(&h.dirs).unwrap();
        let lines = plan.lines().to_vec();
        crate::history::undo(&h.dirs, plan, &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        for file in ["Lantern.opus", "Lantern.lrc"] {
            let back = h.lib(&format!("Chan/Lantern/{file}"));
            assert!(back.exists(), "step {at}: {file}: {lines:?}");
        }
        let said: Vec<String> =
            (crate::diagnose::diagnose(&Fake::default(), &h.dirs, Duration::ZERO)
                .unwrap()
                .said
                .iter())
            .map(|s| format!("{s:?}"))
            .collect();
        assert!(
            !said
                .iter()
                .any(|s| s.contains("Not muman's") || s.contains("changed")),
            "step {at}: {said:?}"
        );
    }
}

#[test]
fn a_file_of_yours_named_as_a_song_s_part_is_left_alone() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    assert!(h.run(&fake, Options::default()).0);
    let song = "Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa";
    let mine = [format!("{song}.opus.part"), format!("{song}.lrc.part")];
    for part in &mine {
        std::fs::write(h.lib(part), "my notes").unwrap();
    }
    let said: Vec<String> = (crate::diagnose::diagnose(
        &crate::testing::Fake::default(),
        &h.dirs,
        std::time::Duration::ZERO,
    )
    .unwrap()
    .said
    .iter())
    .map(|s| format!("{s:?}"))
    .collect();
    for part in &mine {
        let line = format!("Not muman's, left alone: {part}");
        assert!(said.iter().any(|s| s.contains(&line)), "{said:?}");
    }
    assert!(!said.iter().any(|s| s.contains("interrupted")), "{said:?}");
    let sync = |fake: &Fake| {
        let mut out = Vec::new();
        let ok = reconcile(fake, &h.dirs, Options::default(), Recording::Test, &mut out)
            .unwrap()
            .ok;
        (ok, String::from_utf8(out).unwrap())
    };
    let (ok, text) = sync(&fake);
    assert!(ok, "{text}");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { genre = \"Folk\" }\n");
    let (ok, text) = sync(&fake);
    assert!(
        !ok && text.contains("is not muman's, and is in the way of writing it"),
        "{text}"
    );
    for part in &mine {
        assert_eq!(
            std::fs::read_to_string(h.lib(part)).unwrap(),
            "my notes",
            "{text}"
        );
    }
}

#[test]
fn a_run_stopped_freeing_a_path_for_a_move_is_undone_whole_and_said_once() {
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let prepare = || {
        let h = home();
        h.fetched("aaaaaaaaaaa");
        h.fetched("bbbbbbbbbbb");
        h.songs(
            "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { title = \"Same\" }\n\
             [[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { title = \"Same\" }\n",
        );
        assert!(h.run(&fake, Options::default()).0);
        let before: Vec<(PathBuf, Vec<u8>)> = files(&h.lib(""))
            .into_iter()
            .map(|f| (f.clone(), std::fs::read(f).unwrap()))
            .collect();
        h.songs(
            "[[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { title = \"Same\" }\n\
             [[removed]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\nnote = \"Same\"\n",
        );
        (h, before)
    };
    let recorded = |h: &Home| {
        let mut run = Run::begin(&h.dirs.home).unwrap();
        let mut out = Vec::new();
        let synced = reconcile(
            &fake,
            &h.dirs,
            Options::default(),
            Recording::Run(&mut run),
            &mut out,
        );
        run.finish(&h.dirs.home).unwrap();
        (synced.unwrap().ok, String::from_utf8(out).unwrap())
    };
    let (whole, _) = prepare();
    let counted = crate::kill::kill_at(whole.dirs.home.parent().unwrap(), usize::MAX);
    recorded(&whole);
    let steps = counted.seen();
    drop(counted);
    for at in 1..=steps {
        let (h, before) = prepare();
        let kill = crate::kill::kill_at(h.dirs.home.parent().unwrap(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| recorded(&h)));
        drop(kill);
        if stopped.is_ok() {
            continue;
        }
        let (ok, text) = recorded(&h);
        assert!(ok, "step {at}: {text}");
        let plan = crate::history::plan(&h.dirs).unwrap();
        let lines = plan.lines().to_vec();
        let mut named = BTreeSet::new();
        for line in lines.iter().filter_map(|l| l.trim().split_once(": ")) {
            assert!(named.insert(line.0.to_string()), "step {at}: {lines:?}");
        }
        crate::history::undo(&h.dirs, plan, &mut Vec::new())
            .unwrap()
            .finish()
            .unwrap();
        let after: Vec<(PathBuf, Vec<u8>)> = files(&h.lib(""))
            .into_iter()
            .map(|f| (f.clone(), std::fs::read(f).unwrap()))
            .collect();
        assert_eq!(after, before, "step {at}: {lines:?}");
    }
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
    let (ok, text) = h.run_plain(&broken, Options::default());
    assert!(!ok);
    assert!(
        text.contains("Could not read youtube.com:aaaaaaaaaaa"),
        "{text}"
    );
    let broken = infos(&["aaaaaaaaaaa"]).probe("aaaaaaaaaaa", "not json");
    let (_, text) = h.run_plain(&broken, Options::default());
    assert!(text.contains("Not reading 1 source(s)"), "{text}");
    assert!(broken.calls().is_empty(), "{:?}", broken.calls());
    let retry = Options {
        retry: true,
        ..Options::default()
    };
    let broken = infos(&["aaaaaaaaaaa"]).probe("aaaaaaaaaaa", "not json");
    h.run_plain(&broken, retry);
    assert_ne!(broken.calls(), Vec::<Vec<String>>::new());
    std::fs::write(&path, "changed").unwrap();
    let fixed = infos(&["aaaaaaaaaaa"]);
    let (ok, text) = h.run_plain(&fixed, Options::default());
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
    let (ok, text) = h.run_plain(&fake, Options::default());
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
    let (ok, text) = h.run_plain(&fake, Options::default());
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
    let _ = reconcile(
        &infos(&["aaaaaaaaaaa"]),
        &h.dirs,
        Options::default(),
        Recording::Run(&mut run),
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
fn a_file_and_a_folder_of_one_name_trade_places_by_moves_alone() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let with = |other: &str| {
        format!(
            "[library]\ntemplate = \"{{% if id == 'aaaaaaaaaaa' %}}Y{{% else %}}{other}t{{% \
             endif %}}\"\n{TWO}"
        )
    };
    h.songs(&with(""));
    assert!(h.run(&fake, Options::default()).0);
    let dry = Options {
        dry_run: true,
        ..Options::default()
    };
    for (other, a, b) in [
        ("Y.opus/", "Y [aaaaaaaaaaa].opus", "Y.opus/t.opus"),
        ("", "Y.opus", "t.opus"),
    ] {
        h.songs(&with(other));
        let (_, said) = h.run(&fake, dry);
        let (ok, text) = h.run(&fake, Options::default());
        assert!(ok, "{text}");
        assert!(crate::ui::plain(&said).contains("2 moved"), "{said}");
        assert!(
            !text.contains("Wrote") && !text.contains("Could not"),
            "{text}"
        );
        assert!(h.lib(a).is_file() && h.lib(b).is_file(), "{text}");
    }
}

#[test]
fn a_dry_run_names_every_move_and_removal_its_sync_makes() {
    let h = home();
    let ids = ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc"];
    for id in ids {
        h.fetched(id);
    }
    h.songs(&format!(
        "{TWO}[[song]]\nsources = [\"youtube.com:ccccccccccc\"]\n"
    ));
    assert!(h.run(&infos(&ids), Options::default()).0);
    // One song takes the path another leaves, and a third is dropped.
    h.songs(&format!(
        "[library]\ntemplate = \"{{% if id == 'aaaaaaaaaaa' %}}Chan/Title bbbbbbbbbbb/Title \
         bbbbbbbbbbb{{% else %}}moved/{{{{ title }}}}{{% endif %}}\"\n{TWO}"
    ));
    let dry = Options {
        dry_run: true,
        ..Options::default()
    };
    let (_, said) = h.run(&infos(&ids), dry);
    let (ok, done) = h.run(&infos(&ids), Options::default());
    assert!(ok, "{done}");
    let after = |text: &str, lead: &str| -> BTreeSet<String> {
        text.lines()
            .filter_map(|l| Some(l.split_once(lead)?.1.trim().to_string()))
            .collect()
    };
    assert_eq!(
        after(&said, "Would move: "),
        after(&done, "Moved: "),
        "{said}\n{done}"
    );
    assert_eq!(after(&said, "Would move: ").len(), 2, "{said}");
    assert_eq!(
        after(&said, "Would remove: "),
        after(&done, "Removed: "),
        "{said}\n{done}"
    );
    assert_eq!(
        after(&done, "Removed: ").len(),
        2,
        "its song and lyrics: {done}"
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
fn a_song_takes_the_plain_name_a_song_removed_leaves_by_a_move_undone_whole() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let same = "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { title = \"Same\" }\n\
                [[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { title = \"Same\" }\n";
    h.songs(same);
    assert!(h.run(&fake, Options::default()).0);
    let before = files(&h.lib(""))
        .into_iter()
        .map(|f| (f.clone(), std::fs::read(f).unwrap()))
        .collect::<Vec<_>>();
    let told_apart = h.lib("Chan/Same/Same [bbbbbbbbbbb].opus");
    let bytes = std::fs::read(&told_apart).unwrap();
    h.songs(
        "[[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { title = \"Same\" }\n\
         [[removed]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\nnote = \"Same\"\n",
    );
    let dry = Options {
        dry_run: true,
        ..Options::default()
    };
    let (_, said) = h.run(&fake, dry);
    assert!(
        said.contains("Would move: Chan/Same/Same [bbbbbbbbbbb].opus → Chan/Same/Same.opus"),
        "{said}"
    );
    let renders_before = renders(&fake);
    let mut run = Run::begin(&h.dirs.home).unwrap();
    let mut out = Vec::new();
    let synced = reconcile(
        &fake,
        &h.dirs,
        Options::default(),
        Recording::Run(&mut run),
        &mut out,
    )
    .unwrap();
    run.finish(&h.dirs.home).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(synced.ok, "{text}");
    h.holds(&text);
    assert_eq!(renders(&fake), renders_before, "nothing encoded: {text}");
    assert!(text.contains("Removed: Chan/Same/Same.opus"), "{text}");
    assert!(
        text.contains("Moved: Chan/Same/Same [bbbbbbbbbbb].opus → Chan/Same/Same.opus"),
        "{text}"
    );
    assert!(!told_apart.exists(), "{text}");
    assert_eq!(std::fs::read(h.lib("Chan/Same/Same.opus")).unwrap(), bytes);

    let plan = crate::history::plan(&h.dirs).unwrap();
    let mut out = Vec::new();
    crate::history::undo(&h.dirs, plan, &mut out)
        .unwrap()
        .finish()
        .unwrap();
    let after = files(&h.lib(""))
        .into_iter()
        .map(|f| (f.clone(), std::fs::read(f).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        after,
        before,
        "both back: {}",
        String::from_utf8_lossy(&out)
    );
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
#[test]
fn a_failed_song_s_file_kept_counts_once_against_max_size() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    let b = h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    let fake = infos(&["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    let kept = h.lib("Chan/Title bbbbbbbbbbb/Title bbbbbbbbbbb.opus");
    std::fs::write(&kept, vec![0; 900 * 1024]).unwrap();
    h.songs(&format!("[library]\nmax_size = \"1 MiB\"\n{TWO}"));
    std::fs::remove_file(&b).unwrap();
    let (ok, text) = h.run(&infos(&["aaaaaaaaaaa"]), Options::default());
    assert!(!ok && text.contains("Failed"), "{text}");
    assert!(!text.contains("over its max_size"), "{text}");
}

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
            .contains("Opus 256 kbit/s to fit max_size; at best FLAC, copied"),
        "{}",
        status.1
    );
    assert!(status.1.contains("(max_size)"), "{}", status.1);
    let fake = big_flac();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(fake.ran("libopus") && fake.ran("256k"), "{text}");
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
    assert!(
        text.contains("Opus goes no lower than 6 kbit/s") && !text.contains("min_bitrate"),
        "unset, min_bitrate is no remedy: {text}"
    );
    assert!(!h.lib("Artist/Record/02 Song.opus").exists());
    assert!(!h.lib("Artist/Record/02 Song.flac").exists());
    let failures = State::load(&h.dirs.home).unwrap().song_failures;
    assert_eq!(failures.len(), 1, "check names the song left out");
    assert!(
        failures.values().all(|f| f.error.contains("max_size")),
        "{failures:?}"
    );
}

/// Three manual FLACs a second long with no cover, whose audio takes 30,
/// 20 and 10 kB: estimates the size of what the fake writes.
fn three_flacs() -> Fake {
    let short = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "flac",
        "channels": 2, "sample_rate": "44100"}], "format": {"format_name": "flac", "duration": "1.0",
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
fn a_changed_file_forced_fits_as_the_next_sync_does() {
    let fresh = home();
    listing(&fresh, &limit(20_000), &["a", "b", "c"]);
    fresh.run(&three_flacs(), Options::default());
    let fitted = formats(&fresh);

    let h = home();
    listing(&h, &limit(20_000), &["a", "b", "c"]);
    h.run(&three_flacs(), Options::default());
    let (lowered, _) = formats(&h)
        .into_iter()
        .find(|(_, f)| f.is_encoded())
        .expect("the limit binds");
    std::fs::write(h.lib(&lowered.to_string_lossy()), "edited by hand").unwrap();
    let (_, text) = h.run(&three_flacs(), Options::default());
    assert!(text.contains("Left alone, changed since"), "{text}");
    let forced = Options {
        force: true,
        ..Options::default()
    };
    let (ok, text) = h.run(&three_flacs(), forced);
    assert!(ok, "{text}");
    assert_eq!(formats(&h), fitted, "{text}");
    let again = three_flacs();
    let (ok, text) = h.run(&again, Options::default());
    assert!(ok, "{text}");
    assert_eq!(renders(&again), 0, "nothing encoded again: {text}");
    assert_eq!(formats(&h), fitted, "{text}");
}

#[test]
fn a_song_left_as_built_counts_against_the_limit_as_its_file_is() {
    let h = home();
    listing(&h, "", &["a", "b"]);
    let (ok, text) = h.run(&three_flacs(), Options::default());
    assert!(ok, "{text}");
    let built = formats(&h);
    std::fs::write(h.dirs.manual().join("a.flac"), "a copy stopped short").unwrap();
    let cut = || {
        let mut fake = Fake {
            held: vec![("a.flac".into(), 3)],
            ..three_flacs()
        };
        let said = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "flac",
            "channels": 2, "sample_rate": "44100", "duration": "200.0"}], "format":
            {"format_name": "flac", "duration": "200.0", "tags": {"TITLE": "Song", "ARTIST": "Artist",
            "ALBUM": "Record", "track": "2"}}}"#;
        fake.probes.insert(0, ("a.flac".into(), said.into()));
        fake
    };
    let (ok, text) = h.run(&cut(), Options::default());
    assert!(!ok && text.contains("cut off"), "{text}");
    let (held, _) = built
        .iter()
        .find(|(p, _)| {
            let w = &State::load(&h.dirs.home).unwrap().outputs[p];
            w.sources.iter().any(|k| k.to_string().ends_with("a.flac"))
        })
        .cloned()
        .expect("a's file");
    let kept = std::fs::read(h.lib(&held.to_string_lossy())).unwrap();
    let whole: u64 = built
        .iter()
        .map(|(p, _)| {
            std::fs::metadata(h.lib(&p.to_string_lossy()))
                .unwrap()
                .len()
        })
        .sum();
    let max = whole - 2_000;
    let list = std::fs::read_to_string(h.dirs.manifest()).unwrap();
    let block = r#"block_size = "4 KiB""#;
    assert!(list.contains(block), "{list}");
    let limited = list.replacen(block, &format!("block_size = 1\nmax_size = {max}"), 1);
    std::fs::write(h.dirs.manifest(), limited).unwrap();
    let (_, text) = h.run(&cut(), Options::default());
    assert!(
        text.contains("1 song(s) below their best format"),
        "the song left as built is not counted lowered: {text}"
    );
    assert_eq!(
        std::fs::read(h.lib(&held.to_string_lossy())).unwrap(),
        kept,
        "{text}"
    );
    let fitted = formats(&h);
    assert!(
        fitted.iter().any(|(p, f)| *p != held && f.is_encoded()),
        "the song written is lowered for the one left as built: {fitted:?} {text}"
    );
    let total: u64 = fitted
        .iter()
        .map(|(p, _)| {
            std::fs::metadata(h.lib(&p.to_string_lossy()))
                .unwrap()
                .len()
        })
        .sum();
    assert!(total <= max, "{total} > {max}: {text}");
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
fn a_song_only_retagged_keeps_its_measured_size() {
    let settings = "[library]\nmax_size = \"10 MB\"\n[[lyrics.out]]\nto = \"tag\"\n";
    let h = flac_home(settings);
    std::fs::write(
        h.dirs.manual().join("Song.lrc"),
        "[00:01.00]Paper comets\n[00:02.00][Chorus]\n",
    )
    .unwrap();
    let (ok, text) = h.run(&big_flac(), Options::default());
    assert!(ok, "{text}");
    assert!(h.lib("Artist/Record/02 Song.opus").exists(), "{text}");
    h.songs(&format!(
        "{settings}[lyrics.clean.packaging]\ncues = false\n\
         [[song]]\nsources = [\"manual:Song.flac\"]\n"
    ));
    let fake = big_flac();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    let encodes = fake
        .calls()
        .iter()
        .filter(|c| c.iter().any(|a| a == "libopus"))
        .count();
    assert_eq!(encodes, 0, "measured as it was, and only tagged: {text}");
    assert!(h.lib("Artist/Record/02 Song.opus").exists(), "{text}");
}

#[test]
fn a_song_rendered_to_be_measured_is_moved_into_place_not_rendered_again() {
    let h = flac_home("[library]\nmax_size = \"10 MB\"\n");
    let fake = big_flac();
    let (ok, text) = h.run_plain(&fake, Options::default());
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
        sidecars: vec![crate::state::Sidecar::at(lyrics.into())],
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
            Pruned::Removed {
                gone: vec![PathBuf::from("A/x.opus")],
                left: Vec::new()
            }
        )]
    );
}

#[test]
fn a_forced_sync_leaves_a_changed_file_whose_song_shared_only_a_picture() {
    let h = home();
    std::fs::create_dir_all(h.lib("A")).unwrap();
    for file in ["A/kite.opus", "A/kite.lrc", "A/tide.opus"] {
        std::fs::write(h.lib(file), "audio").unwrap();
    }
    let key = |k: &str| SourceKey::parse(k).unwrap();
    // A stamp no file has: the file reads as changed since written.
    let written = |own: &str, stamp: &str| Written {
        sources: vec![key(own), key("manual:added/cover.jpg")],
        sidecars: Vec::new(),
        plan: None,
        stamp: Some(stamp.to_string()),
        digest: None,
    };
    let old = BTreeMap::from([
        (
            PathBuf::from("A/kite.opus"),
            written("manual:kite.flac", "0 0"),
        ),
        (
            PathBuf::from("A/tide.opus"),
            written("manual:tide.flac", "0 0"),
        ),
    ]);
    let mut now = BTreeMap::from([(
        PathBuf::from("A/tide.opus"),
        written("manual:tide.flac", "0 0"),
    )]);
    let forced = prune_plan(&h.dirs.library, &old, &now, &BTreeSet::new(), true);
    assert_eq!(
        forced,
        [(PathBuf::from("A/kite.opus"), Pruned::LeftChanged)]
    );
    // Its own source written elsewhere, the song's file it changed goes.
    now.insert(
        PathBuf::from("B/kite.opus"),
        written("manual:kite.flac", "0 0"),
    );
    let forced = prune_plan(&h.dirs.library, &old, &now, &BTreeSet::new(), true);
    assert!(
        matches!(&forced[..], [(_, Pruned::Removed { gone, .. })] if gone == &[PathBuf::from("A/kite.opus")]),
        "{forced:?}"
    );
}

#[test]
fn a_song_moving_into_a_folder_whose_cover_holder_leaves_carries_its_own_cover() {
    let h = home();
    let folder = "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\n";
    listing(&h, folder, &["a", "b", "c"]);
    let fake = Fake::default()
        .probe("a.flac", &tagged_flac("Kite", 1, "Record", false))
        .probe("b.flac", &tagged_flac("Tide", 2, "Record", false))
        .probe("c.flac", &tagged_flac("Moth", 1, "Other", false));
    assert!(h.run(&fake, Options::default()).0);
    assert!(h.lib("Artist/Record/cover.jpg").exists());
    assert!(h.lib("Artist/Other/cover.jpg").exists());
    // The song holding Record's cover leaves, and Other's only song takes its place.
    h.songs(&format!(
        "{folder}[[song]]\nsources = [\"manual:b.flac\"]\n\
         [[song]]\nsources = [\"manual:c.flac\"]\n\
         tags = {{ title = \"Kite\", track = \"1\", album = \"Record\" }}\n"
    ));
    let said = h.status(&fake, &[], false);
    let before = fake.calls().len();
    let (ok, done) = h.run(&fake, Options::default());
    assert!(ok, "{done}");
    let done = crate::ui::plain(&done);
    assert!(
        said.contains("Would move: Artist/Other/01 Moth.flac → Artist/Record/01 Kite.flac"),
        "{said}"
    );
    assert!(
        done.contains("Moved: Artist/Other/01 Moth.flac → Artist/Record/01 Kite.flac"),
        "{done}"
    );
    assert!(!done.contains("Could not move"), "{done}");
    assert!(
        !done.contains("Added: "),
        "moved, not written again: {done}"
    );
    let lines = |text: &str, lead: &str| -> BTreeSet<String> {
        text.lines()
            .filter_map(|l| Some(l.split_once(lead)?.1.trim().to_string()))
            .collect()
    };
    assert_eq!(
        lines(&said, "Would move: "),
        lines(&done, "Moved: "),
        "{said}\n{done}"
    );
    assert_eq!(
        lines(&said, "Would remove: "),
        lines(&done, "Removed: "),
        "{said}\n{done}"
    );
    assert!(
        !(fake.calls()[before..].iter()).any(|c| c.iter().any(|a| a.ends_with("c.flac"))),
        "the song moved is read from no source: {done}"
    );
    assert!(!h.lib("Artist/Other").exists(), "{done}");
    h.audit(&[], &done);
}

#[test]
fn a_song_whose_changed_folder_cover_stays_behind_writes_one_where_its_holder_left() {
    let h = home();
    let folder = "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\n";
    listing(&h, folder, &["a", "c"]);
    let fake = Fake::default()
        .probe("a.flac", &tagged_flac("Kite", 1, "Record", false))
        .probe("c.flac", &tagged_flac("Moth", 1, "Other", false));
    assert!(h.run(&fake, Options::default()).0);
    std::fs::write(h.lib("Artist/Other/cover.jpg"), "mine").unwrap();
    // The song holding Record's cover leaves, and Other's song moves in.
    h.songs(&format!(
        "{folder}[[song]]\nsources = [\"manual:c.flac\"]\ntags = {{ album = \"Record\" }}\n"
    ));
    let said = h.status(&fake, &[], false);
    assert!(
        !said.contains("Would remove: Artist/Record/cover.jpg"),
        "{said}"
    );
    assert!(
        said.contains("Would leave in place, changed since muman wrote it: Artist/Other/cover.jpg"),
        "{said}"
    );
    let (ok, done) = h.run(&fake, Options::default());
    assert!(ok, "{done}");
    assert!(h.lib("Artist/Record/cover.jpg").exists(), "{done}");
    assert_eq!(
        std::fs::read_to_string(h.lib("Artist/Other/cover.jpg")).unwrap(),
        "mine",
        "{done}"
    );
    h.audit(&["Artist/Other/cover.jpg"], &done);
    let said = crate::diagnose::diagnose(
        &crate::testing::Fake::default(),
        &h.dirs,
        std::time::Duration::ZERO,
    )
    .unwrap();
    assert_eq!(said.problems(), 0, "{:?}", said.said);
    let (_, again) = h.run(&fake, Options::default());
    assert!(again.contains("Up to date: 1 song(s)"), "{again}");
}

#[test]
fn songs_of_two_folders_moved_into_one_say_the_cover_one_of_them_leaves() {
    let h = home();
    let folder = "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\n";
    let album = |name: &str| {
        format!("[[song]]\nsources = [\"manual:{name}.flac\"]\ntags = {{ album = \"Harbor\" }}\n")
    };
    listing(&h, folder, &["a", "c"]);
    let fake = Fake::default()
        .probe("a.flac", &tagged_flac("Kite", 1, "Record", false))
        .probe("c.flac", &tagged_flac("Moth", 2, "Other", false));
    assert!(h.run(&fake, Options::default()).0);
    h.songs(&format!("{folder}{}{}", album("a"), album("c")));
    let said = h.status(&fake, &[], false);
    let (ok, done) = h.run(&fake, Options::default());
    assert!(ok, "{done}");
    let lines = |text: &str, lead: &str| -> BTreeSet<String> {
        text.lines()
            .filter_map(|l| Some(l.split_once(lead)?.1.trim().to_string()))
            .collect()
    };
    let done = crate::ui::plain(&done);
    assert_eq!(
        lines(&said, "Would remove: "),
        lines(&done, "Removed: "),
        "{said}\n{done}"
    );
    assert_eq!(lines(&said, "Would remove: ").len(), 1, "{said}");
    assert!(said.contains("moved, changes: folder cover"), "{said}");
    assert!(done.contains("Updated (folder cover"), "{done}");
    h.audit(&[], &done);
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
    assert!(!text.contains("Measuring"), "the same bytes: {text}");
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
    assert!(h.run_plain(&fake, Options::default()).0);
    let now = State::load(&h.dirs.home).unwrap();
    let key = SourceKey::youtube("aaaaaaaaaaa");
    let digest = now.facts[&key].audio_rev();
    assert!(!digest.contains(':'), "named by its content: {digest}");
    unhashed(now).save(&h.dirs.home).unwrap();
    let (ok, text) = h.run_plain(&fake, Options::default());
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
    assert!(h.run_plain(&big_flac(), Options::default()).0);
    let state = State::load(&h.dirs.home).unwrap();
    assert!(!state.sizes.is_empty(), "a real size is kept");
    let state_sizes = state.sizes.clone();
    unhashed(state).save(&h.dirs.home).unwrap();
    let fake = big_flac();
    let (ok, text) = h.run_plain(&fake, Options::default());
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
        Recording::DryRun,
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
/// plans so named: each song's file's, and those of the bitrates of its
/// codec fitting measured.
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
        let mut formats = vec![plan.format];
        if let crate::resolve::Format::Encode {
            codec, adapt, mix, ..
        } = plan.format
        {
            formats.extend(crate::fit::ladder(codec).iter().map(|k| {
                crate::resolve::Format::Encode {
                    codec,
                    kbps: Some(*k),
                    adapt,
                    mix,
                }
            }));
        }
        for plan in formats.into_iter().map(|f| plan.with_format(f)) {
            if let Some(m) = state.sizes.get(&limit::plan_key(tools, &plan)) {
                kept.insert(limit::plan_key(tools, &plan.respelled(&old)), m.clone());
            }
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
    crate::state::put_old_state(&h.dirs.home, old.as_bytes());

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
    crate::state::put_old_state(&h.dirs.home, old.as_bytes());

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
fn a_source_of_an_extractor_a_site_lists_reads_as_that_sites() {
    let h = home();
    h.fetched("1234567");
    h.songs("[[song]]\nsources = [\"jamendo.com:1234567\"]\n");
    let fake = || {
        Fake::default().info(
            "1234567",
            r#"{"id": "1234567", "title": "Title 1234567", "uploader": "Chan",
                "format_id": "mp31",
                "webpage_url": "https://licensing.jamendo.com/en/track/1234567/title"}"#,
        )
    };
    assert!(h.run(&fake(), Options::default()).0);
    let songs = std::fs::read_to_string(h.dirs.manifest()).unwrap();
    std::fs::write(h.dirs.manifest(), songs.replace("jamendo.com:", "jamendo:")).unwrap();
    let old = by_extractor(
        State::load(&h.dirs.home).unwrap(),
        &[("jamendo.com", "jamendo")],
    );
    crate::state::put_old_state(&h.dirs.home, old.as_bytes());

    let fake = fake();
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert_eq!(renders(&fake), 0, "{text}");
    let songs = std::fs::read_to_string(h.dirs.manifest()).unwrap();
    assert!(songs.contains("\"jamendo.com:1234567\""), "{songs}");
}

#[test]
fn a_source_of_a_sites_alias_is_renamed_by_that_site_and_its_file_moved() {
    let h = home();
    let old = h
        .dirs
        .ytdlp()
        .join("licensing.jamendo.com")
        .join("1234567.mkv");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, b"1234567").unwrap();
    h.songs("[[song]]\nsources = [\"licensing.jamendo.com:1234567\"]\n");
    let fake = Fake::default().info(
        "1234567",
        r#"{"id": "1234567", "title": "Title 1234567", "uploader": "Chan", "format_id": "mp31",
            "webpage_url": "https://licensing.jamendo.com/en/track/1234567/title"}"#,
    );
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(
        text.contains(
            "Renamed by the site it came from: licensing.jamendo.com:1234567 → jamendo.com:1234567"
        ),
        "{text}"
    );
    assert!(!old.exists(), "{text}");
    assert!(
        h.dirs
            .ytdlp()
            .join("jamendo.com")
            .join("1234567.mkv")
            .is_file()
    );
    let (ok, again) = h.run(&infos(&[]), Options::default());
    assert!(ok, "{again}");
    assert!(
        !again.contains("Renamed") && !again.contains("Failed"),
        "{again}"
    );
}

#[test]
fn a_site_rename_stopped_anywhere_ends_at_the_new_key() {
    let fake = Fake::default().info(
        "1234567",
        r#"{"id": "1234567", "title": "Title 1234567", "uploader": "Chan", "format_id": "mp31",
            "webpage_url": "https://licensing.jamendo.com/en/track/1234567/title"}"#,
    );
    let prepare = || {
        let h = home();
        let old = h
            .dirs
            .ytdlp()
            .join("licensing.jamendo.com")
            .join("1234567.mkv");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, b"1234567").unwrap();
        h.songs("[[song]]\nsources = [\"licensing.jamendo.com:1234567\"]\n");
        (h, old)
    };
    let (whole, _) = prepare();
    let counted = crate::kill::kill_at(whole.dirs.home.parent().unwrap(), usize::MAX);
    whole.run(&fake, Options::default());
    let steps = counted.seen();
    drop(counted);
    let (from, to) = (
        SourceKey::parse("licensing.jamendo.com:1234567").unwrap(),
        SourceKey::parse("jamendo.com:1234567").unwrap(),
    );
    for at in 1..=steps {
        let (h, old) = prepare();
        let kill = crate::kill::kill_at(h.dirs.home.parent().unwrap(), at);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            reconcile(
                &fake,
                &h.dirs,
                Options::default(),
                Recording::Test,
                &mut Vec::new(),
            )
        }));
        drop(kill);
        assert!(stopped.is_err(), "not stopped at step {at} of {steps}");
        let (ok, text) = h.run(&fake, Options::default());
        assert!(ok && !text.contains("Failed"), "step {at}: {text}");
        let new = h.dirs.ytdlp().join("jamendo.com").join("1234567.mkv");
        assert_eq!(std::fs::read(&new).unwrap(), b"1234567", "step {at}");
        assert_eq!(
            Manifest::load(&h.dirs.home).unwrap().keys(),
            [to.clone()].into()
        );
        let state = State::load(&h.dirs.home).unwrap();
        assert!(
            state.facts.contains_key(&to) && !state.facts.contains_key(&from),
            "step {at}"
        );
        assert!(
            !old.exists() || same_file::is_same_file(&old, &new).unwrap(),
            "step {at}: the old name, if left, is the same file"
        );
    }
}

#[test]
fn a_library_folder_left_keeps_its_records_while_away_and_the_last_few_only() {
    let dir = tempfile::tempdir().unwrap();
    let written = |stamp: u128| Written {
        sources: vec![],
        sidecars: Vec::new(),
        plan: None,
        stamp: Some(format!("1:{stamp}")),
        digest: None,
    };
    let one = |stamp: u128| BTreeMap::from([(PathBuf::from("A/x.flac"), written(stamp))]);
    let mut state = State {
        library: Some(dir.path().join("now")),
        outputs: one(100),
        ..State::default()
    };
    for n in 1..=LEFT_KEPT + 1 {
        let away = dir.path().join(format!("away{n}"));
        state.left.insert(away, one(n as u128));
    }
    let emptied = dir.path().join("emptied");
    std::fs::create_dir_all(emptied.join("B")).unwrap();
    std::fs::write(emptied.join("B").join("other.flac"), "the user's").unwrap();
    state.left.insert(emptied.clone(), one(99));
    // A share's mount point while it is not mounted, as a desktop shows it.
    let unmounted = dir.path().join("unmounted");
    std::fs::create_dir_all(unmounted.join("empty")).unwrap();
    std::fs::write(unmounted.join(".DS_Store"), "").unwrap();
    state.left.insert(unmounted.clone(), one(98));
    adopt(&mut state, &dir.path().join("next")).unwrap();
    let kept: Vec<PathBuf> = state.left.keys().cloned().collect();
    let mut expected: Vec<PathBuf> = (4..=LEFT_KEPT + 1)
        .map(|n| dir.path().join(format!("away{n}")))
        .chain([dir.path().join("now"), unmounted])
        .collect();
    expected.sort();
    assert_eq!(kept, expected, "away1 to away3 written into longest ago");
    assert!(
        !state.left.contains_key(&emptied),
        "holds files, none of them its own"
    );
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
            sidecars: Vec::new(),
            plan: None,
            stamp: None,
            digest: None,
        },
    );
    assert!(adopt(&mut state, &file).is_err());
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
fn a_song_left_as_built_is_pooled_in_its_album_as_its_file_was_made() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    let source = h.fetched("bbbbbbbbbbb");
    h.songs(ALBUM);
    assert!(h.run(&levelled(), Options::default()).0);
    let album_gains = |h: &Home| -> BTreeSet<String> {
        (plans(h).iter())
            .filter_map(|p| tag(p, "REPLAYGAIN_ALBUM_GAIN").map(str::to_string))
            .collect()
    };
    let built = album_gains(&h);
    assert_eq!(built.len(), 1, "{built:?}");
    let state = State::load(&h.dirs.home).unwrap();
    let (path, _) = (state.outputs.iter())
        .find(|(_, w)| w.sources == [SourceKey::youtube("bbbbbbbbbbb")])
        .unwrap();
    let song = h.dirs.library.join(path);
    let file = std::fs::read(&song).unwrap();
    std::fs::write(&source, "re-encoded louder by the site").unwrap();
    let mut louder = levelled();
    louder.wavs[1].1 = tone(0.9);

    let (ok, text) = h.run(&louder, Options::default());
    assert!(ok, "{text}");
    assert!(crate::ui::plain(&text).contains("left as built"), "{text}");
    assert!(!text.contains("Updated"), "{text}");
    assert_eq!(album_gains(&h), built, "one album, one gain");
    assert_eq!(std::fs::read(&song).unwrap(), file);
    let state = State::load(&h.dirs.home).unwrap();
    let held = state.outputs[path].plan.as_ref().unwrap();
    let gain = tag(held, "REPLAYGAIN_TRACK_GAIN").unwrap();
    let shown = h.status(&louder, &[], false);
    assert!(
        (shown.lines()).any(|l| l.contains("REPLAYGAIN_TRACK_GAIN") && l.contains(gain)),
        "the gain its file holds, {gain}: {shown}"
    );
    let (_, again) = h.run(&louder, Options::default());
    assert!(!again.contains("Updated"), "{again}");

    let accept = Options {
        accept: true,
        ..Options::default()
    };
    let (ok, text) = h.run(&louder, accept);
    assert!(ok, "{text}");
    let taken = album_gains(&h);
    assert_eq!(taken.len(), 1, "{taken:?}");
    assert_ne!(taken, built, "pooled by what it holds once accepted");
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
fn a_gain_put_in_opus_headers_leaves_a_flac_song_alone() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    let path = h.dirs.manual().join("Song.flac");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "flac").unwrap();
    let songs = "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"manual:Song.flac\"]\n";
    h.songs(songs);
    let mut fake = levelled().probe(".flac", crate::testing::FLAC);
    fake.wavs.push(("Song.flac".into(), tone(0.2)));
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(h.lib("Artist/Record/02 Song.flac").exists(), "{text}");
    h.songs(&format!("[loudness]\nmode = \"header\"\n{songs}"));
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(text.contains("Updated (loudness)"), "{text}");
    assert!(!text.contains("Song.flac"), "{text}");
    let (_, again) = h.run(&fake, Options::default());
    assert!(again.contains("Up to date: 2"), "{again}");
}

#[test]
fn loudness_off_levels_nothing_and_keeps_what_sources_carry() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[loudness]\nmode = \"off\"\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(
        fake.ran(crate::ffmpeg::PIPE),
        "decoded for its integrity all the same"
    );
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
    h.run_plain(&fake, Options::default());
    let before = fake.calls().len();
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let (_, text) = h.run_plain(&fake, Options::default());
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
fn facts_measured_before_integrity_are_caught_up_by_one_decode() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let fake = infos(&["aaaaaaaaaaa"]);
    h.run_plain(&fake, Options::default());
    let key = SourceKey::youtube("aaaaaaaaaaa");
    let mut state = State::load(&h.dirs.home).unwrap();
    let audio = state.facts.get_mut(&key).unwrap().audio.as_mut().unwrap();
    assert!(
        audio.analysis.integrity().is_some(),
        "measured with the rest"
    );
    audio.analysis.integrity = None;
    state.save(&h.dirs.home).unwrap();
    let before = fake.calls().len();
    let (_, text) = h.run_plain(&fake, Options::default());
    assert!(text.contains("Analyzing 1 source(s)"), "{text}");
    let calls = fake.calls();
    let new = &calls[before..];
    let piped: Vec<&Vec<String>> = new
        .iter()
        .filter(|c| c.iter().any(|a| a == crate::ffmpeg::PIPE))
        .collect();
    assert_eq!(piped.len(), 1, "{new:?}");
    assert!(
        piped[0].iter().any(|a| a == "framecrc"),
        "its packets listed"
    );
    assert!(
        new.iter().any(|c| c[0] == "ffprobe"),
        "its stream's length read"
    );
    let state = State::load(&h.dirs.home).unwrap();
    let measure = state.facts[&key]
        .audio
        .as_ref()
        .unwrap()
        .analysis
        .integrity();
    assert!(measure.is_some_and(|m| m.span.is_some()), "{measure:?}");
}

#[test]
fn a_subtitle_of_only_notes_is_no_lyrics_and_old_facts_learn_it() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    let mut fake = infos(&["aaaaaaaaaaa"]);
    fake.lrc = Some("[00:01.28]♪♪♪\n".into());
    let (ok, text) = h.run_plain(&fake, Options::default());
    assert!(ok, "{text}");
    let key = SourceKey::youtube("aaaaaaaaaaa");
    let state = State::load(&h.dirs.home).unwrap();
    assert_eq!(state.facts[&key].lyrics[0].sings, Some(false));
    assert!(plans(&h).iter().all(|p| p.lyrics.is_none()));

    // Facts measured before muman asked are read again, alone.
    let mut state = State::load(&h.dirs.home).unwrap();
    state.facts.get_mut(&key).unwrap().lyrics[0].sings = None;
    state.save(&h.dirs.home).unwrap();
    let (ok, text) = h.run_plain(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(text.contains("Reading the lyrics of 1 source(s)"), "{text}");
    assert!(!text.contains("Measuring"), "{text}");
    assert!(!text.contains("has changed"), "{text}");
    let state = State::load(&h.dirs.home).unwrap();
    assert_eq!(state.facts[&key].lyrics[0].sings, Some(false));
    let (_, again) = h.run_plain(&fake, Options::default());
    assert!(!again.contains("Reading the lyrics"), "{again}");
}

#[test]
fn a_damaged_file_of_your_own_is_named_and_never_fetched() {
    let h = home();
    let file = h.dirs.manual().join("a.flac");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "a.flac").unwrap();
    h.songs("[[song]]\nsources = [\"manual:a.flac\"]\n");
    let mut holed = "#tb 0: 1/1000\n".to_string();
    for n in (0..100).filter(|n| !(20..70).contains(n)) {
        let pts = n * 60;
        let _ = writeln!(holed, "0, {pts}, {pts}, 60, 300, 0x0");
    }
    let fake = Fake {
        timelines: vec![("a.flac".into(), holed)],
        ..Fake::default()
    }
    .probe("a.flac", crate::testing::FLAC);
    let (_, text) = h.run_plain(&fake, Options::default());
    assert!(text.contains("manual:a.flac lacks"), "{text}");
    let state = State::load(&h.dirs.home).unwrap();
    assert!(state.failures.is_empty() && state.damaged.is_empty());
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

/// A home holding one manual FLAC in 5.1 under `settings`, and a fake
/// whose stereo mix of it is far quieter than its six channels.
fn surround_home(settings: &str) -> (Home, Fake) {
    let h = flac_home(settings);
    let surround = crate::testing::FLAC.replace(
        r#""channels": 2, "sample_rate": "44100""#,
        r#""channels": 6, "channel_layout": "5.1", "sample_rate": "48000""#,
    );
    let fake = Fake {
        packets: vec![("Song.flac".into(), 30_000_000)],
        wavs: vec![("Song.flac".into(), tone(0.4))],
        mixed: vec![("Song.flac".into(), tone(0.05))],
        ..Fake::default()
    }
    .probe(".flac", &surround);
    (h, fake)
}

#[test]
fn a_song_mixed_to_fit_max_size_is_levelled_by_its_mix() {
    for mode in ["tags", "audio"] {
        let loudness = format!("[loudness]\nmode = \"{mode}\"\n");
        let (by_codec, fake) = surround_home(&format!(
            "{loudness}[audio]\ncodecs = [\"mp3\"]\nlossless = \"mp3\"\n"
        ));
        let (ok, text) = by_codec.run(&fake, Options::default());
        assert!(ok, "{text}");
        let mixed = plans(&by_codec)[0].clone();
        assert!(mixed.format.mix().is_some(), "{mixed:?}");

        // The FLAC is rendered to learn whether it fits, and the fake's
        // rendered files take a few KiB.
        let (h, fake) = surround_home(&format!(
            "{loudness}[library]\nmax_size = \"10 KiB\"\n[audio]\nlossy = \"mp3\"\n"
        ));
        let (ok, text) = h.run(&fake, Options::default());
        assert!(ok, "{text}");
        let fitted = plans(&h)[0].clone();
        assert_eq!(fitted.format.mix(), mixed.format.mix(), "{fitted:?}");
        assert_eq!(
            fitted.loudness.map(|g| g.track),
            mixed.loudness.map(|g| g.track),
            "{mode}: levelled by the stereo it is written in"
        );
        assert_eq!(
            tag(&fitted, "REPLAYGAIN_TRACK_GAIN"),
            tag(&mixed, "REPLAYGAIN_TRACK_GAIN")
        );
        assert_eq!(fitted.chain(), mixed.chain(), "{mode}");
        let encodes: Vec<Vec<String>> = (fake.calls().into_iter())
            .filter(|c| c.iter().any(|a| a == "libmp3lame"))
            .collect();
        let gain = fitted.chain().gain;
        assert_eq!(gain != 0, mode == "audio", "{fitted:?}");
        let volume = format!("volume={}dB", crate::units::Level(gain).decimal());
        let written = encodes.last().unwrap();
        assert_eq!(
            written.iter().any(|a| a.contains(&volume)),
            gain != 0,
            "{mode}: the file written takes the gain levelled: {written:?}"
        );
        let (ok, again) = h.run(&fake, Options::default());
        assert!(ok && again.contains("Up to date: 1"), "{mode}: {again}");
        assert!(!again.contains("Rendering"), "{mode}: {again}");
        let manifest = Manifest::load(&h.dirs.home).unwrap();
        let state = State::load(&h.dirs.home).unwrap();
        let views = crate::query::resolved_views(&manifest, &state, &h.dirs.library).unwrap();
        let shown = views[0].1.as_ref().map(|r| &r.plan);
        assert_eq!(shown, Some(&fitted), "{mode}: shown as written");
    }
}

#[test]
fn a_song_mixed_to_fit_max_size_is_pooled_by_its_mix_in_its_album() {
    let home_of = |settings: &str| {
        let (h, fake) = surround_home(settings);
        let path = h.dirs.manual().join("Other.flac");
        std::fs::write(&path, "other").unwrap();
        h.songs(&format!(
            "{settings}\n[[song]]\nsources = [\"manual:Song.flac\"]\n\
             [[song]]\nsources = [\"manual:Other.flac\"]\n"
        ));
        let other = crate::testing::FLAC.replace(r#""track": "2""#, r#""track": "3""#);
        let mut fake = fake.probe("Other.flac", &other);
        fake.probes.rotate_right(1);
        fake.wavs.push(("Other.flac".into(), tone(0.2)));
        fake.packets.push(("Other.flac".into(), 1_000_000));
        (h, fake)
    };
    let album = |h: &Home| {
        let plans = plans(h);
        let other = plans.iter().find(|p| tag(p, "TRACKNUMBER") == Some("3"));
        other.and_then(|p| p.loudness?.album)
    };
    let (by_codec, fake) = home_of("[audio]\ncodecs = [\"mp3\"]\nlossless = \"mp3\"\n");
    let (ok, text) = by_codec.run(&fake, Options::default());
    assert!(ok, "{text}");
    let (h, fake) = home_of("[library]\nmax_size = \"12 MB\"\n[audio]\nlossy = \"mp3\"\n");
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    assert!(
        plans(&h).iter().any(|p| p.format.mix().is_some()),
        "{:?}",
        plans(&h)
    );
    assert!(album(&h).is_some());
    assert_eq!(album(&h), album(&by_codec), "{text}");
}

/// The source the one song of `curves`' uploads is written from, each
/// given its loudness curve, and why, as `status --all` says it.
fn chosen(curves: &[(&str, Vec<f64>)]) -> (SourceKey, String) {
    let h = home();
    let mut list = Vec::new();
    let mut fake = Fake::default();
    for (id, curve) in curves {
        h.fetched(id);
        list.push(format!("\"youtube.com:{id}\""));
        fake = fake.info(
            id,
            &format!(r#"{{"id": "{id}", "track": "Song", "artists": ["A"], "album": "Record"}}"#),
        );
        fake.pcm.push(((*id).into(), pcm(curve)));
    }
    h.songs(&format!("[[song]]\nsources = [{}]\n", list.join(", ")));
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    let state = State::load(&h.dirs.home).unwrap();
    let written = state.outputs.values().next().unwrap();
    let why = h.status(&fake, &[], true);
    (written.plan.as_ref().unwrap().audio.key.clone(), why)
}

/// A whole song: `frames` of sound between 100 ms of quiet at each end.
fn whole_song(frames: usize, seed: u64) -> Vec<f64> {
    let mut song = vec![0.0; 10];
    song.extend(levels(frames, seed));
    song.extend([0.0; 10]);
    song
}

#[test]
fn a_release_that_starts_at_full_sound_beats_its_video_with_an_intro() {
    // A rock release whose first chord lands on its first sample, and its
    // video, a 10 s skit running into it.
    let mut release = levels(6000, 7);
    release.extend([0.0; 10]);
    let mut video = levels(1000, 99);
    video.extend(&release);
    let (key, why) = chosen(&[("vvvvvvvvvvv", video), ("rrrrrrrrrrr", release)]);
    assert_eq!(key, SourceKey::youtube("rrrrrrrrrrr"), "{why}");
}

#[test]
fn an_excerpt_that_fades_out_never_beats_the_whole_song() {
    let whole = whole_song(6000, 7);
    // The first half, faded out over its last second as a preview is.
    let mut excerpt = whole[..3000].to_vec();
    for (i, level) in excerpt[2900..].iter_mut().enumerate() {
        let fade = 1.0 - (i as f64 + 1.0) / 100.0;
        *level *= fade;
    }
    let (key, why) = chosen(&[("eeeeeeeeeee", excerpt), ("wwwwwwwwwww", whole)]);
    assert_eq!(key, SourceKey::youtube("wwwwwwwwwww"), "{why}");
}

#[test]
fn an_excerpt_cut_in_a_rest_never_beats_the_whole_song() {
    let mut whole = whole_song(6000, 7);
    // A 20 ms rest between two phrases, and the excerpt cut inside it.
    whole[3008..3010].copy_from_slice(&[0.0, 0.0]);
    let excerpt = whole[..3010].to_vec();
    let (key, why) = chosen(&[("eeeeeeeeeee", excerpt), ("wwwwwwwwwww", whole)]);
    assert_eq!(key, SourceKey::youtube("wwwwwwwwwww"), "{why}");
}

#[test]
fn a_header_gain_the_ceiling_holds_at_nothing_leaves_an_opus_song_alone() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    let songs = "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n";
    h.songs(songs);
    // Quiet, so it asks to be raised, but for one peak at full scale,
    // which leaves the raise no room under the ceiling.
    let mut samples = crate::testing::sine(8000, 2, 1000.0, 0.01, 2.0);
    samples[8000] = 0.99;
    let mut fake = infos(&["aaaaaaaaaaa"]);
    fake.wavs = vec![(
        "aaaaaaaaaaa".into(),
        crate::testing::wav(8000, 2, Some(0x3), &samples),
    )];
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    h.songs(&format!("[loudness]\nmode = \"header\"\n{songs}"));
    let (ok, text) = h.run(&fake, Options::default());
    assert!(ok, "{text}");
    let gains = plans(&h)[0].loudness.unwrap();
    assert!(gains.track.gain > 0, "{gains:?}");
    assert_eq!(gains.header(true, true), Some(0), "{gains:?}");
    assert!(text.contains("Up to date: 1"), "{text}");
}

#[test]
fn an_excerpt_from_past_a_long_song_s_first_minute_never_beats_the_whole_song() {
    // A seven-minute album version, and the edit cut from it starting at
    // its vocals, 90 s in.
    let whole = whole_song(42_000, 7);
    let excerpt = whole[9000..27_000].to_vec();
    let (key, why) = chosen(&[("eeeeeeeeeee", excerpt), ("wwwwwwwwwww", whole)]);
    assert_eq!(key, SourceKey::youtube("wwwwwwwwwww"), "{why}");
}

#[test]
fn songs_whose_files_beside_them_share_a_name_are_kept_apart() {
    use crate::codec::Codec;
    use crate::resolve::{AudioRef, Format, Plan, Resolved, TAGGING, Why, render_version};
    let song = |stem: &str, id: &str| {
        (
            0,
            Resolved {
                plan: Plan {
                    folder: None,
                    version: render_version(Codec::Opus),
                    format: Format::Copy { codec: Codec::Opus },
                    audio: AudioRef {
                        key: SourceKey::youtube(id),
                        rev: "r".into(),
                        index: 0,
                    },
                    cover: None,
                    lyrics: None,
                    tags: Vec::new(),
                    loudness: None,
                    tagging: TAGGING,
                    chapters: Vec::new(),
                },
                stem: PathBuf::from(stem),
                why: Why::default(),
            },
        )
    };
    let library = crate::settings::Library::default();
    let naming = library.naming(Path::new("lib")).unwrap();
    // A translation beside a song as `.en.lrc`, its lyrics as `.lrc`.
    let beside = |p: &Path, _: &Plan| {
        let stem = p.with_extension("");
        ["en.lrc", "lrc"]
            .iter()
            .map(|s| PathBuf::from(format!("{}.{s}", stem.display())))
            .collect()
    };
    let mut planned = vec![
        song("Intro", "aaaaaaaaaaa"),
        song("intro.en", "bbbbbbbbbbb"),
    ];
    plan::separate(&mut planned, &[], &naming, &beside);
    assert_eq!(planned[0].1.stem, PathBuf::from("Intro"));
    assert_eq!(
        planned[1].1.stem,
        PathBuf::from("intro.en [bbbbbbbbbbb]"),
        "its `.lrc` is the first song's `.en.lrc` on a filesystem blind to case"
    );
    let mut apart = vec![
        song("Intro", "aaaaaaaaaaa"),
        song("Outro.en", "bbbbbbbbbbb"),
    ];
    plan::separate(&mut apart, &[], &naming, &beside);
    assert_eq!(apart[1].1.stem, PathBuf::from("Outro.en"));
}

#[test]
fn a_file_named_as_another_song_s_folder_is_told_apart_in_either_order() {
    use crate::codec::Codec;
    use crate::resolve::{AudioRef, Format, Plan, Resolved, TAGGING, Why, render_version};
    let song = |stem: PathBuf, id: &str| {
        (
            0,
            Resolved {
                plan: Plan {
                    folder: None,
                    version: render_version(Codec::Flac),
                    format: Format::Copy { codec: Codec::Flac },
                    audio: AudioRef {
                        key: SourceKey::youtube(id),
                        rev: "r".into(),
                        index: 0,
                    },
                    cover: None,
                    lyrics: None,
                    tags: Vec::new(),
                    loudness: None,
                    tagging: TAGGING,
                    chapters: Vec::new(),
                },
                stem,
                why: Why::default(),
            },
        )
    };
    let library = crate::settings::Library::default();
    let naming = library.naming(Path::new("lib")).unwrap();
    let beside = |_: &Path, _: &Plan| Vec::new();
    let inside = Path::new("Y.flac").join("t1");
    for first in [true, false] {
        let mut planned = vec![
            song(inside.clone(), "aaaaaaaaaaa"),
            song(PathBuf::from("y"), "bbbbbbbbbbb"),
        ];
        if !first {
            planned.reverse();
        }
        plan::separate(&mut planned, &[], &naming, &beside);
        let stem = |id: &str| {
            (planned.iter())
                .find(|(_, r)| r.plan.audio.key == SourceKey::youtube(id))
                .map(|(_, r)| r.stem.clone())
                .unwrap()
        };
        assert_eq!(stem("aaaaaaaaaaa"), inside);
        assert_eq!(
            stem("bbbbbbbbbbb"),
            PathBuf::from("y [bbbbbbbbbbb]"),
            "`y.flac` is the folder `Y.flac` on a filesystem blind to case"
        );
    }
}

#[test]
fn a_source_holding_audio_compared_before_is_not_compared_again() {
    let h = home();
    for id in ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc"] {
        h.fetched(id);
    }
    // The third upload is the second's audio, packet for packet.
    let fake = || Fake {
        hashes: vec![
            ("aaaaaaaaaaa".into(), 1),
            ("bbbbbbbbbbb".into(), 2),
            ("ccccccccccc".into(), 2),
        ],
        pcm: vec![
            ("aaaaaaaaaaa".into(), pcm(&levels(600, 1))),
            ("bbbbbbbbbbb".into(), pcm(&levels(600, 2))),
            ("ccccccccccc".into(), pcm(&levels(600, 2))),
        ],
        ..infos(&["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc"])
    };
    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\", \"youtube.com:bbbbbbbbbbb\"]\n");
    let (ok, text) = h.run_plain(&fake(), Options::default());
    assert!(ok && text.contains("Comparing"), "{text}");
    let compared = State::load(&h.dirs.home).unwrap().alignments;
    assert!(!compared.is_empty(), "{text}");

    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\", \"youtube.com:ccccccccccc\"]\n");
    let (ok, text) = h.run_plain(&fake(), Options::default());
    assert!(ok, "{text}");
    assert!(!text.contains("Comparing"), "{text}");
    let kept = State::load(&h.dirs.home).unwrap().alignments;
    assert_eq!(
        kept.iter().map(|a| &a.revs).collect::<Vec<_>>(),
        compared.iter().map(|a| &a.revs).collect::<Vec<_>>(),
        "kept while a listed source holds its audio"
    );

    h.songs("[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n");
    assert!(h.run_plain(&fake(), Options::default()).0);
    assert!(
        State::load(&h.dirs.home).unwrap().alignments.is_empty(),
        "gone once no source kept holds its audio"
    );
}

#[test]
fn a_song_left_as_built_keeps_its_path_from_a_song_named_as_it_was() {
    let h = home();
    let source = h.fetched("aaaaaaaaaaa");
    h.fetched("bbbbbbbbbbb");
    h.songs(TWO);
    let titled = |a: &str| Fake {
        hashes: vec![("aaaaaaaaaaa".into(), 7), ("bbbbbbbbbbb".into(), 8)],
        ..Fake::default()
            .info(
                "aaaaaaaaaaa",
                &format!(r#"{{"id": "aaaaaaaaaaa", "title": "{a}", "uploader": "Chan"}}"#),
            )
            .info(
                "bbbbbbbbbbb",
                r#"{"id": "bbbbbbbbbbb", "title": "Harbor", "uploader": "Chan"}"#,
            )
    };
    assert!(h.run(&titled("Lantern Weather"), Options::default()).0);
    let song = h.lib("Chan/Lantern Weather/Lantern Weather.opus");
    let built = std::fs::read(&song).unwrap();
    std::fs::write(&source, "fetched again").unwrap();
    let list = std::fs::read_to_string(h.dirs.manifest()).unwrap();
    let at = list
        .find("sources = [\"youtube.com:bbbbbbbbbbb\"]")
        .unwrap();
    let titled_b = list[at..].replacen("tags.title = \"\"", "tags.title = \"Lantern Weather\"", 1);
    assert_ne!(titled_b, list[at..], "{list}");
    std::fs::write(h.dirs.manifest(), format!("{}{titled_b}", &list[..at])).unwrap();

    let (_, text) = h.run(&titled("Paper Comets"), Options::default());
    let plain = crate::ui::plain(&text);
    assert!(plain.contains("left as built"), "{plain}");
    assert_eq!(std::fs::read(&song).unwrap(), built, "{plain}");
    let state = State::load(&h.dirs.home).unwrap();
    let rel = PathBuf::from("Chan/Lantern Weather/Lantern Weather.opus");
    assert_eq!(
        state.outputs[&rel].sources,
        vec![SourceKey::parse("youtube.com:aaaaaaaaaaa").unwrap()],
        "{plain}"
    );
}

/// A manual FLAC's probe: a second long, or said to run 200 s where
/// `said`, titled and on `album`, with an embedded cover.
fn tagged_flac(title: &str, track: u32, album: &str, said: bool) -> String {
    let duration = if said { "200.0" } else { "1.0" };
    format!(
        r#"{{"streams": [{{"index": 0, "codec_type": "audio", "codec_name": "flac",
        "channels": 2, "sample_rate": "44100", "duration": "{duration}"}},
        {{"index": 1, "codec_type": "video", "codec_name": "mjpeg", "width": 64, "height": 64,
        "disposition": {{"attached_pic": 1}}}}], "format": {{"format_name": "flac",
        "duration": "{duration}", "tags": {{"TITLE": "{title}", "ARTIST": "Artist",
        "ALBUM": "{album}", "track": "{track}"}}}}}}"#
    )
}

#[test]
fn a_song_left_as_built_keeps_the_cover_its_folder_holds() {
    let h = home();
    listing(
        &h,
        "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\n",
        &["a", "b"],
    );
    let whole = Fake::default()
        .probe("a.flac", &tagged_flac("Kite", 1, "Record", false))
        .probe("b.flac", &tagged_flac("Tide", 2, "Record", false));
    let (ok, text) = h.run(&whole, Options::default());
    assert!(ok, "{text}");
    let cover = PathBuf::from("Artist/Record/cover.jpg");
    let holder = |h: &Home| {
        let state = State::load(&h.dirs.home).unwrap();
        state
            .outputs
            .iter()
            .filter(|(_, w)| w.sidecar_paths().any(|s| *s == cover))
            .map(|(p, _)| p.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        holder(&h),
        [PathBuf::from("Artist/Record/01 Kite.flac")],
        "{text}"
    );

    std::fs::write(h.dirs.manual().join("a.flac"), "a copy stopped short").unwrap();
    let cut = Fake {
        held: vec![("a.flac".into(), 3)],
        ..Fake::default()
            .probe("a.flac", &tagged_flac("Kite", 1, "Other", true))
            .probe("b.flac", &tagged_flac("Tide", 2, "Record", false))
    };
    let (_, text) = h.run(&cut, Options::default());
    assert!(text.contains("cut off"), "{text}");
    assert_eq!(
        holder(&h),
        [PathBuf::from("Artist/Record/01 Kite.flac")],
        "{text}"
    );
}

#[test]
fn a_song_left_as_built_spells_its_album_for_no_other_song() {
    let h = home();
    listing(&h, "", &["a", "b"]);
    let whole = Fake::default()
        .probe("a.flac", &tagged_flac("Kite", 1, "Paper comets", false))
        .probe("b.flac", &tagged_flac("Tide", 2, "Paper comets", false));
    let (ok, text) = h.run(&whole, Options::default());
    assert!(ok, "{text}");
    let built = formats(&h);
    std::fs::write(h.dirs.manual().join("a.flac"), "a copy stopped short").unwrap();
    let cut = Fake {
        held: vec![("a.flac".into(), 3)],
        ..Fake::default()
            .probe("a.flac", &tagged_flac("Kite", 1, "Paper Comets", true))
            .probe("b.flac", &tagged_flac("Tide", 2, "Paper comets", false))
    };
    let (_, text) = h.run(&cut, Options::default());
    assert!(text.contains("cut off"), "{text}");
    assert_eq!(formats(&h), built, "{text}");
}

#[test]
fn a_song_left_as_built_is_shown_and_queried_as_its_file_holds() {
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
    std::fs::write(&source, "fetched again").unwrap();
    let (ok, text) = h.run(&titled("Paper Comets"), Options::default());
    assert!(ok, "{text}");

    let file = "Chan/Lantern Weather/Lantern Weather.opus";
    let shown = h.status(&titled("Paper Comets"), &[], true);
    assert!(shown.contains(&format!("{file} (left as built")), "{shown}");
    assert!(
        (shown.lines()).any(|l| l.trim_start().starts_with("TITLE") && l.contains("Lantern")),
        "{shown}"
    );
    assert!(!shown.contains("Paper Comets"), "{shown}");
    let matched = h.status(&titled("Paper Comets"), &["lantern"], false);
    assert!(matched.contains("left as built"), "{matched}");
    let manifest = crate::manifest::Manifest::load(&h.dirs.home).unwrap();
    let views = crate::query::views(&manifest, &found(&h.dirs).unwrap(), &h.dirs.library);
    let views = views.unwrap();
    assert_eq!(views[0].first("path"), file);
    assert_eq!(views[0].first("title"), "Lantern Weather");
}

#[test]
fn a_song_left_alone_for_a_file_you_changed_is_listed_as_its_file_is() {
    let h = home();
    h.fetched("aaaaaaaaaaa");
    h.songs(ONE);
    let fake = infos(&["aaaaaaaaaaa"]);
    h.run(&fake, Options::default());
    let file = "Chan/Title aaaaaaaaaaa/Title aaaaaaaaaaa.opus";
    std::fs::write(h.lib(file), "retagged by hand").unwrap();
    h.songs(
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { title = \"Night Ferry\" }\n",
    );
    let manifest = crate::manifest::Manifest::load(&h.dirs.home).unwrap();
    let views = crate::query::views(&manifest, &found(&h.dirs).unwrap(), &h.dirs.library);
    let views = views.unwrap();
    assert_eq!(views[0].first("path"), file);
    assert_eq!(views[0].first("title"), "Title aaaaaaaaaaa");
    let shown = h.status(&fake, &["path:aaaaaaaaaaa"], false);
    assert!(shown.contains("left alone"), "{shown}");
    let (_, text) = h.run(&fake, Options::default());
    assert!(text.contains("Left alone"), "{text}");
}

/// What `check` and `info` foresee of the next sync of `h`.
fn foreseen(fake: &Fake, h: &Home) -> Foresight {
    let scratch = crate::atomic::Scratch::new().unwrap();
    let ground = foreground(fake, &h.dirs, Duration::ZERO, scratch.path()).unwrap();
    foresight(fake, &ground.grounds(&h.dirs.library, (false, false))).unwrap()
}

#[test]
fn check_and_info_fit_the_library_as_the_sync_does() {
    let h = flac_home("[library]\nmax_size = \"10 MB\"\n");
    let fake = big_flac();
    let first = foreseen(&fake, &h);
    assert!(
        !first.to_measure.is_empty() && !first.whole(),
        "nothing is measured yet"
    );
    assert!(h.run(&fake, Options::default()).0);
    let lowered = "Artist/Record/02 Song.opus";
    assert!(h.lib(lowered).exists());

    h.songs("[library]\nmax_size = \"2 KiB\"\n[[song]]\nsources = [\"manual:Song.flac\"]\n");
    let ahead = foreseen(&fake, &h);
    let fitted = ahead.fitted.as_ref().unwrap();
    assert_eq!(fitted.left_out, [0]);
    assert!(
        !fitted.guessed.is_empty(),
        "its lowest rung is not measured yet"
    );
    let gone: Vec<&PathBuf> = (ahead.intent.removals())
        .flat_map(|(_, fate)| fate.gone())
        .collect();
    assert_eq!(gone, [&PathBuf::from(lowered)]);
    let said = crate::diagnose::diagnose(&fake, &h.dirs, Duration::ZERO).unwrap();
    assert!(
        (said.said.iter()).any(|s| matches!(
            s,
            crate::diagnose::Said::Problem(l) if l.starts_with("Left out by the next sync")
        )),
        "{:?}",
        said.said
    );
    let (ok, text) = h.run(&fake, Options::default());
    assert!(!ok, "{text}");
    assert!(!h.lib(lowered).exists(), "{text}");
    let after = foreseen(&fake, &h);
    assert!(
        after.whole(),
        "every size it fits by is recorded: {:?}",
        after.fitted
    );
    assert_eq!(after.fitted.as_ref().unwrap().left_out, [0]);
}
