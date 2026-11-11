//! A song's lyrics as the library holds them, inside it and in the files
//! beside it, made from their part of the plan alone
//! ([`crate::artifact::LyricText`]): read from their source, a subtitle
//! stream through ffmpeg, cleaned as the part's switches say
//! ([`crate::clean::lyrics`]), moved by its offset, and written in each
//! form a file or the tags take ([`crate::cues::write`]).

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow};

use super::{Inputs, Job};
use crate::artifact::{LyricText, Parts};
use crate::facts::{LRC_ARGS, LyricsAt};
use crate::ffmpeg::{self, Output};
use crate::runner::Runner;
use crate::store::Located;

/// The files of lyrics `made` has beside the song at `stem`.
pub(super) fn rels(stem: &Path, made: &Parts<'_>) -> Vec<PathBuf> {
    made.lyric_files()
        .map(|f| crate::outputs::lyrics_file(stem, &f.file))
        .collect()
}

/// What each file of lyrics `made` has beside the song holds, from
/// their `text` as kept; none for one whose format takes no such lyrics.
pub(super) fn contents(made: &Parts<'_>, text: &str) -> Vec<Option<Vec<u8>>> {
    made.lyric_files()
        .map(|f| {
            crate::cues::write(text, f.file.format, f.file.words, f.text.end_ms)
                .map(String::into_bytes)
        })
        .collect()
}

/// The lyrics the song holds inside it, from their `text` as kept; none
/// where they go only beside it, or their form takes no such lyrics.
pub(super) fn inside(made: &Parts<'_>, text: &str) -> Option<String> {
    let l = made.lyrics.as_ref()?;
    crate::cues::write(text, l.form.format, l.form.words, None)
}

/// The text `lyrics` make, read from their source, a subtitle stream
/// through ffmpeg alone; none when no line is.
pub(super) fn text<R: Runner>(
    runner: &R,
    job: &Job<'_>,
    lyrics: &LyricText<'_>,
    problems: &mut Vec<String>,
) -> Result<Option<String>> {
    let source = job
        .sources
        .get(lyrics.key)
        .ok_or_else(|| anyhow!("{} is not on disk", lyrics.key))?;
    fs::create_dir_all(job.scratch)?;
    let raw = job.scratch.join("lyrics.lrc");
    let (mut inputs, mut outputs) = (Inputs::default(), Vec::new());
    let mut text = planned(lyrics, source, &raw, &mut inputs, &mut outputs, problems)?;
    if text.is_none() {
        let paths: Vec<&Path> = inputs.0.iter().map(PathBuf::as_path).collect();
        for result in ffmpeg::run_outputs(runner, &paths, &outputs) {
            result?;
        }
        let read = fs::read(&raw).with_context(|| format!("reading {}", raw.display()))?;
        text = Some(cleaned(&crate::lyrics::decode(&read), lyrics, problems));
    }
    Ok(text.filter(|t| t.lines().any(|l| !l.trim().is_empty())))
}

/// `text` cleaned as `lyrics` say, then moved to the audio, without the
/// lines it times past the song's end; the damaged lines left out, and
/// those past the end, are named in `problems`.
pub(super) fn cleaned(text: &str, lyrics: &LyricText<'_>, problems: &mut Vec<String>) -> String {
    let cleaned = crate::clean::lyrics::clean(text, &lyrics.context(), lyrics.clean_off);
    if cleaned.damaged > 0 {
        problems.push(format!(
            "lyrics of {}: {} line(s) left out, opening with a time that reads as none",
            lyrics.key, cleaned.damaged
        ));
    }
    let moved = crate::lyrics::shift_lrc(&cleaned.text, lyrics.shift_ms, lyrics.stretch_ppm);
    let Some(end_ms) = lyrics.end_ms else {
        return moved;
    };
    let (kept, past) = crate::lyrics::until(&moved, end_ms);
    if past > 0 {
        problems.push(format!(
            "lyrics of {}: {past} line(s) left out, timed past the song's end",
            lyrics.key
        ));
    }
    kept
}

/// Plan the lyrics: a file's text now, or a subtitle stream the run
/// converts to `raw`, read once it ran (`None`).
pub(super) fn planned(
    lyrics: &LyricText<'_>,
    source: &Located,
    raw: &Path,
    inputs: &mut Inputs,
    outputs: &mut Vec<Output>,
    problems: &mut Vec<String>,
) -> Result<Option<String>> {
    if let LyricsAt::Stream { index } = lyrics.at {
        let n = inputs.index(&source.path);
        let mut args: Vec<OsString> = vec!["-map".into(), format!("{n}:{index}").into()];
        args.extend(LRC_ARGS.iter().map(OsString::from));
        outputs.push(Output::new(args, raw));
        return Ok(None);
    }
    let text = if *lyrics.at == LyricsAt::Tag {
        crate::tags::lyrics_in(&source.path)
            .ok_or_else(|| anyhow!("the lyrics in {} are gone", source.path.display()))?
            .0
    } else {
        let file = lyrics_file(lyrics.at, source)
            .ok_or_else(|| anyhow!("the lyrics beside {} are gone", source.path.display()))?;
        crate::lyrics::decode(
            &fs::read(&file).with_context(|| format!("reading {}", file.display()))?,
        )
    };
    Ok(Some(cleaned(&text, lyrics, problems)))
}

/// The file lyrics at `at` of `source` are read from; none for a stream
/// or a tag, or lyrics beside it that are gone.
#[must_use]
pub fn lyrics_file(at: &LyricsAt, source: &Located) -> Option<PathBuf> {
    match at {
        LyricsAt::Stream { .. } | LyricsAt::Tag => None,
        LyricsAt::Sidecar => source.lyrics.clone(),
        LyricsAt::File => Some(source.path.clone()),
    }
}
