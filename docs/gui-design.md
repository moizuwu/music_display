# GUI front end: egui/eframe vs iced

Goal: an optional `--gui` window behind cargo feature `gui`, reusing the core library crate (`src/lib.rs`). Version, size and timing figures are **from memory, verify** (not checked online; no web tool used).

## Comparison

| Aspect | egui / eframe | iced |
| --- | --- | --- |
| Licence | MIT or Apache-2.0 | MIT |
| Wayland / X11 | Both via winit (eframe 0.2x/0.3x); needs the `wayland` and `x11` features enabled explicitly | Both via winit; same caveat for client-side decorations on some compositors |
| Cover image | `egui::ColorImage` -> texture from the `image` crate buffer; trivial, we already decode with `image` 0.25 (`cover::Loaded.image`) | `widget::image` / `Handle::from_rgba`; fine, needs the `image` feature |
| Binary / compile | Roughly 8-15 MB stripped, clean build about 1-2 min (approximate) | Similar or slightly larger with wgpu, about 1.5-3 min (approximate); `tiny-skia` software backend is lighter |
| Async / tokio | Immediate mode: own the tokio runtime on a thread, drain the `AppEvent` mpsc receiver each frame, call `ctx.request_repaint()` from a waker task. Simple, fits the existing channel | Elm-style: `Subscription` / `Task` map to messages. Cleaner in theory, but our `AppEvent` channel needs wrapping in a subscription and iced's tokio feature must match our runtime |
| CJK / Hangul text | Default fonts have no CJK: must bundle or load a system font (e.g. Noto Sans CJK) and add it as fallback. Basic shaping only (no complex scripts) | `cosmic-text` shaping with system font fallback: CJK and Hangul work out of the box, better for lyrics |
| Chord diagrams | `Painter` is immediate: lines, circles, text in a few calls. Very easy | `canvas` widget with `Frame`/`Path`; fine but more ceremony (cache, `Program` trait) |
| Theming | `Visuals` + `Style`, easy runtime recolour from cover accent (matches `--cover-theme`) | Custom `Theme`/`Palette` with per-widget style fns; powerful, more boilerplate |
| Parity effort | Lower: per-frame redraw suits karaoke highlight and scrolling lyrics | Higher: state/view/update split is tidy, but animation and custom lists need more work |

## Recommendation

**egui/eframe.** Honest reasoning: iced wins on text (CJK shaping out of the box) and architectural neatness, but this app is a live, per-frame display (smooth lyric scroll, karaoke fill, chord diagrams, cover). Immediate mode makes those cheap, and draining our existing `AppEvent` channel into an egui frame loop is the least glue. The CJK weakness is real but solved by one bundled/system fallback font, which we need to test early (milestone 2). If Hangul rendering quality turns out unacceptable, revisit iced; the core extraction below keeps that switch cheap.

## Reuse vs new

Reused unchanged from the library: `player` (Snapshot, Ctl, Loop), `lyrics` (fetch, `parse_lrc`, `Lyrics`, versions), `chords` (parse, transpose, `align`, capo), `diagram` (shapes; the GUI draws them itself from `lookup`, not `render` strings), `translate`, `genius` (member parts), `spotify`, `config` (keys, load/save), `cover` (`load`, `dominant_accent`), `tags`, `year`, `util`, `AppEvent`. `cava` can be used for bars if wanted.

New / must move: the `App` state machine in `src/app/` (about 3k lines: `mod.rs`, `actions.rs`, `remote.rs`, `settings.rs`) lives in the TUI binary and holds terminal types (`ratatui_image::Picker`/`Protocol`, `ratatui::Rect`, crossterm key types, `Theme` with terminal colours). Proposal:

1. Extract a `core` module (in the library) holding the pure state: current track, position anchor and interpolation, lyrics/active line/offset, chords state + alignment, translation, members, Spotify state, settings model, and `handle(AppEvent)` plus a terminal-free `Action` enum with `run(Action)`.
2. Leave in the binary only: layout rects, `Picker`/protocol cache, key events -> `Action` mapping (`keymap.rs`), drawing in `src/ui/`.
3. Make colours neutral (`Rgb`, not `ratatui::Color`); keep `Theme` conversion in the TUI.
4. Put GUI code in `src/gui/` (one file per module), compiled only with `--features gui`; the egui deps are optional so TUI builds stay unchanged.
5. Karaoke maths (`ui/karaoke.rs::line_progress`) and `ui/text.rs` wrapping that are not terminal-specific should move to the core so both front ends share them.

Do the extraction first, as its own change, with the TUI tests (`src/tests.rs`) as the safety net.

## Milestones

1. **Window shell**: eframe app, runtime thread, `AppEvent` pump, title/artist/album, cover texture, play/pause/next/prev/seek/volume via `Ctl`.
2. **Synced lyrics**: scrolling active line, offset nudging, karaoke fill, CJK font fallback check, lyric versions.
3. **Chords + diagrams**: aligned chords above lines, transpose/capo, painter-drawn diagrams from `diagram::lookup`.
4. **Settings + customization**: settings window over `config` keys, cover-accent theming, light/dark, saved window state.
5. **Annotations + translation**: translated lines shown with originals, member-part tags from `genius`, translate settings (skip list, fallback).

## May be left out of the GUI

- cava bars (needs external tool, TUI-centric look)
- big block font (`bigfont.rs`): the GUI just uses a large font
- terminal image protocol handling (Kitty/Sixel/halfblock), ASCII cover art, `--window-title` terminal title stack
- terminal key remapping UI (`keymap.rs`); GUI can use fixed shortcuts first
- `--minimal` teleprompter layout and three-panel layout flags (replace with resizable panels)
- setup helpers for terminals (`setup.rs` where terminal-specific)
