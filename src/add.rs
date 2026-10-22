//! `muman add`: what URLs name fetched and files copied into the
//! sources, each listed as a song or as a source of the song it is the
//! same recording as, the lyrics, pictures and tag files given to their
//! songs, then a sync.
//!
//! A magnet link or a `.torrent` file names a torrent, whose song files
//! and what they bring are fetched by aria2c ([`crate::aria2`]) and each
//! song file listed as a song is. Its metainfo is kept in the store, and
//! what fetches it again, its name, trackers and v2 hash, in the song
//! list's `[torrents]`; a private torrent's trackers are left out, since
//! their addresses hold a personal key. The torrents given are fetched
//! by one aria2c run, after one that asks the peers of every magnet link
//! given for its metainfo, so a torrent no peer seeds holds up no other.
//!
//! Thousands of files may be given at once, and each step over them is
//! taken once: every song file is probed before any is copied, as many
//! at a time as songs are built, each folder given is listed once for the
//! sidecars of all its files, and the store is read once after every
//! file is in it. Read after each file, the store's walk took longer than
//! measuring the files did.

use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::Job;
use crate::acquire::Proposal;
use crate::cli::{Matching, TagArgs};
use crate::dirs::Dirs;
use crate::given::{Folder, Walked};
use crate::history::Run;
use crate::http::HttpTransport;
use crate::manifest::{Edit, Manifest};
use crate::reconcile::Options;
use crate::runner::Runner;
use crate::source::{SourceKey, id_of};
use crate::store::{Kind, Store};
use crate::sync::{Listing, list, look_up, network, record};
use crate::ui::Prompter;
use crate::{atomic, attach, change, download, ffmpeg, info, probe, store, ui};

/// What `add` was given.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Request<'a> {
    pub inputs: &'a [String],
    pub to: &'a [String],
    pub no_match: bool,
    /// A folder each torrent given is taken up from.
    pub from: Option<&'a Path>,
    pub matching: Matching,
    pub tags: &'a TagArgs,
}

/// Run `add`; whether every step succeeded.
pub(crate) fn run<R: Runner, W: Write>(
    job: &Job,
    runner: &R,
    http: &(dyn HttpTransport + Sync),
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
    request: Request<'_>,
) -> Result<bool> {
    let dirs = &job.dirs;
    crate::create(&dirs.home)?;
    let mut run = Run::begin(&dirs.home)?;
    let mut left_out = 0;
    let mut unread = Vec::new();
    let mut refused = None;
    let done = (|| {
        let Inputs {
            torrents,
            files,
            urls,
        } = sort_inputs(request.inputs)?;
        let (loose, files) = split_loose(&files, &mut unread);
        let mut ok = true;
        if let Some(url) = urls.first().or(torrents.first()).filter(|_| job.offline) {
            return Err(change::Refused(format!(
                "{url} is fetched from the network, and --offline reaches none"
            ))
            .into());
        }
        let sites = Manifest::load(&dirs.home)?.settings.sites;
        let mut beside = Vec::new();
        let mut copied = Vec::new();
        let imported = import(
            runner,
            dirs,
            &sites,
            &files,
            (&mut beside, &mut copied),
            &mut unread,
            out,
        );
        for file in &copied {
            run.copied(&dirs.home, file);
        }
        let mut proposals = imported?;
        if !torrents.is_empty() {
            let (fine, found) = add_torrents(job, runner, &torrents, request.from, out)?;
            ok &= fine;
            proposals.extend(found);
        }
        if !urls.is_empty() {
            let urls: Vec<String> = urls.into_iter().cloned().collect();
            let (fine, additions) = network(job, runner, out, |acquire| {
                acquire.urls(&urls, !request.no_match)
            })?;
            ok &= fine;
            proposals.extend(record(dirs, additions)?);
        }
        let declined: BTreeSet<SourceKey> = if request.no_match {
            proposals.iter().flat_map(|p| p.sources.clone()).collect()
        } else {
            BTreeSet::new()
        };
        let how = Listing {
            mode: crate::mode(request.matching),
            verbose: job.verbose,
            tags: request.tags.tags(),
            loose: !loose.is_empty(),
        };
        let mut prompter = prompter;
        let listed = list(
            runner,
            dirs,
            proposals,
            &how,
            change::reborrow(&mut prompter),
            out,
        )?;
        let attaching = attach::How {
            yes: request.matching.yes,
            to: request.to.to_vec(),
            verbose: job.verbose,
        };
        // A loose file's refusal waits for the songs added with it
        // to be written.
        let attached =
            match attach::attach(runner, dirs, &loose, &beside, &attaching, prompter, out) {
                Ok(a) => a,
                Err(e) if e.downcast_ref::<change::Refused>().is_some() => {
                    refused = Some(e);
                    attach::Attached::default()
                }
                Err(e) => return Err(e),
            };
        left_out = attached.left_out;
        unread.extend(attached.unread);
        give_tags(dirs, &attached.songs, &how, listed, out)?;
        ok &= look_up(job, runner, http, false, &declined, out)?;
        ok &= crate::reconcile_now(job, runner, Options::default(), Some(&mut run), out)?;
        Ok(ok)
    })();
    let ok = crate::recorded(run, &dirs.home, done)?;
    if let Some(e) = refused {
        return Err(e);
    }
    not_added(&unread, left_out, request.matching.yes)?;
    Ok(ok)
}

/// What `add` was given, by how each is added.
struct Inputs<'a> {
    torrents: Vec<&'a String>,
    files: Vec<&'a String>,
    urls: Vec<&'a String>,
}

/// `inputs` sorted into torrents, files on disk and URLs, refused where a
/// file named is not there or a word was meant for `--to`.
fn sort_inputs(inputs: &[String]) -> Result<Inputs<'_>> {
    let (torrents, inputs): (Vec<&String>, Vec<&String>) =
        inputs.iter().partition(|i| names_a_torrent(i));
    let (files, urls): (Vec<&String>, Vec<&String>) =
        inputs.into_iter().partition(|i| Path::new(i).exists());
    let missing = torrents
        .iter()
        .find(|t| !is_magnet(t) && !Path::new(t).exists())
        .or_else(|| urls.iter().find(|u| names_a_path(u)));
    if let Some(missing) = missing {
        bail!("`{missing}`: no such file");
    }
    if let Some(word) = urls.iter().find(|u| is_a_word(u)) {
        bail!(
            "`{word}` is no file or URL; to name songs with it, put it inside \
             --to's quotes, as --to \"artist:venn {word}\""
        );
    }
    Ok(Inputs {
        torrents,
        files,
        urls,
    })
}

/// The error of files `add` could not read, or left out as matched to no
/// song for sure, once the songs added with them are written.
fn not_added(unread: &[String], left_out: usize, yes: bool) -> Result<()> {
    if !unread.is_empty() {
        bail!(
            "{} file(s) could not be added: {}",
            unread.len(),
            unread.join(", ")
        );
    }
    if left_out > 0 {
        let how = if yes {
            "name their songs with --to, or run on a terminal to pick them"
        } else {
            "name their songs with --to, give -y, or run on a terminal to pick them"
        };
        return Err(change::Refused(format!("{left_out} file(s) were not added: {how}")).into());
    }
    Ok(())
}

/// Set the tags given on the command line on every song a loose file was
/// given to, after the file's own, so the command line wins.
pub(crate) fn give_tags<W: Write>(
    dirs: &Dirs,
    songs: &[SourceKey],
    how: &Listing,
    listed: bool,
    out: &mut W,
) -> Result<()> {
    if how.tags.is_empty() || !how.loose {
        return Ok(());
    }
    if songs.is_empty() && !listed {
        return Ok(ui::warning(
            out,
            "No song was given a file, so no tag was set",
        )?);
    }
    let mut manifest = Manifest::load(&dirs.home)?;
    let mut seen = BTreeSet::new();
    for key in songs.iter().filter(|k| seen.insert(*k)) {
        manifest.edit(Edit::Tag {
            key: key.clone(),
            tags: how.tags.clone(),
        });
    }
    manifest.save()
}

/// The files of `files` `add` gives to songs rather than lists, each with
/// what it holds, and the rest. A loose file named as an audio file given
/// with it, in its folder, is that file's sidecar and copied with it. A
/// file muman does not read goes into `unread`, the rest added all the
/// same.
pub(crate) fn split_loose<'a>(
    files: &[&'a String],
    unread: &mut Vec<String>,
) -> (attach::Loose, Vec<&'a String>) {
    let mut loose = Vec::new();
    let mut rest = Vec::new();
    for file in files {
        match attach::sort(Path::new(file)) {
            Ok(Some(what)) => loose.push((PathBuf::from(file), what)),
            Ok(None) => rest.push(*file),
            Err(e) => unread.push(format!("{e:#}")),
        }
    }
    let canonical = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let audio: Vec<PathBuf> = rest
        .iter()
        .map(|f| PathBuf::from(f.as_str()))
        .filter(|p| p.is_file())
        .collect();
    let beside: Vec<PathBuf> = audio
        .iter()
        .flat_map(|a| sidecars(a))
        .map(|p| canonical(&p))
        .collect();
    loose.retain(|(path, _)| !beside.contains(&canonical(path)));
    (loose, rest)
}

/// Copy files and folders the user named into the sources: an original
/// yt-dlp fetched into the store under its key, anything else into the
/// manual folder with the lyrics and pictures named as it is; a folder's
/// files that are no song, lyrics or picture are left out and named,
/// since nothing would read them there. Never moves or overwrites; a
/// name already taken is an error. Each song file
/// copied is noted in `beside` with where it came from, for the loose
/// files given with it, and every file copied in `copied`, for `undo`,
/// those copied before an error too.
pub(crate) fn import<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    sites: &crate::sites::Sites,
    paths: &[&String],
    (beside, copied): (&mut Vec<(PathBuf, SourceKey)>, &mut Vec<PathBuf>),
    unread: &mut Vec<String>,
    out: &mut W,
) -> Result<Vec<Proposal>> {
    let Probed {
        mut walked,
        reads: probed,
    } = probe_songs(runner, paths)?;
    let mut audio = |file: &Path| {
        let reads = store::kind_of(file) != Some(store::Kind::Media)
            || probed
                .get(file)
                .copied()
                .unwrap_or_else(|| reads_as_audio(runner, file));
        if !reads {
            unread.push(format!("{}, which reads as no audio", file.display()));
        }
        reads
    };
    let mut proposals = Vec::new();
    let mut not_read = Vec::new();
    let mut originals_in = Originals {
        held: Store::scan(dirs)?,
        brought: BTreeSet::new(),
        runner,
    };
    let mut brought = Vec::new();
    let mut folders: HashMap<PathBuf, Walked> = HashMap::new();
    for path in paths.iter().map(Path::new) {
        let name = path
            .file_name()
            .with_context(|| format!("{} names no file", path.display()))?;
        if let Some(key) = original(runner, sites, path) {
            let given = (&Folder::new(path), path);
            originals_in.bring_into(dirs, given, key, (out, copied), &mut brought)?;
            continue;
        }
        let to = dirs.manual().join(name);
        if hidden(path) {
            not_read.push(format!(
                "{}, which is hidden, its name starting with a dot, and muman reads no hidden \
                 file; rename it without the dot",
                path.display()
            ));
            continue;
        }
        let mut originals = Vec::new();
        if path.is_dir() {
            let folder = Folder::new(path);
            let walked = match walked.remove(path) {
                Some(walked) => walked,
                None => folder.walk(usize::MAX)?,
            };
            let files = &walked.files;
            for file in files {
                if let Some(key) = original(runner, sites, file) {
                    originals.push((file.clone(), key));
                }
            }
            let mut own = |file: &Path| {
                store::kind_of(file).is_some()
                    && !originals.iter().any(|(o, _)| o == file)
                    && audio(file)
            };
            if originals.len() < files.len() {
                copy_tree(&folder, files, &to, &mut own, copied)?;
            }
            say_left_out(path, files, out)?;
            crate::given::say_outside(path, &walked.outside, out)?;
            for (file, key) in &originals {
                let given = (&folder, file.as_path());
                originals_in.bring_into(dirs, given, key.clone(), (out, copied), &mut brought)?;
            }
        } else if !audio(path) {
            continue;
        } else {
            copy_with_sidecars(path, &to, dirs, (&mut folders, copied), out)?;
        }
        brought.push(Brought::Manual(Manual {
            path: path.to_path_buf(),
            to,
            originals: !originals.is_empty(),
        }));
    }
    // The store is read once for every path, after all are copied: read
    // after each, thousands of files walked thousands of times took
    // longer than measuring them.
    let store = Store::scan(dirs)?;
    for b in brought {
        match b {
            Brought::Ready(proposal) => proposals.push(proposal),
            Brought::Manual(manual) => {
                proposals.extend(manual.propose(dirs, &store, (beside, &mut not_read), out)?);
            }
        }
    }
    unread.extend(not_read);
    Ok(proposals)
}

/// Copy the song file `path` to `to` with the sidecars beside it, its
/// folder listed once in `folders` for every file added from it; a
/// sidecar linked to outside the folder is left out and named.
fn copy_with_sidecars<W: Write>(
    path: &Path,
    to: &Path,
    dirs: &Dirs,
    (folders, copied): (&mut HashMap<PathBuf, Walked>, &mut Vec<PathBuf>),
    out: &mut W,
) -> Result<()> {
    copy_new(&Folder::new(path), path, to, copied)?;
    let folder = path.parent().unwrap_or(Path::new("")).to_path_buf();
    let within = Folder::new(&folder);
    let listed = folders
        .entry(folder.clone())
        .or_insert_with(|| files_in(&within));
    for sidecar in sidecars_among(path, &listed.files) {
        if let Some(n) = sidecar.file_name() {
            copy_new(&within, &sidecar, &dirs.manual().join(n), copied)?;
        }
    }
    let all: Vec<PathBuf> = listed
        .files
        .iter()
        .chain(&listed.outside)
        .cloned()
        .collect();
    let outside: Vec<PathBuf> = sidecars_among(path, &all)
        .into_iter()
        .filter(|s| listed.outside.contains(s))
        .collect();
    crate::given::say_outside(&folder, &outside, out)
}

/// Whether `path` is a file muman reads no song from, its name starting
/// with a dot.
fn hidden(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// Every song file among `paths`, and in the folders among them, probed
/// before any is copied, as many at a time as songs are built: a probe
/// costs as much as a copy, and thousands made one after another cost
/// minutes. The folders' files, as walked, and whether each song file
/// reads as audio.
fn probe_songs<R: Runner>(runner: &R, paths: &[&String]) -> Result<Probed> {
    let mut walked: HashMap<PathBuf, Walked> = HashMap::new();
    let mut media: Vec<PathBuf> = Vec::new();
    for path in paths.iter().map(Path::new) {
        if path.is_dir() {
            let found = Folder::new(path).walk(usize::MAX)?;
            media.extend(found.files.iter().filter(|f| id_of(f).is_none()).cloned());
            walked.insert(path.to_path_buf(), found);
        } else if !hidden(path) && id_of(path).is_none() {
            media.push(path.to_path_buf());
        }
    }
    media.retain(|f| store::kind_of(f) == Some(store::Kind::Media));
    let reads = crate::parallel::map(&media, crate::parallel::builds(), |f| {
        reads_as_audio(runner, f)
    });
    Ok(Probed {
        walked,
        reads: media.into_iter().zip(reads).collect(),
    })
}

/// What [`probe_songs`] found: each folder's files, as walked, and
/// whether each song file reads as audio.
struct Probed {
    walked: HashMap<PathBuf, Walked>,
    reads: HashMap<PathBuf, bool>,
}

/// A file or folder copied into the manual folder, at `to`, to be found
/// there once the store is read; `originals` when the folder held
/// originals, which are proposed on their own.
struct Manual {
    path: PathBuf,
    to: PathBuf,
    originals: bool,
}

impl Manual {
    /// The song files found at `to`, each a proposal and noted in
    /// `beside` with where it came from; a folder holding none is noted
    /// in `not_read`, unless its originals were all it held.
    fn propose<W: Write>(
        self,
        dirs: &Dirs,
        store: &Store,
        (beside, not_read): (&mut Vec<(PathBuf, SourceKey)>, &mut Vec<String>),
        out: &mut W,
    ) -> Result<Vec<Proposal>> {
        let found = media_at(store, dirs, &self.to)?;
        if found.is_empty() && self.path.is_dir() {
            if !self.originals {
                not_read.push(format!(
                    "{}, which holds no song muman reads",
                    self.path.display()
                ));
            }
            return Ok(Vec::new());
        }
        ui::info(
            out,
            &format!(
                "Copied {} into {}",
                self.path.display(),
                dirs.manual().display()
            ),
        )?;
        Ok(propose(&self.path, found, beside))
    }
}

/// The song files `found` under `path` as it was copied in, each a
/// proposal, and noted in `beside` with where it came from.
fn propose(
    path: &Path,
    found: Vec<SourceKey>,
    beside: &mut Vec<(PathBuf, SourceKey)>,
) -> Vec<Proposal> {
    let name = path.file_name().unwrap_or_default();
    found
        .into_iter()
        .map(|key| {
            if let SourceKey::Manual(m) = &key
                && let Some(folder) = path.parent()
            {
                let rel = m.path().strip_prefix(name).unwrap_or(m.path());
                let origin = if path.is_dir() {
                    path.join(rel)
                } else {
                    folder.join(m.path())
                };
                beside.push((origin, key.clone()));
            }
            Proposal {
                label: key.short(),
                sources: vec![key],
                album: None,
            }
        })
        .collect()
}

/// Warn of the files of the folder `dir` given to `add` that are left
/// out, as no song, lyrics or picture, each named below `dir`.
pub(crate) fn say_left_out<W: Write>(dir: &Path, files: &[PathBuf], out: &mut W) -> Result<()> {
    let other: Vec<String> = files
        .iter()
        .filter(|f| store::kind_of(f).is_none())
        .map(|f| f.strip_prefix(dir).unwrap_or(f).display().to_string())
        .collect();
    if !other.is_empty() {
        ui::warning(
            out,
            &format!(
                "Left out of {}, as no song, lyrics or picture: {}",
                dir.display(),
                other.join(", ")
            ),
        )?;
    }
    Ok(())
}

/// The originals one `add` copies into the store, and the store as it
/// was before.
pub(crate) struct Originals<'a, R> {
    held: Store,
    brought: BTreeSet<SourceKey>,
    runner: &'a R,
}

impl<R: Runner> Originals<'_, R> {
    /// [`Self::bring`], its proposal kept among `brought`.
    fn bring_into<W: Write>(
        &mut self,
        dirs: &Dirs,
        given: (&Folder, &Path),
        key: SourceKey,
        (out, copied): (&mut W, &mut Vec<PathBuf>),
        brought: &mut Vec<Brought>,
    ) -> Result<()> {
        brought.push(Brought::Ready(self.bring(
            dirs,
            given,
            key,
            (out, copied),
        )?));
        Ok(())
    }

    /// `file`, an original of `key`, copied to its key's place, whether
    /// given or in a folder given, the one `within`; its info JSON
    /// scrubbed as a fetch's is; one of a key the store holds already is
    /// proposed, not copied again.
    fn bring<W: Write>(
        &mut self,
        dirs: &Dirs,
        (within, file): (&Folder, &Path),
        key: SourceKey,
        (out, copied): (&mut W, &mut Vec<PathBuf>),
    ) -> Result<Proposal> {
        if self.held.has(&key) || self.brought.contains(&key) {
            ui::info(
                out,
                &format!(
                    "{} is {key}, which sources/yt-dlp holds already",
                    file.display()
                ),
            )?;
        } else {
            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("mkv");
            let to = store::fetched_path(&dirs.ytdlp(), &key, ext)
                .with_context(|| format!("{key} is fetched by no site"))?;
            copy_new(within, file, &to, copied)?;
            ui::info(
                out,
                &format!("Copied {} into sources/yt-dlp as {key}", file.display()),
            )?;
            let scrubbed =
                atomic::Scratch::new().and_then(|s| download::scrub(self.runner, &to, s.path()));
            if let Err(e) = scrubbed {
                ui::warning(
                    out,
                    &format!(
                        "{} keeps the cookies and headers it was fetched with: {e:#}",
                        to.display()
                    ),
                )?;
            }
            self.brought.insert(key.clone());
        }
        Ok(Proposal {
            label: key.short(),
            sources: vec![key],
            album: None,
        })
    }
}

/// The key of `file` if it is an original yt-dlp fetched: named as
/// yt-dlp names one, `<title> [<id>]`, holding the info JSON it embeds.
pub(crate) fn original<R: Runner>(
    runner: &R,
    sites: &crate::sites::Sites,
    file: &Path,
) -> Option<SourceKey> {
    let media = store::kind_of(file) == Some(store::Kind::Media);
    media.then(|| fetched_key(runner, sites, file)).flatten()
}

/// Whether `input` is a magnet link.
fn is_magnet(input: &str) -> bool {
    input
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("magnet:"))
}

/// Whether `input` names a torrent: a magnet link, or a `.torrent` file.
fn names_a_torrent(input: &str) -> bool {
    is_magnet(input)
        || Path::new(input)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("torrent"))
}

/// Fetch each torrent `inputs` names, its metainfo kept and recorded, and
/// propose each of its song files no song lists: whether every one was
/// fetched, and the proposals. Every magnet link's metainfo is asked for
/// in one aria2c run, and every torrent's files fetched in one more.
fn add_torrents<R: Runner, W: Write>(
    job: &Job,
    runner: &R,
    inputs: &[&String],
    from: Option<&Path>,
    out: &mut W,
) -> Result<(bool, Vec<Proposal>)> {
    let dirs = &job.dirs;
    let mut manifest = Manifest::load(&dirs.home)?;
    let options = manifest.settings.torrent.clone();
    let store_root = dirs.torrent();
    let partial = dirs.home.join("partial").join(crate::source::TORRENT);
    crate::sync::clear_stale(&partial, options.keep_partial, None);
    let places = crate::aria2::Places {
        store: &store_root,
        partial: &partial,
        options: &options,
    };
    let mut known = manifest.keys();
    known.extend(manifest.removed_keys());
    let (mut ok, metas) = read_all(runner, out, &places, &mut manifest, inputs)?;
    let on_disk = |meta, f| on_disk(places.store, meta, f);
    let mut wanted = Vec::new();
    for meta in &metas {
        let missing: Vec<&crate::torrent::File> = crate::torrent::selected(meta)
            .into_iter()
            .filter(|f| !on_disk(meta, f))
            .collect();
        if missing.is_empty() {
            continue;
        }
        if let Some(from) = from {
            let copied = crate::aria2::take_up(out, &places, meta, &missing, from)?;
            ui::info(
                out,
                &format!(
                    "Taking up {copied} file(s) of {} from {}, checked piece by piece",
                    meta.name,
                    from.display()
                ),
            )?;
        }
        let size =
            |files: &[&crate::torrent::File]| ui::bytes(files.iter().map(|f| f.length).sum());
        ui::info(
            out,
            &format!(
                "Fetching {} of {} file(s), {} of {}, from {}",
                missing.len(),
                meta.files.len(),
                size(&missing),
                size(&meta.files.iter().collect::<Vec<_>>()),
                meta.name
            ),
        )?;
        wanted.push(crate::aria2::Wanted {
            meta,
            files: missing,
        });
    }
    let mut failed = BTreeSet::new();
    let fetched = match stop_on_tool(crate::aria2::fetch_all(runner, out, &places, &wanted))? {
        Ok(fetched) => fetched,
        Err(e) => {
            ui::error(out, &format!("Fetching torrents: {e:#}"))?;
            failed.extend(wanted.iter().map(|w| w.meta.hash()));
            Vec::new()
        }
    };
    if fetched.is_empty() && !wanted.is_empty() {
        ok = false;
    }
    for (w, fetched) in wanted.iter().zip(fetched) {
        if let Some(failure) = fetched.failure {
            let again = if failure.resumes() {
                "; giving it to `add` again goes on"
            } else {
                ""
            };
            ui::error(out, &format!("{}: {failure}{again}", w.meta.name))?;
            if fetched.arrived.is_empty() {
                failed.insert(w.meta.hash());
            } else {
                ui::info(
                    out,
                    &format!(
                        "{}: taking the {} file(s) that arrived whole",
                        w.meta.name,
                        fetched.arrived.len()
                    ),
                )?;
            }
            ok = false;
        }
    }
    let fetched: Vec<&crate::torrent::Metainfo> = metas
        .iter()
        .filter(|m| !failed.contains(m.hash()))
        .collect();
    let proposals = propose_torrents(out, places.store, &fetched, &known)?;
    Ok((ok, proposals))
}

/// `result` as it is, but for an aria2c that cannot run, which stops a
/// run fetching torrents, as a missing yt-dlp stops one fetching videos.
fn stop_on_tool<T>(result: Result<T>) -> Result<Result<T>> {
    match result {
        Err(e) if e.downcast_ref::<crate::tools::ToolError>().is_some() => Err(e),
        other => Ok(other),
    }
}

/// Propose each song file of `metas` that is in the store, `root`, and
/// no song lists or removed, `known`.
fn propose_torrents<W: Write>(
    out: &mut W,
    root: &Path,
    metas: &[&crate::torrent::Metainfo],
    known: &BTreeSet<SourceKey>,
) -> Result<Vec<Proposal>> {
    let mut proposals = Vec::new();
    for meta in metas {
        let songs: Vec<(SourceKey, String)> = crate::torrent::selected(meta)
            .into_iter()
            .filter(|f| store::kind_of(&f.path) == Some(Kind::Media) && on_disk(root, meta, f))
            .map(|f| (meta.key(f), f.path.display().to_string()))
            .collect();
        for (lost, kept) in crate::torrent::shadowed(meta) {
            ui::warning(
                out,
                &format!(
                    "{}: {} is not fetched: some systems keep it at the place of {}",
                    meta.name,
                    lost.display(),
                    kept.display()
                ),
            )?;
        }
        if songs.is_empty() {
            ui::warning(out, &format!("{} holds no song file", meta.name))?;
        }
        proposals.extend(
            songs
                .into_iter()
                .filter(|(key, _)| !known.contains(key))
                .map(|(key, label)| Proposal {
                    sources: vec![key],
                    album: None,
                    label,
                }),
        );
    }
    Ok(proposals)
}

/// Whether the store, `root`, holds the file `f` of `meta`.
fn on_disk(root: &Path, meta: &crate::torrent::Metainfo, f: &crate::torrent::File) -> bool {
    match meta.key(f) {
        SourceKey::Torrent(k) => store::torrent_path(root, &k).is_file(),
        _ => false,
    }
}

/// Read each torrent `inputs` names, every magnet link's metainfo asked
/// of its peers in one run, and keep and record each, a torrent given
/// twice once: whether every one was read, and their metainfo.
fn read_all<R: Runner, W: Write>(
    runner: &R,
    out: &mut W,
    places: &crate::aria2::Places<'_>,
    manifest: &mut Manifest,
    inputs: &[&String],
) -> Result<(bool, Vec<crate::torrent::Metainfo>)> {
    let mut ok = true;
    let mut read = Vec::new();
    for input in inputs {
        match read_torrent(input) {
            Ok(Given::Magnet(m)) => {
                let hash = m.v1.as_deref().or(m.v2.as_deref()).unwrap_or_default();
                let kept = std::fs::read(crate::store::metainfo_path(places.store, hash))
                    .ok()
                    .filter(|b| crate::torrent::Metainfo::read(b).is_ok_and(|t| t.hash() == hash));
                let given = match kept {
                    Some(bytes) => Given::Kept(bytes, m.trackers),
                    None => Given::Magnet(m),
                };
                read.push((input.as_str(), given));
            }
            Ok(given) => read.push((input.as_str(), given)),
            Err(e) => {
                ui::error(out, &format!("{e:#}"))?;
                ok = false;
            }
        }
    }
    let magnets: Vec<&crate::torrent::Magnet> = read
        .iter()
        .filter_map(|(_, g)| match g {
            Given::Magnet(m) => Some(m),
            Given::File(_) | Given::Kept(..) => None,
        })
        .collect();
    let mut asked = stop_on_tool(crate::aria2::metainfos(runner, out, places, &magnets))?
        .unwrap_or_else(|e| magnets.iter().map(|_| Err(format!("{e:#}"))).collect())
        .into_iter();
    let mut metas = Vec::new();
    for (input, given) in read {
        let (bytes, trackers) = match given {
            Given::File(bytes) => (Ok(bytes), Vec::new()),
            Given::Kept(bytes, trackers) => (Ok(bytes), trackers),
            Given::Magnet(m) => (
                asked.next().unwrap_or_else(|| Err("aria2c failed".into())),
                m.trackers,
            ),
        };
        let kept = bytes
            .map_err(anyhow::Error::msg)
            .and_then(|b| keep_given(places, manifest, input, &b, trackers));
        match kept {
            Ok(meta)
                if metas
                    .iter()
                    .any(|m: &crate::torrent::Metainfo| m.hash() == meta.hash()) => {}
            Ok(meta) => metas.push(meta),
            Err(e) => {
                ui::error(out, &format!("{e:#}"))?;
                ok = false;
            }
        }
    }
    Ok((ok, metas))
}

/// A torrent as `add` was given it.
enum Given {
    /// A magnet link, whose metainfo its peers send.
    Magnet(crate::torrent::Magnet),
    /// The bytes of a `.torrent` file.
    File(Vec<u8>),
    /// A magnet link of a torrent whose metainfo the store keeps: its
    /// bytes, and the link's trackers.
    Kept(Vec<u8>, Vec<String>),
}

/// The torrent `input` names, read as far as it is without asking a peer.
fn read_torrent(input: &str) -> Result<Given> {
    if is_magnet(input) {
        return Ok(Given::Magnet(crate::torrent::Magnet::parse(input)?));
    }
    let bytes = std::fs::read(input).with_context(|| format!("reading {input}"))?;
    Ok(Given::File(bytes))
}

/// Read the metainfo `bytes` of the torrent `input` names, then keep and
/// record it.
fn keep_given(
    places: &crate::aria2::Places<'_>,
    manifest: &mut Manifest,
    input: &str,
    bytes: &[u8],
    trackers: Vec<String>,
) -> Result<crate::torrent::Metainfo> {
    let meta = crate::torrent::Metainfo::read(bytes).with_context(|| input.to_owned())?;
    if !meta.fetchable() {
        bail!(
            "{} is a v2 torrent alone, and no fetcher here reads v2 torrents",
            meta.name
        );
    }
    keep_torrent(places.store, manifest, &meta, bytes, trackers)?;
    Ok(meta)
}

/// Keep a torrent's metainfo in the store, `root`, and record it in the
/// song list with its trackers, those of the magnet link it came by too,
/// where it is not private.
fn keep_torrent(
    root: &Path,
    manifest: &mut Manifest,
    meta: &crate::torrent::Metainfo,
    bytes: &[u8],
    mut trackers: Vec<String>,
) -> Result<()> {
    let hash = meta.hash();
    crate::aria2::keep_metainfo(root, hash, bytes)?;
    trackers.extend(meta.trackers.iter().cloned());
    let mut seen = BTreeSet::new();
    trackers.retain(|t| seen.insert(t.clone()));
    manifest.edit(Edit::AddTorrent {
        hash: hash.to_string(),
        torrent: crate::manifest::Torrent {
            name: Some(meta.name.clone()),
            trackers: if meta.private { Vec::new() } else { trackers },
            v2: meta.v2.clone(),
        },
    });
    manifest.save()
}

/// Whether `input`, which names nothing on disk, was meant as a path
/// rather than an address for yt-dlp: written from a folder, or a file
/// name muman reads, with no scheme.
pub(crate) fn names_a_path(input: &str) -> bool {
    let from_folder = ["/", "./", "../", "~"].iter().any(|p| input.starts_with(p))
        || input.contains('\\')
        || input.as_bytes().get(1) == Some(&b':')
            && input
                .as_bytes()
                .get(2)
                .is_some_and(|b| *b == b'/' || *b == b'\\');
    let loose = Path::new(input)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| attach::LOOSE.contains(&e.to_ascii_lowercase().as_str()));
    let a_file = !input.contains("://") && (store::kind_of(Path::new(input)).is_some() || loose);
    !input.contains("://") && from_folder || a_file
}

/// Whether `input`, which names nothing on disk, is a plain word: no
/// address, path or video ID yt-dlp could read, as a query's word given
/// apart from `--to` is.
pub(crate) fn is_a_word(input: &str) -> bool {
    let video_id = input.len() == 11
        && input
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    !video_id && !input.contains(['.', '/', ':', '\\'])
}

/// The song files at or under `path` in the manual folder.
pub(crate) fn media_at(store: &Store, dirs: &Dirs, path: &Path) -> Result<Vec<SourceKey>> {
    let files = if path.is_dir() {
        crate::store::files_below(path, usize::MAX, |_| true)?
    } else {
        vec![path.to_path_buf()]
    };
    Ok(files
        .into_iter()
        .filter_map(|f| {
            Some(SourceKey::Manual(
                f.strip_prefix(dirs.manual()).ok()?.into(),
            ))
        })
        .filter(|k| store.locate(k).is_some_and(|l| l.kind == Kind::Media))
        .collect())
}

/// The key of an original yt-dlp fetched, from the info JSON inside it.
pub(crate) fn fetched_key<R: Runner>(
    runner: &R,
    sites: &crate::sites::Sites,
    path: &Path,
) -> Option<SourceKey> {
    crate::source::bracketed_id(path)?;
    let probed = probe::parse(&runner.output(&probe::ffprobe_command(path)).ok()?).ok()?;
    let attachment = probed.attachment_named("info.json")?;
    let temp = atomic::Scratch::new().ok()?;
    let json = temp.path().join("info.json");
    runner
        .run(&ffmpeg::dump_command(&[(
            path,
            vec![(attachment.ordinal, json.clone())],
        )]))
        .ok()?;
    let info = info::parse(&std::fs::read(json).ok()?).ok()?;
    info.key(sites)
}

/// What one path given to `add` brought into the store.
enum Brought {
    /// An original fetched before, proposed as it was copied.
    Ready(Proposal),
    /// Copied into the manual folder, to be found there.
    Manual(Manual),
}

/// The files in `folder`, in path order, and the links in it leading out
/// of it; a folder that does not read holds none.
fn files_in(folder: &Folder) -> Walked {
    folder.walk(1).unwrap_or_default()
}

/// The `.lrc` and pictures named as a file is, beside it, in any case
/// as the store takes them, but none named exactly as another song file
/// and none linked to outside its folder.
pub(crate) fn sidecars(path: &Path) -> Vec<PathBuf> {
    let folder = Folder::new(path.parent().unwrap_or(Path::new("")));
    sidecars_among(path, &files_in(&folder).files)
}

/// [`sidecars`], among `files`, those of the file's folder listed once
/// for every file added from it.
fn sidecars_among(path: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
    let beside: Vec<PathBuf> = files
        .iter()
        .filter(|p| p.as_path() != path)
        .cloned()
        .collect();
    let song = |p: &PathBuf| store::kind_of(p) == Some(store::Kind::Media);
    let another_s = |p: &PathBuf| {
        beside
            .iter()
            .any(|s| s != p && song(s) && s.with_extension("") == p.with_extension(""))
    };
    beside
        .iter()
        .filter(|p| {
            matches!(
                store::kind_of(p),
                Some(store::Kind::Lyrics | store::Kind::Image)
            ) && store::named_as(p, path)
                && (p.with_extension("") == path.with_extension("") || !another_s(p))
        })
        .cloned()
        .collect()
}

/// Copy `from`, a file of the file or folder given `within`, to `to`, a
/// name not yet taken.
pub(crate) fn copy_new(
    within: &Folder,
    from: &Path,
    to: &Path,
    copied: &mut Vec<PathBuf>,
) -> Result<()> {
    if to.exists() {
        bail!("{} exists already; rename one of them first", to.display());
    }
    if let Some(dir) = to.parent() {
        crate::create(dir)?;
    }
    within.copy(from, to)?;
    copied.push(to.to_path_buf());
    Ok(())
}

/// Copy the files of `files`, as `from` was walked, that `keep` lets
/// through to `to`.
pub(crate) fn copy_tree(
    from: &Folder,
    files: &[PathBuf],
    to: &Path,
    keep: &mut impl FnMut(&Path) -> bool,
    copied: &mut Vec<PathBuf>,
) -> Result<()> {
    if to.exists() {
        bail!("{} exists already; rename one of them first", to.display());
    }
    for file in files {
        if keep(file) {
            let rel = file.strip_prefix(from.path()).unwrap_or(file);
            copy_new(from, file, &to.join(rel), copied)?;
        }
    }
    Ok(())
}

/// Whether ffprobe reads an audio stream in `file`, so a file that holds
/// none, as an empty or cut-off download, is never listed as a song.
pub(crate) fn reads_as_audio<R: Runner>(runner: &R, file: &Path) -> bool {
    runner
        .output(&probe::ffprobe_command(file))
        .ok()
        .and_then(|o| probe::parse(&o).ok())
        .is_some_and(|p| p.audio.is_some())
}
