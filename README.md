# music_display

Rust port of `music_display_2.fish`: cover art, metadata, clickable controls, synced lyrics and cava in one zero-flicker TUI.

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
music_display --cava --underlyrics     # cava under the lyrics (also the default for plain --cava)
music_display -c ~/some/colors.json    # explicit colour file
music_display --surface '#1e1e2e' --primary '#cba6f7'
music_display --terminal-colors        # use the terminal's own theme
music_display --ascii-cover --cover-theme --tint 0
music_display --big-lyrics             # active lyric line in a big block font
```

### Colours

| flag | effect |
| --- | --- |
| *(none)* | first usable colour file wins (see Notes). If none is usable — missing, invalid, or `{}` — the app falls back to the terminal's colours, never to built-in hex values. |
| `--terminal-colors` | skip every colour file; default background/foreground, ANSI cyan accent, grays for fading. No hex colours are used. |
| `--surface HEX` / `--primary HEX` | override just that slot; combine freely with `--terminal-colors` (e.g. `--terminal-colors --primary '#ff8800'` keeps your terminal background). |
| `--cover-theme` | accent (borders, labels, active lyric, progress, cava) follows the dominant vivid colour of the current cover; updates every track. Works with `--terminal-colors`. |
| `--tint 0..1` | duotone strength on the cover itself. `0` = original artwork. Needs a known surface and accent to apply; with `--cover-theme` the duotone's highlight is the cover accent. |
| `--transparent` | don't paint the surface colour as the background. |

### Layout, lyrics, cover

| flag | effect |
| --- | --- |
| `--nolyrics` / `--cava` / `--thirdpanel` / `--underlyrics` | panel layout. In `--cava --thirdpanel` the cava panel is exactly as wide as the metadata panel; cava always runs in `mono`. |
| `--ascii-cover` | draw the cover as coloured ASCII characters (no graphics protocol needed). |
| `--lyric-spacing 0..3` | blank rows between lyric lines (default 1; dropped on very short panels). |
| `--big-lyrics` | render the active line in a 5×7 half-block font (also toggled with `b`). Lines with unsupported characters, or too long for the panel, fall back to bold text. |

| `--chords` | show guitar chords above the lyrics (toggle with `g`; off by default). |
| `--chords-dir DIR` | folder with your chord sheets (default `~/.local/share/music_display/chords`). |
| `--chords-ug` | when there is no local sheet, look the song up on Ultimate Guitar (unofficial, opt-in). |
| `--chords-probe ARTIST TITLE` | diagnose the Ultimate Guitar lookup for one song, print each step, exit. `--probe-save DIR` keeps the raw pages. |
| `--chords-context N` | how many upcoming lines show chords besides the active one (default 1). |

Other flags: `--player NAME` (default `spotify`, `any` = playerctl's pick), `--offset SECS` (default lyric offset per track), `--cava-bars N`. `music_display --help` lists everything.

## Keys

| key | action |
| --- | --- |
| `space` | play / pause |
| `n` / `p` | next / previous |
| `←` / `→` | seek −5 s / +5 s |
| `,` / `.` | lyric offset −0.1 s / +0.1 s |
| `l` | toggle lyrics panel |
| `b` | toggle big-font lyrics |
| `g` | toggle chords |
| `t` / `T` | transpose chords up / down one semitone (resets on track change) |
| `c` | toggle cava (process is killed while hidden) |
| `q` / `Esc` | quit |

Mouse: click the progress bar to seek, click `[ prev ] [ pause ] [ next ]`. Hold Shift to select text.

## Chords

Put your own sheets in the chords folder, named `Artist - Title.ext` (case, punctuation, a
`- Remastered` suffix and extra artists are ignored; `Title.ext` also works):

- `.cho` / `.chopro` / `.pro` / `.crd` / `.chordpro` — ChordPro: `[Am]Hello [G]darkness`, `{capo: 2}`
- `.txt` — chords written above the words (what most sites copy-paste to); `[ch]…[/ch]` tags,
  `[Verse]` headers and tab lines are stripped automatically

Chords are placed on the **synced lyrics** by matching lines (tolerates wording differences,
and a chorus the sheet writes once but the song repeats). The footer shows the file, the match
percentage and the capo; if fewer than 40 % of the lyric lines match, chords are hidden rather
than shown in the wrong place. Chords only appear on the active line and the next one (see
`--chords-context`); in `--big-lyrics` mode they are listed above the big line. `t`/`T` shift
every chord; the sheet's capo is shown, never applied.

### Ultimate Guitar (`--chords-ug`)

Order: your folder → cache → Ultimate Guitar. UG has no public API, so this reads the JSON
embedded in its pages and may break whenever the site changes; it is built to fail softly:

- at most 1 search + 3 page requests per song, and only for songs not already cached
- found sheets are cached for good (`~/.cache/music_display/chords-ug-*.json`, editable ChordPro);
  "no sheet" is cached for 7 days; errors are never cached
- three failures in a row (blocked, captcha, changed page) switch UG off until restart;
  the footer says why. Local sheets and already-cached songs keep working
- pages with fewer than 8 recognisable chords are rejected; candidates are ranked by rating × votes
- it sends a browser-like User-Agent (UG rejects unknown ones), so you are using it at your own
  risk with respect to their terms of service

If a song doesn't come up, run `music_display --chords-probe "Artist" "Title"` to see which step fails.

## Notes

- Colour files are checked in order: `~/.config/noctalia/colors.json`, `~/.cache/noctalia/colors.json`, `~/.config/matugen/colors.json`, `~/.cache/matugen/colors.json`, `~/.config/noctalia/theme.json`.
- Cover art uses Kitty graphics when available, then Sixel / iTerm2, then unicode half-blocks. The bitmap is scaled to exactly fill its cell box so there is no padding sliver at the edge.
- If cava can't start or dies, the cava panel shows why (cava's own error message); press `c` twice to retry.
- The cover's own pixels are always true-colour, even with `--terminal-colors`; everything else uses the terminal palette.
- Caches: `~/.cache/music_display/` (years, lyrics; misses re-checked after a few days, network errors never cached), cover downloads in `/tmp/music_display/`.
