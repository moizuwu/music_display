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
```

Other flags: `--player NAME` (default `spotify`, `any` = playerctl's pick), `--tint 0..1` (cover duotone strength, `0` = original art), `--offset SECS` (default lyric offset per track), `--cava-bars N`, `--transparent` (don't paint the surface colour as background). `music_display --help` lists everything.

## Keys

| key | action |
| --- | --- |
| `space` | play / pause |
| `n` / `p` | next / previous |
| `←` / `→` | seek −5 s / +5 s |
| `,` / `.` | lyric offset −0.1 s / +0.1 s |
| `l` | toggle lyrics panel |
| `c` | toggle cava (process is killed while hidden) |
| `q` / `Esc` | quit |

Mouse: click the progress bar to seek, click `[ prev ] [ pause ] [ next ]`. Hold Shift to select text.

## Notes

- Colour files are checked in order: `~/.config/noctalia/colors.json`, `~/.cache/noctalia/colors.json`, `~/.config/matugen/colors.json`, `~/.cache/matugen/colors.json`, `~/.config/noctalia/theme.json`.
- Cover art uses Kitty graphics when available, then Sixel / iTerm2, then unicode half-blocks.
- Caches: `~/.cache/music_display/` (years, lyrics; misses re-checked after a few days, network errors never cached), cover downloads in `/tmp/music_display/`.
