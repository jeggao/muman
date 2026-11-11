//! The library's modules in layers, lowest first, and any reference
//! that reaches up from one layer to a higher one: `cargo xtask layers`.
//!
//! Each layer is a crate: `crates/<crate>`, then muman itself, the
//! command line, at the top. A crate's layer is how many of the
//! workspace's crates it depends on, as its Cargo.toml lists them, and a
//! module's is its crate's. The manifest is read as TOML, so a
//! dependency counts in any form Cargo takes (a version, a table, a
//! dotted `name.workspace = true`, a `package` it is renamed from),
//! under `[dependencies]` or a `[target.….dependencies]`; a dev- or
//! build-dependency is no part of the crate's code. A module may name
//! any module of its own layer or a lower one; one that names a higher
//! one, in its code, its tests or a doc link, fails the check. Cargo
//! already refuses a crate that uses one above it; this names the module
//! and the place, and holds the tests and doc links too. `--report`
//! prints each module's fan-in and the cycles among a crate's modules.
//!
//! References are read from the code, not compiled: `crate::name` paths
//! and the names in `crate::{…}` groups, with comments set aside by
//! [`crate::comments`]' lexer and the tests of a file (its
//! `#[cfg(test)] mod tests` and any `tests.rs`) counted apart. A name
//! that is no module is an item of a crate's `lib.rs`. A path a macro
//! builds or a glob brings in is missed; the compiler finds those.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use regex::Regex;

use crate::comments::parse::{self, LineKind};

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
    let ranks = crate_ranks(root)?;
    let mut references = Vec::new();
    let mut layers = BTreeMap::from([(
        "lib".to_string(),
        ranks.values().max().copied().unwrap_or(0),
    )]);
    for (module, file, krate) in sources(root)? {
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
        layers.insert(module.clone(), ranks.get(&krate).copied().unwrap_or(0));
        references.extend(references_in(&module, &shown, &text, whole_test));
    }
    let references: Vec<Reference> = references
        .into_iter()
        .map(|mut r| {
            if !layers.contains_key(&r.to) {
                r.to = "lib".into();
            }
            r
        })
        .filter(|r| r.from != r.to)
        .collect();
    // The library's own tests, in no layer, may name any module.
    layers.retain(|m, _| !TEST_MODULES.contains(&m.as_str()));
    let problems = check(&layers, &references);
    if report {
        print!("{}", describe(&layers, &references));
    }
    for p in &problems {
        eprintln!("layers: {p}");
    }
    Ok(problems.len())
}

/// Each crate of the workspace by its layer: how many of the others it
/// depends on, as its Cargo.toml lists them, transitively. muman is the
/// root's; a crate under `crates/` is named by its folder.
pub fn crate_ranks(root: &Path) -> Result<BTreeMap<String, usize>> {
    let top = manifest(&root.join("Cargo.toml"))?;
    let empty = toml::Table::new();
    let shared = top
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(toml::Value::as_table)
        .unwrap_or(&empty);
    let mut needs = BTreeMap::from([("muman".to_string(), dependencies(&top, shared))]);
    for src in source_roots(root)?.into_iter().skip(1) {
        let dir = src.parent().unwrap_or(&src);
        let name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let on = dependencies(&manifest(&dir.join("Cargo.toml"))?, shared);
        needs.insert(name, on);
    }
    let crates: BTreeSet<String> = needs.keys().cloned().collect();
    for (name, on) in &mut needs {
        on.retain(|d| d != name && crates.contains(d));
    }
    let mut ranks = BTreeMap::new();
    for name in needs.keys() {
        let mut seen = BTreeSet::new();
        let mut todo: Vec<&String> = needs[name].iter().collect();
        while let Some(d) = todo.pop() {
            if seen.insert(d.clone()) {
                todo.extend(needs.get(d).into_iter().flatten());
            }
        }
        ranks.insert(name.clone(), seen.len());
    }
    Ok(ranks)
}

fn manifest(path: &Path) -> Result<toml::Table> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.parse()
        .with_context(|| format!("parsing {}", path.display()))
}

/// The crates `manifest` builds its code on, by the names they are
/// published as; an entry with `workspace = true` is read from `shared`,
/// the workspace's `[workspace.dependencies]`.
fn dependencies(manifest: &toml::Table, shared: &toml::Table) -> BTreeSet<String> {
    let targets = manifest
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(toml::Table::values);
    let tables = std::iter::once(manifest.get("dependencies"))
        .chain(targets.map(|t| t.get("dependencies")))
        .flatten()
        .filter_map(toml::Value::as_table);
    let mut on = BTreeSet::new();
    for table in tables {
        for (key, entry) in table {
            let inherited = entry.get("workspace").and_then(toml::Value::as_bool) == Some(true);
            let entry = if inherited {
                shared.get(key).unwrap_or(entry)
            } else {
                entry
            };
            let name = entry.get("package").and_then(toml::Value::as_str);
            on.insert(name.unwrap_or(key).to_string());
        }
    }
    on
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
fn sources(root: &Path) -> Result<Vec<(String, PathBuf, String)>> {
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
        let krate = if src == main {
            "muman".to_string()
        } else {
            let dir = src.parent().unwrap_or(&src);
            dir.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
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
            Some((module, f, krate.clone()))
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

/// The references that reach from a lower layer up to a higher one, by
/// what makes them: an edge code makes is not also a test's or a doc's.
fn upward(
    layers: &BTreeMap<String, usize>,
    references: &[Reference],
) -> BTreeMap<By, BTreeSet<(String, String)>> {
    let mut by: BTreeMap<By, BTreeSet<(String, String)>> = BTreeMap::new();
    for r in references {
        if let (Some(from), Some(to)) = (layers.get(&r.from), layers.get(&r.to))
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
pub fn check(layers: &BTreeMap<String, usize>, references: &[Reference]) -> Vec<String> {
    let mut problems = Vec::new();
    for (by, made) in upward(layers, references) {
        for (from, to) in made {
            let at = references
                .iter()
                .find(|r| r.by == by && r.from == from && r.to == to)
                .map_or("", |r| r.at.as_str());
            problems.push(format!(
                "{at}: {from} reaches up to {to}; name a module of its own layer or below"
            ));
        }
    }
    problems
}

/// Each upward reference with its places, each module's fan-in, and the
/// modules that reach each other round, largest first.
fn describe(layers: &BTreeMap<String, usize>, references: &[Reference]) -> String {
    let mut out = String::new();
    let found = upward(layers, references);
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
    fn a_reference_up_a_layer_is_reported_and_one_down_is_not() {
        let r = |from: &str, to: &str, by| Reference {
            from: from.into(),
            to: to.into(),
            by,
            at: "src/x.rs:1".into(),
        };
        let layers = BTreeMap::from([("units".to_string(), 0), ("state".to_string(), 4)]);
        let problems = check(
            &layers,
            &[r("units", "state", By::Test), r("state", "units", By::Code)],
        );
        assert_eq!(
            problems,
            ["src/x.rs:1: units reaches up to state; name a module of its own layer or below"]
        );
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
        let modules: Vec<(String, String)> = sources(root)
            .unwrap()
            .into_iter()
            .map(|(m, _, c)| (m, c))
            .collect();
        let at = |m: &str, c: &str| (m.to_string(), c.to_string());
        assert_eq!(
            modules,
            [
                at("lib", "muman"),
                at("units", "muman"),
                at("hash", "muman-core"),
                at("store", "muman-core")
            ]
        );
    }

    #[test]
    fn a_crate_s_layer_counts_the_crates_it_stands_on() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let crate_at = |dir: &Path, deps: &str| {
            std::fs::create_dir_all(dir.join("src")).unwrap();
            std::fs::write(
                dir.join("Cargo.toml"),
                format!("[package]\nname = \"x\"\n\n[dependencies]\n{deps}\n[lints]\n"),
            )
            .unwrap();
        };
        crate_at(
            root,
            "muman-media = { path = \"crates/muman-media\" }\nanyhow = \"1\"\n",
        );
        crate_at(&root.join("crates/muman-core"), "anyhow = \"1\"\n");
        crate_at(
            &root.join("crates/muman-media"),
            "muman-core = { path = \"../muman-core\" }\n",
        );
        let ranks = crate_ranks(root).unwrap();
        let want: BTreeMap<String, usize> = [("muman", 2), ("muman-core", 0), ("muman-media", 1)]
            .map(|(c, n)| (c.to_string(), n))
            .into();
        assert_eq!(ranks, want);
    }

    /// The dependencies `manifest` builds on, in a workspace that shares
    /// `muman-core` and, renamed, `core`.
    fn on(manifest: &str) -> Vec<String> {
        let shared: toml::Table = "muman-core = { path = \"crates/muman-core\" }\n\
                                   core = { package = \"muman-core\", path = \"crates/muman-core\" }"
            .parse()
            .unwrap();
        let manifest: toml::Table = manifest.parse().unwrap();
        dependencies(&manifest, &shared).into_iter().collect()
    }

    #[test]
    fn a_dependency_counts_as_a_version() {
        assert_eq!(on("[dependencies]\nmuman-core = \"0.1\""), ["muman-core"]);
    }

    #[test]
    fn a_dependency_counts_as_a_dotted_workspace_key() {
        assert_eq!(
            on("[dependencies]\nmuman-core.workspace = true"),
            ["muman-core"]
        );
    }

    #[test]
    fn a_dependency_counts_as_a_workspace_table() {
        assert_eq!(
            on("[dependencies]\nmuman-core = { workspace = true, features = [\"testing\"] }"),
            ["muman-core"]
        );
    }

    #[test]
    fn a_dependency_counts_as_a_path_table() {
        assert_eq!(
            on("[dependencies]\nmuman-core = { path = \"../muman-core\" }"),
            ["muman-core"]
        );
    }

    #[test]
    fn a_dependency_counts_in_its_own_table() {
        assert_eq!(
            on("[dependencies.muman-core]\npath = \"../muman-core\""),
            ["muman-core"]
        );
    }

    #[test]
    fn a_dependency_counts_by_the_package_it_renames() {
        assert_eq!(
            on("[dependencies]\ncore = { package = \"muman-core\", path = \"../muman-core\" }"),
            ["muman-core"]
        );
        assert_eq!(on("[dependencies]\ncore.workspace = true"), ["muman-core"]);
    }

    #[test]
    fn a_dependency_counts_for_one_target() {
        assert_eq!(
            on("[target.'cfg(unix)'.dependencies]\nmuman-core.workspace = true"),
            ["muman-core"]
        );
    }

    #[test]
    fn a_dev_or_build_dependency_does_not_count() {
        let manifest = "[dev-dependencies]\nmuman-core.workspace = true\n\
                        [build-dependencies]\nmuman-core.workspace = true\n\
                        [target.'cfg(unix)'.dev-dependencies]\nmuman-core.workspace = true";
        assert_eq!(on(manifest), [] as [String; 0]);
    }

    /// The workspace's own manifests, in the style they are written in,
    /// rank every crate: a style no form above reads would put them all
    /// at 0, where no reference can reach up.
    #[test]
    fn the_workspace_s_crates_stand_in_their_layers() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits in the repository");
        let want: BTreeMap<String, usize> = [
            ("muman-core", 0),
            ("muman-media", 1),
            ("muman-net", 2),
            ("muman-config", 3),
            ("muman-engine", 4),
            ("muman", 5),
        ]
        .map(|(c, n)| (c.to_string(), n))
        .into();
        assert_eq!(crate_ranks(root).unwrap(), want);
    }

    #[test]
    fn three_modules_round_in_a_cycle_are_one_component() {
        let edges: BTreeSet<(&str, &str)> = [("a", "b"), ("b", "c"), ("c", "a"), ("c", "d")].into();
        assert_eq!(cycles(&edges), [["a", "b", "c"]]);
    }
}
