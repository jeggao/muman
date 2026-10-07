use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::sync::Mutex;

use super::*;
use crate::dirs::Dirs;
use crate::runner::Line;

/// Answers a listing by the URL it ends with, and a download by writing
/// each video it names into the store unless the archive names it.
#[derive(Default)]
struct Net {
    downloads: Mutex<Vec<Vec<String>>>,
}

const PLAYLIST: &str = r#"{"_type": "playlist", "title": "Songs", "webpage_url": "https://own",
    "webpage_url_basename": "playlist",
    "entries": [{"id": "vid00000009"}, {"id": "vid00000010"}]}"#;
const ALBUM: &str = r#"{"_type": "playlist", "id": "OLAK5uy_abc", "title": "Album - Record",
    "webpage_url": "https://album", "webpage_url_basename": "playlist",
    "entries": [{"id": "vid00000009", "channel": "A - Topic"}, {"id": "vid00000010", "channel": "A - Topic"}]}"#;

impl Runner for Net {
    fn run(&self, _: &[OsString]) -> Result<()> {
        anyhow::bail!("not in tests")
    }

    fn output(&self, cmd: &[OsString]) -> Result<Vec<u8>> {
        let one = cmd.iter().any(|a| a == "--no-playlist");
        let url = cmd.last().and_then(|u| u.to_str()).unwrap_or_default();
        let json = match (url, one) {
            ("https://p", false) => PLAYLIST,
            ("https://album", false) => ALBUM,
            ("https://mix", false) => {
                r#"{"_type": "playlist", "webpage_url_basename": "watch",
                    "entries": [{"id": "vid00000009"}, {"id": "vid00000011"}]}"#
            }
            ("https://mix", true) => r#"{"id": "vid00000010", "duration": 183}"#,
            ("https://v", false) => r#"{"_type": "video", "id": "vid00000009"}"#,
            _ => anyhow::bail!("offline"),
        };
        Ok(json.as_bytes().to_vec())
    }

    fn stream(&self, cmd: &[OsString], on_line: &mut dyn FnMut(Line<'_>)) -> Result<bool> {
        let args: Vec<String> = cmd
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        self.downloads.lock().unwrap().push(args.clone());
        let after = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .map(|i| args[i + 1].clone())
        };
        let store = PathBuf::from(after("--paths").unwrap());
        let done =
            PathBuf::from(&args[args.iter().position(|a| a == "--print-to-file").unwrap() + 2]);
        let skipped = after("--download-archive")
            .map(|p| std::fs::read_to_string(p).unwrap())
            .unwrap_or_default();
        let urls = &args[args.iter().position(|a| a == "--").unwrap() + 1..];
        let ids: Vec<&str> = urls
            .iter()
            .flat_map(|u| match u.as_str() {
                "https://own" | "https://album" => vec!["vid00000009", "vid00000010"],
                "https://v" => vec!["vid00000009"],
                u => u
                    .strip_prefix("https://www.youtube.com/watch?v=")
                    .into_iter()
                    .collect(),
            })
            .collect();
        let mut lines = String::new();
        for id in ids {
            if skipped.contains(id) {
                continue;
            }
            on_line(Line::Out(&format!("[download] {id}")));
            let path = store.join(format!("chan/Song {id} [{id}].mkv"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"mkv").unwrap();
            let _ = writeln!(
                lines,
                "Youtube {id} {}",
                serde_json::to_string(&path).unwrap()
            );
        }
        std::fs::write(done, lines).unwrap();
        Ok(true)
    }
}

fn listed(urls: &[&str]) -> Listed {
    let urls: Vec<String> = urls.iter().map(|u| (*u).to_string()).collect();
    list(&urls, &Net::default(), &mut Vec::new()).unwrap()
}

#[test]
fn a_playlist_is_downloaded_at_its_own_address() {
    let l = listed(&["https://p", "https://v"]);
    assert_eq!(l.urls, ["https://own", "https://v"]);
    let ids: Vec<_> = l.videos.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["vid00000009", "vid00000010"]);
    assert_eq!(l.albums, []);
}

#[test]
fn a_mix_is_only_its_video() {
    let l = listed(&["https://mix"]);
    assert_eq!(l.urls, ["https://mix"]);
    let ids: Vec<_> = l.videos.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["vid00000010"]);
}

#[test]
fn an_unlisted_url_is_downloaded_as_given() {
    let l = listed(&["https://gone"]);
    assert_eq!(l.urls, ["https://gone"]);
    assert!(l.videos.is_empty());
}

struct Setup {
    dir: tempfile::TempDir,
    dirs: Dirs,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        home: dir.path().join("home"),
        library: dir.path().join("lib"),
    };
    std::fs::create_dir_all(dir.path().join("temp")).unwrap();
    Setup { dir, dirs }
}

fn acquire_urls(
    s: &Setup,
    net: &Net,
    known: &[SourceKey],
    urls: &[&str],
) -> (bool, Additions, String) {
    let store = Store::scan(&s.dirs).unwrap();
    let ytdlp = s.dirs.ytdlp();
    let temp = s.dir.path().join("temp");
    let fetcher = Fetcher {
        store: &ytdlp,
        temp: &temp,
        partial: &temp,
        plugins: None,
        options: &crate::settings::Ytdlp::default(),
        live: false,
        runs: Cell::new(0),
    };
    let mut out = Vec::new();
    let urls: Vec<String> = urls.iter().map(|u| (*u).to_string()).collect();
    let (ok, add) = Acquire {
        runner: net,
        fetcher: &fetcher,
        out: &mut out,
        known: known.iter().cloned().collect(),
        removed: BTreeSet::new(),
        store: &store,
    }
    .urls(&urls, false)
    .unwrap();
    (ok, add, String::from_utf8(out).unwrap())
}

#[test]
fn each_new_video_is_proposed_as_a_song_and_known_ones_are_skipped() {
    let s = setup();
    let net = Net::default();
    let (ok, add, text) = acquire_urls(
        &s,
        &net,
        &[SourceKey::youtube("vid00000009")],
        &["https://p"],
    );
    assert!(ok, "{text}");
    let proposed: Vec<&Vec<SourceKey>> = add.proposals.iter().map(|p| &p.sources).collect();
    assert_eq!(proposed, [&vec![SourceKey::youtube("vid00000010")]]);
    assert_eq!(add.proposals[0].label, "Song vid00000010");
    assert!(
        text.contains("yt-dlp: [download] vid00000010"),
        "yt-dlp's lines are labeled: {text}"
    );
}

#[test]
fn an_album_numbers_new_songs_and_joins_known_ones() {
    let s = setup();
    let net = Net::default();
    let (_, add, _) = acquire_urls(
        &s,
        &net,
        &[SourceKey::youtube("vid00000009")],
        &["https://album"],
    );
    let album = SourceKey::playlist("OLAK5uy_abc");
    assert_eq!(add.albums, [(album.clone(), 2)]);
    let places: Vec<(&SourceKey, Option<u32>)> = add
        .proposals
        .iter()
        .map(|p| (&p.sources[0], p.album.as_ref().map(|a| a.1)))
        .collect();
    assert_eq!(
        places,
        [
            (&SourceKey::youtube("vid00000009"), Some(1)),
            (&SourceKey::youtube("vid00000010"), Some(2))
        ]
    );
}

#[test]
fn missing_sources_are_fetched_again_without_the_archive() {
    let s = setup();
    std::fs::create_dir_all(&s.dirs.home).unwrap();
    std::fs::write(
        s.dirs.manifest(),
        "version = 1\n[[song]]\nsources = [\"youtube:vid00000009\", \"manual:gone.flac\"]\n",
    )
    .unwrap();
    let manifest = Manifest::load(&s.dirs.home).unwrap();
    let store = Store::scan(&s.dirs).unwrap();
    let ytdlp = s.dirs.ytdlp();
    let temp = s.dir.path().join("temp");
    let fetcher = Fetcher {
        store: &ytdlp,
        temp: &temp,
        partial: &temp,
        plugins: None,
        options: &crate::settings::Ytdlp::default(),
        live: false,
        runs: Cell::new(0),
    };
    let net = Net::default();
    let mut out = Vec::new();
    let ok = Acquire {
        runner: &net,
        fetcher: &fetcher,
        out: &mut out,
        known: BTreeSet::new(),
        removed: BTreeSet::new(),
        store: &store,
    }
    .missing(&manifest, &BTreeMap::new(), false)
    .unwrap()
    .0;
    assert!(ok, "{}", String::from_utf8(out).unwrap());
    let downloads = net.downloads.lock().unwrap();
    assert_eq!(downloads.len(), 1);
    assert!(!downloads[0].iter().any(|a| a == "--download-archive"));
    assert!(
        Store::scan(&s.dirs)
            .unwrap()
            .has(&SourceKey::youtube("vid00000009"))
    );
}

#[test]
fn a_label_drops_the_id() {
    assert_eq!(
        label_of(Path::new("/s/c/Song ⧸ B [vid00000001].mkv")),
        "Song ⧸ B"
    );
    assert_eq!(label_of(Path::new("/m/Plain.flac")), "Plain");
}

#[test]
fn a_removed_video_is_skipped_in_a_playlist_and_listed_again_named_alone() {
    let s = setup();
    let gone = SourceKey::youtube("vid00000009");
    let fetch = |urls: &[&str]| {
        let store = Store::scan(&s.dirs).unwrap();
        let ytdlp = s.dirs.ytdlp();
        let temp = s.dir.path().join("temp");
        let fetcher = Fetcher {
            store: &ytdlp,
            temp: &temp,
            partial: &temp,
            plugins: None,
            options: &crate::settings::Ytdlp::default(),
            live: false,
            runs: Cell::new(0),
        };
        let mut out = Vec::new();
        let urls: Vec<String> = urls.iter().map(|u| (*u).to_string()).collect();
        let (_, add) = Acquire {
            runner: &Net::default(),
            fetcher: &fetcher,
            out: &mut out,
            known: BTreeSet::from([gone.clone()]),
            removed: BTreeSet::from([gone.clone()]),
            store: &store,
        }
        .urls(&urls, false)
        .unwrap();
        let keys: Vec<SourceKey> = add.proposals.into_iter().flat_map(|p| p.sources).collect();
        (keys, String::from_utf8(out).unwrap())
    };
    let (keys, _) = fetch(&["https://p"]);
    assert_eq!(keys, [SourceKey::youtube("vid00000010")]);
    let (keys, text) = fetch(&["https://v"]);
    assert_eq!(keys, std::slice::from_ref(&gone), "{text}");
    assert!(text.contains("removed before"), "{text}");
}

#[test]
fn a_source_fetched_again_asks_for_the_format_and_page_its_song_records() {
    let s = setup();
    std::fs::create_dir_all(&s.dirs.home).unwrap();
    std::fs::write(
        s.dirs.manifest(),
        "version = 1\n\
         [[song]]\nsources = [\"youtube:vid00000009\"]\n\
         held.\"youtube:vid00000009\" = { audio = \"0123456789abcdef\", format = \"399+251\" }\n\
         [[song]]\nsources = [\"youtube:vid00000010\"]\n\
         [[song]]\nsources = [\"archiveorg:item0001\"]\n\
         held.\"archiveorg:item0001\" = { audio = \"0123456789abcdef\", format = \"1\", \
         url = \"https://archive.example/details/item0001\" }\n",
    )
    .unwrap();
    let manifest = Manifest::load(&s.dirs.home).unwrap();
    let store = Store::scan(&s.dirs).unwrap();
    let ytdlp = s.dirs.ytdlp();
    let temp = s.dir.path().join("temp");
    let fetcher = Fetcher {
        store: &ytdlp,
        temp: &temp,
        partial: &temp,
        plugins: None,
        options: &crate::settings::Ytdlp::default(),
        live: false,
        runs: Cell::new(0),
    };
    let net = Net::default();
    let mut out = Vec::new();
    let (_, failed) = Acquire {
        runner: &net,
        fetcher: &fetcher,
        out: &mut out,
        known: BTreeSet::new(),
        removed: BTreeSet::new(),
        store: &store,
    }
    .missing(&manifest, &BTreeMap::new(), false)
    .unwrap();
    let downloads = net.downloads.lock().unwrap();
    let format_of = |url: &str| {
        let run = downloads
            .iter()
            .find(|d| d.iter().any(|a| a.ends_with(url)))?;
        let at = run.iter().position(|a| a == "--format")?;
        Some(run[at + 1].clone())
    };
    assert_eq!(
        format_of("vid00000009").as_deref(),
        Some("399+251/bv*+251/251/bv*+ba/b"),
        "YouTube's own IDs first"
    );
    assert_eq!(format_of("vid00000010").as_deref(), Some("bv*+ba/b"));
    assert_eq!(
        format_of("archive.example/details/item0001").as_deref(),
        Some("bv*+ba/b"),
        "fetched again from its page, in whatever format the site lists now"
    );
    assert_eq!(downloads.len(), 2);
    assert_eq!(failed.len(), 1, "the fake fetches no page but YouTube's");
}
