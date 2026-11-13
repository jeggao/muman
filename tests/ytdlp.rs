//! yt-dlp handed the command line muman builds, against a file served
//! from this process, so an option yt-dlp renames or drops fails here
//! and not on a user's first fetch; the file it fetched is then scrubbed
//! of its info JSON's addresses and headers, as yt-dlp writes them.
//! Ignored by default, since it needs
//! yt-dlp and ffmpeg; CI runs it on every platform with
//! `cargo test --test ytdlp -- --ignored`. Nothing leaves the machine:
//! the one address fetched is the loopback server, and the generic
//! extractor takes a direct media file as one video.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;

use muman::download::{Passed, Places, ending, upstream_command, ytdlp_command};
use muman::runner::{Line, Runner, System};
use muman::settings::Ytdlp;

const MEDIA: &[u8] = include_bytes!("../testdata/silence.m4a");

/// Serve `media` at `/silence.m4a` to every request on a loopback port,
/// answering whole whatever the range asked, until the process ends.
fn serve(media: Vec<u8>) -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let head = String::from_utf8_lossy(&request[..read]).starts_with("HEAD");
            let _ = write!(
                stream,
                "HTTP/1.0 200 OK\r\nContent-Type: audio/mp4\r\nContent-Length: {}\r\n\
                 Accept-Ranges: none\r\nConnection: close\r\n\r\n",
                media.len()
            );
            if !head {
                let _ = stream.write_all(&media);
            }
        }
    });
    format!("http://127.0.0.1:{port}/silence.m4a")
}

fn lines(runner: &System, cmd: &[OsString]) -> (bool, String) {
    let mut text = String::new();
    let ok = runner
        .stream(cmd, &mut |line| {
            let (Line::Out(l) | Line::Err(l)) = line;
            text.push_str(l);
            text.push('\n');
        })
        .expect("yt-dlp spawns");
    (ok, text)
}

#[test]
#[ignore = "needs yt-dlp and ffmpeg"]
fn yt_dlp_takes_every_option_muman_passes() {
    let runner = System::default();
    for tool in ["ffmpeg", "ffprobe", "yt-dlp"] {
        runner.require(tool).expect("found, at or above its floor");
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (store, temp, done) = (root.join("store"), root.join("temp"), root.join("done"));
    let plugins = muman::plugins::folder(&root.join("cache")).unwrap();
    let options = Ytdlp::default();
    let passed = Passed::write(&options, root).unwrap();
    let places = Places {
        store: &store,
        temp: &temp,
        done: &done,
        plugins: Some(&plugins),
        options: &options,
        passed: &passed,
        batch: None,
    };
    let plain = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("silence.m4a");
    let tagged = root.join("tagged.m4a");
    let mut tag: Vec<OsString> = ["ffmpeg", "-v", "error", "-i"].map(Into::into).into();
    tag.push(plain.into_os_string());
    for arg in [
        "-c",
        "copy",
        "-metadata",
        "title=Night Ferry",
        "-metadata",
        "date=1997",
    ] {
        tag.push(arg.into());
    }
    tag.push(tagged.clone().into_os_string());
    runner.run(&tag).expect("ffmpeg tags the file served");
    let url = serve(std::fs::read(&tagged).unwrap());
    let cmd = ytdlp_command(&places, None, std::slice::from_ref(&url));
    let (ok, text) = lines(&runner, &cmd);
    assert!(ok, "yt-dlp failed:\n{text}");
    let finished = std::fs::read_to_string(&done).expect("yt-dlp lists what it finished");
    let line = finished.lines().next().unwrap_or_default();
    // The ID and the path are JSON-quoted, as `%(id)j` and `%(filepath)j`
    // write them.
    let quoted = line
        .strip_prefix("Generic \"silence\" ")
        .unwrap_or_else(|| panic!("{finished}"));
    let file: String = serde_json::Deserializer::from_str(quoted)
        .into_iter()
        .next()
        .unwrap()
        .unwrap();
    let file = Path::new(&file);
    assert!(
        file.starts_with(&store) && file.is_file(),
        "{}",
        file.display()
    );
    let scratch = root.join("scratch");
    assert!(muman::download::scrub(&runner, file, &scratch).expect("scrubbed"));
    let kept = std::fs::read(scratch.join("scrub").join("info.json")).unwrap();
    let text = String::from_utf8_lossy(&kept);
    assert!(
        !text.contains("\"url\"") && !text.contains("http_headers"),
        "{text}"
    );
    let info = muman::info::parse(&kept).unwrap();
    assert_eq!(info.id.as_deref(), Some("silence"));
    assert!(info.muman_own_tags.is_some(), "{text}");
    let own = muman::probe::parse(&runner.output(&muman::probe::ffprobe_command(file)).unwrap())
        .unwrap()
        .tags;
    assert_eq!(
        own.get("title").map(String::as_str),
        Some("Night Ferry"),
        "{own:?}"
    );
    assert_eq!(own.get("date").map(String::as_str), Some("1997"), "{own:?}");
    assert_eq!(info.formats.len(), 1, "{text}");
    assert!(info.audio_format().is_some(), "{text}");

    let listing = upstream_command(&passed, std::slice::from_ref(&url));
    let (ok, text) = lines(&runner, &listing);
    assert!(ok, "yt-dlp failed:\n{text}");
    assert!(text.contains("\"id\": \"silence\""), "{text}");
    listed_as_fetched(&runner, file, &text, &root.join("hashed"));
}

/// Assert that `listing`, yt-dlp's listing of `file` just fetched, lists
/// the tag fields it was fetched with, as `check --upstream` compares
/// them, its own tags apart; hashed in `scratch`.
fn listed_as_fetched(runner: &System, file: &Path, listing: &str, scratch: &Path) {
    let listed = listing
        .lines()
        .find_map(|l| muman::info::tags_digest(l.as_bytes()))
        .unwrap_or_else(|| panic!("{listing}"));
    let located = muman::store::Located {
        key: muman::source::SourceKey::parse("generic:silence").unwrap(),
        path: file.to_path_buf(),
        kind: muman::store::Kind::Media,
        lyrics: None,
        covers: Vec::new(),
    };
    let (digests, served) = muman::facts::hash(runner, &located, scratch).unwrap();
    let mut facts = muman::facts::Facts::unreadable("r".into());
    facts.take(&digests, served);
    let held = muman::held::Held::now(&located.key, &facts, &muman::sites::Sites::default())
        .expect("a record");
    assert!(
        held.listed.is_some(),
        "its own tags are held beside the listing's: {held:?}"
    );
    assert!(
        !held.tags_differ(&listed),
        "listed again, the file just fetched serves the tags it was fetched with: {held:?}"
    );
}

#[test]
#[ignore = "needs yt-dlp"]
fn the_song_list_s_arguments_reach_yt_dlp_from_its_config_file_as_written() {
    let runner = System::default();
    runner
        .require("yt-dlp")
        .expect("found, at or above its floor");
    let dir = tempfile::tempdir().unwrap();
    let written = "it's \"Marlo Venn\" # not a comment $HOME \\ ☂\non a line of its own";
    let options = Ytdlp {
        args: vec!["--print".into(), written.into()],
        ..Ytdlp::default()
    };
    let passed = Passed::write(&options, dir.path()).unwrap();
    let own = ["yt-dlp", "--ignore-config", "--simulate"].map(OsString::from);
    let cmd = ending(own.into(), &passed, &serve(MEDIA.to_vec()));
    assert!(!cmd.iter().any(|a| a.to_string_lossy().contains("Marlo")));
    let (ok, text) = lines(&runner, &cmd);
    assert!(ok, "yt-dlp failed:\n{text}");
    assert_eq!(text.trim_end(), written);
}
