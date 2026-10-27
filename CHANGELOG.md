# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Cut a release with `scripts/release.sh X.Y.Z`,
which turns the `Unreleased` section into that version.

## [Unreleased]

### Added
- `CHANGELOG.md` and `scripts/release.sh X.Y.Z` (bumps `Cargo.toml` / `Cargo.lock`, dates the
  `Unreleased` section, commits and tags `vX.Y.Z`).
- `--version` and the settings menu show the build as `0.1.0 (a1b2c3d, dirty)`: the version, the
  git commit and whether the working tree had uncommitted changes.

### Changed
- The core (players, lyrics, chords, translation, Spotify, config, cover decoding, cava) is now a
  library crate with no terminal code, so another front end can reuse it. `ui.rs` and `app.rs`
  are split into one file per panel / concern. No behaviour change.

## [0.1.0] - 2026-10-27

First tagged release.

### Added
- Terminal now-playing display for MPRIS players: metadata, cover art (Kitty / Sixel / iTerm2 /
  halfblocks or coloured ASCII), progress bar and clickable transport buttons.
- Synced lyrics from lrclib.net with live offset nudging, lyric browsing, several lyric versions
  per song, a karaoke fill for the active line and a big block-font mode (including Hangul).
- Release year fallback (MPRIS, MusicBrainz, iTunes), local audio tags, tuning, key, BPM and capo.
- Guitar chords from local ChordPro / plain-text sheets and Ultimate Guitar, aligned to the
  synced lyrics, transposable, with chord diagrams (`D`).
- Optional cava visualiser as a third column or under the lyrics.
- Lyric translation (Google, LibreTranslate fallback, disk cache) with a never-translate language
  list and a bilingual mode (`k`).
- Member parts for group songs, from Genius.
- Spotify Web API (`--spotify`): PKCE login, library browser, search, queue, likes, device
  picker and playback on other devices; `--setup` for spotifyd; `use_spotifyd`.
- `config.toml` (flag > file > default) with `--print-config`, a settings menu (`s`), a help
  overlay (`?`) with a key editor and a `[keys]` section for remapping, minimal view (`M`).
- `install.sh`, `install.ps1`, the `mdis` short command and a release workflow.
- Native Windows support through the system media session (SMTC).

### Changed
- `main.rs` split into one file per module.
- Track identity ignores editions, remasters and release changes, so the lyrics and chords stay
  put; language and instrumental releases count as their own track.
- Control row layout: whole buttons only, scaling tiers, grouped with dashes.
- Spotify requests: `Retry-After` is kept in full, polling is slower and the library preload is
  staggered.
- Minimum supported Rust version is 1.90.

### Fixed
- Flicker between remastered versions of the same track.
- "Cursor position could not be read" exit on layout change.
- Wide (CJK) characters misaligning chords and lyrics.
- Hangul lyrics picked for language and instrumental releases.

[Unreleased]: https://github.com/moizuwu/music_display/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/moizuwu/music_display/releases/tag/v0.1.0
