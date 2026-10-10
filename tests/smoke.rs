//! muman end to end, against the real ffmpeg on this machine: songs made
//! from generated audio added, matched by fingerprint, written, checked,
//! removed and put back. Ignored by default, since it needs ffmpeg; CI
//! runs it on every platform with `cargo test --test smoke -- --ignored`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use lofty::file::TaggedFileExt;
use lofty::tag::Accessor;

struct Home {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        // No lookups: the test must not reach LRCLIB, AcoustID, the Cover Art
        // Archive or YouTube.
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::write(
            root.join("home").join("songs.toml"),
            "version = 1\n\n[providers.lrclib]\nenabled = false\n\n\
             [providers.acoustid]\nenabled = false\n\n\
             [providers.coverart]\nenabled = false\n",
        )
        .unwrap();
        Self { _dir: dir, root }
    }

    fn muman(&self, args: &[&str]) -> Output {
        let out = Command::new(env!("CARGO_BIN_EXE_muman"))
            .arg("--home")
            .arg(self.root.join("home"))
            .arg("--library")
            .arg(self.root.join("library"))
            .args(args)
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        eprintln!(
            "muman {}: {}\n{}{}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    /// Thirty seconds of seeded noise shaped into a changing tone, tagged:
    /// the same seed is the same recording, another seed another song.
    fn song(&self, name: &str, seed: u32, title: &str) -> PathBuf {
        let path = self.root.join("in").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let ok = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!(
                "anoisesrc=d=30:c=pink:r=44100:a=0.4:seed={seed},\
                 tremolo=f={}:d=0.8,highpass=f=200",
                2 + seed % 5
            ))
            .args(["-ac", "2", "-metadata"])
            .arg(format!("title={title}"))
            .args([
                "-metadata",
                "artist=Paper Comets",
                "-metadata",
                "album=Harbor Lights",
            ])
            .arg(&path)
            .status()
            .expect("ffmpeg runs")
            .success();
        assert!(ok, "ffmpeg made {name}");
        path
    }

    fn library_files(&self, ext: &str) -> Vec<PathBuf> {
        let mut found = Vec::new();
        walk(&self.root.join("library"), &mut found);
        found.retain(|p| p.extension().is_some_and(|e| e == ext));
        found
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn title(path: &Path) -> String {
    let file = lofty::read_from_path(path).unwrap();
    file.primary_tag()
        .and_then(|t| t.title().map(|s| s.to_string()))
        .unwrap_or_default()
}

fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

#[test]
#[ignore = "needs ffmpeg and ffprobe"]
fn songs_are_added_matched_written_removed_and_put_back() {
    let h = Home::new();
    let first = h.song("first.flac", 1, "Lantern Weather");
    let copy = h.song("copy.wav", 1, "Lantern Weather");
    let other = h.song("other.flac", 2, "Low Orchard");

    assert_eq!(
        h.muman(&["add", "-y"]).status.code(),
        Some(2),
        "add needs inputs"
    );
    let added = h.muman(&[
        "add",
        "-y",
        first.to_str().unwrap(),
        other.to_str().unwrap(),
    ]);
    assert!(added.status.success(), "{}", text(&added));
    let written = h.library_files("flac");
    assert_eq!(written.len(), 2, "{written:?}");
    let mut titles: Vec<String> = written.iter().map(|p| title(p)).collect();
    titles.sort();
    assert_eq!(titles, ["Lantern Weather", "Low Orchard"]);

    // The same audio again joins its song rather than making a third.
    let joined = h.muman(&["add", "-y", copy.to_str().unwrap()]);
    assert!(joined.status.success(), "{}", text(&joined));
    let listed = h.muman(&["list"]);
    assert_eq!(
        String::from_utf8_lossy(&listed.stdout).lines().count(),
        2,
        "{}",
        text(&listed)
    );
    assert_eq!(h.library_files("flac").len(), 2);

    let checked = h.muman(&["check"]);
    assert!(checked.status.success(), "{}", text(&checked));

    let removed = h.muman(&["remove", "-y", "title:Orchard"]);
    assert!(removed.status.success(), "{}", text(&removed));
    assert_eq!(h.library_files("flac").len(), 1);

    let undone = h.muman(&["undo", "-y"]);
    assert!(undone.status.success(), "{}", text(&undone));
    assert_eq!(h.library_files("flac").len(), 2);

    // A new layout moves the files without writing them again.
    let songs = h.root.join("home").join("songs.toml");
    let list = std::fs::read_to_string(&songs).unwrap();
    std::fs::write(
        &songs,
        list.replacen(
            "\ntemplate = \"{{ album_artist }}/{{ album }}/{{ disc_track }}{{ title }}\"\n",
            "\ntemplate = \"{{ artist }} - {{ title }}\"\n",
            1,
        ),
    )
    .unwrap();
    let moved = h.muman(&["sync"]);
    assert!(moved.status.success(), "{}", text(&moved));
    assert!(text(&moved).contains("Moved"), "{}", text(&moved));
    assert!(
        h.root
            .join("library")
            .join("Paper Comets - Lantern Weather.flac")
            .is_file()
    );
}

/// The integrated loudness ffmpeg's own meter gives `path`, in LUFS.
fn ffmpeg_lufs(path: &Path) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(path)
        .args(["-af", "ebur128", "-f", "null", "-"])
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&out.stderr);
    let summary = &log[log.rfind("Integrated loudness").expect("a summary")..];
    summary
        .split_whitespace()
        .skip_while(|w| *w != "I:")
        .nth(1)
        .and_then(|n| n.parse().ok())
        .expect("a loudness")
}

fn tag(path: &Path, key: &str) -> Option<String> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries"])
        .arg(format!("format_tags={key}:stream_tags={key}"))
        .args(["-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find(|l| !l.trim().is_empty())
        .map(str::to_string)
}

#[test]
#[ignore = "needs ffmpeg and ffprobe"]
fn songs_are_levelled_as_ffmpeg_measures_them_and_opus_by_its_header() {
    let h = Home::new();
    let song = h.song("song.flac", 3, "Lantern Weather");
    let added = h.muman(&["add", "-y", song.to_str().unwrap()]);
    assert!(added.status.success(), "{}", text(&added));
    let written = &h.library_files("flac")[0];
    let gain: f64 = tag(written, "REPLAYGAIN_TRACK_GAIN")
        .expect("a gain")
        .trim_end_matches(" dB")
        .parse()
        .unwrap();
    let measured = ffmpeg_lufs(written);
    assert!(
        (gain - (-18.0 - measured)).abs() <= 0.1,
        "{gain} dB against {measured} LUFS"
    );

    // Opus copied with its own gain in its header plays at the target.
    let opus = h.song("lone.opus", 4, "Low Orchard");
    let songs = h.root.join("home").join("songs.toml");
    let list = std::fs::read_to_string(&songs).unwrap();
    std::fs::write(
        &songs,
        list.replacen("\nmode = \"tags\"\n", "\nmode = \"header\"\n", 1)
            .replacen("\nscope = \"album\"\n", "\nscope = \"track\"\n", 1),
    )
    .unwrap();
    let added = h.muman(&["add", "-y", opus.to_str().unwrap()]);
    assert!(added.status.success(), "{}", text(&added));
    let written = &h.library_files("opus")[0];
    let lufs = ffmpeg_lufs(written);
    assert!((lufs + 18.0).abs() <= 0.2, "{lufs} LUFS");
    assert_eq!(tag(written, "R128_TRACK_GAIN").as_deref(), Some("-1280"));
}

/// `out` made by ffmpeg from `args`, or the test fails.
fn ffmpeg(args: &[&str], out: &Path) {
    let ok = Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .arg(out)
        .status()
        .expect("ffmpeg runs")
        .success();
    assert!(ok, "ffmpeg made {}", out.display());
}

fn listed_by(h: &Home, query: &str) -> String {
    let out = h.muman(&["list", "--keys", query]);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
#[ignore = "needs ffmpeg and ffprobe"]
fn lyrics_pictures_and_cue_sheets_are_given_to_their_songs() {
    let h = Home::new();
    let dir = h.root.join("in");
    let first = h.song("First Light.flac", 1, "First Light");
    let second = h.song("Second Light.flac", 2, "Second Light");
    let third = h.song("Third Light.flac", 3, "Third Light");
    let art = dir.join("art.png");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "mandelbrot=s=600x600",
            "-frames:v",
            "1",
        ],
        &art,
    );
    for song in [&first, &second] {
        let covered = song.with_extension("covered.flac");
        let (song, art) = (song.to_str().unwrap(), art.to_str().unwrap());
        ffmpeg(
            &[
                "-i",
                song,
                "-i",
                art,
                "-map",
                "0",
                "-map",
                "1",
                "-c",
                "copy",
                "-disposition:v",
                "attached_pic",
            ],
            &covered,
        );
        std::fs::rename(&covered, song).unwrap();
    }
    let added = h.muman(&[
        "add",
        "--new",
        first.to_str().unwrap(),
        second.to_str().unwrap(),
        third.to_str().unwrap(),
    ]);
    assert!(added.status.success(), "{}", text(&added));

    // A smaller copy of the cover, framed in a 16:9 thumbnail's bars.
    let boxed = h.root.join("elsewhere").join("thumbnail.jpg");
    std::fs::create_dir_all(boxed.parent().unwrap()).unwrap();
    ffmpeg(
        &[
            "-i",
            art.to_str().unwrap(),
            "-vf",
            "scale=360:360,pad=640:360:140:0:black",
            "-q:v",
            "4",
        ],
        &boxed,
    );
    let given = h.muman(&["add", boxed.to_str().unwrap()]);
    assert!(given.status.success(), "{}", text(&given));
    let with_it = listed_by(&h, "key:added/thumbnail.jpg");
    assert!(
        with_it.contains("First Light.flac") && with_it.contains("Second Light.flac"),
        "{with_it}"
    );
    assert!(!with_it.contains("Third Light.flac"), "{with_it}");

    let lrc = h.root.join("elsewhere").join("words.lrc");
    std::fs::write(
        &lrc,
        "[ti:Second Light]\n[ar:Paper Comets]\n[length:00:30.00]\n[00:02.00]a lantern on the water\n",
    )
    .unwrap();
    let given = h.muman(&["add", lrc.to_str().unwrap()]);
    assert!(given.status.success(), "{}", text(&given));
    assert!(listed_by(&h, "key:added/words.lrc").contains("Second Light.flac"));
    assert_eq!(
        h.library_files("lrc").len(),
        1,
        "{:?}",
        h.library_files("lrc")
    );

    let cue = h.root.join("elsewhere").join("album.cue");
    std::fs::write(
        &cue,
        "REM GENRE \"Chamber Pop\"\nPERFORMER \"Paper Comets\"\nTITLE \"Harbor Lights\"\n\
         FILE \"First Light.flac\" WAVE\n  TRACK 01 AUDIO\n    TITLE \"First Light\"\n    INDEX 01 00:00:00\n\
         FILE \"Third Light.flac\" WAVE\n  TRACK 02 AUDIO\n    TITLE \"Third Light\"\n    INDEX 01 00:00:00\n",
    )
    .unwrap();
    let given = h.muman(&["add", cue.to_str().unwrap()]);
    assert!(given.status.success(), "{}", text(&given));
    let genres: Vec<Option<String>> = h
        .library_files("flac")
        .iter()
        .map(|p| tag(p, "genre"))
        .collect();
    assert_eq!(
        genres
            .iter()
            .filter(|g| g.as_deref() == Some("Chamber Pop"))
            .count(),
        2,
        "{genres:?}"
    );
}
