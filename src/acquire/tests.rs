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
            ("https://www.jamendo.com/track/1234567", false) => {
                r#"{"_type": "video", "id": "1234567", "extractor_key": "Jamendo"}"#
            }
            ("https://www.tunes.example/t/item0001", false) => {
                r#"{"_type": "video", "id": "item0001", "extractor_key": "Funkwhale"}"#
            }
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
                "Youtube \"{id}\" {}",
                serde_json::to_string(&path).unwrap()
            );
        }
        std::fs::write(done, lines).unwrap();
        Ok(true)
    }
}

fn listed(urls: &[&str]) -> Listed {
    let urls: Vec<String> = urls.iter().map(|u| (*u).to_string()).collect();
    list(
        &urls,
        &crate::sites::Sites::default(),
        &Ytdlp::default(),
        &Net::default(),
        &mut Vec::new(),
    )
    .unwrap()
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
fn a_video_of_any_site_named_alone_is_named_by_its_key() {
    let l = listed(&[
        "https://v",
        "https://www.jamendo.com/track/1234567",
        "https://www.tunes.example/t/item0001",
    ]);
    let keys = [
        "youtube.com:vid00000009",
        "jamendo.com:1234567",
        "tunes.example:item0001",
    ]
    .map(|k| SourceKey::parse(k).unwrap());
    assert_eq!(l.named, BTreeSet::from(keys));
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
        sites: &crate::sites::Sites::default(),
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
    assert_eq!(add.proposals[0].label, "vid00000010", "named by its key");
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
        "version = 1\n[[song]]\nsources = [\"youtube.com:vid00000009\", \"manual:gone.flac\"]\n",
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
        sites: &crate::sites::Sites::default(),
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
            sites: &crate::sites::Sites::default(),
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
fn a_fetched_file_its_place_cannot_take_fails_the_fetch_and_is_not_left() {
    let s = setup();
    let ytdlp = s.dirs.ytdlp();
    std::fs::create_dir_all(ytdlp.join("youtube.com").join("vid00000009.mkv")).unwrap();
    let store = Store::scan(&s.dirs).unwrap();
    let temp = s.dir.path().join("temp");
    let fetcher = Fetcher {
        store: &ytdlp,
        temp: &temp,
        partial: &temp,
        plugins: None,
        sites: &crate::sites::Sites::default(),
        options: &crate::settings::Ytdlp::default(),
        live: false,
        runs: Cell::new(0),
    };
    let mut out = Vec::new();
    let (ok, add) = Acquire {
        runner: &Net::default(),
        fetcher: &fetcher,
        out: &mut out,
        known: BTreeSet::new(),
        removed: BTreeSet::new(),
        store: &store,
    }
    .urls(&["https://v".to_string()], false)
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!ok, "{text}");
    assert!(add.proposals.is_empty(), "{text}");
    assert!(text.contains("youtube.com:vid00000009: not kept"), "{text}");
    assert!(!text.contains("Nothing new"), "{text}");
    let staged = crate::store::files_below(&ytdlp.join(crate::store::STAGING), 9, |_| true);
    assert!(staged.unwrap_or_default().is_empty());
}

#[test]
fn a_source_fetched_again_asks_for_the_format_and_page_its_song_records() {
    let s = setup();
    std::fs::create_dir_all(&s.dirs.home).unwrap();
    std::fs::write(
        s.dirs.manifest(),
        "version = 1\n\
         [[song]]\nsources = [\"youtube.com:vid00000009\"]\n\
         held.\"youtube.com:vid00000009\" = { audio = \"0123456789abcdef\", format = \"399+251\" }\n\
         [[song]]\nsources = [\"youtube.com:vid00000010\"]\n\
         [[song]]\nsources = [\"tunes.example:item0001\"]\n\
         held.\"tunes.example:item0001\" = { audio = \"0123456789abcdef\", format = \"1\", \
         url = \"https://tunes.example/t/item0001\" }\n\
         [[song]]\nsources = [\"archive.org:item0002\"]\n",
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
        sites: &crate::sites::Sites::default(),
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
        Some("399+251/bv*+251/251/ba/b"),
        "YouTube's own IDs first"
    );
    assert_eq!(format_of("vid00000010").as_deref(), Some("ba/b"));
    assert_eq!(
        format_of("tunes.example/t/item0001").as_deref(),
        Some("ba/b"),
        "fetched again from its page, in whatever format the site lists now"
    );
    assert_eq!(
        format_of("https://archive.org/details/item0002").as_deref(),
        Some("ba/b"),
        "fetched again from the address `[sites]` gives"
    );
    assert_eq!(downloads.len(), 2);
    assert_eq!(failed.len(), 2, "the fake fetches no page but YouTube's");
}

/// yt-dlp as a run of it that cannot load muman's postprocessors, or
/// that reads YouTube no longer: each run's arguments kept.
struct Failing {
    runs: Mutex<Vec<Vec<String>>>,
    line: &'static str,
    /// Whether a run without `--plugin-dirs` succeeds.
    bare_succeeds: bool,
}

impl Runner for Failing {
    fn run(&self, _: &[OsString]) -> Result<()> {
        anyhow::bail!("not in tests")
    }

    fn output(&self, _: &[OsString]) -> Result<Vec<u8>> {
        anyhow::bail!("not in tests")
    }

    fn stream(&self, cmd: &[OsString], on_line: &mut dyn FnMut(Line<'_>)) -> Result<bool> {
        let args: Vec<String> = cmd
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let plugins = args.iter().any(|a| a == "--plugin-dirs");
        self.runs.lock().unwrap().push(args);
        if plugins || !self.bare_succeeds {
            on_line(Line::Err(self.line));
            return Ok(false);
        }
        Ok(true)
    }
}

fn fetch_with(s: &Setup, runner: &Failing, plugins: Option<&Path>) -> (bool, String) {
    let ytdlp = s.dirs.ytdlp();
    let temp = s.dir.path().join("temp");
    let fetcher = Fetcher {
        store: &ytdlp,
        temp: &temp,
        partial: &temp,
        plugins,
        sites: &crate::sites::Sites::default(),
        options: &crate::settings::Ytdlp::default(),
        live: false,
        runs: Cell::new(0),
    };
    let mut out = Vec::new();
    let (ok, _) = fetcher
        .fetch(runner, &mut out, None, &["https://v".to_string()])
        .unwrap();
    (ok, crate::ui::plain(&String::from_utf8(out).unwrap()))
}

#[test]
fn postprocessors_that_do_not_load_are_left_out_of_a_second_run() {
    let s = setup();
    let runner = Failing {
        runs: Mutex::new(Vec::new()),
        line: "ValueError: Invalid plugin directory: /plugins",
        bare_succeeds: true,
    };
    let (ok, text) = fetch_with(&s, &runner, Some(Path::new("/plugins")));
    assert!(ok, "{text}");
    assert!(text.contains("postprocessors did not load"), "{text}");
    let runs = runner.runs.lock().unwrap();
    assert_eq!(runs.len(), 2);
    assert!(runs[0].iter().any(|a| a == "--plugin-dirs"));
    assert!(
        !runs[1]
            .iter()
            .any(|a| a == "--plugin-dirs" || a == "--use-postprocessor")
    );
}

#[test]
fn a_fetch_a_newer_yt_dlp_would_make_says_to_update_it() {
    let s = setup();
    let runner = Failing {
        runs: Mutex::new(Vec::new()),
        line: "ERROR: [youtube] vid00000009: Sign in to confirm you’re not a bot.",
        bare_succeeds: false,
    };
    let (ok, text) = fetch_with(&s, &runner, None);
    assert!(!ok);
    assert!(text.contains("update it (`yt-dlp -U`"), "{text}");
    assert_eq!(
        runner.runs.lock().unwrap().len(),
        1,
        "no second run without plugins"
    );
}

/// YouTube Music as `lookup` asks it, trimmed from what yt-dlp answers:
/// the songs search, each track extracted whole, and each video's audio.
struct Music;

/// The upload: a label's `Artist - Song` audio, six seconds longer than
/// the release for its intro and outro.
const UPLOAD: &str = r#"{"id": "vid00000020", "title": "Marlo Venn - Lantern Weather (Official Audio)",
    "uploader": "Marlo Venn", "channel": "Marlo Venn", "duration": 66, "extractor_key": "Youtube"}"#;

/// The release names its version, which the upload's title does not.
const RELEASE: &str = r#"{"id": "vid00000021", "title": "Lantern Weather (Original Version)",
    "track": "Lantern Weather (Original Version)", "artists": ["Marlo Venn"],
    "artist": "Marlo Venn", "album": "The Glass Orchards", "uploader": "Marlo Venn",
    "channel": "Marlo Venn", "duration": 60, "extractor_key": "Youtube"}"#;

const LIVE: &str = r#"{"id": "vid00000022", "title": "Lantern Weather (Live)",
    "track": "Lantern Weather (Live)", "artists": ["Marlo Venn"], "uploader": "Marlo Venn",
    "channel": "Marlo Venn", "duration": 64, "extractor_key": "Youtube"}"#;

/// `seconds` of 8 kHz noise whose loudness changes every 50 ms, as
/// 16-bit little-endian samples.
fn noise(seconds: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        i32::try_from(state >> 49).unwrap()
    };
    let mut pcm = Vec::new();
    for _ in 0..seconds * 20 {
        let level = next() % 12_000;
        for _ in 0..400 {
            let sample = i16::try_from(level * (next() % 3 - 1)).unwrap();
            pcm.extend(sample.to_le_bytes());
        }
    }
    pcm
}

impl Runner for Music {
    fn run(&self, _: &[OsString]) -> Result<()> {
        anyhow::bail!("not in tests")
    }

    fn output(&self, cmd: &[OsString]) -> Result<Vec<u8>> {
        let args: Vec<String> = cmd
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let last = args.last().map(String::as_str).unwrap_or_default();
        let id = last.rsplit("v=").next().unwrap_or_default();
        let out: String = if last.starts_with("https://music.youtube.com/search?q=") {
            assert!(
                last.contains("Marlo%20Venn%20Marlo%20Venn%20-%20Lantern"),
                "{last}"
            );
            "vid00000022\nvid00000021\nvid00000020\n".into()
        } else if args.iter().any(|a| a == "after_move:filepath") {
            format!("/audio/{id}.opus\n")
        } else if args[0] == "ffmpeg" {
            let input = &args[args.iter().position(|a| a == "-i").unwrap() + 1];
            let track = noise(60, 7);
            return Ok(
                match Path::new(input).file_stem().and_then(|s| s.to_str()) {
                    Some("vid00000020") => [noise(1, 1), track, noise(5, 2)].concat(),
                    Some("vid00000021") => track,
                    _ => noise(64, 3),
                },
            );
        } else {
            let json = match id {
                "vid00000021" => RELEASE,
                "vid00000022" => LIVE,
                _ => anyhow::bail!("offline"),
            };
            // yt-dlp prints each object on one line.
            serde_json::from_str::<serde_json::Value>(json)?.to_string()
        };
        Ok(out.into_bytes())
    }

    fn stream(&self, _: &[OsString], _: &mut dyn FnMut(Line<'_>)) -> Result<bool> {
        anyhow::bail!("not in tests")
    }
}

#[test]
fn a_release_naming_its_version_is_found_by_its_audio() {
    let dir = tempfile::tempdir().unwrap();
    let upload: Entry = serde_json::from_str(UPLOAD).unwrap();
    let found = lookup(
        &Music,
        &crate::sites::Sites::default(),
        &crate::settings::Ytdlp::default(),
        &upload,
        dir.path(),
    )
    .unwrap();
    let (track, aligned) = found.expect("the release is found");
    assert_eq!(track.id, "vid00000021", "the live take is passed over");
    let aligned = aligned.expect("compared by its audio");
    assert!(aligned.fits(), "{aligned:?}");
    assert_eq!(aligned.offset_ms, 1000);
}
