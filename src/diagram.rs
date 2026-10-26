/// Absolute frets per string (-1 = muted, 0 = open) and finger numbers (0 = none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub frets: [i8; 6],
    pub fingers: [u8; 6],
}

/// Open-position shapes that beat the barre version.
const OPEN: &[(&str, &str, &str)] = &[
    ("C", "x32010", "032010"),
    ("D", "xx0232", "000132"),
    ("G", "320003", "210004"),
    ("Dm", "xx0231", "000231"),
    ("C7", "x32310", "032410"),
    ("D7", "xx0212", "000213"),
    ("G7", "320001", "320001"),
    ("B7", "x21202", "021304"),
    ("Cmaj7", "x32000", "032000"),
    ("Dmaj7", "xx0222", "000111"),
    ("Gmaj7", "320002", "210003"),
    ("Dm7", "xx0211", "000211"),
    ("Dsus2", "xx0230", "000230"),
    ("Dsus4", "xx0233", "000123"),
    ("Gsus4", "330013", "340012"),
    ("Csus4", "x33011", "034011"),
];

/// Movable shapes: (quality, E-shape relative frets, E fingers, A-shape frets, A fingers).
#[allow(clippy::type_complexity)]
const MOVABLE: &[(&str, [i8; 6], [u8; 6], [i8; 6], [u8; 6])] = &[
    ("", [0, 2, 2, 1, 0, 0], [1, 3, 4, 2, 1, 1], [-1, 0, 2, 2, 2, 0], [0, 1, 2, 3, 4, 1]),
    ("m", [0, 2, 2, 0, 0, 0], [1, 3, 4, 1, 1, 1], [-1, 0, 2, 2, 1, 0], [0, 1, 3, 4, 2, 1]),
    ("7", [0, 2, 0, 1, 0, 0], [1, 3, 1, 2, 1, 1], [-1, 0, 2, 0, 2, 0], [0, 1, 3, 1, 4, 1]),
    ("m7", [0, 2, 0, 0, 0, 0], [1, 3, 1, 1, 1, 1], [-1, 0, 2, 0, 1, 0], [0, 1, 3, 1, 2, 1]),
    ("maj7", [0, 2, 1, 1, 0, 0], [1, 4, 2, 3, 1, 1], [-1, 0, 2, 1, 2, 0], [0, 1, 3, 2, 4, 1]),
    ("sus4", [0, 2, 2, 2, 0, 0], [1, 2, 3, 4, 1, 1], [-1, 0, 2, 2, 3, 0], [0, 1, 2, 3, 4, 1]),
    ("sus2", [-1, -1, -1, -1, -1, -1], [0; 6], [-1, 0, 2, 2, 0, 0], [0, 1, 3, 4, 1, 1]),
    ("5", [0, 2, 2, -1, -1, -1], [1, 3, 4, 0, 0, 0], [-1, 0, 2, 2, -1, -1], [0, 1, 3, 4, 0, 0]),
];

const SHARP: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// `(pitch class, quality)` for plain chords; `None` for slash chords, dim/aug/9ths…
fn split(name: &str) -> Option<(u8, &'static str)> {
    if name.contains('/') {
        return None;
    }
    let b = name.as_bytes();
    let base: i32 = match b.first()? {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let (acc, rest) = match b.get(1) {
        Some(b'#') => (1, &name[2..]),
        Some(b'b') => (-1, &name[2..]),
        _ => (0, &name[1..]),
    };
    let q = match rest {
        "" | "maj" | "M" => "",
        "m" | "min" | "-" => "m",
        "7" => "7",
        "m7" | "min7" | "-7" => "m7",
        "maj7" | "M7" | "Maj7" => "maj7",
        "sus4" | "sus" => "sus4",
        "sus2" => "sus2",
        "5" => "5",
        _ => return None,
    };
    Some(((base + acc).rem_euclid(12) as u8, q))
}

pub fn lookup(name: &str) -> Option<Shape> {
    let (pc, q) = split(name)?;
    let key = format!("{}{q}", SHARP[pc as usize]);
    if let Some((_, f, fi)) = OPEN.iter().find(|(n, _, _)| *n == key) {
        let frets = f
            .chars()
            .map(|c| if c == 'x' { -1 } else { c.to_digit(10).unwrap() as i8 })
            .collect::<Vec<_>>();
        let fingers = fi.chars().map(|c| c.to_digit(10).unwrap() as u8).collect::<Vec<_>>();
        return Some(Shape {
            frets: frets.try_into().ok()?,
            fingers: fingers.try_into().ok()?,
        });
    }
    let (_, e_f, e_fi, a_f, a_fi) = MOVABLE.iter().find(|m| m.0 == q)?;
    let f6 = (pc as i32 - 4).rem_euclid(12);
    let f5 = (pc as i32 - 9).rem_euclid(12);
    // lowest position wins; E-shape on ties (and sus2 only exists as an A-shape)
    let use_a = *e_f == [-1; 6] || f5 < f6;
    let (rel, fin, base) = if use_a { (a_f, a_fi, f5) } else { (e_f, e_fi, f6) };
    let mut frets = [-1i8; 6];
    let mut fingers = *fin;
    for i in 0..6 {
        if rel[i] >= 0 {
            frets[i] = rel[i] + base as i8;
        }
        // open-position version of a barre shape: the barre finger drops out
        if base == 0 && fingers[i] > 1 {
            fingers[i] -= 1;
        } else if base == 0 && fingers[i] == 1 && rel[i] == 0 {
            fingers[i] = 0;
        }
    }
    // a shape that would need frets above the 12th isn't a sensible diagram
    if frets.iter().any(|f| *f > 12) {
        return None;
    }
    Some(Shape { frets, fingers })
}

/// 8 lines, 11 columns wide: name, mute/open markers, nut, four fret rows.
pub fn render(name: &str, s: &Shape) -> Vec<String> {
    let fretted: Vec<i8> = s.frets.iter().copied().filter(|f| *f > 0).collect();
    let max = fretted.iter().copied().max().unwrap_or(0);
    let base = if max > 4 { fretted.iter().copied().min().unwrap_or(1) } else { 1 };
    let mut out = vec![format!("{name:^11}")];
    out.push(
        s.frets
            .iter()
            .map(|f| match f {
                -1 => 'x',
                0 => 'o',
                _ => ' ',
            })
            .map(String::from)
            .collect::<Vec<_>>()
            .join(" "),
    );
    out.push(if base == 1 { "╒═╤═╤═╤═╤═╕".into() } else { "┌─┬─┬─┬─┬─┐".into() });
    for r in 0..4 {
        let fret = base + r;
        let row = (0..6)
            .map(|i| {
                if s.frets[i] == fret {
                    match s.fingers[i] {
                        0 => "●".to_string(),
                        n => n.to_string(),
                    }
                } else {
                    "│".to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        out.push(if r == 0 && base > 1 { format!("{row} {base}fr") } else { row });
    }
    out
}

/// A little tab: one column per chord, strings e B G D A E from the top.
pub fn tab(shapes: &[Option<Shape>]) -> Vec<String> {
    const NAMES: [char; 6] = ['e', 'B', 'G', 'D', 'A', 'E'];
    (0..6)
        .map(|row| {
            let s = 5 - row;
            let mut line = format!("{}|", NAMES[row]);
            for sh in shapes {
                let cell = match sh {
                    None => "?".to_string(),
                    Some(sh) if sh.frets[s] < 0 => "x".to_string(),
                    Some(sh) => sh.frets[s].to_string(),
                };
                line.push_str(&format!("-{cell:-<2}-"));
            }
            line.push('|');
            line
        })
        .collect()
}
