# music_display

A terminal now-playing display: cover art, metadata, clickable controls, synced lyrics, guitar chords and cava in one zero-flicker TUI.

- Works with any MPRIS player (Spotify, spotifyd, VLC, mpv, …) through `playerctl`.
- Synced lyrics from lrclib.net, optional translation, big-font active line.
- Guitar chords from your own sheets or Ultimate Guitar, with transpose, capo and diagrams.
- Optional Spotify integration (`--spotify`): browse your playlists, Liked Songs, albums and artists, play, queue, like and add to playlists, and see and control what plays on your phone. Needs a one-time login, see [Spotify](#spotify).

### Quick start with Spotify

```sh
# 1. at https://developer.spotify.com/dashboard: create an app, add the redirect URI
#    http://127.0.0.1:8897/callback and copy its Client ID
music_display --spotify-client-id <Client ID> --spotify-login   # 2. log in once in the browser
music_display --spotify                                         # 3. run; `o` opens the library
```

Put `spotify = true` in the config file to make it the default.

## Install

You need `playerctl` (it talks to your player), a C compiler and linker, and **Rust 1.90 or newer**; `cava` is optional (only for `--cava` / `c`). No OpenSSL or libchafa is needed (rustls + pure-Rust image decoding).

**With cargo** (the binary goes to `~/.cargo/bin/music_display`):

```sh
cargo install --git https://github.com/moizuwu/music_display
```

**With the install script** (Arch, Debian / Ubuntu, Fedora and openSUSE families; builds from source and puts `music_display` and the short command `mdis` in `~/.local/bin`):

```sh
curl -fsSL https://raw.githubusercontent.com/moizuwu/music_display/main/install.sh | bash
```

or, to read it before running it:

```sh
curl -fsSLo install.sh https://raw.githubusercontent.com/moizuwu/music_display/main/install.sh
less install.sh
bash install.sh --dry-run     # shows every step, changes nothing
bash install.sh
```

What it does: installs only what is missing (asking first, with `sudo` or `doas`), offers rustup in your home directory when Rust is missing or older than 1.90, clones the source to `~/.local/share/music_display/src` and builds it with `cargo build --release --locked`. Re-running it updates the checkout with a fast-forward and rebuilds only if something changed; if the checkout has local changes or can't fast-forward it stops and leaves everything alone. It never overwrites a file or someone else's symlink named `music_display` or `mdis`.

| option | effect |
| --- | --- |
| `--dry-run` | print the steps, change nothing |
| `--yes` | answer yes to the questions (including the optional `cava`) |
| `--no-deps` | install no packages and no Rust, only check they are there |
| `--ref REF` | install this branch or tag instead of `main` |
| `--force` | rebuild even when already up to date |
| `--aur` | Arch: allow building a dependency from the AUR when it isn't in the official repos (needs `yay` or `paru`; `--yes` does not allow this). Normally never needed. |

Other systems: install the packages above yourself, then use `cargo install` or build from source.

## Build from source

```sh
sudo pacman -S --needed rust playerctl cava   # cava is optional (only for --cava / `c`)
cargo build --release
install -Dm755 target/release/music_display ~/.local/bin/music_display
```

## Run

```sh
music_display                          # Metadata | Lyrics
music_display --nolyrics               # metadata only (toggle lyrics with `l`)
music_display --cava --thirdpanel      # Metadata | Lyrics | Cava
music_display --chords --chords-ug     # chords above the lyrics (local sheets, then Ultimate Guitar)
music_display --minimal                # teleprompter: title + lyrics + thin progress bar
music_display --terminal-colors        # use the terminal's own theme
music_display --print-config           # show the effective configuration and exit
music_display --spotify                # + Spotify library (o), like (f), play here (H), phone playback
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
member_parts = false           # experimental: show who sings each line (Genius, unofficial)
# chords_dir = "~/.local/share/music_display/chords"
chords_context = 99            # 99 = chords on every line; N = active line + N upcoming
volume_step = 5                # percent per + / - / wheel notch
interlude_threshold_secs = 8   # silence longer than this shows [ Instrumental / Solo ]
lyrics_retries = 3             # retries (exponential backoff) after a failed lyrics request
translate = false              # start with translated lyrics (toggle with K)
translate_lang = "en"          # language lyrics are translated into
translate_skip = ""              # languages never translated, e.g. "sv,fi"
translate_fallback = true      # use LibreTranslate when Google is rate-limited / unreachable
translate_fallback_url = "https://libretranslate.com/translate"
# translate_fallback_key = "…" # libretranslate.com needs a key; your own server may not
minimal_view = false           # start in the minimal view
hide_color_warning = false     # hide "(no usable colour file)" in the footer
mouse = true                   # false = don't capture the mouse
spotify = false                # Spotify Web API features (see "Spotify" below)
# spotify_client_id = "…"      # Client ID of your Spotify developer app (for --spotify-login)
spotify_device = "Desktop (spotifyd)"  # spotifyd's device_name, so it counts as "this PC"
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
| `--big-lyrics` | render the active line in a 5×7 half-block font (also `b`). Latin (5×7) and Hangul are drawn big; Hangul syllables are composed from jamo strokes on a finer 12×12 grid (12 columns × 6 rows each) and, when a line doesn't fit that way (too wide or too tall for the panel), on the older 8×8 grid; a compact 3×5 font (no Hangul) is used when the panel is too narrow for the normal one, and other scripts fall back to bold text. Chords of the active line sit right above the glyph of the character they belong to (a chord on a space goes over the next letter; each wrapped piece gets its own chord row in the blank row above it, and chords are pushed apart rather than overlap); the neighbouring lines and the intro / break chords are drawn as in the normal layout. See "Hangul in big mode" below. |
| `--minimal` | start in the minimal teleprompter view (also `M`). |
| `--interlude-secs N` | a silence between two lyric lines longer than N seconds shows `[ Instrumental / Solo ]` (with a countdown) while it lasts; also before the first and after the last line. |
| `--lyrics-retries N` | retries after a failed lyrics request (timeouts, 429 with `Retry-After`, disconnects, 5xx), 0.5 s / 1 s / 2 s backoff. |
| `--big-scale 1..3` / `--lyric-pad 0..10` | size of the big-font line (pixels drawn 1–3× as wide and tall; falls back to a smaller size when it doesn't fit) / blank columns beside the lyrics. |
| `--hide-color-warning` | hide the "(no usable colour file)" note in the footer. |
| `--translate-skip CODES` | languages that are never translated, comma separated (`sv,fi`): the song's language is asked once, from a sample of its lines, before anything is translated; if it is on the list the lyrics stay as written (the title says `never translated (sv)`) and no line is sent, and a single line detected as one of them stays as written inside a song that is translated. Legacy codes (`iw`, `jw`, `zh-CN`) are normalised. If no service can name the language (rate-limited), the translation is held, the title says so, and the check is retried every 20 s. Also in the settings menu ("Never translate": ←/→ pick a language, space adds or removes it). |
| `--translate-fallback BOOL` / `--translate-fallback-url URL` / `--translate-fallback-key KEY` | when Google answers 429 or can't be reached, translate through LibreTranslate (default `libretranslate.com`, which needs a key; point it at your own server otherwise). The rest of that song then stays on the fallback and the lyrics title names it. |
| `--translate` / `--translate-lang CODE` | start with lyrics translated (Google Translate, unofficial endpoint) into CODE (default `en`); toggle with `K`. Chords stay on while a translation is shown, each moved to the same relative place in the translated line. |
| `--member-parts` | experimental, off by default: tag lyric lines with the member singing them (`[Jimin]`), read from Genius' section headers (`[Verse 1: Jimin]`, `[Chorus: Jungkook, V]`). Also in the settings menu as "Member parts (experimental)". See "Korean lyrics and member parts" below. |
| `--volume-step N` | percent per volume change. |

`x`, `i` and `u` change `big_scale`, `lyric_spacing` and `lyric_pad` while the app runs and show the new value in the footer. They are plain lowercase letters, so they work on every keyboard layout (no Shift or AltGr; `+` / `-` are volume and `[` / `]` are capo, which need AltGr on a Swedish layout). They are not saved: the settings menu (`s`) and the config file keep the saved defaults.
| `--no-mouse` | don't capture the mouse. |

### Spotify

| flag | effect |
| --- | --- |
| `--spotify` | turn on the Spotify Web API features: library browser (`o`), like (`f`), play here (`H`), and showing / controlling playback on another device. |
| `--spotify-client-id ID` | Client ID of your Spotify developer app; only the login needs it (it is saved with the token). |
| `--spotify-device NAME` | Spotify Connect name of spotifyd on this PC (its `device_name`, default `Desktop (spotifyd)`). |
| `--spotify-login` / `--spotify-logout` | log in in the browser and save the token / forget it, then exit. |

## Keys

| key | action |
| --- | --- |
| `space` | play / pause |
| `n` / `p` | next / previous |
| `z` | shuffle on / off (only for players that report it) |
| `r` | repeat: off → playlist → track → off (`rep 1` = track) |
| `←` / `→` | seek −5 s / +5 s |
| `+` / `-` | volume up / down (`volume_step`, default 5 %) |
| `m` | mute / restore |
| `,` / `.` | lyric offset −0.1 s / +0.1 s |
| `l` | show / hide the lyrics |
| `K` | translate lyrics on / off (cached per song while the app runs) |
| `b` | big-font active line |
| `x` | big-font size 1 → 2 → 3 → 1 (`big_scale`; the line steps down by itself when it doesn't fit) |
| `i` | line spacing 0 → 1 → 2 → 3 → 0 (`lyric_spacing`) |
| `u` | side padding 0 → 2 → 4 … → 10 → 0 (`lyric_pad`) |
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
| `o` | Spotify library browser (`--spotify`, see below) |
| `f` | like / unlike the song playing (`♥` / `♡` beside the title) |
| `H` | play here: move Spotify playback from the phone / another device to this PC |
| `q` / `Esc` | quit |

Mouse: click the progress bar to seek; click `[ shuf ]   [ rep ]   [ prev ]   [ pause ]   [ next ]   [ - ] [ 65% ] [ + ]` (three blank columns between buttons; shuffle and repeat are lit while on and hidden when the player reports neither); the `[ - ] [ 65% ] [ + ]` part changes the volume, clicking the percentage mutes (`[ MUTE ]`) and restores, the wheel over the control changes it too; **click a lyric line to seek to it** (synced lyrics only; works through wrapped lines and chord rows). Hold Shift to select text.

The control row scales with the window and only ever draws whole buttons. When it gets narrow it gives up, in this order: the volume moves to its own row below the buttons; labels turn compact (`[sh] [rp] [<<] [|>] [>>]`, volume `[-] 65% [+]`); shuffle and repeat are dropped; the volume shrinks to `[-][+]` and then goes; last prev / next go, so play stays longest. Keyboard shortcuts work whatever is hidden. When the window is short, the controls and progress bar keep their rows first, then the metadata, and the cover shrinks or disappears before any of them. The key hints at the bottom of the library browser follow the same rule: whole hints are left out, never cut.

Shuffle and repeat are read with `playerctl shuffle` / `playerctl loop` about every 2 s and shown as they are reported. Some players (Spotify desktop in particular) may ignore the write, in which case the button simply flips back on the next poll.

The volume control follows the player: changes made elsewhere (media keys, the Spotify app) show up within a poll. It is hidden when the player doesn't report a volume.

## Metadata panel

Track, Artist and Album take a full row each. Below them `Track No` and `Release` sit on the left, and `Tuning`, `Capo`, `BPM` and `Key` form a right-hand column, one per row, all starting at the same column:

```
Track No  01-03                Tuning: Drop D
Release   1991-08-12           Capo: 2 (Am)
                               BPM: 120
                               Key: E Minor / 9A
```

Items that no source provides are skipped and the rest move up; on a panel too narrow for two columns the right-hand items follow below, indented under the values. The playback time sits inside the progress bar: elapsed on the left, total on the right. `Release` shows the full `YYYY-MM-DD` when the player or MusicBrainz/iTunes has it (otherwise the year). `BPM`, `Key: E Minor / 9A` (key + Camelot code) and `Capo` come from the **chord sheet** when it carries them (`{key: …}`, `{tempo: …}`, `Key:` / `BPM:` lines, Ultimate Guitar's page metadata). MPRIS carries no key or BPM, so they come from the sheet: with `chords_ug` on the sheet is fetched even while chords are hidden, and a sheet that states no key gets one estimated from its chords. Without a sheet these fields are simply hidden. The `Key` shown is the song's own key and never changes with `t` / `T`; only the chord letters do. *TODO: a real audio-analysis source (the Spotify audio-features endpoint is closed to new apps).*

`Tuning` (shown while chords are on) is the song's original guitar tuning (Standard, Drop D, Half Step Down…) from Ultimate Guitar's page metadata or a `{tuning: …}` line in a ChordPro file; it is also shown in the chord diagram panel title.

The sheet's key is taken to be the key of its chord *shapes*. Key and Camelot show the **sounding** key (shapes + capo); the Capo row says which shapes you finger, e.g. `Capo 3  (shapes in C#m)`. Changing the capo with `[` / `]` re-spells the shapes (and the diagrams) but leaves the sounding key alone.

## Spotify

With `--spotify` (or `spotify = true`) the app talks to the Spotify Web API itself, no other program or helper needed.

**Setup, once:**

1. Create an app at <https://developer.spotify.com/dashboard> and add the redirect URI `http://127.0.0.1:8897/callback` exactly.
2. `music_display --spotify-client-id <Client ID> --spotify-login` opens the browser (if it doesn't, the link is printed). After you allow access the token is saved in `~/.local/state/music_display/spotify/token.json` (mode 600), together with the Client ID, so later runs only need `--spotify`. The token refreshes itself; `--spotify-logout` deletes it.

The login uses PKCE: no client secret is needed or stored. In development mode Spotify only lets the users you add under the app's *User Management* log in, so add your own account there if the login is refused. The login waits 3 minutes for the browser; if port 8897 is taken it says so. Until you are logged in, `o` / `f` / `H` show "not logged in: run music_display --spotify-login".

**Library browser (`o`).** Tabs: Recent · Playlists · Liked · Albums · Artists.

| key | action |
| --- | --- |
| `Tab` / `Shift-Tab`, `1`–`5` | switch tab |
| `↑` / `↓`, `j` / `k`, `PgUp` / `PgDn`, `Home` / `End`, wheel | move |
| `Enter` / `space` | play: a song plays in its list (album, playlist, Liked Songs) from that song on; an album / playlist / artist plays itself |
| `→` / `l` | open an album, your own playlist, an artist (albums and singles, newest first) or Liked Songs (`›` marks what opens) |
| `←` / `h` / `Backspace` | back |
| `a` | add the song to the queue |
| `A` | add the song to one of your playlists (a list of them opens; `Enter` adds) |
| `f` | like the song |
| `r` | reload the list |
| `Esc` / `q` / `o` | close |

Lists are loaded when first shown and kept while the app runs. Spotify only lets development-mode apps open your own (and collaborative) playlists; other people's play with `Enter`. Liked Songs shows the first 1000.

**Other devices.** When no MPRIS player is running (the phone plays, or Spotify runs elsewhere), the app shows what Spotify says plays, with lyrics, cover and chords as usual, and every control (play / pause, next / previous, seek, volume, shuffle, repeat) goes to that device through the API; the footer says `on <device>`. It polls every 1.5 s while that plays (3 s paused, 5 s while an MPRIS player is on screen). `H` moves playback to this PC (the Spotify app when open, else spotifyd, matched by `spotify_device`), and the MPRIS player takes over from there. Play with nothing active anywhere carries on here from where Spotify last stopped, or starts the last played song.

## Local files

Any MPRIS player works (`player = "vlc"`, `"mpv"`, `"local"`…). When it reports a `file://` URL but leaves tags out (artist, album, year, track number, cover), they are read from the file itself (ID3, FLAC/Vorbis, MP4) with `lofty`; a file without tags shows its file name.

## Chords

See [CHORDS.md](CHORDS.md). In short: your own sheets in `~/.local/share/music_display/chords/` (`Artist - Title.cho` / `.txt`), optionally Ultimate Guitar with `--chords-ug`; chords are shown on every lyric line (the active one in bold), intro chords appear before the first lyric, and `D` opens chord diagrams with a small tab. During an instrumental or solo section the chords of that part of the sheet are shown under `[ Instrumental / Solo ]` so you can play along. If a lookup fails, the title is retried without `(…)` groups, `feat.`/`ft.` credits and ` - demo` / ` - remastered` suffixes.

## Notes

- A song counts as the same track across its releases: first artist plus the bare title (no edition, live, remaster or guest suffixes; the album is ignored), except that a release marked as another language or instrumental (`English Version`, `Japanese Ver.`, `Instrumental` in the title or album) is its own track with its own lyrics; for those, Hangul lyrics are never picked. If the player's strings change while the same item keeps playing (same track id, or the position running on at the same length), nothing is refetched and the panel keeps what it showed. With several players running, the one on screen is kept until it goes quiet or another starts playing while it doesn't.
- `MUSIC_DISPLAY_DEBUG=1` appends every player snapshot to `~/.cache/music_display/snapshots.log` (or set it to a file path).
- Colour files are checked in order: `~/.config/noctalia/colors.json`, `~/.cache/noctalia/colors.json`, `~/.config/matugen/colors.json`, `~/.cache/matugen/colors.json`, `~/.config/noctalia/theme.json`.
- Cover art uses Kitty graphics when available, then Sixel / iTerm2, then unicode half-blocks. The bitmap is scaled to exactly fill its cell box.
- If cava can't start or dies, the cava panel shows why (cava's own error message); press `c` twice to retry.
- Two editions of the same track ("Metallica" vs "Metallica (Remastered Deluxe Box Set)", different casing) are the same track: no flicker, no refetch.
- Lyrics: "lyrics unavailable (network error / rate limited / server error)" only appears after every attempt of every fallback (exact match → artist + cleaned title → search) has failed; a real "no lyrics" answer is reported as such, and cached for a few days.
- Caches: `~/.cache/music_display/` (years, lyrics, chords, per-track capo; errors are never cached), cover downloads in `/tmp/music_display/`.

## Korean lyrics and member parts

**Hangul preferred.** When the title or artist contains Hangul, or a lyrics candidate is mostly (30 % or more) Hangul, lyrics in Hangul are ranked above romanized ones (synced still beats plain). A romanized hit from the exact lookup is not final in that case: the search step still runs and a Hangul candidate replaces it if one exists.

**Hangul in big mode.** A terminal can't pick a font, so the big Hangul is drawn from character-cell blocks (`▀ ▄ █`, which every terminal font has). A line with Hangul is tried on the fine 12×12 grid first at the chosen `big_scale`, then on the 8×8 grid at the same scale, then one scale smaller, and so on; the compact font can't draw Hangul, so a very narrow panel shows the line as bold text. The fine grid gives a final consonant four rows instead of three and tells more syllables apart (about 10 460 of the 11 172 blocks have a distinct shape, against about 10 050 on 8×8), but its syllables are wider (13 columns instead of 9), so fewer fit on one row and a long line wraps into more pieces. Quadrant and braille blocks were considered and left out: a quadrant cell doubles only the horizontal resolution (the cramped direction is vertical) and braille depends on the terminal font.

**Member parts (experimental).** With `member_parts = true` / `--member-parts` the song is looked up on Genius (unauthenticated search, then the song page; the title and artist must match closely). Section headers such as `[Verse 1: Jimin]`, `[Chorus: Jungkook, V]` and `[Pre-Chorus: RM & SUGA]` give the members for the lines below them, and those lines are matched in order onto the synced lyrics. The active line and its neighbours get a dim `[Jimin]` tag, each member in a stable colour of the terminal palette. In the normal scrolling layout the tag sits in front of the line; in the big-font layout it is one dim, centred row directly above the big active line (above the chord row when chords are shown, with a row kept free for it on every line of a song that has member data, so nothing jumps). On a narrow panel the tag shortens, `[Jimin, V, Jungkook]` → `[Jimin +2]` → `[Jim…]`, and is left out when there is no room. Lines that can't be matched (for example romanized vs. Hangul wording) are left untagged. Results are cached per track (misses for 7 days); failures are silent. This scrapes a website that has no public API, so it can stop working at any time.
