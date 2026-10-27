//! Songs shown as rows that tell them apart: the album, track, length,
//! audio, cover and lyrics earlier runs measured, in aligned columns.
//!
//! A pick among songs that share a title and artist, as the copies of
//! one recording do, is a pick among what they do not share. So a
//! column every row holds alike says nothing and goes, and a name every
//! row shares moves to the question instead. The key stays, short as
//! its ID or its file's stem, unless two rows' short keys are alike,
//! as two files named `Tide.flac` in two folders are; then each row
//! shows the whole key.
//!
//! A prompt's items are drawn as plain text, so the columns are told
//! apart by their place and their own words ("#2", "timed lyrics"),
//! never by color. They are padded by display width, so a title in a
//! script whose characters take two cells still lines up, and a row
//! wider than the terminal loses whole columns, the least telling
//! first, before its name is cut.

use std::collections::BTreeMap;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::facts::{AudioFacts, Facts};
use crate::query::View;
use crate::resolve::Plan;
use crate::source::SourceKey;

/// What one song shows, each a column's text, empty when unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cells {
    pub name: String,
    pub album: String,
    pub track: String,
    pub length: String,
    pub audio: String,
    pub cover: String,
    pub lyrics: String,
    /// The key's ID or file stem, and the whole key.
    pub short: String,
    pub key: String,
}

/// Rows fitted to a width: what every row shares, and each row's line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rows {
    pub shared: Option<String>,
    pub lines: Vec<String>,
}

/// Columns in the order shown; those before `Key` go first when a row
/// is too wide, from the last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Name,
    Album,
    Track,
    Length,
    Audio,
    Cover,
    Lyrics,
    Key,
}

const COLUMNS: [Column; 8] = [
    Column::Name,
    Column::Album,
    Column::Track,
    Column::Length,
    Column::Audio,
    Column::Cover,
    Column::Lyrics,
    Column::Key,
];

/// What a multi-select draws before an item: the cursor and the box.
const MARGIN: usize = "> [ ] ".len();

const GAP: &str = "  ";

/// What `view`'s song shows: from `plan` where it resolves, else from
/// the first of its sources measured.
#[must_use]
pub fn cells(view: &View, plan: Option<&Plan>, facts: &BTreeMap<SourceKey, Facts>) -> Cells {
    let audio_key = plan.map(|p| &p.audio.key).or_else(|| {
        view.keys
            .iter()
            .find(|k| facts.get(k).is_some_and(|f| f.audio.is_some()))
    });
    let measured = audio_key.and_then(|k| facts.get(k));
    let seconds = measured.and_then(Facts::length);
    let cover = match plan.map(|p| p.cover.as_ref()) {
        Some(Some(c)) => facts
            .get(&c.key)
            .and_then(|f| f.covers.iter().find(|x| x.at == c.at))
            .and_then(|x| x.quality.as_ref())
            .map_or_else(
                || "cover".to_string(),
                |q| format!("{}×{}", q.width, q.height),
            ),
        None if view.cover => "cover".to_string(),
        Some(None) | None => "no cover".to_string(),
    };
    let lyrics = match plan.map(|p| p.lyrics.as_ref()) {
        Some(Some(l)) => facts
            .get(&l.key)
            .and_then(|f| f.lyrics.iter().find(|x| x.at == l.at))
            .map_or("lyrics", |x| {
                if x.timing.is_some() {
                    "timed lyrics"
                } else {
                    "plain lyrics"
                }
            }),
        None if view.lyrics => "lyrics",
        Some(None) | None => "no lyrics",
    };
    let track = view.first("track");
    Cells {
        name: view.name(),
        album: view.first("album"),
        track: if track.is_empty() {
            track
        } else {
            format!("#{track}")
        },
        length: seconds
            .map(|s| minutes((s * 1000.0).round() as i64))
            .unwrap_or_default(),
        audio: measured
            .and_then(|f| f.audio.as_ref())
            .map(|a| audio(a, seconds))
            .unwrap_or_default(),
        cover,
        lyrics: lyrics.to_string(),
        short: view.id().map(SourceKey::short).unwrap_or_default(),
        key: view.id().map(ToString::to_string).unwrap_or_default(),
    }
}

/// A source's audio as a listener names it: lossless by its sample size
/// and rate, as "FLAC 16/44.1", lossy by its bitrate, as "Opus 160k".
#[must_use]
pub fn audio(a: &AudioFacts, seconds: Option<f64>) -> String {
    let label = crate::codec::Codec::probed(&a.codec)
        .map_or_else(|| a.codec.to_uppercase(), |c| c.label().to_string());
    if crate::codec::is_lossless_name(&a.codec, a.profile.as_deref()) {
        let bits = a.quality.as_ref().map_or(a.bits, |q| q.bits);
        let khz = crate::units::khz_of_hz(f64::from(a.sample_rate));
        let float = if a.float { "f" } else { "" };
        return match (bits, a.sample_rate) {
            (0, 0) => label,
            (0, _) => format!("{label} {khz}"),
            _ => format!("{label} {bits}{float}/{khz}"),
        };
    }
    match (a.bytes, seconds) {
        (Some(bytes), Some(s)) if s > 0.0 => {
            format!("{label} {:.0}k", bytes as f64 * 8.0 / s / 1000.0)
        }
        _ => label,
    }
}

/// `ms` as minutes and seconds, as "4:05".
#[must_use]
pub fn minutes(ms: i64) -> String {
    let s = (ms + 500) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// `songs` as aligned rows, each no wider than `width` with room for a
/// multi-select's box, or as wide as they come when `width` is `None`.
#[must_use]
pub fn fit(songs: &[Cells], width: Option<usize>) -> Rows {
    let clean = |s: &str| crate::ui::printable(s).replace(['\t', '\n'], " ");
    let shorts_collide = songs
        .iter()
        .enumerate()
        .any(|(n, s)| songs[..n].iter().any(|o| o.short == s.short));
    let text = |s: &Cells, c: Column| -> String {
        clean(match c {
            Column::Name => &s.name,
            Column::Album => &s.album,
            Column::Track => &s.track,
            Column::Length => &s.length,
            Column::Audio => &s.audio,
            Column::Cover => &s.cover,
            Column::Lyrics => &s.lyrics,
            Column::Key if shorts_collide => &s.key,
            Column::Key => &s.short,
        })
    };
    let alike = |c: Column| songs.windows(2).all(|w| text(&w[0], c) == text(&w[1], c));
    let shared = (songs.len() > 1 && alike(Column::Name))
        .then(|| songs.first().map(|s| text(s, Column::Name)))
        .flatten()
        .filter(|n| !n.is_empty());
    let mut shown: Vec<Column> = COLUMNS
        .into_iter()
        .filter(|c| *c == Column::Key || songs.len() < 2 || !alike(*c))
        .collect();
    let cells: Vec<Vec<String>> = songs
        .iter()
        .map(|s| COLUMNS.iter().map(|c| text(s, *c)).collect())
        .collect();
    let at = |c: Column| COLUMNS.iter().position(|x| *x == c).unwrap_or(0);
    let widest = |c: Column| {
        cells
            .iter()
            .map(|row| row[at(c)].width())
            .max()
            .unwrap_or(0)
    };
    let total = |shown: &[Column]| {
        shown.iter().map(|c| widest(*c)).sum::<usize>() + GAP.len() * shown.len().saturating_sub(1)
    };
    let room = width.map(|w| w.saturating_sub(MARGIN));
    let mut name_room = None;
    if let Some(room) = room {
        for drop in [
            Column::Lyrics,
            Column::Cover,
            Column::Audio,
            Column::Length,
            Column::Track,
            Column::Album,
        ] {
            if total(&shown) <= room {
                break;
            }
            shown.retain(|c| *c != drop);
        }
        let over = total(&shown).saturating_sub(room);
        if over > 0 && shown.contains(&Column::Name) {
            name_room = Some(widest(Column::Name).saturating_sub(over).max(1));
        }
    }
    let lines = cells
        .iter()
        .map(|row| {
            let mut line = String::new();
            for (i, c) in shown.iter().enumerate() {
                if i > 0 {
                    line.push_str(GAP);
                }
                let mut cell = row[at(*c)].clone();
                let mut wide = widest(*c);
                if *c == Column::Name
                    && let Some(r) = name_room
                {
                    cell = cut(&cell, r);
                    wide = r;
                }
                let pad = " ".repeat(wide.saturating_sub(cell.width()));
                if matches!(c, Column::Track | Column::Length) {
                    line.push_str(&pad);
                    line.push_str(&cell);
                } else {
                    line.push_str(&cell);
                    line.push_str(&pad);
                }
            }
            line.trim_end().to_string()
        })
        .collect();
    Rows { shared, lines }
}

/// `text` no wider than `room`, ending in "…" when cut.
fn cut(text: &str, room: usize) -> String {
    if text.width() <= room {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > room {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(name: &str, album: &str, audio: &str, short: &str) -> Cells {
        Cells {
            name: name.into(),
            album: album.into(),
            track: "#2".into(),
            length: "4:12".into(),
            audio: audio.into(),
            cover: "1200×1200".into(),
            lyrics: "timed lyrics".into(),
            short: short.into(),
            key: format!("manual:{short}"),
        }
    }

    #[test]
    fn what_every_row_shares_goes_and_a_shared_name_moves_to_the_question() {
        let rows = fit(
            &[
                song("Tide — Ada Quill", "Lantern Weather", "FLAC 16/44.1", "a"),
                song("Tide — Ada Quill", "Night Ferry Sessions", "Opus 160k", "b"),
            ],
            None,
        );
        assert_eq!(rows.shared.as_deref(), Some("Tide — Ada Quill"));
        assert_eq!(
            rows.lines,
            [
                "Lantern Weather       FLAC 16/44.1  a",
                "Night Ferry Sessions  Opus 160k     b",
            ]
        );
    }

    #[test]
    fn wide_characters_line_up_by_the_cells_they_take() {
        let rows = fit(
            &[
                song("灯台", "Lantern Weather", "x", "a"),
                song("Tide", "Lantern Weather", "x", "b"),
            ],
            None,
        );
        assert_eq!(rows.lines, ["灯台  a", "Tide  b"]);
    }

    #[test]
    fn a_narrow_terminal_drops_the_least_telling_columns_first_never_the_key() {
        let mut a = song(
            "Paper Comets — Marlo Venn",
            "Lantern Weather",
            "FLAC 16/44.1",
            "a",
        );
        let mut b = song("Ember Road — Marlo Venn", "Night Ferry", "Opus 160k", "b");
        b.lyrics = "no lyrics".into();
        b.cover = "no cover".into();
        let wide = fit(&[a.clone(), b.clone()], None);
        assert!(wide.lines[0].contains("timed lyrics"), "{:?}", wide.lines);
        let narrow = fit(&[a.clone(), b.clone()], Some(60));
        assert!(narrow.lines.iter().all(|l| l.width() <= 54), "{narrow:?}");
        assert!(!narrow.lines[0].contains("timed lyrics"), "{narrow:?}");
        assert!(narrow.lines[0].ends_with(" a"), "{narrow:?}");
        a.name = "A title far too long to fit in any terminal this narrow".into();
        let cut = fit(&[a, b], Some(20));
        assert!(cut.lines[0].contains('…'), "{cut:?}");
        assert!(cut.lines[0].ends_with(" a"), "{cut:?}");
    }

    #[test]
    fn short_keys_alike_show_whole_keys() {
        let rows = fit(
            &[
                song("Tide", "A", "x", "Tide"),
                song("Tide", "B", "x", "Tide"),
            ],
            None,
        );
        assert_eq!(rows.lines, ["A  manual:Tide", "B  manual:Tide"]);
    }

    #[test]
    fn a_name_s_terminal_sequences_show_as_text() {
        let rows = fit(
            &[
                song("Tide \x1b]0;X\x07", "A", "x", "a"),
                song("Tide", "B", "x", "b"),
            ],
            None,
        );
        assert!(!rows.lines[0].contains('\x1b'), "{rows:?}");
        assert!(rows.lines[0].contains('\u{FFFD}'), "{rows:?}");
    }

    #[test]
    fn audio_is_named_by_its_sample_size_or_bitrate() {
        let mut a = AudioFacts {
            codec: "flac".into(),
            sample_rate: 44_100,
            bits: 16,
            ..AudioFacts::default()
        };
        assert_eq!(audio(&a, Some(240.0)), "FLAC 16/44.1");
        a.codec = "opus".into();
        a.bytes = Some(4_800_000);
        assert_eq!(audio(&a, Some(240.0)), "Opus 160k");
        assert_eq!(minutes(252_400), "4:12");
    }
}
