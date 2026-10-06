# Changelog

All notable changes to muman. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as
[AGENTS.md](AGENTS.md#versioning) applies it.

## [Unreleased]

### Changed

- **A bare query word matches names, not parts of keys.** A word matched
  inside any source key, so `man` picked every manual song and `tube`
  every video; it now matches a key only as the whole ID or file name.
- **A misspelled query field is refused.** A field that is no tag muman
  knows and that no song has, as `artst:`, stops the command with exit
  code 5, rather than matching nothing, or, negated, every song.
- **Songs of one recording share its records.** An LRCLIB or MusicBrainz
  record another song lists already joins the song that found it too, so
  two releases of one recording both get its lyrics and tags. Before,
  the second song was recorded as having found it, never got it, and
  never looked again. A song list in which songs share a record is
  written as format version 2, which muman 0.1 refuses; `remove --purge`
  keeps a record another song still lists.
- **`undo` refuses before it asks, and exits 5.** `undo -n` showed what
  undoing would do even when `undo` would refuse; both now refuse alike,
  before saying anything, with exit code 5, a refused change, rather
  than 4.

### Fixed

- **LRCLIB is asked politely.** Requests to LRCLIB are spaced across a
  run, and a 429 or 503 from any service is waited out, as long as its
  `Retry-After` asks, and asked again. A service still refusing leaves
  its remaining lookups for the next run instead of recording each as
  failed and putting it off for hours.
- **A sync stopped while writing loses nothing.** Each file a run is
  about to write is marked muman's but unvouched for, and each it wrote
  is kept as it goes. Before, stopping a sync that rewrote many songs
  left every one taken for changed by someone else: never written again,
  and left behind as a duplicate when its path changed. A library a
  stopped run left so needs one `sync --force`.
- **A file of your own is never written over unasked.** A song whose
  path, or its lyrics' path, holds a file muman did not write leaves it
  alone and says so; `sync --force` writes over it and keeps it, so
  `undo` puts it back. A lost state file therefore leaves the whole
  library alone until `sync --force`, rather than writing over it.
- **A song is written from the sources still on disk.** A song whose
  chosen source's file was gone, a deleted file or a video no longer
  online, failed until a later sync; it is written at once from its
  other sources, and its comparisons are no longer made again, and the
  state saved, on every sync.
- **`undo` puts back what a new template moved.** A run that only moved
  songs, as a changed `[library] template` does, left no record, so
  `undo` undid the run before it instead. Moves are recorded, and undone.
- **A stopped run can be undone.** A run's record is written ahead of
  the library changes it lists, so a run stopped partway is listed as
  interrupted and can be undone, and its kept files no longer stay in
  `history/` forever. An undo stopped partway finishes when run again.
- **A hand edit is undone with the run that applied it.** A run's song
  list before is the one the last run left, so undoing a sync that
  applied an edit to `songs.toml` puts the edit back too, instead of the
  following sync applying it again.
- **`[history] max_mib` holds for every kept run together**, as
  documented, rather than for each; a run that needs room lets go of the
  oldest runs first.
- **`status` changes nothing and says what `sync` does.** A dry run
  keeps only what it measured: run against another library folder, it
  no longer saved the state as if the library had moved, so the next
  sync wrote every song again. It decides what to remove, keep or leave
  alone as a sync does, so a file changed by hand is shown left in
  place, not removed, and a song missing its lyrics file is shown
  written again, not up to date.
- **`info` counts sources by where they come from.** LRCLIB and
  MusicBrainz records were counted as fetched by yt-dlp; they are kept
  from lookups, counted and stored apart, and one no song lists any more
  shows as unused, as an unused download does.
- **Lyrics are no longer lost to an instrumental record.** LRCLIB lists a
  song's instrumental or off-vocal take as long as the song; one could
  win over the record with words and mark the song instrumental for
  good. A record with words now wins, an exact title wins over one only
  holding it, as `Rain (Live)` holds `Rain`, on LRCLIB and MusicBrainz
  alike, and an instrumental verdict is checked again after
  `recheck_days`. Songs LRCLIB found nothing or an instrumental for are
  looked up again once; full-width and half-width names now match.
- **A MusicBrainz record fetched again keeps its album.** A record gone
  from the store is fetched again on the release it was kept on, rather
  than the best release of the recording, which could change the
  song's album, track and date.
- **A failure ends when the step succeeds.** A source fetched again, or
  measured after failing, no longer keeps its old failure in the state
  file, and a failed fetch no longer keeps a file that has since arrived
  from being read.

## [0.1.2] - 2026-10-05

### Added

- **Tags from MusicBrainz.** A song no source names an album for looks
  its recording up on MusicBrainz by title, artist and length, and gains
  its title, artists, album, album artist, track, disc, date, track
  total, release country, ISRCs and MusicBrainz IDs, ranked with every
  other source's tags. Of the recordings that fit, the one on an official
  album and on the most releases wins, and of its releases the song's own
  album, then the earliest official album that is no compilation.
  `[providers.musicbrainz]` sets it like any provider, `url` a mirror,
  and the `no-album` condition triggers it.
- **More tags.** Track and disc totals, the ISRC, the release country
  and the MusicBrainz recording, track, release, release group, artist
  and album artist IDs are tags of their own, under Picard's names: read
  from a file's tags, set in `[song.tags]`, and written to every format.
  The release's come with the album, from one source. Sources' tags are
  read again on the next `sync`, and a song whose file tags offer any of
  these is written again.
- **Polite to MusicBrainz.** Requests go at most one a second across a
  whole run, back off and ask again when MusicBrainz says they come too
  fast, and name muman and its repository in the user agent.

## [0.1.1] - 2026-10-05

### Added

- **A Nix flake.** `nix run github:jeggao/muman` runs muman with ffmpeg
  and yt-dlp on Linux and Apple silicon Macs; the flake also has an
  overlay and a development shell, and `nix-build` works without
  flakes.
- **Codecs of your choice.** `[audio] codecs` lists the codecs copied
  as they are, among Opus, Vorbis, AAC, MP3, FLAC and ALAC, and `lossy`
  and `lossless` what the rest is encoded to, each tagged in its own
  format.
- **Ranking you can tune.** Each audio and cover measure has a
  `[quality.*]` table in the song list to switch it off, weigh it
  against the others, or change the steps and cutoffs it is judged by.
- **A library size limit.** `[library] max_size` keeps the library
  under a size: songs that do not fit are written at lower bitrates,
  the least audible loss first. Which songs is decided by the song list,
  its sources and the settings alone, never by what the library held
  before, so a library fitted song by song ends as one fitted at once.
  `status` says which songs and why.
- **`export`.** Writes the song list and the library into one zip;
  `--max-size` encodes songs again at lower bitrates, the least audible
  loss first, until it fits, leaving the library as it is.

### Changed

- **Audio measured against real recordings.** Bandwidth is where a
  lowpass cuts the sound off, so a dark or quiet lossless recording no
  longer ranks below its own lossy copies; mono copied into channels at
  different levels or a few samples apart counts as mono; audio clipped
  and then turned down, or clipped and then lossy-encoded, counts as
  clipped. Sources are measured again on the next `sync`.
- **Copies a little fast or slow.** An upload that plays up to 1% faster
  or slower than its release is still the same recording, and its lyrics
  are moved and stretched to fit. Sources are compared again on the next
  `sync`.
- **`info` lists only the formats the library holds**, AAC, MP3, Vorbis
  and ALAC among them.
- **A song written again only for its format says so**: `Updated
  (format)` rather than `(audio)`.
- **The same song list writes the same bytes.** Ogg files no longer get
  a random stream serial, so a library rendered twice is identical file
  for file. Every song is written once again on the next `sync`.

## [0.1.0] - 2026-10-05

### Added

- **A library kept in step with a song list.** `songs.toml` lists every
  song and the sources it may be made from: yt-dlp downloads, files of
  your own, and LRCLIB lyrics. `sync` writes each song once, as Opus or
  FLAC with tags, cover and lyrics, and deletes only files muman wrote.
- **The best of every source, by measurement.** Audio by real bandwidth,
  stereo and clipping; covers by content and real detail; lyrics timed
  over untimed; tags cleaned by rules you can switch off one by one.
- **Duplicates found by fingerprint.** A new source that is the same
  recording as a listed song joins it; a near match is asked about.
- **YouTube Music matching.** An upload is matched to its release, and
  an album is listed as one.
- **Commands** `add`, `sync`, `status`, `info`, `check`, `list`, `set`,
  `edit`, `remove`, `restore` and `undo`.
- **Library layout by template.** `[library] template` names each song's
  path with MiniJinja; a changed template moves files rather than
  writing them again.
- **Settings in the song list:** `[library]`, `[audio]`, `[ytdlp]` and
  `[history]`, so one list makes one library on every machine.
- **Names safe on every system.** Characters Windows refuses become
  lookalikes, reserved names and trailing dots are handled, and length
  limits apply per name and per path.
- **Linux, macOS and Windows** are built and tested; folders follow each
  platform's conventions, and tools are found through `MUMAN_FFMPEG`,
  `MUMAN_FFPROBE`, `MUMAN_YT_DLP`, beside the executable, or on `PATH`.
- **Hooks** run a command for each song written or once a run changed
  the library, with values as placeholders and `MUMAN_*` variables.

[Unreleased]: https://github.com/jeggao/muman/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/jeggao/muman/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/jeggao/muman/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jeggao/muman/releases/tag/v0.1.0
