//! The architecture the code keeps, checked over its source: where files
//! are changed, what records a save of the song list, which state lives
//! for the whole process, where sites are named, which loop fits songs
//! into a size, where audio is mixed, where
//! a file beside a song is named, where ffmpeg is given a file to read,
//! where what the user gives is walked and copied, where a provider is
//! asked and named, where text is written to a terminal, and how a
//! message names a library file. Each rule
//! names the one place a thing is done; code that does it anywhere else
//! fails here until the rule, and the reason beside it, is changed on
//! purpose.
//!
//! Only code that ships is read: a file's inline `mod tests`, the
//! `tests.rs` and `testing.rs` files, and the modules of a `tests/`
//! folder in a `src` are left out.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The `src` folders the code is in: the root crate's, then each
/// `crates/<crate>/src`.
fn source_roots() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut crates: Vec<PathBuf> = std::fs::read_dir(root.join("crates"))
        .into_iter()
        .flatten()
        .filter_map(|e| Some(e.ok()?.path().join("src")))
        .filter(|src| src.is_dir())
        .collect();
    crates.sort();
    [vec![root.join("src")], crates].concat()
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// Every Rust file of the code, with the `src` it is in.
fn files() -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for src in source_roots() {
        let mut found = Vec::new();
        walk(&src, &mut found);
        out.extend(found.into_iter().map(|f| (src.clone(), f)));
    }
    out
}

/// Whether `path`, in `src`, holds tests alone.
fn tests_alone(src: &Path, path: &Path) -> bool {
    let name = path.file_name().unwrap();
    name == "tests.rs" || name == "testing.rs" || path.starts_with(src.join("tests"))
}

/// `path` from the repository's root, its parts joined by `/` on every
/// platform: two crates' files of one name stay apart.
fn key(path: &Path) -> String {
    path.strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Every source file under `src`, and under each `crates/<crate>/src`,
/// by its [`key`], without its tests.
fn sources() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (src, path) in files() {
        if tests_alone(&src, &path) {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let shipped = match text.find("#[cfg(test)]\nmod tests {") {
            Some(at) => text[..at].to_string(),
            None => text,
        };
        out.insert(key(&path), shipped);
    }
    out
}

/// Each file that ships is read once, under its own key, so every rule
/// holds it.
#[test]
fn every_file_that_ships_is_read() {
    let read = sources();
    let shipped: Vec<String> = files()
        .into_iter()
        .filter(|(src, path)| !tests_alone(src, path))
        .map(|(_, path)| key(&path))
        .collect();
    let unread: Vec<&String> = shipped
        .iter()
        .filter(|f| !read.contains_key(f.as_str()))
        .collect();
    assert!(
        unread.is_empty() && read.len() == shipped.len(),
        "{} files ship, {} are read; unread: {unread:?}",
        shipped.len(),
        read.len()
    );
}

/// The lines of `text` holding any of `needles`, comments aside.
fn lines_with<'a>(text: &'a str, needles: &[&str]) -> Vec<&'a str> {
    text.lines()
        .filter(|l| {
            let code = l.trim_start();
            !code.starts_with("//") && needles.iter().any(|n| code.contains(n))
        })
        .collect()
}

/// Every file that may change files, and why it may. A call that changes
/// a file is one of the std calls clippy's `disallowed-methods` names,
/// which shipped code makes only where an `#[expect]` says why, or one of
/// `atomic`'s primitives; a file the list does not name makes none.
const MUTATIONS: &[(&str, &str)] = &[
    (
        "crates/muman-core/src/atomic.rs",
        "the primitives every other write goes through, a second name for a file, scratch folders, the parts a stopped write left, and the library's lock file as it is let go",
    ),
    (
        "crates/muman-engine/src/library.rs",
        "the ledger: every move and removal in the library, recorded first, \
         and a song's staged files put in place over only what it proved muman's",
    ),
    (
        "crates/muman-engine/src/render.rs",
        "a song's new files staged beside their paths, whatever was at a staged \
         name removed first, and those staged files removed when the song fails \
         or the files beside it, written again alone, cannot be put in place",
    ),
    (
        "crates/muman-engine/src/history.rs",
        "run records, the files kept for undo, undo putting them back or removing lyrics it added and the copies `add` made, and an undone run's record let go of once its sync ends",
    ),
    (
        "crates/muman-core/src/store.rs",
        "an upload its release replaced, deleted from the sources; a file \
         fetched moved to its key's place, replacing the key's other; a \
         file a site renamed named at its new key's place, replacing \
         another there, and its old name removed once the list names the \
         new; a torrent's file aria2c finished moved to its key's place",
    ),
    (
        "crates/muman-net/src/aria2.rs",
        "the metainfo a magnet link's peers sent, dropped once read; a \
         torrent's unfinished folder, once its files are placed; a \
         damaged copy copied in to be checked; the input file of a run, \
         in scratch",
    ),
    ("src/purge.rs", "sources no song uses, deleted on request"),
    (
        "crates/muman-engine/src/state.rs",
        "state.json brought into state.db once: the store made from it \
         linked or renamed into place, its other name and what a stopped \
         import left removed, and state.json kept as state.json.bak",
    ),
    (
        "src/change.rs",
        "a removed song's fetched source, deleted with `remove --purge`",
    ),
    (
        "crates/muman-core/src/given.rs",
        "files copied in by `add`, and a download taken up",
    ),
    (
        "src/add.rs",
        "a file fetched before, removed again when its info JSON cannot be cleared; \
         the files an add copied, removed again when it is refused partway",
    ),
    ("src/sync.rs", "downloads left unfinished too long"),
    (
        "crates/muman-engine/src/acquire.rs",
        "the URL list and download archive handed to yt-dlp, in scratch",
    ),
    (
        "crates/muman-net/src/download.rs",
        "a file just fetched or copied in, written again without the cookies \
         and headers of its info JSON, that JSON kept in scratch; a remux left \
         cut short in partial downloads",
    ),
    (
        "src/editor.rs",
        "the file the editor is opened on, in scratch",
    ),
    (
        "src/export.rs",
        "the zip, made in scratch and placed where no file is, or with \
         --force over one; the folder's song list and library, made in \
         scratch inside it and renamed up; each song encoded for it moved \
         or deleted from scratch once it is in, any other copied in",
    ),
];

#[test]
fn files_change_only_where_the_change_is_recorded_or_scratch() {
    let calls = [
        "atomic::remove(",
        "atomic::rename(",
        "atomic::link(",
        "fs::write(",
        "fs::rename(",
        "fs::copy(",
        "hard_link(",
        "fs::remove_file(",
        "fs::remove_dir(",
        "remove_dir_all(",
        "File::create(",
        "OpenOptions::new(",
    ];
    let listed: BTreeSet<&str> = MUTATIONS.iter().map(|(f, _)| *f).collect();
    let changing: BTreeSet<String> = sources()
        .into_iter()
        .filter(|(_, text)| !lines_with(text, &calls).is_empty())
        .map(|(file, _)| file)
        .collect();
    let unlisted: Vec<&String> = changing
        .iter()
        .filter(|f| !listed.contains(f.as_str()))
        .collect();
    let stale: Vec<&&str> = listed.iter().filter(|f| !changing.contains(**f)).collect();
    assert!(
        unlisted.is_empty() && stale.is_empty(),
        "files that change files, unlisted: {unlisted:?}; listed, changing none: {stale:?}"
    );
}

/// A library file is written over or removed only as the ledger proves
/// it muman's: `render.rs`, which writes into the library, renames,
/// copies and writes nothing itself but staged files it makes anew, and
/// puts them in place through `library::put` and `library::clear`, which
/// take an `Owned` no module but the ledger makes; a file not muman's is
/// taken over, for a forced run, in one place.
#[test]
fn a_library_file_is_written_over_only_as_the_ledger_proves_it_muman_s() {
    let sources = sources();
    let render = lines_with(
        &sources["crates/muman-engine/src/render.rs"],
        &[
            "atomic::rename(",
            "atomic::remove(",
            "fs::rename(",
            "fs::copy(",
            "fs::write(",
            "File::create(",
        ],
    );
    assert!(render.is_empty(), "render.rs:\n{}", render.join("\n"));
    let taken: Vec<String> = sources
        .iter()
        .flat_map(|(file, text)| {
            lines_with(text, &["Ledger::taken("])
                .into_iter()
                .map(move |l| format!("{file}: {}", l.trim()))
        })
        .collect();
    assert!(
        taken.len() == 1 && taken[0].starts_with("crates/muman-engine/src/reconcile/owned.rs: "),
        "{}",
        taken.join("\n")
    );
}

/// The state that lives for the whole process, and why it may.
const STATICS: &[(&str, &str)] = &[
    (
        "crates/muman-core/src/progress.rs",
        "the one progress line a terminal shows",
    ),
    (
        "crates/muman-engine/src/history.rs",
        "a count that tells two runs of one process apart",
    ),
    (
        "crates/muman-core/src/runner.rs",
        "the programs a signal stops with muman, and whether one of the user's own answers it",
    ),
    (
        "crates/muman-core/src/kill.rs",
        "the kills and guarded libraries tests arm, compiled for tests alone",
    ),
    (
        "crates/muman-core/src/errors.rs",
        "whether a thread catches a panic, and what the one caught said, for its error",
    ),
];

#[test]
fn no_state_outlives_a_run_but_what_must() {
    let allowed: Vec<&str> = STATICS.iter().map(|(f, _)| *f).collect();
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        for line in lines_with(&text, &["static "]) {
            let mutable = ["Mutex", "RwLock", "Atomic", "Cell<"]
                .iter()
                .any(|t| line.contains(t));
            if mutable && !allowed.contains(&file.as_str()) {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn sites_are_described_by_the_site_table_alone() {
    // A domain as a string of its own; an address holding one is the
    // YouTube Music lookups' own.
    let domain = |s: &str| {
        let s = s.trim_matches('"');
        let labels: Vec<&str> = s.split('.').collect();
        labels.len() >= 2
            && labels.iter().all(|l| {
                !l.is_empty()
                    && l.bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'-' || b.is_ascii_digit())
            })
            && ["com", "org", "net", "jp", "io", "tv", "fm"].contains(labels.last().unwrap())
    };
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if file == "crates/muman-core/src/source.rs" {
            continue;
        }
        for line in text.lines().filter(|l| !l.trim_start().starts_with("//")) {
            for literal in line.split('"').skip(1).step_by(2) {
                if domain(literal) {
                    wrong.push(format!("{file}: \"{literal}\" in {}", line.trim()));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "name sites in new.toml's [sites], not in code:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn songs_fit_a_size_by_one_loop() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if file != "crates/muman-engine/src/fit.rs" && !lines_with(&text, &["allocate("]).is_empty()
        {
            wrong.push(file);
        }
    }
    assert!(
        wrong.is_empty(),
        "fit by `fit::settle`, which measures what it picks: {wrong:?}"
    );
}

#[test]
fn the_song_list_is_written_by_the_manifest_and_undo_alone() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if file != "crates/muman-config/src/manifest.rs"
            && file != "crates/muman-engine/src/history.rs"
            && !lines_with(&text, &["Name::new(MANIFEST)"]).is_empty()
        {
            wrong.push(file);
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// Every recorder a save of the song list may take, where it is made,
/// and why it records the save: a save takes one, so none is made
/// outside a run but these.
const RECORDERS: &[(&str, &str, &str)] = &[
    (
        "crates/muman-config/src/manifest.rs",
        "Unrecorded",
        "the list a new home starts with, before its first run, and tests",
    ),
    (
        "crates/muman-engine/src/history.rs",
        "Run",
        "a run, begun and its record written before the save",
    ),
    (
        "crates/muman-engine/src/history.rs",
        "Recording<'_>",
        "a sync's run, or the undo's, which it saves as",
    ),
    (
        "crates/muman-engine/src/history.rs",
        "Undone",
        "the undo whose sync saves, its record marked and the runs lock held",
    ),
];

/// Where a save may take `Unrecorded`, which no run records, and why. A
/// dry run's sync takes none: its recorder refuses a save.
const UNRECORDED: &[(&str, &str)] = &[(
    "crates/muman-config/src/manifest.rs",
    "`start`, the list a new home starts with, before its first run",
)];

#[test]
fn the_song_list_is_saved_only_as_a_begun_run_records() {
    let mut made = Vec::new();
    let mut unrecorded = Vec::new();
    for (file, text) in sources() {
        for line in lines_with(&text, &["Recorder for "]) {
            let of = line.split("Recorder for ").nth(1).unwrap_or_default();
            made.push((file.clone(), of.trim_end_matches(" {").to_string()));
        }
        let uses = lines_with(&text, &["Unrecorded"])
            .into_iter()
            .filter(|l| !l.contains("struct ") && !l.contains("Recorder for "))
            .count();
        if uses > 0 {
            unrecorded.push(file);
        }
    }
    let allowed: Vec<(String, String)> = RECORDERS
        .iter()
        .map(|(f, of, _)| ((*f).to_string(), (*of).to_string()))
        .collect();
    assert_eq!(made, allowed, "recorders of a save, in place of a run");
    let allowed: Vec<&str> = UNRECORDED.iter().map(|(f, _)| *f).collect();
    assert_eq!(unrecorded, allowed, "saves no run records");
}

/// Every file that may mix or resample audio, and why. Anywhere else it
/// would change what is measured or written behind the codec's back.
const RESHAPES: &[(&str, &str)] = &[
    (
        "crates/muman-media/src/codec.rs",
        "what a codec does so it holds a source, said by `Codec::adapt`",
    ),
    (
        "crates/muman-core/src/fingerprint.rs",
        "the mono copy at Chromaprint's rate that prints are made of",
    ),
    (
        "crates/muman-media/src/align.rs",
        "the mono copy two recordings are aligned on",
    ),
];

#[test]
fn audio_is_mixed_only_to_fit_a_codec_or_to_print_it() {
    let allowed: Vec<&str> = RESHAPES.iter().map(|(f, _)| *f).collect();
    let options = [
        "\"-ac\"",
        "\"-ar\"",
        "channelmap",
        "mapping_family",
        "aformat",
        "aresample",
        "\"pan",
    ];
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if !allowed.contains(&file.as_str()) {
            for line in lines_with(&text, &options) {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn samples_are_changed_only_as_a_plan_s_chain_says() {
    let filters = [
        "volume=",
        "loudnorm",
        "dynaudnorm",
        "acompressor",
        "alimiter",
    ];
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if file != "crates/muman-media/src/codec.rs" {
            for line in lines_with(&text, &filters) {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "render a gain or dynamics filter in `codec::Codec::encoder_args`, from the plan's \
         chain, so a song's samples change only as its plan says:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn ffmpeg_reads_a_file_only_as_its_input_says() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if file != "crates/muman-media/src/ffmpeg.rs" {
            for line in lines_with(&text, &["\"-i\""]) {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "give ffmpeg and ffprobe a file by `ffmpeg::Input::args`, which says what it may be \
         read as, so a manifest or an address named as a song is never fetched:\n{}",
        wrong.join("\n")
    );
}

/// Where ffmpeg is told to encode a picture, and why there.
const PICTURES: [(&str, &str); 2] = [
    (
        "crates/muman-engine/src/render/cover.rs",
        "the covers written into the library, each with `PICTURE_ARGS`",
    ),
    (
        "crates/muman-media/src/quality.rs",
        "a graymap measured, never written",
    ),
];

/// A picture written into the library is the same bytes on every
/// processor, as far as ffmpeg can make it: each encoder
/// `render/cover.rs` names follows `PICTURE_ARGS` in its function, and no
/// other module encodes one.
#[test]
fn a_picture_is_encoded_with_the_options_that_make_it_alike_everywhere() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("//") || !line.contains("\"-c:v\"") {
                continue;
            }
            let allowed = PICTURES.iter().any(|(f, _)| *f == file);
            let start = lines[..n]
                .iter()
                .rposition(|l| l.trim_start().starts_with("fn ") || l.contains(" fn "))
                .unwrap_or(0);
            let exact = lines[start..n].iter().any(|l| l.contains("PICTURE_ARGS"));
            if !allowed || (file == "crates/muman-engine/src/render/cover.rs" && !exact) {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "encode a picture in `render/cover.rs`, after `PICTURE_ARGS`:\n{}",
        wrong.join("\n")
    );
}

/// A folder's cover is decided once, from all of the folder's songs:
/// `folder.rs` alone gives it to the song that holds it.
#[test]
fn a_folder_s_cover_is_given_in_one_place() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if file == "crates/muman-engine/src/folder.rs" {
            continue;
        }
        for line in lines_with(&text, &[".folder = Some("]) {
            wrong.push(format!("{file}: {}", line.trim()));
        }
    }
    assert!(
        wrong.is_empty(),
        "give a folder's cover through `folder::assign`:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn a_plan_changes_format_with_its_renderer_version() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        for line in lines_with(&text, &["plan.format = "]) {
            wrong.push(format!("{file}: {}", line.trim()));
        }
    }
    assert!(
        wrong.is_empty(),
        "set a plan's format by `Plan::with_format`, which sets its version:\n{}",
        wrong.join("\n")
    );
}

/// Where lyrics are cleaned, and why there. A library file's lyrics are
/// cleaned only as its plan names the cleaning, so a change to what the
/// rules write, or to a switch, writes the file again; anywhere else,
/// lyrics are cleaned only to be measured or matched.
const CLEANS: [(&str, &str, &str); 5] = [
    (
        "crates/muman-engine/src/render/lyrics.rs",
        "lyrics::clean(",
        "writes a song's lyrics as its plan's `clean_off` and `cleaning` say",
    ),
    (
        "crates/muman-media/src/lyrics.rs",
        "clean_lrc(",
        "names the cleaning a measure reads by",
    ),
    (
        "crates/muman-media/src/lyrics.rs",
        "lyrics::all(",
        "cleans by every rule for a measure",
    ),
    (
        "crates/muman-media/src/facts.rs",
        "clean_lrc(",
        "measures how a source's lyrics are timed",
    ),
    (
        "src/attach.rs",
        "clean_lrc(",
        "times lyrics given to `add` to match them",
    ),
];

#[test]
fn lyrics_are_cleaned_for_a_library_file_only_as_its_plan_says() {
    let needles = ["lyrics::clean(", "lyrics::all(", "clean_lrc("];
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        for line in lines_with(&text, &needles) {
            let kept = CLEANS
                .iter()
                .any(|(f, needle, _)| *f == file && line.contains(needle));
            if !kept {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "lyrics are cleaned where no plan names how, so a change to the rules \
         leaves the library as it was:\n{}",
        wrong.join("\n")
    );
}

/// Where a path is given another extension, and why there. A file beside
/// a song is named from the song's path in `outputs.rs` alone, so every
/// module that writes, moves, keeps or removes one names it alike.
const RENAMES: [(&str, &str, &str); 2] = [
    (
        "crates/muman-media/src/facts.rs",
        "with_extension(\"cue\")",
        "finds the cue sheet beside a source",
    ),
    (
        "src/attach.rs",
        "with_extension(\"x\")",
        "folds a song's stem as a file named by it folds",
    ),
];

#[test]
fn a_file_beside_a_song_is_named_in_one_place() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if file == "crates/muman-engine/src/outputs.rs" {
            continue;
        }
        for line in lines_with(&text, &["with_extension(\""]) {
            let kept = line.contains("with_extension(\"\")")
                || RENAMES
                    .iter()
                    .any(|(f, needle, _)| *f == file && line.contains(needle));
            if !kept {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "a path is given an extension outside `outputs.rs`, so a file beside a \
         song may be named two ways:\n{}",
        wrong.join("\n")
    );
}

/// The modules that name library files by their place in the library,
/// which a message shows with `/` on every system, through
/// `relpath::show`, as the guide promises.
const LIBRARY_NAMING: &[&str] = &[
    "crates/muman-engine/src/reconcile.rs",
    "crates/muman-engine/src/reconcile/",
    "crates/muman-engine/src/diagnose.rs",
    "crates/muman-engine/src/folder.rs",
    "crates/muman-engine/src/history.rs",
    "crates/muman-engine/src/library.rs",
    "crates/muman-engine/src/outputs.rs",
    "crates/muman-engine/src/render.rs",
    "src/change.rs",
];

/// The paths those modules show as the platform writes them, each from
/// the root: the library folder, the place it moved from, its root as a
/// folder whose time is set, scratch folders, a leftover found by
/// walking the library, the store's sources and the copies `add` made of
/// them; and a path joined to a folder of the home. One joined to the
/// library's is a library file.
const FROM_THE_ROOT: &[&str] = &[
    "library", "before", "dir", "scratch", "part", "source", "unused", "copy",
];

#[test]
fn a_library_file_is_named_in_a_message_with_slashes() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if !LIBRARY_NAMING.iter().any(|m| file.starts_with(m)) {
            continue;
        }
        for line in lines_with(&text, &[".display()"]) {
            let parts: Vec<&str> = line.split(".display()").collect();
            let rooted = parts[..parts.len() - 1].iter().all(|before| {
                let name = before
                    .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .next()
                    .unwrap_or("");
                FROM_THE_ROOT.contains(&name)
                    || (name.is_empty()
                        && before.contains(".join(")
                        && !before.contains("library.join("))
            });
            if !rooted {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "a path shown by `display` where a library file is named, which Windows would show \
         with `\\`; show one through `relpath::show`, or name a path from the root here:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn lyrics_are_written_in_their_formats_only_as_a_plan_says() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if file == "crates/muman-media/src/cues.rs"
            || file == "crates/muman-engine/src/render/lyrics.rs"
        {
            continue;
        }
        for line in lines_with(&text, &["cues::write("]) {
            wrong.push(format!("{file}: {}", line.trim()));
        }
    }
    assert!(
        wrong.is_empty(),
        "lyrics are written in a format outside `render/lyrics.rs`, which writes them as \
         a plan's outputs say, so a library file could hold what no plan names:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn the_reasons_name_files_that_exist() {
    let all = sources();
    for (file, _) in MUTATIONS {
        assert!(all.contains_key(*file), "{file}");
    }
    for (file, ..) in WRITES {
        assert!(all.contains_key(*file), "{file}");
    }
    for (file, ..) in CLEANS.iter().chain(&RENAMES) {
        assert!(all.contains_key(*file), "{file}");
    }
    for (file, _) in STATICS
        .iter()
        .chain(RESHAPES)
        .chain(COPIES)
        .chain(ASKS)
        .chain(PROVIDER_NAMES)
    {
        assert!(all.contains_key(*file), "{file}");
    }
}

/// Where a provider's service or YouTube's search is asked, and why
/// there: anywhere else would ask without `Config::on`, the song's own
/// `lookups` or `skip`, and `enabled = false` is a privacy switch.
const ASKS: &[(&str, &str)] = &[
    (
        "crates/muman-engine/src/lookup.rs",
        "every lookup and every record fetched again, each by its verdict or its provider on",
    ),
    (
        "crates/muman-engine/src/acquire.rs",
        "add's matching, made only while youtube-music is on, and the searches lookup.rs asks for",
    ),
];

#[test]
fn a_provider_is_asked_only_where_its_switch_is_read() {
    let allowed: Vec<&str> = ASKS.iter().map(|(f, _)| *f).collect();
    let asks = [
        "lrclib::Client {",
        "musicbrainz::Client {",
        "acoustid::Client {",
        "coverart::Client {",
        "acquire::lookup(",
        "acquire::reverse_lookup(",
    ];
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        for line in lines_with(&text, &asks) {
            if !allowed.contains(&file.as_str()) {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Where a provider's name is spelled out, and why: anywhere else names
/// it through `Provider::named` or the provider's constant, so a provider
/// renamed or added is renamed or added everywhere.
const PROVIDER_NAMES: &[(&str, &str)] = &[
    ("crates/muman-core/src/provider.rs", "the names themselves"),
    (
        "crates/muman-config/src/migrate.rs",
        "the triggers edition 7 ran unwritten, as files of that edition named them",
    ),
];

#[test]
fn a_provider_s_name_is_spelled_in_one_place() {
    let allowed: Vec<&str> = PROVIDER_NAMES.iter().map(|(f, _)| *f).collect();
    let names = [
        "\"youtube-music\"",
        "\"lrclib\"",
        "\"musicbrainz\"",
        "\"acoustid\"",
        "\"coverart\"",
    ];
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        for line in lines_with(&text, &names) {
            if !allowed.contains(&file.as_str()) {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Where whether a failed fetch is due again is judged: what a sync
/// fetches is decided once, so the fetches, `status` and `info` agree.
const FETCH_DUE: &[(&str, &str)] = &[
    (
        "crates/muman-engine/src/due.rs",
        "what a sync fetches of the listed sources",
    ),
    (
        "crates/muman-engine/src/lookup.rs",
        "a lookup's record fetched again",
    ),
    (
        "crates/muman-engine/src/state.rs",
        "how long a failure waits",
    ),
];

#[test]
fn what_a_sync_fetches_is_decided_once() {
    let allowed: Vec<&str> = FETCH_DUE.iter().map(|(f, _)| *f).collect();
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        for line in lines_with(&text, &[".due(None"]) {
            if !allowed.contains(&file.as_str()) {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn what_muman_says_is_never_read_back() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        let reads = [
            "split_once(",
            "trim_start_matches(",
            "strip_prefix(",
            ".contains(",
        ];
        for line in lines_with(&text, &reads) {
            let said = ["\"Failed: ", "\"Added: ", "\"Updated", "\"Removed: "];
            if said.iter().any(|s| line.contains(s)) {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Where a fetched source's key is worked out, and why: by
/// `Sites::fetched` alone, so a source is named alike however it was
/// reached.
const NAMED: &[(&str, &str)] = &[
    (
        "crates/muman-core/src/sites.rs",
        "the one place a fetched source is named",
    ),
    (
        "crates/muman-media/src/info.rs",
        "the info JSON yt-dlp embedded in a file",
    ),
    ("crates/muman-net/src/music.rs", "an entry a URL listed"),
    (
        "crates/muman-net/src/download.rs",
        "the files a fetch reports",
    ),
    (
        "crates/muman-engine/src/reconcile/keys.rs",
        "an old key renamed by the page it recorded",
    ),
];

#[test]
fn a_fetched_source_is_named_in_one_place() {
    let allowed: Vec<&str> = NAMED.iter().map(|(f, _)| *f).collect();
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        for line in lines_with(&text, &[".fetched("]) {
            if !allowed.contains(&file.as_str()) {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Where a file is copied byte for byte, through any link, and why: a
/// file the user gives `add` is copied by `given.rs` alone, which reads
/// a regular file at its real path, inside the folder given.
const COPIES: &[(&str, &str)] = &[
    (
        "crates/muman-core/src/given.rs",
        "what the user gives `add`, read only from inside the folder given",
    ),
    (
        "crates/muman-engine/src/history.rs",
        "the library's files undo keeps, and puts back",
    ),
    (
        "crates/muman-engine/src/render.rs",
        "a song's files, built elsewhere, into the library",
    ),
    ("src/export.rs", "the library's files into an export"),
    (
        "crates/muman-net/src/aria2.rs",
        "the store's damaged copy, copied in to be checked",
    ),
];

#[test]
fn what_the_user_gives_is_copied_in_one_place() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if COPIES.iter().all(|(f, _)| *f != file) {
            for line in lines_with(&text, &["fs::copy("]) {
                wrong.push(format!("{file}: {}", line.trim()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "copy what `add` is given through `given::Folder`:\n{}",
        wrong.join("\n")
    );
}

/// Every file that writes with `write!`, `writeln!` or `write_all` to
/// anything but a formatter, the most it may, and why. Anywhere else a
/// message goes through `ui`, which makes the text in it printable, so
/// that a title or a name from elsewhere cannot hold what a terminal acts
/// on.
const WRITES: &[(&str, usize, &str)] = &[
    (
        "crates/muman-core/src/ui.rs",
        9,
        "the messages, their text made printable",
    ),
    (
        "crates/muman-core/src/progress.rs",
        6,
        "a step's line built in a String, from names made printable as given, and written \
         to stderr; the messages the console lifts the status line for",
    ),
    (
        "crates/muman-net/src/ytdlp_log.rs",
        2,
        "yt-dlp's line rewritten in place, made printable",
    ),
    (
        "src/sync.rs",
        1,
        "what torrents fetched beside yt-dlp said through `ui`, held until it is done",
    ),
    ("src/cli.rs", 3, "the help's colours, in a String"),
    (
        "crates/muman-media/src/chapters.rs",
        2,
        "an FFMETADATA file, in a String",
    ),
    ("src/attach.rs", 3, "a song offered to pick, in a String"),
    ("src/editor.rs", 1, "the file the editor opens, in a String"),
    (
        "crates/muman-core/src/hash.rs",
        1,
        "a hash in hex, in a String",
    ),
    (
        "crates/muman-config/src/held.rs",
        1,
        "what a digest is made of, in a String",
    ),
    (
        "crates/muman-net/src/lrclib.rs",
        1,
        "lyrics as an `.lrc` file, in a String",
    ),
    (
        "crates/muman-media/src/lyrics.rs",
        1,
        "a timestamp, in a String",
    ),
    (
        "crates/muman-core/src/sites.rs",
        1,
        "yt-dlp's download archive, in a String",
    ),
    (
        "crates/muman-core/src/store.rs",
        1,
        "an id escaped for a file name, in a String",
    ),
    (
        "src/trust.rs",
        1,
        "a setting's hidden character escaped, in a String",
    ),
    (
        "crates/muman-core/src/atomic.rs",
        2,
        "a whole file's bytes, and a private file's",
    ),
    (
        "crates/muman-media/src/ogg.rs",
        1,
        "an Opus header's page, in its file",
    ),
    (
        "crates/muman-media/src/flac.rs",
        1,
        "a STREAMINFO's sample count, in its file",
    ),
    (
        "crates/muman-engine/src/render.rs",
        3,
        "a song's bytes and its lyrics, in their files, the lyrics again when \
         they alone are written",
    ),
    (
        "crates/muman-media/src/cues.rs",
        3,
        "lyrics as cues and word times, into the text render writes to a file",
    ),
];

/// The `write!`, `writeln!` and `write_all` calls of `text` whose first
/// argument is not a formatter's `f`, comments aside.
fn writes(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for name in ["write!(", "writeln!(", "write_all("] {
            for (at, _) in line.match_indices(name) {
                let next = lines.get(n + 1).copied().unwrap_or_default();
                let rest = format!("{}{next}", &line[at + name.len()..]);
                if !rest.trim_start().starts_with("f,") {
                    found.push(line.trim());
                }
            }
        }
    }
    found
}

#[test]
fn text_reaches_a_terminal_only_through_ui() {
    let allowed: BTreeMap<&str, usize> = WRITES.iter().map(|(f, n, _)| (*f, *n)).collect();
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        let found = writes(&text);
        let most = allowed.get(file.as_str()).copied().unwrap_or(0);
        if found.len() > most {
            wrong.push(format!(
                "{file}: {} where {most} may be:\n{}",
                found.len(),
                found.join("\n")
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "say it through `ui`, which makes its text printable:\n{}",
        wrong.join("\n\n")
    );
}
