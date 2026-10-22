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

use muman::download::{Places, upstream_command, ytdlp_command};
use muman::runner::{Line, Runner, System};
use muman::settings::Ytdlp;

const MEDIA: &[u8] = include_bytes!("../testdata/silence.m4a");

/// Serve `MEDIA` at `/silence.m4a` to every request on a loopback port,
/// answering whole whatever the range asked, until the process ends.
fn serve() -> String {
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
                MEDIA.len()
            );
            if !head {
                let _ = stream.write_all(MEDIA);
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
    let places = Places {
        store: &store,
        temp: &temp,
        done: &done,
        plugins: Some(&plugins),
        options: &options,
        batch: None,
    };
    let url = serve();
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
    assert_eq!(info.formats.len(), 1, "{text}");
    assert!(info.audio_format().is_some(), "{text}");

    let listing = upstream_command(&options, std::slice::from_ref(&url));
    let (ok, text) = lines(&runner, &listing);
    assert!(ok, "yt-dlp failed:\n{text}");
    assert!(text.contains("\"id\": \"silence\""), "{text}");
}
