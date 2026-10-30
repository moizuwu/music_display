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

## As built (checked against the code)

- **Toolkit:** egui/eframe **0.37**, features `glow`, `wayland`, `x11`, `default_fonts`, `system_fonts`, `clipboard` (the default `wgpu` renderer is left out to keep the build lighter). Licence MIT OR Apache-2.0. `cargo build --release --features gui` produced a 22 MB binary here (the default TUI build has no GUI dependencies at all). The version-0.37 API differs from older examples: the app implements `ui(&mut self, ui, frame)` (plus `logic` for work without drawing) and panels are `egui::Panel::left(..).show(ui, ..)`.
- **No App extraction.** The proposal above (moving the `App` state machine into the core crate) was **not** done. The GUI has its own small `Model` (`src/gui/model.rs`, no egui types, unit-tested) fed by the same `AppEvent` tasks, the same pollers (`Ctl::poll_loop` / `follow_loop`) and the same core modules (`lyrics`, `chords`, `translate`, `ai`, `annotations`, `genre`, `genius`, `cover`, `layout`, `config`). The cost is some duplicated orchestration (about 500 lines); the gain is that the terminal front end was left untouched. Extracting the shared part remains a sensible follow-up.
- **Shared with the TUI:** `config.toml` (flag > file > default, `[layout]`, `[ai]`), the panel ids and the layout model (`src/layout.rs`), `--layout-preset`, `--reset-layout`.
- **Fonts:** CJK / Hangul rely on `system_fonts` finding a suitable system font; this was not tested on every platform.
- **Not verified:** that the window looks right on every compositor; the build and the model logic are covered by tests, drawing is not.
- **Look and customization (`[gui]`):** the table is plain data in `src/guiconf.rs` (in the library, so it is tested without egui and `--print-config` can show it; `config::apply` skips it, `guiconf::load` / `save` own it). Keys: `opacity`, `panel_opacity`, `theme`, `accent` (`cover` or hex), `background` (`solid`, `cover-blur`, `gradient`), `font`, `lyrics_size`, `active_size`, `meta_size`, `active_weight` (`regular`, `medium`, `bold`), `corner_radius`, `panel_spacing`, `panel_borders`, `lyrics_align`, `inactive_fade`, `lyrics_animation`, `remember_window`, `always_on_top`, `decorations`; out-of-range numbers are clamped with a warning. The egui side is `src/gui/style.rs`: it builds the styles of both themes (see-through panels, accent as hyperlink and selection colour, corner radius, text sizes), the fonts (an installed family picked through `fontique`, plus a medium / bold family for the active line, since egui has no synthetic bold), paints the background (the cover accent comes from `cover::dominant_accent`; "blur" is the cover shrunk to 24x24 and stretched with linear filtering, not a real blur), draws the panel cards and the round transport icons, and holds the Appearance tab (written to the file when the pointer is released, so a slider drag does not write every frame). The window is always created transparent so `opacity` can change live. Window size and position use eframe's `persistence` feature (`persist_window`); egui's own memory is not persisted, so panel widths still come from `[layout.size]`. A font that is not installed falls back to the built-in one. **Not verified:** the drawing, transparency and always-on-top on Wayland and X11 were not looked at (no display here); only the config parsing, clamping and round trip are tested.
- **Visual extras (`src/gui/extras.rs`):** cava bars (`cava::run` started and stopped by `Model::sync_cava`, drawn with the painter), the minimal view, the layout preset picker, lyric versions (`l`) and the per-track capo files (the same `capo-<key>.txt` as the terminal UI; written synchronously because separate write and delete tasks could finish out of order). The size and weight of the active line belong to `[gui]` (`guiconf`), not to this file. Drawing was not checked in a real window.
- **More customization (`ui_scale`, `theme_preset`, `background_image`, `lyrics_glow`, `lock_layout`, `popped`):** `ui_scale` goes to `Context::set_zoom_factor`; egui's own zoom keys are noticed in `Gui::window_keys` and written back, which also handles Ctrl+Shift+Up / Down (opacity). `GuiConf::apply_preset` holds the Halloween values; `Preset::base` is its window colour. The moving backgrounds are drawn from the clock in `src/gui/backdrop.rs` (embers, fog, bats built from convex polygons, no assets), the picture is decoded once per path in `Look::load_picture`, and the glow is a few stacked rounded rects pushed behind the active line (`lyrics_view.rs`). Pop-out panels (`src/gui/popout.rs`) are *immediate* viewports, because a deferred one needs a `'static` closure and could not borrow the model; the cost is that the child redraws with the parent. Closing the child window docks the panel. **Not verified:** none of this was run in a real window (no display here).
- **Spotify library (`src/gui/library.rs`):** the terminal UI's `Library`, `Level`, `ListRow` and `DevicePicker` (`browser.rs`) and the `Api` / `Cmd` / `Reply` requests of `spotify.rs`, with the reply handling ported from `app/remote.rs`. The client exists only with `spotify = true`. Login, logout and the client rebuild are in the settings (`Spot::toggle_login` / `rebuild`); no preloading. Keyboard navigation in the window: `library::key`. Mock-server tests only; never run against real Spotify.

- **Keys, help, settings, privacy (`keys.rs`, `ux.rs`, `settings.rs`, `privacy.rs`):** the window reads egui events as terminal key codes (`keys::code_of`: typed text for characters, so `L` vs `l` follows what was typed, key events only for arrows, Esc, Enter, Tab) and looks them up in the terminal's `Keymap` (`BINDS` plus `[keys]`); the help window is generated from the same table. The settings window mirrors the terminal menu's groups with the same config keys; setters are `Model` methods without egui (tested for the keys they write). Lyrics go to a non-local AI provider only after the one-time answer (`ai-notice-ack`, shared with the TUI); `ensure_translation` / `ensure_annotations` return while it is pending. `Ux::browse_*` records the lyrics browse keys (up/down/pgup/pgdn/End): the lyrics panel must read `take_browse()` / `browsing()` to apply them and to stop scrolling to the active line; `lyric_spacing`, `lyric_pad`, `big_lyrics` / `big_scale` and `interlude_threshold_secs` are saved and kept in `Args`, and the lyrics panel has to read them. **Unchecked:** none of this was run in a real window.
- **Runtime fixes after the parity audit:** the track-identity rules (`ignore_other_player`, `same_recording`, `keep_known`, `merge_tags`, the volume hold) are in `src/identity.rs` in the library and used by both front ends. The progress bar seeks on release (a preview while dragging) and the volume slider is debounced (150 ms) with a 1.5 s hold so polls do not move it back. Lyrics (`src/gui/lyrics_view.rs`) are drawn one wrapped row at a time so chords sit above their character; the view follows the playing line until the user scrolls (`src/gui/browse.rs`, 6 s) and plain lyrics scroll in proportion to the song; `Model::browse_by(rows)` and `Model::follow()` are the hooks for keys. Translation (`src/gui/translation.rs`) has one abortable task per (song, language), the disk cache, the never-translate list and hold-and-retry; a lyrics version with other content drops the translation (content hash, not length). The event channel wakes the window on every event (`src/gui/window.rs`), fonts are scanned on a background thread, only the metadata column scrolls, `layout.direction` is honoured, and spotifyd / SIGTERM are handled. **Not verified:** none of the drawing (chord placement, scrolling, footer, wrapping controls) was seen in a real window.
