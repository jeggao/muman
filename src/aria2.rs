//! aria2c fetching a torrent's files into the store: its arguments, what
//! its lines say, and each finished file placed at its key's place.
//!
//! aria2c runs with `--no-conf`, so a personal `aria2.conf` cannot change
//! what is fetched or where; the song list's `[torrent]` is how options
//! reach it, `args` last so they win. Its DHT routing table is kept with
//! the unfinished fetches rather than in the user's own aria2 folder.
//!
//! A run fetches several torrents, as `add` fetches all it is given in
//! one and a sync all it fetches again, each a download of its own in the
//! run's input file (`--input-file`), with its own folder, files and GID;
//! what all share stays on the command line. One aria2c keeps one routing
//! table and listens on one port, which several would fight over, as a
//! fixed `--listen-port` in `args` would make them, and torrents no peer
//! seeds stall side by side, [`AT_ONCE`] at a time, rather than one after
//! another. aria2c's exit says only its last error, so how each torrent
//! ended is read from its row of the results, which names it by the first
//! six digits of its GID: the n-th download's GID begins with n. Its
//! error lines name a connection, not a torrent, so torrents failing side
//! by side are said to, with every error listed once.
//!
//! The input file reads a line break in a value as an option of its own
//! and a leading `#` as a comment, so a value holding a control
//! character, or a space at either end, is refused rather than handed
//! on, and a file is written under its index with its extension only
//! where that is letters and digits: a torrent names its files as it
//! likes, and a name that wrote a `dir=` line would write anywhere.
//!
//! A magnet link names a torrent without its metainfo, which aria2c asks
//! its peers for first (`--bt-metadata-only`), saving it beside the
//! unfinished fetches as `<v1 hash>.torrent`; muman then reads and keeps
//! it as a `.torrent` file given to `add` is kept. A link of a torrent
//! whose metainfo the store keeps already is read from there, no peer
//! asked.
//!
//! A fetch selects the files muman takes ([`crate::torrent::selected`])
//! by their place in the torrent, and names each `files/<place>.<ext>`
//! in a folder of the torrent's hash under `partial/torrent`
//! (`--index-out`), so aria2c never writes a path the torrent names,
//! which may not suit this system, and a fetch interrupted resumes at the
//! same paths, its pieces checked again (`--check-integrity`). Pieces
//! overlap the files next to a selected one, which aria2c writes in part
//! and removes once done. A file aria2c has not finished may have holes
//! where pieces came out of order, whatever its size, so a torrent the
//! run did not finish has placed only each file whose every piece checks
//! against its metainfo ([`crate::torrent::checks`]): one piece that
//! never does, as archive.org's torrents hold of a file changed since,
//! keeps the rest of the torrent from no one.
//!
//! Each second aria2c says how far each download has got, as
//! `20MiB/40MiB(50%)`: the bytes it has, of how many, and their share.
//! A download whose size is not yet known, as a magnet link's before its
//! metainfo arrives, says no share. One seeding, its files all fetched,
//! says its share ratio instead, uploaded over fetched (`SEED(0.0)`),
//! which muman says as 100%. A run of several says each one's share, or
//! the bytes it has where there is none. Each summary names every
//! download still running, and only those, so one a summary names and
//! the next does not has stopped, stalled or failed: it is said as
//! stopped rather than at the share it last had, unless that was all of
//! it, as a finished download's last readout is.
//!
//! A torrent already downloaded, as another client leaves it, is taken
//! up from its folder ([`take_up`]): each file muman selects, and each
//! file sharing a piece with one, which aria2c needs to check that piece
//! whole, is copied to the path the fetch writes it at, and aria2c checks
//! every piece and fetches only those missing or wrong. One complete and
//! right needs no peer.
//!
//! A listed file gone from the store is fetched again by `sync` (`due`,
//! [`fetch_due`]), beside yt-dlp's fetches, with the lyrics and pictures
//! it brings: from the torrent's metainfo where the store keeps it, else
//! from a magnet link of its hash and the trackers the song list records,
//! its metainfo asked of its peers and kept again. Files of the torrent
//! no song lists are not fetched again, so what `purge` deleted stays
//! deleted. One that failed lately waits, as a missing video does.
//!
//! A listed file found damaged, cut off or missing audio as a disk or a
//! copy leaves it, or holding other audio than its song was built from,
//! which a torrent's file never may, is fetched again as a damaged download is, at once and
//! then less often ([`crate::state::Failure::due`]). Its damaged copy, and
//! the files sharing a piece with it, are copied in as a download taken
//! up is, so aria2c fetches only the pieces that do not check; the whole
//! copy then takes the damaged one's place.
//!
//! A fetch with no byte arriving for `[torrent] stall` stops, aria2c
//! reporting it unfinished, as a torrent no peer seeds leaves it; it is
//! a failure a later sync tries again, or, for a torrent `add` was
//! given, that `add` resumes when given it again. aria2c reports each tracker it
//! cannot reach as an error, as a torrent's dead trackers make it while
//! others serve; these are said only when the fetch fails.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::manifest::Manifest;
use crate::runner::{Line, Runner};
use crate::settings::Torrent;
use crate::source::SourceKey;
use crate::torrent::{File, Magnet, Metainfo};

/// Where aria2c runs and how.
#[derive(Debug, Clone, Copy)]
pub struct Places<'a> {
    /// The torrents' folder in the store.
    pub store: &'a Path,
    /// Where unfinished fetches wait, a folder for each torrent.
    pub partial: &'a Path,
    pub options: &'a Torrent,
}

/// The most torrents one run fetches at once; the rest wait their turn.
/// Each opens connections to its own peers, and many at once can run
/// into a system's limit on open files.
pub const AT_ONCE: usize = 8;

/// How a readout of a download that seeds begins, its share ratio after.
const SEEDING: &str = "SEED(";

/// How each of aria2c's summaries begins.
const SUMMARY: &str = "*** Download Progress Summary";

/// How one download of a run ended, as its row of aria2c's results says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    Finished,
    /// Left unfinished, as a stall leaves it.
    Unfinished,
    Failed,
}

/// What one aria2c run said beyond its exit.
#[derive(Debug, Default)]
struct Said {
    /// Each download's end, by the six digits its row names its GID by.
    ended: BTreeMap<String, Ended>,
    errors: Vec<String>,
}

/// One download of a run, as aria2c's input file names it: what it
/// fetches, then the options it alone takes.
#[derive(Debug)]
struct Entry {
    uri: OsString,
    options: Vec<(&'static str, OsString)>,
}

/// The GID of the `n`-th download of a run, from 0, whose first six
/// digits, all aria2c's results name it by, are `n + 1`: all zeros is
/// no GID.
fn gid(n: usize) -> String {
    format!("{:06x}{:010x}", n + 1, 0)
}

/// What aria2c's results name the `n`-th download of a run by.
fn short_gid(n: usize) -> String {
    gid(n)[..6].to_string()
}

/// `value` as the bytes aria2c reads from its input file: a path as the
/// system spells it, in UTF-8 on Windows, as aria2c reads it there.
fn input_bytes(value: &OsStr) -> Result<&[u8]> {
    ensure!(
        !cfg!(windows) || value.to_str().is_some(),
        "{} is not Unicode, as aria2c reads it on Windows",
        value.to_string_lossy()
    );
    let bytes = value.as_encoded_bytes();
    let control = bytes.iter().any(u8::is_ascii_control);
    let padded = bytes.first().is_some_and(u8::is_ascii_whitespace)
        || bytes.last().is_some_and(u8::is_ascii_whitespace);
    ensure!(
        !control && !padded,
        "{:?} cannot be handed to aria2c: its input file would read it as more than it is",
        value.to_string_lossy()
    );
    Ok(bytes)
}

/// aria2c's input file of `entries`, each given the GID of its place.
fn input_file(entries: &[Entry]) -> Result<Vec<u8>> {
    let mut text = Vec::new();
    for (n, entry) in entries.iter().enumerate() {
        let uri = input_bytes(&entry.uri)?;
        ensure!(
            !uri.starts_with(b"#"),
            "{} cannot be handed to aria2c: its input file would read it as a comment",
            entry.uri.to_string_lossy()
        );
        text.extend(uri);
        text.push(b'\n');
        let gid = ("gid", OsString::from(gid(n)));
        for (name, value) in std::iter::once(&gid).chain(&entry.options) {
            text.extend(b"  ");
            text.extend(name.as_bytes());
            text.push(b'=');
            text.extend(input_bytes(value)?);
            text.push(b'\n');
        }
    }
    Ok(text)
}

/// The arguments every aria2c run of `downloads` takes.
fn common(places: &Places<'_>, downloads: usize) -> Vec<OsString> {
    let flag = |name: &str, value: &Path| {
        let mut arg = OsString::from(format!("--{name}="));
        arg.push(value);
        arg
    };
    let mut cmd: Vec<OsString> = vec![
        "aria2c".into(),
        "--no-conf=true".into(),
        "--console-log-level=warn".into(),
        "--summary-interval=1".into(),
        "--show-console-readout=false".into(),
        "--enable-color=false".into(),
        "--download-result=full".into(),
        "--auto-file-renaming=false".into(),
        "--allow-overwrite=true".into(),
        "--file-allocation=none".into(),
        // A muman killed outright, as no signal handler sees, takes it along.
        format!("--stop-with-process={}", std::process::id()).into(),
        format!("--bt-stop-timeout={}", places.options.stall.as_secs()).into(),
        format!(
            "--seed-time={}",
            places.options.seed_time.as_secs_f64() / 60.0
        )
        .into(),
        format!("--max-concurrent-downloads={}", downloads.clamp(1, AT_ONCE)).into(),
        flag("dht-file-path", &places.partial.join("dht.dat")),
        flag("dht-file-path6", &places.partial.join("dht6.dat")),
    ];
    cmd.extend(places.options.args.iter().map(OsString::from));
    cmd
}

/// The run that asks the peers of each of `links`, magnet links, for
/// its torrent's metainfo, reading them from `input`.
#[must_use]
pub fn metainfo_command(places: &Places<'_>, links: usize, input: &Path) -> Vec<OsString> {
    let mut cmd = common(places, links);
    let mut dir = OsString::from("--dir=");
    dir.push(places.partial);
    cmd.push(dir);
    cmd.push("--bt-metadata-only=true".into());
    cmd.push("--bt-save-metadata=true".into());
    cmd.push(input_arg(input));
    cmd
}

fn input_arg(input: &Path) -> OsString {
    let mut arg = OsString::from("--input-file=");
    arg.push(input);
    arg
}

/// Where a selected file is written while it is fetched, in its
/// torrent's folder: by its place, with its extension where that is
/// letters and digits alone, as a torrent may name its files anything.
#[must_use]
pub fn index_path(file: &File) -> PathBuf {
    let ext = file
        .path
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_alphanumeric()));
    let name = match ext {
        Some(ext) => format!("{}.{ext}", file.index),
        None => file.index.to_string(),
    };
    Path::new("files").join(name)
}

/// The download that fetches `files` of the torrent whose metainfo is
/// at `metainfo` into `dir`, the files of `beside` written where
/// [`take_up`] copies them, so the pieces they share with `files` check
/// whole.
fn fetch_entry(metainfo: &Path, dir: &Path, files: &[&File], beside: &[&File]) -> Entry {
    let select: Vec<String> = files.iter().map(|f| f.index.to_string()).collect();
    let mut options = vec![
        ("dir", dir.as_os_str().to_os_string()),
        ("select-file", select.join(",").into()),
    ];
    for file in files.iter().chain(beside) {
        let mut value = OsString::from(format!("{}=", file.index));
        value.push(index_path(file));
        options.push(("index-out", value));
    }
    Entry {
        uri: metainfo.as_os_str().to_os_string(),
        options,
    }
}

/// The run that fetches `downloads` torrents' files, reading them from
/// `input`.
#[must_use]
pub fn fetch_command(places: &Places<'_>, downloads: usize, input: &Path) -> Vec<OsString> {
    let mut cmd = common(places, downloads);
    cmd.extend([
        "--continue=true".into(),
        "--check-integrity=true".into(),
        "--bt-remove-unselected-file=true".into(),
    ]);
    cmd.push(input_arg(input));
    cmd
}

/// Run `cmd` over `entries`, written to an input file in a scratch
/// folder of the run's, which a run stopped partway leaves for the next
/// to clear, `labels` naming each on `step`: how each ended, and the
/// errors aria2c reported.
fn run<R: Runner>(
    runner: &R,
    places: &Places<'_>,
    cmd: impl FnOnce(&Path) -> Vec<OsString>,
    entries: &[Entry],
    labels: &[String],
    step: &crate::progress::Step,
) -> Result<(Vec<Ended>, Vec<String>)> {
    let text = input_file(entries)?;
    std::fs::create_dir_all(places.partial)
        .with_context(|| format!("creating {}", places.partial.display()))?;
    let scratch = crate::atomic::Scratch::new()?;
    let input = scratch.path().join("input.txt");
    std::fs::write(&input, &text).with_context(|| format!("writing {}", input.display()))?;
    let (ok, said) = stream(runner, &cmd(&input), step, labels)?;
    let ended = (0..entries.len())
        .map(|n| match said.ended.get(&short_gid(n)) {
            Some(ended) => *ended,
            None if ok => Ended::Finished,
            None => Ended::Failed,
        })
        .collect();
    Ok((ended, said.errors))
}

/// Run `cmd`, its progress noted on `step`, each download named by its
/// label in `labels`: whether it exited successfully, and what it said.
fn stream<R: Runner>(
    runner: &R,
    cmd: &[OsString],
    step: &crate::progress::Step,
    labels: &[String],
) -> Result<(bool, Said)> {
    let mut said = Said::default();
    let mut results = false;
    let mut readouts = Readouts::default();
    let ok = runner.stream(cmd, &mut |line| {
        let text = match line {
            Line::Out(t) | Line::Err(t) => t.trim(),
        };
        if text.starts_with("Download Results:") {
            results = true;
        } else if results && text.contains('|') {
            let mut fields = text.split('|').map(str::trim);
            let (Some(gid), Some(stat)) = (fields.next(), fields.next()) else {
                return;
            };
            let ended = match stat {
                "OK" => Ended::Finished,
                "INPR" => Ended::Unfinished,
                "ERR" | "RM" => Ended::Failed,
                _ => return,
            };
            said.ended.insert(gid.to_string(), ended);
        } else if readouts.read(text) {
            step.note(&format!("aria2c: {}", readouts.said(labels)));
        } else if let Some(error) = error_of(text)
            && !said.errors.contains(&error)
        {
            said.errors.push(error);
        }
    })?;
    Ok((ok, said))
}

/// What a run's summaries have said of its downloads, each by its short
/// GID.
#[derive(Debug, Default)]
struct Readouts {
    /// The last readout of each download a summary has named.
    last: BTreeMap<String, String>,
    /// The downloads the summary being read has named so far.
    named: BTreeSet<String>,
    /// The downloads a summary named and a later one did not.
    stopped: BTreeSet<String>,
}

impl Readouts {
    /// Read `line` of aria2c's: whether it changed what the run has got
    /// to.
    fn read(&mut self, line: &str) -> bool {
        if line.starts_with(SUMMARY) {
            let gone: Vec<String> = self
                .last
                .keys()
                .filter(|gid| !self.named.contains(*gid) && !self.stopped.contains(*gid))
                .cloned()
                .collect();
            self.named.clear();
            self.stopped.extend(gone.iter().cloned());
            return !gone.is_empty();
        }
        let Some((gid, readout)) = line.strip_prefix("[#").and_then(|r| r.split_once(' ')) else {
            return false;
        };
        self.last
            .insert(gid.to_string(), readout.trim_end_matches(']').to_string());
        self.named.insert(gid.to_string());
        self.stopped.remove(gid);
        true
    }

    /// What the readouts say of the run, `labels` naming each download:
    /// the one download's whole, or each one's share done by its label.
    fn said(&self, labels: &[String]) -> String {
        if labels.len() < 2 {
            return self
                .last
                .keys()
                .next()
                .map_or_else(String::new, |gid| self.of(gid, whole));
        }
        self.last
            .keys()
            .map(|gid| {
                let label = (0..labels.len())
                    .find(|&n| short_gid(n) == *gid)
                    .map_or(gid.as_str(), |n| labels[n].as_str());
                format!("{label} {}", self.of(gid, |r| share_done(r).to_string()))
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// What `say` says of the last readout of `gid`, or, of a download
    /// stopped short of all done, that it stopped.
    fn of(&self, gid: &str, say: impl Fn(&str) -> String) -> String {
        let readout = &self.last[gid];
        if self.stopped.contains(gid) && share_done(readout) != "100%" {
            return "stopped".to_string();
        }
        say(readout)
    }
}

/// How much of its download `readout` says is done, by its first field:
/// the share of a known size, all of it while seeding, else the bytes
/// arrived.
fn share_done(readout: &str) -> &str {
    let size = readout.split(' ').next().unwrap_or(readout);
    if size.starts_with(SEEDING) {
        return "100%";
    }
    size.split_once('(')
        .and_then(|(_, share)| share.strip_suffix(')'))
        .unwrap_or_else(|| size.split_once('/').map_or(size, |(done, _)| done))
}

/// `readout` as said of a run's one download, a seeding one as all done.
fn whole(readout: &str) -> String {
    match readout
        .strip_prefix(SEEDING)
        .and_then(|r| r.split_once(')'))
    {
        Some((_, rest)) => format!("100%{rest}"),
        None => readout.to_string(),
    }
}

/// What an error or warning line says, without aria2c's time and
/// connection number.
fn error_of(line: &str) -> Option<String> {
    let at = line.find("[ERROR]").or_else(|| line.find("[WARN]"))?;
    let rest = &line[at..];
    let rest = rest.split_once(']').map_or(rest, |(_, r)| r).trim();
    let rest = rest
        .strip_prefix("CUID#")
        .and_then(|r| r.split_once(" - "))
        .map_or(rest, |(_, r)| r);
    Some(rest.trim().to_string())
}

/// Why a fetch of a torrent's files did not bring them all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Nothing arrived for `[torrent] stall`; what did waits to resume.
    Stalled(std::time::Duration),
    /// aria2c ended it in an error: the one it reported, or, of several
    /// downloads failing at once, how many.
    Aria2c(Option<String>, usize),
    /// A torrent of `BitTorrent` v2 alone, which aria2c cannot fetch.
    V2Only,
    /// Files aria2c finished that the store could not take, and why.
    Unplaced(Vec<String>),
}

impl Failure {
    /// Whether giving the torrent to `add` again may bring the rest.
    #[must_use]
    pub fn resumes(&self) -> bool {
        matches!(self, Self::Stalled(_) | Self::Aria2c(..))
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stalled(stall) => write!(
                f,
                "no peer sent anything for {}; what arrived waits to be resumed",
                crate::units::Time(stall.as_millis() as i64).exact()
            ),
            Self::Aria2c(Some(e), 1) => write!(f, "aria2c: {e}"),
            Self::Aria2c(Some(_), failed) => {
                write!(
                    f,
                    "aria2c failed on {failed} torrents at once; its errors are above"
                )
            }
            Self::Aria2c(None, _) => write!(f, "aria2c failed"),
            Self::V2Only => write!(
                f,
                "a v2 torrent alone, and no fetcher here reads v2 torrents"
            ),
            Self::Unplaced(why) => write!(f, "{}", why.join("; ")),
        }
    }
}

/// Why a download that ended as `ended` failed, `errors` what the run
/// reported and `failed` how many of its downloads ended in an error.
fn failure(places: &Places<'_>, ended: Ended, errors: &[String], failed: usize) -> Failure {
    if ended == Ended::Unfinished {
        return Failure::Stalled(places.options.stall);
    }
    Failure::Aria2c(errors.last().cloned(), failed)
}

/// Say each of `errors` once, where any download of a run failed.
fn warn<W: Write>(out: &mut W, ended: &[Ended], errors: &[String]) -> Result<()> {
    if ended.iter().any(|e| *e != Ended::Finished) {
        for e in errors {
            crate::ui::warning(out, &format!("aria2c: {e}"))?;
        }
    }
    Ok(())
}

/// The metainfo of the torrent each of `magnets` names, from its peers,
/// in one run: its bytes, or why they did not arrive.
pub fn metainfos<R: Runner, W: Write>(
    runner: &R,
    out: &mut W,
    places: &Places<'_>,
    magnets: &[&Magnet],
) -> Result<Vec<Result<Vec<u8>, String>>> {
    let label = |m: &Magnet| m.name.clone().unwrap_or_else(|| m.hash().to_string());
    let mut asked: Vec<&Magnet> = Vec::new();
    for magnet in magnets {
        if magnet.v1.is_some() && !asked.iter().any(|a| a.v1 == magnet.v1) {
            asked.push(magnet);
        }
    }
    let mut ended = Vec::new();
    let mut errors = Vec::new();
    if !asked.is_empty() {
        let entries: Vec<Entry> = asked
            .iter()
            .map(|m| Entry {
                uri: m.link().into(),
                options: Vec::new(),
            })
            .collect();
        let labels: Vec<String> = asked.iter().map(|m| label(m)).collect();
        let step = crate::progress::step_beneath("Fetching metainfo", None);
        (ended, errors) = run(
            runner,
            places,
            |input| metainfo_command(places, entries.len(), input),
            &entries,
            &labels,
            &step,
        )?;
        warn(out, &ended, &errors)?;
    }
    let failed = ended.iter().filter(|e| **e == Ended::Failed).count();
    let found = magnets
        .iter()
        .map(|magnet| {
            let Some(v1) = &magnet.v1 else {
                return Err(format!(
                    "{} names a v2 torrent alone, and no fetcher here reads v2 torrents",
                    label(magnet)
                ));
            };
            let n = asked.iter().position(|a| a.v1.as_ref() == Some(v1));
            let ended = n.map_or(Ended::Failed, |n| ended[n]);
            let saved = places.partial.join(format!("{v1}.torrent"));
            let bytes = (ended == Ended::Finished)
                .then(|| std::fs::read(&saved).ok())
                .flatten();
            bytes.ok_or_else(|| {
                format!(
                    "the metainfo of {} did not arrive: {}",
                    label(magnet),
                    failure(places, ended, &errors, failed)
                )
            })
        })
        .collect();
    for v1 in asked.iter().filter_map(|m| m.v1.as_ref()) {
        let _ = std::fs::remove_file(places.partial.join(format!("{v1}.torrent")));
    }
    Ok(found)
}

/// The metainfo of the torrent `magnet` names, from its peers.
pub fn metainfo_of<R: Runner, W: Write>(
    runner: &R,
    out: &mut W,
    places: &Places<'_>,
    magnet: &Magnet,
) -> Result<Vec<u8>> {
    metainfos(runner, out, places, &[magnet])?
        .pop()
        .unwrap_or_else(|| Err("aria2c failed".to_string()))
        .map_err(anyhow::Error::msg)
}

/// What a fetch of a torrent's files brought.
#[derive(Debug, Default)]
pub struct Fetched {
    /// The key of each file placed in the store.
    pub arrived: Vec<SourceKey>,
    /// Why the fetch did not bring every file, where it did not.
    pub failure: Option<Failure>,
}

/// The files of one torrent a fetch asks for.
#[derive(Debug, Clone)]
pub struct Wanted<'a> {
    /// Its metainfo, which the store keeps.
    pub meta: &'a Metainfo,
    pub files: Vec<&'a File>,
}

/// Fetch the files each of `wanted` names, in one run, and place each
/// at its key's place: what each brought, in order. Each torrent is
/// named once.
pub fn fetch_all<R: Runner, W: Write>(
    runner: &R,
    out: &mut W,
    places: &Places<'_>,
    wanted: &[Wanted<'_>],
) -> Result<Vec<Fetched>> {
    let mut fetched: Vec<Fetched> = wanted.iter().map(|_| Fetched::default()).collect();
    let mut asked = Vec::new();
    for (n, w) in wanted.iter().enumerate() {
        ensure!(
            !wanted[..n].iter().any(|o| o.meta.hash() == w.meta.hash()),
            "{} is asked for twice in one fetch",
            w.meta.name
        );
        if !w.meta.fetchable() {
            fetched[n].failure = Some(Failure::V2Only);
        } else if !w.files.is_empty() {
            asked.push(n);
        }
    }
    if asked.is_empty() {
        return Ok(fetched);
    }
    let mut entries = Vec::new();
    for &n in &asked {
        let meta = wanted[n].meta;
        let dir = places.partial.join(meta.hash());
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let beside = crate::torrent::sharing(meta, &wanted[n].files);
        let metainfo = crate::store::metainfo_path(places.store, meta.hash());
        entries.push(fetch_entry(&metainfo, &dir, &wanted[n].files, &beside));
    }
    let labels: Vec<String> = asked.iter().map(|&n| wanted[n].meta.name.clone()).collect();
    let step = crate::progress::step_beneath("Fetching torrents", None);
    let (ended, errors) = run(
        runner,
        places,
        |input| fetch_command(places, entries.len(), input),
        &entries,
        &labels,
        &step,
    )?;
    drop(step);
    warn(out, &ended, &errors)?;
    let failed = ended.iter().filter(|e| **e == Ended::Failed).count();
    for (&n, ended) in asked.iter().zip(ended) {
        fetched[n] = if ended == Ended::Finished {
            place(out, places, &wanted[n])?
        } else {
            Fetched {
                arrived: place_checked(places, &wanted[n]),
                failure: Some(failure(places, ended, &errors, failed)),
            }
        };
    }
    Ok(fetched)
}

/// Place each file of `wanted` a finished fetch wrote at its key's
/// place.
fn place<W: Write>(out: &mut W, places: &Places<'_>, wanted: &Wanted<'_>) -> Result<Fetched> {
    let meta = wanted.meta;
    let dir = places.partial.join(meta.hash());
    let mut arrived = Vec::new();
    let mut unplaced = Vec::new();
    for file in &wanted.files {
        let from = dir.join(index_path(file));
        let SourceKey::Torrent(key) = meta.key(file) else {
            continue;
        };
        if !from.is_file() {
            crate::ui::warning(
                out,
                &format!("aria2c finished without {}", file.path.display()),
            )?;
            continue;
        }
        match crate::store::place_torrent(places.store, &key, &from) {
            Ok(_) => arrived.push(SourceKey::Torrent(key)),
            Err(e) => unplaced.push(format!("{e:#}")),
        }
    }
    // A file the store could not take stays to be placed by a later run.
    if unplaced.is_empty() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(Fetched {
        arrived,
        failure: (!unplaced.is_empty()).then_some(Failure::Unplaced(unplaced)),
    })
}

/// Place each file of `wanted` an unfinished fetch wrote whole, every
/// piece of it checking, at its key's place, the rest left to resume.
fn place_checked(places: &Places<'_>, wanted: &Wanted<'_>) -> Vec<SourceKey> {
    let meta = wanted.meta;
    let dir = places.partial.join(meta.hash());
    let at = |f: &File| dir.join(index_path(f));
    let whole: Vec<&File> = wanted
        .files
        .iter()
        .copied()
        .filter(|f| crate::torrent::checks(meta, f, at))
        .collect();
    let mut arrived = Vec::new();
    for file in whole {
        if let SourceKey::Torrent(key) = meta.key(file)
            && crate::store::place_torrent(places.store, &key, &at(file)).is_ok()
        {
            arrived.push(SourceKey::Torrent(key));
        }
    }
    arrived
}

/// Fetch `files` of `meta`, whose metainfo the store keeps, and place each
/// at its key's place.
pub fn fetch<R: Runner, W: Write>(
    runner: &R,
    out: &mut W,
    places: &Places<'_>,
    meta: &Metainfo,
    files: &[&File],
) -> Result<Fetched> {
    let wanted = Wanted {
        meta,
        files: files.to_vec(),
    };
    Ok(fetch_all(runner, out, places, &[wanted])?
        .pop()
        .unwrap_or_default())
}

/// Copy what `from`, a folder another client downloaded `meta` into,
/// holds of `files` and the files sharing a piece with them to where a
/// fetch writes each, for aria2c to check and finish: how many were
/// copied. `from` may be the torrent's own folder or the one holding it,
/// or the one file of a torrent of one.
pub fn take_up(
    places: &Places<'_>,
    meta: &Metainfo,
    files: &[&File],
    from: &Path,
) -> Result<usize> {
    let root = if from.join(&meta.name).is_dir() {
        from.join(&meta.name)
    } else {
        from.to_path_buf()
    };
    let dir = places.partial.join(meta.hash());
    let mut copied = 0;
    for file in files
        .iter()
        .copied()
        .chain(crate::torrent::sharing(meta, files))
    {
        let at = if root.is_file() {
            root.clone()
        } else {
            file.path.iter().fold(root.clone(), |p, part| p.join(part))
        };
        if std::fs::metadata(&at).map(|m| m.len()).ok() != Some(file.length) {
            continue;
        }
        let to = dir.join(index_path(file));
        std::fs::create_dir_all(to.parent().unwrap_or(&dir))
            .with_context(|| format!("creating {}", dir.display()))?;
        std::fs::copy(&at, &to)
            .with_context(|| format!("copying {} to {}", at.display(), to.display()))?;
        copied += 1;
    }
    Ok(copied)
}

/// Keep the metainfo `bytes` of the torrent of `hash` in the store's
/// torrents' folder, `root`, unless it is kept.
pub fn keep_metainfo(root: &Path, hash: &str, bytes: &[u8]) -> Result<()> {
    std::fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
    if !crate::store::metainfo_path(root, hash).exists() {
        crate::atomic::write(
            root,
            &crate::atomic::Name::new(format!("{hash}.torrent"))?,
            bytes,
        )?;
    }
    Ok(())
}

/// The listed torrent files the store lacks or holds damaged, by their
/// torrent's hash, each said; those that failed lately wait unless
/// `retry`, and are said as waiting.
pub(crate) fn due<W: Write>(
    out: &mut W,
    due: &crate::sync::Due,
) -> Result<BTreeMap<String, Vec<SourceKey>>> {
    let torrent = |k: &&SourceKey| matches!(k, SourceKey::Torrent(_));
    let waiting = due.waiting.iter().filter(torrent).count();
    if waiting > 0 {
        crate::ui::warning(
            out,
            &format!(
                "Not fetching {waiting} missing torrent file(s) that failed lately; `sync --retry` \
                 tries now"
            ),
        )?;
    }
    let mut by_hash: BTreeMap<String, Vec<SourceKey>> = BTreeMap::new();
    let (mut repaired, mut gone) = (0, 0);
    for (key, fetch) in &due.now {
        if let SourceKey::Torrent(t) = key {
            by_hash
                .entry(t.hash().to_string())
                .or_default()
                .push(key.clone());
            match fetch {
                crate::sync::Fetch::Damaged => repaired += 1,
                crate::sync::Fetch::Missing => gone += 1,
            }
        }
    }
    if repaired > 0 {
        crate::ui::info(
            out,
            &format!("Fetching {repaired} damaged torrent file(s) again"),
        )?;
    }
    if gone > 0 {
        crate::ui::info(
            out,
            &format!("Fetching {gone} missing torrent file(s) again"),
        )?;
    }
    Ok(by_hash)
}

/// Fetch the files `by_hash` names of each torrent again, with the
/// lyrics and pictures a listed song file of it brings: a run for the
/// metainfo the store lost, then one for every torrent's files. Each
/// key that did not arrive, with why.
pub fn fetch_due<R: Runner, W: Write>(
    runner: &R,
    out: &mut W,
    places: &Places<'_>,
    manifest: &Manifest,
    by_hash: &BTreeMap<String, Vec<SourceKey>>,
) -> Result<Vec<(SourceKey, String)>> {
    let mut failed = Vec::new();
    let lost: Vec<(&String, Magnet)> = by_hash
        .keys()
        .filter(|h| !crate::store::metainfo_path(places.store, h).is_file())
        .map(|h| (h, magnet_of(manifest, h)))
        .collect();
    let magnets: Vec<&Magnet> = lost.iter().map(|(_, m)| m).collect();
    let asked = metainfos(runner, out, places, &magnets)
        .unwrap_or_else(|e| magnets.iter().map(|_| Err(format!("{e:#}"))).collect());
    let mut metas = Vec::new();
    for (hash, keys) in by_hash {
        let found = lost.iter().position(|(h, _)| *h == hash).map(|n| &asked[n]);
        match read_kept(places, hash, found) {
            Ok(meta) => metas.push((meta, keys)),
            Err(e) => fail(out, &mut failed, keys, &format!("{e:#}"))?,
        }
    }
    let mut wanted = Vec::new();
    let mut asking = Vec::new();
    for (meta, keys) in &metas {
        let files = wanted_files(places, manifest, meta, keys);
        match repair_from_store(places, meta, &files) {
            Ok(()) => {
                wanted.push(Wanted { meta, files });
                asking.push(*keys);
            }
            Err(e) => fail(out, &mut failed, keys, &format!("{e:#}"))?,
        }
    }
    let fetched = match fetch_all(runner, out, places, &wanted) {
        Ok(fetched) => fetched,
        Err(e) => {
            let keys: Vec<SourceKey> = asking.into_iter().flatten().cloned().collect();
            fail(out, &mut failed, &keys, &format!("{e:#}"))?;
            return Ok(failed);
        }
    };
    for (keys, fetched) in asking.into_iter().zip(fetched) {
        if let Some(why) = fetched.failure {
            let lost: Vec<SourceKey> = keys
                .iter()
                .filter(|k| !fetched.arrived.contains(k))
                .cloned()
                .collect();
            fail(out, &mut failed, &lost, &why.to_string())?;
            continue;
        }
        for key in keys.iter().filter(|k| !fetched.arrived.contains(k)) {
            failed.push((key.clone(), "aria2c did not finish it".to_string()));
        }
    }
    Ok(failed)
}

/// Say each of `keys` could not be fetched again, for `why`, and add it
/// to `failed`.
fn fail<W: Write>(
    out: &mut W,
    failed: &mut Vec<(SourceKey, String)>,
    keys: &[SourceKey],
    why: &str,
) -> Result<()> {
    for key in keys {
        crate::ui::error(out, &format!("Could not fetch {key} again: {why}"))?;
        failed.push((key.clone(), why.to_string()));
    }
    Ok(())
}

/// A magnet link of the torrent of `hash`, with what the song list
/// records of it.
fn magnet_of(manifest: &Manifest, hash: &str) -> Magnet {
    let recorded = manifest.torrents.get(hash).cloned().unwrap_or_default();
    Magnet {
        v1: (hash.len() == crate::torrent::V1_HEX).then(|| hash.to_string()),
        v2: recorded
            .v2
            .clone()
            .or_else(|| (hash.len() == crate::torrent::V2_HEX).then(|| hash.to_string())),
        name: recorded.name.clone(),
        trackers: recorded.trackers,
    }
}

/// The metainfo of the torrent of `hash`: as the store keeps it, else as
/// `asked` of its peers, kept again.
fn read_kept(
    places: &Places<'_>,
    hash: &str,
    asked: Option<&Result<Vec<u8>, String>>,
) -> Result<Metainfo> {
    let kept = crate::store::metainfo_path(places.store, hash);
    let bytes = match asked {
        None => std::fs::read(&kept).with_context(|| format!("reading {}", kept.display()))?,
        Some(Ok(bytes)) => {
            keep_metainfo(places.store, hash, bytes)?;
            bytes.clone()
        }
        Some(Err(why)) => {
            bail!("its metainfo is gone from the store; give its .torrent to `add` again: {why}")
        }
    };
    let meta = Metainfo::read(&bytes)?;
    ensure!(
        meta.hash() == hash,
        "{} is the metainfo of another torrent",
        kept.display()
    );
    Ok(meta)
}

/// The files of `meta` to fetch for `keys`: each, and the lyrics and
/// pictures a listed song file of it brings that the store lacks.
fn wanted_files<'m>(
    places: &Places<'_>,
    manifest: &Manifest,
    meta: &'m Metainfo,
    keys: &[SourceKey],
) -> Vec<&'m File> {
    let hash = meta.hash();
    let listed: Vec<&Path> = manifest
        .keys()
        .into_iter()
        .filter_map(|k| match k {
            SourceKey::Torrent(t) if t.hash() == hash => Some(
                meta.files
                    .iter()
                    .find(|f| crate::relpath::normalized(&f.path) == t.path())?
                    .path
                    .as_path(),
            ),
            _ => None,
        })
        .collect();
    crate::torrent::selected(meta)
        .into_iter()
        .filter(|f| {
            let key = meta.key(f);
            if keys.contains(&key) {
                return true;
            }
            let SourceKey::Torrent(t) = &key else {
                return false;
            };
            crate::store::kind_of(&f.path) != Some(crate::store::Kind::Media)
                && listed.iter().any(|s| f.path.parent() == s.parent())
                && !crate::store::torrent_path(places.store, t).is_file()
        })
        .collect()
}

/// Copy what the store holds of `files`, a damaged copy among them, and
/// of the files sharing a piece with them, to where a fetch writes each,
/// so aria2c fetches only the pieces that do not check.
fn repair_from_store(places: &Places<'_>, meta: &Metainfo, files: &[&File]) -> Result<()> {
    let dir = places.partial.join(meta.hash());
    for file in files
        .iter()
        .copied()
        .chain(crate::torrent::sharing(meta, files))
    {
        let SourceKey::Torrent(key) = meta.key(file) else {
            continue;
        };
        let at = crate::store::torrent_path(places.store, &key);
        let to = dir.join(index_path(file));
        if !at.is_file() || to.exists() {
            continue;
        }
        std::fs::create_dir_all(to.parent().unwrap_or(&dir))
            .with_context(|| format!("creating {}", dir.display()))?;
        std::fs::copy(&at, &to)
            .with_context(|| format!("copying {} to {}", at.display(), to.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Torrent {
        Torrent {
            args: vec!["--listen-port=51413".into()],
            ..Torrent::default()
        }
    }

    fn text(cmd: &[OsString]) -> Vec<String> {
        cmd.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn file(parts: &[&str], index: usize) -> File {
        File {
            path: parts.iter().collect(),
            length: 3,
            root: None,
            index,
            offset: 3 * (index as u64 - 1),
        }
    }

    #[test]
    fn a_fetch_selects_its_files_and_names_each_by_its_place() {
        let options = options();
        let places = Places {
            store: Path::new("store"),
            partial: Path::new("partial"),
            options: &options,
        };
        let files = [
            file(&["01 Paper Comets.flac"], 1),
            file(&["CD 2", "cover"], 4),
        ];
        let entry = fetch_entry(
            Path::new("store/h.torrent"),
            Path::new("partial/h"),
            &files.iter().collect::<Vec<_>>(),
            &[],
        );
        let input = String::from_utf8(input_file(&[entry]).unwrap()).unwrap();
        let out = |i: &str, p: &[&str]| {
            format!(
                "  index-out={i}={}",
                p.iter().collect::<PathBuf>().display()
            )
        };
        let dir = format!("  dir={}", Path::new("partial/h").display());
        let lines: Vec<&str> = input.lines().collect();
        assert_eq!(
            lines[..4],
            [
                "store/h.torrent",
                "  gid=0000010000000000",
                &dir,
                "  select-file=1,4"
            ]
        );
        assert_eq!(
            lines[4..],
            [out("1", &["files", "1.flac"]), out("4", &["files", "4"])]
        );

        let cmd = text(&fetch_command(&places, 1, Path::new("partial/input.txt")));
        assert_eq!(cmd[0], "aria2c");
        assert!(cmd.contains(&"--no-conf=true".to_string()));
        assert!(cmd.contains(&"--seed-time=0".to_string()));
        assert!(cmd.contains(&"--bt-stop-timeout=600".to_string()));
        assert!(cmd.contains(&"--max-concurrent-downloads=1".to_string()));
        assert_eq!(cmd.last().unwrap(), "--input-file=partial/input.txt");
        let args = cmd.iter().position(|a| a == "--listen-port=51413").unwrap();
        assert!(args > cmd.iter().position(|a| a == "--no-conf=true").unwrap());
    }

    #[test]
    fn a_run_fetches_at_most_eight_torrents_at_once_unless_args_say_otherwise() {
        let options = options();
        let places = Places {
            store: Path::new("store"),
            partial: Path::new("partial"),
            options: &options,
        };
        let cmd = text(&fetch_command(&places, 20, Path::new("i")));
        let at_once = cmd
            .iter()
            .position(|a| a == "--max-concurrent-downloads=8")
            .unwrap();
        let args = cmd.iter().position(|a| a == "--listen-port=51413").unwrap();
        assert!(at_once < args, "[torrent] args come last, and win");
    }

    #[test]
    fn nothing_a_torrent_names_is_read_by_aria2c_as_an_option_of_its_own() {
        let sly = file(&["01 Paper Comets.flac\n  dir=elsewhere"], 1);
        assert_eq!(index_path(&sly), Path::new("files").join("1"));
        let entry = |dir: &str| Entry {
            uri: "store/h.torrent".into(),
            options: vec![("dir", dir.into())],
        };
        for dir in [
            "partial/h\n  index-out=1=/elsewhere",
            "partial/h\r",
            " partial/h",
        ] {
            assert!(input_file(&[entry(dir)]).is_err(), "{dir:?}");
        }
        assert!(input_file(&[entry("partial/h")]).is_ok());
        let comment = Entry {
            uri: "#h.torrent".into(),
            options: Vec::new(),
        };
        assert!(input_file(&[comment]).is_err());
    }

    #[test]
    fn each_download_of_a_run_ends_as_its_row_of_the_results_says() {
        let lines = [
            "10/20 10:50:27 [ERROR] CUID#12 - Download aborted. URI=http://tracker.example/a",
            "[#000001 1.2MiB/40MiB(3%) CN:4 SD:2 DL:300KiB ETA:2m]",
            "Download Results:",
            "gid   |stat|avg speed  |  %|path/URI",
            "======+====+===========+===+=====",
            "000001|INPR|       0B/s|  3|partial/h/files/1.flac",
            "   |    |           |  0|partial/h/files/2.png",
            "000002| OK |   1.2MiB/s|100|partial/g/files/1.flac",
            "000003|ERR |       0B/s|  0|partial/f/files/1.flac",
        ];
        let runner = crate::testing::Fake {
            lines: lines.iter().map(ToString::to_string).collect(),
            on_stream: Some(Box::new(|_: &[String]| false)),
            ..crate::testing::Fake::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let options = options();
        let places = Places {
            store: dir.path(),
            partial: dir.path(),
            options: &options,
        };
        let entries: Vec<Entry> = ["h", "g", "f", "e"]
            .iter()
            .map(|h| Entry {
                uri: format!("{h}.torrent").into(),
                options: Vec::new(),
            })
            .collect();
        let labels: Vec<String> = ["Lantern Weather", "Rooms of Salt", "Paper Comets", "Glass"]
            .map(String::from)
            .to_vec();
        let step = crate::progress::step("Fetching", None);
        let (ended, errors) = run(
            &runner,
            &places,
            |input| fetch_command(&places, 4, input),
            &entries,
            &labels,
            &step,
        )
        .unwrap();
        assert_eq!(
            ended,
            [
                Ended::Unfinished,
                Ended::Finished,
                Ended::Failed,
                Ended::Failed
            ],
            "a download the results do not name failed with the run"
        );
        assert_eq!(errors, ["Download aborted. URI=http://tracker.example/a"]);
        let said =
            |n: usize, failed: usize| failure(&places, ended[n], &errors, failed).to_string();
        assert!(said(0, 2).starts_with("no peer sent anything for 10 min"));
        assert_eq!(
            said(2, 1),
            "aria2c: Download aborted. URI=http://tracker.example/a"
        );
        assert!(said(2, 2).contains("failed on 2 torrents at once"));
        assert!(failure(&places, ended[0], &errors, 2).resumes());
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "the input file goes with the run"
        );
    }

    /// The readouts of `lines`, aria2c's, read in turn.
    fn readouts(lines: &[&str]) -> Readouts {
        let mut readouts = Readouts::default();
        for line in lines {
            readouts.read(line);
        }
        readouts
    }

    const AT: &str = "*** Download Progress Summary as of Tue Oct 20 10:50:27 2026 ***";

    #[test]
    fn a_runs_progress_names_each_torrent_by_its_share_done() {
        let labels = ["Lantern Weather".to_string(), "Rooms of Salt".to_string()];
        let readouts = readouts(&[
            AT,
            "[#000001 1.2MiB/40MiB(3%) CN:4]",
            "[#000002 20MiB/40MiB(50%) CN:2]",
        ]);
        assert_eq!(
            readouts.said(&labels),
            "Lantern Weather 3%, Rooms of Salt 50%"
        );
        assert_eq!(readouts.said(&labels[..1]), "1.2MiB/40MiB(3%) CN:4");
    }

    #[test]
    fn a_finished_torrent_seeding_reads_as_all_done_not_as_its_ratio() {
        let labels = ["Lantern Weather".to_string(), "Rooms of Salt".to_string()];
        let readouts = readouts(&[
            AT,
            "[#000001 SEED(0.0) CN:0 SD:0]",
            "[#000002 20MiB/40MiB(50%) CN:2 SD:1 DL:1MiB ETA:20s]",
        ]);
        assert_eq!(
            readouts.said(&labels),
            "Lantern Weather 100%, Rooms of Salt 50%"
        );
        assert_eq!(readouts.said(&labels[..1]), "100% CN:0 SD:0");
    }

    #[test]
    fn a_torrent_no_summary_names_any_more_reads_as_stopped_not_where_it_was() {
        // What aria2c 1.37.0 wrote of a torrent no peer seeds stalling beside
        // one fetched, a summary a second, its FILE: lines left out.
        let lines = [
            AT,
            "[#000001 0B/2.9KiB(0%) CN:0 SD:0 DL:0B]",
            "[#000002 304KiB/2.8MiB(10%) CN:1 SD:1 DL:101KiB ETA:25s]",
            AT,
            "[#000002 464KiB/2.8MiB(15%) CN:1 SD:1 DL:115KiB ETA:21s]",
            AT,
            "[#000002 2.8MiB/2.8MiB(98%) CN:1 SD:1 DL:134KiB]",
            AT,
            "[#000002 SEED(0.0) CN:0 SD:0]",
            AT,
        ];
        let labels = ["Lantern Weather".to_string(), "Rooms of Salt".to_string()];
        let mut readouts = Readouts::default();
        let mut said = Vec::new();
        for line in lines {
            if readouts.read(line) {
                said.push(readouts.said(&labels));
            }
        }
        assert_eq!(
            said,
            [
                "Lantern Weather 0%",
                "Lantern Weather 0%, Rooms of Salt 10%",
                "Lantern Weather 0%, Rooms of Salt 15%",
                "Lantern Weather stopped, Rooms of Salt 15%",
                "Lantern Weather stopped, Rooms of Salt 98%",
                "Lantern Weather stopped, Rooms of Salt 100%",
            ],
            "a summary is known whole at the next, so one gone reads as gone then"
        );
        assert_eq!(readouts.said(&labels[..1]), "stopped");
    }

    #[test]
    fn a_download_gone_once_all_done_stays_all_done() {
        let labels = ["Lantern Weather".to_string(), "Rooms of Salt".to_string()];
        // A magnet link's metainfo, as aria2c 1.37.0 says it whole before
        // the download goes.
        let readouts = readouts(&[
            AT,
            "[#000001 3.6KiB/3.6KiB(100%) CN:1 SD:1]",
            "[#000002 0B/0B CN:0 SD:0 DL:0B]",
            AT,
            "[#000002 0B/0B CN:0 SD:0 DL:0B]",
            AT,
        ]);
        assert_eq!(
            readouts.said(&labels),
            "Lantern Weather 100%, Rooms of Salt 0B"
        );
        let named_again = self::readouts(&[
            AT,
            "[#000001 0B/40MiB(0%) CN:0]",
            AT,
            AT,
            "[#000001 400KiB/40MiB(1%) CN:1]",
        ]);
        assert_eq!(named_again.said(&labels), "Lantern Weather 1%");
    }

    #[test]
    fn every_readout_aria2c_writes_reads_as_its_share_done() {
        // Each shape aria2c's ConsoleStatCalc.cc writes, with the parentheses an
        // upload adds after the size.
        for (readout, share) in [
            ("1.2MiB/40MiB(3%) CN:4 SD:2 DL:300KiB ETA:2m", "3%"),
            ("40MiB/40MiB(100%) CN:1 SD:1", "100%"),
            (
                "1.2MiB/40MiB(3%) CN:4 DL:300KiB UL:20KiB(1.1MiB) ETA:2m",
                "3%",
            ),
            ("1234B/40000B(3%) CN:1 DL:300B", "3%"),
            ("0B/0B CN:1 SD:0 DL:0B", "0B"),
            ("16KiB/0B CN:1 DL:4KiB UL:2KiB(8KiB)", "16KiB"),
            ("SEED(0.0) CN:0 SD:0", "100%"),
            ("SEED(1.5) CN:3 SD:0 UL:40KiB(60MiB)", "100%"),
            ("SEED(--) CN:0 SD:0", "100%"),
        ] {
            assert_eq!(share_done(readout), share, "{readout}");
        }
        for (readout, said) in [
            (
                "SEED(1.5) CN:3 SD:0 UL:40KiB(60MiB)",
                "100% CN:3 SD:0 UL:40KiB(60MiB)",
            ),
            ("0B/0B CN:1 SD:0 DL:0B", "0B/0B CN:1 SD:0 DL:0B"),
        ] {
            assert_eq!(whole(readout), said, "{readout}");
        }
    }
}
