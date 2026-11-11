//! A source's chapters, as an audiobook or a video in parts has them, kept
//! in the song made from it: the song stays one file, and a player skips
//! from chapter to chapter within it.
//!
//! ffprobe reads them from any container ([`crate::probe`]); a song's
//! `resolve::Plan` holds its audio's, and each container is
//! given them in its own form:
//!
//! | Container | Chapters |
//! |---|---|
//! | MP4 | Chapters ffmpeg writes from an `FFMETADATA` file ([`ffmetadata`]) |
//! | Ogg, FLAC | `CHAPTER000=00:00:00.000`, `CHAPTER000NAME=…` ([`vorbis`]), as ffmpeg writes them |
//! | MP3 | An `ID3v2` `CTOC` listing a `CHAP` for each, its title in a `TIT2` ([`id3`]) |
//! | WavPack | None: APEv2 names no form for them |
//!
//! Vorbis comments number chapters in three digits, `CHAPTER000` to
//! `CHAPTER999`, as the chapter extension of the Vorbis comment names
//! them and ffmpeg reads them: a `CHAPTER1000NAME` it reads as the name
//! of chapter 100. Ogg and FLAC keep the first [`VORBIS_MOST`], and the
//! song says the rest are not kept ([`held`]). A `CTOC` counts its
//! entries in one byte, so more than 255 chapters are listed in a
//! hierarchy: a top-level `CTOC` of `CTOC`s, each of at most 255, as
//! `ID3v2`'s chapter frames allow; players and ffmpeg read the `CHAP`
//! frames themselves, in the order of their times.

use std::borrow::Cow;
use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::codec::Container;

/// The most chapters Vorbis comments number, `CHAPTER000` to `CHAPTER999`.
pub const VORBIS_MOST: usize = 1000;

/// The most entries one `CTOC` lists: its count is one byte.
const CTOC_MOST: usize = u8::MAX as usize;

/// One chapter: where it starts and ends, in milliseconds, and its title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chapter {
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
}

/// The chapters of audio `length_ms` long, where it is known: each
/// within it, those past its end gone. One chapter alone, or none, is no
/// chapters: there is nothing to skip between.
#[must_use]
pub fn within(chapters: &[Chapter], length_ms: Option<u64>) -> Vec<Chapter> {
    let end = length_ms.unwrap_or(u64::MAX);
    let kept: Vec<Chapter> = chapters
        .iter()
        .filter(|c| c.start_ms < end)
        .map(|c| Chapter {
            end_ms: c.end_ms.min(end),
            ..c.clone()
        })
        .collect();
    if kept.len() < 2 { Vec::new() } else { kept }
}

/// The chapters `container` holds, and what it leaves out where it
/// leaves out any it could have held a form for.
#[must_use]
pub fn held(mut chapters: Vec<Chapter>, container: Container) -> (Vec<Chapter>, Option<String>) {
    match container {
        Container::WavPack => (Vec::new(), None),
        Container::Ogg | Container::Flac if chapters.len() > VORBIS_MOST => {
            let why = format!(
                "chapters: the first {VORBIS_MOST} of {} kept, as many as Vorbis comments number",
                chapters.len()
            );
            chapters.truncate(VORBIS_MOST);
            (chapters, Some(why))
        }
        _ => (chapters, None),
    }
}

/// The chapters as an `FFMETADATA` file, which ffmpeg reads with
/// `-map_chapters`.
#[must_use]
pub fn ffmetadata(chapters: &[Chapter]) -> String {
    let mut text = String::from(";FFMETADATA1\n");
    for c in chapters {
        let _ = write!(
            text,
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={}\n",
            c.start_ms, c.end_ms
        );
        if !c.title.is_empty() {
            let _ = writeln!(text, "title={}", escaped(&c.title));
        }
    }
    text
}

/// `text` with what `FFMETADATA` reads as its own escaped: ffmpeg ends a
/// line at a carriage return as at a line break, so a title holding
/// either unescaped would end there and the rest be read as lines of the
/// file, a chapter of a title's making among them. A NUL, which ends
/// ffmpeg's reading of the file whether escaped or not, is dropped.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().filter(|c| *c != '\0') {
        if matches!(c, '=' | ';' | '#' | '\\' | '\n' | '\r') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The chapters as Vorbis comments, numbered from 0 as ffmpeg numbers
/// them; the first [`VORBIS_MOST`] alone.
#[must_use]
pub fn vorbis(chapters: &[Chapter]) -> Vec<(String, String)> {
    let mut comments = Vec::new();
    for (n, c) in chapters.iter().take(VORBIS_MOST).enumerate() {
        comments.push((format!("CHAPTER{n:03}"), timestamp(c.start_ms)));
        if !c.title.is_empty() {
            comments.push((format!("CHAPTER{n:03}NAME"), c.title.clone()));
        }
    }
    comments
}

fn timestamp(ms: u64) -> String {
    let (s, ms) = (ms / 1000, ms % 1000);
    format!("{:02}:{:02}:{:02}.{ms:03}", s / 3600, s / 60 % 60, s % 60)
}

/// The chapters as `ID3v2` frames: a top-level `CTOC` naming each `CHAP`
/// in order, or, past [`CTOC_MOST`], `CTOC`s naming them in runs of that
/// many, as deep as it takes; a chapter's title its `TIT2`.
#[must_use]
pub fn id3(chapters: &[Chapter]) -> Vec<lofty::id3::v2::Frame<'static>> {
    use lofty::id3::v2::{
        ChapterFrame, ChapterTableOfContentsFrame, CtocFlags, Frame, FrameId, FrameList,
        TextInformationFrame,
    };

    if chapters.is_empty() {
        return Vec::new();
    }
    let ids: Vec<Cow<'static, str>> = (0..chapters.len())
        .map(|n| Cow::Owned(format!("chp{n}")))
        .collect();
    let toc = |id: Cow<'static, str>, top_level, entries: Vec<Cow<'static, str>>| {
        Frame::TableOfContents(ChapterTableOfContentsFrame::new(
            id,
            CtocFlags {
                top_level,
                ordered: true,
            },
            entries,
            FrameList::new(),
        ))
    };
    let mut inner = Vec::new();
    let mut top = ids.clone();
    while top.len() > CTOC_MOST {
        top = top
            .chunks(CTOC_MOST)
            .map(|run| {
                let id: Cow<'static, str> = Cow::Owned(format!("toc{}", inner.len()));
                inner.push(toc(id.clone(), false, run.to_vec()));
                id
            })
            .collect();
    }
    let mut frames = vec![toc(Cow::Borrowed("toc"), true, top)];
    frames.append(&mut inner);
    let ms = |t: u64| u32::try_from(t).unwrap_or(u32::MAX);
    for (id, c) in ids.into_iter().zip(chapters) {
        let mut children = FrameList::new();
        if !c.title.is_empty()
            && let Ok(title) = FrameId::new("TIT2")
        {
            children.insert(Frame::Text(TextInformationFrame::new(
                title,
                lofty::TextEncoding::UTF8,
                c.title.clone(),
            )));
        }
        frames.push(Frame::Chapter(ChapterFrame::new(
            id,
            ms(c.start_ms)..ms(c.end_ms),
            u32::MAX..u32::MAX,
            children,
        )));
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> Vec<Chapter> {
        vec![
            Chapter {
                start_ms: 0,
                end_ms: 61_500,
                title: "Lantern; Weather = One".into(),
            },
            Chapter {
                start_ms: 61_500,
                end_ms: 3_725_042,
                title: String::new(),
            },
        ]
    }

    #[test]
    fn chapters_are_held_within_the_audio() {
        let cut = within(&two(), Some(90_000));
        assert_eq!(cut.len(), 2);
        assert_eq!(cut[1].end_ms, 90_000);
        assert!(within(&two(), Some(30_000)).is_empty(), "one is none");
        assert_eq!(within(&two(), None), two());
    }

    #[test]
    fn each_container_s_form_holds_every_chapter() {
        assert_eq!(
            ffmetadata(&two()),
            ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=61500\n\
             title=Lantern\\; Weather \\= One\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=61500\n\
             END=3725042\n"
        );
        assert_eq!(
            vorbis(&two()),
            [
                ("CHAPTER000".into(), "00:00:00.000".into()),
                ("CHAPTER000NAME".into(), "Lantern; Weather = One".into()),
                ("CHAPTER001".into(), "00:01:01.500".into()),
            ]
        );
        let frames = id3(&two());
        assert_eq!(frames.len(), 3, "a CTOC and two CHAPs");
        assert_eq!(id3(&[]).len(), 0);
    }

    fn many(n: usize) -> Vec<Chapter> {
        (0..n as u64)
            .map(|i| Chapter {
                start_ms: i * 1000,
                end_ms: (i + 1) * 1000,
                title: format!("Part {i}"),
            })
            .collect()
    }

    #[test]
    fn any_number_of_chapters_is_listed_in_ctocs_of_at_most_255() {
        use lofty::id3::v2::{Frame, Id3v2Tag};
        use lofty::tag::TagExt;
        use std::collections::BTreeMap;

        fn walk(tocs: &BTreeMap<String, Vec<Cow<'_, str>>>, id: &str, out: &mut Vec<String>) {
            for entry in &tocs[id] {
                if tocs.contains_key(entry.as_ref()) {
                    walk(tocs, entry, out);
                } else {
                    out.push(entry.to_string());
                }
            }
        }

        for n in [2, 255, 256, 1005, 70_000] {
            let frames = id3(&many(n));
            let mut tocs = BTreeMap::new();
            let mut top = Vec::new();
            for f in &frames {
                if let Frame::TableOfContents(t) = f {
                    assert!(t.entries.len() <= 255, "{n}: {} entries", t.entries.len());
                    if t.flags.top_level {
                        top.push(t.id.to_string());
                    }
                    tocs.insert(t.id.to_string(), t.entries.to_vec());
                }
            }
            assert_eq!(top, ["toc"], "{n}: one top level");
            let mut reached = Vec::new();
            walk(&tocs, "toc", &mut reached);
            let chaps: Vec<String> = (0..n).map(|i| format!("chp{i}")).collect();
            assert_eq!(reached, chaps, "{n}: each CHAP once, in order");
            assert_eq!(frames.len(), n + tocs.len());

            if n <= 1005 {
                let mut tag = Id3v2Tag::new();
                for f in frames {
                    tag.insert(f);
                }
                let mut bytes = Vec::new();
                tag.dump_to(&mut bytes, lofty::config::WriteOptions::default())
                    .unwrap_or_else(|e| panic!("{n}: {e}"));
            }
        }
    }

    #[test]
    fn ogg_and_flac_keep_the_chapters_vorbis_comments_number() {
        let (kept, why) = held(many(1005), Container::Flac);
        assert_eq!(kept, many(1000));
        assert_eq!(
            why.as_deref(),
            Some("chapters: the first 1000 of 1005 kept, as many as Vorbis comments number")
        );
        assert_eq!(held(many(1000), Container::Ogg), (many(1000), None));
        assert_eq!(held(many(1005), Container::Mp3), (many(1005), None));
        assert_eq!(held(many(3), Container::WavPack), (Vec::new(), None));

        let comments = vorbis(&many(1005));
        assert_eq!(comments.len(), 2000);
        assert_eq!(comments[1998].0, "CHAPTER999");
    }

    #[test]
    fn a_title_cannot_end_its_line_in_ffmetadata() {
        let forged = Chapter {
            start_ms: 0,
            end_ms: 1000,
            title: "One\r[CHAPTER]\rSTART=1500\ntitle=Two\0".into(),
        };
        assert_eq!(
            ffmetadata(&[forged]),
            ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\n\
             title=One\\\r[CHAPTER]\\\rSTART\\=1500\\\ntitle\\=Two\n"
        );
    }
}
