# Changelog

All notable changes to muman. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as
[AGENTS.md](AGENTS.md#versioning) applies it.

## [Unreleased]

### Added

- **A Nix flake.** `nix run github:jeggao/muman` runs muman with ffmpeg
  and yt-dlp on Linux and Apple silicon Macs; the flake also has an
  overlay and a development shell, and `nix-build` works without
  flakes.

### Changed

- **Audio measured against real recordings.** Bandwidth is where a
  lowpass cuts the sound off, so a dark or quiet lossless recording no
  longer ranks below its own lossy copies; mono copied into channels at
  different levels counts as mono; audio clipped and then turned down
  counts as clipped. Sources are measured again on the next `sync`.

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

[Unreleased]: https://github.com/jeggao/muman/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jeggao/muman/releases/tag/v0.1.0
