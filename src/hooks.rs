//! Commands the song list's `[[hook]]`s run as the library changes:
//! one after each song file is written, before muman records its
//! size and time, so a hook may tag it further; one after a run that
//! wrote or removed anything. Each is an argument list, run without a
//! shell; its output is relayed and a failure is warned of, never fatal.

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
    /// A run wrote or removed anything: `{written}` and `{removed}` are
    /// how many files.
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
    for hook in hooks.iter().filter(|h| h.on == event) {
        let argv: Vec<OsString> = hook
            .run
            .iter()
            .map(|word| {
                values
                    .iter()
                    .fold(word.clone(), |w, (name, v)| {
                        w.replace(&format!("{{{name}}}"), v)
                    })
                    .into()
            })
            .collect();
        let mut lines = Vec::new();
        let ran = runner.stream(&argv, &mut |line| {
            let (Line::Out(l) | Line::Err(l)) = line;
            lines.push(l.to_string());
        });
        for line in lines {
            crate::ui::trace(out, &format!("hook: {line}"))?;
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

/// The placeholders a written file's hooks fill in.
#[must_use]
pub fn written_values(library: &Path, rel: &Path) -> [(&'static str, String); 3] {
    [
        ("path", library.join(rel).to_string_lossy().into_owned()),
        ("rel", rel.to_string_lossy().into_owned()),
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
        let values = written_values(Path::new("/lib"), Path::new("A/b.opus"));
        let values: Vec<(&str, &str)> = values.iter().map(|(k, v)| (*k, v.as_str())).collect();
        run(&fake, &hooks, Event::Written, &values, &mut out).unwrap();
        assert_eq!(
            fake.calls(),
            [vec!["tagger".to_string(), "/lib/A/b.opus".to_string()]]
        );
    }

    #[test]
    fn a_bad_hook_is_refused() {
        let read_one = |t: &str| read(&t.parse().unwrap());
        assert!(read_one("[[hook]]\non = \"sometimes\"\nrun = [\"x\"]\n").is_err());
        assert!(read_one("[[hook]]\non = \"changed\"\nrun = []\n").is_err());
        assert!(read_one("[[hook]]\non = \"changed\"\nrun = \"x y\"\n").is_err());
    }
}
