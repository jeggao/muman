# Changelog

All notable changes to muman. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as
[AGENTS.md](AGENTS.md#versioning) applies it.

## [Unreleased]

## [0.3.0] - 2026-10-07

### Added

- **`check --upstream`** asks each site, downloading nothing, whether
  it still serves each fetched source in the format and at the size its
  song records, with the same tags, and names each it serves otherwise:
  other audio, the format withdrawn, other tags, or the video taken
  down, whose copy in the store is then the only one.
- **What each source held, recorded.** Each song records in the song
  list, as `held."<key>"`, a digest of each part of a built file each
  of its sources holds, its audio, covers, lyrics and tags, and for what
  yt-dlp fetched, the format served and its size. A source fetched again
  that holds otherwise, as a site re-encoding a video, replacing its
  thumbnail or editing its title, leaves its song as built, with a
  warning naming each part changed, until `sync --accept`; a song with
  no file left is built from it and the warning stays. A file of your
  own that changes is followed.
- **Fetched again as fetched.** A YouTube video fetched again, its file
  lost, asks for the format its song records before `[ytdlp] format`,
  so a yt-dlp that ranks formats otherwise, or a site offering new ones,
  serves the same stream. A video of any other site records the page it
  came from, and is fetched again from it, where it could not be at all.

- **WavPack.** `wavpack` is a codec like the others, written as `.wv`
  with APEv2 tags, and a source in it can be copied. `.w64`, `.rf64`,
  `.aifc`, `.caf`, `.tak`, `.shn`, `.dts`, `.thd`, `.mlp` and `.dff`
  files are picked up as songs.

- **ReplayGain kept.** A source's `REPLAYGAIN_TRACK_GAIN` and
  `_PEAK`, and its album's when the song's release comes from it too,
  are written into its song, where they were dropped; an Opus file
  holds the gains as R128's, and an Opus source's R128 gains are read.
- **Pre-emphasis and album images said.** A source whose cue sheet,
  in a FLAC file or a `.cue` beside it, marks it pre-emphasized is named
  when it is measured, since its song keeps no such flag and plays
  bright; so is an album image with a `.cue` beside it, which is one
  song.

- **Speaker layouts chosen like codecs.** `[audio] layouts` lists the
  layouts a song is written in as it is, `"5.1"` taking `5.1(side)` too,
  and `[audio] downmix` what audio in any other is mixed into: the first
  with no more channels than it has. A mix is scaled so it cannot clip,
  and drops the source's ReplayGain. Songs already written stay as
  they are until `layouts` is changed.

### Changed

- **Sites in the song list.** Which extractors each site names its
  sources by, the IDs each kind of its sources takes and where one is
  fetched again from are the song list's `[sites]` tables, written out
  at their defaults like any setting; adding a site is a table, and an
  archive.org source is fetched again from its own address.
- **Sources named by their site.** What yt-dlp fetched is keyed
  `<site>:<id>`, as `youtube.com:<id>` or `archive.org:<id>`, rather
  than by yt-dlp's extractor, as `youtube:<id>`. A video found on
  YouTube Music is the same `youtube.com:<id>`, and a YouTube playlist
  is `youtube.com:playlist/<id>`. A song list or state an earlier muman
  wrote reads as before and is written the new way, writing no song
  again; a source of a site muman has no domain for is renamed by the
  page it was fetched from. A file fetched by its address is named, and
  fetched again, by that address rather than by the mirror that served
  it. `state.json` is format 2, which an earlier muman refuses.
- **`status` takes a query.** It shows only the songs a query matches,
  as `list` reads one, each in full, up to date or not. Without one it
  shows the songs a sync would change and counts the rest, which
  `--all` shows too; a last line counts the songs by what a sync does
  with them.
- **A tag edit writes tags only.** A song whose plan changed only in its
  tags has them written into its library file, its cover and embedded
  lyrics kept, rather than its audio encoded again: one tag on an
  encoded 15-minute song took 13.5 s and takes 0.2 s. A song moved by a
  new template has its tags written after the move the same way.
- **Sources known by their content.** A song names each part it takes
  from a source, its audio, cover or lyrics, by a SHA-256 digest of
  that part's packets or file, not by the file's size and time: a
  source fetched again, or a home copied without its file times, writes
  no song again while it holds the same content, and a file you
  dropped in whose tags a tagger changed has the new tags written into
  its song rather than its audio encoded again. Sources measured before
  are hashed once, decoding nothing, and nothing is written.
- **Each song has its file.** Every decision a sync makes about a song,
  to keep, move, guard, write or delete its file, is made of the one
  file made from most of its own sources, and `status` says what a sync
  does by the same decisions.
- **Settings take their units.** Sizes, bitrates, times, frequencies
  and shares are written with a unit, read by one parser for each kind:
  `opus_bitrate = "160 kb/s"`, `[quality.purity] step = "2 s"`,
  `[quality.bandwidth] step = "500 Hz"`, `[ytdlp] keep_partial =
  "14 days"`, `[history] max_size = "2 GiB"`, `[providers.*] recheck =
  "30 days"`, `incoherence = "1 %"`. Each unit is known by its symbol
  and its name, as `"2 gibibytes"` or `"3 hours"`.
- **Old setting names are renamed for you.** A song list of an earlier
  edition reads as the current one: `opus_kbps = 160` reads as
  `opus_bitrate = "160 kb/s"`, and the next `sync` writes it so, its
  comments kept; `status` and `check` name each one meanwhile. A
  setting named both ways is an error. A song list this muman wrote is
  refused by an earlier one, which names the setting it does not know.
- **Lyrics offsets are times.** A song's `lyrics_offset = "-120 ms"`
  moves its lyrics earlier, and `set lyrics_offset=80ms` sets it;
  `lyrics_offset_ms` is still read and set.

### Fixed

- **Opus ended 312 samples late.** Every Opus file, copied or encoded,
  had its timestamps shifted by its encoder's delay, so it played 6.5 ms
  of padding past its end and gapless albums gapped; Opus songs are
  written again on the next `sync`, and no other song is.
- **Ties broken by the audio.** Two sources the measures scored alike
  went to the one listed first, a 16-bit copy over its 24-bit master
  among them. Lossless audio wins, then the one whose samples use more
  bits, then the lower sample rate.

- **Surround and float audio measured as it is.** Audio was mixed into
  two channels before it was measured, which pushed 7.1 noise at half
  scale past full scale, and a 5.1 master measured 2.58% clipped and
  lost to its own stereo downmix. Each channel is measured
  apart. A float master past full scale counted its overs as clipped,
  so a copy cut at full scale could win; overs in float lossless audio
  are not clipping. A stereo file with one channel inverted measured as
  silence and ranked last; each channel's spectrum is summed rather than
  their mix. Measuring a two-hour file took 3.8 GiB in ffmpeg, which now
  decodes only the excerpts it measures. `status` names a surround
  source's layout. Sources are measured again on the next `sync`.

- **Every channel and sample kept.** A codec no longer gets audio it
  cannot hold. A 5.1 AC-3, E-AC-3, DTS or TrueHD source, whose decoder
  names its speakers as side ones, failed to encode to Opus, and so did
  4.0, 7.1 (wide) and more than eight channels; Opus names the side
  speakers as back ones, and writes any other layout with no speakers
  named. ALAC mixed 7.1 into 6.1 and quad into 5.0 without a word, and
  FLAC failed past eight channels and cut 32-bit and float samples to 24
  bits, clipping float above full scale; lossless audio either cannot
  hold whole is written as FLAC, or else WavPack. Vorbis failed at rates
  such as 22.05 and 96 kHz, and is resampled to 44.1 or 48 kHz from
  them. TrueHD, MLP, DTS-HD Master Audio and DSD counted as lossy, and
  were encoded to `[audio] lossy`. A file with several audio streams was
  made from the first, which could be a stereo downmix beside the
  master; it is made from the best. Sources are read again on the next
  `sync`.

- **A failing song deleted the folder another song was being written
  into.** A song that fails leaves its folder for the sync, which
  clears it once every song is written.

- **Name limits said, not bent.** A `max_name_bytes` under 40 or a
  `max_folder_bytes` under 16 is refused, where it was raised to that
  without a word; an empty template is refused; and a sync warns of the
  songs `max_path` cannot fit.
- **Comparisons counted.** The comparing step names its pairs and the
  recordings it decodes, which its progress counts.
- **A song deleted by hand stays deleted.** A song taken out of
  `songs.toml` by hand whose file is in `sources/manual` is kept under
  `[[removed]]` by the next sync, as `muman remove` keeps it, where it
  was listed again at once as a file dropped in.
- **A freed path is moved into.** A song whose path another song
  leaves in the same sync, as `Untitled [vid00000001]` becomes
  `Untitled` once the other is renamed, is moved there, where it was
  written again.
- **The same MP3 every time.** An MP3's ID3 frames are written in one
  order, where the same song could be written with them in another
  order from one run to the next.
- **Repeated tags.** A file with a tag given twice, as two `TITLE`s, is
  read as two values, the first naming the file, where its title was
  both joined with `;`. A file's tags are read as muman writes them, the
  same names for ID3, MP4 and Vorbis comments.
- **Cut-off files.** A file whose audio stops before its header says, as
  an interrupted copy leaves, is as long as its audio, is measured
  within it, and is warned of; it was measured past its end and ranked
  last without a word. Audio with nothing to judge, as silence, is
  warned of too.
- **Why a song has no print.** `duplicates` says which songs are too
  short to print, hold only silence, or cannot be made, where it said a
  sync would measure them.
- **Bitrates out of range.** A bitrate of 0 is refused when the song
  list is read, where libopus chose its own; an Opus bitrate above
  256 kb/s a channel, which libopus refuses, is held to it.
- **No second copy when the template changes.** A library file whose
  size or time changed since muman wrote it, as a ReplayGain scanner,
  a backup restore or a copy that resets times changes it, moves to its
  new path with the song. It was written again at the new path and left
  at the old, so a new template could double the library; `sync
  --force` did the same. When the song changed too, the file is left
  where it is until `sync --force`, which deletes it once the song is
  written again.

- **Decomposed file names.** A manual file whose name is in Unicode NFD,
  as files copied from a Mac often are, is listed by `add` with the tags
  it was given, and read on Linux and Windows, where it was listed but
  never found.
- **Unreadable audio.** A file whose audio ffprobe cannot decode, such
  as one cut off in its first bytes or a misnamed text file, is reported
  as unreadable when measured and waits for `sync --retry`, rather than
  failing in ffmpeg on every run.
- **Hand-set names on untagged files.** A song whose sources name no
  artist or title takes its album artist from a hand-set artist, and its
  album from a hand-set title as a single, instead of
  `Unknown Artist` and `Unknown Album`.
- **Track 0.** A track or disc numbered 0 in yt-dlp's info, as
  archive.org items have, is no number, not `00` before the title.
- **Short songs on AcoustID.** A song of 2 s or less, whose print is
  empty, is not looked up on AcoustID, which refused it as invalid and
  was asked again after every wait.
- **A song list gone missing removes nothing.** With `songs.toml`
  deleted or moved away, a `sync` read it as listing no song and removed
  the whole library, and `add` wrote a new list holding only what it
  added. Every command that changes the song list now refuses, naming
  how many songs the library holds; a song list that lists no song
  still removes them.
- **Undo history kept for runs that did something.** A dry run, a
  refused or declined change, or a command that failed before reaching
  the library kept a run with no outputs, which pushed real runs out of
  `[history] runs`, and undoing it would have removed every song.
- **Undo over a file you edited.** `undo` put the file a run replaced
  back over one edited since the run wrote it; it now refuses, as it
  does when the song list changed.
- **`edit`, quit without saving.** Quitting after muman reopened the
  file, for a mistake or for a song list changed meanwhile, applied the
  text anyway or reopened it again without end; it now changes nothing.
- **A library named relatively.** `--library lib` was recorded as typed,
  so a run started in another folder took the library as moved and
  every file in it as not muman's; folders are now recorded whole. A
  library that is a file is refused before the old one is forgotten.
- **A song taken out while its file was fresh.** A song deleted from the
  song list by hand within ten seconds of its file being touched was
  listed again by the next sync; it is now kept out.
- **A file stamped in the future.** A dropped file whose time is ahead of
  the clock, as a camera or a FAT stick can leave, waited as "still
  being copied in" until the clock caught up; it is now read at once.
- **An empty picture beside a song.** A zero-byte `.jpg` or `.png` beside
  a song, or as its folder's cover, left the song's audio unmeasured and
  unfingerprinted, ranking it last; each picture is now measured alone,
  and one that does not open costs only itself.
- **Covers too large for FLAC.** A cover over the 16 MiB a FLAC metadata
  block holds failed the song's write on every sync and left an empty
  folder; it is now made a JPEG at most 3000 pixels wide. Lyrics that
  large are not embedded, said so, and a failed write leaves no empty
  folders.
- **A file cut off.** A source cut off partway, as a copy or a download
  stopped, was copied into the library broken; it is now encoded, so the
  song ends cleanly where the source breaks off. `check --decode` now
  names such a file: ffmpeg exits 0 past a broken frame, so every error
  it writes counts.
- **Undo of a song retagged and moved.** Undoing a run that moved a
  song and wrote its new tags, as a changed album does, left the new
  file at the song's path and the old one beside it; the library now
  comes back as it was.
- **A run killed while moving files.** Files a killed run had moved
  before it could record them were no longer muman's; the next run
  follows them by the run's own record.
- **What a killed run leaves behind.** A run killed while measuring left
  its scratch folder, hundreds of MB, in the temporary folder, and a
  killed `export` its partial zip; the next run removes them. A `.part`
  or `.moving` left beside a song's path is removed by the next sync. A
  disk that fills while measuring stops the run, rather than marking each
  source unreadable until `sync --retry`.
- **A read-only song file.** A tag change to a library file made
  read-only failed on every sync; it is written as a new file.
- **A song that lists no source.** `sources = "manual:…"`, a string
  rather than a list, was read as no source, and a song listing none
  failed on every sync yet exited 0; both are now refused when the song
  list is read, naming the song. `restore` of a song whose sources other
  songs list now keeps it removed instead of listing it with none.
- **`set` that changes nothing** rewrote the song list and the state and
  said what it pinned; it now says nothing changed. `genre=a GENRE=b`
  sets both values, and `format=mp3` or `key=…`, which are no tags, are
  refused rather than written as tags.
- **Queries.** A key typed in another case, as `MANUAL:…`, names its
  song. A query that does not read, as a bad pattern, now exits 5 like
  one naming no field.
- **Smaller points.** `add --date` refuses what is no date. `remove -n`
  says which files stay because they changed since muman wrote them. A
  reader that stops early, as `| head` does, ends the output quietly.
  The exit codes list a command line that does not parse under 2.
- **A source edited without a new time.** A file in the manual folder
  changed in place with its size and modification time kept, as some
  taggers or `cp -p` leave it, was never measured again; on Linux and
  macOS the time a file last changed at all now counts too.
- **A library put back from a copy.** Files copied back without their
  times were taken as changed by hand and no longer followed the song
  list; one whose bytes are still the ones muman wrote is its own again.
  This covers files written from now on.
- **A dropped file moved.** Moving a file in the manual folder rewrote
  its song, though nothing in it changed.
- **`export` of an edited song list.** An export after the song list was
  edited, before a sync, put a song list in the zip that its library did
  not follow; it is now refused until a sync.
- **Stale records.** A source taken out of its song left its
  `held."<key>"` line behind; it now goes with it.
- **A library in the sources.** A library inside the home's sources, as
  the manual folder, read every song it wrote back as a new one, run
  after run; it is now refused.
- **A move onto a file of your own.** A song whose new path, after a
  tag change, held a file not muman's lost its file from the library; it
  now stays where it was, with a warning, until `sync --force`.
- **Names at the smallest limits.** At `max_name_bytes` near 40, two
  songs told apart by a long source ID lost their title and went past
  the limit; the ID is now cut to leave the name a start. A limit above
  255, which no filesystem holds, is refused.
- **A title changing only its case** moves its file, as other renames
  do, rather than writing the song again.
- **Invisible and reordering names.** A title of nothing visible, as a
  lone zero-width space, counts as none; characters that reorder text,
  which can make a name read as another, are left out of file names. A
  template's `{{ track }}` with no track writes nothing, not `none`.
- **Two files of one name.** Two files in the manual folder whose names
  differ only in Unicode normalization, as `café` composed and
  decomposed, are said; one was passed over silently.
- **`list` one line a song.** A file name holding a newline or tab broke
  `list`'s lines.
- **A song list behind a link.** A `songs.toml` that is a link was
  replaced by a file on the next write, and every rewritten file of the
  home lost its permissions; the link is now written through and the
  permissions kept.
- **Settings errors name their line.** An error in a setting named a
  line counted without the song list's opening comments; it now names
  the line in the file.
- **Lyrics files.** UTF-16 without a byte-order mark is read; a `.lrc`
  of other data is no lyrics rather than garbled ones, and costs its
  song only them; `[mm:ss:xx]` times read as timed.
- **`add` mistakes.** A path that names nothing is said so rather than
  handed to yt-dlp as an address, and a file that is no song, lyrics or
  picture is refused rather than copied in. A `lyrics_offset` past an
  hour is refused. The `max_size` advice names `min_bitrate`.
- **`export --max-size` gave up** at a size it could reach, as 3 MiB
  for songs whose lowest bitrates take 2.1 MiB, when its estimates kept
  moving; it now settles on the sizes it has encoded.
- **One library, one spelling.** `--library lib/` and `--library lib`
  were recorded apart, rewriting the state on a run that changed
  nothing.
- **A history that cannot be written** no longer stops a command halfway,
  its song list changed and its library not: the run goes on, with a
  warning that it cannot be undone.
- **`status` after a dropped file moved** said its song failed and the
  file was new; it now says the next sync follows the move.
- **Smaller points.** The notice of sources not read again is given once
  a run. `cover:` and `lyrics:` in a query take `yes` or `none` and
  refuse anything else, and a `list --format` naming a field no song has
  is refused, as a query naming one is.
- **Two homes, one library.** Two homes syncing into one library at once
  wrote over each other's partial files; a run now waits for any other
  writing the same library, by a lock kept in the system's temporary
  folder.
- **A cover the tags cannot hold.** A cover ffmpeg reads but a song's
  tags cannot take, as a picture whose header is cut, failed the song's
  write on every sync; the song is now written without it, said so.
- **`status` of a home not made yet** no longer makes it.
- **A record's ID named its file.** A MusicBrainz server answering with
  an ID such as `../../state` had muman write the record over
  `state.json`, or anywhere else; an ID that is no MusicBrainz ID is now
  refused, and no file muman keeps is written outside its folder.
- **Runs at once kept apart in the history.** A command that waited on
  another, as two `set`s or a `sync` beside an `undo`, recorded the
  other's change to the song list as its own, so undoing it undid both,
  or brought back what an undo had removed. Each command that changes the
  home now holds it from first to last, `edit` once its editor closes.
- **Undo over a file of your own.** `undo` put a file the run had removed
  back over a file you had put in its place since, and wrote again over
  a file you had edited when the history had no room to keep it; it now
  refuses both. An undo stopped partway finishes when run again; one
  after a library put back from a copy is no longer refused.
- **Undo of a home's first `add`** left no song list, and every command
  after it refused for the songs the library still held.
- **A history budget below one file** let go of every other run's
  record and kept the file anyway; such a file is now written again
  instead, as the guide says.
- **A run killed between a song and its lyrics** left the lyrics at the
  old path for good; the next run moves them after it.
- **A link from the manual folder into the library** read the library's
  songs back in as new ones on every sync; such files are passed over.
- **`check` during a sync** reported what that sync was writing as left
  by an interrupted run; it now says a run is under way.
- **A new song list** was made readable by its owner alone.
- **`[library] max_size` left the library over its limit** for good when
  encoders kept missing their bitrates, as Opus did by a third: the last
  fit was made on estimates and the sizes measured after it never fitted
  again. A fit whose passes run out now settles on sizes measured.
- **An archive.org item of several files** was fetched whole and listed
  no song: its files' IDs, `<item>/<file>`, were taken for no key's, so
  the files were left in the store where `purge` did not see them. Each
  file is now a song, fetched again from its own page, and an item of
  one file is listed without a warning.
- **A web page served as a cover** was embedded as the album art and
  recorded as found; a body that is no JPEG, PNG or WebP now fails the
  lookup.
- **An error page in every warning:** a failed request printed and
  recorded the server's whole body, megabytes of it; only its first
  200 characters are kept.
- **A refusing service asked again in the same run:** a later round
  made the lookups a refusal had put off, and a provider whose
  `recheck` is 0 was asked twice a run. Neither is now.
- **`Retry-After` as a date** was ignored; it is read as RFC 9110 has
  servers send it. A gzip body no one asked for now says so, rather than
  "invalid utf-8".
- **`sync --rematch`'s help** promised every lookup; it says now that a
  song with a source from a provider asks it nothing.
- **Another recording's IDs:** a MusicBrainz record whose title and
  artist lost to the song's own still gave it its ISRC and IDs; they now
  come only from a source titling the recording as the song is titled.
- **A 10 MB title** from a server failed the song's write on every
  run; a value over 4 KiB is offered by no source. A NUL in a value no
  longer cuts the tag short, and characters reordering text no longer
  reach the tags or `list`.
- **`info`'s library size** added up bytes where `sync` counts whole
  `[library] block_size` blocks, so the two disagreed on whether the
  library fit; `info` counts blocks too.
- **A 5.1 FLAC** lost the channel mask ffmpeg writes for it, and played
  as 5.1 with side speakers; every song is written once more for it.
  AC-3 files in the manual folder are listed, where they were passed
  over without a word.
- **A hand edit made while a run went** was recorded as that run's, and
  its `undo` dropped it without a word; a run records the song list it
  last read or wrote, so `undo` refuses instead.
- **`edit` said "Changed"** for a song removed from the song list while
  the editor was open, though nothing was applied; it says the song is
  no longer listed.
- **A tag's broken release ID** kept a song from any cover: the archive
  was asked for that ID alone. An ID that is no MBID is passed over and
  the album searched for by its names.
- **`export --max-size` refused on estimates:** a zip too small for
  the lowest bitrates as estimated was refused without encoding one;
  the lowest are encoded and measured first, as the library's
  `max_size` does, both fitting by one loop.
- **A run killed between two chained moves**, a song moved to a new
  folder and another into the place it left, was followed by neither,
  and the first song stayed at its new path with its old tags. The
  moves a run made are followed as the ordered list they are.
- **A link back up, walked once.** A folder in the manual folder linking
  to a folder it is inside listed each file in it dozens of times, under
  ever longer paths, and `add` of such a folder copied it as often; the
  link is now passed over, as a link to nothing is. Other linked
  folders are read as before.
- **Lyrics in older encodings.** A `.lrc` in GBK, Shift-JIS,
  Windows-1252 or another older encoding read as replacement characters;
  it is now told by its bytes and read as written.
- **Names cut whole.** A name cut to its byte limit ends between
  characters as a reader sees them, never inside a flag or between a
  letter and its accent; a song whose name was cut so moves to the new
  name.
- **The editor on Windows.** A `$VISUAL` or `$EDITOR` holding a path, as
  `C:\Tools\edit.exe`, lost its backslashes and did not start.

## [0.2.3] - 2026-10-07

### Added

- **Every setting written out.** A new song list holds each setting at
  its default, not commented out, and a list made earlier gains the
  ones it lacks on its next write. `edition` names the defaults they
  were written from. When a later muman changes a default, `sync`,
  `status` and `check` name each setting still at the old one, and
  `sync --update-defaults` moves them to the new default; settings you
  changed stay.

### Changed

- **Each song is one block.** A song's tags are written as
  `tags.title = …` lines under its `sources`, not as a `[song.tags]`
  table after a blank line, so blank lines fall only between songs.
  Album and removed songs' tags are written the same way. A
  `[song.tags]` table still reads, and the next write turns it into
  dotted keys, its comments kept.

## [0.2.2] - 2026-10-06

### Changed

- **Help fits the terminal.** `--help` wraps to the terminal's width,
  up to 100 columns, and the tables after the options, the exit codes,
  query terms and assignments, wrap with it, their text under each term
  on a narrow terminal. Help is in color: headings green, commands,
  flags and quoted values cyan, values to fill in yellow, defaults
  dimmed; `NO_COLOR` turns it off.
- **The status line fits the terminal.** Narrower than 100 columns, it
  drops the percentage, then the bar, then the time left, so it stays
  one line and keeps room for the songs under way. Its time left is the
  step's rate so far, as plain lines say it.

### Fixed

- **One status line during lookups.** A fetch a lookup started drew a
  second status line over the lookups' own, the two taking turns with
  each tick, and messages after it could run into the line. The fetch
  takes the line until it ends, then gives it back.

## [0.2.1] - 2026-10-06

### Added

- **MusicBrainz tags by fingerprint.** A song without an album asks
  AcoustID for its recording by the fingerprint of its audio before it
  searches MusicBrainz by name, so a file with wrong tags, or none,
  still finds its tags. The new `acoustid` provider sends the first two
  minutes' fingerprint and the length, takes a result scored 0.5 or more
  within 3 s of the song's length, and keeps the recording as a
  `musicbrainz:` record. `[providers.acoustid]` sets its `key`, `url`
  and the usual limits; `enabled = false` sends no fingerprints.

## [0.2.0] - 2026-10-06

### Added

- **Progress for long steps.** Measuring, comparing, looking songs up,
  fetching, writing, decoding and exporting say how far they have got:
  on a terminal, one status line under the messages with the step, a
  bar, the count, the time left and the songs under way, and the tab or
  taskbar's progress where the terminal shows it; in a log, a plain line
  every tenth of the way or ten seconds. `--progress` (or
  `MUMAN_PROGRESS`) chooses `auto`, `plain`, `json` events for a program
  to read, or `none`.
- **`muman purge`.** Deletes fetched sources and lookup records no song
  uses, which `status` and `info` name as unused, saying first what goes
  and how much it frees. A removed song's sources stay for `restore`,
  and files of your own are never touched.
- **Covers from the Cover Art Archive.** A song with no cover, or one
  too soft to look sharp (fewer than 500 px of detail, as `info`
  counts), looks up its album's front cover on the Cover Art Archive, by
  its MusicBrainz release group or release ID, else by its album and
  album artist searched on MusicBrainz. The cover joins every song of
  the album, which asks once between them, and is picked by the same
  measures as any cover. `[providers.coverart]` sets it like any
  provider, and the `small-cover` condition triggers it.
- **`muman duplicates`.** Lists songs listed apart that are one
  recording, by their audio fingerprints: a file of an album there
  twice first, then a track and its copies on other albums. It changes
  nothing; a query names the groups shown. It compares only prints of
  lengths one recording can have, so 1,500 songs take seconds.

### Changed

- **A template's `artist` and `artists` are as documented.** They held
  every artist joined into one name, so a template naming folders by
  `{{ artist }}` made one per combination of artists; `artist` is the
  first and `artists` the list. A custom template using them moves the
  songs it names differently, which `undo` puts back; the default
  template is unaffected.
- **`status`, `info` and `check` report on stdout.** Their reports went
  to stderr with every message, so `muman status | less` showed nothing;
  what they report is on stdout, and what they do on the way, measuring
  or decoding, stays on stderr.
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
- **Paths outside the library are shown whole.** `status` and `info`
  dropped the leading `/` of an unused file's path, and the drive on
  Windows; a file still being copied into the manual folder is named by
  its `manual:` key.
- **`check` finds a song's lyrics file missing**, which `sync` writes
  again.
- **An upload a release takes the place of is deleted.** An upload
  fetched before YouTube Music's release replaced it stayed in the store,
  unused, for good.
- **A comment right under the song list's header stays.** It was taken
  for part of the header and replaced with it on the next save.
- **`edit` keeps comments inside a song, and can remove every song it
  opened.** A comment above a key of a `[[song]]` was left out of the
  file the editor opened, and the song counted as changed and written
  without it; deleting every song opened did nothing.
- **Stale partial downloads go from every folder.** yt-dlp keeps an
  unfinished download in a folder per channel under `partial/`, which
  `[ytdlp] partial_days` never reached; it now does, and empty folders
  go too.
- **A song's new lyrics survive its old name going on Windows and
  macOS.** A song written again in another format under a name
  differing only in case had its new lyrics file deleted with the old,
  which a filesystem blind to case takes for the same file.
- **A name telling two songs apart is made safe.** The part of a file's
  own name added to tell two songs of one path apart was written as it
  was, so a character Windows refuses reached the library.
- **YouTube lookups made at once keep apart.** Lookups under way at once
  downloaded the audio they compare into one folder, so two comparing
  the same candidate wrote one file and could fail or misjudge.
- **Fitting `max_size` always ends.** When songs only just did not fit,
  by the margin fitting leaves for its estimates, no song was left out
  and the sync asked again for ever.
- **A song written again says so.** A song written with nothing changed,
  by `sync --force` or after a stopped run, was said to be updated with
  an empty list of changes, or added.
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

[Unreleased]: https://github.com/jeggao/muman/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/jeggao/muman/compare/v0.2.3...v0.3.0
[0.2.3]: https://github.com/jeggao/muman/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/jeggao/muman/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/jeggao/muman/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/jeggao/muman/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/jeggao/muman/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/jeggao/muman/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jeggao/muman/releases/tag/v0.1.0
