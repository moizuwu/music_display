/// 5×7 bitmap, one `u8` per row, bit 4 = leftmost pixel.
fn base_glyph(c: char) -> Option<[u8; 7]> {
    Some(match c {
        ' ' => [0; 7],
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        ',' => [0, 0, 0, 0, 0b01100, 0b00100, 0b01000],
        '!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100],
        '?' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100],
        '\'' => [0b01100, 0b00100, 0b01000, 0, 0, 0, 0],
        '"' => [0b01010, 0b01010, 0, 0, 0, 0, 0],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        ':' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0],
        ';' => [0, 0b01100, 0b01100, 0, 0b01100, 0b00100, 0b01000],
        '(' => [
            0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
        ],
        ')' => [
            0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
        ],
        '&' => [
            0b01100, 0b10010, 0b10100, 0b01000, 0b10101, 0b10010, 0b01101,
        ],
        '/' => [
            0b00001, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b10000,
        ],
        '+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
        '=' => [0, 0, 0b11111, 0, 0b11111, 0, 0],
        '*' => [0, 0b00100, 0b10101, 0b01110, 0b10101, 0b00100, 0],
        '#' => [
            0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
        ],
        '%' => [
            0b11000, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b00011,
        ],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        '♪' => [
            0b00010, 0b00011, 0b00010, 0b00010, 0b01110, 0b11110, 0b01100,
        ],
        _ => return None,
    })
}

/// Accent marks as a top row. The letter's top two rows are merged to make room for it,
/// so accented capitals stay 7 rows tall.
fn accented(c: char) -> Option<[u8; 7]> {
    let (base, mark): (char, u8) = match c {
        'Ä' => ('A', 0b01010),
        'Ë' => ('E', 0b01010),
        'Ï' => ('I', 0b01010),
        'Ö' => ('O', 0b01010),
        'Ü' => ('U', 0b01010),
        'Ÿ' => ('Y', 0b01010),
        'Å' => ('A', 0b00100),
        'Á' | 'É' | 'Í' | 'Ó' | 'Ú' | 'Ý' => (
            match c {
                'Á' => 'A',
                'É' => 'E',
                'Í' => 'I',
                'Ó' => 'O',
                'Ú' => 'U',
                _ => 'Y',
            },
            0b00010,
        ),
        'À' | 'È' | 'Ì' | 'Ò' | 'Ù' => (
            match c {
                'À' => 'A',
                'È' => 'E',
                'Ì' => 'I',
                'Ò' => 'O',
                _ => 'U',
            },
            0b01000,
        ),
        'Â' | 'Ê' | 'Î' | 'Ô' | 'Û' => (
            match c {
                'Â' => 'A',
                'Ê' => 'E',
                'Î' => 'I',
                'Ô' => 'O',
                _ => 'U',
            },
            0b00100,
        ),
        'Ñ' => ('N', 0b01101),
        'Ø' => return Some([0b01110, 0b10011, 0b10101, 0b10101, 0b10101, 0b11001, 0b01110]),
        _ => return None,
    };
    let g = base_glyph(base)?;
    Some([mark, g[0] | g[1], g[2], g[3], g[4], g[5], g[6]])
}

/// Which glyph set a piece of text is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    /// Latin 5×7, Hangul composed on 8×8.
    Normal,
    /// Latin 3×5 (no Hangul), for panels too narrow for the normal one.
    Compact,
    /// Hangul on a 12×12 grid (strokes drawn per jamo), Latin 5×7 beside it; used by
    /// the caller when the line has Hangul and the room is there, else [`Font::Normal`].
    Fine,
}

impl Font {
    /// Pixel rows in one piece, including the blank row under it (they pair up into
    /// half-block rows).
    fn grid_h(self) -> usize {
        match self {
            Font::Normal => 8,
            Font::Compact => 6,
            Font::Fine => 12,
        }
    }
}

/// One drawable character: `w` pixel columns, bit `w - 1 - x` of a row is column `x`.
#[derive(Clone, Copy)]
struct Glyph {
    w: usize,
    rows: [u16; 12],
}

fn latin(c: char) -> Option<Glyph> {
    let g = base_glyph(c).or_else(|| accented(c))?;
    let mut rows = [0u16; 12];
    for (r, b) in rows.iter_mut().zip(g) {
        *r = b as u16;
    }
    Some(Glyph { w: 5, rows })
}

/// A Latin glyph placed on the 12-row Fine grid, a little above the middle like the Latin
/// letters of a Korean typeface sit next to Hangul.
fn latin_fine(c: char) -> Option<Glyph> {
    let g = latin(c)?;
    let mut rows = [0u16; 12];
    rows[3..10].copy_from_slice(&g.rows[..7]);
    Some(Glyph { w: g.w, rows })
}

/// Compact 3×5 variant for panels too narrow for the normal font (letters, digits and a
/// little punctuation; everything else is `None`).
fn compact_glyph(c: char) -> Option<Glyph> {
    let g: [u8; 5] = match c {
        ' ' => [0; 5],
        'A' => [0b010, 0b101, 0b111, 0b101, 0b101],
        'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
        'C' => [0b011, 0b100, 0b100, 0b100, 0b011],
        'D' => [0b110, 0b101, 0b101, 0b101, 0b110],
        'E' => [0b111, 0b100, 0b110, 0b100, 0b111],
        'F' => [0b111, 0b100, 0b110, 0b100, 0b100],
        'G' => [0b011, 0b100, 0b101, 0b101, 0b011],
        'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
        'I' => [0b111, 0b010, 0b010, 0b010, 0b111],
        'J' => [0b001, 0b001, 0b001, 0b101, 0b010],
        'K' => [0b101, 0b101, 0b110, 0b101, 0b101],
        'L' => [0b100, 0b100, 0b100, 0b100, 0b111],
        'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
        'N' => [0b110, 0b101, 0b101, 0b101, 0b101],
        'O' => [0b010, 0b101, 0b101, 0b101, 0b010],
        'P' => [0b110, 0b101, 0b110, 0b100, 0b100],
        'Q' => [0b010, 0b101, 0b101, 0b110, 0b011],
        'R' => [0b110, 0b101, 0b110, 0b101, 0b101],
        'S' => [0b011, 0b100, 0b010, 0b001, 0b110],
        'T' => [0b111, 0b010, 0b010, 0b010, 0b010],
        'U' => [0b101, 0b101, 0b101, 0b101, 0b111],
        'V' => [0b101, 0b101, 0b101, 0b101, 0b010],
        'W' => [0b101, 0b101, 0b111, 0b111, 0b101],
        'X' => [0b101, 0b101, 0b010, 0b101, 0b101],
        'Y' => [0b101, 0b101, 0b010, 0b010, 0b010],
        'Z' => [0b111, 0b001, 0b010, 0b100, 0b111],
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b110, 0b001, 0b010, 0b100, 0b111],
        '3' => [0b110, 0b001, 0b010, 0b001, 0b110],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b110, 0b001, 0b110],
        '6' => [0b011, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b110],
        '.' => [0, 0, 0, 0, 0b010],
        ',' => [0, 0, 0, 0b010, 0b100],
        '!' => [0b010, 0b010, 0b010, 0, 0b010],
        '?' => [0b110, 0b001, 0b010, 0, 0b010],
        '\'' => [0b010, 0b010, 0, 0, 0],
        '"' => [0b101, 0b101, 0, 0, 0],
        '-' => [0, 0, 0b111, 0, 0],
        ':' => [0, 0b010, 0, 0b010, 0],
        '(' => [0b010, 0b100, 0b100, 0b100, 0b010],
        ')' => [0b010, 0b001, 0b001, 0b001, 0b010],
        '/' => [0b001, 0b001, 0b010, 0b100, 0b100],
        '+' => [0, 0b010, 0b111, 0b010, 0],
        '=' => [0, 0b111, 0, 0b111, 0],
        '_' => [0, 0, 0, 0, 0b111],
        _ => return None,
    };
    let mut rows = [0u16; 12];
    for (r, b) in rows.iter_mut().zip(g) {
        *r = b as u16;
    }
    Some(Glyph { w: 3, rows })
}

// ── Hangul: syllables are composed from a few jamo shapes ───────────────
type Canvas = [[bool; 8]; 8];

/// 5×5 shapes of the basic consonants: ㄱ ㄴ ㄷ ㄹ ㅁ ㅂ ㅅ ㅇ ㅈ ㅊ ㅋ ㅌ ㅍ ㅎ.
const CONS: [[u8; 5]; 14] = [
    [0b11111, 0b00001, 0b00001, 0b00001, 0b00001],
    [0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
    [0b11111, 0b10000, 0b10000, 0b10000, 0b11111],
    [0b11111, 0b00001, 0b11111, 0b10000, 0b11111],
    [0b11111, 0b10001, 0b10001, 0b10001, 0b11111],
    [0b10001, 0b10001, 0b11111, 0b10001, 0b11111],
    [0b00100, 0b00100, 0b01010, 0b01010, 0b10001],
    [0b01110, 0b10001, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00100, 0b01010, 0b01010, 0b10001],
    [0b00100, 0b11111, 0b00100, 0b01010, 0b10001],
    [0b11111, 0b00001, 0b11111, 0b00001, 0b00001],
    [0b11111, 0b10000, 0b11111, 0b10000, 0b11111],
    [0b11111, 0b01010, 0b01010, 0b01010, 0b11111],
    [0b00100, 0b11111, 0b01110, 0b10001, 0b01110],
];

/// Initial consonant index (0–18) → (index into [`CONS`], drawn twice for ㄲ ㄸ ㅃ ㅆ ㅉ).
fn cons_shape(id: u8) -> (usize, bool) {
    match id {
        0 => (0, false),
        1 => (0, true),
        2 => (1, false),
        3 => (2, false),
        4 => (2, true),
        5 => (3, false),
        6 => (4, false),
        7 => (5, false),
        8 => (5, true),
        9 => (6, false),
        10 => (6, true),
        11 => (7, false),
        12 => (8, false),
        13 => (8, true),
        14 => (9, false),
        15 => (10, false),
        16 => (11, false),
        17 => (12, false),
        _ => (13, false),
    }
}

/// Final consonant 1–27 → one or two initial-consonant indices (clusters like ㄳ are two).
fn final_parts(t: u32) -> (u8, Option<u8>) {
    match t {
        1 => (0, None),
        2 => (1, None),
        3 => (0, Some(9)),
        4 => (2, None),
        5 => (2, Some(12)),
        6 => (2, Some(18)),
        7 => (3, None),
        8 => (5, None),
        9 => (5, Some(0)),
        10 => (5, Some(6)),
        11 => (5, Some(7)),
        12 => (5, Some(9)),
        13 => (5, Some(16)),
        14 => (5, Some(17)),
        15 => (5, Some(18)),
        16 => (6, None),
        17 => (7, None),
        18 => (7, Some(9)),
        19 => (9, None),
        20 => (10, None),
        21 => (11, None),
        22 => (12, None),
        23 => (14, None),
        24 => (15, None),
        25 => (16, None),
        26 => (17, None),
        _ => (18, None),
    }
}

/// Draw consonant `id` into the `w`×`h` box at (`x0`, `y0`), resampled from its 5×5 shape.
fn stamp(cv: &mut Canvas, id: u8, x0: usize, y0: usize, w: usize, h: usize) {
    let (shape, double) = cons_shape(id);
    if double && w >= 2 {
        let w1 = w.div_ceil(2);
        stamp_one(cv, shape, x0, y0, w1, h);
        stamp_one(cv, shape, x0 + w1, y0, w - w1, h);
    } else {
        stamp_one(cv, shape, x0, y0, w, h);
    }
}

fn stamp_one(cv: &mut Canvas, shape: usize, x0: usize, y0: usize, w: usize, h: usize) {
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = ((2 * x + 1) * 5 / (2 * w), (2 * y + 1) * 5 / (2 * h));
            if CONS[shape][sy] & (1 << (4 - sx)) != 0 && x0 + x < 8 && y0 + y < 8 {
                cv[y0 + y][x0 + x] = true;
            }
        }
    }
}

/// Vertical vowel (ㅏ ㅐ ㅑ ㅒ ㅓ ㅔ ㅕ ㅖ ㅣ = 0–7, 20) in the `w`×`h` box at (`x0`, `y0`).
fn vowel_vertical<const N: usize>(cv: &mut [[bool; N]; N], v: u32, x0: usize, w: usize, y0: usize, h: usize) {
    let right = x0 + w - 1;
    let mid = y0 + (h - 1) / 2;
    let ticks: Vec<usize> = match v {
        0 | 1 | 4 | 5 => vec![mid],
        2 | 3 | 6 | 7 => vec![y0 + h / 4, y0 + h - 1 - h / 4],
        _ => vec![],
    };
    let stems: Vec<usize> = match v {
        0 | 2 => vec![x0],
        1 | 3 => vec![x0, right],
        4 | 6 => vec![right],
        5 | 7 => vec![right - 1, right],
        _ => vec![x0 + (w - 1) / 2],
    };
    for &x in &stems {
        for row in cv.iter_mut().skip(y0).take(h) {
            row[x] = true;
        }
    }
    for &y in &ticks {
        let span = match v {
            0 | 2 => x0 + 1..=right,
            1 | 3 => x0 + 1..=right - 1,
            4 | 6 => x0..=right - 1,
            _ => x0..=right - 2,
        };
        for x in span {
            cv[y][x] = true;
        }
    }
}

/// Horizontal vowel (ㅗ ㅛ ㅜ ㅠ ㅡ = 8, 12, 13, 17, 18) over columns `xa..=xb`, rows `y0..y0 + h`.
fn vowel_horizontal<const N: usize>(cv: &mut [[bool; N]; N], v: u32, (xa, xb): (usize, usize), y0: usize, h: usize) {
    let rise = h.saturating_sub(1).min(2);
    let bar = match v {
        8 | 12 => y0 + h - 1,
        13 | 17 => y0,
        _ => y0 + h / 2,
    };
    cv[bar][xa..=xb].fill(true);
    let nubs: Vec<usize> = match v {
        8 | 13 => vec![(xa + xb) / 2],
        12 | 17 => vec![xa + 1, xb - 1],
        _ => vec![],
    };
    for x in nubs {
        for k in 1..=rise {
            let y = if matches!(v, 8 | 12) { bar - k } else { bar + k };
            cv[y][x] = true;
        }
    }
}

/// Compose a syllable block (U+AC00–U+D7A3) in an 8×8 pixel cell.
fn hangul(c: char) -> Option<Glyph> {
    let idx = (c as u32).checked_sub(0xAC00).filter(|i| *i < 11172)?;
    let (l, v, t) = ((idx / 588) as u8, (idx % 588) / 28, idx % 28);
    let mut cv: Canvas = [[false; 8]; 8];
    // with a final consonant the body is squeezed into the top 5 rows
    let body = if t > 0 { 5 } else { 8 };
    match v {
        0..=7 | 20 => {
            stamp(&mut cv, l, 0, 0, 4, body);
            vowel_vertical(&mut cv, v, 5, 3, 0, body);
        }
        8 | 12 | 13 | 17 | 18 => {
            let ih = if t > 0 { 3 } else { 5 };
            stamp(&mut cv, l, 1, 0, 6, ih);
            vowel_horizontal(&mut cv, v, (1, 6), ih, body - ih);
        }
        _ => {
            // compound: horizontal part under the initial, vertical part on the right
            let hh = if t > 0 { 2 } else { 3 };
            stamp(&mut cv, l, 0, 0, 4, body - hh);
            let (hor, ver) = match v {
                9 => (8, 0),
                10 => (8, 1),
                11 => (8, 20),
                14 => (13, 4),
                15 => (13, 5),
                16 => (13, 20),
                _ => (18, 20),
            };
            vowel_horizontal(&mut cv, hor, (0, 3), body - hh, hh);
            vowel_vertical(&mut cv, ver, 5, 3, 0, body);
        }
    }
    if t > 0 {
        match final_parts(t) {
            (a, None) => stamp(&mut cv, a, 1, 5, 6, 3),
            (a, Some(b)) => {
                stamp(&mut cv, a, 1, 5, 3, 3);
                stamp(&mut cv, b, 4, 5, 3, 3);
            }
        }
    }
    let mut rows = [0u16; 12];
    for (r, line) in rows.iter_mut().zip(cv) {
        for (x, on) in line.iter().enumerate() {
            if *on {
                *r |= 1 << (7 - x);
            }
        }
    }
    Some(Glyph { w: 8, rows })
}

// ── Hangul, fine: 12×12, every jamo drawn from strokes ──────────────────
type Fine = [[bool; 12]; 12];

fn px(cv: &mut Fine, x: usize, y: usize) {
    if x < 12 && y < 12 {
        cv[y][x] = true;
    }
}

fn hline(cv: &mut Fine, y: usize, xa: usize, xb: usize) {
    for x in xa..=xb {
        px(cv, x, y);
    }
}

fn vline(cv: &mut Fine, x: usize, ya: usize, yb: usize) {
    for y in ya..=yb {
        px(cv, x, y);
    }
}

/// A straight stroke between two points (Bresenham).
fn stroke(cv: &mut Fine, (xa, ya): (usize, usize), (xb, yb): (usize, usize)) {
    let (mut x, mut y) = (xa as i32, ya as i32);
    let (dx, dy) = ((xb as i32 - x).abs(), -(yb as i32 - y).abs());
    let (sx, sy) = (if x < xb as i32 { 1 } else { -1 }, if y < yb as i32 { 1 } else { -1 });
    let mut err = dx + dy;
    loop {
        px(cv, x as usize, y as usize);
        if (x, y) == (xb as i32, yb as i32) {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// ㅅ: two legs from the apex row `y0` to the bottom `y1`, across columns `x0..=x1`.
fn legs(cv: &mut Fine, x0: usize, x1: usize, y0: usize, y1: usize) {
    let (xl, xr) = (x0 + (x1 - x0) / 2, x0 + (x1 - x0).div_ceil(2));
    stroke(cv, (xl, y0), (x0, y1));
    stroke(cv, (xr, y0), (x1, y1));
}

/// ㅇ: a ring with rounded corners in the box.
fn ring(cv: &mut Fine, x0: usize, y0: usize, w: usize, h: usize) {
    let (x1, y1) = (x0 + w - 1, y0 + h - 1);
    if w < 3 || h < 3 {
        hline(cv, y0, x0, x1);
        hline(cv, y1, x0, x1);
        return;
    }
    hline(cv, y0, x0 + 1, x1 - 1);
    hline(cv, y1, x0 + 1, x1 - 1);
    vline(cv, x0, y0 + 1, y1 - 1);
    vline(cv, x1, y0 + 1, y1 - 1);
}

/// One initial consonant (`shape`: the [`CONS`] index) in the `w`×`h` box at (`x0`, `y0`).
fn fine_shape(cv: &mut Fine, shape: usize, x0: usize, y0: usize, w: usize, h: usize) {
    if w == 0 || h == 0 {
        return;
    }
    let (x1, y1) = (x0 + w - 1, y0 + h - 1);
    let ym = y0 + (h - 1) / 2;
    let xm = x0 + (w - 1) / 2;
    match shape {
        0 => {
            // ㄱ
            hline(cv, y0, x0, x1);
            vline(cv, x1, y0, y1);
        }
        1 => {
            // ㄴ
            vline(cv, x0, y0, y1);
            hline(cv, y1, x0, x1);
        }
        2 => {
            // ㄷ
            hline(cv, y0, x0, x1);
            vline(cv, x0, y0, y1);
            hline(cv, y1, x0, x1);
        }
        3 => {
            // ㄹ
            hline(cv, y0, x0, x1);
            vline(cv, x1, y0, ym);
            hline(cv, ym, x0, x1);
            vline(cv, x0, ym, y1);
            hline(cv, y1, x0, x1);
        }
        4 => {
            // ㅁ
            hline(cv, y0, x0, x1);
            hline(cv, y1, x0, x1);
            vline(cv, x0, y0, y1);
            vline(cv, x1, y0, y1);
        }
        5 => {
            // ㅂ
            vline(cv, x0, y0, y1);
            vline(cv, x1, y0, y1);
            hline(cv, ym, x0, x1);
            hline(cv, y1, x0, x1);
        }
        6 => legs(cv, x0, x1, y0, y1), // ㅅ
        7 => ring(cv, x0, y0, w, h),   // ㅇ
        8 => {
            // ㅈ
            hline(cv, y0, x0, x1);
            legs(cv, x0, x1, (y0 + 1).min(y1), y1);
        }
        9 => {
            // ㅊ
            px(cv, xm, y0);
            hline(cv, (y0 + 1).min(y1), x0, x1);
            legs(cv, x0, x1, (y0 + 2).min(y1), y1);
        }
        10 => {
            // ㅋ
            hline(cv, y0, x0, x1);
            vline(cv, x1, y0, y1);
            hline(cv, ym, x0, x1);
        }
        11 => {
            // ㅌ
            hline(cv, y0, x0, x1);
            vline(cv, x0, y0, y1);
            hline(cv, ym, x0, x1);
            hline(cv, y1, x0, x1);
        }
        12 => {
            // ㅍ
            hline(cv, y0, x0, x1);
            hline(cv, y1, x0, x1);
            vline(cv, x0 + (w / 4).max(1).min(w - 1), y0, y1);
            vline(cv, x1 - (w / 4).max(1).min(w - 1), y0, y1);
        }
        _ => {
            // ㅎ
            px(cv, xm, y0);
            hline(cv, (y0 + 1).min(y1), x0, x1);
            if h > 3 {
                ring(cv, x0 + (w / 4).min(1), y0 + 2, w - 2 * (w / 4).min(1), h - 2);
            }
        }
    }
}

/// Initial consonant `id` (0–18); the doubled ones (ㄲ ㄸ ㅃ ㅆ ㅉ) are two narrow copies.
fn fine_cons(cv: &mut Fine, id: u8, x0: usize, y0: usize, w: usize, h: usize) {
    let (shape, double) = cons_shape(id);
    if double && w >= 4 {
        let w1 = w.div_ceil(2);
        fine_shape(cv, shape, x0, y0, w1 - 1, h);
        fine_shape(cv, shape, x0 + w1, y0, w - w1, h);
    } else {
        fine_shape(cv, shape, x0, y0, w, h);
    }
}

/// Compose a syllable block on a 12×12 grid. The body (initial + vowel) takes the top 7
/// rows when there is a final consonant, which gets the bottom 4.
fn hangul_fine(c: char) -> Option<Glyph> {
    let idx = (c as u32).checked_sub(0xAC00).filter(|i| *i < 11172)?;
    let (l, v, t) = ((idx / 588) as u8, (idx % 588) / 28, idx % 28);
    let mut cv: Fine = [[false; 12]; 12];
    let body = if t > 0 { 7 } else { 12 };
    match v {
        0..=7 | 20 => {
            fine_cons(&mut cv, l, 0, 0, 7, body);
            vowel_vertical(&mut cv, v, 8, 4, 0, body);
        }
        8 | 12 | 13 | 17 | 18 => {
            // initial on top, the horizontal vowel under it (a blank row between)
            let (ih, vy, vh) = if t > 0 { (4, 5, 2) } else { (7, 8, 4) };
            fine_cons(&mut cv, l, 2, 0, 8, ih);
            vowel_horizontal(&mut cv, v, (1, 10), vy, vh);
        }
        _ => {
            // compound: the horizontal part under the initial (left), the vertical part right
            let (ih, vy, vh) = if t > 0 { (4, 5, 2) } else { (7, 8, 4) };
            fine_cons(&mut cv, l, 0, 0, 6, ih);
            let (hor, ver) = match v {
                9 => (8, 0),
                10 => (8, 1),
                11 => (8, 20),
                14 => (13, 4),
                15 => (13, 5),
                16 => (13, 20),
                _ => (18, 20),
            };
            vowel_horizontal(&mut cv, hor, (0, 5), vy, vh);
            vowel_vertical(&mut cv, ver, 8, 4, 0, body);
        }
    }
    if t > 0 {
        match final_parts(t) {
            (a, None) => fine_cons(&mut cv, a, 1, 8, 10, 4),
            (a, Some(b)) => {
                fine_cons(&mut cv, a, 0, 8, 5, 4);
                fine_cons(&mut cv, b, 6, 8, 5, 4);
            }
        }
    }
    let mut rows = [0u16; 12];
    for (r, line) in rows.iter_mut().zip(cv) {
        for (x, on) in line.iter().enumerate() {
            if *on {
                *r |= 1 << (11 - x);
            }
        }
    }
    Some(Glyph { w: 12, rows })
}

fn glyph(c: char, font: Font) -> Option<Glyph> {
    match font {
        Font::Compact => compact_glyph(c),
        Font::Normal => latin(c).or_else(|| hangul(c)),
        Font::Fine => latin_fine(c).or_else(|| hangul_fine(c)),
    }
}

/// Does the text contain Hangul syllables (so the Fine font applies)?
pub fn has_hangul(text: &str) -> bool {
    text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c))
}

/// Upper-case; `None` if the font can't draw the character (CJK ideographs, Arabic…
/// make the caller fall back to normal text).
fn fold(c: char, font: Font) -> Option<char> {
    let c = match c {
        '’' | '‘' | '`' | '´' => '\'',
        '“' | '”' => '"',
        '–' | '—' => '-',
        c if c.is_ascii() => c.to_ascii_uppercase(),
        c => {
            let mut up = c.to_uppercase();
            match (up.next(), up.next()) {
                (Some(u), None) => u, // `ß` → "SS" is not one letter: leave it undrawable
                _ => c,
            }
        }
    };
    glyph(c, font).map(|_| c)
}

/// Columns used by one glyph incl. the 1-column gap after it: Latin 6, Hangul 9 (13 in the
/// Fine font); a space is 3 columns (2 in the compact font, 4 in the Fine one).
fn advance(c: char, font: Font) -> usize {
    if c == ' ' {
        match font {
            Font::Normal => 3,
            Font::Compact => 2,
            Font::Fine => 4,
        }
    } else {
        glyph(c, font).map_or(6, |g| g.w + 1)
    }
}

/// Render `text` as rows of half-block characters, at most `max_cols` wide and
/// `max_chunks` wrapped pieces tall (each piece = 4 text rows, pieces separated by one
/// blank row). `None` when it can't be drawn — unsupported characters, a word wider than
/// the panel, or too many pieces — so the caller can fall back to normal text.
#[cfg(test)]
pub fn render(text: &str, max_cols: usize, max_chunks: usize) -> Option<Vec<String>> {
    render_scaled(text, max_cols, max_chunks, 1)
}

/// [`render`] with every pixel drawn `scale` (1–3) times as wide and tall; a piece is
/// then `4 * scale` rows tall.
#[cfg(test)]
pub fn render_scaled(text: &str, max_cols: usize, max_chunks: usize, scale: usize) -> Option<Vec<String>> {
    render_map(text, max_cols, max_chunks, scale, Font::Normal).map(|r| r.rows)
}

/// The compact 3×5 font (Latin letters and digits only): a piece is 3 rows tall.
#[cfg(test)]
pub fn render_compact(text: &str, max_cols: usize, max_chunks: usize) -> Option<Vec<String>> {
    render_map(text, max_cols, max_chunks, 1, Font::Compact).map(|r| r.rows)
}

/// One wrapped piece of a rendered line.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // read by the chord alignment that follows
pub struct Chunk {
    /// Index of its first row in [`Rendered::rows`].
    pub row: usize,
    /// Columns it is wide (every row of the piece has this many).
    pub width: usize,
    /// For each drawn source character (index into the text's `chars()`), the first column
    /// of its glyph. Whitespace has no entry.
    pub cols: Vec<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // `chunks` is read by the chord alignment that follows
pub struct Rendered {
    pub rows: Vec<String>,
    pub chunks: Vec<Chunk>,
}

impl Rendered {
    /// Where a chord attached to source character `idx` goes: (piece, column). A chord on
    /// whitespace takes the next glyph; past the end it takes the last one.
    #[allow(dead_code)]
    pub fn column_of(&self, idx: usize) -> Option<(usize, usize)> {
        let mut last = None;
        for (k, ch) in self.chunks.iter().enumerate() {
            for &(i, col) in &ch.cols {
                if i >= idx {
                    return Some((k, col));
                }
                last = Some((k, col));
            }
        }
        last
    }
}

/// A piece of text becomes words of (folded char, source index); a space has `usize::MAX`.
type Word = Vec<(char, usize)>;

/// [`render_scaled`] / [`render_compact`] in any [`Font`], also telling where each source
/// character landed (see [`Chunk::cols`]).
pub fn render_map(text: &str, max_cols: usize, max_chunks: usize, scale: usize, font: Font) -> Option<Rendered> {
    let scale = if font == Font::Compact { 1 } else { scale.clamp(1, 3) };
    let adv = |c: char| advance(c, font) * scale;
    let mut words: Vec<Word> = Vec::new();
    let mut word: Word = Vec::new();
    for (i, c) in text.chars().enumerate() {
        if c.is_whitespace() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else if c == '…' {
            word.extend(std::iter::repeat_n(('.', i), 3));
        } else {
            word.push((fold(c, font)?, i));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    if words.is_empty() {
        return None;
    }

    // Greedy wrap by rendered width.
    let mut chunks: Vec<Word> = Vec::new();
    let mut cur: Word = Vec::new();
    let mut cur_w = 0usize; // columns incl. the trailing gap
    for w in words {
        let ww: usize = w.iter().map(|&(c, _)| adv(c)).sum();
        if ww - 1 > max_cols {
            return None;
        }
        if !cur.is_empty() && cur_w + adv(' ') + ww - 1 > max_cols {
            chunks.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        if !cur.is_empty() {
            cur.push((' ', usize::MAX));
            cur_w += adv(' ');
        }
        cur_w += ww;
        cur.extend(w);
    }
    chunks.push(cur);
    if chunks.len() > max_chunks {
        return None;
    }

    // pixel rows per piece (7 + 1 blank, 5 + 1 blank, 12 incl. blank), so they pair up
    // into half-block rows
    let grid_h = font.grid_h();
    let mut rows = Vec::new();
    let mut pieces = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        if i > 0 {
            rows.push(String::new());
        }
        let mut px: Vec<Vec<bool>> = vec![Vec::new(); grid_h];
        let mut cols = Vec::new();
        for &(c, src) in chunk {
            if c == ' ' {
                for row in px.iter_mut() {
                    row.extend(std::iter::repeat_n(false, advance(' ', font)));
                }
                continue;
            }
            cols.push((src, px[0].len() * scale));
            let g = glyph(c, font).unwrap_or(Glyph { w: 5, rows: [0; 12] });
            for (y, row) in px.iter_mut().enumerate() {
                for x in 0..g.w {
                    row.push(g.rows[y] & (1 << (g.w - 1 - x)) != 0);
                }
                row.push(false);
            }
        }
        for row in px.iter_mut() {
            row.pop(); // trailing gap
        }
        // each pixel becomes a scale × scale block
        if scale > 1 {
            px = px
                .iter()
                .flat_map(|row| {
                    let wide: Vec<bool> = row.iter().flat_map(|&b| std::iter::repeat_n(b, scale)).collect();
                    std::iter::repeat_n(wide, scale)
                })
                .collect();
        }
        pieces.push(Chunk { row: rows.len(), width: px[0].len(), cols });
        for pair in 0..grid_h / 2 * scale {
            let (top, bot) = (&px[pair * 2], &px[pair * 2 + 1]);
            rows.push(
                top.iter()
                    .zip(bot)
                    .map(|(&t, &b)| match (t, b) {
                        (true, true) => '█',
                        (true, false) => '▀',
                        (false, true) => '▄',
                        (false, false) => ' ',
                    })
                    .collect(),
            );
        }
    }
    Some(Rendered { rows, chunks: pieces })
}

/// The glyph pixels of one character in a font, as text rows (for tests and tuning).
#[cfg(test)]
pub fn dump(c: char, font: Font) -> Vec<String> {
    let g = glyph(c, font).unwrap();
    (0..font.grid_h().min(12))
        .map(|y| (0..g.w).map(|x| if g.rows[y] & (1 << (g.w - 1 - x)) != 0 { '#' } else { '.' }).collect())
        .collect()
}
