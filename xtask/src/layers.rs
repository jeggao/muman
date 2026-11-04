//! The library's modules in layers, lowest first, and the references
//! that reach up from one layer to a higher one: `cargo xtask layers`.
//!
//! A module may name any module of its own layer or a lower one. Each
//! reference upward that the code makes today is listed, with the layer
//! split that cuts it, in [`KNOWN`]; one only tests make in
//! [`TEST_KNOWN`]; one a doc link makes in [`DOC_KNOWN`]. The check
//! fails on a module in no layer, on an upward reference no list names,
//! and on a listed one the code no longer makes, so the lists only
//! shrink, by the pull request that cuts each, until the layers can be
//! crates. `--report` prints what the lists are made from: each upward
//! reference with its place, each module's fan-in, and the cycles.
//!
//! References are read from the code, not compiled: `crate::name` paths
//! and the names in `crate::{…}` groups, with comments set aside by
//! [`crate::comments`]' lexer and the tests of a file (its
//! `#[cfg(test)] mod tests` and any `tests.rs`) counted apart. A name
//! that is no module is an item of `lib.rs`. A path a macro builds or a
//! glob brings in is missed; the compiler finds those once the layers
//! are crates.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use regex::Regex;

use crate::comments::parse::{self, LineKind};

/// Each layer and its modules, lowest first; `lib` is the crate root.
pub const LAYERS: [(&str, &[&str]); 6] = [
    (
        "core",
        &[
            "units",
            "hash",
            "relpath",
            "lang",
            "platform",
            "portable",
            "parallel",
            "template",
            "ui",
            "progress",
            "atomic",
            "kill",
            "testing",
            "tools",
            "runner",
            "fingerprint",
            "http",
            "source",
            "provider",
            "sites",
            "defaults",
            "errors",
            "dirs",
            "store",
            "torrent",
            "given",
        ],
    ),
    (
        "media",
        &[
            "ffmpeg",
            "probe",
            "chapters",
            "codec",
            "picture",
            "quality",
            "align",
            "analysis",
            "loudness",
            "integrity",
            "ogg",
            "cue",
            "cues",
            "lyrics",
            "clean",
            "tags",
            "tagfile",
            "info",
            "similar",
            "naming",
            "record",
            "facts",
        ],
    ),
    (
        "net",
        &[
            "lrclib",
            "musicbrainz",
            "acoustid",
            "coverart",
            "music",
            "download",
            "ytdlp_log",
            "plugins",
            "aria2",
        ],
    ),
    (
        "config",
        &[
            "settings",
            "manifest",
            "migrate",
            "held",
            "hooks",
            "providers",
            "terms",
        ],
    ),
    (
        "engine",
        &[
            "state",
            "resolve",
            "levels",
            "artifact",
            "reconcile",
            "render",
            "limit",
            "fit",
            "folder",
            "outputs",
            "lookup",
            "due",
            "identify",
            "acquire",
            "library",
            "history",
            "query",
            "rows",
            "album",
        ],
    ),
    (
        "cli",
        &[
            "lib",
            "cli",
            "trust",
            "add",
            "sync",
            "change",
            "attach",
            "duplicates",
            "export",
            "check",
            "purge",
            "overview",
            "guided",
            "editor",
            "redact",
        ],
    ),
];

/// The upward references the library's code makes, as `(from, to)`.
pub const KNOWN: &[(&str, &str)] = &[];

/// The upward references only tests make.
pub const TEST_KNOWN: &[(&str, &str)] = &[];

/// The upward references only doc links make.
pub const DOC_KNOWN: &[(&str, &str)] = &[];

/// Files that are tests as a whole, not a module's code.
const TEST_FILES: [&str; 1] = ["tests.rs"];

/// The library's own test module, `src/tests.rs`.
const TEST_MODULES: [&str; 1] = ["tests"];

/// What a reference is made by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum By {
    Code,
    Test,
    Doc,
}

/// One reference from a module to another, where it is made.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reference {
    pub from: String,
    pub to: String,
    pub by: By,
    pub at: String,
}

pub fn run(root: &Path, report: bool) -> Result<usize> {
    let mut references = Vec::new();
    let mut modules = BTreeSet::from(["lib".to_string()]);
    for (module, file) in sources(root)? {
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        let shown = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string();
        let whole_test = file
            .file_name()
            .is_some_and(|n| TEST_FILES.iter().any(|t| n == *t))
            || file.components().any(|c| c.as_os_str() == "tests");
        modules.insert(module.clone());
        references.extend(references_in(&module, &shown, &text, whole_test));
    }
    let references: Vec<Reference> = references
        .into_iter()
        .map(|mut r| {
            if !modules.contains(&r.to) {
                r.to = "lib".into();
            }
            r
        })
        .filter(|r| r.from != r.to)
        .collect();
    // The library's own tests, in no layer, may name any module.
    modules.retain(|m| !TEST_MODULES.contains(&m.as_str()));
    let problems = check(&modules, &references);
    if report {
        print!("{}", describe(&references));
    }
    for p in &problems {
        eprintln!("layers: {p}");
    }
    Ok(problems.len())
}

/// The folders the library's code is in: `src`, then each crate's
/// `crates/<crate>/src`, as the library is split into crates.
pub fn source_roots(root: &Path) -> Result<Vec<PathBuf>> {
    let mut roots = vec![root.join("src")];
    let crates = root.join("crates");
    if crates.is_dir() {
        let mut found = Vec::new();
        for entry in
            std::fs::read_dir(&crates).with_context(|| format!("reading {}", crates.display()))?
        {
            let src = entry?.path().join("src");
            if src.is_dir() {
                found.push(src);
            }
        }
        found.sort();
        roots.extend(found);
    }
    Ok(roots)
}

/// Every Rust file of the library, with the module it belongs to:
/// `<module>.rs`, or a file under `<module>/`, in any of its
/// [`source_roots`]. The binary's `main.rs` and a crate's `lib.rs` are
/// the roots the modules hang from, no module of their own.
fn sources(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            if path.is_dir() {
                walk(&path, found)?;
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
        Ok(())
    }
    let main = root.join("src");
    let mut out = Vec::new();
    for src in source_roots(root)? {
        let mut files = Vec::new();
        walk(&src, &mut files)?;
        files.sort();
        out.extend(files.into_iter().filter_map(|f| {
            if f == src.join("main.rs") || (src != main && f == src.join("lib.rs")) {
                return None;
            }
            let first = f.strip_prefix(&src).ok()?.components().next()?;
            let name = first.as_os_str().to_string_lossy();
            let module = name.strip_suffix(".rs").unwrap_or(&name).to_string();
            Some((module, f))
        }));
    }
    Ok(out)
}

/// The references `module`'s file `text` makes, its tests' apart.
#[must_use]
pub fn references_in(module: &str, shown: &str, text: &str, whole_test: bool) -> Vec<Reference> {
    let path = Regex::new(r"crate::([A-Za-z_][A-Za-z0-9_]*)").expect("a valid pattern");
    let group = Regex::new(r"crate::\{").expect("a valid pattern");
    let link = Regex::new(r"\[`?crate::([A-Za-z_][A-Za-z0-9_]*)").expect("a valid pattern");
    let parsed = parse::parse(text);
    let tests_from = text
        .lines()
        .zip(text.lines().skip(1))
        .position(|(a, b)| a.trim() == "#[cfg(test)]" && b.trim_start().starts_with("mod tests"))
        .unwrap_or(usize::MAX);
    // The code alone, comments blanked, so a group may span lines.
    let mut code = String::new();
    let mut found = Vec::new();
    for (n, line) in parsed.lines.iter().enumerate() {
        let by = if whole_test || n >= tests_from {
            By::Test
        } else {
            By::Code
        };
        let mut kept = line.raw.clone();
        if let Some(c) = &line.comment {
            if c.doc {
                for m in link.captures_iter(&c.text) {
                    found.push((m[1].to_string(), By::Doc, n));
                }
            }
            kept.replace_range(c.byte_range.clone(), &" ".repeat(c.byte_range.len()));
        }
        if matches!(line.kind, LineKind::Doc | LineKind::Comment) {
            kept.clear();
        }
        for m in path.captures_iter(&kept) {
            found.push((m[1].to_string(), by, n));
        }
        code.push_str(&kept);
        code.push('\n');
    }
    for m in group.find_iter(&code) {
        let n = code[..m.start()].matches('\n').count();
        let by = if whole_test || n >= tests_from {
            By::Test
        } else {
            By::Code
        };
        for name in group_heads(&code[m.end()..]) {
            found.push((name, by, n));
        }
    }
    found
        .into_iter()
        .map(|(to, by, n)| Reference {
            from: module.to_string(),
            to,
            by: if by == By::Doc && whole_test {
                By::Test
            } else {
                by
            },
            at: format!("{shown}:{}", n + 1),
        })
        .collect()
}

/// The first name of each item at the top of a `{…}` group, `rest`
/// beginning just after its `{`.
fn group_heads(rest: &str) -> Vec<String> {
    let mut heads = Vec::new();
    let mut depth = 0usize;
    let mut item = String::new();
    for c in rest.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth == 0 => break,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                heads.extend(head(&item));
                item.clear();
                continue;
            }
            _ => {}
        }
        if depth == 0 {
            item.push(c);
        }
    }
    heads.extend(head(&item));
    heads
}

fn head(item: &str) -> Option<String> {
    let name: String = item
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty() && name != "self").then_some(name)
}

fn rank(module: &str) -> Option<usize> {
    LAYERS.iter().position(|(_, ms)| ms.contains(&module))
}

/// The references that reach from a lower layer up to a higher one, by
/// what makes them: an edge code makes is not also a test's or a doc's.
fn upward(references: &[Reference]) -> BTreeMap<By, BTreeSet<(String, String)>> {
    let mut by: BTreeMap<By, BTreeSet<(String, String)>> = BTreeMap::new();
    for r in references {
        if let (Some(from), Some(to)) = (rank(&r.from), rank(&r.to))
            && to > from
        {
            by.entry(r.by)
                .or_default()
                .insert((r.from.clone(), r.to.clone()));
        }
    }
    let code = by.get(&By::Code).cloned().unwrap_or_default();
    for kind in [By::Test, By::Doc] {
        if let Some(set) = by.get_mut(&kind) {
            set.retain(|e| !code.contains(e));
        }
    }
    by
}

#[must_use]
pub fn check(modules: &BTreeSet<String>, references: &[Reference]) -> Vec<String> {
    let mut problems: Vec<String> = modules
        .iter()
        .filter(|m| rank(m).is_none())
        .map(|m| format!("{m} is in no layer; add it to LAYERS in xtask/src/layers.rs"))
        .collect();
    let found = upward(references);
    for (by, list, name) in [
        (By::Code, KNOWN, "KNOWN"),
        (By::Test, TEST_KNOWN, "TEST_KNOWN"),
        (By::Doc, DOC_KNOWN, "DOC_KNOWN"),
    ] {
        let listed: BTreeSet<(String, String)> = list
            .iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect();
        let made = found.get(&by).cloned().unwrap_or_default();
        for (from, to) in made.difference(&listed) {
            let at = references
                .iter()
                .find(|r| r.by == by && r.from == *from && r.to == *to)
                .map_or("", |r| r.at.as_str());
            problems.push(format!(
                "{at}: {from} reaches up to {to}; name a module of its own layer or below"
            ));
        }
        for (from, to) in listed.difference(&made) {
            problems.push(format!(
                "{name} lists {from} -> {to}, which nothing makes now; remove it"
            ));
        }
    }
    problems
}

/// Each upward reference with its places, each module's fan-in, and the
/// modules that reach each other round, largest first.
fn describe(references: &[Reference]) -> String {
    let mut out = String::new();
    let found = upward(references);
    for (by, edges) in &found {
        let _ = writeln!(out, "upward, by {by:?}: {} edge(s)", edges.len());
        for (from, to) in edges {
            let places: Vec<&str> = references
                .iter()
                .filter(|r| r.by == *by && r.from == *from && r.to == *to)
                .map(|r| r.at.as_str())
                .collect();
            let _ = writeln!(out, "    (\"{from}\", \"{to}\"), // {}", places.join(", "));
        }
    }
    let code: BTreeSet<(&str, &str)> = references
        .iter()
        .filter(|r| r.by == By::Code)
        .map(|r| (r.from.as_str(), r.to.as_str()))
        .collect();
    let mut fan_in: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, to) in &code {
        *fan_in.entry(to).or_default() += 1;
    }
    let mut by_fan: Vec<(&str, usize)> = fan_in.into_iter().collect();
    by_fan.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let _ = writeln!(out, "fan-in:");
    for (m, n) in by_fan {
        let _ = writeln!(out, "    {m} {n}");
    }
    for cycle in cycles(&code) {
        let _ = writeln!(out, "cycle of {}: {}", cycle.len(), cycle.join(" "));
    }
    out
}

/// The strongly connected components of more than one module, largest
/// first (Tarjan's algorithm).
#[must_use]
pub fn cycles(edges: &BTreeSet<(&str, &str)>) -> Vec<Vec<String>> {
    struct Walk<'a> {
        next: BTreeMap<&'a str, Vec<&'a str>>,
        index: BTreeMap<&'a str, usize>,
        low: BTreeMap<&'a str, usize>,
        stack: Vec<&'a str>,
        on: BTreeSet<&'a str>,
        found: Vec<Vec<String>>,
    }
    impl<'a> Walk<'a> {
        fn visit(&mut self, v: &'a str) {
            let i = self.index.len();
            self.index.insert(v, i);
            self.low.insert(v, i);
            self.stack.push(v);
            self.on.insert(v);
            for w in self.next.get(v).cloned().unwrap_or_default() {
                if !self.index.contains_key(w) {
                    self.visit(w);
                    let low = self.low[v].min(self.low[w]);
                    self.low.insert(v, low);
                } else if self.on.contains(w) {
                    let low = self.low[v].min(self.index[w]);
                    self.low.insert(v, low);
                }
            }
            if self.low[v] == self.index[v] {
                let mut component = Vec::new();
                while let Some(w) = self.stack.pop() {
                    self.on.remove(w);
                    component.push(w.to_string());
                    if w == v {
                        break;
                    }
                }
                if component.len() > 1 {
                    component.sort();
                    self.found.push(component);
                }
            }
        }
    }
    let mut walk = Walk {
        next: BTreeMap::new(),
        index: BTreeMap::new(),
        low: BTreeMap::new(),
        stack: Vec::new(),
        on: BTreeSet::new(),
        found: Vec::new(),
    };
    for (a, b) in edges {
        walk.next.entry(a).or_default().push(b);
    }
    let nodes: BTreeSet<&str> = edges.iter().flat_map(|(a, b)| [*a, *b]).collect();
    for v in nodes {
        if !walk.index.contains_key(v) {
            walk.visit(v);
        }
    }
    walk.found.sort_by_key(|c| std::cmp::Reverse(c.len()));
    walk.found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(found: &[Reference]) -> Vec<(String, String, By)> {
        found
            .iter()
            .map(|r| (r.from.clone(), r.to.clone(), r.by))
            .collect()
    }

    #[test]
    fn paths_and_groups_count_and_comments_do_not() {
        let text = "use crate::{atomic, settings::{Library, Restrict}, self};\n\
                    // crate::store in a comment\n\
                    /// See [`crate::render`].\n\
                    fn f() { crate::probe::parse(); crate::Job::new(); }\n\
                    #[cfg(test)]\n\
                    mod tests {\n    use crate::reconcile;\n}\n";
        let found = references_in("tags", "src/tags.rs", text, false);
        let mut got = edges(&found);
        got.sort();
        let want = |to: &str, by| ("tags".to_string(), to.to_string(), by);
        assert_eq!(
            got,
            [
                want("Job", By::Code),
                want("atomic", By::Code),
                want("probe", By::Code),
                want("reconcile", By::Test),
                want("render", By::Doc),
                want("settings", By::Code),
            ]
        );
    }

    #[test]
    fn a_reference_up_a_layer_is_reported_unless_listed_and_a_stale_entry_is_too() {
        let r = |from: &str, to: &str, by| Reference {
            from: from.into(),
            to: to.into(),
            by,
            at: "src/x.rs:1".into(),
        };
        let modules: BTreeSet<String> = ["units", "state", "nowhere"].map(String::from).into();
        let problems = check(
            &modules,
            &[r("units", "state", By::Code), r("state", "units", By::Code)],
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("nowhere is in no layer"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("units reaches up to state"))
        );
        assert!(!problems.iter().any(|p| p.contains("state reaches up")));
    }

    #[test]
    fn a_crate_s_modules_count_beside_the_library_s() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for file in [
            "src/lib.rs",
            "src/main.rs",
            "src/units.rs",
            "crates/muman-core/src/lib.rs",
            "crates/muman-core/src/hash.rs",
            "crates/muman-core/src/store/tests.rs",
            "crates/notes/README.md",
        ] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        let roots = source_roots(root).unwrap();
        assert_eq!(
            roots,
            [root.join("src"), root.join("crates/muman-core/src")]
        );
        let modules: Vec<String> = sources(root).unwrap().into_iter().map(|(m, _)| m).collect();
        assert_eq!(modules, ["lib", "units", "hash", "store"]);
    }

    #[test]
    fn three_modules_round_in_a_cycle_are_one_component() {
        let edges: BTreeSet<(&str, &str)> = [("a", "b"), ("b", "c"), ("c", "a"), ("c", "d")].into();
        assert_eq!(cycles(&edges), [["a", "b", "c"]]);
    }
}
