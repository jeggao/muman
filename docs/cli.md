# Command-line reference

<!-- Generated from the clap definitions in src/cli.rs by `cargo xtask docs`; edit those, not this file. -->

## `muman`

Keep a music library made from YouTube and files of your own, each song from the best of its sources.

songs.toml in the state root lists every song and the sources it may be made from: what yt-dlp fetched, kept whole in sources/yt-dlp, and files dropped into sources/manual. Each source is measured — its audio's real bandwidth, stereo and clipping, how much of it is not the song, its pictures' content and real detail, its lyrics, its tags — and for each song the best audio, cover, lyrics and tags are picked from among them by those measures alone. The song is written to the library where the song list's [library] template puts it, <album artist>/<album>/<track title> by default, its audio copied or encoded, tagged and covered, with its lyrics. A song is written again only when what it is made from changes, and a file muman wrote that no listed song makes any more is deleted.

A new source the same recording as a listed song, by its fingerprint, is added to that song; one that may be is asked about.

**Usage:** `muman [OPTIONS] <COMMAND>`

```text
Exit codes:
  0  Every song was written, or nothing needed to be; or a change was
     declined
  2  yt-dlp, ffmpeg or ffprobe is missing, or the system names no
     home folder
  4  yt-dlp failed, at least one song could not be written, the song
     list or state could not be read or written, or check found a problem
  5  A query matched no song, a change needs a terminal, -y or --all, or
     undo refused
```

**Subcommands:**

* `add` — Add songs: fetch what URLs name, or copy files into the manual folder, list each as a song or as a source of the song it is the same recording as, set any tags given on each, then sync
* `sync` — Bring the library in line with the song list: fetch any missing source, add files dropped into the manual folder, make every lookup due, write every song whose sources or tags changed, and delete what no song makes
* `list` — List the songs a query matches, one per line: its first key, then its artist, title and album, separated by tabs
* `remove` — Remove the songs a query matches: their library files go, and they are kept under `[[removed]]` so no playlist or dropped file lists them again
* `restore` — List again the removed songs a query matches, fetching any source purged
* `set` — Set tags, pins or the lyrics offset on the songs a query matches: each `NAME=VALUE` sets, `NAME!` clears, the other words query
* `edit` — Edit the songs a query matches in $VISUAL or $EDITOR, as their song-list entries, then apply what changed
* `undo` — Put the song list and the library back as they were before the last run that changed them; how many runs are kept is the song list's `[history] runs`
* `purge` — Delete fetched sources and lookup records no song uses: an upload a release took the place of, a source taken out of its song. A removed song's sources stay for `restore`, and a file of your own is never touched. A source listed again is fetched again
* `check` — Check the library against what muman recorded: files missing, empty or changed since written, left by an interrupted run, or not muman's; sources missing or unreadable
* `status` — Say what a sync would write, and for each song where each of its aspects comes from and why; change nothing
* `export` — Write the song list and the library, as the last run left them, into one zip: songs.toml at its root, the songs under library/. With --max-size, songs are encoded again at lower bitrates, the least audible loss first, until the zip fits; the library itself is left as it is
* `duplicates` — List the songs listed apart that are one recording, by their audio fingerprints: a file of an album there twice, or a track and its copies on other albums. Groups on one album come first; songs any group holds that the query matches name the groups shown. Changes nothing
* `info` — Count what the library holds, whether it is in step with the song list, the lookups due, and what in it could be better: lossy or narrow audio, missing or soft covers, missing lyrics or tags. Reads only what earlier runs recorded; `--verbose` names every song counted

**Options:**

* `--home <DIR>` — The state root: the song list, the state file and the sources [default: the platform's local data folder, then `muman`]
* `--library <DIR>` — Folder the songs are written to [default: the song list's `[library] path`, else the platform's music folder, then `muman`]
* `-v`, `--verbose` — Say each yt-dlp, ffmpeg and ffprobe command as it runs, and how close each new source came to every song it was compared with
* `--progress <MODE>` — How a long step says how far it has got, on stderr: `auto`, a status line on a terminal and plain lines elsewhere; `plain` lines; `json` events, one a line; or `none`

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

Add songs: fetch what URLs name, or copy files into the manual folder, list each as a song or as a source of the song it is the same recording as, set any tags given on each, then sync

**Usage:** `muman add [OPTIONS] <URL|FILE>...`

**Arguments:**

* `<URL|FILE>` — Video, playlist, album or channel URLs, as yt-dlp reads them; or audio files and folders of them, copied into the manual folder, or originals yt-dlp fetched, into the store

**Options:**

* `--no-match` — Keep each video as uploaded, never looking for its YouTube Music track; other lookups are still made
* `-y`, `--yes` — Add a new source to the listed song it may be the same recording as without asking
* `--new` — Make every new source a song of its own, without comparing it
* `--title <TEXT>`
* `--artist <NAME>` — Given again, one more artist
* `--album <TEXT>`
* `--album-artist <NAME>`
* `--genre <TEXT>` — Given again, one more genre
* `--date <DATE>` — The release date, as YYYY-MM-DD or a year
* `--track <N>`
* `--disc <N>`
* `--tag <NAME=VALUE>` — Any Vorbis comment; given again with one name, one more value



### `muman sync`

Bring the library in line with the song list: fetch any missing source, add files dropped into the manual folder, make every lookup due, write every song whose sources or tags changed, and delete what no song makes

**Usage:** `muman sync [OPTIONS]`

**Options:**

* `--rematch` — Make every lookup now, whatever an earlier one found: an upload on YouTube Music for its track, a track on YouTube for an upload with subtitles, a song on LRCLIB for its lyrics, a song on MusicBrainz for its album
* `--force` — Write every song again, changed or not, a library file changed since muman wrote it included
* `--retry` — Read again sources that could not be read before, and fetch again at once those that failed to
* `--update-defaults` — Move each setting still at the default of the edition the song list names to this muman's default, and raise the edition; settings you changed stay
* `-y`, `--yes` — Add a new source to the listed song it may be the same recording as without asking
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
  youtube:<id>          The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.
```

**Arguments:**

* `<QUERY>`

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
  youtube:<id>          The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.
```

**Arguments:**

* `<QUERY>`

**Options:**

* `--purge` — Also delete their sources: fetched files outright, files of your own to the trash
* `-y`, `--yes` — Make the change without asking; required without a terminal
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
  youtube:<id>          The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.
```

**Arguments:**

* `<QUERY>`

**Options:**

* `-y`, `--yes` — Make the change without asking; required without a terminal
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
Every other word is a query, as `muman list` reads it.
```

**Arguments:**

* `<QUERY|NAME=VALUE|NAME!>`

**Options:**

* `-y`, `--yes` — Make the change without asking; required without a terminal
* `--all` — Act on every song the query matches, without picking among them
* `-n`, `--dry-run` — Say what would change; change nothing



### `muman edit`

Edit the songs a query matches in $VISUAL or $EDITOR, as their song-list entries, then apply what changed

**Usage:** `muman edit [OPTIONS] [QUERY]...`

```text
Query:
  lumo fenn             Every word in the title, artist, album or album
                        artist, or a source's ID or file name whole
  artist:fenn           A field containing the text
  'artist:=Lumo Fenn'   A field equal to the text, quoted with spaces
  title::^one           A field matching a regular expression
  ^lyrics:yes           Not matching the term
  youtube:<id>          The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.
```

**Arguments:**

* `<QUERY>`

**Options:**

* `--all` — Act on every song the query matches, without picking among them



### `muman undo`

Put the song list and the library back as they were before the last run that changed them; how many runs are kept is the song list's `[history] runs`

**Usage:** `muman undo [OPTIONS]`

**Options:**

* `-y`, `--yes`
* `-n`, `--dry-run` — Say what would be put back; change nothing



### `muman purge`

Delete fetched sources and lookup records no song uses: an upload a release took the place of, a source taken out of its song. A removed song's sources stay for `restore`, and a file of your own is never touched. A source listed again is fetched again

**Usage:** `muman purge [OPTIONS]`

**Options:**

* `-y`, `--yes`
* `-n`, `--dry-run` — Say what would be deleted; change nothing



### `muman check`

Check the library against what muman recorded: files missing, empty or changed since written, left by an interrupted run, or not muman's; sources missing or unreadable

**Usage:** `muman check [OPTIONS]`

**Options:**

* `--decode` — Also decode every source in full, to find a truncated download



### `muman status`

Say what a sync would write, and for each song where each of its aspects comes from and why; change nothing

**Usage:** `muman status`



### `muman export`

Write the song list and the library, as the last run left them, into one zip: songs.toml at its root, the songs under library/. With --max-size, songs are encoded again at lower bitrates, the least audible loss first, until the zip fits; the library itself is left as it is

**Usage:** `muman export [OPTIONS]`

**Options:**

* `-o`, `--output <PATH>` — The zip to write, or a folder to write muman.zip in

  Default value: `muman.zip`
* `--max-size <SIZE>` — The most the zip may take, such as 4GiB or 700MB



### `muman duplicates`

List the songs listed apart that are one recording, by their audio fingerprints: a file of an album there twice, or a track and its copies on other albums. Groups on one album come first; songs any group holds that the query matches name the groups shown. Changes nothing

**Usage:** `muman duplicates [QUERY]...`

```text
Query:
  lumo fenn             Every word in the title, artist, album or album
                        artist, or a source's ID or file name whole
  artist:fenn           A field containing the text
  'artist:=Lumo Fenn'   A field equal to the text, quoted with spaces
  title::^one           A field matching a regular expression
  ^lyrics:yes           Not matching the term
  youtube:<id>          The song listing that source
Fields are any tag, and key, path, format (the extension: opus, ogg,
flac, mp3, m4a), cover and lyrics (yes, none). Case is ignored.
```

**Arguments:**

* `<QUERY>`



### `muman info`

Count what the library holds, whether it is in step with the song list, the lookups due, and what in it could be better: lossy or narrow audio, missing or soft covers, missing lyrics or tags. Reads only what earlier runs recorded; `--verbose` names every song counted

**Usage:** `muman info`
