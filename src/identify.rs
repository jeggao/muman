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
//!
//! The question shows the new source beside the listed song it may be:
//! the name, album, length and audio of each, and how alike they sound.
//! A difference in length often settles it, as a live take or a video
//! with a skit runs longer, and the album says which of two songs of
//! one name it is. Keeping it apart comes first, as it is what happens
//! without a terminal, and the answers are named by what they do.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::io::Write;

use crate::ui::Prompter;
use anyhow::Result;
use unicode_width::UnicodeWidthStr;

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

/// A song new sources may join: what to call it, its album and place
/// in it as the card shows them, and its sources.
struct Known {
    name: String,
    album: String,
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
// The verdicts read in one place, as the guide's table lists them.
#[expect(clippy::too_many_lines)]
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
            analyses: crate::analysis::wanted(&manifest.settings.loudness),
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
            album: album_of(s, &manifest, state.facts.get(&s.sources[0])),
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
                album: String::new(),
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
                .filter_map(|k| Some((k, mine.compare(print(k)?)?)))
                .max_by(|a, b| stronger(&a.1, &b.1))
                .map(|(k, m)| (k.clone(), m))
        });
        let matched: Vec<Option<SourceKey>> = closest
            .iter()
            .map(|c| c.as_ref().map(|(k, _)| k.clone()))
            .collect();
        let mut ranked: Vec<(usize, Match)> = closest
            .into_iter()
            .enumerate()
            .filter_map(|(n, m)| Some((n, m?.1)))
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
                            let ours = p.sources.iter().find(|k| print(k).is_some());
                            let theirs = matched[n].as_ref().and_then(|k| state.facts.get(k));
                            let new = ours.and_then(|k| state.facts.get(k));
                            for line in card(&p.label, new, &known[n], theirs, &m) {
                                crate::ui::line(out, &line)?;
                            }
                            let items = ["Its own song".to_string(), format!("Add to {song}")];
                            let question = format!("Where does {} go?", p.label);
                            (ask.select(&question, &items)? == Some(1)).then_some(n)
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
                album: String::new(),
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

/// The album `song` is on as the card names it, with its place there.
fn album_of(song: &crate::manifest::Song, manifest: &Manifest, facts: Option<&Facts>) -> String {
    let named = |tags: &crate::manifest::Tags| {
        tags.iter()
            .find(|(k, _)| crate::tags::vorbis_key(k) == Field::Album.vorbis())
            .and_then(|(_, v)| v.first().cloned())
    };
    let album = named(&song.tags)
        .or_else(|| {
            let key = song.album.as_ref()?;
            named(&manifest.album(key)?.tags)
        })
        .or_else(|| facts?.tags.get(&Field::Album)?.values.first().cloned())
        .unwrap_or_default();
    match (album.is_empty(), song.track) {
        (false, Some(t)) => format!("{album} · #{t}"),
        (true, Some(t)) => format!("#{t}"),
        _ => album,
    }
}

/// The lines that show a new source, `label`, beside the listed `song`
/// it may be: `new` and `theirs` the facts of each, `m` how alike.
fn card(
    label: &str,
    new: Option<&Facts>,
    song: &Known,
    theirs: Option<&Facts>,
    m: &Match,
) -> Vec<String> {
    let shown = |s: Option<f64>| {
        s.map(|s| crate::rows::minutes((s * 1000.0).round() as i64))
            .unwrap_or_default()
    };
    let audio = |f: Option<&Facts>| {
        f.and_then(|f| Some(crate::rows::audio(f.audio.as_ref()?, f.length())))
            .unwrap_or_default()
    };
    let (ours, other) = (new.and_then(Facts::length), theirs.and_then(Facts::length));
    let mut length = shown(ours);
    if let (Some(a), Some(b)) = (ours, other) {
        let d = (a - b).round() as i64;
        if d != 0 {
            let sign = if d > 0 { "+" } else { "−" };
            length = format!("{length}  {sign}{}", crate::rows::minutes(d.abs() * 1000));
        }
    }
    let offered = new
        .and_then(|f| f.tags.get(&Field::Album)?.values.first().cloned())
        .unwrap_or_default();
    let table = [
        ("", "new source".to_string(), "listed song".to_string()),
        ("name", name_from(new, label), song.name.clone()),
        ("album", offered, song.album.clone()),
        ("length", length, shown(other)),
        ("audio", audio(new), audio(theirs)),
    ];
    let wide = table.iter().map(|(_, a, _)| a.width()).max().unwrap_or(0);
    let mut lines = vec![format!("{label} may be a song already listed")];
    for (what, a, b) in table {
        let pad = " ".repeat(wide - a.width());
        lines.push(format!("  {what:<7}  {a}{pad}  {b}").trim_end().to_string());
    }
    lines.push(format!(
        "  {}: an excerpt, a live take or another mix",
        m.describe()
    ));
    lines.push(
        "  Added to the song, it is one more source; the sync picks the best audio by measure"
            .to_string(),
    );
    lines
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
