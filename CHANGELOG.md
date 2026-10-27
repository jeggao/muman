# Changelog

All notable changes to muman. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as
[AGENTS.md](AGENTS.md#versioning) applies it.

## [Unreleased]

### Added

- **`[lyrics] prefer`** ranks lyrics by their role and timing, best
  first: put translations, the original or a romanization first, or
  timed or untimed lyrics before any role, and leave out a role never to
  take it.
- **`--trust`**, or `MUMAN_TRUST=1`, trusts the commands and servers the
  song list sets, as it sets them, without asking: for a run with no
  terminal to ask on.
- **`export --redact` shares a song list without its secrets.** The
  zip's `songs.toml` is written with AcoustID's key, the logins, headers
  and tokens in yt-dlp's and aria2c's arguments (`--password`,
  `--add-header`, `--rpc-secret`, `po_token` and the like) and the
  credentials in any address replaced by `<redacted>`, its comments and
  layout kept. A path, as `--cookies` names, stays, as does any other
  word of a hook or a program. Without it the song list goes in as it is.
- **Lyrics are cleaned by rules of their own,** each a switch in the
  song list's `[clean.lyrics.<tier>]`, as tags are by `[clean.<tier>]`.
  Beside cues and credits they take out the space LRCLIB puts after each
  time, invisible characters, subtitle markup and `\h`, a subtitle's line
  written again for every frame or typed out as it grows, the title and
  artist restated around the lyrics, ID tags in untimed lyrics, and a
  Cyrillic or Greek letter hidden in a Latin word; credits are read in
  more shapes, `Translation (English) : …`, `Voice  …` or a credit split
  over two lines, and cues in untimed lyrics too.
- **`[library] source_comments`** chooses whether the comment a source's
  own tags carry is written, as a store or an uploader left it; a
  `comment` set in a song's or an album's `tags` is written either way.

### Changed

- **Songs to pick are told apart, and a change asks by name.** When a
  query matches several songs, each is listed by its album, track,
  length, audio, cover and lyrics, where they differ, and none is
  picked to begin with. A change marks each song `−`, `~` or `+`, counts
  what it does and what `undo` puts back, and asks "Remove 2 songs?"
  rather than "Go ahead?"; `purge` and `undo` ask by name too.
  `remove --purge` that trashes files of your own takes the count of
  songs typed.
- **A new source that may be a listed song is shown beside it** before
  `add` or `sync` asks: the name, album, length and audio of each and
  how alike they sound, then "Its own song" or "Add to" the song, keeping
  it apart first.
- **Lyrics are told apart by their role in the song:** the original,
  in the language it is sung in; a romanization, in that language's
  Latin letters; or a translation into a language you read. Before,
  only their language counted, so the Japanese lyrics of a song sung in
  Japanese were never taken where only `en` was listed. The language
  sung is the song's `language`: set by hand, else a MusicBrainz work's
  lyrics language, a file's own tag, the language YouTube says a video's
  audio is in, or that of the audio file's own lyrics tag. A subtitle's
  language no longer sets it, since a subtitle may be a translation.
- **`[defaults] lyrics` is `[lyrics] read`,** the languages you read,
  and the next write moves it. A translation is taken only into one of
  them. It takes any language tag, as `zh-Hant`
  or `pt-BR`, matched as a browser matches one: `zh-TW` takes `zh-Hant`
  and never `zh-Hans`, and `ger`, `deu` and `de` are one language.
- **A `language` tag takes any spelling of a language,** as `ja` or
  `ja-JP`, and is written as its ISO 639-2 terminologic code, `deu`
  rather than `ger`.
- **MusicBrainz is asked once more for each song a search finds,** for
  the languages of its works' lyrics.
- **A song's lyrics are written again when their cleaning changes,** as
  a rule writes otherwise in a new muman or a switch is flipped; before,
  lyrics written under older rules stayed as they were. The first sync
  writes every song with lyrics again.

- **A song list that sets commands or servers asks once before muman
  runs them.** The first command after updating that could run a song
  list's hook, a program of its `[tools]`, yt-dlp's or aria2c's
  arguments, or reach its providers' servers or sites' addresses lists
  them and asks whether to trust them; without a terminal it exits with
  5 and runs nothing. Answer once on a terminal, or give a script or a
  scheduled `muman sync` `--trust` once. `-y` trusts nothing.
- **A hook that hands a shell a script with a placeholder in it is
  refused.** In `run = ["sh", "-c", "tag {path}"]` a title holding
  `$(...)` ran as a command; the same goes for bash, zsh, dash, ksh,
  fish, PowerShell's `-Command` and `cmd /c`. The song list is refused
  until the script reads the value from the environment instead:
  `run = ["sh", "-c", "tag \"$MUMAN_PATH\""]`, or `$env:MUMAN_PATH` in
  PowerShell. A placeholder after a shell's script, as `$1`, stays.
- **`{rel}` and `MUMAN_REL` begin with `./`,** so a library path
  beginning with `-` is never read as an option. A hook that joins
  `{rel}` to a folder of its own still works; one that compares it with
  a path as written before drops the `./` first.
- **Where `[ytdlp] args` set what an option of muman's own sets,
  muman's wins,** as yt-dlp reads its config file before its command
  line; `format`, `sub_langs` and `concurrent_fragments` set three of
  those.
- **A source's own comment is no longer written** unless `[library]
  source_comments` is set; the next `sync` retags each song that held
  one.
- **MP3 grouping and work are written where Picard writes them by
  default:** a grouping in `TIT1`, a work in a `WORK` user text frame,
  rather than iTunes' `GRP1` and `TIT1`. A source's `GRP1` is still read
  as its grouping.

### Fixed

- **`duplicates` tells albums of one name apart by their artist,** so a
  recording on two artists' "Best Of" is no longer called a file one
  album holds twice.
- **An MP3's lyrics are marked with the language they are in,** as
  their source names it, or `XXX` where it names none, rather than with
  the song's `LANGUAGE`; its comment with English, as Picard marks one,
  so Picard reads it as the song's comment. The next `sync` retags every
  song.
- **A torrent fetched beside yt-dlp no longer loses its folder.** A
  sync clearing yt-dlp's stale partial downloads removed the empty
  `partial/torrent` folder while aria2c's fetch made a folder in it, so
  a torrent fetched in the same sync could fail with no such file.
- **A `.lrc` of your own beside a song muman wrote is left alone.** A
  sync that wrote the song again, as a change of `[loudness] mode`
  makes it, deleted or wrote over that `.lrc`, and `undo` did not bring
  it back. It keeps the song from being written again, with a warning,
  as a file of your own at the song's path does, until `sync --force`
  writes over or removes it, kept for `undo` first.
- **`[library] restrict = "none"` names no file Windows cannot hold, on
  Windows.** A device name such as `CON` or `LPT9.mp3`, and a name
  ending in a dot or a space, were kept as they were; they get the `_`
  and lose the dot as under `windows`.
- **A chapter's title cannot add chapters to the song made from it.**
  A source's chapter title holding a carriage return ended its line in
  the chapter list ffmpeg reads for an MP4, so the rest of the title was
  read as lines of the list, and could forge a chapter. A chapter title
  keeps no control character, a line break or a tab read as a space,
  and a carriage return is escaped in the list as a line break is.
- **A provider turned off is asked nothing.** With `enabled = false`,
  `sync` still fetched its records gone from the store again, `coverart`
  searched MusicBrainz and `acoustid` fetched its recordings from there
  with `musicbrainz` off, and `add` looked each video's YouTube Music
  track up with `youtube-music` off. `acoustid` is now off while
  `musicbrainz` is, and `coverart` then finds a cover by the IDs a
  song's tags name alone.

### Security

- **A file is never followed to the network by ffmpeg.** ffmpeg and
  ffprobe pick how to read a file by what it holds, not its name, so a
  song file, cover or torrent's file holding a streaming playlist, as a
  DASH manifest, made them fetch every address it listed, `--offline`
  or not, and one holding a concat script opened the files it named.
  Each file is now read only as one of the audio, picture and subtitle
  formats muman takes; any other is refused before it is read further.
- **A path is never read as an address.** A file given to `add` by a
  path relative to the current folder, as `http:host?x.flac` or
  `concat:a|b`, reached ffmpeg as an address to fetch or join; every
  path is now made absolute first.
- **A picture's name is never read as a pattern of other files.** A
  cover named as `cover%d.jpg` was read as the numbered pictures beside
  it, as `cover1.jpg`; it is now refused.
- **A server's `Retry-After` date far in the future no longer ends the
  run.** A date past the year 9999 crashed the lookup step, on every
  run after too, as nothing was recorded; it is now no date, and the
  request is asked again as after any refusal.
- **A lookup asked over https stays on https.** A redirect from an
  https service to a plain http address, where the lookup would go on
  unencrypted, fails that request. A server the song list names with
  `http://` is followed as before.
- **A crafted `.torrent` file is refused rather than crashing muman.**
  A piece length past libtorrent's largest, files running past 256 TiB,
  a v2 piece length that is no power of two from 16 KiB, or a count of
  piece hashes that does not match the files refuse the torrent, where
  muman could stop on an allocation of terabytes or an overflow. A
  piece is checked a chunk at a time, so a long one no longer takes its
  length in memory.
- **A torrent naming a file with a control character or a drive refuses
  it.** A name holding an escape could drive the terminal it was
  printed to, and a name such as `C:x` reads, on Windows, a folder off
  the one `add --from` names. A name of dots and spaces alone, which
  Windows trims to `..` or nothing, is refused too.
- **Lyrics full of unclosed `<` or `{` are read in time linear in their
  length.** Lyrics from LRCLIB, a subtitle, a tag or a file of tens of
  thousands of them took seconds to minutes, and are read as before.
- **A picture of more than 8192 × 8192 pixels is not measured.** A
  small PNG of one color can hold a picture of a hundred megapixels or
  more, which took over a gigabyte of memory and nearly a minute to
  measure; it is now no picture, as one that does not decode is.
- **A tag file gives a name at most 64 values and a song 256 names.**
  A file of tens of thousands of them took half a minute to read; the
  rest are reported as skipped, with at most 64 reasons.
- **A file muman stages beside a song in the library follows no link.**
  The `.part` a song is written to, by muman or by ffmpeg, is made anew,
  whatever was at its name removed first, so a link someone who may
  write the library folder left there no longer sends the write
  elsewhere, and a staged file that is no longer a plain file is not
  put in place.
- **The library's lock is kept in a folder of your own,** under your
  cache folder and private, rather than in the system's temporary
  folder, where another user could make it first as a link, to have
  muman make or empty a file of their choosing, or hold it, to keep
  every run waiting. Two users who share one library folder no longer
  keep each other out of it.
- **A `state.json` or a run's record naming a path outside its folder
  is refused**, naming the file, rather than have a sync or `undo`
  remove, move or copy a file through `..` or, on Windows, a drive.
- **`add` copies nothing a link leads to outside the folder given.** A
  link inside a folder given to `add`, or beside a file given, to a file
  or a folder outside it, as a `Song.lrc` linked to a key in your home,
  is left out and named rather than copied into the sources and written
  into the library; one leading inside the folder is followed. `add
  --from` takes up a torrent's files alike. A file or folder named on
  the command line is still followed wherever it leads.
- **Text from elsewhere cannot drive the terminal.** A video's or a
  playlist's title, a torrent's name, a line yt-dlp wrote or a
  service's error could hold escape sequences, which reached a terminal
  as they were: one could retitle the window, write the clipboard,
  show a link to one address that opens another, or clear the screen
  and say what muman did not. Every message, a status line's progress,
  `list`'s lines and a prompt's items show such a character as `�`, the
  rest of its sequence as text, and so too a mark that would turn the
  rest of a line around; muman's own colours are kept.
- **A song list you are given runs nothing you have not trusted.** Its
  `[[hook]]`s, `[tools.*] command`, `[ytdlp] args` and `extractor_args`
  (yt-dlp's `--exec`), `[torrent] args` (aria2c's
  `--on-download-complete`), `[providers.*] url` and `key` and `[sites]`
  fetch addresses run programs and send requests as you, and a song list
  from someone else, or one inside an export's zip, ran them unasked, a
  `[tools.ffprobe]` wrapper even on `muman info`. muman keeps what you
  trusted in `state.json`, by value, so formatting, comments, songs and
  settings at their defaults never ask, and asks again when one changes,
  by hand or by an `undo` putting it back. Each is shown escaped, so no
  control character hides part of one. The docs say what each can do.
- **A login in the song list stays off the command line.** Any user of
  the machine could read every yt-dlp run's arguments, `[ytdlp] args`
  and `extractor_args` among them, with `ps`. They now reach yt-dlp in a
  config file only you can read; of `[torrent] args`, a login, password,
  header, proxy or tracker reaches aria2c the same way.
- **`--verbose` hides logins, passwords, headers and tokens** in each
  command it says, which it printed as given.
- **An AcoustID error names no key.** An answer that did not read was
  reported with the request's whole address, the AcoustID key and the
  song's fingerprint in it, printed and recorded in `state.json`; an
  error names a request without its query.
- **A private tracker's passkey stays out of aria2c's errors.** An error
  naming a tracker's announce address, printed and recorded in
  `state.json`, keeps its scheme and host alone.
- **A new home is yours alone.** On Linux and macOS the home muman
  makes, the run history's folders and each run's record, which holds
  the song list's text, are made readable by you alone, as is each run's
  scratch folder. A home made before keeps its permissions; `chmod -R
  go= <home>` makes it yours alone.
- **A fetched source keeps only what muman reads of its info JSON.** It
  kept the address of every format, fragment, thumbnail and subtitle,
  which YouTube signs with your IP address and a PO token, beside the
  cookies and headers it already dropped.
- **A source that cannot be cleaned is not kept.** A file whose info
  JSON could not be cleared was kept anyway, with a warning; its fetch
  now fails, to be made again, and a file given to `add` is not added.

## [0.7.0] - 2026-10-21

### Added

- **A song may list a file of a torrent.** `torrent:<hash>/<path>` names
  a file by its torrent's hash and its path inside the torrent; the
  torrent's metainfo and files are kept under `sources/torrent`, and
  `purge` deletes what no song lists. A song list that lists one is
  format version 4, which an older muman refuses.
- **`add` fetches a torrent's songs.** Given a magnet link or a
  `.torrent` file, it fetches the song files with aria2c, with the lyrics
  and pictures they bring, and lists each as a song; of two files kept
  at one place, as `song?.mp3` and `song？.mp3`, the first; the song list's
  `[torrents]` records what fetches the torrent again, without a private
  torrent's trackers. `sync` fetches a listed torrent's file gone from
  the store again, from its kept metainfo or from its hash and recorded
  trackers. `add --from <FOLDER>` takes up a torrent another client
  downloaded, checking it piece by piece and fetching only what is
  missing or wrong. A torrent's file found damaged, or holding other
  audio than its song was built from, is fetched again as a damaged
  download is, only its pieces that do not check fetched.
  Torrents given together, and those a `sync` fetches again, are
  fetched by one aria2c run, up to eight at a time, so one no peer
  seeds stalls beside the others rather than before them; a `sync`
  fetches them beside yt-dlp's fetches rather than after them. A fetch
  that does not finish keeps each file whose pieces all check, so one
  piece no peer holds right does not keep the rest of a torrent out.
  While a listed torrent's metainfo is gone from the store, its song
  files stay sources, `purge` keeps its folder and `check` says so.
- **aria2c is one of the programs muman runs**, to fetch torrents:
  `[tools.aria2c]` and `MUMAN_ARIA2C` name it as the others are named,
  and `[torrent]` sets the arguments every run takes, how long a fetch
  waits with nothing arriving, 1 s or more, how long a fetched torrent
  is seeded and how long an unfinished one is kept. Only a run that
  fetches a torrent needs it.
- **Jamendo and Audius are sites of their own.** A Jamendo track is
  `jamendo.com:<id>` whether it was found on `jamendo.com` or
  `licensing.jamendo.com`, and an Audius track is `audius.co:<id>`; a
  key an earlier muman named by the licensing domain is renamed by a
  sync, its file moved, as `[sites]` `aliases` lists it. Each
  is fetched again by its ID, an Audius track though its page's address
  changed with its title, and a track of an album or playlist already
  kept is skipped without being fetched.
- **Dailymotion and Niconico videos are fetched again from their own
  pages,** `dailymotion.com/video/<id>` and `nicovideo.jp/watch/<id>`,
  rather than from a player embedded on another site, and a key written
  into the song list by hand can be fetched.

- **A signal that stops muman stops every program it started.** A
  `kill` or a service manager's stop reached muman alone, and left
  aria2c fetching into the home; now ffmpeg, yt-dlp and aria2c stop
  with it, and muman exits 130, as after a Ctrl-C. aria2c stops too
  when muman is killed outright. While `edit` runs your editor, a
  Ctrl-C is the editor's.
- **Shell completion.** `muman completions <SHELL>` writes a script
  that completes muman's commands, flags, their fixed values and paths
  in bash, zsh, fish, PowerShell or elvish; its help says how to load it
  in each.
  The Homebrew, AUR and Nix packages install it for bash, zsh and fish.

### Changed

- **Niconico's comments and Bilibili's danmaku are not fetched.**
  `[ytdlp] sub_langs` leaves them out with YouTube's live chat: they
  are viewers' comments listed as subtitles, and each was left behind as
  a file of its own. `sync --update-defaults` moves a song list still at
  the old default.

### Fixed

- **A file of an archive.org item has its item's artist and date
  however it was added.** yt-dlp gave the item's creator, dates,
  licence and description to a file added by its own address but not to
  one added with the whole item, which came in as "Unknown Artist" with
  no date, and whose tags changed when a sync fetched it again by its
  own address. A file added by its download address,
  `archive.org/download/<item>/<file>`, is that file of the item: yt-dlp
  read it as a plain file of a mirror, with no artist, keyed by its name
  alone, and a sync could not fetch it again.
- **An archive.org file whose name holds a space, `#`, `?`, `%` or a
  letter outside ASCII is a source.** `add` said "yt-dlp listed nothing"
  for one and dropped what it fetched. Its key holds the name as it is,
  as `archive.org:<item>/01 Lantern #2.mp3`, as a torrent's file's key
  holds its path, and its file in `sources/yt-dlp` writes in hex what a
  file's name may not hold on every system. Such a file yt-dlp fetched
  elsewhere and given to `add` goes into `sources/yt-dlp` under its key,
  where it went into `sources/manual`.
- **An archive.org file whose name holds a `+` is fetched again.** The
  address a missing source is fetched from percent-encodes its ID, as
  archive.org read a `+` in it as a space and found no such file.
- **A source whose name is too long for a file's is kept.** An
  archive.org file named in percent-escapes, whose `%`s the store
  escapes again, ran past the 255 bytes a name may take, and its song
  was dropped, the run exiting 0 and a later `add` saying "Nothing new".
  A name too long is cut into folders, and a torrent's file shortened by
  its hash; a fetched file the store still cannot take, as one apart from
  another only in case where the filesystem is blind to it, is said as
  an error and the run exits 4.
- **What yt-dlp fetched but could not embed is not left behind.** A
  subtitle in JSON, as Niconico's comments under a `[ytdlp] sub_langs`
  that asks for them, stayed in the store's hidden staging folder, which
  no `purge` or `check` saw; each fetch now stages its files in a
  scratch folder of its own, which goes when the fetch ends.
- **A source damaged on disk is fetched again by the sync that finds
  it.** A sync measured after its fetches, so a file damaged since it
  was measured failed its song in that sync and came again only in the
  next; it is measured before them now.
- **A fetch stopped while yt-dlp remuxed leaves no song cut short.**
  Stopped mid-remux, ffmpeg left the `.mkv` short beside its whole
  original in `partial/`, and the next run took it for finished: a song
  ended early that `check --decode` passed. Such an `.mkv` is now
  deleted before a fetch and the original remuxed again, and a fetched
  source whose audio ends well before the length its site states is
  damaged, fetched again as a cut-off download is.
- **A removed song's video added alone by URL is listed again on every
  site,** as on YouTube; on another site `add` said "Nothing new".
- **A key an earlier muman named by an extractor `[sites]` lists, as
  `jamendo:<id>`, reads as that site's,** `jamendo.com:<id>`, in the
  song list and in a query, as `youtube:<id>` does, where a sync renamed
  it by its page's domain, as `licensing.jamendo.com`, and a query found
  nothing by it.
- **`check --upstream` takes a site that lists no formats, as Audius,
  for serving the one it names,** rather than saying every source of it
  is no longer served.
- **`status` says a song whose source a sync would fetch is made once
  fetched,** rather than that it failed and exiting 4.
- **`info` counts a notice about the song list, as an old default or a
  misspelled table, as no song that cannot be made.**

## [0.6.0] - 2026-10-20

### Added

- **`export` writes the library under other settings.** `--set
  table.key=value` changes any setting of `[library]`, `[audio]` or
  `[loudness]` for the export alone, and `--codec`, `--template` and
  `--lyrics` change the commonest: `--codec mp3` writes every song in
  MP3. A song the change leaves alone is copied from the library; every
  other is made again from its sources, as a sync under those settings
  would write it.
- **`export --folder`** writes what the zip would hold into a new or
  empty folder, for a card or a player. It is made beside the folder and
  renamed into place, and `--max-size` counts each file in whole blocks
  of `[library] block_size`.
- **The song list names the programs muman runs.** `[tools.ffmpeg]`,
  `[tools.ffprobe]` and `[tools.yt-dlp]` each take a `command`, a name,
  a path or a program with its arguments, so a fork that takes the same
  options runs in a tool's place on every machine; `MUMAN_FFMPEG`,
  `MUMAN_FFPROBE` and `MUMAN_YT_DLP` still override it on one.
  `version_check = false` runs a release older than muman asks for, or
  one numbered its own way.
- **`[ytdlp] extractor_args`** sets yt-dlp's `--extractor-args` for each
  fetch, by extractor, in place of the ones muman passed YouTube as
  fixed.

### Changed

- **`export` takes less room while it writes.** Each song encoded for
  the zip is deleted from the scratch folder once it is in.

- **yt-dlp fetches a video's audio alone.** `[ytdlp] format` is
  `"ba/b"`: muman never read the video itself, which only took room in
  the store. A source fetched again still asks for the format it was
  fetched in.
- **Vorbis, AAC and MP3 are copied as they are**, beside Opus and FLAC,
  in `[audio] codecs`. Encoding them to Opus lost a generation, and made
  a 128 kb/s MP3 a larger file at 160 kb/s.
- **Real stereo ranks before bandwidth.** `[quality.stereo] weight` is
  5000, so a mono copy no longer beats a stereo one for a few kHz more
  treble.
- **These three are settings edition 3.** A song list still at an old
  default keeps it; `sync`, `status` and `check` name it, and `muman
  sync --update-defaults` moves it. A value you set stays.
- **`[ytdlp] args` reach every yt-dlp run**: the listing of a URL, the
  YouTube Music lookups and the audio a comparison downloads, as well as
  the fetch, so a login or a JavaScript runtime given there serves them
  all.
- **With `[ytdlp] plugins = false`, YouTube is asked with its
  `web_music` client too**, as `extractor_args` says; remove it there
  to ask without.

### Fixed

- **A release that names its version is found for its upload.** A
  YouTube Music track named `Song (2011 Remaster)`, `Song (Original
  Version)` or `Song - <the name in another script>` was passed over for
  an upload titled `Artist - Song`, though its audio was the same
  recording. Its name now matches without what it adds, unless that
  names another recording, as `(Live)` or `(Remix)` does. Uploads that
  found no release are looked up again once.
- **A short upload matches its release.** Comparing two recordings
  counted the windows past the release's edges as disagreeing, though
  they could not agree; in a clip of half a minute they were a third of
  the windows, so a clip with an intro or outro was never the same
  recording as its track. Every comparison is made once again.

## [0.5.1] - 2026-10-18

### Added

- **Each release lists its archives' checksums** in `SHA256SUMS`, and
  attests that this repository's release workflow built them, which
  `gh attestation verify` checks.

### Changed

- **The Linux binaries run on glibc 2.28 or newer**: Debian 10, Ubuntu
  20.04 and RHEL 8 or later. They needed glibc 2.39, which Debian 12 and
  Ubuntu 22.04 lack.
- **The Windows binary carries its C runtime**, so it runs on a Windows
  without the Visual C++ Redistributable.
- **Measuring is faster.** The true peak is scanned only where a run of
  samples could lift it, through the same filter as before, bit for
  bit, where every sample was filtered; and the stereo and bandwidth
  measures sum as they did in a third less time. Over 38 public-domain
  recordings, 2.9 hours of FLAC, on four cores, `add` took 35 to 39 s
  where it took 38 to 43 s, its measuring step 23 or 24 s where it took
  27 or 28 s, and recorded the same measures to the byte.
- **Adding thousands of files is faster.** `add` read the library once
  for each file it copied in and listed a file's folder once for each
  sidecar it looked for; it reads the library once after the copies and
  lists each folder once, and asks ffprobe about the files in parallel.
  Copying 2,038 files in took 7 s where it took 165 s.
- **A FLAC's tags and cover go into padding.** ffmpeg leaves room for
  them in the file it encodes, 16 KiB and the cover's size, and they are
  written into it; a FLAC was written whole a second time to tag it.
  One that does not fit is still rewritten.

### Fixed

- **A long `add` or `sync` no longer fills the disk.** The excerpts and
  prints ffmpeg wrote while measuring a source, 15 MB of them, stayed in
  the temporary folder until the run ended: 299 sources in, they took
  2.9 GB, and 2,038 sources ran the disk out. Each source's are removed
  when it is measured.

## [0.5.0] - 2026-10-18

### Added

- **A floor for each program.** ffmpeg and ffprobe older than 5.1, or
  yt-dlp older than 2025.03.21, the oldest releases that take every
  option muman passes, stop a run with exit code 2 and a message naming
  the version found and where a newer one comes from, as a missing
  program does; so does one found that does not run, as a `yt-dlp` whose
  Python is gone. Each was tried on every song or fetch and failed with
  an error naming an option.
- **`info` names each program's version and path**, or why it cannot
  be run.
- **A fetch a newer yt-dlp would make says so.** When yt-dlp fails with
  a sign-in check, a failed signature extraction, a format that is not
  available or a 403, the run says to update it, naming the version
  that ran.
- **A fetch survives postprocessors that do not load.** A yt-dlp that
  refuses muman's plugin folder or does not know its postprocessors is
  run once more without them, with a warning, where it fetched nothing.
- **Built binaries for each release**, for Linux, macOS and Windows, on
  the releases page.

### Changed

- **Servers are verified by the system's certificate store**, as the
  platform's own programs verify them, so a proxy or an authority the
  machine trusts is trusted by lookups too. They were verified by roots
  built into muman alone, which a machine's own authorities never
  reached.
- **Thirty more tags**, read from every source's own tags and from tag
  files, set by hand, and written where Picard writes each in every
  format: tempo (`bpm`), key, compilation, composer, lyricist,
  conductor, remixer, label, catalogue number, barcode, ASIN, artists,
  the sort names, original date, release type and status, media,
  script, disc subtitle, mood, grouping, work, language, copyright,
  comment, and the MusicBrainz work and disc IDs. A tempo is held
  whole in MP3 and M4A, a compilation as M4A's flag, and a key in
  `TKEY` where that frame holds it.
- **Release details from MusicBrainz**: the release type and status
  and the medium from the answers muman already asks for, and the
  label, catalogue number, barcode, ASIN, script and original date,
  its release group's first release as Picard writes it, from one
  lookup of the release, asked once a run for every song of an album.
  A lookup of it that fails is made again on a later run.
- **A song's language is its lyrics'**, where no source says another,
  and MP3 lyrics carry it.
- **`composer`, `bpm` and `key` in the path template.**
- **Chapters are kept.** A source's chapters, as an M4B audiobook or a
  video in parts has them, are written into its song's file: MP4
  chapters, `CHAPTER000` comments in FLAC, Ogg and Opus, and `CTOC` and
  `CHAP` frames in MP3; WavPack holds none. They were dropped. A song
  whose source has chapters is written once again.
- **`[library] max_path` counts characters as Windows does**, in UTF-16
  units, so an emoji counts two; a path of such characters is cut
  shorter than before.
- **`restrict = "none"` replaces `\` too**, which Windows reads as a
  folder, so a library made on Linux copies to Windows whole.
- **Every song is tagged once again on the first sync.** MP3, M4A and
  WavPack files are tagged as Picard tags them; their audio is not
  written again.
- **A tag set by hand is read as its field holds it.** A track or disc
  set as `3/12` writes the number and the total, a date as
  `2019/07/04` writes `2019-07-04`, an ISRC is written without its
  dashes, `true` is written `1`, and an unquoted TOML date is read as
  its text. A value its field cannot hold, as a date of `July 2019` or
  `2019-02-30`, a track of `fast`, a ReplayGain gain, which muman
  measures, or a table, refuses the song list, naming the song and its
  first source, where it was written as it was or skipped; so does a tag
  name of other than printable ASCII, which FLAC and WavPack dropped.
- **Sources offer only values their fields hold.** A source's date that
  reads as none is no longer written as it was; its tags are read again
  on the next sync.

### Fixed

- **A home on a filesystem without extended attributes fetches.** yt-dlp
  was asked to write each file's source URL into its extended
  attributes, and on a filesystem with none, as some network and
  removable drives and sandboxes have, failed the whole download after
  fetching it. muman reads nothing from them: the URL is in the info
  JSON each file embeds.
- **Opus of three, four or more than eight channels is written.** A
  source in a layout no Vorbis layout names, as 2.1, 3.1 or 7.1.4,
  written to Opus failed on every sync: its tags could not be written
  into the file ffmpeg made.
- **An info JSON with one odd field is still read.** A number written
  as text or a `null` among a video's artists made muman ignore all of
  yt-dlp's info JSON, so the file was added as a manual source with no
  tags; now only that field is lost.
- **Subtitles in a legacy encoding keep every line.** An `.srt` or
  `.vtt` given to `add` in an encoding other than UTF-8, as Windows-1252
  or Shift-JIS, lost each cue ffmpeg could not read as UTF-8; it is
  decoded as lyrics files are before it is converted.
- **Surround sources are mixed, not folded, into MP3 and AAC.** Audio
  with more channels than MP3's two or AAC's eight was left to the
  encoder to fold in, unscaled, and its loudness tags were the
  source's, several dB off what plays. It is mixed into the first
  `[audio] downmix` layout the codec holds and measured as mixed; AAC
  in 5.1 with side speakers, or of channels no speakers are named for,
  as an Opus of mapping family 255, which failed every sync, names them
  in the standard layout. Only such songs are encoded again.
- **An MP3 with a date of no calendar can be tagged again.** A date
  that is none, written into an MP3, made every later change to the
  song's tags fail.
- **A song file's own lyrics are its lyrics.** Lyrics a source holds in
  its tags, `USLT` in MP3, `LYRICS` or `UNSYNCEDLYRICS` in a Vorbis
  comment, `©lyr` in M4A and `Lyrics` in APEv2, were never read, so a
  song whose only lyrics were embedded was written without any. They
  are lyrics of that source, ranked as any are, timed when they are
  LRC, in the language a `USLT` frame names. Every source's tags are
  read again once to find them.
- **Sung lines in brackets or shaped as credits are kept.** Any line
  wholly in parentheses was dropped as a cue, and any line starting
  `Music by`, `Mix:` or the like as a credit, so backing vocals and
  lines such as `Music by the water` were lost. A bracketed line is a
  cue only when its words are, as `[Chorus]` or `(Guitar solo)`, and a
  credit only before the first line sung or after the last, with a
  name after its word.
- **A name cut inside an emoji ends on no joiner.** With a small
  `max_name_bytes`, a character longer than the room alone, as a family
  emoji, was cut between its code points and could leave the name
  ending in a lone zero-width joiner or variation selector; these are
  dropped from the end of such a cut.
- **A sidecar named as its song in another case comes with it.** A
  picture or `.lrc` named as a manual song file but in another case,
  as `Song.jpg` beside `song.flac`, was not taken, though a folder's
  `COVER.JPG` was, nor copied by `add` with its song. Sidecars are
  matched in any case, one in the song's own case first.
- **`add` names the files it does not read and adds the rest.** A file
  given that muman reads nothing from, as a `.pdf`, stopped the whole
  `add` with nothing copied, and a folder's such files were copied into
  `sources/manual` silently, where nothing read them. The file is named
  and left out, the songs given with it are written all the same and
  `add` exits with 4; a folder's are left out with a warning naming
  them.
- **A track set as `5/9` keeps its number.** MP3, M4A and WavPack wrote
  track 0 of the old total, and FLAC the number and a stale total.
- **`album artist` names the album artist** in a tag file, as foobar2000
  writes it, rather than adding a tag beside it.
- **The state file is read back exactly, and written only when it
  changes.** A measure read from it could differ from the one written
  in its last bit, so the first sync after an `add` wrote the file
  again; and a sync with nothing to do wrote it anew all the same.
- **A `work` tag no longer fails an MP3.** Set by hand or from a tag
  file, it was written under a frame that holds no text, and every
  write of the song failed.
- **MP3 keeps every value of a tag.** Several ISRCs, labels or sort
  names were cut to the first.
- **ReplayGain in M4A is named as Picard names it**, in capitals, and
  read from M4A files Picard tagged.
- **A WavPack cover is named by its format**, so ffmpeg and players
  that read the item's file name show it.
- **MP3 lyrics carry their language** where the song's `LANGUAGE` tag
  names it.
- **A picture is known by its bytes, not its name.** A PNG named `.jpg`
  was embedded as a JPEG, and a WebP named `.jpg` as a JPEG atom no
  player reads; each is written as what it is.
- **A WavPack song keeps its cover when its tags change.** Changing a
  tag of a WavPack song wrote it again without its cover, and no later
  run noticed; every file written is now read back, and a tag or a cover
  it does not hold as written is told.
- **A cover past what a container holds is made smaller in every
  one**, not only in FLAC: a large PNG made ffmpeg drop every tag of a
  WavPack file, its lyrics counted, and a cover past 12 MiB, a third
  larger as Ogg holds it, made every later tag change of an Opus or Ogg
  Vorbis song fail.
- **Lyrics move whole.** Moving lyrics to another cut of the
  recording moved each line's time but not its words', which then came
  before their own line; an `[offset:]` was kept for players to apply
  and ignored where a line starts before the track; times were cut to
  the hundredth rather than rounded; a `[length:]` no longer true was
  kept; and a timed line of no words, which ends the line before it,
  was dropped.
- **A name that composes longer still fits.** A name was cut to its
  byte limit before it was composed into Unicode NFC, which can lengthen
  it, so a Devanagari title could pass 255 bytes and fail on every
  sync; `max_path` cut names inside a flag or an emoji.
- **Names Windows holds as one are told apart** by case as NTFS folds
  it: `ΟΔΟΣ` and `οδοσ` were written as two files where Windows keeps one.
- **Cue sheets as rippers write them are read.** A sheet of one file a
  track, its pregaps in the file before, as EAC writes it, was refused;
  a `FILE` path with `\` matched no song outside Windows; a name left
  unquoted was cut at its first space; and a data track counted as a
  song. A sheet of a mixed-mode CD, its data first, gives no track a
  number past its total.
- **A song past an hour gets its lyrics.** LRCLIB refuses a duration
  past 3600 s, and its refusal ended the lookup; such a song is now
  found by search.
- **Lyrics streams in more languages are found** where a file names
  them by ISO 639-2, as Swedish `swe` or Hebrew `heb`.
- **Exported files keep their time.** The zip's times were read as the
  extracting machine's local time; each entry also carries its time in
  UTC, as Info-ZIP, 7-Zip and libarchive read it.
- **yt-dlp's list of every artist on an album is not its album artist**;
  only a list of one is.
- **A back cover is no song's cover.** A source's back cover, disc or
  booklet could be embedded as the front cover when it was the sharper
  picture; it is taken only when no other picture is, and a picture
  typed only "other" is no cover beside one typed the front cover. A
  picture named as its song is a front whatever the song's name, as
  `Back Home.jpg` beside `Back Home.flac`.
- **A source whose sound sits away from its middle is measured.**
  Audio was judged on three short excerpts at a quarter, half and three
  quarters of its length, and a track whose song ends long before a
  hidden one, all three silent, was judged to have no sound and ranked
  after every other source. Where all three fall silent they are taken
  again where the whole decode found sound; sources judged soundless
  before are measured once again.
- **A picture cut short is no cover.** A JPEG or PNG that stops before
  its end was measured by the size its header gives, as sharp as it
  was made, and embedded with the rest of it gray, beating a whole
  smaller cover. A picture that does not decode whole now ranks after
  every one that does, and is never embedded.
- **An M4A or M4B cover ffprobe cannot see is found.** A cover stored
  in its `covr` atom with no type, as some audiobook tools write it,
  was never seen, and the song was written without one; it is read
  from the atom. Sources measured before keep what they had until they
  are measured again.
- **Several artists are listed in `ARTISTS`** as Picard lists them,
  beside `ARTIST`, which holds them joined; the list was lost.
- **An MP3 of a low sample rate is planned at the bitrate it holds**:
  64 kb/s below 16 kHz and 160 below 32 kHz, which LAME writes whatever
  it is asked, so `max_size` counts what is written.
- **24-bit audio in WavPack is named 24-bit.** It was named 32-bit,
  as ffmpeg hands it to the encoder; its samples were whole either way.
  Only such songs are encoded again.
- **A cover is looked up after the release it is of.** The Cover Art
  Archive was asked in the same round as AcoustID and MusicBrainz,
  before the song had a release's IDs, so it searched for the album by
  its names: it often found none, then waited out `recheck`, or found
  another release's cover. It now waits for those lookups, and a cover
  lookup that found nothing before is made once again.
- **A song's MusicBrainz IDs never contradict.** A file's own recording
  ID was kept beside the release IDs of another recording a lookup
  found; the recording IDs now come with the release's where its
  source names one.
- **A name told apart from another keeps within `max_path`.** The ID
  added to a name two songs share could take the path past it.
- **Subtitles converted to lyrics lose their markup**, as `<i>`, and an
  `.lrc` in UTF-16 without a byte-order mark is read in any script, not
  only one mostly in Latin letters.
- **M4B audiobooks, MP2, Speex, WMA and Sun `.au` files are audio** to
  `add`, which took them for files it does not read.
- **A network that is down puts lookups off.** A lookup that reached no
  server, its name not resolving, its connection refused or never
  opening, or a proxy refusing to connect it, waited out its whole
  timeout, as did every other due one after it, and each was recorded
  failed, to wait an hour, doubling to a week. The first now puts its
  provider's other lookups off to the next run, recorded as nothing, as
  a refusal for going too fast does, and a connection waits at most
  10 s to open. A service that answers with an error still fails that
  lookup alone.
- **A refusal holds back a request already given its turn.** A request
  that took its turn before a service refused for going too fast still
  went when the turn came; it now waits out the refusal too, and once a
  service refuses or cannot be reached for the run, no request waiting
  for its turn is sent.
- **A listed record that is gone is asked for less often.** A song's
  `lrclib:`, `musicbrainz:` or `coverart:` record that its service no
  longer has was asked for on every sync; like any fetch that fails, it
  now waits an hour, doubling, until it is found again or `sync --retry`
  asks at once. One whose service refuses or cannot be reached waits for
  the next run, as a lookup does, and is not counted as failed.
- **A fetched source keeps no cookies or headers.** The info JSON yt-dlp
  embeds in each file held the cookies and HTTP headers its requests
  sent, a login passed with `--cookies` or `--cookies-from-browser` in
  `[ytdlp] args` among them; each file fetched, or given to `add` as one
  yt-dlp fetched, is now written once more without them. Sources fetched
  before are left as they are, cookies and all: `purge` deletes only
  sources no song uses, so to clean one, delete its file and the next
  `sync` fetches it again without them, its song warning if the site
  now serves other audio or tags.

## [0.4.1] - 2026-10-15

### Fixed

- **No yanked crate in the build.** muman 0.4.0 locked zip 8, every
  release of which is yanked, so `cargo install --locked muman` warned
  of it; muman builds with zip 9.

## [0.4.0] - 2026-10-15

### Added

- **Loudness normalization.** Every song is measured whole, as EBU R128
  measures loudness, and levelled to `[loudness] target`, -18 LUFS
  unless set otherwise: ReplayGain 2.0 track and album gains and true
  peaks are written into its tags, R128 gains in Opus, the album's
  measured over all its songs together. `[loudness] mode = "header"`
  also writes the gain into each Opus file's header, which every Opus
  player applies, without encoding anything again; `mode = "audio"`
  scales the samples for players that read no gain. `scope` picks the
  album's or the song's own gain, and `ceiling` the highest peak a
  raising gain may lift a song to. A gain set by hand wins. A mixed
  song is measured as mixed.
- **`add` gives lyrics, pictures, tag files and cue sheets to their
  songs.** Each is matched by an ID it shares with a song, its file
  name, its fields (an `.lrc`'s ID tags, a tag file's, or its own name)
  and what it holds: lyrics by the words the song sings, a picture by
  how the song's cover looks. Names are compared in every script,
  folding accents, width, kana and Traditional Chinese. What is not
  surely one song's is asked about on a terminal, with a search of the
  song list; `--to` names the songs instead. Lyrics and pictures become
  sources of the song; tags from `NAME=value`, ffmetadata, beets or
  MusicBrainz JSON and cue sheets are set on it by hand, each beets
  field under the name Picard writes it as, a credit as "A feat. B"
  giving way to the artists it names, and `--verbose` names what was
  skipped.
- **A damaged download is fetched again.** Audio cut off or missing
  from what yt-dlp fetched is fetched again on the next sync, then less
  often, up to five copies or until `sync --retry`, and a whole copy is
  taken in its place without `sync --accept`.
- **`--offline`, or `MUMAN_OFFLINE=1`,** reaches no network for a run:
  no lookup, no fetch, and URLs refused.
- **`sync` on a new home writes the song list,** every setting in it, so
  lookups can be turned off before the first `add`.

### Changed

- **Fetched sources are kept by their key,** at
  `sources/yt-dlp/<site>/<id>.mkv`, where they were named by their
  uploader and title, which a site may call otherwise from one fetch to
  the next: an unfinished download was left behind rather than resumed,
  and one source could be kept twice, unseen. A file there that is no
  source, as a subtitle yt-dlp could not embed, is named by `info` and
  deleted by `purge`. Sources fetched before stay where they are, read
  as ever. An older muman finds no source fetched in this layout, and
  would fetch those again.
- **The first sync measures every song's loudness and retags it.** It
  decodes each source once, in full, then writes the tags of every
  song again, encoding none: a `written` hook runs for each song, and
  `undo` keeps the files replaced as far as `[history] max_size` allows.
  `[loudness] mode = "off"` keeps the gains a source's own tags carry,
  as before.
- **A picture of your own may be listed by several songs,** as the songs
  of an album share its cover. A song list where one is is format
  version 3, which an older muman refuses. A song whose every source
  another song lists too is refused, since edits would take it for that
  song.
- **`add` exits with 5 when a file it was given was left out,** having
  matched no song for sure with no terminal to ask on.

### Fixed

- **Of two sources whose albums rank alike,** the one holding more of
  the release's fields, as track, date and IDs, gives them all, where
  the first listed did even when it named the album alone.
- **`add Song.flac Song.lrc`** copies the lyrics once, beside their
  song, where it failed saying they existed already.
- **`undo` of a run that gave a song lyrics** removes the `.lrc` it
  wrote beside the song, where it stayed, recorded by nothing.
- **A song's own `.lrc` or picture moved within `sources/manual`** is
  followed by its song, as a moved song file is, where the song lost it.
- **Two songs listing one lookup record** each keep their record of what
  it holds, where the second's was written again on every sync.
- **`remove --purge`** keeps a source a removed song still lists, so
  that `restore` gets the song back whole, where it deleted it.
- **A song list's format version** goes back down once its songs no
  longer need it, on the next `sync`, so that an older muman reads it
  again; it changed only with another edit.
- **`undo` after `remove --purge`** refuses until you put the files of
  your own it trashed back, naming each, where it listed their songs
  again with nothing to make them from, and every sync after failed.
- **`restore` after `remove --purge`** refuses until you put the files
  of your own it trashed back, naming each, where it listed their songs
  with nothing to make them from, and every sync after failed.
- **A song joined to another in `edit`** is said to be joined and left
  out of `[[removed]]`, where it was noted as removed and `restore`
  claimed to list it again, changing nothing. A removed song whose
  sources a song lists again is dropped from `[[removed]]` on the next
  write.
- **A home inside the library** is refused by every command that
  writes, where `add` copied the file in and listed nothing, taking it
  for one of the library's.
- **`add` of a file named with a leading dot** is refused, as muman
  reads no hidden file, where it was copied in and never listed. A
  folder's hidden files, as `._Song.flac`, are not copied, and a folder
  holding no song is named and exits with 4, where `add` said it was
  copied.
- **`info`** counts an emptied library file as to write, and a song
  with no file on disk as one that cannot be made, as `status` and
  `check` say, where it counted both up to date.
- **`set` and `add --tag`** refuse `track=abc`, `date=yesterday` and a
  ReplayGain or R128 gain, which muman measures, where they wrote the
  first two and showed the gain set, writing the measured one.
- **`add <URL>` under `--offline`** is refused with exit 5, as `check
  --upstream` is, before copying any file given with it, where it
  exited with 4 having copied them.
- **`export`** exits with 4 when it leaves out a song missing from the
  library, where it exited with 0.
- **`--progress json`** writes each message as a `message` event, so
  that every line on stderr reads as JSON, where messages were plain
  lines among the events.
- **A query ignores accents and how a name was composed,** so `cafe`
  and `café` match "Café" however it was written, where a name a Mac
  wrote decomposed matched only a decomposed query, and `cafe` none.
- **`duplicates`** puts first and names a group an album holds twice
  when other albums hold it too, where only a group on one album came
  first.
- **Messages:** `status` on a home with no song says so, where it said
  nothing; a removed song is named once by its artist, where `restore`
  named it twice; a source that lost its cover and lyrics has "no
  cover or lyrics"; `info --verbose` names each song with its key, so
  that two of one title are told apart.
- **`export`** dates each entry of its zip by its file, where every one
  read 1980-01-01.
- **`restore album:…`** finds removed songs by their tags and their
  sources', as `list` and `remove` find songs, where it read only the
  name noted when they were removed.
- **A field's value in quotes,** as `'album:="The Glass Orchards"'`,
  matches without them, where the quotes were part of the text.
- **An emptied library file** is written again by `sync`, as `check`
  says, where it was left as changed since written.
- **`check`** names every file in the library muman did not write, as
  `info` counts them, where it named only audio and `.lrc` files.
- **A song file ffprobe finds no audio in** is not copied in or listed
  by `add`, which names it and exits with 4, where it was listed and
  failed every run after.
- **A negative `lyrics_offset`** starts the line still sung as the song
  begins at 0:00, where it dropped it with the lines over by then.
- **A closed pipe,** as `muman sync | head` leaves, drops what the run
  says after rather than stopping it halfway through writing the
  library.
- **`export -o`** into a folder that is not there is refused, where it
  made every folder the path named.
- **`set`** exits with 5 when it refuses a change, as other changes do,
  where it exited with 4.
- **A misspelled settings table,** as `[audoi]`, is warned of by `sync`
  and `check`, where its settings did nothing unnoticed. A misspelled
  `[[song]]`, `[[album]]` or `[[removed]]` is an error, where `sync`
  warned of it and took every song it listed out of the library.
- **An MP4 tag of several values,** as two artists or two genres, is
  one atom holding each, as Picard writes it and taggers read it, where
  each value was an atom of its own. The first sync writes every `.m4a`
  song again.
- **`[library] max_size` and `export --max-size`** raise each song
  lowered back as far as the room left allows, where songs that fit at
  their best stayed lowered and room went unused.
- **`sync --force` under `[library] max_size`** fits a song whose file
  changed since muman wrote it as any other, as the next `sync` does,
  where it counted the file it was about to replace, wrote the song at
  its best, and left the next sync to encode it again.
- **A lowered `[history] max_size`** binds the runs already kept: each
  run that records lets go of the oldest runs' files until the history
  fits, keeping their records so `undo` writes those files again from
  their sources, where old runs kept their files past the limit. The
  run that follows a hand edit of `[history]` keeps to the edit, where
  it kept to the limits before it.
- **`undo` of a run that moved a song and retagged it** without room
  to keep the file has the song written again where it moves back,
  where the file kept the run's tags and the next undo refused it as
  changed since muman wrote it.
- **An excerpt joined to its song** leaves the whole song's audio
  chosen, where the rest of the song counted against it as sound beyond
  the song and the excerpt replaced it. A source that ends where
  another plays the song on, cut off mid-sound, in a short rest or
  faded out, counts the song it lacks against itself. A release that
  starts on a full chord at its first sample still beats its video's
  intro, so an excerpt that starts mid-song and runs to the song's end
  reads as whole. Every pair of sources is compared again on the next
  sync.
- **Silence held off centre by a DC offset,** as an old rip's lead-in
  can be, counts as no sound beyond the song, where it counted as
  sound and the rip lost to another copy of its song. Every pair of
  sources is compared again on the next sync.
- **An excerpt starting past its song's first minute, or a video whose
  intro runs past a minute,** is compared with the rest of its song,
  where no shift past a minute was tried, the two were never found one
  recording, and the excerpt could win on bandwidth. Every pair of
  sources is compared again on the next sync.
- **A copy of a track under about 9 s,** as an album's interlude,
  joins its song, and `duplicates` finds two songs listing it, where
  the two never agreed on the 6 s a match needed and the copy became a
  second song, even byte for byte the same.
- **Lossless audio whose notes end low over a flat noise floor**
  measures its full bandwidth, where a partial standing over the noise
  read as a lowpass at 2 to 3 kHz and the song's own lossy copy won.
  Every source is measured again on the next sync.
- **`[loudness] mode` changed between `tags` and `header`** writes
  again only the Opus songs, whose headers it changes, where it
  reported every other song's loudness updated and kept each in
  history, its bytes the same.
- **A gain `[loudness] ceiling` leaves nothing of,** in an Opus header
  or the samples, leaves a song alone as tags alone do, where its file
  was written again the same.
- **A library file retagged by a tagger that keeps its size and time**
  is left alone as changed since muman wrote it, its bytes compared
  with those written as it is about to be written over, moved with new
  tags or removed, where the next change to its song wrote over it.
- **An `undo` stopped partway** stays the latest run until the sync it
  ends with is done, so the next `undo` finishes it, where one stopped
  in that sync had dropped the run's record: the next `undo` undid the
  run before it as well, and `check` found no problem. `check` names
  it, and a run begun meanwhile finishes it.
- **The `changed` hook** runs after a sync that only moved files and
  after an undo that only put files back, where it ran only once a file
  was written or removed; `{moved}` counts the files moved, and an
  undo's files put back count as written.
- **`undo` of `sync --force`** puts back a file you had changed since
  muman wrote it as still changed, where it took it for muman's own,
  `check` found nothing and the next change to its song wrote over it.
- **A sync stopped while writing new files** after a hand edit of the
  song list can be undone, the edit with it, where `undo` refused,
  saying the song list had changed, as the run had written no record
  before its first new file.
- **`undo` of an `add`** takes out the copy it made in `sources/`, as
  `undo -n` says, where the copy stayed and the next sync listed the
  song again as a file dropped in. A copy changed since goes to the
  trash.
- **A source cut off since its song was built,** as a copy stopped
  short leaves a file of your own, leaves the song's file as built and
  fails the song, exit 4, until the whole file is back or `sync --force`
  writes it, where the song followed it, its library file replaced by
  the stub. `check` names every source cut off.
- **The library's lock file** in the system's temporary folder is
  removed as each run lets go of it, where one stayed for every library
  folder muman ever wrote.
- **A step with nothing to do** shows nothing, where plain progress
  said `[5/5] Writing: 0/0, done in 0 s`.
- **A setting added to an older song list** comes without the
  commented-out hint for another setting the table sets or hints at
  already, by its name or an older one, where `[audio]` got a second
  `# min_bitrate` hint.
- **The time left a step says** follows the pace of its last minute,
  where it was the pace since the step began: after a run of cheap
  items it said `about 1 s left` with twenty minutes to go, then rose
  for the rest of the step. Measuring counts the audio sources alone,
  as lookup records and pictures take a moment each.
- **A subtitle running past the audio is no cut-off.** A source was
  judged cut off by the file's length, which counts its subtitles too,
  so most videos with subtitles were: `check` named each and failed,
  and their songs were encoded again where the audio would have been
  copied. The audio stream's own length is read now, and those songs
  are copied on the next sync.
- **Damaged audio is found.** Each source's decode is measured against
  its packets: audio cut off, its packets ending before its stream says,
  and audio missing, as holes a download that lost pieces leaves or
  packets too corrupt to decode, which passed as whole. `sync` warns,
  `check` names each, `info` counts the songs, and a source missing
  audio ranks below any whole copy of the recording. Sources measured
  before are checked once, on the next sync, by one decode each.
- **A library folder moved or copied keeps its files muman's.** Named
  anew, by `--library` or `[library] path`, a library left every file
  there alone as not muman's, so `set` and `remove` no longer touched
  them, and only `sync --force`, writing every song again, took them
  back. Each file at its place as muman wrote it, by its size and time
  or, copied with new times, by its bytes, stays muman's now.
- **`info`** counts the songs up to date, to write, to move and left
  alone by the decisions a sync makes, as `status` shows them, where it
  judged them its own way: after a library folder was named anew it
  said every song would be written there anew and counted them up to
  date.
- **A subtitle of only `♪` is no lyrics,** where it was picked as a
  song's lyrics, untimed, though nothing was written: `info` counted the
  song as having lyrics and `list lyrics:yes` matched it. Sources
  measured before are read again once, their subtitles alone.
- **An original yt-dlp fetched, inside a folder `add` is given,** goes
  into `sources/yt-dlp` under its key, as one given alone does, where it
  was copied into the manual folder as a file of your own; one of a key
  the store holds already is not copied again, where a second copy was
  kept under the name of the folder it came from.
- **`check --decode`** measures each source's audio afresh and names
  what is cut off or missing, where it named any line ffmpeg wrote, a
  warning from its muxer about timestamps among them.

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

[Unreleased]: https://github.com/jeggao/muman/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/jeggao/muman/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/jeggao/muman/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/jeggao/muman/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/jeggao/muman/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/jeggao/muman/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/jeggao/muman/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/jeggao/muman/compare/v0.2.3...v0.3.0
[0.2.3]: https://github.com/jeggao/muman/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/jeggao/muman/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/jeggao/muman/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/jeggao/muman/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/jeggao/muman/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/jeggao/muman/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jeggao/muman/releases/tag/v0.1.0
