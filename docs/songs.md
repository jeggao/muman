# Choosing and changing songs

How songs are named from the command line and changed there: the
query `list`, `remove`, `restore`, `set`, `edit` and `duplicates` take,
what each change does and how it is confirmed, and songs listed apart
that are one recording. The song list itself is in [the
guide](guide.md); how each song is made from its sources, in [making
the library](library.md).

## Queries

`list`, `remove`, `restore`, `set` and `edit` take a query, read from
what earlier runs resolved, without fetching or measuring. Its terms
are all required, case and accents ignored, so `cafe` matches "Café":

| Term | Matches |
|---|---|
| `paper comets` | Each word in the title, artist, album or album artist, or a source's whole ID or file name |
| `artist:venn` | A field containing the text |
| `artist:="Marlo Venn"` | A field equal to the text |
| `title::^paper` | A field matching a regular expression |
| `^lyrics:yes` | A song the term does not match |
| `youtube.com:vid00000001` | The song listing that key |

A field is any tag, or `key`, `path`, `format` (the file's extension:
`opus`, `ogg`, `flac`, `mp3`, `m4a`),
`cover` and `lyrics` (`yes`, `none`). A field that is no tag muman knows
and that no song has is refused, so a misspelled one never matches
nothing, or, negated, every song.

## Changing songs

- **`list`** writes each song's key, artist, title and album, separated
  by tabs; `-f` takes a template of `{field}`s, `--removed` lists the
  tombstones.
- **`set`** sets tags, pins and `lyrics_offset`: `genre=Folk` sets,
  `genre!` clears so the sources' value shows, `audio=<key>` pins,
  `audio!` unpins, `lyrics=false` takes none. A track or disc number
  that is no number, a date that is none, and the gains muman measures
  are refused, as by `add --tag`.
- **`edit`** opens the songs' entries in `$VISUAL` or `$EDITOR`. Delete
  a table to remove its song, move a key to another table to join the
  two, or into a new table without an `id` to split it off. A file with
  problems reopens with each as an `# ERROR:` line.
- **`remove`** moves songs to `[[removed]]` and deletes their library
  files; `--purge` also deletes fetched sources and trashes your own.
- **`restore`** lists removed songs again, fetching any purged source.
  It refuses until you put back from the trash the files of your own
  `--purge` moved there, naming each.
- **`undo`** puts the song list and library back as before the last run
  that changed them, one run further back each time: files it wrote over
  or removed come back, a file you had changed still yours, files it
  moved move back, a file `add` copied into `sources/` goes, deleted
  while it holds what was copied and moved to the trash once changed,
  and an edit you made by hand before that run is undone with it. A run stopped partway, by a
  crash or Ctrl-C, can be undone too, and so can an undo stopped partway:
  until the sync it ends with is done, it stays the latest run, which
  the next `undo` finishes, going no further back, and `check` names.
  It refuses when the song list changed since, and after `remove
  --purge` until you put the files of your own it trashed back from the
  trash, where it names them. `[history]` sets how many runs are kept
  and how much space they take together; each run that records holds
  the history to it, letting go of the oldest runs' files first, and a
  file not kept is written again from its sources.

```bash
muman list | fzf -m -d '\t' --with-nth 2.. --accept-nth 1 | xargs muman remove -y
muman set artist:="Marlo Venn" album="Lantern Weather"
```

Songs to pick among are listed by what tells them apart: album, track,
length, audio, cover, lyrics and key, each shown only where the songs
differ, and a name they all share said in the question. None is picked
to begin with. Every change then says what it does first, each song
marked `−` when it leaves the list, `~` when it changes and `+` when it
comes back, each tag as `old → new`, with a count and what `undo` puts
back, and asks by name, as "Remove 2 songs?":

| Situation | On a terminal | Without one |
|---|---|---|
| The query matches nothing | Refused, exit 5 | Refused, exit 5 |
| It matches several songs | Picked from a list | Refused unless `--all`, exit 5 |
| The change itself | Asked, unless `-y` | Made only with `-y`, else exit 5 |
| `remove --purge` trashing files of your own | The count of songs typed, unless `-y` | Made only with `-y`, else exit 5 |

`edit` takes no `-y`: it needs a terminal for its editor, `$VISUAL`,
else `$EDITOR`, else `vi`, or Notepad on Windows. A query of keys alone
takes every song it names. `-n` says what would change and stops. A song changed meanwhile, by hand or by another run,
refuses the save.

## Duplicates

`duplicates` lists songs listed apart that are one recording, by
their audio fingerprints: a file of an album there twice first, then a
track and its copies on other albums, which a library of whole albums
keeps on purpose. An album is told by its album artist with its name. A
query names the groups shown. Listing changes nothing.

`duplicates --resolve` steps through the groups on a terminal. Each is
shown side by side, one column a song: key, album and track, length,
audio, cover, lyrics, tags set by hand and how alike each is to the
first, the song a merge keeps, whose source the merged song's audio
would come from. Merging the copies an album holds twice is offered
first; copies across albums come after, once asked, with keeping them
apart offered first:

| Answer | Does |
|---|---|
| Merge | The others' sources, and what each held when built, move into the song kept; its album, place, pins and lyrics offset stay, and the others' are let go, named in what it says it does |
| Keep apart, Decide later | Nothing; the group is asked about again next time |
| Remove one | Takes the song picked out of the list, as `remove` |
| Stop | Asks no more, and goes on to what was picked |

A tag set by hand to two values is asked about, which stays. What was
picked is then said and asked about, `-y` making it without asking and
`-n` stopping there, and made as one change: `undo` puts it all back.
Without a terminal it is refused.
