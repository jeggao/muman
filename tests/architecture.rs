//! The architecture the code keeps, checked over its source: where files
//! are changed, which state lives for the whole process, where sites are
//! named, which loop fits songs into a size, and where audio is mixed. Each rule names the one
//! place a thing is done; code that does it anywhere else fails here
//! until the rule, and the reason beside it, is changed on purpose.
//!
//! Only code that ships is read: a file's inline `mod tests` and the
//! `tests.rs` and `testing.rs` files are left out.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Every source file under `src`, by its path from `src`, without its
/// tests.
fn sources() -> BTreeMap<String, String> {
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
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    walk(&src, &mut found);
    found
        .into_iter()
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            name != "tests.rs" && name != "testing.rs"
        })
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            let shipped = match text.find("#[cfg(test)]\nmod tests {") {
                Some(at) => text[..at].to_string(),
                None => text,
            };
            let name = p
                .strip_prefix(&src)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            (name, shipped)
        })
        .collect()
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

/// Every file that may change files, the most calls it may make, and why
/// it may. A file the list does not name changes none; one that makes
/// more calls than its count has grown a way to change files that this
/// list, and its reason, should say.
const MUTATIONS: &[(&str, usize, &str)] = &[
    (
        "atomic.rs",
        5,
        "the primitives every other write goes through, scratch folders, and the library's lock file as it is let go",
    ),
    (
        "library.rs",
        7,
        "the ledger: every move and removal in the library, recorded first, \
         and a song's staged files put in place over only what it proved muman's",
    ),
    (
        "render.rs",
        6,
        "a song's new files staged beside their paths, whatever was at a staged \
         name removed first, and those staged files removed when the song fails",
    ),
    (
        "history.rs",
        15,
        "run records, the files kept for undo, undo putting them back or removing lyrics it added and the copies `add` made, and an undone run's record let go of once its sync ends",
    ),
    (
        "store.rs",
        4,
        "an upload its release replaced, deleted from the sources; a file \
         fetched moved to its key's place, replacing the key's other; a \
         torrent's file aria2c finished moved to its key's place",
    ),
    (
        "aria2.rs",
        5,
        "the metainfo a magnet link's peers sent, dropped once read; a \
         torrent's unfinished folder, once its files are placed; a \
         download taken up, or a damaged copy, copied in to be checked; \
         the input file of a run, in scratch",
    ),
    ("purge.rs", 1, "sources no song uses, deleted on request"),
    (
        "change.rs",
        1,
        "a removed song's fetched source, deleted with `remove --purge`",
    ),
    ("add.rs", 1, "files copied in by `add`"),
    ("sync.rs", 2, "downloads left unfinished too long"),
    (
        "acquire.rs",
        2,
        "the URL list and download archive handed to yt-dlp, in scratch",
    ),
    (
        "download.rs",
        3,
        "a file just fetched or copied in, written again without the cookies \
         and headers of its info JSON, that JSON kept in scratch; a remux left \
         cut short in partial downloads",
    ),
    (
        "editor.rs",
        1,
        "the file the editor is opened on, in scratch",
    ),
    (
        "export.rs",
        6,
        "the zip or the folder, made in scratch and renamed into place, an \
         empty folder there replaced; each song encoded for it moved or \
         deleted from scratch once it is in, any other copied in",
    ),
];

#[test]
fn files_change_only_where_the_change_is_recorded_or_scratch() {
    let calls = [
        "atomic::remove(",
        "atomic::rename(",
        "fs::remove_file(",
        "fs::rename(",
        "fs::copy(",
        "hard_link(",
        "fs::remove_dir(",
        "remove_dir_all(",
        "fs::write(",
    ];
    let allowed: BTreeMap<&str, usize> = MUTATIONS.iter().map(|(f, n, _)| (*f, *n)).collect();
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        let found = lines_with(&text, &calls);
        let most = allowed.get(file.as_str()).copied().unwrap_or(0);
        if found.len() > most {
            wrong.push(format!(
                "{file}: {} where {most} may be:\n{}",
                found.len(),
                found.join("\n")
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
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
        &sources["render.rs"],
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
        taken.len() == 1 && taken[0].starts_with("reconcile.rs: "),
        "{}",
        taken.join("\n")
    );
}

/// The state that lives for the whole process, and why it may.
const STATICS: &[(&str, &str)] = &[
    ("progress.rs", "the one progress line a terminal shows"),
    (
        "history.rs",
        "a count that tells two runs of one process apart",
    ),
    (
        "runner.rs",
        "the programs a signal stops with muman, and whether one of the user's own answers it",
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
        if file == "source.rs" {
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
        "name sites in manifest/new.toml's [sites], not in code:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn songs_fit_a_size_by_one_loop() {
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        if file != "fit.rs" && !lines_with(&text, &["allocate("]).is_empty() {
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
        if file != "manifest.rs"
            && file != "history.rs"
            && !lines_with(&text, &["Name::new(MANIFEST)"]).is_empty()
        {
            wrong.push(file);
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// Every file that may mix or resample audio, and why. Anywhere else it
/// would change what is measured or written behind the codec's back.
const RESHAPES: &[(&str, &str)] = &[
    (
        "codec.rs",
        "what a codec does so it holds a source, said by `Codec::adapt`",
    ),
    (
        "fingerprint.rs",
        "the mono copy at Chromaprint's rate that prints are made of",
    ),
    ("align.rs", "the mono copy two recordings are aligned on"),
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
        if file != "codec.rs" {
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

#[test]
fn the_reasons_name_files_that_exist() {
    let all = sources();
    for (file, ..) in MUTATIONS {
        assert!(all.contains_key(*file), "{file}");
    }
    for (file, _) in STATICS.iter().chain(RESHAPES) {
        assert!(all.contains_key(*file), "{file}");
    }
}

/// Where a file or folder of a run's own is made, and why: anywhere
/// else, one a run stopped partway leaves would be nobody's to remove.
const SCRATCH: &[(&str, &str)] = &[
    (
        "atomic.rs",
        "a whole file written beside its place, and a run's scratch folder",
    ),
    (
        "reconcile.rs",
        "a step's folder inside a run's scratch folder",
    ),
];

#[test]
fn a_runs_own_files_are_made_in_its_scratch_folder() {
    let allowed: Vec<&str> = SCRATCH.iter().map(|(f, _)| *f).collect();
    let mut wrong = Vec::new();
    for (file, text) in sources() {
        for line in lines_with(&text, &["tempfile::Builder", "tempfile_in(", "tempdir_in("]) {
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
    ("sync.rs", "what a sync fetches of the listed sources"),
    ("lookup.rs", "a lookup's record fetched again"),
    ("state.rs", "how long a failure waits"),
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
    ("sites.rs", "the one place a fetched source is named"),
    ("info.rs", "the info JSON yt-dlp embedded in a file"),
    ("music.rs", "an entry a URL listed"),
    ("download.rs", "the files a fetch reports"),
    ("reconcile.rs", "an old key renamed by the page it recorded"),
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
