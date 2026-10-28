# Roadmap

Only free or offline-friendly ideas. Effort: S (hours), M (a day or two), L (a week+). "Builds on" names real code.

## Essential

- **Local `.lrc` files** next to the audio or in a lyrics dir, preferred over lrclib. S. Builds on `lyrics::parse_lrc`, `lyrics::to_lyrics`, `tags::path_of`, `chords::find_local` (same lookup pattern).
- **Saved per-track lyric offset**, remembered like the per-track capo. S. Builds on `App.offset` and the offset nudge in `src/app/mod.rs`, cache dir in `util::cache_dir`, capo persistence via `AppEvent::Chords { capo }`.
- **`--waybar` JSON output** (text, tooltip, class) printed on change, no TUI. S. Builds on `player::Snapshot`, `Snapshot::key`, the poller in `player.rs`, `cli::Args`.
- **Scrobbling to ListenBrainz** (free token; Last.fm optional). M. Builds on `player::Snapshot` (play/length), `util::normalize_track`, `reqwest::Client` in `App`, offline queue in `util::cache_dir`.
- **Word-level timing from enhanced LRC** (`<mm:ss.xx>`). M. Builds on `lyrics::parse_lrc` (currently drops word stamps), `ui/karaoke.rs::line_progress` (currently estimates by text length).
- **Export lyrics + chords as text** (ChordPro or plain, current transpose/capo). S. Builds on `chords::to_chordpro`, `chords::align`, `chords::transpose`.
- **Theme presets** (built-in named palettes, selectable by flag/settings row). S. Builds on `theme.rs`, `color.rs`, `app/settings.rs` rows, `config::Key` table.
- **Robust shutdown/terminal restore and error notes audit**: surface every failed fetch in `set_note`. S. Builds on `app/remote.rs::set_note`, `shutdown.rs`.

## Fun

- **Practice mode**: loop a section (A/B on lyric lines), metronome tick from BPM, slow-down where the player supports rate. M. Builds on `Ctl::seek_to`, `chords::Aligned` line times, `AppEvent::Player` position anchor in `App`.
- **Local web overlay for OBS** (tiny HTTP server serving current line + cover as a browser source). M. Builds on `tokio`, `Snapshot`, `cover::Loaded`, active-line logic in `app/mod.rs`.
- **Discord rich presence** (local IPC socket, no account tokens beyond the app id). M. Builds on `Snapshot`, `cover` art URL (`art_url`).
- **Export lyrics + chords as an image** (PNG card with cover and accent). M. Builds on `cover::dominant_accent`, `image`, `diagram::lookup`, `bigfont.rs` glyphs.
- **Auto-scroll chord sheet with a "play-along" cue** that highlights the next chord change. S. Builds on `chords::Aligned`, `ui/diagrams.rs`, interlude chords in `ui/lyrics.rs`.
- **Chord key detection badge + suggested capo** for easier shapes. S. Builds on `chords::estimate_key`, `KeyInfo::camelot`, `chords::capo_shift`.
- **Lyrics search across cache** (find a song by a line you remember). M. Builds on the lyrics cache in `lyrics.rs` (`CacheEntry`), `chords::norm` / `dice` fuzzy match, `ui/library.rs` list UI.
- **Listening stats** (top tracks/artists from a local play log, shown as a panel). M. Builds on `Snapshot::key`, `App` history, `ui/library.rs`.
- **Sleep timer / pause-after-song**. S. Builds on `Ctl::send`, `app/actions.rs` Action enum, `ui/footer.rs` for the countdown.
- **Pronunciation / romanisation line for Hangul** next to the original. M. Builds on `lyrics::is_hangul`, `hangul_ratio`, translate display path (`AppEvent::Translated`), `genius` member tags.
- **Local cover and tag editor shortcut** (fix wrong year/artist in cache). S. Builds on `year.rs` cache, `tags::read`.
