use super::*;

pub(super) fn draw_cover(f: &mut Frame, app: &mut App, area: Rect) {
    let th = app.theme.clone();
    let fs = app.picker.font_size();
    let (fw, fh) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
    let ascii = app.args.ascii_cover;
    let picker = &app.picker;
    match &mut app.cover {
        CoverState::Ready { src, rendered } => {
            // Largest whole-cell box with the artwork's aspect ratio, centred.
            let (w, h) = (src.width() as f64, src.height() as f64);
            let scale =
                ((area.width as f64 * fw as f64) / w).min((area.height as f64 * fh as f64) / h);
            let cw = (((w * scale) / fw as f64).floor() as u16).clamp(1, area.width);
            let ch = (((h * scale) / fh as f64).floor() as u16).clamp(1, area.height);
            let r = Rect {
                x: area.x + (area.width - cw) / 2,
                y: area.y + (area.height - ch) / 2,
                width: cw,
                height: ch,
            };

            if rendered.as_ref().map(Rendered::cells) != Some((cw, ch)) {
                let to_ascii = |src: &image::DynamicImage| Rendered::Ascii {
                    cells: (cw, ch),
                    art: cover::ascii_art(src, cw, ch),
                };
                *rendered = Some(if ascii {
                    to_ascii(src)
                } else {
                    // Pixel-exact bitmap: scaled and centre-cropped to *precisely* cw×ch
                    // cells, so the graphics protocol has nothing to pad. (Letting it fit
                    // a slightly-off aspect padded the remainder with transparent black,
                    // which showed up as a dark sliver on the right/bottom edge.)
                    let exact = src.resize_to_fill(
                        cw as u32 * fw,
                        ch as u32 * fh,
                        FilterType::Triangle,
                    );
                    match picker.new_protocol(exact, Size::new(cw, ch), Resize::Fit(None)) {
                        Ok(p) => Rendered::Graphics {
                            cells: (cw, ch),
                            proto: Box::new(p),
                        },
                        Err(_) => to_ascii(src),
                    }
                });
            }
            match rendered {
                Some(Rendered::Graphics { proto, .. }) => {
                    f.render_widget(Image::new(proto.as_ref()), r)
                }
                Some(Rendered::Ascii { art, .. }) => draw_ascii(f.buffer_mut(), r, art),
                None => {}
            }
        }
        other => {
            let msg = match other {
                CoverState::Loading => "loading cover…",
                _ => "♪",
            };
            let y = area.y + area.height / 2;
            f.render_widget(
                Paragraph::new(Line::styled(msg, Style::new().fg(th.fade(0.5))))
                    .alignment(Alignment::Center),
                Rect {
                    y,
                    height: 1,
                    ..area
                },
            );
        }
    }
}

pub(super) fn draw_ascii(buf: &mut Buffer, r: Rect, art: &cover::AsciiArt) {
    for y in 0..art.rows.min(r.height) {
        for x in 0..art.cols.min(r.width) {
            let (ch, (cr, cg, cb)) = art.cells[y as usize * art.cols as usize + x as usize];
            if let Some(cell) = buf.cell_mut((r.x + x, r.y + y)) {
                let mut utf8 = [0u8; 4];
                cell.set_symbol(ch.encode_utf8(&mut utf8))
                    .set_fg(Color::Rgb(cr, cg, cb));
            }
        }
    }
}
