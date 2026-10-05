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
        // No lookups: the test must not reach LRCLIB or YouTube.
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::write(
            root.join("home").join("songs.toml"),
            "version = 1\n\n[providers.lrclib]\nenabled = false\n",
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
            "version = 1\n",
            "version = 1\n\n[library]\ntemplate = \"{{ artist }} - {{ title }}\"\n",
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
