//! `muman edit`: the songs a query picks opened in an editor as
//! their song-list entries, and what changed applied. A file that does
//! not read reopens with what is wrong atop it; songs changed in the
//! song list while it was open are said so, and saving again applies.
//!
//! A file saved over songs changed meanwhile reopens on the songs as
//! they are now, comments and values alike, so that saving it again
//! cannot quietly take back what changed. The user's edits are merged in
//! field by field, three ways: a value the user changed from what was
//! first shown is theirs, any other is the song list's now, so an edit
//! wins over the change only where both touched one field. The note atop
//! names each field the change set, song by song.
//!
//! Above each song are comments on what it resolves to now: each tag
//! written, where it came from, a source, the song's own tags, its
//! album's or its place there, with the cleaning that changed it, and
//! below it what other sources offer apart from it, so a tag set by
//! hand is chosen with what it overrides in sight. They are the
//! table's prefix, which a save puts back as the song list had it, so
//! none reaches the song list; a value is shown on one line and cut
//! short, so none can end its comment and read as TOML. What each
//! source held when its song was built, `held`, is muman's record and
//! is left out of the file, then put back for every key a table lists,
//! whichever song listed it before, so a source joined to another song
//! keeps its record.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

use crate::ui::Prompter;
use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::change::{self, Read};
use crate::dirs::Dirs;
use crate::held::Held;
use crate::manifest::{Edit, Rewritten, SongTable};
use crate::query::View;
use crate::reconcile::Resolver;
use crate::resolve::{self, Offered, Resolved};
use crate::source::SourceKey;
use crate::store::Store;
use crate::tags::Field;

const HEADER: &str = "\
# Edit the songs below, then save and close; quit without saving to
# change nothing. Each [[song]] is one song, named by its id: leave the
# id as it is. Delete a [[song]] to remove it. Move a key from one
# song's sources to another's to join them, or into a new [[song]]
# without an id to make it a song of its own. A tag left empty keeps
# what the sources offer; above each song is what it resolves to now,
# each tag with where it comes from and what other sources offer.
";

/// Lines muman puts atop a file it reopens.
const PROBLEM: &str = "# ERROR: ";
const NOTICE: &str = "# NOTE: ";

/// One song as opened: its id, its name, its table as listed, and the
/// comments on where its tags come from.
#[derive(Clone)]
struct Opened {
    id: SourceKey,
    name: String,
    note: crate::manifest::Note,
    table: Table,
    origins: Vec<String>,
}

/// The longest a value shows in a comment, in columns.
const SHOWN: usize = 60;

/// `values` as one comment can hold them: on one line, cut short.
fn shown(values: &[String]) -> String {
    let joined = crate::ui::printable(&values.join("; "))
        .replace('\n', "⏎")
        .replace('\t', " ");
    crate::rows::cut(&joined, SHOWN)
}

/// A comment line for each tag `r` writes, with where it came from,
/// and below it each other source's offer apart from it.
fn origins(r: &Resolved, offers: &BTreeMap<Field, Vec<Offered>>) -> Vec<String> {
    let measured = |k: &str| k.starts_with("REPLAYGAIN_") || k.starts_with("R128_");
    let width = (r.plan.tags.iter())
        .filter(|(k, _)| !measured(k))
        .map(|(k, _)| k.len())
        .max()
        .unwrap_or(0);
    let mut lines = Vec::new();
    for ((key, values), why) in r.plan.tags.iter().zip(&r.why.tags) {
        if measured(key) {
            continue;
        }
        let mut from = why.from.clone();
        if !why.cleaned.is_empty() {
            from = format!("{from}, cleaned: {}", why.cleaned.join(", "));
        }
        lines.push(format!("# {key:<width$}  {}  ← {from}", shown(values)));
        let others = Field::named(key)
            .and_then(|f| offers.get(&f))
            .into_iter()
            .flatten()
            .filter(|o| o.values != *values && !o.values.is_empty());
        for o in others {
            lines.push(format!(
                "# {:width$}    or {}  ← {}",
                "",
                shown(&o.values),
                o.key
            ));
        }
    }
    lines
}

/// `table` with what each key it lists held when built, from `held`.
fn with_held(mut table: Table, held: &BTreeMap<SourceKey, Held>) -> Table {
    let keys: Vec<SourceKey> = (table.get("sources").and_then(Item::as_array))
        .into_iter()
        .flatten()
        .filter_map(|v| SourceKey::parse(v.as_str()?).ok())
        .collect();
    for key in keys {
        if let Some(h) = held.get(&key) {
            crate::held::set(&mut table, &key, h);
        }
    }
    table
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
    format!("{HEADER}{}", render_doc(opened, views))
}

/// The songs of the file the editor opens, below its header.
fn render_doc(opened: &[Opened], views: &[&View]) -> DocumentMut {
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
            .filter(|(k, _)| k.get() != "id" && k.get() != "held")
        {
            table.insert_formatted(key, item.clone());
        }
        let mut prefix = format!("\n# {}\n", describe(view));
        if let Some(path) = &view.path {
            let _ = writeln!(prefix, "# → {}", path.display());
        }
        for line in &o.origins {
            prefix.push_str(line);
            prefix.push('\n');
        }
        table.decor_mut().set_prefix(prefix);
        list.push(table);
    }
    doc.insert("song", Item::ArrayOfTables(list));
    doc
}

/// `item` as its value reads, without the spaces and comments around it.
fn bare(item: Option<&Item>) -> Option<String> {
    let mut item = item?.clone();
    if let Item::Value(v) = &mut item {
        v.decor_mut().clear();
    }
    Some(item.to_string().trim().to_string())
}

/// The keys of `tables`, each once, in the order first met, but the id
/// and the record of what was held, which are muman's.
fn keys_of(tables: &[&Table]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for (k, _) in tables.iter().flat_map(|t| t.iter()) {
        if k != "id" && k != "held" && !keys.iter().any(|x| x == k) {
            keys.push(k.to_string());
        }
    }
    keys
}

/// `theirs`, a song as the song list holds it now, with each value the
/// user changed in `mine` from `base`, what was first shown, taken from
/// `mine`; a key the user deleted is deleted.
fn merge(base: &Table, mine: &Table, theirs: &mut Table) {
    for key in keys_of(&[base, mine]) {
        let (b, m) = (base.get(&key), mine.get(&key));
        if let Some(Item::Table(m)) = m
            && let Some(Item::Table(t)) = theirs.get_mut(&key)
        {
            let empty = Table::new();
            merge(b.and_then(Item::as_table).unwrap_or(&empty), m, t);
            continue;
        }
        if bare(b) == bare(m) {
            continue;
        }
        match mine.get_key_value(&key) {
            Some((k, item)) => {
                theirs.insert_formatted(k, item.clone());
            }
            None => {
                theirs.remove(&key);
            }
        }
    }
}

/// The fields of a song that differ between `then` and `now`, each named
/// as the file names it, as `tags.genre`.
fn changed_fields(then: &Table, now: &Table, prefix: &str, out: &mut Vec<String>) {
    for key in keys_of(&[then, now]) {
        let (a, b) = (then.get(&key), now.get(&key));
        let name = format!("{prefix}{key}");
        match (a, b) {
            (Some(Item::Table(a)), Some(Item::Table(b))) => {
                changed_fields(a, b, &format!("{name}."), out);
            }
            (Some(Item::Table(t)), None) | (None, Some(Item::Table(t))) => {
                changed_fields(t, &Table::new(), &format!("{name}."), out);
            }
            _ if bare(a) != bare(b) => out.push(name),
            _ => {}
        }
    }
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
    t.remove("held");
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
    let held: BTreeMap<SourceKey, Held> = (opened.iter())
        .filter_map(|o| read.manifest.song_with(&o.id))
        .flat_map(|s| s.held.iter().map(|(k, h)| (k.clone(), h.clone())))
        .collect();
    // Numbered as the file numbers its songs, before the whole list's
    // reading numbers them as the list does.
    let mut listed: BTreeMap<SourceKey, usize> = BTreeMap::new();
    let not_opened = |k: &SourceKey| {
        read.manifest
            .songs
            .iter()
            .filter(|s| s.id().is_none_or(|id| !opened.iter().any(|o| &o.id == id)))
            .find(|s| s.sources.contains(k))
            .and_then(|s| s.id().cloned())
    };
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
        let sources = Sources {
            known: &known,
            store,
            not_opened: &not_opened,
        };
        sources.check(&table, n, &mut listed, &mut problems);
        match id.as_ref().map(|i| i.as_str()) {
            None => new.push(SongTable(with_held(table, &held))),
            Some(Some(id)) => match opened.iter().find(|o| o.id.to_string() == id) {
                Some(o) if seen.contains(&o.id) => {
                    problems.push(format!("{what}: {id} is the id of an earlier song too"));
                }
                Some(o) => {
                    seen.push(o.id.clone());
                    if content(&table) != content(&o.table) {
                        let table = with_held(table, &held);
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
        songs.push((o.id.clone(), Rewritten::Removed(o.note.clone())));
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

/// What the sources a song of the file lists are checked against.
struct Sources<'a> {
    /// The sources of the songs opened.
    known: &'a [&'a SourceKey],
    store: &'a Store,
    /// The song not opened that lists a source, if any.
    not_opened: &'a dyn Fn(&SourceKey) -> Option<SourceKey>,
}

impl Sources<'_> {
    /// What is wrong with the sources of `table`, the file's song `n`:
    /// one of no song opened and not in the store, or one `listed` by
    /// another song, the file's or one not opened.
    fn check(
        &self,
        table: &Table,
        n: usize,
        listed: &mut BTreeMap<SourceKey, usize>,
        problems: &mut Vec<String>,
    ) {
        let what = format!("song {}", n + 1);
        for key in table
            .get("sources")
            .and_then(Item::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
        {
            match SourceKey::parse(key) {
                Ok(k) if !self.known.contains(&&k) && !self.store.has(&k) => problems.push(
                    format!("{what}: {k} is a source of no song opened and is not in the store"),
                ),
                Ok(k) if crate::manifest::shareable(&k) => {}
                Ok(k) => {
                    if let Some(other) = listed.get(&k) {
                        problems.push(format!("{what}: {k} is listed by song {} too", other + 1));
                    } else if let Some(owner) = (self.not_opened)(&k) {
                        problems.push(format!(
                            "{what}: {k} is listed by {owner}, a song not opened"
                        ));
                    }
                    listed.insert(k, n);
                }
                Err(e) => problems.push(format!("{what}: {e:#}")),
            }
        }
    }
}

/// Say what an edit does, song by song; one in `gone`, removed from the
/// song list while it was open, takes none of it.
fn summarize<W: Write>(
    edit: &Edit,
    opened: &[Opened],
    gone: &[SourceKey],
    out: &mut W,
) -> Result<()> {
    let Edit::Rewrite { songs, new } = edit else {
        return Ok(());
    };
    let keys = |t: &Table| -> Vec<String> {
        t.get("sources")
            .and_then(Item::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    };
    let listed: Vec<String> = songs
        .iter()
        .filter_map(|(_, to)| match to {
            Rewritten::Table(t) => Some(&t.0),
            Rewritten::Removed(_) => None,
        })
        .chain(new.iter().map(|t| &t.0))
        .flat_map(keys)
        .collect();
    for (id, to) in songs {
        let name = opened
            .iter()
            .find(|o| &o.id == id)
            .map_or_else(|| id.to_string(), |o| o.name.clone());
        match to {
            _ if gone.contains(id) => {
                crate::ui::warning(out, &format!("Not changed: {name}, no longer listed"))?;
            }
            Rewritten::Table(_) => crate::ui::info(out, &format!("Changed: {name}"))?,
            Rewritten::Removed(_)
                if opened
                    .iter()
                    .find(|o| &o.id == id)
                    .is_some_and(|o| keys(&o.table).iter().all(|k| listed.contains(k))) =>
            {
                crate::ui::info(out, &format!("Joined to another song: {name}"))?;
            }
            Rewritten::Removed(_) => crate::ui::warning(out, &format!("Removed: {name}"))?,
        }
    }
    for _ in new {
        crate::ui::info(out, "Split off: a new song")?;
    }
    Ok(())
}

/// Open the songs the query picks in an editor, run by `editor` on the
/// file's path, and apply what changed, under the run `later` begins as
/// it saves. Returns whether anything did.
// Writes the file the editor is opened on, in scratch.
#[expect(clippy::disallowed_methods)]
pub fn edit<W: Write>(
    dirs: &Dirs,
    terms: &[String],
    all: bool,
    mut prompter: Option<&mut dyn Prompter>,
    editor: &mut dyn FnMut(&Path) -> Result<()>,
    later: &mut crate::history::Later,
    out: &mut W,
) -> Result<bool> {
    if prompter.is_none() {
        return Err(change::Refused("Editing needs a terminal".into()).into());
    }
    let mut read = Read::new(dirs, terms)?;
    let all = all || read.query.is_empty();
    let picked = read.pick(all, "edit", change::reborrow(&mut prompter))?;
    if picked.is_empty() {
        crate::ui::info(out, "Nothing to edit")?;
        return Ok(false);
    }
    let store = Store::scan(dirs)?;
    let mut opened = open(&read, &picked, &dirs.library)?;
    let mut views: Vec<View> = picked.iter().map(|n| read.views[*n].clone()).collect();
    let mut first = render(&opened, &views.iter().collect::<Vec<_>>());
    let dir = tempfile::tempdir().context("creating a temporary directory")?;
    let file = dir.path().join("songs.toml");
    let mut shown = first.clone();
    let mut expected = read.songs(&picked);
    let mut gone = Vec::new();
    loop {
        std::fs::write(&file, &shown).with_context(|| format!("writing {}", file.display()))?;
        let modified = |f: &Path| std::fs::metadata(f).and_then(|m| m.modified()).ok();
        let written = modified(&file);
        editor(&file)?;
        // An editor on Windows may save with CRLF line endings, which
        // would otherwise count as a change to every line.
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?
            .replace("\r\n", "\n");
        // Quit without saving, whatever was shown last: the file as
        // written, untouched since.
        let saved = modified(&file) != written || text != shown;
        if !saved || without_notes(&text) == without_notes(&first) {
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
        later.begin()?;
        read.manifest.edit(edit.clone());
        let changed = read.manifest.save_if_unchanged(&expected)?;
        if changed.is_empty() {
            later.saved()?;
            summarize(&edit, &opened, &gone, out)?;
            return Ok(true);
        }
        later.abandon();
        read = Read::new(dirs, terms)?;
        gone = opened
            .iter()
            .filter(|o| read.manifest.song_with(&o.id).is_none())
            .map(|o| o.id.clone())
            .collect();
        let then = std::mem::take(&mut opened);
        let (now, now_views, live) = reopen(&read, then.clone(), views, &dirs.library)?;
        let mut said = Vec::new();
        for (was, is) in then.iter().zip(&now) {
            let mut fields = Vec::new();
            changed_fields(&was.table, &is.table, "", &mut fields);
            if gone.contains(&was.id) {
                said.push(format!("{} is no longer listed", was.id));
            } else if !fields.is_empty() {
                said.push(format!("{} sets {}", was.id, change::spoken_owned(&fields)));
            }
        }
        let doc = render_doc(&now, &now_views.iter().collect::<Vec<_>>());
        first = format!("{HEADER}{doc}");
        let merged = merged(&text, &then, doc);
        (opened, views) = (now, now_views);
        expected = read.songs(&live);
        let what = if said.is_empty() {
            names(&changed)
        } else {
            said.join("; ")
        };
        shown = with_notes(
            &format!("{HEADER}{merged}"),
            NOTICE,
            &[format!(
                "The song list changed while this was open: {what}.\nBelow is each song as it is now, \
                 your edits over it. Save again to apply them; quit without saving to keep it."
            )],
        );
    }
}

/// `keys` as a note names them.
fn names(keys: &[SourceKey]) -> String {
    let names: Vec<String> = keys.iter().map(ToString::to_string).collect();
    change::spoken_owned(&names)
}

/// `opened` as the song list holds each now, its comments made anew,
/// with what each shows and the songs' places in `read`; one no longer
/// listed under its id stays as it was opened.
fn reopen(
    read: &Read,
    opened: Vec<Opened>,
    views: Vec<View>,
    library: &Path,
) -> Result<(Vec<Opened>, Vec<View>, Vec<usize>)> {
    let at = |o: &Opened| (read.manifest.songs.iter()).position(|s| s.id() == Some(&o.id));
    let live: Vec<usize> = opened.iter().filter_map(at).collect();
    let mut fresh = open(read, &live, library)?.into_iter();
    let mut now = Vec::new();
    let mut shown = Vec::new();
    for (o, view) in opened.into_iter().zip(views) {
        if let Some(n) = at(&o) {
            now.push(fresh.next().context("a song reopened twice")?);
            shown.push(read.views[n].clone());
        } else {
            now.push(o);
            shown.push(view);
        }
    }
    Ok((now, shown, live))
}

/// The file `mine` the user saved, merged into `theirs`, the songs
/// `then` as the song list holds them now: [`merge`] for each song the
/// user kept, none the user deleted, and any song of the user's own
/// after them, as one split off.
fn merged(mine: &str, then: &[Opened], mut theirs: DocumentMut) -> DocumentMut {
    let Ok(mine) = mine.parse::<DocumentMut>() else {
        return theirs;
    };
    let mine: Vec<&Table> = (mine.get("song").and_then(Item::as_array_of_tables))
        .into_iter()
        .flat_map(|list| list.iter())
        .collect();
    let id_of = |t: &Table| t.get("id").and_then(Item::as_str).map(str::to_string);
    let Some(list) = theirs
        .get_mut("song")
        .and_then(Item::as_array_of_tables_mut)
    else {
        return theirs;
    };
    let mut kept = toml_edit::ArrayOfTables::new();
    for mut table in list.iter().cloned() {
        let id = id_of(&table);
        let base = then.iter().find(|o| Some(o.id.to_string()) == id);
        let edited = mine.iter().find(|t| id_of(t) == id);
        match (base, edited) {
            (Some(base), Some(edited)) => {
                merge(&base.table, edited, &mut table);
                kept.push(table);
            }
            (Some(_), None) => {}
            (None, _) => kept.push(table),
        }
    }
    let opened =
        |t: &&&Table| id_of(t).is_some_and(|id| then.iter().any(|o| o.id.to_string() == id));
    for table in mine.iter().filter(|t| !opened(t)) {
        kept.push((*table).clone());
    }
    *list = kept;
    theirs
}

/// The songs at `picked` as opened, each with where its tags come from.
fn open(read: &Read, picked: &[usize], library: &Path) -> Result<Vec<Opened>> {
    let ids: Vec<SourceKey> = picked
        .iter()
        .filter_map(|n| read.views[*n].id().cloned())
        .collect();
    let mut tables = read.manifest.tables_by_id(&ids)?;
    let settings = &read.manifest.settings;
    let resolver = Resolver::new(&read.manifest, settings, &read.state, library)?;
    let mut opened = Vec::new();
    for n in picked {
        let id = read.views[*n]
            .id()
            .context("a song lists no source")?
            .clone();
        let Some(table) = tables.remove(&id) else {
            bail!("{id} is no longer listed");
        };
        let song = &read.manifest.songs[*n];
        let origins = read.resolved[*n].as_ref().map_or_else(Vec::new, |r| {
            origins(r, &resolve::offers(&resolver.input(song, None)))
        });
        opened.push(Opened {
            id,
            name: read.views[*n].name(),
            note: read.views[*n].note(),
            table,
            origins,
        });
    }
    Ok(opened)
}

/// Run `$VISUAL`, else `$EDITOR`, else the system's own, on `path`. On
/// Unix the variable is a shell command, as other programs read it; on
/// Windows it is split into words as Windows programs split a command
/// line and its program found with `PATHEXT`, so `code --wait` finds
/// `code.cmd`, and the path is passed as an argument of its own rather
/// than through cmd.exe's quoting.
// Runs the editor the user names.
#[expect(clippy::disallowed_methods)]
pub fn launch(path: &Path) -> Result<()> {
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|e| !e.trim().is_empty()))
        .unwrap_or_else(|| if cfg!(windows) { "notepad" } else { "vi" }.to_string());
    let _theirs = crate::runner::Interactive::begin();
    let status = if cfg!(windows) {
        let words = windows_words(&editor)?;
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

/// `editor` as the words a Windows program reads, so the `\` between
/// folders in `C:\Tools\edit.exe` stays.
fn windows_words(editor: &str) -> Result<Vec<String>> {
    let words = crate::tools::windows_words(editor);
    if words.is_empty() {
        bail!("reading the editor `{editor}`: it names no program");
    }
    Ok(words)
}

#[cfg(test)]
mod tests;
