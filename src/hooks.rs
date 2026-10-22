//! Commands the song list's `[[hook]]`s run as the library changes:
//! one after each song file is written, before muman records its
//! size and time, so a hook may tag it further; one after a run that
//! wrote, moved or removed any file, an undo's files put back counting
//! as written. Each is an argument list, run without a
//! shell; its output is relayed and a failure is warned of, never fatal.
//!
//! The values a hook is given come both as `{name}` placeholders in its
//! words and as `MUMAN_<NAME>` environment variables. A Windows batch
//! file (`.bat`, `.cmd`) gets the variables only: cmd.exe re-reads its
//! arguments, so a title passed as one could run as a command.
//!
//! For the same reason a hook that hands a shell a script, as `sh -c`,
//! PowerShell's `-Command` or `cmd /c` do, is refused where the song list
//! is read when a placeholder is inside that script: filled
//! in, a title as `Dusk $(touch x) Lane` would run. The script reads the
//! variable instead, as `"$MUMAN_PATH"`; a placeholder in a word after a
//! POSIX shell's script is one of its arguments, `$1` on, and is kept.
//! The shell is told by the program's file stem, `env` and its
//! assignments passed over, without case on Windows, where `BASH.EXE` is
//! bash.
//!
//! `{rel}` is the song's path in the library led by `./`, with `/` on
//! every system as [`crate::relpath`] writes it, so a path beginning
//! with `-` is never read as an option.

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, Item};

use crate::runner::{Line, Runner};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A song's audio file was written: `{path}` is it.
    Written,
    /// A run wrote, moved or removed any file: `{written}`, `{moved}`
    /// and `{removed}` are how many.
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    pub on: Event,
    pub run: Vec<String>,
}

/// The `[[hook]]`s a song list sets.
pub fn read(doc: &DocumentMut) -> Result<Vec<Hook>> {
    let Some(item) = doc.get("hook") else {
        return Ok(Vec::new());
    };
    let list = item
        .as_array_of_tables()
        .context("`hook` must be a list of [[hook]] tables")?;
    let mut hooks = Vec::new();
    for (n, t) in list.iter().enumerate() {
        let what = format!("hook {}", n + 1);
        let on = match t.get("on").and_then(Item::as_str) {
            Some("written") => Event::Written,
            Some("changed") => Event::Changed,
            _ => bail!("{what}: `on` must be \"written\" or \"changed\""),
        };
        let run: Vec<String> = t
            .get("run")
            .and_then(Item::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .filter(|r: &Vec<String>| !r.is_empty())
            .with_context(|| format!("{what}: `run` must be a list of words, the command first"))?;
        for (key, _) in t {
            if !["on", "run"].contains(&key) {
                bail!("{what}: `{key}` is no setting");
            }
        }
        let inlined = script(&run).into_iter().find_map(|word| {
            PLACEHOLDERS
                .into_iter()
                .find(|name| word.contains(&format!("{{{name}}}")))
        });
        if let Some(name) = inlined {
            let variable = format!("MUMAN_{}", name.to_ascii_uppercase());
            bail!(
                "{what}: `{{{name}}}` is filled into the script a shell runs, where a name \
                 holding `$(...)` or a quote would run as a command; read the variable instead, \
                 as \"${variable}\" in sh or $env:{variable} in PowerShell"
            );
        }
        hooks.push(Hook { on, run });
    }
    Ok(hooks)
}

/// Run each hook for `event` with its placeholders filled in.
pub fn run<R: Runner, W: Write>(
    runner: &R,
    hooks: &[Hook],
    event: Event,
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
        .collect();
    for hook in hooks.iter().filter(|h| h.on == event) {
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

/// The words of `run` a shell reads as its script: after a POSIX shell's
/// or fish's `-c`, the one word that follows; after PowerShell's
/// `-Command` or cmd's `/c` or `/k`, every word, as each joins the rest
/// into one command line.
fn script(run: &[String]) -> Vec<&str> {
    let mut words = run;
    if words.first().is_some_and(|w| stem_is(w, &["env"])) {
        words = &words[1..];
        while words
            .first()
            .is_some_and(|w| w.starts_with('-') || w.contains('='))
        {
            words = &words[1..];
        }
    }
    let Some((program, args)) = words.split_first() else {
        return Vec::new();
    };
    let rest = |at: Option<usize>| {
        at.map_or_else(Vec::new, |i| {
            args[i + 1..].iter().map(String::as_str).collect()
        })
    };
    if stem_is(program, &["sh", "bash", "zsh", "dash", "ksh", "fish"]) {
        if let Some(inline) = args.iter().find_map(|a| a.strip_prefix("--command=")) {
            return vec![inline];
        }
        let flags = |a: &str| {
            a.strip_prefix('-')
                .is_some_and(|f| f.contains('c') && f.chars().all(|c| c.is_ascii_alphabetic()))
        };
        let at = args.iter().position(|a| a == "--command" || flags(a));
        return rest(at).into_iter().take(1).collect();
    }
    if stem_is(program, &["pwsh", "powershell"]) {
        let command = |a: &String| {
            let a = a.to_ascii_lowercase();
            let name = a.strip_prefix('-').or_else(|| a.strip_prefix('/'));
            name.is_some_and(|n| !n.is_empty() && "command".starts_with(n))
        };
        return rest(args.iter().position(command));
    }
    if stem_is(program, &["cmd"]) {
        let at = args
            .iter()
            .position(|a| a.eq_ignore_ascii_case("/c") || a.eq_ignore_ascii_case("/k"));
        return rest(at);
    }
    Vec::new()
}

/// Whether `program`'s file stem is one of `names`, without case on
/// Windows.
fn stem_is(program: &str, names: &[&str]) -> bool {
    let Some(stem) = Path::new(program).file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    names.iter().any(|n| {
        if cfg!(windows) {
            n.eq_ignore_ascii_case(stem)
        } else {
            *n == stem
        }
    })
}

/// Every placeholder a hook may be given.
const PLACEHOLDERS: [&str; 6] = ["path", "rel", "library", "written", "moved", "removed"];

/// The values a written file's hooks are given: its absolute path, its
/// path in the library with `/` led by `./`, and the library folder.
#[must_use]
pub fn written_values(library: &Path, rel: &Path) -> [(&'static str, String); 3] {
    [
        ("path", library.join(rel).to_string_lossy().into_owned()),
        ("rel", format!("./{}", crate::relpath::show(rel))),
        ("library", library.to_string_lossy().into_owned()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Fake;

    #[test]
    fn hooks_read_and_run_with_their_placeholders() {
        let doc: DocumentMut = "[[hook]]\non = \"written\"\nrun = [\"tagger\", \"{path}\"]\n\
            [[hook]]\non = \"changed\"\nrun = [\"mpc\", \"update\"]\n"
            .parse()
            .unwrap();
        let hooks = read(&doc).unwrap();
        assert_eq!(hooks.len(), 2);
        let fake = Fake::default();
        let mut out = Vec::new();
        let library = Path::new("lib");
        let rel: std::path::PathBuf = ["A", "b.opus"].iter().collect();
        let values = written_values(library, &rel);
        assert_eq!(values[1].1, "./A/b.opus");
        let values: Vec<(&str, &str)> = values.iter().map(|(k, v)| (*k, v.as_str())).collect();
        run(&fake, &hooks, Event::Written, &values, &mut out).unwrap();
        assert_eq!(
            fake.calls(),
            [vec![
                "tagger".to_string(),
                library.join(&rel).to_string_lossy().into_owned()
            ]]
        );
    }

    #[test]
    fn placeholders_are_filled_once() {
        let values = [("path", "a {library} b"), ("library", "lib")];
        assert_eq!(
            fill("{path}|{library}|{other}", &values),
            "a {library} b|lib|{other}"
        );
        assert!(is_batch("cmd") && is_batch("CMD.EXE") && is_batch("tag.bat"));
        assert!(!is_batch("rsgain"));
    }

    #[test]
    fn a_batch_file_gets_no_placeholders() {
        let doc: DocumentMut = "[[hook]]\non = \"written\"\nrun = [\"tag.CMD\", \"{path}\"]\n"
            .parse()
            .unwrap();
        let fake = Fake::default();
        let mut out = Vec::new();
        run(
            &fake,
            &read(&doc).unwrap(),
            Event::Written,
            &[("path", "x")],
            &mut out,
        )
        .unwrap();
        assert_eq!(fake.calls(), Vec::<Vec<String>>::new());
        assert!(crate::ui::plain(&String::from_utf8(out).unwrap()).contains("MUMAN_*"));
    }

    #[test]
    fn a_bad_hook_is_refused() {
        let read_one = |t: &str| read(&t.parse().unwrap());
        assert!(read_one("[[hook]]\non = \"sometimes\"\nrun = [\"x\"]\n").is_err());
        assert!(read_one("[[hook]]\non = \"changed\"\nrun = []\n").is_err());
        assert!(read_one("[[hook]]\non = \"changed\"\nrun = \"x y\"\n").is_err());
    }

    #[test]
    fn a_placeholder_inside_a_shell_s_script_is_refused() {
        let read_run = |run: &str| {
            read(
                &format!("[[hook]]\non = \"written\"\nrun = {run}\n")
                    .parse()
                    .unwrap(),
            )
        };
        for run in [
            r#"["sh", "-c", "echo tagged {path}"]"#,
            r#"["/bin/bash", "-ec", "tag {rel}"]"#,
            r#"["fish", "--command=tag {path}"]"#,
            r#"["/usr/bin/env", "LC_ALL=C", "zsh", "-c", "tag '{path}'"]"#,
            r#"["pwsh", "-Command", "Write-Host", "{path}"]"#,
            r#"["powershell.exe", "-c", "tag {library}"]"#,
            r#"["cmd", "/C", "tag", "{path}"]"#,
        ] {
            let e = read_run(run).unwrap_err();
            assert!(format!("{e}").contains("MUMAN_"), "{run}: {e}");
        }
        for run in [
            r#"["sh", "-c", "tag \"$MUMAN_PATH\""]"#,
            r#"["sh", "-c", "tag \"$1\"", "sh", "{path}"]"#,
            r#"["tagger", "-c", "{path}"]"#,
            r#"["bash", "tag.sh", "{rel}"]"#,
        ] {
            assert!(read_run(run).is_ok(), "{run}");
        }
        assert_eq!(
            read_run(r#"["BASH.EXE", "-c", "{path}"]"#).is_err(),
            cfg!(windows)
        );
    }

    #[test]
    fn rel_is_led_by_dot_slash_so_no_name_reads_as_an_option() {
        let rel: std::path::PathBuf = ["-Lumo Fenn", "01 Rain.opus"].iter().collect();
        let values = written_values(Path::new("lib"), &rel);
        assert_eq!(values[1].1, "./-Lumo Fenn/01 Rain.opus");
    }
}
