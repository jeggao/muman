//! Whether the user trusts what the song list would have muman run or
//! reach. Some of its settings run programs, send requests or hold
//! secrets:
//!
//! | Setting | What it can do |
//! |---|---|
//! | `[[hook]]` | Run its command |
//! | `[tools.*] command` | Run in place of ffmpeg, ffprobe, yt-dlp or aria2c; `info` asks each its version |
//! | `[ytdlp] args`, `extractor_args` | Have yt-dlp run a command (`--exec`, `--netrc-cmd`), load code (`--plugin-dirs`) or send a header |
//! | `[torrent] args` | Have aria2c run a command (`--on-download-complete`) |
//! | `[providers.*] url`, `key` | Send lookups, and AcoustID's key, to another server |
//! | `[sites]` `fetch` | Fetch a source from another address |
//!
//! They live in the song list so that one list renders one library alike
//! on every machine ([`crate::settings`]), so a list someone hands over,
//! or one inside an export's zip, would run its commands as the user. As
//! direnv does with a folder's `.envrc`, muman keeps in `state.json` what
//! the user trusted, and asks again when the list holds more.
//!
//! Each such setting is recorded by its name and a digest of its value as
//! read, so the file's formatting and comments, the order of its keys, a
//! setting [`crate::migrate`] renames and every song leave the record as
//! it was. One at its default, this muman's or one
//! [`crate::migrate::HISTORY`] records an edition had, is none of them: a
//! new home's list holds nothing to trust, nor does one `sync
//! --update-defaults` moved, nor the new home's list `undo` puts back.
//! `[library] path` is none either: it places only the audio and lyrics
//! muman writes, and muman deletes only files it wrote. The servers a
//! song's sources and a torrent's trackers name are what a sync fetches
//! from, songs rather than settings; the docs say so.
//!
//! The list is trusted while each such setting it holds is recorded with
//! the value it holds now. One taken out needs no trust, and the record
//! follows the list down, so a hook taken out by hand and put back by
//! `undo` is asked about again. Otherwise each is listed, the new and the
//! changed marked, and the user asked once on a terminal; without one the
//! command is refused, exit 5, unless `--trust` trusts the list as it is.
//! `-y` answers what a command asks about songs, never this. Names and
//! values are shown escaped, so neither a control character nor a
//! bidirectional mark can hide part of one.
//!
//! Every command that may run a tool, a hook or a request asks first
//! ([`gated`]). `undo` asks about the list it puts back, before putting
//! it back. `list`, `duplicates`, `purge` and `completions` read only
//! what muman recorded and run nothing; nor does `check` without
//! `--decode` or `--upstream`, nor an export that encodes nothing, which
//! copies the library and the song list as they are. None of them asks,
//! so a list not trusted yet can be read, and exported as it is.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use toml_edit::DocumentMut;

use crate::cli::Command;
use crate::hooks::Event;
use crate::migrate::{HISTORY, Step};
use crate::provider::Provider;
use crate::state::State;
use crate::ui::Prompter;

/// One setting that runs a program or reaches a server, as the song list
/// holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// As `[ytdlp] args`.
    pub name: String,
    /// Its value as said to the user, a key masked.
    pub shown: String,
    digest: String,
}

impl Held {
    fn new(name: String, value: String) -> Self {
        Self {
            digest: digest(&name, &value),
            shown: value,
            name,
        }
    }

    fn secret(name: String, value: &str) -> Self {
        Self {
            digest: digest(&name, &canonical(&json!(value))),
            shown: format!("a key of {} characters", value.chars().count()),
            name,
        }
    }
}

/// Whether `command` may run a tool, a hook or a request on the song
/// list's settings, and so asks first. `undo` asks apart, about the list
/// it puts back.
#[must_use]
pub fn gated(command: &Command) -> bool {
    match command {
        Command::Duplicates { .. } => crate::merges(command),
        Command::List { .. }
        | Command::Purge { .. }
        | Command::Completions { .. }
        | Command::Undo { .. } => false,
        Command::Check { decode, upstream } => *decode || *upstream,
        Command::Export { .. } => crate::needs_ffmpeg(command),
        _ => true,
    }
}

/// What the song list in `home` holds that runs a program or reaches a
/// server; a new home's list when it has none.
pub fn held_in(home: &Path) -> Result<Vec<Held>> {
    let file = home.join(crate::dirs::MANIFEST);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => crate::manifest::NEW.to_string(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    held_of(&text).with_context(|| format!("reading {}", file.display()))
}

/// What the song list `text` holds that runs a program or reaches a
/// server.
pub fn held_of(text: &str) -> Result<Vec<Held>> {
    let doc: DocumentMut = text
        .trim_start_matches('\u{feff}')
        .parse()
        .context("not valid TOML")?;
    held(&doc)
}

/// Each setting of `doc` that runs a program or reaches a server and is
/// not at its default.
pub fn held(doc: &DocumentMut) -> Result<Vec<Held>> {
    let settings = crate::settings::read(doc)?;
    let providers = crate::provider::read(doc)?;
    let defaults = crate::settings::Settings::default();
    let mut held = Vec::new();
    for (n, hook) in crate::hooks::read(doc)?.iter().enumerate() {
        let on = match hook.on {
            Event::Written => "written",
            Event::Changed => "changed",
        };
        let value = format!(
            "on = {}, run = {}",
            canonical(&json!(on)),
            canonical(&json!(hook.run))
        );
        held.push(Held::new(format!("[[hook]] {}", n + 1), value));
    }
    let (tools, default) = (&settings.tools, &defaults.tools);
    for (name, program, default) in [
        ("ffmpeg", &tools.ffmpeg, &default.ffmpeg),
        ("ffprobe", &tools.ffprobe, &default.ffprobe),
        ("yt-dlp", &tools.yt_dlp, &default.yt_dlp),
        ("aria2c", &tools.aria2c, &default.aria2c),
    ] {
        let table = if name == "yt-dlp" {
            "tools.\"yt-dlp\"".to_string()
        } else {
            format!("tools.{name}")
        };
        unless_default(
            &mut held,
            (
                &format!("tools.{name}.command"),
                format!("[{table}] command"),
            ),
            &json!(program.command),
            &json!(default.command),
        );
    }
    let ytdlp = &settings.ytdlp;
    unless_default(
        &mut held,
        ("ytdlp.args", "[ytdlp] args".into()),
        &json!(ytdlp.args),
        &json!(defaults.ytdlp.args),
    );
    for (extractor, args) in &ytdlp.extractor_args {
        unless_default(
            &mut held,
            (
                &format!("ytdlp.extractor_args.{extractor}"),
                format!("[ytdlp] extractor_args.{}", canonical(&json!(extractor))),
            ),
            &json!(args),
            &json!(defaults.ytdlp.extractor_args.get(extractor)),
        );
    }
    unless_default(
        &mut held,
        ("torrent.args", "[torrent] args".into()),
        &json!(settings.torrent.args),
        &json!(defaults.torrent.args),
    );
    for p in Provider::ALL {
        let set = providers.settings(p);
        if let Some(url) = set.url.as_ref().filter(|_| set.url != p.defaults().url) {
            held.push(Held::new(
                format!("[providers.{p}] url"),
                canonical(&json!(url)),
            ));
        }
        if let Some(key) = set.key.as_ref().filter(|_| set.key != p.defaults().key) {
            held.push(Held::secret(format!("[providers.{p}] key"), key));
        }
    }
    let built_in: BTreeMap<(&str, &str), &str> = crate::sites::built_in()
        .fetches()
        .map(|(domain, prefix, fetch)| ((domain, prefix), fetch))
        .collect();
    for (domain, prefix, fetch) in settings.sites.fetches() {
        if built_in.get(&(domain, prefix)) != Some(&fetch) {
            held.push(Held::new(
                format!(
                    "[sites.\"{domain}\"] fetch, prefix {}",
                    canonical(&json!(prefix))
                ),
                canonical(&json!(fetch)),
            ));
        }
    }
    Ok(held)
}

/// Push `value` as `name` unless it is `default`, or the default an
/// earlier edition gave `key`, the setting as `migrate` names it.
fn unless_default(
    held: &mut Vec<Held>,
    (key, name): (&str, String),
    value: &Value,
    default: &Value,
) {
    let text = canonical(value);
    let was = HISTORY
        .iter()
        .flat_map(|edition| edition.steps)
        .filter_map(|step| match step {
            Step::Default(change) if change.key == key => toml_value(change.was),
            _ => None,
        });
    if !std::iter::once(default.clone())
        .chain(was)
        .any(|d| canonical(&d) == text)
    {
        held.push(Held::new(name, text));
    }
}

/// `text`, a TOML value, as JSON.
fn toml_value(text: &str) -> Option<Value> {
    let table: toml::Table = toml::from_str(&format!("v = {text}")).ok()?;
    serde_json::to_value(table.get("v")?).ok()
}

/// `value` written one way only, as TOML would write a list of words.
fn canonical(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", items.join(", "))
        }
        other => other.to_string(),
    }
}

fn digest(name: &str, value: &str) -> String {
    let hash = Sha256::digest(format!("{name}\0{value}").as_bytes());
    crate::facts::hex(&hash[..16])
}

/// `text` with each control character, and each mark that reorders or
/// hides text, written as an escape: a terminal shows them as nothing,
/// or acts on them.
fn escaped(text: &str) -> String {
    text.chars().fold(String::new(), |mut out, c| {
        let hidden = matches!(
            c,
            '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'
        );
        if c.is_control() || hidden {
            let _ = write!(out, "\\u{{{:04x}}}", u32::from(c));
        } else {
            out.push(c);
        }
        out
    })
}

/// Go on only once the user trusts what `held`, the song list's, sets:
/// at once when the record in `home` holds each as it is, else as
/// `trust` or the terminal answers. Returns whether to go on; without a
/// terminal or `trust`, refuses.
pub fn gate<W: Write>(
    home: &Path,
    held: &[Held],
    trust: bool,
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    let trusted = State::load(home)?.trusted;
    let now: BTreeMap<String, String> = held
        .iter()
        .map(|h| (h.name.clone(), h.digest.clone()))
        .collect();
    if held.iter().all(|h| trusted.get(&h.name) == Some(&h.digest)) {
        if trusted != now {
            State::keep_trusted(home, &now)?;
        }
        return Ok(true);
    }
    crate::ui::warning(
        out,
        "The song list sets commands muman runs, or servers it reaches, that are not trusted here:",
    )?;
    for h in held {
        let mark = match trusted.get(&h.name) {
            None => " (new)",
            Some(d) if *d != h.digest => " (changed)",
            Some(_) => "",
        };
        crate::ui::line(
            out,
            &format!("  {}{mark}: {}", escaped(&h.name), escaped(&h.shown)),
        )?;
    }
    crate::ui::info(
        out,
        "Read them as you would a script: muman runs them as you.",
    )?;
    if !trust {
        let Some(prompter) = prompter else {
            return Err(crate::change::Refused(
                "Nothing run: without a terminal to ask on, --trust trusts the song list as it is"
                    .into(),
            )
            .into());
        };
        if !prompter.ask("Trust them, and run them?")? {
            crate::ui::info(out, "Nothing run")?;
            return Ok(false);
        }
    }
    State::keep_trusted(home, &now)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::MockPrompter;

    #[test]
    fn a_new_home_s_list_holds_nothing_to_trust() {
        assert_eq!(held_of(crate::manifest::NEW).unwrap(), []);
        assert_eq!(held_of("version = 1\n").unwrap(), []);
    }

    #[test]
    fn each_setting_that_runs_or_reaches_is_held_unless_at_its_default() {
        let text = r#"version = 1
[ytdlp]
args = ["--exec", "echo {}"]
extractor_args = { youtube = "player_client=web", other = "x=1" }
[torrent]
args = ["--on-download-complete=/opt/done"]
[tools.ffprobe]
command = ["/opt/wrap", "ffprobe"]
[tools."yt-dlp"]
command = ["yt-dlp"]
[providers.lrclib]
url = "https://lyrics.example"
[providers.acoustid]
key = "k0000000"
[sites."tunes.example"]
kinds = [{ fetch = "https://tunes.example/t/{id}" }]
[[hook]]
on = "changed"
run = ["mpc", "update"]
"#;
        let held = held_of(text).unwrap();
        let names: Vec<&str> = held.iter().map(|h| h.name.as_str()).collect();
        let want = "[[hook]] 1|[tools.ffprobe] command|[ytdlp] args|\
                    [ytdlp] extractor_args.\"other\"|[ytdlp] extractor_args.\"youtube\"|\
                    [torrent] args|[providers.lrclib] url|[providers.acoustid] key|\
                    [sites.\"tunes.example\"] fetch, prefix \"\"";
        assert_eq!(names.join("|"), want);
        assert_eq!(held[0].shown, r#"on = "changed", run = ["mpc", "update"]"#);
        assert_eq!(held[7].shown, "a key of 8 characters");
    }

    #[test]
    fn formatting_comments_songs_and_old_names_change_nothing() {
        let one = "version = 1\n[ytdlp]\nargs = [\"--exec\", \"x\"]\npartial_days = 3\n";
        let two = "version = 1\n# mine\n[[song]]\nsources = [\"manual:a.flac\"]\n\n\
                   [ytdlp]\nkeep_partial = \"3 days\"\nargs = [ \"--exec\",\n  \"x\" ]\n";
        assert_eq!(held_of(one).unwrap(), held_of(two).unwrap());
        let three = "version = 1\n[ytdlp]\nargs = [\"--exec\", \"y\"]\n";
        assert_ne!(held_of(one).unwrap(), held_of(three).unwrap());
        let mut held = Vec::new();
        let format = |v: &str| ("ytdlp.format", v.to_string());
        unless_default(&mut held, format("old"), &json!("bv*+ba/b"), &json!("ba/b"));
        assert_eq!(held, [], "an earlier edition's default is a default");
        unless_default(&mut held, format("mine"), &json!("ba"), &json!("ba/b"));
        assert_eq!(held.len(), 1);
    }

    #[test]
    fn what_could_hide_part_of_a_value_is_shown_escaped() {
        assert_eq!(escaped("a\u{1b}[2Kb"), "a\\u{001b}[2Kb");
        assert_eq!(escaped("x\u{202e}y\u{9b}"), "x\\u{202e}y\\u{009b}");
        assert_eq!(escaped("Lumo Fenn é"), "Lumo Fenn é");
    }

    #[test]
    fn a_list_is_asked_about_once_refused_without_a_terminal_and_followed_down() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let hooked = "version = 1\n[[hook]]\non = \"changed\"\nrun = [\"mpc\", \"update\"]\n";
        let held = held_of(hooked).unwrap();
        let gate_with = |held: &[Held], answers: Option<&[bool]>, trust: bool| {
            let mut prompter = answers.map(|a| MockPrompter::answering(a.iter().copied()));
            let mut out = Vec::new();
            let p = prompter.as_mut().map(|p| p as &mut dyn Prompter);
            let went = gate(home, held, trust, p, &mut out);
            let asked = prompter.map_or(0, |p| p.asked.len());
            (
                went,
                crate::ui::plain(&String::from_utf8(out).unwrap()),
                asked,
            )
        };
        let (went, said, _) = gate_with(&held, None, false);
        let e = went.unwrap_err();
        assert!(
            e.downcast_ref::<crate::change::Refused>().is_some()
                && format!("{e}").contains("--trust")
        );
        assert!(
            said.contains("[[hook]] 1 (new): on = \"changed\""),
            "{said}"
        );
        assert!(!gate_with(&held, Some(&[false]), false).0.unwrap());
        assert!(State::load(home).unwrap().trusted.is_empty());
        assert!(gate_with(&held, Some(&[true]), false).0.unwrap());
        assert_eq!(gate_with(&held, Some(&[]), false).2, 0, "trusted once");

        let changed = held_of(&hooked.replace("update", "rescan")).unwrap();
        let (went, said, _) = gate_with(&changed, None, true);
        assert!(
            went.unwrap() && said.contains("[[hook]] 1 (changed)"),
            "{said}"
        );
        assert!(gate_with(&[], None, false).0.unwrap());
        assert!(State::load(home).unwrap().trusted.is_empty());
        assert!(
            gate_with(&changed, None, false).0.is_err(),
            "taken out, then asked again"
        );
    }
}
