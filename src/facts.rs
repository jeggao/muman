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
//! separate output.
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

use crate::ffmpeg::{self, Output};
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
    format!("{}+{}", quality::METHOD, fingerprint::METHOD)
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
            url: info.key().map_or_else(
                || info.page().map(str::to_string),
                |key| crate::source::page_to_fetch(&key, info.page()),
            ),
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
            served: None,
            tags_digest: None,
            hash_method: HASH_METHOD.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioFacts {
    /// Absolute stream index.
    pub index: u32,
    pub codec: String,
    pub channels: u32,
    pub quality: Option<AudioQuality>,
    /// The bytes of the audio stream's packets, which a copy carries.
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

impl AudioFacts {
    /// Whether the codec keeps every sample as recorded.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.codec == "flac"
            || self.codec == "alac"
            || self.codec == "wavpack"
            || self.codec == "ape"
            || self.codec == "tta"
            || self.codec.starts_with("pcm_")
    }
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

/// Measure one located source, writing intermediates to `scratch`.
pub fn gather<R: Runner>(runner: &R, located: &Located, scratch: &Path) -> Result<Facts> {
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
        Kind::Media => media(runner, located, scratch, &mut facts)?,
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

    let mut inputs: Vec<PathBuf> = vec![located.path.clone()];
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
        for (n, start) in segment_starts(probed.duration).into_iter().enumerate() {
            let path = scratch.join(format!("segment{n}"));
            outputs.push((
                Output::new(quality::segment_output(0, a.index, start), &path),
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
        inputs.push(dumped);
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

    let input_refs: Vec<&Path> = inputs.iter().map(PathBuf::as_path).collect();
    let mut plain: Vec<Output> = outputs.iter().map(|(o, _)| o.clone()).collect();
    let mut results = ffmpeg::run_outputs(runner, &input_refs, &plain);
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
    let mut segments = Vec::new();
    let mut bytes = None;
    let mut packets_end = None;
    let mut digests = Digests::default();
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
            Measured::Segment(path) => segments.push(quality::read_segment(&path)),
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
        }
    }
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
        if quality::audio(&segments, a.sample_rate).is_none() {
            segments = measure_segments(runner, located, a.index, held, scratch);
        }
    }
    facts.audio = probed.audio.map(|a| AudioFacts {
        quality: quality::audio(&segments, a.sample_rate),
        index: a.index,
        codec: a.codec,
        channels: a.channels,
        bytes,
        digest: None,
    });
    facts.take(&digests, served);
    Ok(())
}

/// Seconds a file's packets may fall short of what its header says
/// before it counts as cut off: a container rounds its length.
const CUT_SLACK_S: f64 = 1.0;

/// The segments of a source's audio `held` seconds long, decoded again
/// where its header said it ran longer.
fn measure_segments<R: Runner>(
    runner: &R,
    located: &Located,
    index: u32,
    held: f64,
    scratch: &Path,
) -> Vec<Vec<[f32; 2]>> {
    let outputs: Vec<(Output, PathBuf)> = segment_starts(Some(held))
        .into_iter()
        .enumerate()
        .map(|(n, start)| {
            let path = scratch.join(format!("held{n}"));
            (
                Output::new(quality::segment_output(0, index, start), &path),
                path,
            )
        })
        .collect();
    let plain: Vec<Output> = outputs.iter().map(|(o, _)| o.clone()).collect();
    ffmpeg::run_outputs(runner, &[located.path.as_path()], &plain)
        .into_iter()
        .zip(outputs)
        .filter(|(result, _)| result.is_ok())
        .map(|(_, (_, path))| quality::read_segment(&path))
        .collect()
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
