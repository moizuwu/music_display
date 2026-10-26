# Chords in music_display

Guitar chords shown above the synced lyrics, on every line (the active line in bold).

```sh
music_display --chords                 # your own sheets only
music_display --chords --chords-ug     # + Ultimate Guitar when you have no local sheet
```

Off by default. Press `g` at any time to toggle.

## Keys

| key | action |
| --- | --- |
| `g` | chords on / off (also shows the lyrics panel) |
| `t` / `T` | transpose up / down one semitone (resets on track change) |
| `]` / `[` | capo up / down, 0–12 (a per-track override, remembered across restarts) |
| `0` | back to the sheet's own capo |
| `D` | chord diagram panel |
| `b` | big-font lyrics; chords are listed in a row above the big line |

## Flags

| flag | meaning |
| --- | --- |
| `--chords` | start with chords on |
| `--chords-dir DIR` | folder with your sheets (default `~/.local/share/music_display/chords`) |
| `--chords-context N` | limit chords to the active line + N upcoming ones (default 99 = every line) |
| `--chords-ug` | look songs up on Ultimate Guitar when there is no local sheet |
| `--chords-probe ARTIST TITLE` | diagnose the Ultimate Guitar lookup for one song and exit |
| `--probe-save DIR` | with `--chords-probe`: keep the raw pages |

## Where sheets come from

Order: **your folder → cache → Ultimate Guitar** (only with `--chords-ug`).

### Your own sheets

Name files `Artist - Title.ext` (`Title.ext` also works). Case, punctuation,
`- Remastered` / `(Live)` suffixes and extra artists are ignored. If several formats exist,
`.cho` wins over `.txt`.

| extension | format |
| --- | --- |
| `.cho` `.chopro` `.chordpro` `.crd` `.pro` | ChordPro: `[Am]Hello [G]darkness`, `{capo: 2}` |
| `.txt` | chords written above the words (what most sites copy-paste to) |

```
{capo: 2}
[Am]Hello dar[G]kness my old friend
```

```
Capo 2
   Am        G
Hello darkness my old friend
```

`[ch]…[/ch]` tags, `[Verse]` / `Chorus:` headers, `x2` marks and tab lines are stripped.
A file needs at least 4 recognisable chords to be used.

### Ultimate Guitar (`--chords-ug`)

Unofficial: UG has no public API, so this reads the JSON embedded in its pages and can break
when the site changes. It is built to fail softly.

- At most 1 search + 3 page requests per song, and only for songs not already cached.
- Only "Chords" pages whose artist and title match are considered, ranked by rating × ln(votes).
  Pages with fewer than 8 recognisable chords are rejected.
- Found sheets are cached for good in `~/.cache/music_display/chords-ug-*.json` (readable
  ChordPro). "No sheet" is cached 7 days. Errors are never cached.
- Three failures in a row (blocked, captcha, changed page) switch UG off until restart. The
  footer says why; local sheets and cached songs keep working.
- It sends a browser-like User-Agent because UG rejects unknown ones. Using it is at your own
  risk with respect to their terms of service, which is why it is opt-in.

To replace a bad cached sheet, put your own `Artist - Title.cho` in the chords folder (local
files take priority), or delete the song's `chords-ug-*.json`.

## How chords are placed

Chords are matched onto the **synced lyrics**, line by line:

1. Each lyric line is compared with the next ten sheet lines in order (fuzzy, so wording
   differences are fine).
2. If nothing matches, it is compared with the whole sheet (a chorus written once on the
   sheet but sung several times).
3. Each chord stays attached to the word it sat on, even if the sheet words the line a little
   differently.

The footer shows `chords: <file> <match %> · capo N · ±semitones`. If fewer than **40 %** of the
lyric lines match, chords are hidden (footer: "weak match") rather than shown in the wrong
place. Common causes: a live/acoustic/cover sheet, or lyrics from a different version.

Without synced lyrics (plain or none) there is nothing to place chords on, and the footer says so.

## Intro

Chord-only lines at the top of the sheet (before the first sung line) are shown as an
**intro** section before the first lyric; it is highlighted until the first lyric starts.

## Capo, key and transposing

Shapes vs sounding key: the sheet's chords are the shapes to finger with the sheet's capo.
`[` / `]` choose a different capo (0–12): the shapes are re-spelled, so a higher capo gives
lower shapes, and the sounding key stays the same. The metadata panel shows
`Key: E Minor / 9A` (sounding key + Camelot), `Capo: 3  (shapes in C#m)`, and
`[sheet: 2]` when you've overridden the sheet's capo. The override is saved per track
(`~/.cache/music_display/capo-*.txt`); `0` clears it.

The key comes from the sheet (`{key: Dm}` / `Key: Dm` / Ultimate Guitar's page data) and is
taken to be the key of the shapes; no key on the sheet means no Key / Camelot display.
`t` / `T` shift every chord (including the bass note in `C/E`) and the sounding key with it.
Flat or sharp spelling follows the key being shown (F, Bb, Eb, Ab, Db major and relatives
use flats), or the sheet's own spelling when there is no key.

## Chord diagrams (`D`)

A side panel with fretboard diagrams for the chords of the current stretch of the song
(the active line and the next few lines, up to 6 chords), with the chord being played
highlighted, plus a small tab underneath. The highlighted chord is an estimate from how far
into the line you are (sheets carry no timing). Built in: major, minor, 7, m7, maj7, sus2,
sus4 and power chords on all 12 roots; anything else (dim, aug, 9ths, slash chords) shows
"diagram unavailable" instead of a wrong diagram. The panel needs a lyrics column at least
56 columns wide and is hidden below that.

## Troubleshooting

| footer / symptom | meaning and fix |
| --- | --- |
| `no sheet found for this track` | not in your folder (or UG has none). Add a `.cho`/`.txt`, or use `--chords-ug` |
| `weak match (N%) — hidden` | sheet doesn't fit these lyrics; add your own sheet for this song |
| `found, but there are no synced lyrics…` | lrclib has no synced lyrics for the track |
| `HTTP 403 from ultimate-guitar` | UG blocked the request; wait, or use local sheets |
| `page format not recognised` | UG changed its page or served a captcha; run `--chords-probe` |
| `switched off after repeated failures` | three failures in a row; restart to retry |

Diagnose a song:

```sh
music_display --chords-probe "Artist" "Title" --probe-save ~/ug-fixtures
```

It prints the search, the ranked candidates, the fetched page and the first parsed lines. If
a step fails, that step is the problem.
