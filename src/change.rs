//! Changes to songs a query picks: removing them, listing removed ones
//! again, and setting their tags and pins. Each says what it would do,
//! asks on a terminal or takes `-y`, and records its edits only while
//! the songs are still listed as they were read.
//!
//! Songs to pick among are shown as [`crate::rows`] shows them, by what
//! tells them apart, and none is picked to begin with: a pick that
//! starts with every song ticked makes one Enter a change to all of them.
//! What a change does is said song by song, `−` for a song that leaves
//! the list, `~` for one that changes and `+` for one that comes back,
//! then counted with what `muman undo` can put back, and the question
//! names the change. Most changes take a yes, as `undo` reverses them.
//! One that moves files of the user's own to the trash takes the count
//! of songs typed instead: `undo` cannot take a file back from the
//! desktop's trash on every platform, and a yes given from habit should
//! not be enough to put it there.
//!
//! What `remove` says of the library files it deletes and leaves is what
//! the sync after it decides ([`crate::reconcile::foresight`]), of the songs
//! planned as the list is without the songs it removes: a file beside a
//! song that the user changed since muman wrote it is left in place, said
//! so and not counted, as the sync leaves it, and a folder's file the next
//! song of its folder takes is not said at all.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;

use crate::ui::Prompter;
use anyhow::{Context, Result, bail};
use toml_edit::{Table, value};

use crate::cli::Confirm;
use crate::dirs::Dirs;
use crate::manifest::{self, Edit, Manifest, Rewritten, Song, SongTable};
use crate::query::QueryMatch;
use crate::query::{self, Query, View};
use crate::reconcile::Pruned;
use crate::relpath;
use crate::resolve::Resolved;
use crate::rows;
use crate::source::SourceKey;
use crate::state::State;
use crate::store::Store;

pub use crate::errors::Refused;

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

/// How a song is named in what a change says: its name, its album and
/// its key.
pub(crate) fn named(view: &View) -> String {
    let album = view.first("album");
    let key = view.id().map(ToString::to_string).unwrap_or_default();
    if album.is_empty() {
        format!("{} · {key}", view.name())
    } else {
        format!("{} · {album} · {key}", view.name())
    }
}

/// `items` as said aloud, as "genre, date and audio".
fn spoken(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `items` as said aloud, as [`spoken`] says them.
pub(crate) fn spoken_owned(items: &[String]) -> String {
    spoken(&items.iter().map(String::as_str).collect::<Vec<_>>())
}

/// `n` of `thing`, as "1 song" or "3 songs".
pub(crate) fn counted(n: usize, thing: &str) -> String {
    if n == 1 {
        format!("1 {thing}")
    } else {
        format!("{n} {thing}s")
    }
}

/// The songs among `views` the query matches: one, all of them under
/// `--all` or when the query is keys alone, each naming one song, else
/// those picked on a terminal, each shown by its `cells`.
/// Refused when none match, or several without a terminal or `--all`.
pub fn pick(
    views: &[View],
    cells: &dyn Fn(usize) -> rows::Cells,
    query: &Query,
    all: bool,
    verb: &str,
    prompter: Option<&mut dyn Prompter>,
) -> Result<Vec<usize>> {
    let found: Vec<usize> = (0..views.len())
        .filter(|n| !views[*n].keys.is_empty() && query.matches(&views[*n]))
        .collect();
    if found.is_empty() && verb == "restore" {
        return refuse("No removed song matches the query; `muman list --removed` lists them");
    }
    if found.is_empty() {
        return refuse("No song matches the query");
    }
    if found.len() == 1 || all || (query.names_keys() && found.len() <= query.keys_named()) {
        return Ok(found);
    }
    let shown: Vec<rows::Cells> = found.iter().map(|n| cells(*n)).collect();
    let rows = rows::fit(&shown, prompter.as_ref().map(|p| p.width()));
    let matched = match &rows.shared {
        Some(name) => format!("{} songs match, all {name}", found.len()),
        None => format!("{} songs match", found.len()),
    };
    match prompter {
        Some(p) => {
            let picked = p.choose(&format!("{matched}; which to {verb}?"), &rows.lines, &[])?;
            Ok(picked
                .into_iter()
                .filter_map(|i| found.get(i).copied())
                .collect())
        }
        None => refuse(format!(
            "{matched}; narrow the query, or give --all to {verb} every one:\n  {}",
            rows.lines.join("\n  ")
        )),
    }
}

/// What a change asks before it is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// A yes or no, the question naming the change.
    YesNo(String),
    /// A count to type, for a change `undo` cannot take back in full.
    Typed { count: usize, question: String },
}

/// Whether to go ahead with what was said: never on a dry run, at once
/// with `-y`, else as the terminal answers `ask`.
pub fn confirmed<W: Write>(
    confirm: &Confirm,
    ask: &Ask,
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    if confirm.dry_run {
        crate::ui::info(out, "Dry run: nothing changed")?;
        return Ok(false);
    }
    if confirm.yes {
        return Ok(true);
    }
    let Some(p) = prompter else {
        return refuse("Without a terminal to ask on, -y makes the change");
    };
    let yes = match ask {
        Ask::YesNo(question) => p.ask(question)?,
        Ask::Typed { count, question } => p
            .text(
                question,
                &format!("type {count} to go ahead; anything else changes nothing"),
            )?
            .is_some_and(|typed| typed.trim() == count.to_string()),
    };
    if !yes {
        crate::ui::info(out, "Nothing changed")?;
    }
    Ok(yes)
}

/// What the songs a query reads are, with the list and state they came
/// from.
#[derive(Debug)]
pub struct Read {
    pub manifest: Manifest,
    pub state: State,
    pub views: Vec<View>,
    /// Each song's plan, where it resolves, in the order of `views`.
    pub resolved: Vec<Option<Resolved>>,
    pub query: Query,
}

impl Read {
    pub fn new(dirs: &Dirs, terms: &[String]) -> Result<Self> {
        let manifest = Manifest::load(&dirs.home)?;
        let state = crate::reconcile::found(dirs)?;
        let (views, resolved): (Vec<View>, Vec<_>) =
            query::resolved_views(&manifest, &state, &dirs.library)?
                .into_iter()
                .unzip();
        let query = Query::parse(terms, &query::extractors(&manifest))?;
        query.check_fields(&views)?;
        Ok(Self {
            manifest,
            state,
            views,
            resolved,
            query,
        })
    }

    /// The songs the query picks among those read, as [`pick`] picks.
    pub fn pick(
        &self,
        all: bool,
        verb: &str,
        prompter: Option<&mut dyn Prompter>,
    ) -> Result<Vec<usize>> {
        pick(
            &self.views,
            &|n| self.cells(n),
            &self.query,
            all,
            verb,
            prompter,
        )
    }

    /// What song `n` shows when picked among others.
    #[must_use]
    pub fn cells(&self, n: usize) -> rows::Cells {
        let plan = self.resolved[n].as_ref().map(|r| &r.plan);
        rows::cells(&self.views[n], plan, &self.state.facts)
    }

    /// The songs at `picked`, as they were read.
    #[must_use]
    pub fn songs(&self, picked: &[usize]) -> Vec<Song> {
        picked
            .iter()
            .map(|n| self.manifest.songs[*n].clone())
            .collect()
    }

    /// Save the recorded edits unless a picked song changed meanwhile,
    /// as `run` records.
    pub fn save(&mut self, picked: &[usize], run: &mut crate::history::Run) -> Result<()> {
        let expected = self.songs(picked);
        let changed = self.manifest.save_if_unchanged(&expected, run)?;
        if !changed.is_empty() {
            self.manifest.discard();
            return refuse(format!(
                "the song list changed while this ran, for {}; nothing was changed, run it again",
                changed
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(())
    }
}

/// Every file in the store a key brings: what yt-dlp fetched under its
/// ID, or a manual or torrent's file with its own lyrics and pictures.
fn store_files(store: &Store, key: &SourceKey) -> Vec<PathBuf> {
    match key {
        SourceKey::Remote { site, id } if crate::store::kept(site).is_some() => {
            crate::store::kept(site).map_or_else(Vec::new, |k| store.record_files(k, id))
        }
        SourceKey::Remote { .. } => store.fetched_files(key),
        SourceKey::Manual(_) | SourceKey::Torrent(_) => {
            store.locate(key).map_or_else(Vec::new, |l| {
                std::iter::once(l.path.clone())
                    .chain(l.lyrics)
                    .chain(
                        l.covers
                            .into_iter()
                            .filter(|c| crate::store::named_as(c, &l.path)),
                    )
                    .collect()
            })
        }
    }
}

/// Remove the songs the query picks, and with `purge` their sources.
/// Returns whether anything was removed.
// One more than clippy allows: the runner and the settling the sync after hears and waits by.
#[expect(clippy::too_many_arguments)]
pub fn remove<R: crate::runner::Runner, W: Write>(
    runner: &R,
    (dirs, settling): (&Dirs, std::time::Duration),
    terms: &[String],
    purge: bool,
    confirm: &Confirm,
    mut prompter: Option<&mut dyn Prompter>,
    later: &mut crate::history::Later,
    out: &mut W,
) -> Result<bool> {
    let mut read = Read::new(dirs, terms)?;
    let picked = read.pick(confirm.all, "remove", reborrow(&mut prompter))?;
    if picked.is_empty() {
        crate::ui::info(out, "Nothing removed")?;
        return Ok(false);
    }
    let store = Store::scan(dirs)?;
    let mut doomed: Vec<(PathBuf, bool)> = Vec::new();
    // Each song's files of the user's own it trashes, noted under
    // `[[removed]]` so `restore` waits for every one of them.
    let mut trashed: Vec<Vec<SourceKey>> = Vec::new();
    let mut deleted = 0;
    let leaving = Leaving::of(runner, (dirs, settling), &read, &picked)?;
    for n in &picked {
        let mut own_trashed = Vec::new();
        let view = &read.views[*n];
        crate::ui::line(out, &format!("  − {}", named(view)))?;
        deleted += leaving.say(&read.state, view, out)?;
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
            // A removed song's sources stay, so that restore gets it whole.
            let held = |k: &SourceKey| read.manifest.removed.iter().any(|r| r.sources.contains(k));
            for key in view.keys.iter().filter(|k| !shared(k)) {
                let own = matches!(key, SourceKey::Manual(_));
                if held(key) {
                    for source in store_files(&store, key) {
                        crate::ui::line(
                            out,
                            &format!("    keeps {}, which a removed song lists", source.display()),
                        )?;
                    }
                    continue;
                }
                for source in store_files(&store, key) {
                    let how = if own { "trashes" } else { "deletes" };
                    crate::ui::line(out, &format!("    {how} {}", source.display()))?;
                    if own && let Ok(rel) = source.strip_prefix(dirs.manual()) {
                        own_trashed.push(SourceKey::Manual(rel.into()));
                    }
                    doomed.push((source, own));
                }
            }
        }
        trashed.push(own_trashed);
    }
    for file in &leaving.others {
        deleted += 1;
        crate::ui::line(
            out,
            &format!(
                "  and the sync after deletes {}, as `muman status` says",
                relpath::show(file)
            ),
        )?;
    }
    let own = doomed.iter().filter(|(_, own)| *own).count();
    let runs = read.manifest.settings.history.runs;
    let (said, ask) = removal(picked.len(), (deleted, doomed.len() - own, own), runs);
    crate::ui::info(out, &said)?;
    if !confirmed(confirm, &ask, prompter, out)? {
        return Ok(false);
    }
    for (n, files) in picked.iter().zip(trashed) {
        let view = &read.views[*n];
        let Some(id) = view.id() else {
            continue;
        };
        read.manifest.edit(Edit::Remove {
            key: id.clone(),
            note: view.note(),
        });
        read.manifest.edit(Edit::Trashed {
            key: id.clone(),
            files,
        });
    }
    let run = later.begin()?;
    for (file, _) in doomed.iter().filter(|(_, own)| *own) {
        run.trashed(&dirs.home, file);
    }
    read.save(&picked, run)?;
    purge_all(doomed)?;
    Ok(true)
}

/// What the sync after `remove` does with each library file of the songs
/// it removes, as it decides it ([`crate::reconcile::foresight`]), and the
/// files of other songs it removes too, as one `max_size` leaves out.
struct Leaving {
    fates: BTreeMap<PathBuf, Pruned>,
    others: Vec<PathBuf>,
}

impl Leaving {
    /// What the sync after the songs at `picked` leave `read`'s list
    /// decides of their files, from the grounds it gathers
    /// ([`crate::reconcile::gather`]), a moved source heard by `runner`: of
    /// the songs planned as the list is without them, as a folder's file
    /// the next song of its folder takes. A list whose settings the sync
    /// refuses removes nothing.
    fn of<R: crate::runner::Runner>(
        runner: &R,
        (dirs, settling): (&Dirs, std::time::Duration),
        read: &Read,
        picked: &[usize],
    ) -> Result<Self> {
        let paths: BTreeSet<PathBuf> = (picked.iter())
            .flat_map(|n| files_of(&read.state, &read.views[*n]))
            .collect();
        // As `read` loaded it, so a song is at the place it was picked by.
        let mut manifest = Manifest::load(&dirs.home)?;
        let listed = std::mem::take(&mut manifest.songs);
        manifest.songs = (listed.into_iter().enumerate())
            .filter(|(n, _)| !picked.contains(n))
            .map(|(_, song)| song)
            .collect();
        let scratch = crate::atomic::Scratch::new()?;
        let ground = crate::reconcile::gather(
            dirs,
            manifest,
            crate::reconcile::Gathering::forecast(&dirs.home, settling),
            crate::reconcile::hearing(runner, scratch.path()),
            &mut std::io::sink(),
        )?;
        let grounds = ground.grounds(&dirs.library, (false, false));
        let removals = match crate::reconcile::foresight(runner, &grounds) {
            Ok(foresight) => foresight.intent.removals().cloned().collect(),
            Err(e) if e.downcast_ref::<crate::errors::Refused>().is_some() => Vec::new(),
            Err(e) => return Err(e),
        };
        let (fates, rest): (Vec<_>, Vec<_>) =
            (removals.into_iter()).partition(|(path, _)| paths.contains(path));
        Ok(Self {
            fates: fates.into_iter().collect(),
            others: (rest.iter())
                .flat_map(|(_, fate)| fate.gone())
                .cloned()
                .collect(),
        })
    }

    /// Say what becomes of each file of `view`'s song; how many go.
    fn say<W: Write>(&self, state: &State, view: &View, out: &mut W) -> Result<usize> {
        let mut deleted = 0;
        for path in files_of(state, view) {
            let Some(fate) = self.fates.get(&path) else {
                continue;
            };
            let left: &[PathBuf] = match fate {
                Pruned::LeftChanged => std::slice::from_ref(&path),
                _ => fate.left(),
            };
            for file in fate.gone() {
                deleted += 1;
                crate::ui::line(out, &format!("    deletes {}", relpath::show(file)))?;
            }
            for file in left {
                crate::ui::line(
                    out,
                    &format!(
                        "    leaves in place, changed since muman wrote it, {}",
                        relpath::show(file)
                    ),
                )?;
            }
        }
        Ok(deleted)
    }
}

/// The library files the state records of `view`'s song.
fn files_of(state: &State, view: &View) -> Vec<PathBuf> {
    let id = view.id();
    (state.outputs.iter())
        .filter(|(_, w)| id.is_some_and(|id| w.sources.contains(id)))
        .map(|(p, _)| p.clone())
        .collect()
}

/// Move each file of `doomed` that is the user's own to the trash, and
/// delete the others, which muman fetched.
// Deletes a removed song's fetched source, with `remove --purge`.
#[expect(clippy::disallowed_methods)]
fn purge_all(doomed: Vec<(PathBuf, bool)>) -> Result<()> {
    for (source, own) in doomed {
        if own {
            crate::history::to_trash(&source)
                .with_context(|| format!("moving {} to the trash", source.display()))?;
        } else {
            match std::fs::remove_file(&source) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(e).with_context(|| format!("deleting {}", source.display()));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// What `muman undo` would do to the `n` things just named, `does` and
/// `after` around `it` or `them`, as `("puts", " back")`; or, with
/// `[history] runs = 0`, that no run is kept to undo it by.
pub(crate) fn undo_note(runs: usize, n: usize, (does, after): (&str, &str)) -> String {
    if runs == 0 {
        return "`[history] runs = 0` keeps no run, so this cannot be undone".to_string();
    }
    let them = if n == 1 { "it" } else { "them" };
    format!("`muman undo` {does} {them}{after}")
}

/// What removing `songs` does, with `deleted` library files and
/// `fetched` files of fetched sources deleted and `own` files of the
/// user's own trashed, `runs` kept to undo it by, and what to ask
/// before.
fn removal(
    songs: usize,
    (deleted, fetched, own): (usize, usize, usize),
    runs: usize,
) -> (String, Ask) {
    let named = counted(songs, "song");
    let leave = if songs == 1 { "leaves" } else { "leave" };
    let gone = counted(deleted + fetched, "file");
    if own > 0 {
        let trashed = counted(own, "file");
        let undo = if runs == 0 {
            undo_note(runs, songs, ("puts", " back"))
        } else {
            let them = if own == 1 { "it" } else { "them" };
            format!("`muman undo` waits until you put {them} back")
        };
        let said = format!(
            "{named} {leave} the list, {gone} deleted, {trashed} of yours to the trash. {undo}"
        );
        let question = format!("Type {songs} to remove {named} and trash {trashed} of yours:");
        return (
            said,
            Ask::Typed {
                count: songs,
                question,
            },
        );
    }
    let undo = if fetched > 0 {
        undo_note(
            runs,
            songs,
            ("lists", " again, and a sync fetches what was deleted"),
        )
    } else {
        undo_note(runs, songs, ("puts", " back"))
    };
    (
        format!("{named} {leave} the list, {gone} deleted. {undo}"),
        Ask::YesNo(format!("Remove {named}?")),
    )
}

/// List again the removed songs the query picks. Returns whether any
/// was.
pub fn restore<R: crate::runner::Runner, W: Write>(
    runner: &R,
    (dirs, settling): (&Dirs, std::time::Duration),
    terms: &[String],
    confirm: &Confirm,
    mut prompter: Option<&mut dyn Prompter>,
    later: &mut crate::history::Later,
    out: &mut W,
) -> Result<bool> {
    let mut manifest = Manifest::load(&dirs.home)?;
    let query = Query::parse(terms, &query::extractors(&manifest))?;
    let state = State::load(&dirs.home)?;
    let views: Vec<View> = manifest
        .removed
        .iter()
        .map(|r| query::removed_view(r, &state))
        .collect();
    query.check_fields(&views)?;
    let picked = pick(
        &views,
        &|n| rows::cells(&views[n], None, &state.facts),
        &query,
        confirm.all,
        "restore",
        reborrow(&mut prompter),
    )?;
    if picked.is_empty() {
        crate::ui::info(out, "Nothing restored")?;
        return Ok(false);
    }
    // A file of your own `remove --purge` trashed would leave its song
    // listed with nothing to make it from, and every sync failing.
    // A file moved within the folder is followed by the sync after.
    let store = Store::scan(dirs)?;
    let scratch = crate::atomic::Scratch::new()?;
    let hear = crate::reconcile::hearing(runner, scratch.path());
    let moves = crate::reconcile::found_moves(&manifest, &state, &store, settling, hear);
    let gone: std::collections::BTreeSet<String> = picked
        .iter()
        .flat_map(|n| {
            let removed = &manifest.removed[*n];
            crate::reconcile::not_back(
                &dirs.manual(),
                &store,
                &moves.moved,
                (&removed.sources, &removed.trashed),
            )
        })
        .filter_map(|k| match k {
            SourceKey::Manual(m) => Some(format!("  {}", dirs.manual().join(m.path()).display())),
            _ => None,
        })
        .collect();
    let gone: Vec<String> = gone.into_iter().collect();
    if !gone.is_empty() {
        return Err(Refused(format!(
            "These files of your own are gone, as `remove --purge` moves them to the \
             trash; put them back, then restore again:\n{}",
            gone.join("\n")
        ))
        .into());
    }
    for n in &picked {
        crate::ui::line(out, &format!("  + {}", named(&views[*n])))?;
    }
    let songs = counted(picked.len(), "song");
    let undo = undo_note(
        manifest.settings.history.runs,
        picked.len(),
        ("takes", " out again"),
    );
    crate::ui::info(
        out,
        &format!(
            "{songs} {} to the list. {undo}",
            if picked.len() == 1 {
                "comes back"
            } else {
                "come back"
            }
        ),
    )?;
    if !confirmed(
        confirm,
        &Ask::YesNo(format!("List {songs} again?")),
        prompter,
        out,
    )? {
        return Ok(false);
    }
    for id in picked.iter().filter_map(|n| views[*n].id()) {
        manifest.edit(Edit::Restore(id.clone()));
    }
    manifest.save(later.begin()?)?;
    Ok(true)
}

/// One `NAME=VALUE` or `NAME!` of `set`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Assign {
    Tag(String, Vec<String>),
    Pin(&'static str, Option<String>),
    Offset(Option<i64>),
    /// The MusicBrainz release the song is pinned to, or none.
    Release(Option<String>),
}

/// Why `name` is no tag `set` sets, if it is none: a field a query reads
/// of a song ([`crate::terms::DERIVED`]) that is no pin, or how the song
/// list names a song's parts.
pub(crate) fn no_tag(name: &str) -> Option<String> {
    let lower = name.trim().to_ascii_lowercase();
    let pin = ["audio", "cover", "lyrics"].contains(&lower.as_str());
    let derived = crate::terms::DERIVED.contains(&lower.as_str()) && !pin;
    (derived || ["sources", "id", "held"].contains(&lower.as_str())).then(|| {
        format!("{lower} is no tag; set a tag, audio, cover, lyrics, lyrics_offset or release")
    })
}

/// Refuse `term`, setting a lyrics offset of `ms`, when it is no song's.
fn refuse_offset(term: &str, ms: Option<i64>) -> Result<()> {
    match ms.and_then(crate::manifest::offset_refused) {
        Some(why) => refuse(format!("`{term}` {why}")),
        None => Ok(()),
    }
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
        if let Some(why) = no_tag(&lower) {
            return refuse(format!("`{term}`: {why}"));
        }
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
                refuse_offset(term, ms)?;
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
                refuse_offset(term, ms)?;
                other.push(Assign::Offset(ms));
            }
            "release" => {
                let id = match v.map(str::trim).filter(|v| !v.is_empty()) {
                    None => None,
                    Some(v) => match crate::tags::Kind::Mbid.read(v) {
                        Ok(id) => Some(id),
                        Err(_) => {
                            return refuse(format!("`{term}` is no MusicBrainz release ID"));
                        }
                    },
                };
                other.push(Assign::Release(id));
            }
            _ => {
                // Shown and stored as it takes effect; one that reads as
                // nothing sets nothing, so it clears as `NAME!` does.
                let v = match v.map(|v| crate::tags::by_hand(name, v)).transpose() {
                    Ok(v) => v,
                    Err(why) => return refuse(why),
                };
                match (
                    tags.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)),
                    v,
                ) {
                    (Some((_, values)), Some(v)) if !v.is_empty() => values.push(v),
                    (Some(_), Some(_)) => {}
                    (Some(_), None) => bail!("`{term}` both sets and clears {name}"),
                    (None, v) => tags.push((
                        name.to_string(),
                        v.filter(|v| !v.is_empty()).into_iter().collect(),
                    )),
                }
            }
        }
    }
    let mut assigns: Vec<Assign> = tags.into_iter().map(|(n, v)| Assign::Tag(n, v)).collect();
    assigns.extend(other);
    Ok((query, assigns))
}

impl Assign {
    /// The tag, pin or setting it changes.
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Tag(name, _) => name,
            Self::Pin(pin, _) => pin,
            Self::Offset(_) => "lyrics_offset",
            Self::Release(_) => "release",
        }
    }

    /// How it changes `view`'s song, in words.
    pub(crate) fn describe(&self, view: &View) -> String {
        match self {
            Self::Tag(name, values) => {
                let now = crate::tags::escaped(&view.values(name).join("; "));
                let now = if now.is_empty() {
                    "nothing".to_string()
                } else {
                    now
                };
                if values.is_empty() {
                    format!("{name}: {now} → what the sources offer")
                } else {
                    format!(
                        "{name}: {now} → {}",
                        crate::tags::escaped(&values.join("; "))
                    )
                }
            }
            Self::Pin(pin, Some(v)) if v == "false" => format!("{pin}: none taken"),
            Self::Pin(pin, Some(key)) => format!("{pin}: pinned to {key}"),
            Self::Pin(pin, None) => format!("{pin}: picked by measure"),
            Self::Offset(Some(ms)) if *ms < 0 => format!("lyrics earlier by {} ms", -ms),
            Self::Offset(Some(ms)) => format!("lyrics later by {ms} ms"),
            Self::Offset(None) => "lyrics offset: none".to_string(),
            Self::Release(Some(id)) => format!("release: pinned to {id}"),
            Self::Release(None) => "release: chosen with its album".to_string(),
        }
    }

    pub(crate) fn apply(&self, table: &mut Table, song: &Song) -> Result<()> {
        match self {
            Self::Tag(name, values) => manifest::set_tags(table, &[(name.clone(), values.clone())]),
            Self::Pin(pin, Some(v)) if v == "false" => {
                table.insert(pin, value(false));
            }
            Self::Pin(pin, Some(v)) => {
                let key = SourceKey::parse(v)?;
                if !song.has(&key) {
                    return Err(Refused(format!(
                        "{key} is not a source of the song listing {}",
                        song.id().map(ToString::to_string).unwrap_or_default()
                    ))
                    .into());
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
            Self::Release(Some(id)) => {
                table.insert("release", value(id.as_str()));
            }
            Self::Release(None) => {
                table.remove("release");
            }
        }
        Ok(())
    }
}

/// Whether `a` changes any song at `picked` as the song list holds it:
/// one already so, as a pin taken off a song pinned to nothing, is no
/// change.
pub(crate) fn changes(read: &Read, picked: &[usize], a: &Assign) -> Result<bool> {
    let ids: Vec<SourceKey> = picked
        .iter()
        .filter_map(|n| read.manifest.songs[*n].id().cloned())
        .collect();
    let mut tables = read.manifest.tables_by_id(&ids)?;
    for n in picked {
        let song = &read.manifest.songs[*n];
        let Some(table) = song.id().and_then(|id| tables.get_mut(id)) else {
            continue;
        };
        let before = table.to_string();
        a.apply(table, song)?;
        if table.to_string() != before {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The songs at `picked` with `assigns` applied, each said to `out` as
/// it changes, by the assignments that change it; a song already so is
/// left out.
pub(crate) fn rewrite<W: Write>(
    read: &Read,
    picked: &[usize],
    assigns: &[Assign],
    out: &mut W,
) -> Result<Vec<(SourceKey, Rewritten)>> {
    let ids: Vec<SourceKey> = picked
        .iter()
        .filter_map(|n| read.manifest.songs[*n].id().cloned())
        .collect();
    let mut tables = read.manifest.tables_by_id(&ids)?;
    let mut songs = Vec::new();
    for n in picked {
        let view = &read.views[*n];
        let song = &read.manifest.songs[*n];
        let id = song.id().context("a song lists no source")?.clone();
        let Some(mut table) = tables.remove(&id) else {
            bail!("{id} is no longer listed");
        };
        let mut said = Vec::new();
        for a in assigns {
            let before = table.to_string();
            a.apply(&mut table, song)?;
            if table.to_string() != before {
                said.push(a.describe(view));
            }
        }
        if said.is_empty() {
            continue;
        }
        crate::ui::line(out, &format!("  ~ {}", named(view)))?;
        for line in said {
            crate::ui::line(out, &format!("    {line}"))?;
        }
        songs.push((id, Rewritten::Table(SongTable(table))));
    }
    Ok(songs)
}

/// Say that `n` songs change, and what `undo` does, `runs` kept.
pub(crate) fn say_changed<W: Write>(n: usize, runs: usize, out: &mut W) -> Result<()> {
    let change = if n == 1 { "changes" } else { "change" };
    let undo = undo_note(runs, n, ("puts", " back"));
    crate::ui::info(out, &format!("{} {change}. {undo}", counted(n, "song")))?;
    Ok(())
}

/// Set what `terms` assign on the songs its query picks. Returns whether
/// anything was set.
pub fn set<W: Write>(
    dirs: &Dirs,
    terms: &[String],
    confirm: &Confirm,
    mut prompter: Option<&mut dyn Prompter>,
    later: &mut crate::history::Later,
    out: &mut W,
) -> Result<bool> {
    let (query_terms, assigns) = split_terms(terms).map_err(|e| Refused(format!("{e:#}")))?;
    if assigns.is_empty() {
        if let Some(p) = prompter
            && !query_terms.is_empty()
        {
            return crate::guided::edit(dirs, &query_terms, confirm, p, later, out);
        }
        return refuse(
            "Nothing to set: give NAME=VALUE or NAME!, or a query alone on a terminal to set \
             them step by step",
        );
    }
    if query_terms.is_empty() {
        return refuse("Name the songs to set with a query");
    }
    let mut read = Read::new(dirs, &query_terms)?;
    let picked = read.pick(confirm.all, "set", reborrow(&mut prompter))?;
    if picked.is_empty() {
        crate::ui::info(out, "Nothing set")?;
        return Ok(false);
    }
    let songs = rewrite(&read, &picked, &assigns, out)?;
    if songs.is_empty() {
        crate::ui::info(out, "Nothing changed: each song is so already")?;
        return Ok(false);
    }
    let changed = counted(songs.len(), "song");
    say_changed(songs.len(), read.manifest.settings.history.runs, out)?;
    let mut names: Vec<&str> = Vec::new();
    for a in &assigns {
        if !names.contains(&a.name()) {
            names.push(a.name());
        }
    }
    let ask = Ask::YesNo(format!("Set {} on {changed}?", spoken(&names)));
    if !confirmed(confirm, &ask, prompter, out)? {
        return Ok(false);
    }
    read.manifest.edit(Edit::Rewrite {
        songs,
        new: Vec::new(),
    });
    read.save(&picked, later.begin()?)?;
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
            "GENRE=French",
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
        for term in [
            "from=manual",
            "HAS=title",
            "key=x",
            "path=x",
            "format=opus",
            "id!",
        ] {
            let e = split_terms(&strings(&[term])).unwrap_err();
            assert!(e.downcast_ref::<Refused>().is_some(), "{term}: {e:#}");
            assert!(format!("{e:#}").contains("is no tag"), "{term}: {e:#}");
        }
        let id = "00000000-0000-4000-8000-00000000000A";
        let (_, assigns) = split_terms(&strings(&[&format!("release={id}")])).unwrap();
        assert_eq!(assigns, [Assign::Release(Some(id.to_ascii_lowercase()))]);
        let (_, assigns) = split_terms(&strings(&["release!"])).unwrap();
        assert_eq!(assigns, [Assign::Release(None)]);
        let e = split_terms(&strings(&["release=abc"])).unwrap_err();
        assert!(
            format!("{e:#}").contains("no MusicBrainz release ID"),
            "{e:#}"
        );
        assert!(split_terms(&strings(&["lyrics_offset_ms=soon"])).is_err());
        assert!(split_terms(&strings(&["genre=a", "genre!"])).is_err());
        let e = split_terms(&strings(&["format=mp3"])).unwrap_err();
        assert!(e.downcast_ref::<Refused>().is_some(), "{e:#}");
    }

    fn views(n: usize) -> Vec<View> {
        (0..n)
            .map(|i| View {
                keys: vec![SourceKey::youtube(&format!("{i:0>11}"))],
                ..View::default()
            })
            .collect()
    }

    /// Each of `views` shown as the copy of one song on its own album.
    fn copies(n: usize) -> rows::Cells {
        rows::Cells {
            name: "Tide — Ada Quill".into(),
            album: ["Lantern Weather", "Night Ferry Sessions", "Paper Comets"][n % 3].into(),
            short: format!("{n:0>11}"),
            ..rows::Cells::default()
        }
    }

    #[test]
    fn several_matches_need_all_or_a_terminal() {
        let q = Query::default();
        let pick = |views: &[View], all, p| pick(views, &copies, &q, all, "remove", p);
        assert_eq!(pick(&views(1), false, None).unwrap(), [0]);
        let e = pick(&views(2), false, None).unwrap_err();
        assert!(e.downcast_ref::<Refused>().is_some(), "{e:#}");
        assert!(format!("{e:#}").contains("Night Ferry Sessions"), "{e:#}");
        assert_eq!(pick(&views(2), true, None).unwrap(), [0, 1]);
        let mut p = crate::ui::MockPrompter::new();
        p.push_choice([1]);
        assert_eq!(pick(&views(3), false, Some(&mut p)).unwrap(), [1]);
        assert!(pick(&[], true, None).is_err());
    }

    #[test]
    fn songs_alike_are_picked_by_what_tells_them_apart_none_ticked() {
        let mut p = crate::ui::MockPrompter::new();
        p.push_choice([0]);
        let q = Query::default();
        pick(&views(2), &copies, &q, false, "remove", Some(&mut p)).unwrap();
        assert_eq!(
            p.asked,
            ["2 songs match, all Tide — Ada Quill; which to remove?\n  \
             Lantern Weather       00000000000\n  \
             Night Ferry Sessions  00000000001"]
        );
        assert_eq!(p.ticked, [Vec::<usize>::new()]);
    }

    #[test]
    fn a_change_without_a_terminal_needs_yes() {
        let mut out = Vec::new();
        let go = Ask::YesNo("Remove 1 song?".into());
        let ask = Confirm::default();
        assert!(confirmed(&ask, &go, None, &mut out).is_err());
        let yes = Confirm {
            yes: true,
            ..Confirm::default()
        };
        assert!(confirmed(&yes, &go, None, &mut out).unwrap());
        let dry = Confirm {
            yes: true,
            dry_run: true,
            ..Confirm::default()
        };
        assert!(!confirmed(&dry, &go, None, &mut out).unwrap());
    }

    #[test]
    fn trashing_files_of_the_user_s_own_takes_the_count_typed() {
        let mut out = Vec::new();
        let ask = Ask::Typed {
            count: 3,
            question: "Type 3 to remove 3 songs and trash 3 files of yours:".into(),
        };
        let mut p = crate::ui::MockPrompter::new();
        p.push_text(Some("2"))
            .push_text(Some(" 3 "))
            .push_text(None);
        let confirm = Confirm::default();
        assert!(!confirmed(&confirm, &ask, Some(&mut p), &mut out).unwrap());
        assert!(confirmed(&confirm, &ask, Some(&mut p), &mut out).unwrap());
        assert!(!confirmed(&confirm, &ask, Some(&mut p), &mut out).unwrap());
        let mut p = crate::ui::MockPrompter::answering([true]);
        let named = Ask::YesNo("Remove 3 songs?".into());
        assert!(confirmed(&confirm, &named, Some(&mut p), &mut out).unwrap());
        assert_eq!(p.asked, ["Remove 3 songs?"]);
    }

    #[test]
    fn a_list_reads_as_said() {
        assert_eq!(spoken(&["genre"]), "genre");
        assert_eq!(spoken(&["genre", "date", "audio"]), "genre, date and audio");
        assert_eq!(counted(1, "song"), "1 song");
        assert_eq!(counted(2, "song"), "2 songs");
    }
}
