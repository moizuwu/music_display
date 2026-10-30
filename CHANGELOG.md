# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Cut a release with `scripts/release.sh X.Y.Z`,
which turns the `Unreleased` section into that version.

## [Unreleased]

### Added
- `colors` (`off`, `file`, `gtk`, `auto`) and `colors_file` in `[gui]`: the window can take its base colour, text and accent from a colour file (matugen / noctalia, the files the terminal UI reads) or the GTK theme, and follows them when they change; the terminal UI re-reads its colour file when it changes.
- `effects` and `effects_intensity` in `[gui]`: snow, autumn leaves, dandelion seeds, wind, pollen, embers, fog, bats and eggs as a layer of their own over any background and look (`auto` = the look's own); the backgrounds are now `solid`, `cover-blur`, `gradient`, `image`, `sky`, `meadow` and `lawn`, and files with the old moving backgrounds keep working.
- `lyrics_fallback` (on by default): when lrclib has no lyrics, or fails more often than `lyrics_retries` allows, NetEase (synced), lyrics.ovh and the plain lyrics of the Genius page are tried; a find from them is looked up again after a day. Config key, flag, settings row.
- A dark / light toggle for the window (`v`, a top-bar button) that works with every look; Halloween has a light palette now, picking a look keeps the theme.
- The settings, keys, library, devices and AI-question windows are windows of their own with the window's background and opacity; the sizes of panels inside a column are remembered.
- The translated line and the member tag of the active line light up with it, and the glow covers all three.
- GUI looks Christmas, Easter and Midsummer next to Halloween, with the backgrounds `snow`, `eggs` and `meadow`; the light looks also work in the dark theme.
- Placement and size of the translated line and of the member tag: `translation_position` (`below`, `above`), `member_position` (`auto`, `below`, `above`, `inline`) in the terminal UI and the window, and `translation_size` / `member_size` in `[gui]`.
- `auto_zero_capo`: a sheet that says "capo 2" is shown with capo 0 (the sounding chords); your own capo for a track still wins. Config key, `--auto-zero-capo`, settings row and GUI checkbox.
- `CHANGELOG.md` and `scripts/release.sh X.Y.Z` (bumps `Cargo.toml` / `Cargo.lock`, dates the
  `Unreleased` section, commits and tags `vX.Y.Z`).
- `--version` and the settings menu show the build as `0.1.0 (a1b2c3d, dirty)`: the version, the
  git commit and whether the working tree had uncommitted changes.

- `translate_mode` (`off`, `both`, `translated`): flag `--translate-mode`, config key and a "Translation shown as" row in the settings menu.
- Optional AI lyric translation (`translate_engine = "ai"`, `[ai]` table, `--ai-*` flags, settings rows): the
  whole song in one request to Ollama or any OpenAI-compatible endpoint, Gemini, Claude or OpenAI with your own key;
  validated, retried once, cached, with a fallback to the standard engine and a first-use privacy notice.
- Annotations panel (experimental, off by default; `N`): Genius annotations matched to the active
  lyric line, or short AI-generated notes (labelled); `--annotations`, `--annotations-mode`,
  `genius_token` / `GENIUS_TOKEN`, settings rows. Misses cached 7 days, errors never, soft failure.
- `--gui` (build with `--features gui`): an egui/eframe window with metadata, cover and controls,
  synced lyrics with karaoke fill, chords and drawn diagrams, translation (including the AI engine),
  annotations, a settings window and the layout editor, on the same config file and layout as the
  terminal UI. `docs/gui-design.md` explains the choice; the README has a feature parity table.
- `docs/ROADMAP.md`: ideas that are free or work offline.
- `Genre` metadata field (MPRIS, file tags, then MusicBrainz) with `--genre-lookup` / `genre_lookup` and a settings row.
- Layout customization: a `[layout]` table in `config.toml` (panel order, visibility, size in
  percent, split direction, metadata field order and visibility, control buttons), the
  **Customize layout** page in the settings menu, `--layout-preset NAME` and `--reset-layout`. The
  old flags (`--nolyrics`, `--cava`, `--thirdpanel`, `--underlyrics`) are now presets of it.
- Metadata fields are an ordered list (`title`, `artist`, `album`, `track_no`, `release`,
  `genre`, `tuning`, `capo`, `bpm`, `key`), each shown or hidden in any order.
- `[ settings ]` and `[ ? ]` buttons in the top row; clicking Key, BPM, Capo or Tuning opens the chord
  diagrams; right-click a metadata field for Hide / Move up / Move down; drag a field to reorder
  and a column border to resize.
- Core modules for the next features, not wired up yet: `genre` (tag normalisation, MusicBrainz
  ranking), `ai` (whole-song translation through any OpenAI-compatible endpoint, Gemini, Claude or
  OpenAI) and `layout`.

### Fixed
- Cava placement: the "Third panel" row in the settings menu now writes `[layout] cava_under_lyrics`
  (it only wrote `thirdpanel`, which the layout then overrode, so the choice was lost on restart), the
  layout alone decides where cava goes, and "below the lyrics" is the default again. The customize
  page shows `bottom` / `side` on the Cava row and switches it with `p`.
- The cover is above the track information again; a saved layout with the old default panel order is
  migrated.
- The annotations panel no longer takes a third of the screen: it is narrower by default, and when the
  chord diagrams are on as well they share one column with it (diagrams above, notes below).
- Toggling lyrics, cava or the diagrams with their keys now keeps the layout's panel list in step.

### Changed
- The top row is always there (it holds the new buttons and, with Spotify, `[ library ]`).
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
