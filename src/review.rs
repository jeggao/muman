//! `muman review`: each song whose picks are in doubt, one at a time, with
//! what can settle it.
//!
//! `check` lists what resolving left in doubt ([`Doubt`]): a title won by
//! the order of `sources`, a source set aside as naming another
//! recording, a song holding two recordings or playing another length
//! than its record says. Here each song the query picks that holds any
//! is shown with them, and a menu answers each:
//!
//! - change its tags and pins, as `muman set` with a query does on a
//!   terminal ([`crate::guided`]);
//! - pin it to one of the releases AcoustID found its recording on, its
//!   `release` key ([`muman_engine::release`]);
//! - leave a field out of everything one torrent or one channel offers,
//!   where a doubt names one of its sources, a `[[tags.from]]` in the song
//!   list ([`muman_media::overrides`]);
//! - leave it as it is, or stop.
//!
//! Each change is saved as it is made, so stopping keeps what was
//! decided, under one run begun at the first ([`crate::history::Later`]):
//! its record is written ahead of every save, so a review stopped
//! anywhere, by Ctrl-C or a kill, leaves no change a record does not
//! cover, and one `undo` reverses them all. The run lets go of the runs
//! lock before each next question, so no other run waits on one; a run
//! begun meanwhile goes on in its record, as in a stopped run's, and ends
//! it, the answers before then undone with that run. The sync after the
//! last answer goes on in the record too.
//!
//! One run per answer would also leave none uncovered, but take as many
//! `undo`s as answers and push older runs out of the history's count,
//! and its sync could not be one: a run that only syncs records no change
//! of the song list, so undoing it would take back the library alone,
//! which the sync after that undo writes again.

use std::collections::BTreeSet;
use std::io::Write;

use anyhow::Result;

use crate::change::Read;
use crate::cli::Confirm;
use crate::dirs::Dirs;
use crate::errors::Refused;
use crate::manifest::{Edit, Manifest, Song};
use crate::resolve::{Doubt, Resolved};
use crate::source::SourceKey;
use crate::state::State;
use crate::tags::Field;
use crate::ui::Prompter;

/// What one answer to a song in doubt does.
enum Answer {
    /// Its tags and pins, changed field by field.
    Open,
    /// Pinned to one of its recording's releases.
    Release,
    /// `field` left out of everything a torrent or a channel offers.
    Never {
        field: Field,
        torrent: Option<String>,
        channel: Option<String>,
    },
    Skip,
    Stop,
}

impl Answer {
    fn label(&self) -> String {
        match self {
            Self::Open => "Change its tags and pins…".into(),
            Self::Release => "Pin it to one of its recording's releases…".into(),
            Self::Never {
                field,
                torrent,
                channel,
            } => match (torrent, channel) {
                (Some(hash), _) => format!(
                    "Never take {} from torrent {}",
                    field.vorbis(),
                    &hash[..hash.len().min(8)]
                ),
                (None, Some(name)) => format!("Never take {} from channel {name}", field.vorbis()),
                (None, None) => format!("Never take {}", field.vorbis()),
            },
            Self::Skip => "Leave it as it is".into(),
            Self::Stop => "Stop reviewing".into(),
        }
    }
}

/// Walk the songs `terms` picks whose picks are in doubt. Returns whether
/// anything changed.
pub fn review<W: Write>(
    dirs: &Dirs,
    terms: &[String],
    prompter: Option<&mut dyn Prompter>,
    later: &mut crate::history::Later,
    out: &mut W,
) -> Result<bool> {
    let Some(p) = prompter else {
        return Err(Refused(
            "`review` asks about each song in doubt, so it needs a terminal; \
             `muman check` lists them"
                .into(),
        )
        .into());
    };
    let read = Read::new(dirs, terms)?;
    let picked = read.pick(true, "review", None)?;
    let doubtful: Vec<usize> = picked
        .into_iter()
        .filter(|n| {
            read.resolved[*n]
                .as_ref()
                .is_some_and(|r| !r.why.doubts.is_empty())
        })
        .collect();
    if doubtful.is_empty() {
        crate::ui::info(out, "Nothing in doubt")?;
        return Ok(false);
    }
    let mut changed = false;
    for (at, &n) in doubtful.iter().enumerate() {
        let (Some(r), Some(id)) = (&read.resolved[n], read.manifest.songs[n].id()) else {
            continue;
        };
        crate::ui::line(
            out,
            &format!(
                "{} ({} of {})",
                read.views[n].name(),
                at + 1,
                doubtful.len()
            ),
        )?;
        for doubt in &r.why.doubts {
            crate::ui::line(out, &format!("  {}", doubt.line()))?;
        }
        let song = &read.manifest.songs[n];
        let answers = answers(song, r, &read.state);
        let labels: Vec<String> = answers.iter().map(Answer::label).collect();
        let Some(chosen) = p.select("What settles it?", &labels)? else {
            break;
        };
        match &answers[chosen] {
            Answer::Open => {
                let confirm = Confirm {
                    yes: false,
                    all: true,
                    dry_run: false,
                };
                changed |= crate::guided::edit(dirs, &[id.to_string()], &confirm, p, later, out)?;
            }
            Answer::Release => {
                changed |= pin_release(dirs, song, id, &read.state, p, later, out)?;
            }
            Answer::Never {
                field,
                torrent,
                channel,
            } => {
                let mut m = Manifest::load(&dirs.home)?;
                m.edit(Edit::TagsFrom {
                    torrent: torrent.clone(),
                    channel: channel.clone(),
                    drop: vec![field.vorbis().to_lowercase()],
                });
                m.save(later.begin()?)?;
                crate::ui::info(
                    out,
                    &format!("Added to [[tags.from]]: {}", answers[chosen].label()),
                )?;
                changed = true;
            }
            Answer::Skip => {}
            Answer::Stop => break,
        }
        later.let_go();
    }
    Ok(changed)
}

/// What can answer `song`'s doubts, `r` as it resolves.
fn answers(song: &Song, r: &Resolved, state: &State) -> Vec<Answer> {
    let mut answers = vec![Answer::Open];
    if !releases(song, state).is_empty() {
        answers.push(Answer::Release);
    }
    let mut offered = BTreeSet::new();
    for doubt in &r.why.doubts {
        let Doubt::Contested { field, key, .. } = doubt else {
            continue;
        };
        let (torrent, channel) = match key {
            SourceKey::Torrent(t) => (Some(t.hash().to_string()), None),
            other => (None, channel_of(other, state)),
        };
        if (torrent.is_some() || channel.is_some()) && offered.insert((*field, key.clone())) {
            answers.push(Answer::Never {
                field: *field,
                torrent,
                channel,
            });
        }
    }
    answers.extend([Answer::Skip, Answer::Stop]);
    answers
}

/// The channel an upload credits, where it names no artist of its own.
fn channel_of(key: &SourceKey, state: &State) -> Option<String> {
    let artist = state.facts.get(key)?.tags.get(&Field::Artist)?;
    (!artist.structured)
        .then(|| artist.values.first().cloned())
        .flatten()
}

/// The releases AcoustID found `song`'s recording on, each once: its ID
/// and how it is shown.
fn releases(song: &Song, state: &State) -> Vec<(String, String)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let records = song.sources.iter().filter(
        |k| matches!(k, SourceKey::Remote { site, .. } if site == crate::provider::MUSICBRAINZ),
    );
    for record in records {
        for recording in state.candidates(&song.sources, record) {
            for r in &recording.releases {
                if seen.insert(r.id.clone()) {
                    let about: Vec<String> = [
                        r.date.clone(),
                        r.country.clone(),
                        (!r.types.is_empty()).then(|| r.types.join(", ")),
                        r.track.map(|t| format!("track {t}")),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    out.push((r.id.clone(), format!("{} ({})", r.title, about.join("; "))));
                }
            }
        }
    }
    out
}

/// Pin `song`, listed by `id`, to a release its recording is on, or take
/// away the pin it has, saved under the run `later` begins. Returns
/// whether the song list changed.
fn pin_release<W: Write>(
    dirs: &Dirs,
    song: &Song,
    id: &SourceKey,
    state: &State,
    p: &mut dyn Prompter,
    later: &mut crate::history::Later,
    out: &mut W,
) -> Result<bool> {
    let releases = releases(song, state);
    let mut labels: Vec<String> = releases.iter().map(|(_, shown)| shown.clone()).collect();
    if song.release.is_some() {
        labels.push("No pin: let its album decide".into());
    }
    let Some(chosen) = p.select("Which release is it on?", &labels)? else {
        return Ok(false);
    };
    let release = releases.get(chosen).map(|(id, _)| id.clone());
    if release == song.release {
        return Ok(false);
    }
    let mut m = Manifest::load(&dirs.home)?;
    m.edit(Edit::Release {
        key: id.clone(),
        release: release.clone(),
    });
    m.save(later.begin()?)?;
    let said = match &release {
        Some(r) => format!("Pinned to release {r}"),
        None => "Its release pin taken away".into(),
    };
    crate::ui::info(out, &said)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::{Format, Why};

    fn doubted(doubts: Vec<Doubt>) -> Resolved {
        Resolved {
            plan: muman_engine::resolve::testing::every_part(Format::Copy {
                codec: crate::codec::Codec::Flac,
            }),
            stem: "song".into(),
            why: Why {
                doubts,
                ..Why::default()
            },
        }
    }

    #[test]
    fn a_title_contested_from_a_torrent_or_a_channel_can_be_left_out_of_it() {
        let torrent = SourceKey::parse(&format!("torrent:{}/01.flac", "a".repeat(40))).unwrap();
        let upload = SourceKey::youtube("vid00000001");
        let mut state = State::default();
        let mut facts = crate::facts::Facts::unreadable("r".into());
        let channel = crate::tags::Offer {
            values: vec!["Orchard Archive".into()],
            structured: false,
            also: Vec::new(),
        };
        facts.tags.insert(Field::Artist, channel);
        state.facts.insert(upload.clone(), facts);
        let contested = |key: &SourceKey| Doubt::Contested {
            field: Field::Title,
            key: key.clone(),
            why: String::new(),
        };
        let song = Song {
            sources: vec![torrent.clone(), upload.clone()],
            ..Song::default()
        };
        let r = doubted(vec![contested(&torrent), contested(&upload)]);
        let labels: Vec<String> = answers(&song, &r, &state)
            .iter()
            .map(Answer::label)
            .collect();
        assert_eq!(
            labels,
            [
                "Change its tags and pins…",
                "Never take TITLE from torrent aaaaaaaa",
                "Never take TITLE from channel Orchard Archive",
                "Leave it as it is",
                "Stop reviewing"
            ]
        );
    }

    #[test]
    fn review_needs_a_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("library"),
            locks: None,
        };
        let e = review(
            &dirs,
            &[],
            None,
            &mut crate::history::Later::new(&dirs.home).unwrap(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("needs a terminal"), "{e}");
    }
}
