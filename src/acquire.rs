//! Everything that reaches the network: yt-dlp fetching into the store,
//! the listing that says what a URL names, and the YouTube Music
//! lookups that pair an upload with its released track.
//!
//! Nothing here touches the song list or the library; each step returns
//! what it found as proposals for the songs they belong to.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::align::{self, Alignment};
use crate::download::{self, Fetched};
use crate::manifest::Manifest;
use crate::music::{self, Entry, Listing};
use crate::parallel;
use crate::provider::Provider;
use crate::runner::Runner;
use crate::settings::Ytdlp;
use crate::source::{SourceKey, watch_url};
use crate::state::{self, Failure, Looked, Outcome, Step};
use crate::store::Store;
use crate::ytdlp_log::Relay;

/// yt-dlp runs into the store, each with its own list of the files it
/// finished, one at a time, its lines relayed as they come.
#[derive(Debug)]
pub struct Fetcher<'a> {
    pub store: &'a Path,
    pub temp: &'a Path,
    /// Where yt-dlp keeps what it has not finished: beside the store, so
    /// a later run resumes it and the finished file is renamed in.
    pub partial: &'a Path,
    pub plugins: Option<&'a Path>,
    pub options: &'a Ytdlp,
    /// Whether output goes to a terminal, where progress is one line.
    pub live: bool,
    pub runs: Cell<usize>,
}

impl Fetcher<'_> {
    /// Whether yt-dlp succeeded, and the files it finished.
    pub fn fetch<R: Runner, W: Write>(
        &self,
        runner: &R,
        out: &mut W,
        template: &str,
        archive: Option<&Path>,
        urls: &[String],
    ) -> Result<(bool, Vec<Fetched>)> {
        let n = self.runs.get();
        self.runs.set(n + 1);
        let done = self.temp.join(format!("done-{n}"));
        // Well under Windows' 32,767-character command line, with room
        // for every other argument.
        let long = urls.iter().map(|u| u.len() + 1).sum::<usize>() > 16_000;
        let batch = self.temp.join(format!("urls-{n}"));
        if long {
            std::fs::write(&batch, urls.join("\n"))
                .with_context(|| format!("writing {}", batch.display()))?;
        }
        let places = download::Places {
            store: self.store,
            temp: self.partial,
            done: &done,
            plugins: self.plugins,
            options: self.options,
            batch: long.then_some(batch.as_path()),
        };
        // A video is one download; a playlist or channel, how many is not
        // known until yt-dlp lists it.
        let videos = urls.iter().all(|u| u.contains("watch?v="));
        let step = crate::progress::step(
            "Fetching",
            videos.then_some(urls.len() as u64).filter(|n| *n > 0),
        );
        let urls = if long { &[][..] } else { urls };
        let cmd = download::ytdlp_command(&places, template, archive, urls);
        let ok = {
            let live = self.live && !crate::progress::drawing();
            let mut relay = Relay::new(out, live).counting(&step);
            runner.stream(&cmd, &mut |line| relay.line(line))?
        };
        let fetched = std::fs::read_to_string(&done)
            .map(|s| download::finished(&s))
            .unwrap_or_default();
        Ok((ok, fetched))
    }
}

/// Sources that belong to one song, with what is known of its album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub sources: Vec<SourceKey>,
    pub album: Option<(SourceKey, u32)>,
    /// How to name it to the user before its tags are read.
    pub label: String,
}

/// What a network step found.
#[derive(Debug, Default)]
pub struct Additions {
    pub proposals: Vec<Proposal>,
    pub albums: Vec<(SourceKey, u32)>,
    /// Uploads a release is kept in place of, not as a source.
    pub replaced: Vec<(SourceKey, SourceKey)>,
    /// Keys a song lists that a release takes the place of.
    pub renames: Vec<(SourceKey, SourceKey)>,
    /// Each upload looked up on YouTube Music before it was fetched.
    pub looked_up: Vec<Looked>,
}

/// The envelope of a local file's audio.
pub(crate) fn local_envelope<R: Runner>(runner: &R, path: &Path) -> Result<Vec<f64>> {
    Ok(align::envelope(&runner.output(&align::pcm_command(path))?))
}

/// The envelope of a video's audio, downloaded alone into `dir`.
fn remote_envelope<R: Runner>(runner: &R, id: &str, dir: &Path) -> Result<Vec<f64>> {
    std::fs::create_dir_all(dir)?;
    let printed = runner.output(&align::audio_command(&watch_url(id), id, dir))?;
    let path = String::from_utf8_lossy(&printed)
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("yt-dlp named no audio file for {id}"))?;
    local_envelope(runner, &path)
}

/// The track a video is the same song as, if YouTube Music has one: a
/// flat search for the candidates, each extracted whole at once, then
/// the audio of each named one compared with the upload's until one is
/// the same recording. Where the audio cannot be had, a candidate of the
/// same length stands.
pub(crate) fn lookup<R: Runner>(
    runner: &R,
    video: &Entry,
    audio: &Path,
) -> Result<Option<(Entry, Option<Alignment>)>> {
    let Some(search) = music::search_command(video) else {
        return Ok(None);
    };
    let mut ids = music::ids(&runner.output(&search)?);
    // A release in a playlist finds itself.
    ids.retain(|id| *id != video.id);
    let candidates: Vec<Entry> = parallel::map(&ids, ids.len(), |id| {
        runner
            .output(&music::video_command(id))
            .map(|json| music::entries(&json))
            .unwrap_or_default()
    })
    .into_iter()
    .flatten()
    .collect();
    let named = music::named(video, &candidates);
    if named.is_empty() {
        return Ok(None);
    }
    let upload = remote_envelope(runner, &video.id, audio).ok();
    for track in named {
        let compared = upload.as_ref().and_then(|u| {
            let t = remote_envelope(runner, &track.id, audio).ok()?;
            align::align(u, &t)
        });
        match compared {
            Some(a) if a.fits() => return Ok(Some((track.clone(), Some(a)))),
            None if music::pick(video, std::slice::from_ref(track)).is_some() => {
                return Ok(Some((track.clone(), None)));
            }
            _ => {}
        }
    }
    Ok(None)
}

/// The upload of a track's recording with subtitles, if YouTube has one:
/// its artist and name searched, the candidates with subtitles kept,
/// and each compared with the track's audio until one is the same
/// recording.
pub(crate) fn reverse_lookup<R: Runner>(
    runner: &R,
    track: &Entry,
    track_audio: &[f64],
    audio: &Path,
) -> Result<Option<(Entry, Alignment)>> {
    let Some(search) = music::video_search_command(track) else {
        return Ok(None);
    };
    let found = music::entries(&runner.output(&search)?);
    let named: Vec<String> = music::uploads_of(track, &found)
        .into_iter()
        .map(|e| e.id.clone())
        .collect();
    let whole: Vec<Entry> = parallel::map(&named, named.len(), |id| {
        let json = runner.output(&music::video_command(id)).ok()?;
        music::entries(&json).into_iter().next()
    })
    .into_iter()
    .flatten()
    .filter(|e| e.has_subtitles() == Some(true))
    .collect();
    for upload in whole {
        let Ok(up) = remote_envelope(runner, &upload.id, audio) else {
            continue;
        };
        if let Some(a) = align::align(&up, track_audio).filter(Alignment::fits) {
            return Ok(Some((upload, a)));
        }
    }
    Ok(None)
}

/// What the URLs name: each as the download is to be given it, the
/// videos among them, each once, and the albums.
#[derive(Debug, Default)]
pub struct Listed {
    pub urls: Vec<String>,
    pub videos: Vec<Entry>,
    pub albums: Vec<(SourceKey, u32)>,
    /// Each album video's album and place on it.
    pub places: BTreeMap<String, (SourceKey, u32)>,
    /// Videos a URL named alone, not by a playlist.
    pub named: BTreeSet<String>,
}

/// List each URL once, yt-dlp deciding what it names. A playlist goes to
/// the download at its own address, since the download takes a watch URL
/// that also names a playlist as the one video; a Mix, which plays on
/// without end, goes as given, to be that one video. A URL that cannot be
/// listed goes as given too.
pub fn list<R: Runner, W: Write>(urls: &[String], runner: &R, out: &mut W) -> Result<Listed> {
    let found = parallel::map(urls, parallel::LOOKUPS, |url| {
        let listing = music::listing(&runner.output(&music::list_command(url))?)
            .ok_or_else(|| anyhow!("yt-dlp listed nothing"))?;
        let mix_video = match listing {
            Listing::Mix { .. } => runner
                .output(&music::one_video_command(url))
                .ok()
                .and_then(|json| music::entries(&json).into_iter().next()),
            _ => None,
        };
        Ok::<_, anyhow::Error>((listing, mix_video))
    });
    let mut listed = Listed::default();
    let mut seen = HashSet::new();
    for (url, found) in urls.iter().zip(found) {
        let videos = match found {
            Ok((Listing::Video(video), _)) => {
                listed.urls.push(url.clone());
                listed.named.insert(video.id.clone());
                vec![*video]
            }
            Ok((
                Listing::Playlist {
                    url: own,
                    title,
                    videos,
                    album,
                },
                _,
            )) => {
                let title = title.unwrap_or_else(|| url.clone());
                match album {
                    Some(album) => {
                        let key = SourceKey::playlist(&album.id);
                        crate::ui::info(
                            out,
                            &format!("{title}: an album of {} track(s)", album.tracks),
                        )?;
                        for (id, place) in album.places {
                            listed.places.insert(id, (key.clone(), place));
                        }
                        listed.albums.push((key, album.tracks));
                    }
                    None => crate::ui::info(out, &format!("{title}: {} video(s)", videos.len()))?,
                }
                listed.urls.push(own.unwrap_or_else(|| url.clone()));
                videos
            }
            Ok((Listing::Mix { title }, video)) => {
                let title = title.unwrap_or_else(|| url.clone());
                crate::ui::info(
                    out,
                    &format!("{title} is a Mix, without end: taking only its video"),
                )?;
                listed.urls.push(url.clone());
                video.into_iter().collect()
            }
            Err(e) => {
                crate::ui::warning(out, &format!("{url}: not listed: {e:#}"))?;
                listed.urls.push(url.clone());
                Vec::new()
            }
        };
        listed
            .videos
            .extend(videos.into_iter().filter(|v| seen.insert(v.id.clone())));
    }
    Ok(listed)
}

/// One run's network work, its runner, fetcher and output.
pub struct Acquire<'a, R, W> {
    pub runner: &'a R,
    pub fetcher: &'a Fetcher<'a>,
    pub out: &'a mut W,
    /// Every key already a source, removed, or an upload a release
    /// replaced.
    pub known: BTreeSet<SourceKey>,
    /// Keys of removed songs: named alone, one is listed again.
    pub removed: BTreeSet<SourceKey>,
    pub store: &'a Store,
}

impl<R, W> std::fmt::Debug for Acquire<'_, R, W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Acquire")
            .field("known", &self.known.len())
            .finish_non_exhaustive()
    }
}

impl<R: Runner, W: Write> Acquire<'_, R, W> {
    pub(crate) fn temp(&self) -> &Path {
        self.fetcher.temp
    }

    /// Fetch what the URLs name, each upload's YouTube Music track first
    /// when `music_match`. Returns whether every download succeeded.
    pub fn urls(&mut self, urls: &[String], music_match: bool) -> Result<(bool, Additions)> {
        let listed = list(urls, self.runner, self.out)?;
        for id in &listed.named {
            let key = SourceKey::youtube(id);
            if self.removed.contains(&key) {
                crate::ui::info(self.out, &format!("Listing {key} again, removed before"))?;
                self.known.remove(&key);
            }
        }
        let mut add = Additions {
            albums: listed.albums.clone(),
            ..Additions::default()
        };
        let place = |id: &str| listed.places.get(id).cloned();
        let mut ok = true;
        // A listed video already a source still joins its album.
        for video in &listed.videos {
            let key = SourceKey::youtube(&video.id);
            if self.known.contains(&key)
                && let Some(album) = place(&video.id)
            {
                add.proposals.push(Proposal {
                    sources: vec![key],
                    album: Some(album),
                    label: video.title.clone().unwrap_or_else(|| video.id.clone()),
                });
            }
        }
        if music_match {
            ok &= self.switch_to_tracks(&listed, &mut add)?;
        }
        let archive = self.temp().join("archive");
        let skip: Vec<SourceKey> = self.known.iter().cloned().collect();
        std::fs::write(&archive, download::archive_lines(&skip))?;
        let (fine, fetched) = self.fetcher.fetch(
            self.runner,
            self.out,
            download::OUTPUT_TEMPLATE,
            Some(&archive),
            &listed.urls,
        )?;
        if !fine {
            ok = false;
            crate::ui::warning(
                self.out,
                "yt-dlp reported a failure; adding what it finished",
            )?;
        }
        for f in fetched {
            if add.proposals.iter().any(|p| p.sources.contains(&f.key)) {
                continue;
            }
            let id = f.key.id().unwrap_or_default().to_string();
            add.proposals.push(Proposal {
                album: place(&id),
                label: label_of(&f.path),
                sources: vec![f.key],
            });
        }
        if add.proposals.is_empty() && ok {
            crate::ui::info(
                self.out,
                "Nothing new: every video named is a source already",
            )?;
        }
        Ok((ok, add))
    }

    /// Look each new upload up on YouTube Music and keep the track of each
    /// that has one, the upload beside it when the two are one recording
    /// and the upload has subtitles to give.
    fn switch_to_tracks(&mut self, listed: &Listed, add: &mut Additions) -> Result<bool> {
        let videos: Vec<&Entry> = listed
            .videos
            .iter()
            .filter(|v| {
                v.is_youtube()
                    && !self.known.contains(&SourceKey::youtube(&v.id))
                    && !v.is_track()
                    && v.duration.is_some()
            })
            .collect();
        if videos.is_empty() {
            return Ok(true);
        }
        crate::ui::info(
            self.out,
            &format!("Looking up {} video(s) on YouTube Music", videos.len()),
        )?;
        let audio = self.temp().join("audio");
        let runner = self.runner;
        // A folder each: two lookups downloading one candidate at once
        // would write one file.
        let found = parallel::map(&videos, parallel::LOOKUPS, |v| {
            lookup(runner, v, &audio.join(&v.id))
        });
        let mut ok = true;
        for (video, found) in videos.iter().zip(found) {
            let title = video.title.clone().unwrap_or_else(|| video.id.clone());
            let outcome = match &found {
                Ok(Some((track, _))) => Outcome::Found(SourceKey::youtube(&track.id)),
                Ok(None) => Outcome::Nothing,
                Err(e) => Outcome::Failed {
                    count: 1,
                    error: format!("{e:#}"),
                },
            };
            add.looked_up.push(Looked::now(
                SourceKey::youtube(&video.id),
                Provider::YouTubeMusic,
                outcome,
            ));
            let (track, alignment) = match found {
                Ok(Some(found)) => found,
                Ok(None) => continue,
                Err(e) => {
                    crate::ui::warning(self.out, &format!("{title}: search failed: {e:#}"))?;
                    continue;
                }
            };
            let (upload_key, track_key) =
                (SourceKey::youtube(&video.id), SourceKey::youtube(&track.id));
            crate::ui::info(
                self.out,
                &format!(
                    "{title}: keeping the YouTube Music track {}",
                    watch_url(&track.id)
                ),
            )?;
            let template = download::template_in(&video.folder());
            if !self.store.has(&track_key) && !self.fetch_one(&template, &track_key)? {
                ok = false;
                crate::ui::warning(self.out, "  the track failed; keeping the upload")?;
                continue;
            }
            self.known.insert(upload_key.clone());
            let fits = alignment.map_or_else(|| music::same_length(video, &track), |a| a.fits());
            let mut sources = vec![track_key.clone()];
            if fits && video.has_subtitles() != Some(false) {
                let shift = alignment.map_or(0, |a| a.offset_ms);
                crate::ui::info(
                    self.out,
                    &format!(
                        "  the same recording, {shift} ms apart: keeping the upload too, for its lyrics"
                    ),
                )?;
                if self.fetch_one(&template, &upload_key)? {
                    sources.push(upload_key);
                } else {
                    ok = false;
                    crate::ui::warning(
                        self.out,
                        "  the upload failed; the track has no lyrics for now",
                    )?;
                    add.replaced.push((upload_key, track_key));
                }
            } else {
                self.discard_upload(&upload_key)?;
                add.replaced.push((upload_key, track_key));
            }
            add.proposals.push(Proposal {
                sources,
                album: listed.places.get(&video.id).cloned(),
                label: title,
            });
        }
        Ok(ok)
    }

    /// Delete an upload a release takes the place of, fetched before by a
    /// run that listed it alone: no song lists it now.
    fn discard_upload(&mut self, upload: &SourceKey) -> Result<()> {
        let none = std::collections::BTreeSet::new();
        for file in self.store.discard(std::slice::from_ref(upload), &none)? {
            crate::ui::info(
                self.out,
                &format!("  deleted the upload, {}", file.display()),
            )?;
        }
        Ok(())
    }

    /// Fetch one source by its key's URL; whether it arrived.
    pub(crate) fn fetch_one(&mut self, template: &str, key: &SourceKey) -> Result<bool> {
        let Some(url) = key.url() else {
            return Ok(false);
        };
        let (ok, fetched) = self
            .fetcher
            .fetch(self.runner, self.out, template, None, &[url])?;
        Ok(ok && fetched.iter().any(|f| &f.key == key))
    }

    /// Fetch again every listed source the store no longer holds that
    /// can be fetched, the archive bypassed; one that failed lately waits
    /// unless `retry`. Returns whether all fetched arrived, and each that
    /// did not, with why.
    pub fn missing(
        &mut self,
        manifest: &Manifest,
        failures: &BTreeMap<SourceKey, Failure>,
        retry: bool,
    ) -> Result<(bool, Vec<(SourceKey, String)>)> {
        type Urls = Vec<(SourceKey, String)>;
        let now = state::now_secs();
        let (gone, waiting): (Urls, Urls) = manifest
            .keys()
            .into_iter()
            .filter(|k| !self.store.has(k))
            .filter_map(|k| Some((k.clone(), k.url()?)))
            .partition(|(k, _)| {
                retry
                    || failures
                        .get(k)
                        .filter(|f| f.step == Step::Fetch)
                        .is_none_or(|f| f.due(None, now))
            });
        if !waiting.is_empty() {
            crate::ui::warning(
                self.out,
                &format!(
                    "Not fetching {} missing source(s) that failed lately; `sync --retry` tries now",
                    waiting.len()
                ),
            )?;
        }
        if gone.is_empty() {
            return Ok((true, Vec::new()));
        }
        crate::ui::info(
            self.out,
            &format!("Fetching {} missing source(s) again", gone.len()),
        )?;
        let urls: Vec<String> = gone.iter().map(|(_, u)| u.clone()).collect();
        let (_, fetched) = self.fetcher.fetch(
            self.runner,
            self.out,
            download::OUTPUT_TEMPLATE,
            None,
            &urls,
        )?;
        let mut failed = Vec::new();
        for (key, _) in &gone {
            if !fetched.iter().any(|f| &f.key == key) {
                crate::ui::error(self.out, &format!("Could not fetch {key} again"))?;
                failed.push((key.clone(), "yt-dlp did not finish it".to_string()));
            }
        }
        Ok((failed.is_empty(), failed))
    }
}

/// A file's name without its ID and extension, to name it by.
fn label_of(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    match stem.rfind(" [") {
        Some(at) if stem.ends_with(']') => stem[..at].to_string(),
        _ => stem,
    }
}

#[cfg(test)]
mod tests;
