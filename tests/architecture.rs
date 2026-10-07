//! The architecture the code keeps, checked over its source: where files
//! are changed, which state lives for the whole process, where sites are
//! named, and which loop fits songs into a size. Each rule names the one
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
        4,
        "the primitives every other write goes through, and scratch folders",
    ),
    (
        "library.rs",
        5,
        "the ledger: every move and removal in the library, recorded first",
    ),
    (
        "render.rs",
        8,
        "a song's new files, written beside and renamed into a path the ledger kept",
    ),
    (
        "history.rs",
        11,
        "run records, the files kept for undo, and undo putting them back",
    ),
    (
        "store.rs",
        1,
        "an upload its release replaced, deleted from the sources",
    ),
    ("purge.rs", 1, "sources no song uses, deleted on request"),
    (
        "change.rs",
        1,
        "a removed song's fetched source, deleted with `remove --purge`",
    ),
    (
        "lib.rs",
        3,
        "downloads left unfinished too long, and files copied in by `add`",
    ),
    (
        "acquire.rs",
        2,
        "the URL list and download archive handed to yt-dlp, in scratch",
    ),
    (
        "editor.rs",
        1,
        "the file the editor is opened on, in scratch",
    ),
    (
        "export.rs",
        2,
        "the zip, written in scratch and renamed into place",
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

/// The state that lives for the whole process, and why it may.
const STATICS: &[(&str, &str)] = &[
    ("progress.rs", "the one progress line a terminal shows"),
    (
        "history.rs",
        "a count that tells two runs of one process apart",
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

#[test]
fn the_reasons_name_files_that_exist() {
    let all = sources();
    for (file, ..) in MUTATIONS {
        assert!(all.contains_key(*file), "{file}");
    }
    for (file, _) in STATICS {
        assert!(all.contains_key(*file), "{file}");
    }
}
