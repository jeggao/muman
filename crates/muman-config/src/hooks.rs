//! The commands the song list's `[[hook]]`s run once a command has
//! changed the library: once, after a run that wrote, moved or removed
//! any file, an undo's files put back counting as written, and after
//! the run has let go of the home and the library, so a hook may run
//! muman there. Each is an argument list, run without a shell; its
//! output is relayed and a failure is warned of, never fatal.
//!
//! A hook is given the library folder and how many files the run wrote,
//! moved and removed, as `{library}`, `{written}`, `{moved}` and
//! `{removed}` in its words and as `MUMAN_<NAME>` environment variables.
//! None of them holds text from a song: a title or a file's name, taken
//! from an upload, filled into a word a shell or another program reads
//! as code would run as a command, and no program could be told apart
//! from its name as one that does. A Windows batch file (`.bat`, `.cmd`)
//! gets the variables only, as cmd.exe reads its arguments again and
//! would split a library path holding `&`.
//!
//! A hook is read on its own: one muman cannot run is kept, refused with
//! its reason, said by each command that would have run it and by
//! `check`, and the song list and every other hook still work. A hook
//! `on = "written"`, which ran on each song file as it was written, is
//! one: it changed a file muman had placed, outside the song's plan.
//!
//! Every hook is given [`HOOK_HOME`], and a muman it starts runs no
//! hook, so a hook that syncs cannot run itself forever.

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use toml_edit::{DocumentMut, Item};

use crate::runner::{Line, Runner};

/// A `[[hook]]` the song list sets, run once a command has changed the
/// library, unless refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    pub run: Vec<String>,
    /// Why it never runs, when muman cannot run it.
    pub refused: Option<String>,
}

/// The `[[hook]]`s a song list sets, each read on its own.
#[must_use]
pub fn read(doc: &DocumentMut) -> Vec<Hook> {
    let Some(item) = doc.get("hook") else {
        return Vec::new();
    };
    let Some(list) = item.as_array_of_tables() else {
        return vec![Hook {
            run: Vec::new(),
            refused: Some("`hook` must be a list of [[hook]] tables".into()),
        }];
    };
    list.iter().map(one).collect()
}

fn one(t: &toml_edit::Table) -> Hook {
    let run: Vec<String> = t
        .get("run")
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let refused = match t.get("on").and_then(Item::as_str) {
        Some("changed") => None,
        Some("written") => Some(
            "muman runs no hook on each file written; a tag it should write is given with \
             `muman add` as a tag file, and ReplayGain is [loudness]'s"
                .to_string(),
        ),
        _ => Some("`on` must be \"changed\"".to_string()),
    }
    .or_else(|| {
        run.is_empty()
            .then(|| "`run` must be a list of words, the command first".to_string())
    })
    .or_else(|| {
        t.iter()
            .find(|(key, _)| !["on", "run"].contains(key))
            .map(|(key, _)| format!("`{key}` is no setting"))
    });
    Hook { run, refused }
}

/// The variable a hook is given its run's home in, so a muman it starts
/// runs no hook.
pub const HOOK_HOME: &str = "MUMAN_HOOK_HOME";

/// The hooks a command's runs made due, and what those runs changed,
/// run once the command has let go of the home.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[must_use]
pub struct Due {
    hooks: Vec<Hook>,
    library: Option<std::path::PathBuf>,
    written: usize,
    moved: usize,
    removed: usize,
}

impl Due {
    /// What one run changed in `library`, its song list setting `hooks`;
    /// a later run's hooks are the ones its song list left.
    pub fn add(
        &mut self,
        hooks: &[Hook],
        library: &Path,
        (written, moved, removed): (usize, usize, usize),
    ) {
        self.hooks = hooks.to_vec();
        self.library = Some(library.to_path_buf());
        self.written += written;
        self.moved += moved;
        self.removed += removed;
    }

    /// This and `later`, a run after it: the hooks its song list left,
    /// and what both changed.
    pub fn then(&mut self, later: Self) {
        if later.library.is_some() {
            self.hooks = later.hooks;
            self.library = later.library;
        }
        self.written += later.written;
        self.moved += later.moved;
        self.removed += later.removed;
    }

    /// Whether any run changed a file.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.written + self.moved + self.removed > 0
    }

    /// Run each hook, saying each refused one, when any file changed.
    pub fn run<R: Runner, W: Write>(&self, runner: &R, home: &Path, out: &mut W) -> Result<()> {
        let Some(library) = self.library.as_deref().filter(|_| self.changed()) else {
            return Ok(());
        };
        let library = library.to_string_lossy();
        let counts = [self.written, self.moved, self.removed].map(|n| n.to_string());
        let values = [
            ("library", library.as_ref()),
            ("written", counts[0].as_str()),
            ("moved", counts[1].as_str()),
            ("removed", counts[2].as_str()),
        ];
        run(runner, home, &self.hooks, &values, out)
    }
}

/// Run each hook of `hooks` with `values` filled in.
fn run<R: Runner, W: Write>(
    runner: &R,
    home: &Path,
    hooks: &[Hook],
    values: &[(&str, &str)],
    out: &mut W,
) -> Result<()> {
    let env: Vec<(String, String)> = values
        .iter()
        .map(|(name, v)| {
            (
                format!("MUMAN_{}", name.to_ascii_uppercase()),
                (*v).to_string(),
            )
        })
        .chain([(HOOK_HOME.to_string(), home.to_string_lossy().into_owned())])
        .collect();
    for (n, hook) in hooks.iter().enumerate() {
        if let Some(why) = &hook.refused {
            crate::ui::warning(out, &format!("Hook {} not run: {why}", n + 1))?;
            continue;
        }
        let placeholders = hook
            .run
            .iter()
            .any(|w| values.iter().any(|(n, _)| w.contains(&format!("{{{n}}}"))));
        if placeholders && is_batch(&hook.run[0]) {
            crate::ui::warning(
                out,
                &format!(
                    "Hook `{}` not run: a batch file takes the MUMAN_* variables, not {{placeholders}}",
                    hook.run.join(" ")
                ),
            )?;
            continue;
        }
        let argv: Vec<OsString> = hook
            .run
            .iter()
            .map(|word| fill(word, values).into())
            .collect();
        let mut lines = Vec::new();
        let ran = runner.stream_env(&argv, &env, &mut |line| {
            let (Line::Out(l) | Line::Err(l)) = line;
            lines.push(l.to_string());
        });
        for line in lines {
            crate::ui::relayed(out, &format!("hook: {line}"))?;
        }
        match ran {
            Ok(true) => {}
            Ok(false) => {
                crate::ui::warning(out, &format!("Hook `{}` failed", hook.run.join(" ")))?;
            }
            Err(e) => crate::ui::warning(out, &format!("Hook `{}`: {e:#}", hook.run.join(" ")))?,
        }
    }
    Ok(())
}

/// `word` with each `{name}` replaced by its value, in one pass, so a
/// value that itself holds `{library}` stays as it is.
fn fill(word: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut rest = word;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after.find('}').and_then(|close| {
            let name = &after[..close];
            values
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| (*v, close))
        });
        if let Some((v, close)) = value {
            out.push_str(v);
            rest = &after[close + 1..];
        } else {
            out.push('{');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Whether `program` is, or is found as, a Windows batch file, or is
/// cmd.exe itself, which reads its arguments the same way.
fn is_batch(program: &str) -> bool {
    let batch = |p: &Path| {
        p.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("bat") || e.eq_ignore_ascii_case("cmd"))
            || p.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("cmd"))
    };
    batch(Path::new(program)) || (cfg!(windows) && which::which(program).is_ok_and(|p| batch(&p)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Fake;

    fn hooks(text: &str) -> Vec<Hook> {
        read(&text.parse().unwrap())
    }

    fn changed() -> Due {
        let mut due = Due::default();
        let hooks =
            hooks("[[hook]]\non = \"changed\"\nrun = [\"mpc\", \"update\", \"{written}\"]\n");
        due.add(&hooks, Path::new("lib"), (2, 0, 0));
        due
    }

    #[test]
    fn a_hook_runs_with_its_placeholders_filled_once_a_file_changed() {
        let fake = Fake::default();
        changed()
            .run(&fake, Path::new("home"), &mut Vec::new())
            .unwrap();
        assert_eq!(fake.calls(), [["mpc", "update", "2"]]);
        let fake = Fake::default();
        let mut quiet = Due::default();
        quiet.add(&changed().hooks, Path::new("lib"), (0, 0, 0));
        quiet
            .run(&fake, Path::new("home"), &mut Vec::new())
            .unwrap();
        assert!(fake.calls().is_empty(), "nothing changed, no hook");
    }

    /// Records the variables each command it runs is given.
    #[derive(Default)]
    struct Recording(std::sync::Mutex<Vec<Vec<(String, String)>>>);

    impl Runner for Recording {
        fn run(&self, _: &[OsString]) -> Result<()> {
            Ok(())
        }

        fn output(&self, _: &[OsString]) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }

        fn stream(&self, _: &[OsString], _: &mut dyn FnMut(Line<'_>)) -> Result<bool> {
            Ok(true)
        }

        fn stream_env(
            &self,
            _: &[OsString],
            env: &[(String, String)],
            _: &mut dyn FnMut(Line<'_>),
        ) -> Result<bool> {
            self.0.lock().unwrap().push(env.to_vec());
            Ok(true)
        }
    }

    #[test]
    fn a_hook_is_given_its_run_s_home_library_and_counts() {
        let recording = Recording::default();
        changed()
            .run(&recording, Path::new("home"), &mut Vec::new())
            .unwrap();
        let given = recording.0.into_inner().unwrap();
        let var = |name: &str| {
            given[0]
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(var(HOOK_HOME).as_deref(), Some("home"));
        assert_eq!(var("MUMAN_LIBRARY").as_deref(), Some("lib"));
        assert_eq!(var("MUMAN_WRITTEN").as_deref(), Some("2"));
    }

    #[test]
    fn placeholders_are_filled_once() {
        let values = [("written", "a {library} b"), ("library", "lib")];
        assert_eq!(
            fill("{written}|{library}|{path}", &values),
            "a {library} b|lib|{path}"
        );
        assert!(is_batch("cmd") && is_batch("CMD.EXE") && is_batch("tag.bat"));
        assert!(!is_batch("mpc"));
    }

    #[test]
    fn a_batch_file_gets_no_placeholders() {
        let mut due = Due::default();
        let hooks = hooks("[[hook]]\non = \"changed\"\nrun = [\"tag.CMD\", \"{library}\"]\n");
        due.add(&hooks, Path::new("lib"), (1, 0, 0));
        let fake = Fake::default();
        let mut out = Vec::new();
        due.run(&fake, Path::new("home"), &mut out).unwrap();
        assert_eq!(fake.calls(), Vec::<Vec<String>>::new());
        assert!(crate::ui::plain(&String::from_utf8(out).unwrap()).contains("MUMAN_*"));
    }

    #[test]
    fn a_hook_muman_cannot_run_is_refused_alone() {
        let refused = |t: &str| hooks(t)[0].refused.clone();
        for bad in [
            "[[hook]]\non = \"sometimes\"\nrun = [\"x\"]\n",
            "[[hook]]\non = \"changed\"\nrun = []\n",
            "[[hook]]\non = \"changed\"\nrun = \"x y\"\n",
            "[[hook]]\non = \"changed\"\nrun = [\"x\"]\nshell = true\n",
        ] {
            assert!(refused(bad).is_some(), "{bad}");
        }
        let written = refused("[[hook]]\non = \"written\"\nrun = [\"tagger\", \"{path}\"]\n");
        assert!(written.unwrap().contains("tag file"));
        assert_eq!(hooks("hook = 3\n").len(), 1);
        let mixed = hooks(
            "[[hook]]\non = \"written\"\nrun = [\"tagger\"]\n\
             [[hook]]\non = \"changed\"\nrun = [\"mpc\", \"update\"]\n",
        );
        assert!(mixed[0].refused.is_some() && mixed[1].refused.is_none());
        let mut due = Due::default();
        due.add(&mixed, Path::new("lib"), (1, 0, 0));
        let fake = Fake::default();
        let mut out = Vec::new();
        due.run(&fake, Path::new("home"), &mut out).unwrap();
        assert_eq!(fake.calls(), [["mpc", "update"]]);
        assert!(String::from_utf8(out).unwrap().contains("Hook 1 not run"));
    }
}
