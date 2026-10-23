//! What a dry run says a sync would do: each song's verdict and where
//! its aspects come from, how many songs it does each thing with, and
//! what it would remove.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use anyhow::Result;

use crate::dirs::Dirs;
use crate::limit;
use crate::manifest::Manifest;
use crate::resolve::{Plan, Resolved};
use crate::source::SourceKey;
use crate::state::{State, Written};
use crate::store::Store;

use super::decide::Doing;
use super::plan::{PlannedSong, name_of, path_of};
use super::prune::{Pruned, prune_plan};
use super::write::changes;

/// Say which songs were left out for want of room, and what the library
/// would need.
pub(super) fn left_out<W: Write>(
    manifest: &Manifest,
    state: &State,
    fitted: &limit::Fitted,
    out: &mut W,
) -> Result<()> {
    const NAMED: usize = 10;
    let names: Vec<String> = fitted
        .left_out
        .iter()
        .take(NAMED)
        .map(|n| name_of(&manifest.songs[*n], None, &state.facts))
        .collect();
    let more = fitted.left_out.len().saturating_sub(NAMED);
    crate::ui::error(
        out,
        &format!(
            "max_size {} cannot hold the library: at the lowest bitrates it takes {}; left out: {}{} (raise [library] max_size or lower [audio] min_bitrate)",
            crate::ui::bytes(fitted.max),
            crate::ui::bytes(fitted.floor.unwrap_or(0)),
            names.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        ),
    )?;
    Ok(())
}

/// Say how full the library is projected to be against its limit.
pub(super) fn fit_summary<W: Write>(fitted: &limit::Fitted, out: &mut W) -> Result<()> {
    crate::ui::info(
        out,
        &format!(
            "Library: {} of {} (max_size); {} song(s) below their best format",
            crate::ui::bytes(fitted.projected),
            crate::ui::bytes(fitted.max),
            fitted.lowered
        ),
    )?;
    Ok(())
}

/// Where each aspect of a song's plan comes from, and why.
pub(super) fn show_why<W: Write>(r: &Resolved, out: &mut W) -> Result<()> {
    writeln!(out, "  audio   {}: {}", r.plan.audio.key, r.why.audio)?;
    if let Some(why) = &r.why.fit {
        writeln!(out, "  format  {why}")?;
    }
    if let (Some(c), Some(why)) = (&r.plan.cover, &r.why.cover) {
        writeln!(out, "  cover   {}: {why}", c.key)?;
    }
    if let (Some(l), Some(why)) = (&r.plan.lyrics, &r.why.lyrics) {
        writeln!(out, "  lyrics  {}: {why}", l.key)?;
    }
    let width = r.plan.tags.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for ((key, values), why) in r.plan.tags.iter().zip(&r.why.tags) {
        let cleaned = if why.cleaned.is_empty() {
            String::new()
        } else {
            format!(", cleaned: {}", why.cleaned.join(", "))
        };
        writeln!(
            out,
            "  {key:<width$} {} ({}{cleaned})",
            values.join("; "),
            why.from
        )?;
    }
    Ok(())
}

/// What a dry run shows: the decisions a sync would act on.
pub(super) struct Shown<'a> {
    pub(super) manifest: &'a Manifest,
    pub(super) planned: &'a [(usize, Resolved)],
    /// What a sync would do with each of `planned`.
    pub(super) decided: &'a [Doing],
    /// Each song's file, as [`files_of`](super::decide::files_of) finds it.
    pub(super) files: &'a BTreeMap<usize, PathBuf>,
    pub(super) old: &'a BTreeMap<PathBuf, Written>,
    pub(super) state: &'a State,
    pub(super) failed: &'a BTreeSet<SourceKey>,
    /// What a sync would fetch before it plans.
    pub(super) to_fetch: &'a BTreeSet<SourceKey>,
}

/// The state as `decided`'s moves would leave it, which pruning is judged
/// by, saying each move.
pub(super) fn would_move<W: Write>(
    planned: &[PlannedSong],
    decided: &[Doing],
    old: &BTreeMap<PathBuf, Written>,
    showing: &Showing,
    out: &mut W,
) -> Result<BTreeMap<PathBuf, Written>> {
    let mut moved = old.clone();
    for ((n, r), d) in planned.iter().zip(decided) {
        if let Doing::Move { from, .. } = d
            && let Some(written) = moved.remove(from)
        {
            if showing.matches(*n) {
                crate::ui::info(
                    out,
                    &format!(
                        "Would move: {} → {}",
                        crate::relpath::show(from),
                        crate::relpath::show(&path_of(r))
                    ),
                )?;
            }
            moved.insert(path_of(r), written);
        }
    }
    Ok(moved)
}

/// Which songs `status` shows.
pub(super) struct Showing {
    /// The songs a query matched; every song without one.
    pub(super) matched: Option<BTreeSet<usize>>,
    /// Show the songs a sync leaves as they are, as a query does.
    pub(super) all: bool,
}

impl Showing {
    fn matches(&self, n: usize) -> bool {
        self.matched.as_ref().is_none_or(|m| m.contains(&n))
    }

    fn shows(&self, n: usize, fate: Fate) -> bool {
        self.matched
            .as_ref()
            .map_or(self.all || fate != Fate::Kept, |m| m.contains(&n))
    }
}

/// What a sync does with a song, as `status` counts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Fate {
    New,
    Changed,
    Again,
    Moved,
    Alone,
    Kept,
}

impl Fate {
    const ALL: [Self; 6] = [
        Self::New,
        Self::Changed,
        Self::Again,
        Self::Moved,
        Self::Alone,
        Self::Kept,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Changed => "changed",
            Self::Again => "written again",
            Self::Moved => "moved",
            Self::Alone => "left alone",
            Self::Kept => "up to date",
        }
    }
}

/// What `status` says a sync does with a song, its file made by `before`.
pub(crate) fn verdict(d: &Doing, before: Option<&Plan>, plan: &Plan) -> (Fate, String) {
    match d {
        Doing::Keep => (Fate::Kept, "up to date".to_string()),
        Doing::Move { retag: false, .. } => (Fate::Moved, "moved".to_string()),
        Doing::Move { retag: true, .. } => (Fate::Moved, "moved, changes: tags".to_string()),
        Doing::Guard { .. } => (
            Fate::Alone,
            "left alone, changed since muman wrote it".to_string(),
        ),
        Doing::Unowned => (Fate::Alone, "left alone, not muman's".to_string()),
        Doing::Blocked { .. } => (
            Fate::Alone,
            "left where it is, its new path holding a file not muman's".to_string(),
        ),
        Doing::Held { .. } => (
            Fate::Alone,
            "left as built, a source holding otherwise".to_string(),
        ),
        Doing::Retag | Doing::Write => match before {
            Some(p) if p == plan => (Fate::Again, "written again".to_string()),
            Some(p) => (
                Fate::Changed,
                format!("changes: {}", changes(p, plan).join(", ")),
            ),
            None => (Fate::New, "new".to_string()),
        },
    }
}

/// How many of the songs `showing` matches a sync does each thing with,
/// how many it plans once it has fetched their sources, and how many it
/// cannot plan; those up to date it hides it says how
/// to show.
pub(super) fn tally<W: Write>(
    fates: &BTreeMap<Fate, usize>,
    failed: usize,
    fetched: usize,
    showing: &Showing,
    out: &mut W,
) -> Result<()> {
    let mut parts: Vec<String> = Fate::ALL
        .iter()
        .filter_map(|f| Some(format!("{} {}", fates.get(f)?, f.name())))
        .collect();
    if fetched > 0 {
        parts.push(format!("{fetched} made once fetched"));
    }
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if parts.is_empty() {
        let said = if showing.matched.is_some() {
            "No song matches the query"
        } else {
            "No song listed yet; `muman add` adds one"
        };
        crate::ui::info(out, said)?;
        return Ok(());
    }
    let hidden = showing.matched.is_none() && !showing.all && fates.contains_key(&Fate::Kept);
    let hint = if hidden {
        "; `--all` or a query shows those up to date"
    } else {
        ""
    };
    crate::ui::info(out, &format!("Songs: {}{hint}", parts.join(", ")))?;
    Ok(())
}

/// Say what a sync would do, by the decisions it would make: each song
/// `showing` shows, its file and where each aspect comes from, then how
/// many songs it does each thing with. Without a query, then also what
/// it would remove and the files no song lists.
// The steps read in the order they run; split up, the order would be in the call sites.
#[expect(clippy::too_many_lines)]
pub(super) fn status<W: Write>(
    shown: &Shown<'_>,
    showing: &Showing,
    store: &Store,
    listed: &BTreeSet<SourceKey>,
    dirs: &Dirs,
    settling: Duration,
    out: &mut W,
) -> Result<()> {
    let Shown {
        manifest,
        planned,
        decided,
        files,
        old,
        state,
        failed,
        to_fetch,
    } = shown;
    let library = &dirs.library;
    let moved = would_move(planned, decided, old, showing, out)?;
    let mut after: BTreeMap<PathBuf, Written> = BTreeMap::new();
    let mut fates: BTreeMap<Fate, usize> = BTreeMap::new();
    for ((n, r), d) in planned.iter().zip(*decided) {
        let song = &manifest.songs[*n];
        let mut path = path_of(r);
        let before = moved
            .get(&path)
            .or_else(|| files.get(n).and_then(|f| old.get(f)))
            .and_then(|w| w.plan.as_ref());
        if let Doing::Guard { file } | Doing::Held { file } | Doing::Blocked { file } = d {
            path.clone_from(file);
        }
        let (fate, verdict) = verdict(d, before, &r.plan);
        if showing.matches(*n) {
            *fates.entry(fate).or_default() += 1;
        }
        if showing.shows(*n, fate) {
            writeln!(
                out,
                "{}",
                crate::ui::Style::Accent.paint(&name_of(song, Some(r), &state.facts))
            )?;
            writeln!(
                out,
                "  → {} ({verdict})",
                crate::ui::Style::Path.paint(&crate::relpath::show(&path))
            )?;
            show_why(r, out)?;
        }
        let written = match d {
            Doing::Keep | Doing::Guard { .. } | Doing::Held { .. } | Doing::Blocked { .. } => {
                moved.get(&path).cloned()
            }
            Doing::Unowned => None,
            _ => Some(Written {
                sources: song.sources.clone(),
                lyrics: r.plan.lyrics.as_ref().map(|_| path.with_extension("lrc")),
                plan: Some(r.plan.clone()),
                stamp: None,
                digest: None,
            }),
        };
        if let Some(written) = written {
            after.insert(path, written);
        }
    }
    let planned_songs: BTreeSet<usize> = planned.iter().map(|(n, _)| *n).collect();
    let (fetched, unplanned): (Vec<usize>, Vec<usize>) = (0..manifest.songs.len())
        .filter(|n| showing.matches(*n) && !planned_songs.contains(n))
        .partition(|n| {
            manifest.songs[*n]
                .sources
                .iter()
                .any(|k| to_fetch.contains(k))
        });
    tally(&fates, unplanned.len(), fetched.len(), showing, out)?;
    // What a query matches is songs; these are files.
    if showing.matched.is_some() {
        return Ok(());
    }
    for (path, fate) in prune_plan(library, &moved, &after, failed, false) {
        match fate {
            Pruned::Removed(_) => crate::ui::warning(
                out,
                &format!("Would remove: {}", crate::relpath::show(&path)),
            )?,
            Pruned::LeftChanged => crate::ui::warning(
                out,
                &format!(
                    "Would leave in place, changed since muman wrote it: {}",
                    crate::relpath::show(&path)
                ),
            )?,
            Pruned::KeptForFailed => {}
        }
    }
    for twin in store.twins() {
        crate::ui::warning(
            out,
            &format!(
                "Not read, as another file in the manual folder has its name once \
                 normalized: {}; rename one of them",
                crate::relpath::show(twin)
            ),
        )?;
    }
    let mut known = listed.clone();
    known.extend(manifest.removed_keys());
    let (mut unlisted, mut settling) = store.unlisted(&known, SystemTime::now(), settling);
    settling.retain(|path| {
        let key = SourceKey::Manual(path.into());
        let was_listed = state.listed.contains(&key);
        if was_listed {
            unlisted.push(key);
        }
        !was_listed
    });
    for key in unlisted {
        let line = if state.listed.contains(&key) {
            format!("Taken out of the song list by hand, kept out by the next sync: {key}")
        } else {
            format!("Not listed yet, added by the next sync: {key}")
        };
        crate::ui::info(out, &line)?;
    }
    for path in settling {
        crate::ui::info(
            out,
            &format!(
                "Still being copied in, left for a later run: {}",
                SourceKey::Manual(path.into())
            ),
        )?;
    }
    // A manual file not listed is added by the next sync; only fetched
    // and kept files can lie unused.
    for path in store
        .unused(listed)
        .iter()
        .filter(|p| !p.starts_with(dirs.manual()))
    {
        crate::ui::info(out, &format!("Unused: {}", path.display()))?;
    }
    Ok(())
}
