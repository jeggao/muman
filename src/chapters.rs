//! A source's chapters, as an audiobook or a video in parts has them, kept
//! in the song made from it: the song stays one file, and a player skips
//! from chapter to chapter within it.
//!
//! ffprobe reads them from any container ([`crate::probe`]); a song's
//! [`crate::resolve::Plan`] holds its audio's, and each container is
//! given them in its own form:
//!
//! | Container | Chapters |
//! |---|---|
//! | MP4 | Chapters ffmpeg writes from an `FFMETADATA` file ([`ffmetadata`]) |
//! | Ogg, FLAC | `CHAPTER000=00:00:00.000`, `CHAPTER000NAME=…` ([`vorbis`]), as ffmpeg writes them |
//! | MP3 | An `ID3v2` `CTOC` listing a `CHAP` for each, its title in a `TIT2` ([`id3`]) |
//! | WavPack | None: APEv2 names no form for them |

use std::borrow::Cow;
use std::fmt::Write;

use serde::{Deserialize, Serialize};

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

/// `text` with what `FFMETADATA` reads as its own escaped.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The chapters as Vorbis comments, numbered from 0 as ffmpeg numbers
/// them.
#[must_use]
pub fn vorbis(chapters: &[Chapter]) -> Vec<(String, String)> {
    let mut comments = Vec::new();
    for (n, c) in chapters.iter().enumerate() {
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

/// The chapters as `ID3v2` frames: a `CTOC` naming each `CHAP` in order,
/// a chapter's title its `TIT2`.
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
    let mut frames = vec![Frame::TableOfContents(ChapterTableOfContentsFrame::new(
        "toc",
        CtocFlags {
            top_level: true,
            ordered: true,
        },
        ids.clone(),
        FrameList::new(),
    ))];
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
}
