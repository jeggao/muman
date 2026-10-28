# Command-line reference

<!-- Generated from the clap definitions in src/cli.rs by `cargo xtask docs`; edit those, not this file. -->

## `muman`

Keep a music library made from YouTube and files of your own, each song from the best of its sources.

songs.toml in the home folder lists every song and the sources it may be made from: what yt-dlp fetched, kept whole in sources/yt-dlp, and files dropped into sources/manual. Each source is measured — its audio's real bandwidth, stereo and clipping, how much of it is not the song, its pictures' content and real detail, its lyrics, its tags — and for each song the best audio, cover, lyrics and tags are picked from among them by those measures alone. The song is written to the library where the song list's [library] template puts it, <album artist>/<album>/<track title> by default, its audio copied or encoded, tagged and covered, with its lyrics. A song is written again only when what it is made from changes, and a file muman wrote that no listed song makes any more is deleted.

A new source the same recording as a listed song, by its fingerprint, is added to that song; one that may be is asked about.

**Usage:** `muman [OPTIONS] <COMMAND>`

```text
Exit codes:
  0  Every song was written, or nothing needed to be; or a change was
     declined
  2  yt-dlp, aria2c, ffmpeg or ffprobe is missing, does not run or is
     older than muman runs, the system names no home folder, or the
     command line does not parse or gives a setting that does not read
  4  yt-dlp failed, at least one song could not be written, the song
     list or state could not be read or written, or check found a problem
  5  A query for a change did not read or matched no song, a change
     needs a terminal, -y or --all, a file added matched no song for sure
     and was left out, a change or undo was refused, or the song list
     sets commands or servers not trusted, with no terminal to ask on
  130  Stopped by Ctrl-C or a signal to stop, with every program
       muman started
```

**Subcommands:**

* `add` — Add songs: fetch what URLs name, or copy files into the manual folder, list each as a song or as a source of the song it is the same recording as, set any tags given on each, then sync. Lyrics, pictures and tag files are matched to the songs they belong to, and asked about when that is not sure
* `sync` — Bring the library in line with the song list: fetch any missing source, add files dropped into the manual folder, make every lookup due, write every song whose sources or tags changed, and delete what no song makes
* `list` — List the songs a query matches, one per line: its first key, then its artist, title and album, separated by tabs
* `remove` — Remove the songs a query matches: their library files go, and they are kept under `[[removed]]` so no playlist or dropped file lists them again
* `restore` — List again the removed songs a query matches, fetching any source purged
* `set` — Set tags, pins or the lyrics offset on the songs a query matches: each `NAME=VALUE` sets, `NAME!` clears, the other words query
* `edit` — Edit the songs a query matches in $VISUAL or $EDITOR, as their song-list entries, then apply what changed. Needs a terminal
* `undo` — Put the song list and the library back as they were before the last run that changed them; how many runs are kept is the song list's `[history] runs`
* `purge` — Delete fetched sources and lookup records no song uses: an upload a release took the place of, a source taken out of its song. A removed song's sources stay for `restore`, and a file of your own is never touched. A source listed again is fetched again
* `check` — Check the library against what muman recorded: files missing, empty or changed since written, left by an interrupted run, or not muman's; sources missing, unreadable or damaged
* `status` — Say what a sync would write, and for each song where each of its aspects comes from and why; the song list and the library are left as they are, and what it measures is kept for the next sync. A song a sync leaves as it is is counted, not shown, unless a query matches it
* `export` — Write the song list and the library, as the last run left them, into one zip, or with --folder into a new folder: songs.toml at its root, the songs under library/. The library's settings may be changed for the export alone: each song the change touches is made again from its sources, as a library under those settings would hold it. With --max-size, songs are encoded again at lower bitrates, the least audible loss first, until the export fits. The library and the song list are left as they are
* `duplicates` — List the songs listed apart that are one recording, by their audio fingerprints: a file of an album there twice, or a track and its copies on other albums. Groups an album holds twice come first; songs any group holds that the query matches name the groups shown. Changes nothing, unless `--resolve`
* `info` — Count what the library holds, whether it is in step with the song list, the lookups due, and what in it could be better: lossy or narrow audio, missing or soft covers, missing lyrics or tags. Reads only what earlier runs recorded; `--verbose` names every song counted
* `completions` — Write a script that completes muman's commands, flags and values in a shell, for the shell to load

**Options:**

* `--home <DIR>` — The home folder: the song list, the state file and the sources [default: the platform's local data folder, then `muman`]
* `--library <DIR>` — Folder the songs are written to [default: the song list's `[library] path`, else the platform's music folder, then `muman`]
* `--offline` — Reach no network: make no lookup, fetch nothing missing, and refuse URLs; what is on disk is written as ever
* `--trust` — Trust the commands and servers the song list sets, as it sets them now, without asking: its hooks, its programs, yt-dlp's and aria2c's arguments, the providers' servers and keys, and the sites' fetch addresses. A list setting any not trusted yet is asked about on a terminal and refused without one; -y does not trust it
* `-v`, `--verbose` — Say each yt-dlp, aria2c, ffmpeg and ffprobe command as it runs, logins, passwords, headers and tokens hidden, and how close each new source came to every song it was compared with
* `--progress <MODE>` — How a long step says how far it has got, on stderr: `auto`, a status line on a terminal and plain lines elsewhere; `plain` lines; `json` events, one a line, each message one too; or `none`

  Default value: `auto`

  Possible values:
  - `auto`:
    A status line on a terminal, plain lines elsewhere
  - `plain`:
    Plain lines, terminal or not
  - `json`:
    JSON events, one a line
  - `none`:
    Nothing




### `muman add`

Add songs: fetch what URLs name, or copy files into the manual folder, list each as a song or as a source of the song it is the same recording as, set any tags given on each, then sync. Lyrics, pictures and tag files are matched to the songs they belong to, and asked about when that is not sure

**Usage:** `muman add [OPTIONS] <URL|MAGNET|FILE>...`

**Arguments:**

* `<URL|MAGNET|FILE>` — Video, playlist, album or channel URLs, as yt-dlp reads them; magnet links and .torrent files, whose song files aria2c fetches; or audio files and folders of them, copied into the manual folder, an original yt-dlp fetched among them into sources/yt-dlp under its key; or lyrics (.lrc, .txt, .srt, .vtt), pictures, tag files (NAME=value .txt, .ffmeta, .json) and cue sheets, each given to its songs

**Options:**

* `--to <QUERY>` — Give every lyrics, picture and tag file to the songs this query names instead of matching them by what they hold: one song, or for a picture every one, or for a cue sheet those its tracks go among. The query is split into terms as a shell splits words, so quote it whole: --to "artist:venn lantern" is two terms, --to 'album:="The Glass Orchards"' one
* `--no-match` — Keep each video as uploaded, never looking for its YouTube Music track; other lookups are still made
* `--from <FOLDER>` — Take up each torrent given from this folder, where another client downloaded it: its files are copied in and checked piece by piece, and only what is missing or wrong is fetched
* `-y`, `--yes` — Decide without asking: a new source joins the listed song it may be the same recording as, and a lyrics, picture or tag file goes to the song it is clearly nearest
* `--new` — Make every new source a song of its own, without comparing it
* `--title <TEXT>` — The song's title
* `--artist <NAME>` — Given again, one more artist
* `--album <TEXT>` — The album's title
* `--album-artist <NAME>` — The album's artist, where the library's folders are named from
* `--genre <TEXT>` — Given again, one more genre
* `--date <DATE>` — The release date, as YYYY-MM-DD or a year
* `--track <N>` — The track number on its disc
* `--disc <N>` — The disc number
* `--tag <NAME=VALUE>` — Any Vorbis comment; given again with one name, one more value



### `muman sync`

Bring the library in line with the song list: fetch any missing source, add files dropped into the manual folder, make every lookup due, write every song whose sources or tags changed, and delete what no song makes

**Usage:** `muman sync [OPTIONS]`

**Options:**

* `--rematch` — Make every lookup a song still wants now, whatever an earlier one found: a song that has what a provider's `until` names, or whose record from it gives all it can, asks it nothing
* `--force` — Write every song again, changed or not, a library file changed since muman wrote it included
* `--retry` — Read again sources that could not be read before, and fetch again at once those that failed to
* `--accept` — Take what each fetched source holds now, its audio, cover, lyrics or tags, where it differs from what its song was built from, as after a fetch again, and record it in the song list
* `--update-defaults` — Move each setting still at the default of the edition the song list names to this muman's default, and raise the edition; settings you changed stay
* `-y`, `--yes` — Decide without asking: a new source joins the listed song it may be the same recording as, and a lyrics, picture or tag file goes to the song it is clearly nearest
* `--new` — Make every new source a song of its own, without comparing it



### `muman list`

List the songs a query matches, one per line: its first key, then its artist, title and album, separated by tabs

**Usage:** `muman list [OPTIONS] [QUERY]...`

```text
Query:
  lumo fenn             Every word in the title, artist, album or album
                        artist, or a source's ID or file name whole
  artist:fenn           A field containing the text
  'artist:=Lumo Fenn'   A field equal to the text, quoted with spaces
  title::^one           A field matching a regular expression
  ^lyrics:yes           Not matching the term
  youtube.com:<id>      The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.
```

**Arguments:**

* `<QUERY>` — Terms every song listed matches; none lists every song

**Options:**

* `-f`, `--format <TEMPLATE>` — Write each song by a template of `{field}`s instead
* `--keys` — Write each song's first key alone
* `--removed` — List removed songs instead: each key and the song it was



### `muman remove`

Remove the songs a query matches: their library files go, and they are kept under `[[removed]]` so no playlist or dropped file lists them again

**Usage:** `muman remove [OPTIONS] <QUERY>...`

```text
Query:
  lumo fenn             Every word in the title, artist, album or album
                        artist, or a source's ID or file name whole
  artist:fenn           A field containing the text
  'artist:=Lumo Fenn'   A field equal to the text, quoted with spaces
  title::^one           A field matching a regular expression
  ^lyrics:yes           Not matching the term
  youtube.com:<id>      The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.
```

**Arguments:**

* `<QUERY>` — Terms every song removed matches

**Options:**

* `--purge` — Also delete their sources: fetched files outright, files of your own to the trash
* `-y`, `--yes` — Make the change without asking; needed without a terminal
* `--all` — Act on every song the query matches, without picking among them
* `-n`, `--dry-run` — Say what would change; change nothing



### `muman restore`

List again the removed songs a query matches, fetching any source purged

**Usage:** `muman restore [OPTIONS] <QUERY>...`

```text
Query:
  lumo fenn             Every word in the title, artist, album or album
                        artist, or a source's ID or file name whole
  artist:fenn           A field containing the text
  'artist:=Lumo Fenn'   A field equal to the text, quoted with spaces
  title::^one           A field matching a regular expression
  ^lyrics:yes           Not matching the term
  youtube.com:<id>      The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.
```

**Arguments:**

* `<QUERY>` — Terms every removed song listed again matches; its tags and its sources' are read as when it was listed

**Options:**

* `-y`, `--yes` — Make the change without asking; needed without a terminal
* `--all` — Act on every song the query matches, without picking among them
* `-n`, `--dry-run` — Say what would change; change nothing



### `muman set`

Set tags, pins or the lyrics offset on the songs a query matches: each `NAME=VALUE` sets, `NAME!` clears, the other words query

**Usage:** `muman set [OPTIONS] <QUERY|NAME=VALUE|NAME!>...`

```text
Assignments:
  genre=House        Set a tag; given again, one more value
  genre!             Clear the tag set, so the sources' value shows
  audio=<key>        Pin the audio, cover or lyrics to one of its sources
  lyrics=false       Take no lyrics
  audio!             Unpin
  lyrics_offset=80ms Move the lyrics later; a negative time, earlier
Every other word is a query, as `muman list` reads it. A query alone, on
a terminal, sets them step by step: each field with where its value comes
from and what the sources offer.
```

**Arguments:**

* `<QUERY|NAME=VALUE|NAME!>` — Assignments, and terms every song changed matches

**Options:**

* `-y`, `--yes` — Make the change without asking; needed without a terminal
* `--all` — Act on every song the query matches, without picking among them
* `-n`, `--dry-run` — Say what would change; change nothing



### `muman edit`

Edit the songs a query matches in $VISUAL or $EDITOR, as their song-list entries, then apply what changed. Needs a terminal

**Usage:** `muman edit [OPTIONS] [QUERY]...`

```text
Query:
  lumo fenn             Every word in the title, artist, album or album
                        artist, or a source's ID or file name whole
  artist:fenn           A field containing the text
  'artist:=Lumo Fenn'   A field equal to the text, quoted with spaces
  title::^one           A field matching a regular expression
  ^lyrics:yes           Not matching the term
  youtube.com:<id>      The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.
```

**Arguments:**

* `<QUERY>` — Terms every song edited matches; none offers every song

**Options:**

* `--all` — Act on every song the query matches, without picking among them



### `muman undo`

Put the song list and the library back as they were before the last run that changed them; how many runs are kept is the song list's `[history] runs`

**Usage:** `muman undo [OPTIONS]`

**Options:**

* `-y`, `--yes` — Put back without asking; needed without a terminal
* `-n`, `--dry-run` — Say what would be put back; change nothing



### `muman purge`

Delete fetched sources and lookup records no song uses: an upload a release took the place of, a source taken out of its song. A removed song's sources stay for `restore`, and a file of your own is never touched. A source listed again is fetched again

**Usage:** `muman purge [OPTIONS]`

**Options:**

* `-y`, `--yes` — Delete without asking; needed without a terminal
* `-n`, `--dry-run` — Say what would be deleted; change nothing



### `muman check`

Check the library against what muman recorded: files missing, empty or changed since written, left by an interrupted run, or not muman's; sources missing, unreadable or damaged

**Usage:** `muman check [OPTIONS]`

**Options:**

* `--decode` — Decode every source in full again, to find audio cut off or missing in a file changed since it was measured
* `--upstream` — Also ask each site whether it still serves each fetched source as its song records, downloading nothing: a source re-encoded, no longer served in its format, or taken down



### `muman status`

Say what a sync would write, and for each song where each of its aspects comes from and why; the song list and the library are left as they are, and what it measures is kept for the next sync. A song a sync leaves as it is is counted, not shown, unless a query matches it

**Usage:** `muman status [OPTIONS] [QUERY]...`

```text
Query:
  lumo fenn             Every word in the title, artist, album or album
                        artist, or a source's ID or file name whole
  artist:fenn           A field containing the text
  'artist:=Lumo Fenn'   A field equal to the text, quoted with spaces
  title::^one           A field matching a regular expression
  ^lyrics:yes           Not matching the term
  youtube.com:<id>      The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.
```

**Arguments:**

* `<QUERY>` — Terms every song shown matches; none shows every song changed

**Options:**

* `--all` — Show every song, those up to date too



### `muman export`

Write the song list and the library, as the last run left them, into one zip, or with --folder into a new folder: songs.toml at its root, the songs under library/. The library's settings may be changed for the export alone: each song the change touches is made again from its sources, as a library under those settings would hold it. With --max-size, songs are encoded again at lower bitrates, the least audible loss first, until the export fits. The library and the song list are left as they are

**Usage:** `muman export [OPTIONS]`

```text
Examples:
  --codec mp3 --set audio.mp3_bitrate=192kb/s
                     Every song in MP3 at 192 kb/s
  --template '{{ artist }} - {{ title }}' --set library.restrict=ascii
                     One folder of songs, named in ASCII
  --set loudness.mode=audio
                     Each song's gain applied to its samples
  --set 'cover.out=[{ to = "file", scope = "folder", name = "cover" }]'
                     Each album's cover as a file in its folder, not in its songs
A song changed since muman wrote it, or whose sources are gone, is
written as the library holds it, and said.
```

**Options:**

* `-o`, `--output <PATH>` — The zip to write, or a folder to write muman.zip in; with --folder, the folder to make, new or empty [default: muman.zip, or muman with --folder]
* `--folder` — Write a folder holding what the zip would, rather than the zip
* `--max-size <SIZE>` — The most the export may take, such as 4GiB or 700MB; in a folder, each file counts whole blocks of `[library] block_size`
* `--codec <CODEC>` — Write every song in this lossy codec, lossless songs too: sets `[audio] codecs`, `lossy` and `lossless` to it

  Possible values: `opus`, `vorbis`, `aac`, `mp3`

* `--template <TEMPLATE>` — Each song's path without its extension, as `[library] template`
* `--lyrics <WHERE>` — Where lyrics go: a `.lrc` beside each song (sidecar), inside it (embedded) or both, as `[[lyrics.out]]` says

  Possible values: `sidecar`, `embedded`, `both`

* `--redact` — Write the song list with each secret it holds replaced by `<redacted>`: AcoustID's key, the passwords, user names and headers in yt-dlp's and aria2c's arguments, tokens in yt-dlp's extractor arguments, and the credentials in any address
* `--set <KEY=VALUE>` — A setting of `[library]`, `[audio]` or `[loudness]`, `lyrics.out`, `cover.out` or a cleaning rule, as `tags.clean.tidy.spacing`, for the export alone, such as `audio.mp3_bitrate=192kb/s`; repeatable, and set after the flags above. VALUE is TOML, or else text



### `muman duplicates`

List the songs listed apart that are one recording, by their audio fingerprints: a file of an album there twice, or a track and its copies on other albums. Groups an album holds twice come first; songs any group holds that the query matches name the groups shown. Changes nothing, unless `--resolve`

**Usage:** `muman duplicates [OPTIONS] [QUERY]...`

```text
Query:
  lumo fenn             Every word in the title, artist, album or album
                        artist, or a source's ID or file name whole
  artist:fenn           A field containing the text
  'artist:=Lumo Fenn'   A field equal to the text, quoted with spaces
  title::^one           A field matching a regular expression
  ^lyrics:yes           Not matching the term
  youtube.com:<id>      The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.
```

**Arguments:**

* `<QUERY>` — Terms a song in each group shown matches; none shows every group

**Options:**

* `--resolve` — Step through the groups on a terminal, each shown side by side, to merge its songs into one, keep them apart or remove one, then make what was chosen as one change `undo` puts back
* `--merge <GROUPS>` — Merge each group of the kind named into its song with the best audio, without stepping through them; a group whose songs set a tag by hand to two values is left for `--resolve`

  Possible values:
  - `twice`:
    Each group an album holds twice, the copies on that album

* `-y`, `--yes` — Make what was chosen without the last question
* `-n`, `--dry-run` — Say what would change; change nothing



### `muman info`

Count what the library holds, whether it is in step with the song list, the lookups due, and what in it could be better: lossy or narrow audio, missing or soft covers, missing lyrics or tags. Reads only what earlier runs recorded; `--verbose` names every song counted

**Usage:** `muman info`



### `muman completions`

Write a script that completes muman's commands, flags and values in a shell, for the shell to load

**Usage:** `muman completions <SHELL>`

```text
Loading the script:
  bash        In ~/.bashrc: `eval "$(muman completions bash)"`
  zsh         In ~/.zshrc, after compinit: `source <(muman completions zsh)`
  fish        `muman completions fish > ~/.config/fish/completions/muman.fish`
  powershell  In $PROFILE: `muman completions powershell | Out-String |
              Invoke-Expression`
  elvish      In rc.elv: `eval (muman completions elvish | slurp)`
Write a script saved to a file again after updating muman.
```

**Arguments:**

* `<SHELL>` — The shell the script is for

  Possible values: `bash`, `elvish`, `fish`, `powershell`, `zsh`
