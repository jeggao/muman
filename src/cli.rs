//! Command line: what to add, sync or show, and where the state root
//! and the library are.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

pub const EXIT_CODES_HELP: &str = "\
Exit codes:
  0  Every song was written, or nothing needed to be; or a change was
     declined
  2  yt-dlp, ffmpeg or ffprobe is missing, or the system names no
     home folder
  4  yt-dlp failed, at least one song could not be written, the song
     list or state could not be read or written, or check found a problem
  5  A query matched no song, a change needs a terminal, -y or --all, or
     undo refused";

#[derive(Debug, Parser)]
#[command(
    name = env!("CARGO_PKG_NAME"),
    about = "Keep a music library made from YouTube and files of your own, each song from the best of its sources.",
    long_about = "Keep a music library made from YouTube and files of your own, each song from the best of its sources.\n\n\
        songs.toml in the state root lists every song and the sources it may be made from: \
        what yt-dlp fetched, kept whole in sources/yt-dlp, and files dropped into sources/manual. \
        Each source is measured — its audio's real bandwidth, stereo and clipping, how much of it \
        is not the song, its pictures' content and real detail, its lyrics, its tags — and for \
        each song the best audio, cover, lyrics and tags are picked from among them by those \
        measures alone. The song is written to the library where the song list's [library] template puts it, \
        <album artist>/<album>/<track title> by default, its audio copied or encoded, tagged \
        and covered, with its lyrics. \
        A song is written again only when what it is made from changes, and a file muman \
        wrote that no listed song makes any more is deleted.\n\n\
        A new source the same recording as a listed song, by its fingerprint, is added to that \
        song; one that may be is asked about.",
    version,
    after_help = EXIT_CODES_HELP,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// The state root: the song list, the state file and the sources
    /// [default: the platform's local data folder, then `muman`].
    #[arg(long, global = true, env = "MUMAN_HOME", value_name = "DIR")]
    pub home: Option<PathBuf>,

    /// Folder the songs are written to [default: the song list's
    /// `[library] path`, else the platform's music folder, then `muman`].
    #[arg(long, global = true, env = "MUMAN_LIBRARY", value_name = "DIR")]
    pub library: Option<PathBuf>,

    /// Say each yt-dlp, ffmpeg and ffprobe command as it runs, and how
    /// close each new source came to every song it was compared with.
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

/// How new sources that may be a listed song are decided.
#[derive(Debug, Clone, Copy, Default, Args)]
pub struct Matching {
    /// Add a new source to the listed song it may be the same recording
    /// as without asking.
    #[arg(short = 'y', long, conflicts_with = "new")]
    pub yes: bool,

    /// Make every new source a song of its own, without comparing it.
    #[arg(long)]
    pub new: bool,
}

/// How a change to songs a query selects is confirmed.
#[derive(Debug, Clone, Copy, Default, Args)]
pub struct Confirm {
    /// Make the change without asking; required without a terminal.
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Act on every song the query matches, without picking among them.
    #[arg(long)]
    pub all: bool,

    /// Say what would change; change nothing.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

/// Tags set in `[song.tags]` of every song an `add` lists or adds to,
/// over what its sources offer.
#[derive(Debug, Clone, Default, Args)]
#[command(next_help_heading = "Tags")]
pub struct TagArgs {
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,

    /// Given again, one more artist.
    #[arg(long, value_name = "NAME")]
    pub artist: Vec<String>,

    #[arg(long, value_name = "TEXT")]
    pub album: Option<String>,

    #[arg(long, value_name = "NAME")]
    pub album_artist: Option<String>,

    /// Given again, one more genre.
    #[arg(long, value_name = "TEXT")]
    pub genre: Vec<String>,

    /// The release date, as YYYY-MM-DD or a year.
    #[arg(long, value_name = "DATE")]
    pub date: Option<String>,

    #[arg(long, value_name = "N")]
    pub track: Option<u32>,

    #[arg(long, value_name = "N")]
    pub disc: Option<u32>,

    /// Any Vorbis comment; given again with one name, one more value.
    #[arg(long, value_name = "NAME=VALUE", value_parser = name_value)]
    pub tag: Vec<(String, String)>,
}

impl TagArgs {
    /// Each tag given, by its song-list name, with its values in order.
    #[must_use]
    pub fn tags(&self) -> Vec<(String, Vec<String>)> {
        let one = |s: &Option<String>| s.iter().cloned().collect::<Vec<_>>();
        let number = |n: &Option<u32>| n.iter().map(u32::to_string).collect::<Vec<_>>();
        let mut tags = vec![
            ("title".to_string(), one(&self.title)),
            ("artist".to_string(), self.artist.clone()),
            ("album".to_string(), one(&self.album)),
            ("album_artist".to_string(), one(&self.album_artist)),
            ("genre".to_string(), self.genre.clone()),
            ("date".to_string(), one(&self.date)),
            ("track".to_string(), number(&self.track)),
            ("disc".to_string(), number(&self.disc)),
        ];
        for (name, value) in &self.tag {
            let field = crate::tags::vorbis_key(name);
            match tags
                .iter_mut()
                .find(|(n, _)| crate::tags::vorbis_key(n) == field)
            {
                Some((_, values)) => values.push(value.clone()),
                None => tags.push((name.clone(), vec![value.clone()])),
            }
        }
        tags.retain(|(_, values)| !values.is_empty());
        tags
    }
}

fn name_value(text: &str) -> Result<(String, String), String> {
    match text.split_once('=') {
        Some((name, value)) if !name.trim().is_empty() => {
            Ok((name.trim().to_string(), value.to_string()))
        }
        _ => Err(format!("`{text}` is not NAME=VALUE")),
    }
}

const QUERY_HELP: &str = "\
Query:
  lumo fenn             Every word in the title, artist, album or album
                        artist, or a source's ID or file name whole
  artist:fenn           A field containing the text
  'artist:=Lumo Fenn'   A field equal to the text, quoted with spaces
  title::^one           A field matching a regular expression
  ^lyrics:yes           Not matching the term
  youtube:<id>          The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.";

const SET_HELP: &str = "\
Assignments:
  genre=House        Set a tag; given again, one more value
  genre!             Clear the tag set, so the sources' value shows
  audio=<key>        Pin the audio, cover or lyrics to one of its sources
  lyrics=false       Take no lyrics
  audio!             Unpin
  lyrics_offset_ms=120
Every other word is a query, as `muman list` reads it.";

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Add songs: fetch what URLs name, or copy files into the manual
    /// folder, list each as a song or as a source of the song it is the
    /// same recording as, set any tags given on each, then sync.
    Add {
        /// Video, playlist, album or channel URLs, as yt-dlp reads them;
        /// or audio files and folders of them, copied into the manual
        /// folder, or originals yt-dlp fetched, into the store.
        #[arg(required = true, value_name = "URL|FILE")]
        inputs: Vec<String>,

        /// Keep each video as uploaded, never looking for its YouTube
        /// Music track; other lookups are still made.
        #[arg(long)]
        no_match: bool,

        #[command(flatten)]
        matching: Matching,

        #[command(flatten)]
        tags: TagArgs,
    },
    /// Bring the library in line with the song list: fetch any missing
    /// source, add files dropped into the manual folder, make every
    /// lookup due, write every song whose sources or tags changed, and
    /// delete what no song makes.
    Sync {
        /// Make every lookup now, whatever an earlier one found: an upload
        /// on YouTube Music for its track, a track on YouTube for an upload
        /// with subtitles, a song on LRCLIB for its lyrics, a song on
        /// MusicBrainz for its album.
        #[arg(long)]
        rematch: bool,

        /// Write every song again, changed or not, a library file changed
        /// since muman wrote it included.
        #[arg(long)]
        force: bool,

        /// Read again sources that could not be read before, and fetch
        /// again at once those that failed to.
        #[arg(long)]
        retry: bool,

        #[command(flatten)]
        matching: Matching,
    },
    /// List the songs a query matches, one per line: its first key, then
    /// its artist, title and album, separated by tabs.
    #[command(after_help = QUERY_HELP)]
    List {
        #[arg(value_name = "QUERY")]
        query: Vec<String>,

        /// Write each song by a template of `{field}`s instead.
        #[arg(short, long, value_name = "TEMPLATE", conflicts_with = "keys")]
        format: Option<String>,

        /// Write each song's first key alone.
        #[arg(long)]
        keys: bool,

        /// List removed songs instead: each key and the song it was.
        #[arg(long)]
        removed: bool,
    },
    /// Remove the songs a query matches: their library files go, and they
    /// are kept under `[[removed]]` so no playlist or dropped file lists
    /// them again.
    #[command(after_help = QUERY_HELP)]
    Remove {
        #[arg(required = true, value_name = "QUERY")]
        query: Vec<String>,

        /// Also delete their sources: fetched files outright, files of
        /// your own to the trash.
        #[arg(long)]
        purge: bool,

        #[command(flatten)]
        confirm: Confirm,
    },
    /// List again the removed songs a query matches, fetching any source
    /// purged.
    #[command(after_help = QUERY_HELP)]
    Restore {
        #[arg(required = true, value_name = "QUERY")]
        query: Vec<String>,

        #[command(flatten)]
        confirm: Confirm,
    },
    /// Set tags, pins or the lyrics offset on the songs a query matches:
    /// each `NAME=VALUE` sets, `NAME!` clears, the other words query.
    #[command(after_help = SET_HELP)]
    Set {
        #[arg(required = true, value_name = "QUERY|NAME=VALUE|NAME!")]
        terms: Vec<String>,

        #[command(flatten)]
        confirm: Confirm,
    },
    /// Edit the songs a query matches in $VISUAL or $EDITOR, as their
    /// song-list entries, then apply what changed.
    #[command(after_help = QUERY_HELP)]
    Edit {
        #[arg(value_name = "QUERY")]
        query: Vec<String>,

        /// Act on every song the query matches, without picking among them.
        #[arg(long)]
        all: bool,
    },
    /// Put the song list and the library back as they were before the
    /// last run that changed them; how many runs are kept is the song
    /// list's `[history] runs`.
    Undo {
        #[arg(short = 'y', long)]
        yes: bool,

        /// Say what would be put back; change nothing.
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Delete fetched sources and lookup records no song uses: an upload
    /// a release took the place of, a source taken out of its song. A
    /// removed song's sources stay for `restore`, and a file of your own
    /// is never touched. A source listed again is fetched again.
    Purge {
        #[arg(short = 'y', long)]
        yes: bool,

        /// Say what would be deleted; change nothing.
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Check the library against what muman recorded: files missing,
    /// empty or changed since written, left by an interrupted run, or
    /// not muman's; sources missing or unreadable.
    Check {
        /// Also decode every source in full, to find a truncated download.
        #[arg(long)]
        decode: bool,
    },
    /// Say what a sync would write, and for each song where each of its
    /// aspects comes from and why; change nothing.
    Status,
    /// Write the song list and the library, as the last run left them,
    /// into one zip: songs.toml at its root, the songs under library/.
    /// With --max-size, songs are encoded again at lower bitrates, the
    /// least audible loss first, until the zip fits; the library itself
    /// is left as it is.
    Export {
        /// The zip to write, or a folder to write muman.zip in.
        #[arg(short, long, value_name = "PATH", default_value = "muman.zip")]
        output: PathBuf,

        /// The most the zip may take, such as 4GiB or 700MB.
        #[arg(long, value_name = "SIZE", value_parser = crate::fit::parse_size)]
        max_size: Option<u64>,
    },
    /// List the songs listed apart that are one recording, by their audio
    /// fingerprints: a file of an album there twice, or a track and its
    /// copies on other albums. Groups on one album come first; songs any
    /// group holds that the query matches name the groups shown. Changes
    /// nothing.
    #[command(after_help = QUERY_HELP)]
    Duplicates {
        #[arg(value_name = "QUERY")]
        query: Vec<String>,
    },
    /// Count what the library holds, whether it is in step with the song
    /// list, the lookups due, and what in it could be better: lossy or narrow audio,
    /// missing or soft covers, missing lyrics or tags. Reads only what
    /// earlier runs recorded; `--verbose` names every song counted.
    Info,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("muman").chain(args.iter().copied()))
    }

    #[test]
    fn add_needs_an_input_and_keeps_their_order() {
        assert!(parse(&["add"]).is_err());
        let Command::Add {
            inputs,
            no_match,
            matching,
            tags,
        } = parse(&["add", "a", "b.flac"]).unwrap().command
        else {
            panic!("not add");
        };
        assert_eq!(inputs, ["a", "b.flac"]);
        assert!(!no_match && !matching.yes && !matching.new);
        assert_eq!(tags.tags(), []);
    }

    #[test]
    fn tag_flags_name_song_list_tags_in_order() {
        let Command::Add { tags, .. } = parse(&[
            "add",
            "--artist",
            "Marlo Venn",
            "--tag",
            "COMPOSER=Marlo Venn",
            "--track",
            "3",
            "--artist",
            "The Glass Orchards",
            "--tag",
            "ARTIST=Ada Quill",
            "a.flac",
        ])
        .unwrap()
        .command
        else {
            panic!("not add");
        };
        let strings = |v: &[&str]| v.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            tags.tags(),
            [
                (
                    "artist".to_string(),
                    strings(&["Marlo Venn", "The Glass Orchards", "Ada Quill"])
                ),
                ("track".to_string(), strings(&["3"])),
                ("COMPOSER".to_string(), strings(&["Marlo Venn"])),
            ]
        );
        assert!(parse(&["add", "--tag", "novalue", "a"]).is_err());
        assert!(parse(&["sync", "--artist", "A"]).is_err());
    }

    #[test]
    fn yes_and_new_exclude_each_other() {
        assert!(parse(&["sync", "-y", "--new"]).is_err());
        assert!(parse(&["add", "-y", "u"]).is_ok());
    }

    #[test]
    fn folders_go_before_or_after_the_command() {
        let cli = parse(&["--home", "/h", "sync", "--library", "/l"]).unwrap();
        assert_eq!(cli.home, Some(PathBuf::from("/h")));
        assert_eq!(cli.library, Some(PathBuf::from("/l")));
    }

    #[test]
    fn export_reads_a_size_with_its_unit() {
        let Command::Export { output, max_size } =
            parse(&["export", "-o", "out.zip", "--max-size", "4GiB"])
                .unwrap()
                .command
        else {
            panic!("not export");
        };
        assert_eq!(output, PathBuf::from("out.zip"));
        assert_eq!(max_size, Some(4 << 30));
        assert!(parse(&["export", "--max-size", "lots"]).is_err());
    }

    #[test]
    fn a_command_is_required() {
        assert!(parse(&[]).is_err());
        assert!(matches!(
            parse(&["status"]).unwrap().command,
            Command::Status
        ));
    }
}
