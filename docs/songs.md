# Songs

The commands that change songs take a query naming them, say what they
will do before they do it, and can be undone. This page
covers queries, the commands that change songs, giving songs lyrics,
pictures and tags from files, and merging songs that are one recording.
The song list itself is in [the guide](guide.md), and how a song is
made from its sources in [making the library](library.md).

## Queries

`list`, `remove`, `restore`, `set`, `edit`, `status` and `duplicates`
take a query. muman answers it from what earlier runs resolved, without
fetching or measuring anything. Every term must match, and case,
accents and how a character was composed are ignored, so `cafe`
matches "Café":

| Term | Matches |
|---|---|
| `paper comets` | Songs with each word in the title, artist, album or album artist, or as a source's whole ID or file name |
| `artist:venn` | A field containing the text |
| `artist:="Marlo Venn"` | A field equal to the text |
| `title::^paper` | A field matching a regular expression |
| `^lyrics:yes` | Songs the term does not match |
| `youtube.com:vid00000001` | The song listing that key |

A field is any tag, or one of these:

| Field | Holds |
|---|---|
| `key` | The song's source keys |
| `path` | Its path in the library |
| `format` | Its file's extension, as `opus`, `flac` or `m4a` |
| `lyrics` | `yes`, `none`, `timed` or `untimed` |
| `cover` | `yes`, `none`, `sharp` or `small`, by the detail `info` counts |
| `from` | A provider one of its sources came from, as `from:youtube-music`; a file of your own or a torrent's is `manual` |
| `has` | A tag a source or the song list gives, as `has:date` |

`lyrics`, `cover`, `from` and `has` compare their value whole, so
`lyrics:timed` never matches "untimed". An album artist muman takes
from the artist, or an album named after a single's title, is no tag
for `has`. muman refuses a field that is no tag it knows and that no
song has, so a misspelled field never quietly matches nothing, or every
song when negated.

## Changing songs

- **`list`** writes each song's first key, artist, title and album,
  separated by tabs. `-f` takes a template of `{field}`s instead,
  `--keys` writes the first key alone, and `--removed` lists the songs
  `remove` took out.
- **`set`** sets tags, pins and the lyrics offset. `genre=Folk` sets a
  tag, and given twice sets two values; `genre!` clears it so the
  sources' value shows. `audio=<key>` pins the audio to a source and
  `audio!` unpins it; `lyrics=false` takes no lyrics. `set` refuses a
  track or disc that is no number, a date that is none, and a ReplayGain
  gain or peak.
- **`set` with a query alone**, on a terminal, steps through the songs
  field by field. Each field shows its value and where it came from;
  opened, it shows what each source offers, for you to keep, take, type
  over, choose among or clear. You choose a pin among the song's
  sources, each shown with what was measured of it, and type the lyrics
  offset as a time; "Another tag…" names any other field. For songs all
  on one album, you can set an album field, such as the genre or date,
  on the album itself, for its songs and any added later, which clears
  each song's own. Nothing is saved until "Review and save", which says
  each change and asks.
- **`edit`** opens the songs' entries in `$VISUAL`, else `$EDITOR`,
  else `vi`, or Notepad on Windows. Delete a table to remove its song.
  Move a key into another table to join two songs, or into a new table
  without an `id` to split it off. A file with problems opens again
  with each as an `# ERROR:` line. Comments above each song show each
  tag it writes, where that came from and what other sources offer;
  they never reach the song list.
- **`remove`** moves songs to `[[removed]]` and deletes their library
  files. `--purge` also deletes their fetched sources, and moves your
  own files to the trash.
- **`restore`** lists removed songs again, fetching any source purged.
  It refuses until you take back from the trash the files of yours that
  `--purge` moved there, and names each.
- **`undo`** puts the song list and the library back as before the last
  run that changed them, one run further back each time.

```bash
muman list | fzf -m -d '\t' --with-nth 2.. --accept-nth 1 | xargs muman remove -y
muman set artist:="Marlo Venn" album="Lantern Weather"
```

### What undo puts back

`undo` brings back the files a run wrote over or removed, and moves
back the files it moved. A file you had changed comes back still yours.
A file `add` copied into the sources goes: deleted while it holds what
was copied, moved to the trash once changed. A hand edit to the song
list is undone with the run that applied it.

A run stopped partway, by a crash or Ctrl-C, can be undone, and so can
an undo stopped partway. An undo ends with a sync, and until that sync
is done the undo stays the latest run: the next `undo` finishes it and
goes no further back, and `check` names it.

`undo` refuses when the song list changed since the run. After `remove
--purge`, it refuses until you take back from the trash the files it
names. `[history]` sets how many runs are kept and how much space their
files may take together. When space runs short the oldest runs' files
go first, and undoing a run whose files are gone writes them again
from their sources.

### Picking and confirming

When a query matches several songs, you pick among them, listed by what
tells them apart: album, track, length, audio, cover, lyrics and key,
each shown only where the songs differ. None is picked to begin with.

Every change then says what it does, song by song: `−` for a song that
leaves the list, `~` for one that changes and `+` for one that comes
back, each tag as `old → new`. It counts the songs, says what `undo`
can put back, and asks by name, as "Remove 2 songs?":

| Situation | On a terminal | Without one |
|---|---|---|
| The query matches nothing | Refused, exit code 5 | Refused, exit code 5 |
| It matches several songs | Picked from a list | Refused, exit code 5, unless `--all` |
| The change itself | Asked, unless `-y` | Made only with `-y`, else exit code 5 |
| `remove --purge` moving files of yours to the trash | The count of songs typed, unless `-y` | Made only with `-y`, else exit code 5 |

`-n` says what would change and stops. A query of keys alone takes
every song it names. `edit` needs a terminal for its editor and takes
no `-y`. A song changed meanwhile, by hand or by another run, refuses
the save of `remove`, `restore` or `set NAME=VALUE`. `edit` opens again
and says so: save again to apply your edit over the change, or quit
without saving to keep it. `set` stepping through fields applies your
choices to the song as it is now.

## Giving songs lyrics, pictures and tags

`muman add` also takes files that are no song, and gives each to the
songs it belongs to:

| File | Is |
|---|---|
| `.lrc`, a `.txt` of plain lyrics, `.srt`, `.vtt` | Lyrics, copied in as `.lrc` and listed among the song's sources |
| JPEG, PNG or WebP | A cover, listed by every song whose cover looks like it |
| `NAME=value` lines (`.txt`, `.tags`, `.vc`), `;FFMETADATA1` (`.ffmeta`), `.json` from beets or MusicBrainz | Tags, set on the song by hand; the file is not kept |
| `.cue` | Tags of the album and of each track, each track's set on its song |

A lyrics file or picture is ranked like any other source, so a song
may keep its timed lyrics over new untimed ones; pin the new ones with
`muman set` to choose them. A tag file's gains, peaks, embedded
pictures, lyrics and encoder names describe another file and are
skipped. A beets field sets the Vorbis comment Picard writes it as,
`label` as `LABEL`. One Picard has no name for is skipped, and beets'
own fields, as its path and bitrate, set nothing. `--verbose` names
each tag skipped.

### How a file finds its song

muman matches each file by the strongest evidence it shows:

1. An ID it shares with the song: a video ID in its name, an ISRC or a
   MusicBrainz recording ID.
1. Its name, as the song's file is named.
1. Its fields: an `.lrc`'s `[ti:]`, `[ar:]`, `[al:]` and `[length:]`,
   a tag file's, or its own name read as `03 - Artist - Title`,
   `Artist - Title` or `Title`.
1. What it holds: lyrics against the words the song's lyrics sing, and
   a picture against the song's covers by how they look, whatever its
   size, compression or bars.

Names are compared within their own script. Accents, case, width and
Traditional or Simplified forms count for little, but kanji and romaji,
or Hangul and Latin, cannot be compared by their letters and decide
nothing.

| Evidence | Verdict |
|---|---|
| An ID, the song file's name, the same words or a cover alike | The song's |
| Title, artist, album, track and length all but equal, and nearer than any other song | The song's |
| A title alone, two songs as near, another version (live, instrumental, remix…), or a near match | Asked |
| Lyrics ending after the song does, or stating another length | Not the song's |

On a terminal, asked means you pick among the likeliest songs, each
shown with why:

```text
Lantern Weather — Marlo Venn · The Glass Orchards · 3:20   close name, length agrees
```

`same name`, `close name` or `other name` says how the names compare,
and the rest what else agrees or not. You may also search the song
list with a query, or leave the file out. `-y` takes the likeliest song
when it is clearly the nearest.

Without a terminal, a file not surely a song's is left out, and `add`
exits with code 5 once the rest is done. A file that cannot be used is
not added, and `add` exits with code 4 after writing the songs given
with it: a `.pdf`, an empty `.lrc`, a picture that does not decode
whole, a song file with no audio, or a cue sheet that splits one song's
file into tracks.

`--to` names the songs with a query instead of matching; quote the
query as one word. For a picture it names every song to give it to,
for a cue sheet the songs its tracks go among:

```bash
muman add "Lantern Weather.lrc"
muman add front.jpg --to 'album:="The Glass Orchards"'
muman add words.txt --to "artist:venn lantern"
muman add album.cue rips/*.flac --new
```

A picture that looks like the covers of more than three songs on more
than one album is asked about rather than given to all. A cover given
with songs from its folder goes to those songs. Several songs may list
one picture of yours, but each song must still list a file of its own:
a song whose every source another song lists too is refused. Lyrics
belong to one song, so the same lyrics given to a second song are
copied for it. A picture or `.lrc` moved within `sources/manual` is
followed, as a song file is.

## Duplicates

`muman duplicates` lists songs listed apart that are one recording, by
their audio fingerprints. First come files an album holds twice, then
tracks with copies on other albums, which a library of whole albums
keeps on purpose. An album is told by its album artist and name. A
query picks the groups shown. Listing changes nothing.

`duplicates --resolve` steps through the groups on a terminal. Each
group is shown side by side, a column for each song: key, album and
track, length, audio, cover, lyrics, hand-set tags and how alike each
sounds to the first. The first is the song a merge keeps, the one
whose source the merged song's audio would come from. For copies an
album holds twice, merging is offered first. Copies across albums come
after, once you ask, with keeping them apart offered first.

| Answer | Does |
|---|---|
| Merge | Moves the others' sources, with what each held when built, into the song kept. Its album, place, pins and lyrics offset stay; the others' are let go, and named |
| Keep apart, Decide later | Nothing; the group is asked about again next time |
| Remove one | Takes the song you pick out of the list, as `remove` does |
| Stop | Asks no more, and goes on to what was picked |

A tag the songs set by hand to two values is asked about, to choose
which stays. What you picked is then said and asked about, `-y` making
it without asking and `-n` stopping there. It is made as one change,
so one `undo` puts it all back. Without a terminal, `--resolve` is
refused.

`duplicates --merge twice` merges the copies of each recording an album
holds twice, without stepping through them, into the song a merge
would keep. A group whose songs set a tag by hand to two values is left
for `--resolve`, and named. It asks as other changes do: without a
terminal, `-y` makes it and `-n` says what it would do.
