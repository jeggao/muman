//! Everything a source offers a song, measured once per revision of its
//! files: its audio and how good it is, its pictures, its lyrics, its
//! tags and its fingerprint.
//!
//! The measuring run also lists the audio's packets, whose sizes sum to
//! what a copy takes, for `[library] max_size`; facts made before are
//! given theirs by [`audio_bytes`] when a limit asks.
//!
//! A media file costs at most three runs: ffprobe, one ffmpeg dumping
//! its attachments, and one ffmpeg writing every measured excerpt as a
//! separate output. That run decodes the whole audio once for its
//! fingerprint, and the same decode goes, as WAV on its stdout, to the
//! analyses [`crate::analysis`] runs over the whole of it. An analysis a
//! source's facts lack, or hold by another method, is caught up by one
//! run of its own, [`analyze`], measuring nothing else again; the Opus
//! header's gain, which ffprobe does not show, is read beside it.
//!
//! Each audio stream, picture and lyrics a source offers is also known
//! by a digest of its content, which plans name it by, so a source
//! fetched again or copied without its file times is the same source
//! when it holds the same content. A stream's digest is the SHA-256 of
//! its packets, copied, not decoded: two downloads of one file a minute
//! apart differed in 425 bytes of their container, where yt-dlp embeds
//! the time and its cookies, and in no packet. Packets survive a remux
//! too: Opus, AAC, FLAC and MP3 each hashed the same in their own
//! container and in Matroska, remuxed once or twice, and a FLAC with
//! new tags and a new cover hashed the same, where the decoded samples
//! of the Opus, AAC and MP3 differed by container, decoders being
//! required to agree only within a tolerance. An attachment, a sidecar
//! and a file of its own are hashed whole. The tags a source offers are
//! known by the fields of its info JSON they are read from, as
//! [`info::TAG_FIELDS`] lists them, the rest of it changing on every
//! fetch, or by a file's own tags, before any cleaning. Facts hashed by
//! another [`HASH_METHOD`] are hashed again by [`hash`], which decodes
//! nothing.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::analysis::{self, Analysis, Pass};
use crate::ffmpeg::{self, Input, Output, Piped};
use crate::fingerprint::{self, Print};
use crate::info::{self, VideoInfo};
use crate::lyrics::{self, Language, Timing};
use crate::probe::{self, Probed};
use crate::quality::{self, AudioQuality, ImageQuality};
use crate::runner::Runner;
use crate::store::{Kind, Located};
use crate::tags::{self, Offers};

/// The measures facts are made by; facts made by others are made again.
#[must_use]
pub fn method() -> String {
    format!(
        "{}+{}+{}",
        quality::METHOD,
        fingerprint::METHOD,
        probe::METHOD
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Facts {
    /// The revision of the files they were measured on.
    pub rev: String,
    pub method: String,
    pub duration: Option<f64>,
    pub audio: Option<AudioFacts>,
    pub covers: Vec<CoverFacts>,
    pub lyrics: Vec<LyricsFacts>,
    pub tags: Offers,
    /// The `tags::METHOD` the tags were read by.
    #[serde(default)]
    pub tags_method: String,
    /// A YouTube Music release, by its info: a track name, or a " - Topic"
    /// channel; read with the tags.
    #[serde(default)]
    pub release: bool,
    #[serde(default, deserialize_with = "fingerprint::read_stored")]
    pub print: Option<Print>,
    /// The length a cut-off file's header said, where its packets end
    /// sooner, at [`Facts::duration`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_from: Option<f64>,
    /// What the source holds that a library file cannot keep.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unkept: Vec<Unkept>,
    /// What a site served, for what yt-dlp fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub served: Option<Served>,
    /// The digest of the tags it offers as they are written in it, before
    /// muman reads them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags_digest: Option<String>,
    /// The [`HASH_METHOD`] each part was hashed by, or tried; empty for
    /// facts made before muman hashed sources.
    #[serde(default)]
    pub hash_method: String,
}

/// What a source's digests are of; facts hashed otherwise are hashed
/// again, decoding nothing, as [`hash`] does.
pub const HASH_METHOD: &str = "hash/2";

/// The format a site served a source in, by yt-dlp's info.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Served {
    /// yt-dlp's format ID, as `251` or `399+251`.
    pub format: String,
    /// The size the site gave its audio format, where it gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// The page it was fetched from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Served {
    fn of(info: &VideoInfo) -> Option<Self> {
        Some(Self {
            format: info.format_id.clone()?,
            size: info.audio_format().and_then(|f| f.filesize),
            url: info.page().map(str::to_string),
        })
    }
}

impl Facts {
    /// Whether these still describe a source at revision `rev`.
    #[must_use]
    pub fn holds_for(&self, rev: &str) -> bool {
        // Facts recorded before revisions held change times hold while
        // the sizes and modification times do, as they did then.
        let same = self.rev == rev || self.rev == crate::store::without_change_times(rev);
        same && self.method == method()
    }

    /// What a plan names its audio by: its digest, or, where it has
    /// none, the revision of its files.
    #[must_use]
    pub fn audio_rev(&self) -> String {
        self.audio
            .as_ref()
            .and_then(|a| a.digest.clone())
            .unwrap_or_else(|| self.rev.clone())
    }

    /// What a plan names `cover` by, as [`Facts::audio_rev`] its audio.
    #[must_use]
    pub fn cover_rev(&self, cover: &CoverFacts) -> String {
        cover.digest.clone().unwrap_or_else(|| self.rev.clone())
    }

    /// What a plan names `lyrics` by, as [`Facts::audio_rev`] its audio.
    #[must_use]
    pub fn lyrics_rev(&self, lyrics: &LyricsFacts) -> String {
        lyrics.digest.clone().unwrap_or_else(|| self.rev.clone())
    }

    /// Take `digests` and `served`, as [`hash`] read them.
    pub fn take(&mut self, digests: &Digests, served: Option<Served>) {
        if let Some(a) = &mut self.audio {
            a.digest.clone_from(&digests.audio);
        }
        for c in &mut self.covers {
            c.digest = digests.cover(&c.at);
        }
        for l in &mut self.lyrics {
            l.digest = digests.lyrics(&l.at);
        }
        self.tags_digest.clone_from(&digests.tags);
        self.served = served;
        self.hash_method = HASH_METHOD.to_string();
    }

    /// Whether each part was hashed as muman hashes them now.
    #[must_use]
    pub fn hashed(&self) -> bool {
        self.hash_method == HASH_METHOD
    }

    /// The analyses of `wanted` the audio lacks; none for no audio.
    #[must_use]
    pub fn unanalyzed(&self, wanted: &[analysis::Kind]) -> Vec<analysis::Kind> {
        self.audio
            .as_ref()
            .map(|a| a.analysis.stale(wanted))
            .unwrap_or_default()
    }

    /// Whether the tags were read the way they are now.
    #[must_use]
    pub fn tags_hold(&self) -> bool {
        self.tags_method == tags::METHOD
    }

    /// Facts of a file that could not be read at all.
    #[must_use]
    pub fn unreadable(rev: String) -> Self {
        Self {
            rev,
            method: method(),
            duration: None,
            audio: None,
            covers: Vec::new(),
            lyrics: Vec::new(),
            tags: Offers::new(),
            tags_method: tags::METHOD.to_string(),
            release: false,
            print: None,
            cut_from: None,
            unkept: Vec::new(),
            served: None,
            tags_digest: None,
            hash_method: HASH_METHOD.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioFacts {
    /// Absolute stream index.
    pub index: u32,
    pub codec: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub channels: u32,
    /// The speakers in ffmpeg's name; `None` when the file names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(default)]
    pub sample_rate: u32,
    /// Bits a decoded sample keeps; 0 when unknown.
    #[serde(default)]
    pub bits: u32,
    #[serde(default)]
    pub float: bool,
    pub quality: Option<AudioQuality>,
    /// The bytes of the audio stream's packets, which a copy carries.
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// What the whole of the audio holds.
    #[serde(default, skip_serializing_if = "is_unanalyzed")]
    pub analysis: Analysis,
}

fn is_unanalyzed(a: &Analysis) -> bool {
    *a == Analysis::default()
}

impl AudioFacts {
    /// Whether the codec keeps every sample as recorded.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        crate::codec::is_lossless_name(&self.codec, self.profile.as_deref())
    }

    /// What decides which codecs can hold this audio.
    #[must_use]
    pub fn shape(&self) -> crate::codec::Shape<'_> {
        crate::codec::Shape {
            channels: self.channels,
            layout: self.layout.as_deref(),
            sample_rate: self.sample_rate,
            bits: self.bits,
            float: self.float,
        }
    }
}

/// What a source holds that its library file does not keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unkept {
    /// Its cue sheet marks its audio pre-emphasized, as some early CDs
    /// were mastered: a player de-emphasizes it only by that flag, which
    /// is lost with the cue sheet, so it plays bright.
    PreEmphasis,
    /// A cue sheet of its name beside it, which splits an album image
    /// into its tracks; the image is one song.
    CueSheet,
}

/// Where a picture sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoverAt {
    /// A stream ffmpeg reads the picture from.
    Picture { index: u32 },
    /// An attachment, which must be dumped first.
    Attachment { ordinal: usize },
    /// The `n`th picture that came with a manual file.
    Sidecar(usize),
    /// The source is the picture.
    File,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverFacts {
    pub at: CoverAt,
    pub mimetype: String,
    pub quality: Option<ImageQuality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LyricsAt {
    /// A subtitle stream.
    Stream { index: u32 },
    /// The `.lrc` that came with a manual file.
    Sidecar,
    /// The source is the `.lrc`.
    File,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricsFacts {
    pub at: LyricsAt,
    pub language: Language,
    /// `None` for lyrics with no timed line.
    pub timing: Option<Timing>,
    /// The length of the recording an `.lrc` says it is timed to.
    #[serde(default)]
    pub stated_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

/// The digests of what one source offers, by where each part sits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Digests {
    pub audio: Option<String>,
    pub covers: Vec<(CoverAt, String)>,
    pub lyrics: Vec<(LyricsAt, String)>,
    pub tags: Option<String>,
}

impl Digests {
    fn cover(&self, at: &CoverAt) -> Option<String> {
        self.covers
            .iter()
            .find(|(a, _)| a == at)
            .map(|(_, d)| d.clone())
    }

    fn lyrics(&self, at: &LyricsAt) -> Option<String> {
        self.lyrics
            .iter()
            .find(|(a, _)| a == at)
            .map(|(_, d)| d.clone())
    }
}

/// The digest of `bytes`: the first 128 bits of their SHA-256 in hex,
/// which no two contents share by chance and a person can still read.
#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes)[..16])
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The digest an ffmpeg `hash` output wrote, as `SHA256=<hex>`, cut as
/// [`digest`] cuts its own.
fn read_hash(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let hex = text.trim().strip_prefix("SHA256=")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| hex[..32].to_ascii_lowercase())
}

/// [`digest`] of a file's bytes, read a block at a time.
pub(crate) fn digest_file(path: &Path) -> Option<String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path).ok()?, &mut hasher).ok()?;
    Some(hex(&hasher.finalize()[..16]))
}

/// The ffmpeg output options that hash the packets of stream
/// `input:index`, copied.
fn hash_output(input: usize, index: u32) -> Vec<std::ffi::OsString> {
    let map = format!("{input}:{index}");
    ["-map", &map, "-c", "copy", "-hash", "sha256", "-f", "hash"]
        .into_iter()
        .map(Into::into)
        .collect()
}

/// Measure one located source, writing intermediates to `scratch`, and
/// analyze its audio for each of `wanted`.
pub fn gather<R: Runner>(
    runner: &R,
    located: &Located,
    scratch: &Path,
    wanted: &[analysis::Kind],
) -> Result<Facts> {
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let mut facts = Facts::unreadable(located.rev());
    match located.kind {
        Kind::Lyrics => {
            let text = read_text(&located.path)?;
            facts.lyrics.push(LyricsFacts {
                digest: digest_file(&located.path),
                ..lyrics_of(LyricsAt::File, &text)
            });
        }
        Kind::Image => {
            let out = scratch.join("file.pgm");
            let outputs = [Output::new(quality::gray_output(0, "v:0"), &out)];
            let ok = ffmpeg::run_outputs(runner, &[located.path.as_path()], &outputs);
            facts.covers.push(CoverFacts {
                at: CoverAt::File,
                mimetype: mimetype_of(&located.path),
                quality: ok
                    .into_iter()
                    .next()
                    .and_then(Result::ok)
                    .and_then(|()| measure_image(&out)),
                digest: digest_file(&located.path),
            });
        }
        Kind::Tags => {
            facts.tags = tags::from_record(&crate::musicbrainz::read(&located.path)?);
            facts.tags_digest = digest_file(&located.path);
        }
        Kind::Media => media(runner, located, scratch, wanted, &mut facts)?,
    }
    Ok(facts)
}

/// The tags of one located source, read again: a media file's are
/// probed, its pictures not dumped nor anything measured.
pub fn retag<R: Runner>(runner: &R, located: &Located, scratch: &Path) -> Result<(Offers, bool)> {
    match located.kind {
        Kind::Media => {}
        Kind::Tags => {
            let record = crate::musicbrainz::read(&located.path)?;
            return Ok((tags::from_record(&record), false));
        }
        Kind::Lyrics | Kind::Image => return Ok((Offers::new(), false)),
    }
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let probed = probe::parse(&runner.output(&probe::ffprobe_command(&located.path))?)?;
    let info = dump_attachments(runner, located, &probed, scratch, false);
    let release = info.as_ref().is_some_and(VideoInfo::is_release);
    Ok((tags_of(info.as_ref(), &probed, &located.path), release))
}

/// The digest of the tags a media file offers as written: of the fields
/// of its info JSON, dumped into `scratch`, that tags are read from, for
/// what yt-dlp fetched; else of its own tags.
fn tags_digest(fetched: bool, probed: &Probed, path: &Path, scratch: &Path) -> Option<String> {
    if fetched {
        return info::tags_digest(&std::fs::read(scratch.join("info.json")).ok()?);
    }
    let own = tags::read_file(path).unwrap_or_else(|| {
        probed
            .tags
            .iter()
            .map(|(k, v)| (k.clone(), vec![v.clone()]))
            .collect()
    });
    Some(digest(&serde_json::to_vec(&own).ok()?))
}

/// What a media file's tags offer: its info JSON's, for what yt-dlp
/// fetched; else its own, as `lofty` reads them, or as ffprobe does for a
/// format `lofty` does not read.
fn tags_of(info: Option<&VideoInfo>, probed: &Probed, path: &Path) -> Offers {
    match info {
        Some(info) => tags::from_info(info),
        None => tags::from_container(&tags::read_file(path).unwrap_or_else(|| {
            probed
                .tags
                .iter()
                .map(|(k, v)| (k.clone(), vec![v.clone()]))
                .collect()
        })),
    }
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let text = crate::lyrics::decode(&bytes);
    if !crate::lyrics::is_text(&text) {
        anyhow::bail!("{} holds no text", path.display());
    }
    Ok(text)
}

fn lyrics_of(at: LyricsAt, text: &str) -> LyricsFacts {
    LyricsFacts {
        at,
        language: Language::Unstated,
        timing: lyrics::timing(&lyrics::clean_lrc(text)),
        stated_ms: lyrics::stated_length(text),
        digest: None,
    }
}

fn mimetype_of(path: &Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/unknown",
    }
    .to_string()
}

fn measure_image(pgm: &Path) -> Option<ImageQuality> {
    let bytes = std::fs::read(pgm).ok()?;
    let (w, h, pixels) = ffmpeg::read_pgm(&bytes)?;
    quality::image(pixels, w, h)
}

/// Where each measured segment of a recording `duration` long starts.
fn segment_starts(duration: Option<f64>) -> Vec<f64> {
    let Some(d) = duration.filter(|d| *d > 0.0) else {
        return vec![0.0];
    };
    if d <= quality::SEGMENT_SECONDS * 2.0 {
        return vec![0.0];
    }
    quality::SEGMENTS
        .iter()
        .map(|share| (d * share).min(d - quality::SEGMENT_SECONDS).max(0.0))
        .collect()
}

/// What one ffmpeg output measures, to read back once it ran.
enum Measured {
    OpusHead,
    Print,
    Packets,
    Segment(PathBuf),
    Subtitle(u32, Language),
    Cover(CoverAt, String),
    Hash(Part),
}

/// A part of a source a digest is of.
enum Part {
    Audio,
    Cover(CoverAt),
    Lyrics(LyricsAt),
}

/// The outputs hashing each stream of `probed` a plan may take: its
/// audio, its pictures and its subtitles.
fn hash_outputs(probed: &Probed, scratch: &Path) -> Vec<(Output, Part)> {
    let audio = probed.audio.iter().map(|a| (a.index, Part::Audio));
    let pictures = probed
        .pictures
        .iter()
        .map(|p| (p.index, Part::Cover(CoverAt::Picture { index: p.index })));
    let subtitles = probed
        .subtitles
        .iter()
        .map(|s| (s.index, Part::Lyrics(LyricsAt::Stream { index: s.index })));
    audio
        .chain(pictures)
        .chain(subtitles)
        .map(|(index, part)| {
            let path = scratch.join(format!("hash{index}"));
            (Output::new(hash_output(0, index), &path), part)
        })
        .collect()
}

/// Put the digest an output wrote under its part in `digests`.
fn note_hash(digests: &mut Digests, part: Part, path: &Path) {
    let Some(d) = read_hash(path) else {
        return;
    };
    match part {
        Part::Audio => digests.audio = Some(d),
        Part::Cover(at) => digests.covers.push((at, d)),
        Part::Lyrics(at) => digests.lyrics.push((at, d)),
    }
}

/// The digests of the files a media source brings whole: its image
/// attachments, dumped into `scratch`, and its sidecars.
fn file_digests(located: &Located, probed: &Probed, scratch: &Path, digests: &mut Digests) {
    for a in probed.image_attachments() {
        if let Some(d) = digest_file(&scratch.join(format!("att{}", a.ordinal))) {
            digests
                .covers
                .push((CoverAt::Attachment { ordinal: a.ordinal }, d));
        }
    }
    for (n, cover) in located.covers.iter().enumerate() {
        if let Some(d) = digest_file(cover) {
            digests.covers.push((CoverAt::Sidecar(n), d));
        }
    }
    if let Some(d) = located.lyrics.as_deref().and_then(digest_file) {
        digests.lyrics.push((LyricsAt::Sidecar, d));
    }
}

/// The digests of one located source and what a site served it in, by
/// one ffmpeg run that copies its streams and decodes nothing: for facts
/// measured before muman hashed sources.
pub fn hash<R: Runner>(
    runner: &R,
    located: &Located,
    scratch: &Path,
) -> Result<(Digests, Option<Served>)> {
    let mut digests = Digests::default();
    match located.kind {
        Kind::Lyrics => {
            if let Some(d) = digest_file(&located.path) {
                digests.lyrics.push((LyricsAt::File, d));
            }
            return Ok((digests, None));
        }
        Kind::Image => {
            if let Some(d) = digest_file(&located.path) {
                digests.covers.push((CoverAt::File, d));
            }
            return Ok((digests, None));
        }
        Kind::Tags => {
            digests.tags = digest_file(&located.path);
            return Ok((digests, None));
        }
        Kind::Media => {}
    }
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let probed = probe::parse(&runner.output(&probe::ffprobe_command(&located.path))?)?;
    let info = dump_attachments(runner, located, &probed, scratch, true);
    let outputs = hash_outputs(&probed, scratch);
    let plain: Vec<Output> = outputs.iter().map(|(o, _)| o.clone()).collect();
    let ran = ffmpeg::run_outputs(runner, &[located.path.as_path()], &plain);
    for ((output, part), result) in outputs.into_iter().zip(ran) {
        if result.is_ok() {
            note_hash(&mut digests, part, &output.path);
        }
    }
    file_digests(located, &probed, scratch, &mut digests);
    digests.tags = tags_digest(info.is_some(), &probed, &located.path, scratch);
    Ok((digests, info.as_ref().and_then(Served::of)))
}

#[allow(clippy::too_many_lines)]
fn media<R: Runner>(
    runner: &R,
    located: &Located,
    scratch: &Path,
    wanted: &[analysis::Kind],
    facts: &mut Facts,
) -> Result<()> {
    let probed = probe::parse(&runner.output(&probe::ffprobe_command(&located.path))?)?;
    facts.duration = probed.duration;
    let info = dump_attachments(runner, located, &probed, scratch, true);
    facts.tags = tags_of(info.as_ref(), &probed, &located.path);
    facts.release = info.as_ref().is_some_and(VideoInfo::is_release);
    let served = info.as_ref().and_then(Served::of);
    let tags = tags_digest(info.is_some(), &probed, &located.path, scratch);
    let info = info.unwrap_or_default();

    let mut inputs = vec![Input::from(located.path.as_path())];
    let mut outputs: Vec<(Output, Measured)> = hash_outputs(&probed, scratch)
        .into_iter()
        .map(|(o, part)| (o, Measured::Hash(part)))
        .collect();
    if let Some(a) = &probed.audio {
        outputs.push((
            Output::new(fingerprint::output(0, a.index), &scratch.join("print.pcm")),
            Measured::Print,
        ));
        outputs.push((
            Output::new(packets_output(0, a.index), &scratch.join("packets.crc")),
            Measured::Packets,
        ));
        if !wanted.is_empty() && a.codec == "opus" {
            outputs.push((
                Output::new(opus_head_output(0, a.index), &scratch.join("head.opus")),
                Measured::OpusHead,
            ));
        }
        for (n, start) in segment_starts(probed.duration).into_iter().enumerate() {
            let path = scratch.join(format!("segment{n}"));
            inputs.push(Input::new(quality::segment_input(start), &located.path));
            outputs.push((
                Output::new(quality::segment_output(inputs.len() - 1, a.index), &path),
                Measured::Segment(path),
            ));
        }
    }
    for sub in &probed.subtitles {
        if let Some(language) = lyrics::language(sub, &info) {
            let args = ["-map".to_string(), format!("0:{}", sub.index)];
            let mut args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
            args.extend(LRC_ARGS.iter().map(Into::into));
            outputs.push((
                Output::new(args, &scratch.join(format!("sub{}.lrc", sub.index))),
                Measured::Subtitle(sub.index, language),
            ));
        }
    }
    for p in &probed.pictures {
        outputs.push((
            Output::new(
                quality::gray_output(0, &p.index.to_string()),
                &scratch.join(format!("pic{}.pgm", p.index)),
            ),
            Measured::Cover(CoverAt::Picture { index: p.index }, p.mimetype.clone()),
        ));
    }
    for a in probed.image_attachments() {
        let dumped = scratch.join(format!("att{}", a.ordinal));
        if !dumped.exists() {
            continue;
        }
        inputs.push(Input::from(dumped.as_path()));
        outputs.push((
            Output::new(
                quality::gray_output(inputs.len() - 1, "v:0"),
                &scratch.join(format!("att{}.pgm", a.ordinal)),
            ),
            Measured::Cover(
                CoverAt::Attachment { ordinal: a.ordinal },
                a.mimetype.clone(),
            ),
        ));
    }

    let mut plain: Vec<Output> = outputs.iter().map(|(o, _)| o.clone()).collect();
    let mut pass = Pass::new(if probed.audio.is_some() {
        wanted.to_vec()
    } else {
        Vec::new()
    });
    let mut results = match probed.audio.as_ref().filter(|_| pass.wants()) {
        Some(a) => {
            let piped = Output::new(analysis_output(0, a.index, None), Path::new(ffmpeg::PIPE));
            ffmpeg::run_piped(runner, &inputs, &plain, &piped, &mut |p| pass.piped(&p)).0
        }
        None => ffmpeg::run(runner, &inputs, &plain),
    };
    // Each picture beside the file in a run of its own: one ffmpeg cannot
    // open, as an empty file, fails every run it is an input of.
    for (n, cover) in located.covers.iter().enumerate() {
        let output = Output::new(
            quality::gray_output(0, "v:0"),
            &scratch.join(format!("side{n}.pgm")),
        );
        results.extend(ffmpeg::run_outputs(
            runner,
            &[cover.as_path()],
            std::slice::from_ref(&output),
        ));
        plain.push(output.clone());
        outputs.push((
            output,
            Measured::Cover(CoverAt::Sidecar(n), mimetype_of(cover)),
        ));
    }
    let channels = probed.audio.as_ref().map_or(1, |a| a.channels as usize);
    let headroom = probed
        .audio
        .as_ref()
        .is_some_and(|a| a.float && a.is_lossless());
    let mut segments = Vec::new();
    let mut bytes = None;
    let mut packets_end = None;
    let mut digests = Digests::default();
    let mut opus_gain = 0;
    for ((output, measured), result) in outputs.into_iter().zip(results) {
        if result.is_err() {
            continue;
        }
        match measured {
            Measured::Print => {
                facts.print = runner.fingerprint(&output.path).ok().and_then(Print::new);
            }
            Measured::Packets => {
                let text = std::fs::read_to_string(&output.path).unwrap_or_default();
                bytes = packet_bytes(&text);
                packets_end = packet_seconds(&text);
            }
            Measured::Segment(path) => segments.push(quality::read_segment(&path, channels)),
            Measured::Subtitle(index, language) => {
                if let Ok(text) = read_text(&output.path) {
                    facts.lyrics.push(LyricsFacts {
                        at: LyricsAt::Stream { index },
                        language,
                        timing: lyrics::timing(&lyrics::clean_lrc(&text)),
                        stated_ms: None,
                        digest: None,
                    });
                }
            }
            Measured::Cover(at, mimetype) => facts.covers.push(CoverFacts {
                at,
                mimetype,
                quality: measure_image(&output.path),
                digest: None,
            }),
            Measured::Hash(part) => note_hash(&mut digests, part, &output.path),
            Measured::OpusHead => opus_gain = read_opus_gain(&output.path),
        }
    }
    let analysis = with_opus_gain(pass.finish(), opus_gain);
    file_digests(located, &probed, scratch, &mut digests);
    digests.tags = tags;
    // Lyrics beside a song that do not read cost the song only them.
    if let Some(text) = located.lyrics.as_deref().and_then(|l| read_text(l).ok()) {
        facts.lyrics.push(lyrics_of(LyricsAt::Sidecar, &text));
    }
    if let (Some(said), Some(held), Some(a)) = (facts.duration, packets_end, &probed.audio)
        && held + CUT_SLACK_S < said
    {
        facts.duration = Some(held);
        facts.cut_from = Some(said);
        if quality::audio(&segments, a.sample_rate, headroom).is_none() {
            segments = measure_segments(runner, located, a, held, scratch);
        }
    }
    facts.unkept = unkept(&located.path);
    facts.audio = probed.audio.map(|a| AudioFacts {
        quality: quality::audio(&segments, a.sample_rate, headroom),
        index: a.index,
        codec: a.codec,
        profile: a.profile,
        channels: a.channels,
        layout: a.layout,
        sample_rate: a.sample_rate,
        bits: a.bits,
        float: a.float,
        bytes,
        digest: None,
        analysis,
    });
    facts.take(&digests, served);
    Ok(())
}

/// What `path` holds, or has beside it, that a library file does not
/// keep.
fn unkept(path: &Path) -> Vec<Unkept> {
    let cue = path.with_extension("cue");
    let sheet = std::fs::read(&cue).ok().map(|b| crate::lyrics::decode(&b));
    let mut unkept = Vec::new();
    let flagged = sheet.as_deref().is_some_and(|text| {
        text.lines().any(|l| {
            let mut words = l.split_whitespace();
            words
                .next()
                .is_some_and(|w| w.eq_ignore_ascii_case("FLAGS"))
                && words.any(|w| w.eq_ignore_ascii_case("PRE"))
        })
    });
    if flagged || flac_emphasis(path) {
        unkept.push(Unkept::PreEmphasis);
    }
    if sheet.is_some() {
        unkept.push(Unkept::CueSheet);
    }
    unkept
}

/// Whether a FLAC file's `CUESHEET` block marks any track pre-emphasized.
fn flac_emphasis(path: &Path) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    // Metadata blocks come first and are small but for pictures, which
    // are skipped unread.
    let mut file = std::io::BufReader::new(file);
    let mut magic = [0; 4];
    if file.read_exact(&mut magic).is_err() || &magic != b"fLaC" {
        return false;
    }
    loop {
        let mut header = [0; 4];
        if file.read_exact(&mut header).is_err() {
            return false;
        }
        let last = header[0] & 0x80 != 0;
        let kind = header[0] & 0x7f;
        let length = u32::from_be_bytes([0, header[1], header[2], header[3]]);
        if kind == CUESHEET {
            let mut block = vec![0; length as usize];
            return file.read_exact(&mut block).is_ok() && cuesheet_emphasis(&block);
        }
        if last
            || std::io::copy(
                &mut (&mut file).take(u64::from(length)),
                &mut std::io::sink(),
            )
            .is_err()
        {
            return false;
        }
    }
}

const CUESHEET: u8 = 5;

/// Whether a `CUESHEET` block's tracks hold one flagged pre-emphasized.
fn cuesheet_emphasis(block: &[u8]) -> bool {
    // The catalog number, lead-in, flags and reserved bytes, then the
    // track count; each track its offset, number, ISRC, flags, reserved
    // bytes and index count, then its indices.
    const TRACKS_AT: usize = 128 + 8 + 1 + 258;
    let Some(&count) = block.get(TRACKS_AT) else {
        return false;
    };
    let mut at = TRACKS_AT + 1;
    for _ in 0..count {
        let (Some(&flags), Some(&indices)) = (block.get(at + 21), block.get(at + 35)) else {
            return false;
        };
        if flags & 0x40 != 0 {
            return true;
        }
        at += 36 + 12 * usize::from(indices);
    }
    false
}

/// Seconds a file's packets may fall short of what its header says
/// before it counts as cut off: a container rounds its length.
const CUT_SLACK_S: f64 = 1.0;

/// The segments of a source's audio `held` seconds long, decoded again
/// where its header said it ran longer.
fn measure_segments<R: Runner>(
    runner: &R,
    located: &Located,
    audio: &probe::Audio,
    held: f64,
    scratch: &Path,
) -> Vec<quality::Segment> {
    let starts = segment_starts(Some(held));
    let inputs: Vec<Input> = starts
        .iter()
        .map(|start| Input::new(quality::segment_input(*start), &located.path))
        .collect();
    let paths: Vec<PathBuf> = (0..starts.len())
        .map(|n| scratch.join(format!("held{n}")))
        .collect();
    let outputs: Vec<Output> = paths
        .iter()
        .enumerate()
        .map(|(n, path)| Output::new(quality::segment_output(n, audio.index), path))
        .collect();
    let channels = audio.channels as usize;
    ffmpeg::run(runner, &inputs, &outputs)
        .into_iter()
        .zip(paths)
        .filter(|(result, _)| result.is_ok())
        .map(|(_, path)| quality::read_segment(&path, channels))
        .collect()
}

impl Pass {
    fn piped(&mut self, piped: &Piped<'_>) {
        match piped {
            Piped::Bytes(b) => self.take(b),
            Piped::Again => self.again(),
        }
    }
}

/// The ffmpeg output options that decode the audio stream `input:index`
/// whole for [`crate::analysis`]: 32-bit float WAV at its own rate and
/// channels, or mixed through `filter` as a song is mixed.
fn analysis_output(input: usize, index: u32, filter: Option<String>) -> Vec<std::ffi::OsString> {
    let mut args = vec!["-map".to_string(), format!("{input}:{index}")];
    if let Some(f) = filter {
        args.extend(["-af".to_string(), f]);
    }
    args.extend(["-c:a", "pcm_f32le", "-f", "wav"].map(String::from));
    args.into_iter().map(Into::into).collect()
}

/// The ffmpeg output options that copy the first packet of the Opus stream
/// `input:index` into Ogg, for the gain its header holds.
fn opus_head_output(input: usize, index: u32) -> Vec<std::ffi::OsString> {
    let map = format!("{input}:{index}");
    ["-map", &map, "-c:a", "copy", "-frames:a", "1", "-f", "opus"]
        .into_iter()
        .map(Into::into)
        .collect()
}

fn read_opus_gain(path: &Path) -> i16 {
    std::fs::read(path)
        .ok()
        .and_then(|b| crate::ogg::output_gain(&b))
        .unwrap_or(0)
}

fn with_opus_gain(mut analysis: Analysis, gain: i16) -> Analysis {
    if let Some(m) = analysis.loudness.as_mut().and_then(|m| m.value.as_mut()) {
        m.opus_gain = gain;
    }
    analysis
}

/// The analyses of `wanted` of `audio`, a stream of `located`, by one run
/// of their own: for facts measured before they were asked for.
pub fn analyze<R: Runner>(
    runner: &R,
    located: &Located,
    audio: &AudioFacts,
    wanted: &[analysis::Kind],
    scratch: &Path,
) -> Result<Analysis> {
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let head = scratch.join("head.opus");
    let outputs = if audio.codec == "opus" {
        vec![Output::new(opus_head_output(0, audio.index), &head)]
    } else {
        Vec::new()
    };
    let piped = Output::new(
        analysis_output(0, audio.index, None),
        Path::new(ffmpeg::PIPE),
    );
    let mut pass = Pass::new(wanted.to_vec());
    let inputs = [Input::from(located.path.as_path())];
    let (results, _) =
        ffmpeg::run_piped(runner, &inputs, &outputs, &piped, &mut |p| pass.piped(&p));
    let gain = if results.first().is_some_and(Result::is_ok) {
        read_opus_gain(&head)
    } else {
        0
    };
    Ok(with_opus_gain(pass.finish(), gain))
}

/// The analyses of `wanted` of `audio` mixed through `filter`, as a song
/// mixed into another layout is written.
pub fn analyze_mix<R: Runner>(
    runner: &R,
    located: &Located,
    audio: &AudioFacts,
    filter: String,
    wanted: &[analysis::Kind],
) -> Analysis {
    let piped = Output::new(
        analysis_output(0, audio.index, Some(filter)),
        Path::new(ffmpeg::PIPE),
    );
    let mut pass = Pass::new(wanted.to_vec());
    let inputs = [Input::from(located.path.as_path())];
    let _ = ffmpeg::run_piped(runner, &inputs, &[], &piped, &mut |p| pass.piped(&p));
    pass.finish()
}

/// The ffmpeg output options that list every packet of the audio stream
/// `input:index`, copied, one line each with its size.
fn packets_output(input: usize, index: u32) -> Vec<std::ffi::OsString> {
    let map = format!("{input}:{index}");
    ["-map", &map, "-c:a", "copy", "-f", "framecrc"]
        .into_iter()
        .map(Into::into)
        .collect()
}

/// The sum of the packet sizes a `framecrc` list holds: the fifth field
/// of each line that is no `#` comment.
#[must_use]
pub fn packet_bytes(text: &str) -> Option<u64> {
    let sizes: Option<Vec<u64>> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split(',').nth(4)?.trim().parse().ok())
        .collect();
    sizes.filter(|s| !s.is_empty()).map(|s| s.iter().sum())
}

fn read_packets(path: &Path) -> Option<u64> {
    packet_bytes(&std::fs::read_to_string(path).ok()?)
}

/// Where the last packet a `framecrc` list holds ends, in seconds: the
/// length of the audio the file holds, whatever its header says.
#[must_use]
pub fn packet_seconds(text: &str) -> Option<f64> {
    let (num, den) = text
        .lines()
        .find_map(|l| l.strip_prefix("#tb 0:"))?
        .trim()
        .split_once('/')?;
    let (num, den): (f64, f64) = (num.trim().parse().ok()?, den.trim().parse().ok()?);
    let end = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let mut fields = l.split(',').skip(2).map(|f| f.trim().parse::<i64>().ok());
            Some(fields.next()?? + fields.next()??)
        })
        .max()?;
    #[allow(clippy::cast_precision_loss)]
    (den > 0.0).then(|| end as f64 * num / den)
}

/// The bytes of a located source's audio stream `index`, by one ffmpeg
/// run that copies its packets and decodes nothing.
pub fn audio_bytes<R: Runner>(
    runner: &R,
    located: &Located,
    index: u32,
    scratch: &Path,
) -> Result<u64> {
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let path = scratch.join("packets.crc");
    let outputs = [Output::new(packets_output(0, index), &path)];
    let ran = ffmpeg::run_outputs(runner, &[located.path.as_path()], &outputs);
    ran.into_iter().next().unwrap_or(Ok(()))?;
    read_packets(&path).with_context(|| format!("listing the packets of {}", located.key))
}

/// The options that write one subtitle stream as LRC. Without
/// `-map_metadata -1` the muxer writes every container tag as an LRC
/// header line, and bitexact drops its own version lines.
pub const LRC_ARGS: [&str; 6] = ["-map_metadata", "-1", "-fflags", "+bitexact", "-f", "lrc"];

/// Dump the info JSON, and the image attachments when `images`, into
/// `scratch`, and read the info JSON. A dump that fails costs those facts only.
fn dump_attachments<R: Runner>(
    runner: &R,
    located: &Located,
    probed: &Probed,
    scratch: &Path,
    images: bool,
) -> Option<VideoInfo> {
    let info_json = scratch.join("info.json");
    let mut dumps: Vec<(usize, PathBuf)> = probed
        .image_attachments()
        .filter(|_| images)
        .map(|a| (a.ordinal, scratch.join(format!("att{}", a.ordinal))))
        .collect();
    if let Some(a) = probed.attachment_named("info.json") {
        dumps.push((a.ordinal, info_json.clone()));
    }
    if dumps.is_empty() || probed.audio.is_none() {
        return None;
    }
    let _ = runner.run(&ffmpeg::dump_command(&[(located.path.as_path(), dumps)]));
    info::parse(&std::fs::read(&info_json).ok()?).ok()
}

#[cfg(test)]
mod tests;
