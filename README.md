# music_display

A terminal now-playing display: cover art, metadata, clickable controls, synced lyrics, guitar chords and cava in one zero-flicker TUI.

## Build

```sh
sudo pacman -S --needed rust playerctl cava   # cava is optional (only for --cava / `c`)
cargo build --release
install -Dm755 target/release/music_display ~/.local/bin/music_display
```

No OpenSSL or libchafa needed (rustls + pure-Rust image decoding).

## Run

```sh
music_display                          # Metadata | Lyrics
music_display --nolyrics               # metadata only (toggle lyrics with `l`)
music_display --cava --thirdpanel      # Metadata | Lyrics | Cava
music_display --chords --chords-ug     # chords above the lyrics (local sheets, then Ultimate Guitar)
music_display --minimal                # teleprompter: title + lyrics + thin progress bar
music_display --terminal-colors        # use the terminal's own theme
music_display --print-config           # show the effective configuration and exit
```

Press `?` inside the app for the full list of keys and mouse actions (it is generated from the same table the key handler uses).

## Configuration

`~/.config/music_display/config.toml` (or `$XDG_CONFIG_HOME/music_display/config.toml`, or `--config FILE`). Nothing is created for you. **Command-line flags override the file, the file overrides the built-in defaults.** A missing file is fine; unknown keys and invalid values print a warning with the line number to stderr and fall back to the default.

```toml
player = "spotify"             # MPRIS player(s) for playerctl: a name, a comma list in priority
                               # order ("vlc,mpv"), "local" (common desktop players) or "any"
offset = 0.0                   # default lyric offset, seconds
tint = 1.0                     # cover duotone strength 0..1
transparent = false
terminal_colors = false
ascii_cover = false
cover_theme = false
cava = false
cava_bars = 64
thirdpanel = false
big_lyrics = false
lyric_spacing = 1              # 0..3
lyric_pad = 0                  # 0..10 blank columns on each side of the lyrics
big_scale = 1                  # 1..3, size of the big-font line
chords = false
chords_ug = false
# chords_dir = "~/.local/share/music_display/chords"
chords_context = 99            # 99 = chords on every line; N = active line + N upcoming
volume_step = 5                # percent per + / - / wheel notch
interlude_threshold_secs = 8   # silence longer than this shows [ Instrumental / Solo ]
lyrics_retries = 3             # retries (exponential backoff) after a failed lyrics request
translate = false              # start with translated lyrics (toggle with K)
translate_lang = "en"          # language lyrics are translated into
translate_fallback = true      # use LibreTranslate when Google is rate-limited / unreachable
translate_fallback_url = "https://libretranslate.com/translate"
# translate_fallback_key = "…" # libretranslate.com needs a key; your own server may not
minimal_view = false           # start in the minimal view
hide_color_warning = false     # hide "(no usable colour file)" in the footer
mouse = true                   # false = don't capture the mouse
```

`music_display --help` lists every flag with its config key.

### Colours

| flag | effect |
| --- | --- |
| *(none)* | first usable colour file wins (see Notes). If none is usable — missing, invalid, or `{}` — the app falls back to the terminal's colours, never to built-in hex values. |
| `--terminal-colors` | skip every colour file; default background/foreground, ANSI cyan accent, grays for fading. No hex colours are used. |
| `--surface HEX` / `--primary HEX` | override just that slot; combine freely with `--terminal-colors`. |
| `--cover-theme` | accent follows the dominant vivid colour of the current cover; updates every track. |
| `--tint 0..1` | duotone strength on the cover itself. `0` = original artwork. |
| `--transparent` | don't paint the surface colour as the background. |

### Layout, lyrics, cover

| flag | effect |
| --- | --- |
| `--nolyrics` / `--cava` / `--thirdpanel` / `--underlyrics` | panel layout. In `--cava --thirdpanel` the cava panel is exactly as wide as the metadata panel; cava always runs in `mono`. |
| `--ascii-cover` | draw the cover as coloured ASCII characters (no graphics protocol needed). |
| `--lyric-spacing 0..3` | blank rows between lyric lines (default 1; dropped on very short panels). |
| `--big-lyrics` | render the active line in a 5×7 half-block font (also `b`). |
| `--minimal` | start in the minimal teleprompter view (also `M`). |
| `--interlude-secs N` | a silence between two lyric lines longer than N seconds shows `[ Instrumental / Solo ]` (with a countdown) while it lasts; also before the first and after the last line. |
| `--lyrics-retries N` | retries after a failed lyrics request (timeouts, 429 with `Retry-After`, disconnects, 5xx), 0.5 s / 1 s / 2 s backoff. |
| `--big-scale 1..3` / `--lyric-pad 0..10` | size of the big-font line (pixels drawn 1–3× as wide and tall; falls back to a smaller size when it doesn't fit) / blank columns beside the lyrics. |
| `--hide-color-warning` | hide the "(no usable colour file)" note in the footer. |
| `--translate-fallback BOOL` / `--translate-fallback-url URL` / `--translate-fallback-key KEY` | when Google answers 429 or can't be reached, translate through LibreTranslate (default `libretranslate.com`, which needs a key; point it at your own server otherwise). The rest of that song then stays on the fallback and the lyrics title names it. |
| `--translate` / `--translate-lang CODE` | start with lyrics translated (Google Translate, unofficial endpoint) into CODE (default `en`); toggle with `K`. Chords are hidden while a translation is shown. |
| `--volume-step N` | percent per volume change. |
| `--no-mouse` | don't capture the mouse. |

## Keys

| key | action |
| --- | --- |
| `space` | play / pause |
| `n` / `p` | next / previous |
| `←` / `→` | seek −5 s / +5 s |
| `+` / `-` | volume up / down (`volume_step`, default 5 %) |
| `m` | mute / restore |
| `,` / `.` | lyric offset −0.1 s / +0.1 s |
| `l` | show / hide the lyrics |
| `K` | translate lyrics on / off (cached per song while the app runs) |
| `b` | big-font active line |
| `↑` / `↓`, `PgUp` / `PgDn`, mouse wheel over the lyrics | browse the lyrics without leaving the song; `End` (or 6 s idle) returns to the playing line |
| `g` | chords on / off |
| `t` / `T` | transpose chords up / down one semitone |
| `]` / `[` | capo up / down (0–12), remembered per track |
| `0` | back to the sheet's own capo |
| `D` | chord diagram panel |
| `M` | minimal teleprompter view |
| `s` / `S` | settings menu (`Esc`, `q`, `s` or `S` close it): toggles and steppers for the player source (Spotify, VLC, mpv, Rhythmbox, local, any), chords, translation, minimal view, big lyrics, spacing, cava and more. Changes apply immediately and are saved to the config file (comments in it are not kept); items marked `*` need a restart |
| `c` | toggle cava (process is killed while hidden) |
| `?` | help overlay (↑/↓ scroll, any other key closes) |
| `q` / `Esc` | quit |

Mouse: click the progress bar to seek; click `[ prev ] [ pause ] [ next ]`; `[ - ] [ 65% ] [ + ]` change the volume, clicking the percentage mutes (`[ MUTE ]`) and restores, the wheel over the control changes it too; **click a lyric line to seek to it** (synced lyrics only; works through wrapped lines and chord rows). Hold Shift to select text.

The volume control follows the player: changes made elsewhere (media keys, the Spotify app) show up within a poll. It is hidden when the player doesn't report a volume.

## Metadata panel

Rows: `Track No | Tuning`, `Release | Capo`, `BPM | Key` (without a capo the tuning takes the Capo slot; hidden items leave no gap). The playback time sits inside the progress bar: elapsed on the left, total on the right. `Release` shows the full `YYYY-MM-DD` when the player or MusicBrainz/iTunes has it (otherwise the year). `BPM`, `Key: E Minor / 9A` (key + Camelot code) and `Capo` come from the **chord sheet** when it carries them (`{key: …}`, `{tempo: …}`, `Key:` / `BPM:` lines, Ultimate Guitar's page metadata). MPRIS carries no key or BPM, so they come from the sheet: with `chords_ug` on the sheet is fetched even while chords are hidden, and a sheet that states no key gets one estimated from its chords. Without a sheet these fields are simply hidden. The `Key` shown is the song's own key and never changes with `t` / `T`; only the chord letters do. *TODO: a real audio-analysis source (the Spotify audio-features endpoint is closed to new apps).*

`Tuning` (shown while chords are on) is the song's original guitar tuning (Standard, Drop D, Half Step Down…) from Ultimate Guitar's page metadata or a `{tuning: …}` line in a ChordPro file; it is also shown in the chord diagram panel title.

The sheet's key is taken to be the key of its chord *shapes*. Key and Camelot show the **sounding** key (shapes + capo); the Capo row says which shapes you finger, e.g. `Capo 3  (shapes in C#m)`. Changing the capo with `[` / `]` re-spells the shapes (and the diagrams) but leaves the sounding key alone.

## Local files

Any MPRIS player works (`player = "vlc"`, `"mpv"`, `"local"`…). When it reports a `file://` URL but leaves tags out (artist, album, year, track number, cover), they are read from the file itself (ID3, FLAC/Vorbis, MP4) with `lofty`; a file without tags shows its file name.

## Chords

See [CHORDS.md](CHORDS.md). In short: your own sheets in `~/.local/share/music_display/chords/` (`Artist - Title.cho` / `.txt`), optionally Ultimate Guitar with `--chords-ug`; chords are shown on every lyric line (the active one in bold), intro chords appear before the first lyric, and `D` opens chord diagrams with a small tab. During an instrumental or solo section the chords of that part of the sheet are shown under `[ Instrumental / Solo ]` so you can play along. If a lookup fails, the title is retried without `(…)` groups, `feat.`/`ft.` credits and ` - demo` / ` - remastered` suffixes.

## Notes

- Colour files are checked in order: `~/.config/noctalia/colors.json`, `~/.cache/noctalia/colors.json`, `~/.config/matugen/colors.json`, `~/.cache/matugen/colors.json`, `~/.config/noctalia/theme.json`.
- Cover art uses Kitty graphics when available, then Sixel / iTerm2, then unicode half-blocks. The bitmap is scaled to exactly fill its cell box.
- If cava can't start or dies, the cava panel shows why (cava's own error message); press `c` twice to retry.
- Two editions of the same track ("Metallica" vs "Metallica (Remastered Deluxe Box Set)", different casing) are the same track: no flicker, no refetch.
- Lyrics: "lyrics unavailable (network error / rate limited / server error)" only appears after every attempt of every fallback (exact match → artist + cleaned title → search) has failed; a real "no lyrics" answer is reported as such, and cached for a few days.
- Caches: `~/.cache/music_display/` (years, lyrics, chords, per-track capo; errors are never cached), cover downloads in `/tmp/music_display/`.
