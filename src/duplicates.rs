//! Songs listed apart that are one recording, by the prints of the audio
//! each is written from: the copy of a track a compilation repeats, or
//! one file of an album ripped twice.
//!
//! Every pair of songs is compared as `identify` compares a new source
//! with a song, and a pair called the same recording links the two;
//! linked songs make a group. A pair is compared only when the shorter
//! print is at least `RATIO` of the longer: the same recording spans
//! at least that share of the longer print, and the overlap of two prints
//! is no longer than the shorter, so no pair left out could be one. A
//! real library of 1,546 songs took 8 s, and found 12 groups on one album
//! and 144 across albums.
//!
//! A group with an album holding it twice is most likely a file there
//! twice, whatever other albums hold it too. An album is told by its
//! album artist with its name, so two albums of one name by two artists
//! are two, and a song on none is on no album. A group on several albums,
//! once on each, is a song and its copies elsewhere, which a library of
//! whole albums keeps.
//!
//! Listing them changes nothing. `--resolve` steps through the groups on
//! a terminal, those an album holds twice first, each shown side by side
//! with the song `merge` would keep; merging is suggested for a file
//! there twice, keeping apart for copies on other albums, which are
//! offered only once those are done. What is chosen is said, asked
//! about, and made as one change, so one `undo` puts it back.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use anyhow::{Context, Result, bail};
use unicode_width::UnicodeWidthStr;

use crate::change::{self, Ask, Refused};
use crate::cli::Confirm;
use crate::dirs::Dirs;
use crate::fingerprint::{Indexed, Match, Verdict};
use crate::manifest::{Edit, Manifest, Song, Tags};
use crate::parallel;
use crate::query::{self, Query, View};
use crate::reconcile;
use crate::rows;
use crate::runner::Runner;
use crate::state::State;
use crate::store::Store;
use crate::ui::Prompter;

mod merge;

/// The least share of the longer print the shorter is, for two to be
/// compared: the coverage the same recording needs (`fingerprint`).
const RATIO: f64 = 0.8;

/// What finding the groups read and found.
struct Found {
    manifest: Manifest,
    state: State,
    views: Vec<View>,
    /// The groups the query names: those an album holds twice, then
    /// those across albums.
    twice: Vec<Vec<usize>>,
    across: Vec<Vec<usize>>,
    /// How many groups there are, the query aside.
    all: usize,
    /// How alike each pair called one recording is, by their places in
    /// the song list, the lower first.
    alike: BTreeMap<(usize, usize), Match>,
}

/// The groups of songs the query matches any of that are one recording,
/// saying what is left unknown to `out`.
fn find<W: Write>(dirs: &Dirs, terms: &[String], out: &mut W) -> Result<Found> {
    let manifest = Manifest::load(&dirs.home)?;
    let state = State::load(&dirs.home)?;
    let (views, resolved): (Vec<_>, Vec<_>) =
        query::resolved_views(&manifest, &state, &dirs.library)?
            .into_iter()
            .unzip();
    let query = Query::parse(terms, &query::extractors(&manifest))?;
    query.check_fields(&views)?;
    let planned: Vec<(usize, &crate::resolve::Resolved)> = resolved
        .iter()
        .enumerate()
        .filter_map(|(n, r)| Some((n, r.as_ref()?)))
        .collect();
    let (mut printed, mut short, mut still) = (Vec::new(), 0_usize, 0_usize);
    for (n, r) in &planned {
        match state.facts.get(&r.plan.audio.key).map(|f| f.print.as_ref()) {
            Some(Some(print)) => {
                let indexed = Indexed::new(print);
                if indexed.is_empty() {
                    still += 1;
                } else {
                    printed.push((*n, indexed));
                }
            }
            Some(None) => short += 1,
            None => {}
        }
    }
    let unmade = manifest.songs.len() - planned.len();
    for (count, why) in [
        (
            unmade,
            "cannot be made, so have no print; `status` says why",
        ),
        (short, "are too short to print, 2 s or less"),
        (
            still,
            "hold only silence or held notes, which any two songs share",
        ),
    ] {
        if count > 0 {
            crate::ui::info(
                out,
                &format!("{count} song(s) {why}; they are compared with none"),
            )?;
        }
    }
    printed.sort_by_key(|(_, p)| p.len());
    let step = crate::progress::step("Comparing prints", Some(printed.len() as u64));
    let pairs = parallel::map(
        &(0..printed.len()).collect::<Vec<_>>(),
        parallel::builds(),
        |&i| {
            let (a_at, a) = &printed[i];
            let linked = printed[i + 1..]
                .iter()
                .take_while(|(_, b)| ratio(a.len(), b.len()) >= RATIO)
                .filter_map(|(b_at, b)| {
                    let m = a.compare_indexed(b)?;
                    (m.verdict() == Verdict::Same).then_some((
                        (*a_at).min(*b_at),
                        (*a_at).max(*b_at),
                        m,
                    ))
                })
                .collect::<Vec<_>>();
            step.advance(1);
            linked
        },
    );
    // Done before the report, so a plain progress line does not follow it.
    drop(step);
    let alike: BTreeMap<(usize, usize), Match> = pairs
        .into_iter()
        .flatten()
        .map(|(a, b, m)| ((a, b), m))
        .collect();
    let groups = groups(manifest.songs.len(), alike.keys().copied());
    let all = groups.len();
    let ids = |g: &[usize]| g.iter().map(|n| album_id(&views[*n])).collect::<Vec<_>>();
    let (twice, across): (Vec<Vec<usize>>, Vec<Vec<usize>>) = groups
        .into_iter()
        .filter(|g| g.iter().any(|n| query.matches(&views[*n])))
        .partition(|g| twice(&ids(g)).is_some());
    Ok(Found {
        manifest,
        state,
        views,
        twice,
        across,
        all,
        alike,
    })
}

/// Say each group of songs the query matches any of that are one
/// recording to `report`, and what is left unknown to `out`.
pub fn report<W: Write, D: Write>(
    dirs: &Dirs,
    terms: &[String],
    out: &mut W,
    report: &mut D,
) -> Result<()> {
    let found = find(dirs, terms, out)?;
    let views = &found.views;
    let album = |n: usize| views[n].first("album");
    for group in found.twice.iter().chain(&found.across) {
        let ids: Vec<_> = group.iter().map(|n| album_id(&views[*n])).collect();
        let albums = ids.iter().collect::<BTreeSet<_>>().len();
        let on = match (albums, twice(&ids)) {
            (1, _) => format!("on one album, {}", album(group[0])),
            (n, Some(at)) => format!("on {n} albums, twice on {}", album(group[at])),
            (n, None) => format!("on {n} albums"),
        };
        crate::ui::line(
            report,
            &format!("{} songs, one recording, {on}:", group.len()),
        )?;
        for n in group {
            let view = &views[*n];
            let key = view.id().map(ToString::to_string).unwrap_or_default();
            crate::ui::line(report, &format!("  {key}\t{}\t{}", view.name(), album(*n)))?;
        }
    }
    summary(report, (found.twice.len(), found.across.len()), found.all)
}

/// What the user chose for one group.
enum Choice {
    /// `others` merged into `keeper`, `winners` the tags set apart.
    Merge {
        keeper: usize,
        others: Vec<usize>,
        winners: Tags,
    },
    Remove(usize),
}

/// What a group's question offers, each with what it does.
enum Answer {
    Merge(Vec<usize>),
    Apart,
    RemoveOne,
    Later,
    Stop,
}

/// Step through each group the query names on a terminal, merging its
/// songs into one, keeping them apart or removing one, and make what was
/// chosen as one change. Returns whether anything changed.
pub fn resolve<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    terms: &[String],
    confirm: &Confirm,
    mut prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    if prompter.is_none() {
        return Err(Refused(
            "Resolving duplicates asks on a terminal; `muman duplicates` lists them".into(),
        )
        .into());
    }
    let mut found = find(dirs, terms, out)?;
    if found.twice.is_empty() && found.across.is_empty() {
        summary(out, (0, 0), found.all)?;
        return Ok(false);
    }
    let groups: Vec<(bool, Vec<usize>)> = (found.twice.iter().map(|g| (true, g.clone())))
        .chain(found.across.iter().map(|g| (false, g.clone())))
        .collect();
    let together: Vec<Song> = groups
        .iter()
        .map(|(_, g)| merge::together(&found.manifest, g))
        .collect();
    let store = Store::scan(dirs)?;
    if reconcile::compare(runner, &store, &together, &mut found.state, out)? {
        State::keep_measures(&dirs.home, &found.state)?;
    }
    let m = &found.manifest;
    let resolver = reconcile::Resolver::new(m, &m.settings, &found.state, &dirs.library)?;
    let mut chosen = Vec::new();
    for (i, (twice, group)) in groups.iter().enumerate() {
        let Some(p) = change::reborrow(&mut prompter) else {
            break;
        };
        if !twice && i > 0 && groups[i - 1].0 {
            let more = change::counted(found.across.len(), "group");
            if !p.ask(&format!(
                "Go on to {more} across albums, which a library of whole albums keeps?"
            ))? {
                break;
            }
        }
        let asking = Asking {
            found: &found,
            resolver: &resolver,
            at: (i + 1, groups.len()),
            twice: *twice,
            group,
        };
        match asking.ask(p, out)? {
            Step::Chose(choice) => chosen.push(choice),
            Step::Skip => {}
            Step::Stop => break,
        }
    }
    apply(&mut found, &chosen, *confirm, prompter, out)
}

/// Where a group's question leaves the steps.
enum Step {
    Chose(Choice),
    Skip,
    Stop,
}

/// One group to ask about, with what showing it reads.
struct Asking<'a> {
    found: &'a Found,
    resolver: &'a reconcile::Resolver<'a>,
    /// Its place among the groups, and how many there are.
    at: (usize, usize),
    twice: bool,
    group: &'a [usize],
}

impl Asking<'_> {
    /// Show the group side by side and ask what becomes of it.
    fn ask<W: Write>(&self, p: &mut dyn Prompter, out: &mut W) -> Result<Step> {
        let (found, m) = (self.found, &self.found.manifest);
        let ids: Vec<_> = (self.group.iter())
            .map(|n| album_id(&found.views[*n]))
            .collect();
        let twins: Vec<usize> = (self.group.iter().zip(&ids))
            .filter(|(_, id)| id.is_some() && ids.iter().filter(|o| o == id).count() > 1)
            .map(|(n, _)| *n)
            .collect();
        let keeper = merge::keeper(
            m,
            self.resolver,
            if self.twice { &twins } else { self.group },
        );
        let shown: Vec<usize> = std::iter::once(keeper)
            .chain(self.group.iter().copied().filter(|n| *n != keeper))
            .collect();
        let letter = |n: usize| letter(shown.iter().position(|s| *s == n).unwrap_or(0));
        let letters = |ns: &[usize]| ns.iter().map(|n| letter(*n)).collect::<Vec<_>>();
        let on = if self.twice {
            format!("twice on {}", found.views[twins[0]].first("album"))
        } else {
            "across albums".to_string()
        };
        let (at, of) = self.at;
        crate::ui::info(out, &format!("Group {at} of {of} · {on} · one recording"))?;
        let cells: Vec<rows::Cells> = (shown.iter())
            .map(|n| {
                let plan = self.resolver.resolve(&m.songs[*n]).ok().map(|r| r.plan);
                rows::cells(&found.views[*n], plan.as_ref(), &found.state.facts)
            })
            .collect();
        for line in table(&shown, &cells, keeper, m, &found.alike) {
            crate::ui::line(out, &line)?;
        }
        let rest = shown[1..].to_vec();
        let k = letter(keeper);
        let suggested: Vec<usize> = twins.iter().copied().filter(|n| *n != keeper).collect();
        let mut offered: Vec<(String, Answer)> = Vec::new();
        if self.twice {
            let them = change::spoken_owned(&letters(&suggested));
            offered.push((
                format!("Merge {them} into {k} (suggested: an album holds it twice)"),
                Answer::Merge(suggested.clone()),
            ));
        } else {
            offered.push((
                "Keep apart (suggested: each on an album of its own)".into(),
                Answer::Apart,
            ));
        }
        if !self.twice || suggested.len() < rest.len() {
            offered.push((format!("Merge all into {k}"), Answer::Merge(rest)));
        }
        if self.twice {
            offered.push(("Keep apart".into(), Answer::Apart));
        }
        offered.push(("Remove one…".into(), Answer::RemoveOne));
        offered.push(("Decide later".into(), Answer::Later));
        offered.push(("Stop and review what I picked".into(), Answer::Stop));
        let items: Vec<String> = offered.iter().map(|(s, _)| s.clone()).collect();
        let answer = (p.select(&format!("Group {at}:"), &items)?)
            .and_then(|i| offered.into_iter().nth(i))
            .map_or(Answer::Later, |(_, a)| a);
        Ok(match answer {
            Answer::Merge(others) => {
                let mut winners: Tags = Vec::new();
                let merged: Vec<usize> = std::iter::once(keeper)
                    .chain(others.iter().copied())
                    .collect();
                for clash in merge::clashes(m, &merged) {
                    let items: Vec<String> = (clash.set.iter())
                        .map(|(v, by)| format!("{}  ({})", v.join("; "), letters(by).join(", ")))
                        .collect();
                    let question = format!("{} is set apart; which stays?", clash.tag);
                    let i = p.select(&question, &items)?.unwrap_or(0);
                    winners.push((clash.tag, clash.set[i.min(clash.set.len() - 1)].0.clone()));
                }
                Step::Chose(Choice::Merge {
                    keeper,
                    others,
                    winners,
                })
            }
            Answer::RemoveOne => {
                let items: Vec<String> = (shown.iter())
                    .map(|n| format!("{}  {}", letter(*n), change::named(&found.views[*n])))
                    .collect();
                match p.select("Remove which?", &items)? {
                    Some(i) => Step::Chose(Choice::Remove(shown[i])),
                    None => Step::Skip,
                }
            }
            Answer::Apart | Answer::Later => Step::Skip,
            Answer::Stop => Step::Stop,
        })
    }
}

/// The letter a group's song at `at` goes by, from A.
fn letter(at: usize) -> String {
    char::from(b'A' + u8::try_from(at % 26).unwrap_or(0)).to_string()
}

/// A group's songs side by side, one column each, `keeper` first.
fn table(
    shown: &[usize],
    cells: &[rows::Cells],
    keeper: usize,
    m: &Manifest,
    alike: &BTreeMap<(usize, usize), Match>,
) -> Vec<String> {
    let tags = |n: usize| {
        let set: Vec<String> = m.songs[n]
            .tags
            .iter()
            .map(|(k, v)| format!("{k}={}", v.join("; ")))
            .collect();
        if set.is_empty() {
            "none".to_string()
        } else {
            set.join(", ")
        }
    };
    let close = |n: usize| {
        if n == keeper {
            return "—".to_string();
        }
        alike
            .get(&(n.min(keeper), n.max(keeper)))
            .map_or_else(|| "through another".to_string(), Match::describe)
    };
    let head: Vec<String> = (0..shown.len()).map(letter).collect();
    let row = |f: &dyn Fn(usize, &rows::Cells) -> String| -> Vec<String> {
        shown.iter().zip(cells).map(|(n, c)| f(*n, c)).collect()
    };
    let lines: Vec<(&str, Vec<String>)> = vec![
        ("", head),
        ("key", row(&|_, c| c.key.clone())),
        (
            "album",
            row(&|_, c| {
                [c.album.as_str(), c.track.as_str()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" · ")
            }),
        ),
        ("length", row(&|_, c| c.length.clone())),
        ("audio", row(&|_, c| c.audio.clone())),
        ("cover", row(&|_, c| c.cover.clone())),
        ("lyrics", row(&|_, c| c.lyrics.clone())),
        ("by hand", row(&|n, _| tags(n))),
        ("alike", row(&|n, _| close(n))),
    ];
    let wide: Vec<usize> = (0..shown.len())
        .map(|at| {
            lines
                .iter()
                .map(|(_, v)| crate::ui::printable(&v[at]).width())
                .max()
                .unwrap_or(0)
        })
        .collect();
    lines
        .iter()
        .map(|(what, values)| {
            let mut line = format!("  {what:<8}");
            for (at, v) in values.iter().enumerate() {
                let v = crate::ui::printable(v);
                line.push_str("  ");
                line.push_str(&v);
                line.push_str(&" ".repeat(wide[at] - v.width()));
            }
            line.trim_end().to_string()
        })
        .collect()
}

/// Say what `chosen` does, ask, and make it as one change. Returns
/// whether anything changed.
fn apply<W: Write>(
    found: &mut Found,
    chosen: &[Choice],
    confirm: Confirm,
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    if chosen.is_empty() {
        crate::ui::info(out, "Nothing changed")?;
        return Ok(false);
    }
    let m = &found.manifest;
    let named = |n: usize| change::named(&found.views[n]);
    crate::ui::info(out, "Picked:")?;
    let (mut merged, mut fewer, mut removed) = (0, 0, 0);
    let mut edits = Vec::new();
    let mut touched = Vec::new();
    for choice in chosen {
        match choice {
            Choice::Merge {
                keeper,
                others,
                winners,
            } => {
                crate::ui::line(out, &format!("  ~ {}", named(*keeper)))?;
                for (tag, values) in winners {
                    crate::ui::line(out, &format!("      {tag}: {}", values.join("; ")))?;
                }
                for o in others {
                    crate::ui::line(
                        out,
                        &format!("  − {}, its sources moved into the song above", named(*o)),
                    )?;
                    let gone = merge::let_go(&m.songs[*o]);
                    if !gone.is_empty() {
                        crate::ui::line(
                            out,
                            &format!(
                                "      lets go of {}",
                                change::spoken_owned(
                                    &gone.iter().map(ToString::to_string).collect::<Vec<_>>()
                                )
                            ),
                        )?;
                    }
                }
                edits.push(merge::edit(m, *keeper, others, winners)?);
                touched.push(*keeper);
                touched.extend(others);
                merged += 1;
                fewer += others.len();
            }
            Choice::Remove(n) => {
                crate::ui::line(out, &format!("  − {}", named(*n)))?;
                let key = m.songs[*n].id().context("a song lists no source")?.clone();
                edits.push(Edit::Remove {
                    key,
                    note: found.views[*n].name(),
                });
                touched.push(*n);
                removed += 1;
                fewer += 1;
            }
        }
    }
    crate::ui::info(
        out,
        &format!(
            "{} fewer in the list. `muman undo` puts them back",
            change::counted(fewer, "song")
        ),
    )?;
    let question = match (merged, removed) {
        (0, r) => format!("Remove {}?", change::counted(r, "song")),
        (g, 0) => format!("Merge {}?", change::counted(g, "group")),
        (g, r) => format!(
            "Merge {} and remove {}?",
            change::counted(g, "group"),
            change::counted(r, "song")
        ),
    };
    if !change::confirmed(&confirm, &Ask::YesNo(question), prompter, out)? {
        return Ok(false);
    }
    found.manifest.trial(&edits)?;
    let expected: Vec<Song> = touched
        .iter()
        .map(|n| found.manifest.songs[*n].clone())
        .collect();
    for edit in edits {
        found.manifest.edit(edit);
    }
    let changed = found.manifest.save_if_unchanged(&expected)?;
    if !changed.is_empty() {
        found.manifest.discard();
        bail!("the song list changed while this ran; nothing was changed, run it again");
    }
    Ok(true)
}

/// What tells the album `view`'s song is on from another: its album
/// artist, else its artist, with its name; `None` on no album.
fn album_id(view: &View) -> Option<(String, String)> {
    let name = view.first("album");
    if name.is_empty() {
        return None;
    }
    let mut by = view.first("album_artist");
    if by.is_empty() {
        by = view.first("artist");
    }
    Some((by, name))
}

/// Where in `albums`, one per song of a group, an album holds it a
/// second time.
fn twice(albums: &[Option<(String, String)>]) -> Option<usize> {
    albums
        .iter()
        .enumerate()
        .find(|(n, a)| a.is_some() && albums[..*n].contains(a))
        .map(|(n, _)| n)
}

/// The line after the groups shown, `alone` with an album holding it
/// twice and `across` albums, of `found` in all.
fn summary<D: Write>(report: &mut D, (alone, across): (usize, usize), found: usize) -> Result<()> {
    if found == 0 {
        crate::ui::success(report, "No two songs are one recording")?;
    } else if alone + across == 0 {
        crate::ui::info(
            report,
            "No group of one recording holds a song the query matches",
        )?;
    } else {
        crate::ui::info(
            report,
            &format!(
                "{alone} group(s) an album holds twice, {across} across albums. To keep one song of a \
                 group, move the others' sources into its [[song]] with `muman edit`, or \
                 remove them"
            ),
        )?;
    }
    Ok(())
}

/// The shorter of two lengths over the longer.
fn ratio(a: usize, b: usize) -> f64 {
    a.min(b) as f64 / a.max(b).max(1) as f64
}

/// The groups of more than one that `links` make of `count` songs, each
/// in song-list order, ordered by their first song.
fn groups(count: usize, links: impl IntoIterator<Item = (usize, usize)>) -> Vec<Vec<usize>> {
    let mut root: Vec<usize> = (0..count).collect();
    for (a, b) in links {
        let (a, b) = (root_of(&mut root, a), root_of(&mut root, b));
        root[a.max(b)] = a.min(b);
    }
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for n in 0..count {
        let r = root_of(&mut root, n);
        by_root.entry(r).or_default().push(n);
    }
    by_root.into_values().filter(|g| g.len() > 1).collect()
}

/// The first song of `n`'s group, shortening the path to it on the way.
fn root_of(root: &mut [usize], mut n: usize) -> usize {
    while root[n] != n {
        root[n] = root[root[n]];
        n = root[n];
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_album_holding_a_recording_twice_is_found_among_others() {
        let on = |by: &str, name: &str| Some((by.to_string(), name.to_string()));
        let both = [
            on("Ada Quill", "Lantern Weather"),
            on("Ada Quill", "Best Of"),
            on("Ada Quill", "Lantern Weather"),
        ];
        assert_eq!(twice(&both), Some(2));
        assert_eq!(twice(&both[..2]), None);
    }

    #[test]
    fn albums_of_one_name_by_two_artists_or_no_album_hold_nothing_twice() {
        let on = |by: &str, name: &str| Some((by.to_string(), name.to_string()));
        assert_eq!(
            twice(&[on("Ada Quill", "Best Of"), on("Marlo Venn", "Best Of")]),
            None
        );
        assert_eq!(twice(&[None, None]), None);
        let mut view = View::default();
        assert_eq!(album_id(&view), None);
        view.tags = vec![
            ("ARTIST".into(), vec!["Marlo Venn".into()]),
            ("ALBUM".into(), vec!["Best Of".into()]),
        ];
        assert_eq!(album_id(&view), on("Marlo Venn", "Best Of"));
    }

    #[test]
    fn linked_songs_make_one_group() {
        assert_eq!(
            groups(6, [(4, 1), (1, 3), (2, 5)]),
            [vec![1, 3, 4], vec![2, 5]]
        );
        assert_eq!(groups(3, []), Vec::<Vec<usize>>::new());
    }

    #[test]
    fn prints_too_unalike_in_length_are_never_one_recording() {
        assert!(ratio(800, 1000) >= RATIO);
        assert!(ratio(799, 1000) < RATIO);
    }
}
