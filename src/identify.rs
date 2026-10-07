//! Which song new sources belong to, by their fingerprints: a source the
//! same recording as a listed song joins it, one that may be is asked
//! about, and any other is a song of its own.
//!
//! A new source is compared with every listed song's sources, and its
//! strongest match decides: a surer verdict first, then the larger share
//! (`fingerprint`). One that may be the song, an excerpt, a video with a
//! long skit, or another master or mix, is asked about on a terminal;
//! elsewhere it is kept as a song of its own with a warning rather than
//! joined unasked, since a song made of two recordings writes only one.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::io::Write;

use crate::ui::Prompter;
use anyhow::Result;

use crate::acquire::Proposal;
use crate::dirs::Dirs;
use crate::facts::Facts;
use crate::fingerprint::{Indexed, Match, Verdict};
use crate::manifest::{Edit, Manifest};
use crate::parallel;
use crate::reconcile;
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::State;
use crate::store::Store;
use crate::tags::Field;

/// What to do with a source that may be a listed song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Ask, when there is a terminal to ask at; else keep it apart.
    Ask,
    /// Add it to the song.
    Yes,
    /// Never match: every new source is a song of its own.
    New,
}

/// A song new sources may join: what to call it, and its sources.
struct Known {
    name: String,
    keys: Vec<SourceKey>,
}

fn name_from(facts: Option<&Facts>, fallback: &str) -> String {
    let offer = |f: Field| {
        facts
            .and_then(|x| x.tags.get(&f))
            .and_then(|o| o.values.first())
            .cloned()
    };
    match (offer(Field::Title), offer(Field::Artist)) {
        (Some(t), Some(a)) => format!("{t} — {a}"),
        (Some(t), None) => t,
        _ => fallback.to_string(),
    }
}

/// The edits that list each proposal: under the song it is the same
/// recording as, or as a song of its own. Measures what it compares, and
/// keeps the measures for the reconcile that follows.
#[allow(clippy::too_many_lines)]
pub fn identify<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    proposals: Vec<Proposal>,
    mode: Mode,
    verbose: bool,
    mut prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<Vec<Edit>> {
    if proposals.is_empty() {
        return Ok(Vec::new());
    }
    let manifest = Manifest::load(&dirs.home)?;
    let listed = manifest.keys();
    let mut state = State::load(&dirs.home)?;
    if mode != Mode::New {
        let store = Store::scan(dirs)?;
        let wanted: BTreeSet<SourceKey> = listed
            .iter()
            .cloned()
            .chain(proposals.iter().flat_map(|p| p.sources.iter().cloned()))
            .collect();
        let temp = crate::atomic::Scratch::new()?;
        let home = &dirs.home;
        let mut how = reconcile::Measuring {
            retry: false,
            say_unread: false,
            checkpoint: &mut |s: &State| State::keep_measures(home, s),
        };
        reconcile::measure(
            runner,
            &store,
            &wanted,
            &mut state,
            temp.path(),
            &mut how,
            out,
        )?;
    }
    let print = |k: &SourceKey| state.facts.get(k).and_then(|f| f.print.as_ref());
    let mut known: Vec<Known> = manifest
        .songs
        .iter()
        .filter(|s| !s.sources.is_empty())
        .map(|s| Known {
            name: name_from(state.facts.get(&s.sources[0]), &s.sources[0].to_string()),
            keys: s.sources.clone(),
        })
        .collect();

    let mut edits = Vec::new();
    for p in proposals {
        if mode == Mode::New || p.sources.iter().any(|k| listed.contains(k)) {
            edits.push(Edit::Add {
                sources: p.sources,
                album: p.album,
            });
            continue;
        }
        let Some(mine) = p.sources.iter().find_map(&print) else {
            known.push(Known {
                name: p.label.clone(),
                keys: p.sources.clone(),
            });
            edits.push(Edit::Add {
                sources: p.sources,
                album: p.album,
            });
            continue;
        };
        let mine = Indexed::new(mine);
        let closest = parallel::map(&known, parallel::builds(), |song| {
            song.keys
                .iter()
                .filter_map(|k| mine.compare(print(k)?))
                .max_by(stronger)
        });
        let mut ranked: Vec<(usize, Match)> = closest
            .into_iter()
            .enumerate()
            .filter_map(|(n, m)| Some((n, m?)))
            .collect();
        ranked.sort_by(|a, b| stronger(&b.1, &a.1));
        if verbose {
            for (n, m) in ranked.iter().take(3) {
                crate::ui::trace(
                    out,
                    &format!(
                        "{} against {}: {}, spanning {:.0}% of the longer: {:?}",
                        p.label,
                        known[*n].name,
                        m.describe(),
                        m.coverage * 100.0,
                        m.verdict()
                    ),
                )?;
            }
        }
        let best = ranked.first().copied();
        let join = match best {
            Some((n, m)) => {
                let song = &known[n].name;
                match m.verdict() {
                    Verdict::Same => {
                        crate::ui::info(
                            out,
                            &format!(
                                "{}: the same recording as {song} ({}); adding it to that song",
                                p.label,
                                m.describe()
                            ),
                        )?;
                        Some(n)
                    }
                    Verdict::Unsure if mode == Mode::Yes => {
                        crate::ui::info(
                            out,
                            &format!(
                                "{}: likely {song} ({}); adding it to that song",
                                p.label,
                                m.describe()
                            ),
                        )?;
                        Some(n)
                    }
                    Verdict::Unsure => {
                        if let Some(ask) = prompter.as_deref_mut() {
                            ask.ask(&format!(
                                "{} sounds like {song} ({}). Add it as a source of that song?",
                                p.label,
                                m.describe()
                            ))?
                            .then_some(n)
                        } else {
                            {
                                crate::ui::warning(
                                    out,
                                    &format!(
                                        "{} may be {song} ({}); kept as a song of its own. \
                                     To merge them, move its sources into that song's, or add it again with -y",
                                        p.label,
                                        m.describe()
                                    ),
                                )?;
                                None
                            }
                        }
                    }
                    Verdict::Different => None,
                }
            }
            None => None,
        };
        if let Some(n) = join {
            let mut sources: Vec<SourceKey> = crate::manifest::id_of(&known[n].keys)
                .cloned()
                .into_iter()
                .collect();
            sources.extend(p.sources.iter().cloned());
            known[n].keys.extend(p.sources);
            edits.push(Edit::Add {
                sources,
                album: p.album,
            });
        } else {
            known.push(Known {
                name: p.label.clone(),
                keys: p.sources.clone(),
            });
            edits.push(Edit::Add {
                sources: p.sources,
                album: p.album,
            });
        }
    }
    Ok(edits)
}

/// Which of two matches is stronger: a surer verdict, then more alike.
fn stronger(a: &Match, b: &Match) -> Ordering {
    let rank = |v: Verdict| match v {
        Verdict::Same => 2,
        Verdict::Unsure => 1,
        Verdict::Different => 0,
    };
    rank(a.verdict())
        .cmp(&rank(b.verdict()))
        .then(a.share.total_cmp(&b.share))
}

#[cfg(test)]
mod tests;
