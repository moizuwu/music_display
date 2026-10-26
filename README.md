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
translate = false              # start with translated lyrics (toggle with K; k keeps the original line too)
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
spotify_device = ""            # only if you run spotifyd: its `device_name` (e.g. "Living Room PC"), so it
                               # counts as "this PC". Empty = just the Spotify app on this computer
use_spotifyd = false           # spotifyd is the player on this PC (set up with --setup): `spotify_device` then
                               # counts as "this PC"; off = only the Spotify app on this computer does.
                               # While on, spotifyd is started with the app (if not already running)
kill_spotifyd_on_exit = true   # stop the spotifyd this app started when it quits (never one that was already running)
spotify_button = true          # the [ library ] button at the top left (only shown while Spotify is on)
spotify_queue_tab = true       # the Queue tab in the library browser
# [keys]                       # must come last in the file; see "Remapping keys" below
# next = "j"
# mute = ["w", "F5"]
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
| `--spotify-device NAME` | only if you run spotifyd: its Spotify Connect name (the `device_name` in its config, say `Living Room PC`), so it counts as "this PC". Default: empty, which matches only the Spotify app on this computer (named after the host name) and devices of type "Computer". |
| `--use-spotifyd` | spotifyd is the player on this PC: the `spotify_device` name then counts as "this PC" (for `H`, play and the device list). Off, only the Spotify app on this computer does. While on, `spotifyd --no-daemon` is started together with the app unless one is already running. `--setup` turns it on for you (see "Setup (spotifyd)" below); also a row in the settings menu. |
| `--no-kill-spotifyd` | leave the spotifyd that this app started running when it quits (by default it is stopped again; a spotifyd that was already running is never touched). Config: `kill_spotifyd_on_exit = false`; also the *Stop spotifyd on exit* row in the settings menu. |
| `--setup` | guided one-time setup of spotifyd and the Spotify login, then exit (see "Setup (spotifyd)" below). |
| `--no-spotify-button` | hide the `[ library ]` button at the top left (config `spotify_button = false`; also in the settings menu). It only exists while Spotify is on; `o` opens the library either way. |
| `--no-spotify-queue-tab` | hide the Queue tab of the library browser (config `spotify_queue_tab = false`; also in the settings menu). |
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
| `K` | translate lyrics on / off: the translation replaces the original line (cached per song while the app runs) |
| `k` | translation on / off with the original kept: each line is followed by its translation (italic). With big lyrics the translation replaces the line as with `K`. `K` and `k` switch between the two modes |
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
| `s` / `S` | settings menu (`Esc`, `q`, `s` or `S` close it): toggles and steppers for the player source (Spotify, VLC, mpv, Rhythmbox, local, any), chords, translation, minimal view, big lyrics, spacing, cava and more, and a **Spotify** group (on / off, Client ID, log in / out, spotifyd name, status). Rows that take text (`Enter` starts typing, `Enter` saves, `Esc` cancels) swallow every key, so `q` and `s` are letters while you type. Changes apply immediately and are saved to the config file (comments in it are not kept); items marked `*` need a restart |
| `c` | toggle cava (process is killed while hidden) |
| `?` | help overlay (↑/↓ scroll, any other key closes) |
| `o` | Spotify library browser (`--spotify`, see below) |
| `f` | like / unlike the song playing (`♥` / `♡` beside the title) |
| `H` | play here: move Spotify playback from the phone / another device to this PC |
| `d` | choose a Spotify device (the list of where Spotify can play; `Enter` moves playback there) |
| `q` / `Esc` | quit |

Mouse: click the progress bar to seek; click `[ shuf ]   [ rep ]   [ prev ]   [ pause ]   [ next ]   [ - ] [ 65% ] [ + ]` (three blank columns between buttons; shuffle and repeat are lit while on and hidden when the player reports neither); the `[ - ] [ 65% ] [ + ]` part changes the volume, clicking the percentage mutes (`[ MUTE ]`) and restores, the wheel over the control changes it too; **click on a lyric line's text to seek to it** (synced lyrics only; works through wrapped lines and chord rows; the blank space beside the text does nothing). Hold Shift to select text.

The control row scales with the window and only ever draws whole buttons. When it gets narrow it gives up, in this order: the volume moves to its own row below the buttons; labels turn compact (`[sh] [rp] [<<] [|>] [>>]`, volume `[-] 65% [+]`); shuffle and repeat are dropped; the volume shrinks to `[-][+]` and then goes; last prev / next go, so play stays longest. Keyboard shortcuts work whatever is hidden. When the window is short, the controls and progress bar keep their rows first, then the metadata, and the cover shrinks or disappears before any of them. The key hints at the bottom of the library browser follow the same rule: whole hints are left out, never cut.

Shuffle and repeat are read with `playerctl shuffle` / `playerctl loop` about every 2 s and shown as they are reported. Some players (Spotify desktop in particular) may ignore the write, in which case the button simply flips back on the next poll.

The volume control follows the player: changes made elsewhere (media keys, the Spotify app) show up within a poll. It is hidden when the player doesn't report a volume.

## Remapping keys

The `[keys]` section of `config.toml` changes the keys of the main view, one line per action: `action = "key"`, or a list of keys. It has to come after every other setting in the file (a TOML section runs to the end of the file).

```toml
[keys]
next = "j"                     # n no longer does anything; j plays the next track
prev = "k"
mute = ["w", "F5"]             # several keys for one action
big_scale = "2"                # unshifted keys work on any layout
```

**Key syntax.** One character (`"n"`, `"T"`, `"["`, `"+"`; case matters, `"T"` is a capital T), or a name: `space`, `enter`, `esc`, `tab`, `backspace`, `left`, `right`, `up`, `down`, `pageup`, `pagedown`, `home`, `end`, `f1` … `f12`. Modifiers (`ctrl+x`, `alt+x`, `shift+x`) are not supported: the key handler ignores Ctrl and Alt, so use the plain key, a capital for Shift.

**Actions** (default keys in brackets): `quit` (`q`, `Esc`), `play_pause` (`space`), `next` (`n`), `prev` (`p`), `shuffle` (`z`), `repeat` (`r`), `seek_back` (`←`), `seek_fwd` (`→`), `vol_up` (`+`, `=`), `vol_down` (`-`), `mute` (`m`), `sync_earlier` (`,`), `sync_later` (`.`), `toggle_lyrics` (`l`), `toggle_big` (`b`), `big_scale` (`x`), `line_spacing` (`i`), `lyric_pad` (`u`), `browse_up` (`↑`), `browse_down` (`↓`), `browse_page_up` (`PgUp`), `browse_page_down` (`PgDn`), `browse_end` (`End`), `translate` (`K`), `translate_both` (`k`), `toggle_chords` (`g`), `transpose_up` (`t`), `transpose_down` (`T`), `capo_up` (`]`), `capo_down` (`[`), `capo_reset` (`0`), `diagrams` (`D`), `toggle_cava` (`c`), `minimal` (`M`), `settings` (`s`, `S`), `help` (`?`), `library` (`o`), `like` (`f`), `play_here` (`H`), `devices` (`d`).

**Rules.**
- An entry **replaces** that action's default keys. An action you don't mention keeps its keys.
- A problem never stops the program and never touches the other lines: an unknown action, a key that isn't one, a modifier, the same key twice in a list, an empty list, or a key that another action in the final table already uses each print a warning with the line number (`config.toml:3: [keys] next: 'p' is already bound to prev; keeping the default`) and that action keeps its default keys. Two actions can still swap keys (`next = "p"` with `prev = "n"`). When two entries want the same key, the earlier line keeps it.
- `quit` and `help` can never lose their keys: an entry for them only **adds** keys, and no other action can take `q`, `Esc` or `?`. `Ctrl+C` always quits.
- The library browser, the settings menu, the device list and the help overlay have their own fixed keys (listed in the help); only the main-view keys above are remappable.
- The `?` overlay and the key hints in the footer show the keys you configured. `music_display --print-config` ends with a `[keys]` block of the effective bindings, every action with its keys, which is valid config to paste back.
- The settings menu writes the file again when you change a setting: your `[keys]` lines are kept (as the same values, in alphabetical order), but comments in the file are not, as for every other setting.

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

With `--spotify` (or `spotify = true`, or the switch in the settings menu) the app talks to the Spotify Web API itself, no other program or helper needed. While it is off nothing happens: no request, no extra process, no prompt, and `o` / `f` / `H` only say how to turn it on.

**Setup, once:**

1. Create an app at <https://developer.spotify.com/dashboard> and add the redirect URI `http://127.0.0.1:8897/callback` exactly.
2. `music_display --spotify-client-id <Client ID> --spotify-login` opens the browser (if it doesn't, the link is printed). After you allow access the token is saved in `~/.local/state/music_display/spotify/token.json` (mode 600), together with the Client ID, so later runs only need `--spotify`. The token refreshes itself; `--spotify-logout` deletes it.

The login uses PKCE: no API key, no client secret, only the Client ID, which is not secret. In development mode Spotify only lets the users you add under the app's *User Management* log in, so add your own account there if the login is refused. The login waits 3 minutes for the browser; if port 8897 is taken it says so.

**From the settings menu (`s`)** the same thing works without leaving the app: in the **Spotify** group switch *Spotify features* on, open *Client ID* (`Enter`, paste or type it, `Enter`), then *Log in*. The login runs in the background ("waiting for the browser…", pressing it again cancels; if the browser can't be opened it says so and the command line login above prints the link). *Log out* deletes the token. The status line under the group header reads `logged in`, `not logged in`, `token expired: log in again`, `off`, or Spotify's own error. Everything is saved to the config file; the switch takes effect at once, no restart. Until you are logged in, `o` / `f` / `H` say so.

**Is spotifyd your player?** Then put its `device_name` into *spotifyd device name* (`spotify_device`) so that it counts as "this PC" for `H` and the device checks. This is only an example of what to enter; nothing is assumed by default, and with the field empty just the Spotify app on this computer is matched.

**Library browser (`o`, or the `[ library ]` button at the top left).** Tabs: Recent · Playlists · Liked · Albums · Artists · Search · Queue. On a terminal at least 110 columns wide it is a **panel on the left** and the player keeps the rest of the width: the player stays usable beside it: keys the browser doesn't use (`n` / `p`, `+` / `-`, `m`, `z`, `M`…) still reach it, and so do clicks on its buttons and progress bar (the browser keeps `space`, `Enter`, `r`, `f` and `l`, so those mean what the table below says while it is open). On a narrower terminal it is an overlay over the middle of the screen, and in the minimal view it is always an overlay. `Esc` (or `q`, or `o` again, or the button) closes it.

| key | action |
| --- | --- |
| `Tab` / `Shift-Tab`, `1`–`7` | switch tab (`6` search, `7` queue) |
| `Q` | the Queue tab (`q` is quit, so the queue has a capital) |
| `/` | search: jumps to the Search tab with the field ready (typing swallows every key, `q` and `o` included; `Enter` runs the search, `Esc` leaves the field) |
| `d` | choose a device (see below) |
| `↑` / `↓`, `j` / `k`, `PgUp` / `PgDn`, `Home` / `End`, wheel | move |
| `Enter` / `space` | play: a song plays in its list (album, playlist, Liked Songs) from that song on; an album / playlist / artist plays itself |
| `→` / `l` | open an album, your own playlist, an artist (albums and singles, newest first) or Liked Songs (`›` marks what opens) |
| `←` / `h` / `Backspace` | back |
| `a` | add the song to the queue |
| `A` | add the song to one of your playlists (a list of them opens; `Enter` adds) |
| `f` | like the song |
| `r` | reload the list |
| `Esc` / `q` / `o` | close |

Spotify only lets development-mode apps open your own (and collaborative) playlists; other people's play with `Enter`. Liked Songs shows the first 1000.

**Mouse.** It works the same in the panel and in the overlay, in exactly the places that are drawn: click a tab to switch to it, click a row to select it, click the selected row again to play it (that is `Enter`), the wheel moves the selection three rows at a time, and `Esc` closes. A click outside closes the overlay; beside the side panel a click outside is for the player (its buttons and the progress bar work as usual) and leaves the panel open. The device list (`d`) takes clicks the same way.

**Loading and the cache.** Nothing is requested until Spotify is on and you are logged in. Then the main lists (Recent, Playlists, Albums, Artists) load in the background, one after the other, so they are there when you open the browser; Liked Songs (up to 1000 songs) loads when you first open it. What was loaded is kept in memory for the session and shown at once when you open the browser or switch tabs; a list older than a minute is fetched again in the background while the old rows stay on screen (`refreshing…`), a fresh one costs no request at all. The queue is asked for again almost every time you look at it. Nothing is written to disk. When Spotify answers `429` it says how long to wait (`Retry-After`), and until that time has passed no request is sent at all, not by the browser, the background load or the player polling.

**Queue (`Q`, tab 7).** What is playing now and what comes next (`GET /me/player/queue`). `Enter` plays the one under the cursor (Spotify lets an app start a song, not jump inside the queue, so it plays that song on its own). If Spotify refuses it you see `Queue unavailable: <reason>`. The tab can be switched off in the settings menu (*Queue tab in the library*) or with `spotify_queue_tab = false`.

**Search.** One request finds songs, albums, artists and playlists (at most 10 of each, the most a development-mode app gets), listed under headings. The keys are the same as in the other tabs: `Enter` plays (a song plays alone), `a` queues, `A` adds to a playlist, `f` likes, `→` opens an album, an artist or one of your own playlists, `←` goes back to the results where you left them. While a new search runs the old results stay on screen, dimmed, and an answer to an older search that arrives late is dropped.

**Devices (`d`, also inside the browser).** Lists where Spotify can play right now (phone, computer, speakers): `▶` marks the one playing, `this PC` the Spotify app on this computer (or spotifyd when `spotify_device` names it). `Enter` moves playback there and keeps playing; `r` reloads the list. A device Spotify restricts can't be controlled by apps and says so; with no device at all the list tells you to open Spotify somewhere or start spotifyd.

**Errors.** What Spotify refuses is shown in the footer instead of failing silently: `429` as "rate limiting: try again in N s" (from its `Retry-After`), `403` as "Premium is needed" or Spotify's own reason, and a command with nothing playing anywhere as "no active device: press d".

**Other devices.** When no MPRIS player is running (the phone plays, or Spotify runs elsewhere), the app shows what Spotify says plays, with lyrics, cover and chords as usual, and every control (play / pause, next / previous, seek, volume, shuffle, repeat) goes to that device through the API; the footer says `on <device>`. It polls every 1.5 s while that plays (3 s paused, 5 s while an MPRIS player is on screen). `H` moves playback to this PC (the Spotify app when it is open, else spotifyd when `spotify_device` names it), and the MPRIS player takes over from there. Play with nothing active anywhere carries on here from where Spotify last stopped, or starts the last played song.

## Setup (spotifyd)

Optional. [spotifyd](https://github.com/Spotifyd/spotifyd) turns this PC into a Spotify Connect device, so music_display can play on it without the Spotify app. Nothing is set up unless you ask: `music_display --setup` does it once, step by step:

1. Checks that `spotifyd` is installed. If it isn't, it prints the install command for your distribution (pacman, apt, dnf or zypper) or the upstream link, and stops without changing anything.
2. Runs `spotifyd authenticate` in your terminal (spotifyd's own login; follow its prompts).
3. Logs you in to Spotify in the browser (the PKCE login from the Spotify section; asks for your app's Client ID unless `spotify_client_id` is already set).
4. Writes `~/.config/spotifyd/spotifyd.conf` (just `device_name`) and the entries `spotify = true`, `spotify_client_id`, `spotify_device` and `use_spotifyd = true` to `config.toml`. The device name is asked for (default: your `spotify_device`, else `music_display`).
5. Checks that spotifyd is running and that the device shows up in your Spotify devices, and says which of the two is missing if not.

An existing `spotifyd.conf` or `config.toml` is only replaced if you answer yes (the default is no), and is copied to `<name>.bak-<timestamp>` first. If you say no, the lines to add by hand are printed. Neither file holds a token or password: spotifyd keeps its credentials in its own cache and the login token stays in `~/.local/state/music_display/spotify/`. `--setup` never starts spotifyd itself, but with `use_spotifyd` on the app does: it runs `spotifyd --no-daemon` at startup unless spotifyd is already running (for example as a service) and stops that one again when it quits; turn *Stop spotifyd on exit* (`kill_spotifyd_on_exit`) off to leave it running. `use_spotifyd` (also in the settings menu) says spotifyd is the player on this PC, so `spotify_device` counts as "this PC"; leave it off if you only use the Spotify app or other devices.

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
