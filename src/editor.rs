//! `muman edit`: the songs a query picks opened in an editor as
//! their song-list entries, and what changed applied. A file that does
//! not read reopens with what is wrong atop it; songs changed in the
//! song list while it was open are said so, and saving again applies.

use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

use crate::ui::Prompter;
use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::change::{self, Read};
use crate::dirs::Dirs;
use crate::manifest::{Edit, Rewritten, SongTable};
use crate::query::View;
use crate::source::SourceKey;
use crate::store::Store;

const HEADER: &str = "\
# Edit the songs below, then save and close; quit without saving to
# change nothing. Each [[song]] is one song, named by its id: leave the
# id as it is. Delete a [[song]] to remove it. Move a key from one
# song's sources to another's to join them, or into a new [[song]]
# without an id to make it a song of its own. A tag left empty keeps
# what the sources offer; above each song is what it resolves to now.
";

/// Lines muman puts atop a file it reopens.
const PROBLEM: &str = "# ERROR: ";
const NOTICE: &str = "# NOTE: ";

/// One song as opened: its id, its name, and its table as listed.
struct Opened {
    id: SourceKey,
    name: String,
    table: Table,
}

fn describe(view: &View) -> String {
    let mut parts = vec![view.name()];
    let album = view.first("album");
    if !album.is_empty() {
        parts.push(album);
    }
    let date = view.first("date");
    if !date.is_empty() {
        parts.push(date);
    }
    let mut has = view.format.clone().map_or_else(Vec::new, |f| vec![f]);
    if view.cover {
        has.push("cover".into());
    }
    if view.lyrics {
        has.push("lyrics".into());
    }
    if !has.is_empty() {
        parts.push(has.join(", "));
    }
    parts.join(" · ")
}

/// The file the editor opens.
fn render(opened: &[Opened], views: &[&View]) -> String {
    let mut doc = DocumentMut::new();
    let mut list = toml_edit::ArrayOfTables::new();
    for (o, view) in opened.iter().zip(views) {
        let mut table = Table::new();
        table.insert("id", value(o.id.to_string()));
        // Each key as it is, the comments above it held in its decor.
        for (key, item) in o
            .table
            .iter()
            .filter_map(|(k, _)| o.table.get_key_value(k))
            .filter(|(k, _)| k.get() != "id")
        {
            table.insert_formatted(key, item.clone());
        }
        let mut prefix = format!("\n# {}\n", describe(view));
        if let Some(path) = &view.path {
            let _ = writeln!(prefix, "# → {}", path.display());
        }
        table.decor_mut().set_prefix(prefix);
        list.push(table);
    }
    doc.insert("song", Item::ArrayOfTables(list));
    format!("{HEADER}{doc}")
}

/// `text` without the lines muman put atop it.
fn without_notes(text: &str) -> String {
    text.lines()
        .filter(|l| !l.starts_with(PROBLEM) && !l.starts_with(NOTICE))
        .fold(String::new(), |mut acc, l| {
            acc.push_str(l);
            acc.push('\n');
            acc
        })
}

fn with_notes(text: &str, prefix: &str, notes: &[String]) -> String {
    let mut out: String = notes
        .iter()
        .flat_map(|n| n.lines().map(|l| format!("{prefix}{l}\n")))
        .collect();
    out.push_str(&without_notes(text));
    out
}

fn content(t: &Table) -> String {
    let mut t = t.clone();
    t.decor_mut().clear();
    t.remove("id");
    t.to_string()
}

/// The edit `text` asks for, or what is wrong with it. `None` when it
/// changes nothing.
fn interpret(
    text: &str,
    opened: &[Opened],
    read: &Read,
    store: &Store,
) -> std::result::Result<Option<Edit>, Vec<String>> {
    let doc: DocumentMut = text.parse().map_err(|e| vec![format!("{e}")])?;
    let tables: Vec<Table> = match doc.get("song") {
        None => Vec::new(),
        Some(item) => item
            .as_array_of_tables()
            .ok_or_else(|| vec!["`song` must be a list of [[song]] tables".to_string()])?
            .iter()
            .cloned()
            .collect(),
    };
    // An empty file is an editor that wrote nothing, not every song
    // deleted; a file left with its header and no song is.
    if tables.is_empty() && text.trim().is_empty() {
        return Ok(None);
    }
    let mut problems = Vec::new();
    let mut seen = Vec::new();
    let mut songs = Vec::new();
    let mut new = Vec::new();
    let known: Vec<&SourceKey> = opened
        .iter()
        .filter_map(|o| read.manifest.song_with(&o.id))
        .flat_map(|s| &s.sources)
        .collect();
    for (n, mut table) in tables.into_iter().enumerate() {
        let what = format!("song {}", n + 1);
        let id = table.remove("id");
        if table
            .get("sources")
            .and_then(Item::as_array)
            .is_none_or(toml_edit::Array::is_empty)
        {
            problems.push(format!(
                "{what} lists no source; delete the whole [[song]] to remove it"
            ));
        }
        for key in table
            .get("sources")
            .and_then(Item::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
        {
            match SourceKey::parse(key) {
                Ok(k) if !known.contains(&&k) && !store.has(&k) => problems.push(format!(
                    "{what}: {k} is a source of no song opened and is not in the store"
                )),
                Ok(_) => {}
                Err(e) => problems.push(format!("{what}: {e:#}")),
            }
        }
        match id.as_ref().map(|i| i.as_str()) {
            None => new.push(SongTable(table)),
            Some(Some(id)) => match opened.iter().find(|o| o.id.to_string() == id) {
                Some(o) if seen.contains(&o.id) => {
                    problems.push(format!("{what}: {id} is the id of an earlier song too"));
                }
                Some(o) => {
                    seen.push(o.id.clone());
                    if content(&table) != content(&o.table) {
                        songs.push((o.id.clone(), Rewritten::Table(SongTable(table))));
                    }
                }
                None => problems.push(format!(
                    "{what}: no song opened has the id {id}; leave ids as they were"
                )),
            },
            Some(None) => problems.push(format!("{what}: its id must be the key it was")),
        }
    }
    for o in opened.iter().filter(|o| !seen.contains(&o.id)) {
        songs.push((o.id.clone(), Rewritten::Removed(o.name.clone())));
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    if songs.is_empty() && new.is_empty() {
        return Ok(None);
    }
    let edit = Edit::Rewrite { songs, new };
    read.manifest
        .trial(std::slice::from_ref(&edit))
        .map_err(|e| vec![format!("{e:#}")])?;
    Ok(Some(edit))
}

/// Say what an edit does, song by song.
fn summarize<W: Write>(edit: &Edit, opened: &[Opened], out: &mut W) -> Result<()> {
    let Edit::Rewrite { songs, new } = edit else {
        return Ok(());
    };
    for (id, to) in songs {
        let name = opened
            .iter()
            .find(|o| &o.id == id)
            .map_or_else(|| id.to_string(), |o| o.name.clone());
        match to {
            Rewritten::Table(_) => crate::ui::info(out, &format!("Changed: {name}"))?,
            Rewritten::Removed(_) => crate::ui::warning(out, &format!("Removed: {name}"))?,
        }
    }
    for _ in new {
        crate::ui::info(out, "Split off: a new song")?;
    }
    Ok(())
}

/// Open the songs the query picks in an editor, run by `editor` on the
/// file's path, and apply what changed. Returns whether anything did.
pub fn edit<W: Write>(
    dirs: &Dirs,
    terms: &[String],
    all: bool,
    mut prompter: Option<&mut dyn Prompter>,
    editor: &mut dyn FnMut(&Path) -> Result<()>,
    out: &mut W,
) -> Result<bool> {
    if prompter.is_none() {
        return Err(change::Refused("Editing needs a terminal".into()).into());
    }
    let mut read = Read::new(dirs, terms)?;
    let picked = change::pick(
        &read.views,
        &read.query,
        all || read.query.is_empty(),
        "edit",
        change::reborrow(&mut prompter),
    )?;
    if picked.is_empty() {
        crate::ui::info(out, "Nothing to edit")?;
        return Ok(false);
    }
    let store = Store::scan(dirs)?;
    let mut opened = Vec::new();
    for n in &picked {
        let id = read.views[*n]
            .id()
            .context("a song lists no source")?
            .clone();
        let Some(table) = read
            .manifest
            .tables_of(std::slice::from_ref(&id))?
            .into_iter()
            .next()
        else {
            bail!("{id} is no longer listed");
        };
        opened.push(Opened {
            id,
            name: read.views[*n].name(),
            table,
        });
    }
    let views: Vec<&View> = picked.iter().map(|n| &read.views[*n]).collect();
    let first = render(&opened, &views);
    let dir = tempfile::tempdir().context("creating a temporary directory")?;
    let file = dir.path().join("songs.toml");
    let mut shown = first.clone();
    let mut expected = read.songs(&picked);
    loop {
        std::fs::write(&file, &shown).with_context(|| format!("writing {}", file.display()))?;
        editor(&file)?;
        // An editor on Windows may save with CRLF line endings, which
        // would otherwise count as a change to every line.
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?
            .replace("\r\n", "\n");
        if without_notes(&text) == without_notes(&first) {
            crate::ui::info(out, "Nothing changed")?;
            return Ok(false);
        }
        let edit = match interpret(&text, &opened, &read, &store) {
            Ok(Some(edit)) => edit,
            Ok(None) => {
                crate::ui::info(out, "Nothing changed")?;
                return Ok(false);
            }
            Err(problems) => {
                shown = with_notes(&text, PROBLEM, &problems);
                continue;
            }
        };
        read.manifest.edit(edit.clone());
        let changed = read.manifest.save_if_unchanged(&expected)?;
        if changed.is_empty() {
            summarize(&edit, &opened, out)?;
            return Ok(true);
        }
        read.manifest = crate::manifest::Manifest::load(&dirs.home)?;
        expected = changed
            .iter()
            .filter_map(|k| read.manifest.song_with(k).cloned())
            .chain(
                expected
                    .iter()
                    .filter(|e| !e.sources.first().is_some_and(|k| changed.contains(k)))
                    .cloned(),
            )
            .collect();
        let names: Vec<String> = changed.iter().map(ToString::to_string).collect();
        shown = with_notes(
            &text,
            NOTICE,
            &[format!(
                "The song list changed while this was open, for {}.\nSave again to apply this over it; quit without saving to keep it.",
                names.join(", ")
            )],
        );
    }
}

/// Run `$VISUAL`, else `$EDITOR`, else the system's own, on `path`. On
/// Unix the variable is a shell command, as other programs read it; on
/// Windows it is split into words the same way and its program found
/// with `PATHEXT`, so `code --wait` finds `code.cmd`, and the path is
/// passed as an argument of its own rather than through cmd.exe's
/// quoting.
pub fn launch(path: &Path) -> Result<()> {
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|e| !e.trim().is_empty()))
        .unwrap_or_else(|| if cfg!(windows) { "notepad" } else { "vi" }.to_string());
    let status = if cfg!(windows) {
        let words = shell_words::split(&editor)
            .ok()
            .filter(|w| !w.is_empty())
            .with_context(|| format!("reading the editor `{editor}`"))?;
        let program = which::which(&words[0]).unwrap_or_else(|_| words[0].clone().into());
        std::process::Command::new(program)
            .args(&words[1..])
            .arg(path)
            .status()
    } else {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{editor} \"$1\""))
            .arg("sh")
            .arg(path)
            .status()
    }
    .with_context(|| format!("starting {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}; nothing changed");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
