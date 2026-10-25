use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardLayout {
    Russian,
    English,
}

/// Paints a non-interactive, scaled keyboard illustration.
pub fn draw_keyboard(ui: &mut Ui, layout: KeyboardLayout, active: Option<char>) {
    let width = ui.available_width().max(1.0);
    let height = (width * 0.356).min(ui.available_height()).max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let pad_x = width * 0.018;
    let pad_y = height * 0.055;
    let gap = width * 0.007;
    let unit = (width - 2.0 * pad_x - 14.0 * gap) / 15.0;
    let key_h = (height - 2.0 * pad_y - 4.0 * gap) / 5.0;
    let step_y = key_h + gap;
    let x0 = rect.left() + pad_x;
    let y0 = rect.top() + pad_y;

    let outer = rect.shrink(1.0);
    painter.rect_filled(outer, CornerRadius::same(12), Color32::from_rgb(24, 29, 36));
    painter.rect_stroke(
        outer,
        CornerRadius::same(12),
        Stroke::new(1.0, Color32::from_rgb(57, 66, 76)),
        StrokeKind::Inside,
    );

    // Soft finger zones echo the original trainer's colored guide lines.
    let zones = [
        (0.0, 2.0, Color32::from_rgb(99, 120, 137)),
        (2.0, 2.0, Color32::from_rgb(89, 133, 119)),
        (4.0, 2.0, Color32::from_rgb(133, 139, 91)),
        (6.0, 3.0, Color32::from_rgb(126, 119, 91)),
        (9.0, 2.0, Color32::from_rgb(133, 139, 91)),
        (11.0, 2.0, Color32::from_rgb(89, 133, 119)),
        (13.0, 2.0, Color32::from_rgb(99, 120, 137)),
    ];
    for (start, count, color) in zones {
        let r = Rect::from_min_max(
            Pos2::new(x0 + start * (unit + gap), y0),
            Pos2::new(
                x0 + (start + count) * unit + (start + count - 1.0) * gap,
                y0 + 4.0 * key_h + 3.0 * gap,
            ),
        );
        painter.rect_filled(
            r,
            CornerRadius::same(5),
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 18),
        );
        painter.rect_stroke(
            r,
            CornerRadius::same(5),
            Stroke::new(
                0.7,
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 85),
            ),
            StrokeKind::Inside,
        );
    }

    let rows: [&[(&str, &str, f32)]; 5] = match layout {
        KeyboardLayout::English => [
            &[
                ("~", "`", 1.),
                ("!", "1", 1.),
                ("@", "2", 1.),
                ("#", "3", 1.),
                ("$", "4", 1.),
                ("%", "5", 1.),
                ("^", "6", 1.),
                ("&", "7", 1.),
                ("*", "8", 1.),
                ("(", "9", 1.),
                (")", "0", 1.),
                ("_", "-", 1.),
                ("+", "=", 1.),
                ("Backspace", "", 2.),
            ],
            &[
                ("Tab", "", 1.5),
                ("Q", "", 1.),
                ("W", "", 1.),
                ("E", "", 1.),
                ("R", "", 1.),
                ("T", "", 1.),
                ("Y", "", 1.),
                ("U", "", 1.),
                ("I", "", 1.),
                ("O", "", 1.),
                ("P", "", 1.),
                ("{", "[", 1.),
                ("}", "]", 1.),
                ("|", "\\", 1.5),
            ],
            &[
                ("Caps", "", 1.8),
                ("A", "", 1.),
                ("S", "", 1.),
                ("D", "", 1.),
                ("F", "", 1.),
                ("G", "", 1.),
                ("H", "", 1.),
                ("J", "", 1.),
                ("K", "", 1.),
                ("L", "", 1.),
                (":", ";", 1.),
                ("\"", "'", 1.),
                ("Enter", "", 2.2),
            ],
            &[
                ("Shift", "", 2.2),
                ("Z", "", 1.),
                ("X", "", 1.),
                ("C", "", 1.),
                ("V", "", 1.),
                ("B", "", 1.),
                ("N", "", 1.),
                ("M", "", 1.),
                ("<", ",", 1.),
                (">", ".", 1.),
                ("?", "/", 1.),
                ("Shift", "", 2.8),
            ],
            &[
                ("Ctrl", "", 1.3),
                ("Win", "", 1.2),
                ("Alt", "", 1.2),
                ("", " ", 6.0),
                ("Alt", "", 1.2),
                ("Win", "", 1.2),
                ("Menu", "", 1.2),
                ("Ctrl", "", 1.3),
            ],
        ],
        KeyboardLayout::Russian => [
            &[
                ("Ё", "`", 1.),
                ("!", "1", 1.),
                ("\"", "2", 1.),
                ("№", "3", 1.),
                (";", "4", 1.),
                ("%", "5", 1.),
                (":", "6", 1.),
                ("?", "7", 1.),
                ("*", "8", 1.),
                ("(", "9", 1.),
                (")", "0", 1.),
                ("_", "-", 1.),
                ("+", "=", 1.),
                ("Backspace", "", 2.),
            ],
            &[
                ("Tab", "", 1.5),
                ("Й", "", 1.),
                ("Ц", "", 1.),
                ("У", "", 1.),
                ("К", "", 1.),
                ("Е", "", 1.),
                ("Н", "", 1.),
                ("Г", "", 1.),
                ("Ш", "", 1.),
                ("Щ", "", 1.),
                ("З", "", 1.),
                ("Х", "", 1.),
                ("Ъ", "", 1.),
                ("\\", "/", 1.5),
            ],
            &[
                ("Caps", "", 1.8),
                ("Ф", "", 1.),
                ("Ы", "", 1.),
                ("В", "", 1.),
                ("А", "", 1.),
                ("П", "", 1.),
                ("Р", "", 1.),
                ("О", "", 1.),
                ("Л", "", 1.),
                ("Д", "", 1.),
                ("Ж", "", 1.),
                ("Э", "", 1.),
                ("Enter", "", 2.2),
            ],
            &[
                ("Shift", "", 2.2),
                ("Я", "", 1.),
                ("Ч", "", 1.),
                ("С", "", 1.),
                ("М", "", 1.),
                ("И", "", 1.),
                ("Т", "", 1.),
                ("Ь", "", 1.),
                ("Б", "", 1.),
                ("Ю", "", 1.),
                ("?", ",", 1.),
                ("Shift", "", 2.8),
            ],
            &[
                ("Ctrl", "", 1.3),
                ("Win", "", 1.2),
                ("Alt", "", 1.2),
                ("", " ", 6.0),
                ("Alt", "", 1.2),
                ("Win", "", 1.2),
                ("Menu", "", 1.2),
                ("Ctrl", "", 1.3),
            ],
        ],
    };

    for (row_index, row) in rows.iter().enumerate() {
        let total_units: f32 = row.iter().map(|(_, _, w)| *w).sum();
        let row_gaps = gap * (row.len().saturating_sub(1) as f32);
        let offset_units = match row_index {
            1 => 0.15,
            2 => 0.35,
            3 => 0.55,
            _ => 0.0,
        };
        let row_unit = (width - 2.0 * pad_x - row_gaps) / (total_units + offset_units);
        let mut x = x0 + offset_units * row_unit;
        for &(top, bottom, key_units) in *row {
            let w = row_unit * key_units;
            let key_rect = Rect::from_min_size(
                Pos2::new(x, y0 + row_index as f32 * step_y),
                Vec2::new(w, key_h),
            );
            let is_active = active.is_some_and(|ch| {
                let target = ch.to_lowercase().next().unwrap_or(ch);
                top.chars()
                    .chain(bottom.chars())
                    .any(|c| c.to_lowercase().next() == Some(target))
                    || (ch == ' ' && bottom == " ")
            });
            let face = if is_active {
                Color32::from_rgb(205, 112, 43)
            } else {
                Color32::from_rgb(64, 72, 81)
            };
            let border = if is_active {
                Color32::from_rgb(255, 173, 92)
            } else {
                Color32::from_rgb(111, 124, 136)
            };
            painter.rect_filled(
                key_rect.translate(Vec2::new(0.0, 1.5)),
                CornerRadius::same(5),
                Color32::from_rgb(15, 19, 24),
            );
            painter.rect_filled(key_rect, CornerRadius::same(5), face);
            painter.rect_stroke(
                key_rect,
                CornerRadius::same(5),
                Stroke::new(if is_active { 1.8 } else { 1.0 }, border),
                StrokeKind::Inside,
            );
            let main_font = FontId::proportional((key_h * 0.45).clamp(10.0, 28.0));
            if !top.is_empty() && !bottom.is_empty() && top.chars().count() == 1 {
                painter.text(
                    Pos2::new(key_rect.center().x, key_rect.top() + key_h * 0.30),
                    Align2::CENTER_CENTER,
                    top,
                    FontId::proportional((key_h * 0.24).clamp(8.0, 15.0)),
                    Color32::from_rgb(190, 198, 205),
                );
                painter.text(
                    Pos2::new(key_rect.center().x, key_rect.top() + key_h * 0.70),
                    Align2::CENTER_CENTER,
                    bottom,
                    main_font,
                    Color32::from_rgb(236, 239, 241),
                );
            } else {
                let label = if top.is_empty() { "Space" } else { top };
                painter.text(
                    key_rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(
                        (key_h * if key_units > 1.5 { 0.31 } else { 0.36 }).clamp(8.0, 21.0),
                    ),
                    Color32::from_rgb(236, 239, 241),
                );
            }
            x += w + gap;
        }
    }
}
