# Stress runs

The tests in this repository run offline against fakes. What they
cannot find, muman's behaviour on real recordings, real sites and real
file managers, has been found by stress runs made by hand, and their
findings are the fixes in the changelog. This file records how each
run was made, as its pull request described it, so the next one repeats
it rather than starting over, and names what no run has covered yet.
Nothing here is scripted: a run is a session against a scratch home, and
its findings go into the pull request that fixes them and into this file.

## Runs made

| Before | Against | Found |
|---|---|---|
| 0.3.0 | A first stress run, later runs of killed runs and locks, and an audio run over every codec and layout | File names and moved files; encoders handed audio they cannot hold, Opus ending 312 samples late, measures mixing channels into stereo before measuring (#17) |
| 0.4.0 | Two rounds of black-box and source stress runs | From undo safety to excerpt choice (#18) |
| 0.4.0 | A real library of about 2,000 songs, 2,213 sources, on 0.3.0 | The damage rules: three sources damaged, where the old rule flagged 41; a moved library forgotten; fetched sources named by a title a site may change (#20) |
| 0.5.0 | Six audits, of audio, tags, lyrics, covers, names and exports against their standards; then 48 public-domain recordings and 27 sidecar files from archive.org, five areas, every scenario twice from scratch | Ten defects that fail a song or lose data silently, eight of them from six design choices since replaced (#25) |
| 0.6.1 | Each built-in `[sites]` table against yt-dlp 2026.08.19: its extractors' keys, IDs and pages, and whether each fetch address names the source again; then Jamendo, Audius, Niconico and Dailymotion end to end, fetched, deleted and fetched again | archive.org's fetch address losing a `+`, Niconico's comments and Bilibili's danmaku fetched as subtitles, Jamendo named by two domains, Audius playlists never skipped; archive.org files named with a space, `#`, `?`, `%` or a letter outside ASCII refused as sources, in about half the multi-file items among its most fetched community audio; the files of an item added whole missing the item's creator and date |

The run before 0.5.0 also checked determinism: two fresh homes, four `add`
orders, all seven encoders, different time zones, locales and CPU
counts, interrupted syncs and repeated exports gave byte-identical
libraries, but for the padding lofty leaves in an M4A whose tag shrank.

A run after 0.5.0 timed `add` instead of testing it: 38 recordings of the
archive.org item `MusopenCollectionAsFlac`, the 32 Goldberg Variations
and six orchestral and chamber movements, 2.9 hours and 959 MB of FLAC,
into a fresh home on four cores, three times. It took 38 to 43 s: 27 or
28 s measuring, 3 to 5 s writing, the rest copying the files in,
disk-bound. Of the measuring
step's processor time, ffmpeg's run took two thirds, its decode half of
that and the packet hash and the fingerprint's resampling most of the
rest; of muman's own, the true-peak scan took the most, then the
K-weighting filter, then the stereo and bandwidth measures, which cost
the same on a one-minute variation as on a seventeen-minute movement.
The changes it led to are in the changelog; the facts the home
recorded were the same to the byte before and after them. What stayed:
the packet hash and the resampling are ffmpeg's, and changing either
changes what every library holds, so neither was.

A second run after 0.5.0 timed a library of thousands: the 38 recordings
and 2,000 songs of 30 s made by a script, a two-voice melody drawn from
a seeded random sequence over faint pink noise so that no two measured
as one recording, 500 each as FLAC, Opus, MP3 and M4A, tagged as ten
tracks to an album by 60 invented artists with a cover embedded where
ffmpeg embeds one, 2.9 GB in all. `add` of the 2,038 into a fresh home
on four cores took 1,010 s: 165 s copying the files in, 10 min 5 s
measuring, 3 min 31 s writing. The copying read the library once per
file and listed a folder once per sidecar looked for; the measuring kept
each source's 15 MB of scratch until the run ended, 2.9 GB by the 299th
source, and ran the disk out until a sweeper removed them; the writing
wrote each FLAC whole twice, once encoding and once tagging. After the
changes in the changelog, the same `add` took 824 s: 7 s copying, 9 min
8 s measuring, 3 min 29 s writing, and the scratch held only the sources
being measured. The home ended at 13 MB of `state.json`, 528 KB of
`songs.toml`, 4.0 GB of sources, 11 MB of history and a 4.1 GB library.
What stayed: every command plans the whole library before it answers,
so `list` took 4 s and a `sync` with nothing to do 6 to 7 s, before and
after, and `add` of one more song 8 s, `remove` and `undo` of it 3.9 to
4.5 s.

## How a run is made

1. Build the branch under test and make a scratch home with
   `MUMAN_HOME`, never a home in use.
1. Choose public-domain recordings and sidecar files, as the archive.org
   items the run before 0.5.0 used; the fixtures are not in the repository, so
   list the items used in the pull request.
1. Take one area at a time, as the audits did: audio, tags, lyrics,
   covers, names, exports. For each, write the scenarios first, run
   every one twice from scratch, and compare the libraries byte for
   byte.
1. Reproduce each finding before its fix, and give each fix a test that
   fails without it, over the fakes; then run the finding's scenario
   against the real files once more.
1. Record the fixtures, the scenarios and the findings in the pull
   request, and add the run to the table above.

## Not covered yet

- archive.org sources make no lookups, which needs a provider type of
  their own.
- A PNG drawn only in its alpha channel measures as having no detail.
- A retagged M4A keeps the padding lofty adds when a tag shrinks, so its
  bytes depend on its edit history.
- An interrupted sync encodes every song again.
- Vimeo asked for a login, TikTok refused the address and Audiomack's
  extractor failed, so the run tested none of the three.
