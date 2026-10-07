//! Changes to songs a query picks: removing them, listing removed ones
//! again, and setting their tags and pins. Each says what it would do,
//! asks on a terminal or takes `-y`, and records its edits only while
//! the songs are still listed as they were read.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::ui::Prompter;
use anyhow::{Context, Result, bail};
use toml_edit::{Table, value};

use crate::cli::Confirm;
use crate::dirs::Dirs;
use crate::manifest::{self, Edit, Manifest, Rewritten, Song, SongTable};
use crate::query::{self, Query, View};
use crate::source::SourceKey;
use crate::state::State;
use crate::store::Store;

/// A change refused for how it was asked: nothing matched, or it needs
/// a terminal, `-y` or `--all`.
#[derive(Debug)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

fn refuse<T>(why: impl Into<String>) -> Result<T> {
    Err(Refused(why.into()).into())
}

/// The prompter for one call, the rest of the function keeping it.
pub fn reborrow<'a>(prompter: &'a mut Option<&mut dyn Prompter>) -> Option<&'a mut dyn Prompter> {
    match prompter {
        Some(p) => Some(&mut **p),
        None => None,
    }
}

fn label(view: &View) -> String {
    format!("{} — {}", view.first("key"), view.name())
}

/// The songs among `views` the query matches: one, all of them under
/// `--all` or when the query is keys alone, each naming one song, else
/// those picked on a terminal.
/// Refused when none match, or several without a terminal or `--all`.
pub fn pick(
    views: &[View],
    query: &Query,
    all: bool,
    verb: &str,
    prompter: Option<&mut dyn Prompter>,
) -> Result<Vec<usize>> {
    let found: Vec<usize> = (0..views.len())
        .filter(|n| !views[*n].keys.is_empty() && query.matches(&views[*n]))
        .collect();
    if found.is_empty() {
        return refuse("No song matches the query");
    }
    if found.len() == 1 || all || (query.names_keys() && found.len() <= query.keys_named()) {
        return Ok(found);
    }
    let labels: Vec<String> = found.iter().map(|n| label(&views[*n])).collect();
    match prompter {
        Some(p) => {
            let picked = p.choose(
                &format!("{} songs match; which to {verb}?", found.len()),
                &labels,
            )?;
            Ok(picked
                .into_iter()
                .filter_map(|i| found.get(i).copied())
                .collect())
        }
        None => refuse(format!(
            "{} songs match; narrow the query, or give --all to {verb} every one:\n  {}",
            found.len(),
            labels.join("\n  ")
        )),
    }
}

/// Whether to go ahead with what was said: never on a dry run, at once
/// with `-y`, else as the terminal answers.
pub fn confirmed<W: Write>(
    confirm: &Confirm,
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    if confirm.dry_run {
        return Ok(false);
    }
    if confirm.yes {
        return Ok(true);
    }
    match prompter {
        Some(p) => {
            let yes = p.ask("Go ahead?")?;
            if !yes {
                crate::ui::info(out, "Nothing changed")?;
            }
            Ok(yes)
        }
        None => refuse("Without a terminal to ask on, -y makes the change"),
    }
}

/// What the songs a query reads are, with the list and state they came
/// from.
#[derive(Debug)]
pub struct Read {
    pub manifest: Manifest,
    pub state: State,
    pub views: Vec<View>,
    pub query: Query,
}

impl Read {
    pub fn new(dirs: &Dirs, terms: &[String]) -> Result<Self> {
        let manifest = Manifest::load(&dirs.home)?;
        let state = State::load(&dirs.home)?;
        let views = query::views(&manifest, &state, &dirs.library)?;
        let query = Query::parse(terms, &query::extractors(&manifest))?;
        query.check_fields(&views)?;
        Ok(Self {
            manifest,
            state,
            views,
            query,
        })
    }

    /// The songs at `picked`, as they were read.
    #[must_use]
    pub fn songs(&self, picked: &[usize]) -> Vec<Song> {
        picked
            .iter()
            .map(|n| self.manifest.songs[*n].clone())
            .collect()
    }

    /// Save the recorded edits unless a picked song changed meanwhile.
    pub fn save(&mut self, picked: &[usize]) -> Result<()> {
        let expected = self.songs(picked);
        let changed = self.manifest.save_if_unchanged(&expected)?;
        if !changed.is_empty() {
            self.manifest.discard();
            bail!(
                "the song list changed while this ran, for {}; nothing was changed, run it again",
                changed
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Ok(())
    }
}

/// Every file in the store a key brings: what yt-dlp fetched under its
/// ID, or a manual file with its own lyrics and pictures.
fn store_files(store: &Store, dirs: &Dirs, key: &SourceKey) -> Result<Vec<PathBuf>> {
    Ok(match key {
        SourceKey::Remote { site, id } if crate::store::kept(site).is_some() => {
            crate::store::kept(site).map_or_else(Vec::new, |k| k.files(dirs, id))
        }
        SourceKey::Remote { id, .. } => store.fetched_files(id)?,
        SourceKey::Manual(_) => store.locate(key).map_or_else(Vec::new, |l| {
            let stem = l.path.with_extension("");
            std::iter::once(l.path.clone())
                .chain(l.lyrics)
                .chain(
                    l.covers
                        .into_iter()
                        .filter(|c| c.with_extension("") == stem),
                )
                .collect()
        }),
    })
}

/// Remove the songs the query picks, and with `purge` their sources.
/// Returns whether anything was removed.
pub fn remove<W: Write>(
    dirs: &Dirs,
    terms: &[String],
    purge: bool,
    confirm: &Confirm,
    mut prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    let mut read = Read::new(dirs, terms)?;
    let picked = pick(
        &read.views,
        &read.query,
        confirm.all,
        "remove",
        reborrow(&mut prompter),
    )?;
    if picked.is_empty() {
        crate::ui::info(out, "Nothing removed")?;
        return Ok(false);
    }
    let store = Store::scan(dirs)?;
    let mut doomed: Vec<(PathBuf, bool)> = Vec::new();
    crate::ui::info(out, &format!("Removing {} song(s):", picked.len()))?;
    for n in &picked {
        let view = &read.views[*n];
        writeln!(out, "  {}", label(view))?;
        let written = read
            .state
            .outputs
            .iter()
            .filter(|(_, w)| view.id().is_some_and(|id| w.sources.contains(id)));
        for (path, w) in written {
            for file in std::iter::once(path).chain(&w.lyrics) {
                writeln!(out, "    deletes {}", dirs.library.join(file).display())?;
            }
        }
        if purge {
            // A record another song still lists stays for that song.
            let shared = |k: &SourceKey| {
                manifest::shareable(k)
                    && read
                        .manifest
                        .songs
                        .iter()
                        .enumerate()
                        .any(|(m, s)| !picked.contains(&m) && s.has(k))
            };
            for key in view.keys.iter().filter(|k| !shared(k)) {
                let own = matches!(key, SourceKey::Manual(_));
                for file in store_files(&store, dirs, key)? {
                    let how = if own { "trashes" } else { "deletes" };
                    writeln!(out, "    {how} {}", file.display())?;
                    doomed.push((file, own));
                }
            }
        }
    }
    if !confirmed(confirm, prompter, out)? {
        return Ok(false);
    }
    for n in &picked {
        let view = &read.views[*n];
        let Some(id) = view.id() else {
            continue;
        };
        read.manifest.edit(Edit::Remove {
            key: id.clone(),
            note: view.name(),
        });
    }
    read.save(&picked)?;
    for (file, own) in doomed {
        if own {
            to_trash(&file).with_context(|| format!("moving {} to the trash", file.display()))?;
        } else {
            match std::fs::remove_file(&file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(e).with_context(|| format!("deleting {}", file.display()));
                }
                _ => {}
            }
        }
    }
    Ok(true)
}

/// Move a file of the user's own to the desktop's trash. On macOS this
/// asks the file manager API directly rather than scripting Finder,
/// which prompts for permission and fails over SSH.
fn to_trash(file: &Path) -> Result<(), trash::Error> {
    #[allow(unused_mut)]
    let mut trash = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        trash.set_delete_method(DeleteMethod::NsFileManager);
    }
    trash.delete(file)
}

/// List again the removed songs the query picks. Returns whether any
/// was.
pub fn restore<W: Write>(
    dirs: &Dirs,
    terms: &[String],
    confirm: &Confirm,
    mut prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    let mut manifest = Manifest::load(&dirs.home)?;
    let query = Query::parse(terms, &query::extractors(&manifest))?;
    let views: Vec<View> = manifest.removed.iter().map(query::removed_view).collect();
    query.check_fields(&views)?;
    let picked = pick(
        &views,
        &query,
        confirm.all,
        "restore",
        reborrow(&mut prompter),
    )?;
    if picked.is_empty() {
        crate::ui::info(out, "Nothing restored")?;
        return Ok(false);
    }
    crate::ui::info(out, &format!("Listing {} song(s) again:", picked.len()))?;
    for n in &picked {
        writeln!(out, "  {}", label(&views[*n]))?;
    }
    if !confirmed(confirm, prompter, out)? {
        return Ok(false);
    }
    for id in picked.iter().filter_map(|n| views[*n].id()) {
        manifest.edit(Edit::Restore(id.clone()));
    }
    manifest.save()?;
    Ok(true)
}

/// One `NAME=VALUE` or `NAME!` of `set`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Assign {
    Tag(String, Vec<String>),
    Pin(&'static str, Option<String>),
    Offset(Option<i64>),
}

/// The query terms and the assignments among `terms`: a word with `=`
/// before any `:`, or ending in `!`, assigns.
fn split_terms(terms: &[String]) -> Result<(Vec<String>, Vec<Assign>)> {
    let mut query = Vec::new();
    let mut tags: Vec<(String, Vec<String>)> = Vec::new();
    let mut other = Vec::new();
    for term in terms {
        let assignment = match term.split_once('=') {
            Some((name, v)) if !name.contains(':') && !name.is_empty() => Some((name, Some(v))),
            _ => term
                .strip_suffix('!')
                .filter(|n| !n.contains(':') && !n.is_empty())
                .map(|n| (n, None)),
        };
        let Some((name, v)) = assignment else {
            query.push(term.clone());
            continue;
        };
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "audio" | "cover" | "lyrics" => {
                let pin = match lower.as_str() {
                    "audio" => "audio",
                    "cover" => "cover",
                    _ => "lyrics",
                };
                if let Some(v) = v
                    && v != "false"
                {
                    SourceKey::parse(v).with_context(|| format!("`{term}`"))?;
                }
                if v == Some("false") && pin != "lyrics" {
                    bail!("`{term}`: only lyrics can be false");
                }
                other.push(Assign::Pin(pin, v.map(str::to_string)));
            }
            "lyrics_offset" => {
                let ms = v
                    .map(|v| {
                        v.parse::<crate::units::Time>()
                            .map(|t| t.0)
                            .map_err(|e| anyhow::anyhow!("`{term}`: {e}"))
                    })
                    .transpose()?;
                other.push(Assign::Offset(ms));
            }
            // The name before times took their unit, in milliseconds.
            "lyrics_offset_ms" => {
                let ms = v
                    .map(|v| {
                        v.parse::<i64>()
                            .with_context(|| format!("`{term}` is no number"))
                    })
                    .transpose()?;
                other.push(Assign::Offset(ms));
            }
            _ => match (tags.iter_mut().find(|(n, _)| *n == name), v) {
                (Some((_, values)), Some(v)) => values.push(v.to_string()),
                (Some(_), None) => bail!("`{term}` both sets and clears {name}"),
                (None, v) => tags.push((
                    name.to_string(),
                    v.map(str::to_string).into_iter().collect(),
                )),
            },
        }
    }
    let mut assigns: Vec<Assign> = tags.into_iter().map(|(n, v)| Assign::Tag(n, v)).collect();
    assigns.extend(other);
    Ok((query, assigns))
}

impl Assign {
    /// How it changes `view`'s song, in words.
    fn describe(&self, view: &View) -> String {
        match self {
            Self::Tag(name, values) => {
                let now = view.values(name).join("; ");
                let now = if now.is_empty() {
                    "nothing".to_string()
                } else {
                    now
                };
                if values.is_empty() {
                    format!("{name}: {now} → what the sources offer")
                } else {
                    format!("{name}: {now} → {}", values.join("; "))
                }
            }
            Self::Pin(pin, Some(key)) => format!("{pin}: pinned to {key}"),
            Self::Pin(pin, None) => format!("{pin}: picked by measure"),
            Self::Offset(Some(ms)) => format!("lyrics later by {ms} ms"),
            Self::Offset(None) => "lyrics offset: none".to_string(),
        }
    }

    fn apply(&self, table: &mut Table, song: &Song) -> Result<()> {
        match self {
            Self::Tag(name, values) => manifest::set_tags(table, &[(name.clone(), values.clone())]),
            Self::Pin(pin, Some(v)) if v == "false" => {
                table.insert(pin, value(false));
            }
            Self::Pin(pin, Some(v)) => {
                let key = SourceKey::parse(v)?;
                if !song.has(&key) {
                    bail!(
                        "{key} is not a source of the song listing {}",
                        song.id().map(ToString::to_string).unwrap_or_default()
                    );
                }
                table.insert(pin, value(v.as_str()));
            }
            Self::Pin(pin, None) => {
                table.remove(pin);
            }
            Self::Offset(Some(ms)) => {
                table.remove("lyrics_offset_ms");
                table.insert("lyrics_offset", value(crate::units::Time(*ms).exact()));
            }
            Self::Offset(None) => {
                table.remove("lyrics_offset_ms");
                table.remove("lyrics_offset");
            }
        }
        Ok(())
    }
}

/// Set what `terms` assign on the songs its query picks. Returns whether
/// anything was set.
pub fn set<W: Write>(
    dirs: &Dirs,
    terms: &[String],
    confirm: &Confirm,
    mut prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    let (query_terms, assigns) = split_terms(terms)?;
    if assigns.is_empty() {
        return refuse("Nothing to set: give NAME=VALUE or NAME!");
    }
    if query_terms.is_empty() {
        return refuse("Name the songs to set with a query");
    }
    let mut read = Read::new(dirs, &query_terms)?;
    let picked = pick(
        &read.views,
        &read.query,
        confirm.all,
        "set",
        reborrow(&mut prompter),
    )?;
    if picked.is_empty() {
        crate::ui::info(out, "Nothing set")?;
        return Ok(false);
    }
    let ids: Vec<SourceKey> = picked
        .iter()
        .filter_map(|n| read.manifest.songs[*n].id().cloned())
        .collect();
    let mut tables = read.manifest.tables_by_id(&ids)?;
    let mut songs = Vec::new();
    for n in &picked {
        let view = &read.views[*n];
        let song = &read.manifest.songs[*n];
        writeln!(out, "  {}", label(view))?;
        for a in &assigns {
            writeln!(out, "    {}", a.describe(view))?;
        }
        let id = song.id().context("a song lists no source")?.clone();
        let Some(mut table) = tables.remove(&id) else {
            bail!("{id} is no longer listed");
        };
        for a in &assigns {
            a.apply(&mut table, song)?;
        }
        songs.push((id, Rewritten::Table(SongTable(table))));
    }
    if !confirmed(confirm, prompter, out)? {
        return Ok(false);
    }
    read.manifest.edit(Edit::Rewrite {
        songs,
        new: Vec::new(),
    });
    read.save(&picked)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(t: &[&str]) -> Vec<String> {
        t.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn words_with_an_equals_or_a_bang_assign_and_the_rest_query() {
        let (query, assigns) = split_terms(&strings(&[
            "daft",
            "title:=x",
            "genre=House",
            "genre=French",
            "date!",
            "audio=youtube.com:aaaaaaaaaaa",
            "lyrics=false",
            "lyrics_offset_ms=120",
        ]))
        .unwrap();
        assert_eq!(query, strings(&["daft", "title:=x"]));
        assert_eq!(
            assigns,
            [
                Assign::Tag("genre".into(), strings(&["House", "French"])),
                Assign::Tag("date".into(), Vec::new()),
                Assign::Pin("audio", Some("youtube.com:aaaaaaaaaaa".into())),
                Assign::Pin("lyrics", Some("false".into())),
                Assign::Offset(Some(120)),
            ]
        );
        assert!(split_terms(&strings(&["cover=false"])).is_err());
        assert!(split_terms(&strings(&["lyrics_offset_ms=soon"])).is_err());
        assert!(split_terms(&strings(&["genre=a", "genre!"])).is_err());
    }

    fn views(n: usize) -> Vec<View> {
        (0..n)
            .map(|i| View {
                keys: vec![SourceKey::youtube(&format!("{i:0>11}"))],
                ..View::default()
            })
            .collect()
    }

    #[test]
    fn several_matches_need_all_or_a_terminal() {
        let q = Query::default();
        assert_eq!(pick(&views(1), &q, false, "remove", None).unwrap(), [0]);
        let e = pick(&views(2), &q, false, "remove", None).unwrap_err();
        assert!(e.downcast_ref::<Refused>().is_some(), "{e:#}");
        assert_eq!(pick(&views(2), &q, true, "remove", None).unwrap(), [0, 1]);
        let mut p = crate::ui::MockPrompter::new();
        p.push_choice([1]);
        assert_eq!(
            pick(&views(3), &q, false, "remove", Some(&mut p)).unwrap(),
            [1]
        );
        assert!(pick(&[], &q, true, "remove", None).is_err());
    }

    #[test]
    fn a_change_without_a_terminal_needs_yes() {
        let mut out = Vec::new();
        let ask = Confirm::default();
        assert!(confirmed(&ask, None, &mut out).is_err());
        let yes = Confirm {
            yes: true,
            ..Confirm::default()
        };
        assert!(confirmed(&yes, None, &mut out).unwrap());
        let dry = Confirm {
            yes: true,
            dry_run: true,
            ..Confirm::default()
        };
        assert!(!confirmed(&dry, None, &mut out).unwrap());
    }
}
