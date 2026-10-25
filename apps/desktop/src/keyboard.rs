use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardLayout {
    Russian,
    English,
}

/// Paints a non-interactive, scaled keyboard illustration.
pub fn draw_keyboard(ui: &mut Ui, layout: KeyboardLayout, active: Option<char>) {
    let width = ui.available_width().max(1.0);
    let height = (width * 0.405).min(ui.available_height()).max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let pad_x = width * 0.018;
    let pad_y = height * 0.055;
    let gap = width * 0.007;
    let legend_h = (height * 0.12).clamp(12.0, 22.0);
    let key_h = (height - 2.0 * pad_y - 4.0 * gap - legend_h) / 5.0;
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

    let finger_colors = [
        Color32::from_rgb(190, 119, 125), // left pinky
        Color32::from_rgb(198, 145, 102), // left ring
        Color32::from_rgb(178, 169, 103), // left middle
        Color32::from_rgb(111, 164, 139), // left index
        Color32::from_rgb(102, 154, 177), // right index
        Color32::from_rgb(128, 139, 190), // right middle
        Color32::from_rgb(165, 128, 184), // right ring
        Color32::from_rgb(190, 119, 157), // right pinky
    ];

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
        for (key_index, &(top, bottom, key_units)) in row.iter().enumerate() {
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
            let finger = finger_for_key(row_index, key_index);
            let zone_color = finger.map(|i| finger_colors[i]);
            let face = if is_active {
                Color32::from_rgb(205, 112, 43)
            } else {
                zone_color.map_or(Color32::from_rgb(64, 72, 81), |c| {
                    Color32::from_rgb(
                        (64 + c.r() as u16 / 4) as u8,
                        (72 + c.g() as u16 / 4) as u8,
                        (81 + c.b() as u16 / 4) as u8,
                    )
                })
            };
            let border = if is_active {
                Color32::from_rgb(255, 173, 92)
            } else {
                zone_color.unwrap_or(Color32::from_rgb(111, 124, 136))
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
            if !is_active {
                if let Some(color) = zone_color {
                    let marker = Rect::from_min_max(
                        Pos2::new(key_rect.left() + 2.0, key_rect.bottom() - 4.0),
                        Pos2::new(key_rect.right() - 2.0, key_rect.bottom() - 1.0),
                    );
                    painter.rect_filled(marker, CornerRadius::same(1), color);
                }
            }
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

    let legend_y = y0 + 5.0 * key_h + 4.0 * gap + 1.0;
    let legend = [
        ("Л миз.", 0),
        ("Л без.", 1),
        ("Л сред.", 2),
        ("Л указ.", 3),
        ("П указ.", 4),
        ("П сред.", 5),
        ("П без.", 6),
        ("П миз.", 7),
    ];
    let item_w = (width - 2.0 * pad_x) / legend.len() as f32;
    for (i, (label, color_index)) in legend.into_iter().enumerate() {
        let left = x0 + i as f32 * item_w;
        let swatch = Rect::from_min_size(Pos2::new(left, legend_y + 3.0), Vec2::new(7.0, 7.0));
        painter.rect_filled(swatch, CornerRadius::same(2), finger_colors[color_index]);
        painter.text(
            Pos2::new(left + 10.0, legend_y + 6.5),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional((height * 0.055).clamp(8.0, 11.0)),
            Color32::from_rgb(201, 208, 214),
        );
    }
}

/// Indexes follow physical QWERTY key positions; Russian legends use the same hardware.
fn finger_for_key(row: usize, key: usize) -> Option<usize> {
    match row {
        // Number row: `, 1-5 / 6-0, -, =. Function keys are excluded.
        0 => match key {
            0..=1 => Some(0),
            10..=12 => Some(7),
            2 => Some(1),
            9 => Some(6),
            3 => Some(2),
            8 => Some(5),
            4..=5 => Some(3),
            6..=7 => Some(4),
            _ => None,
        },
        // Ignore Tab and the final backslash key.
        1 => match key {
            1 => Some(0),
            11 | 12 => Some(7),
            2 => Some(1),
            3 => Some(2),
            4..=5 => Some(3),
            6..=7 => Some(4),
            8 => Some(5),
            9 => Some(6),
            10 => Some(7),
            _ => None,
        },
        2 => match key {
            1 => Some(0),
            11 | 12 => Some(7),
            2 => Some(1),
            3 => Some(2),
            4..=5 => Some(3),
            6..=7 => Some(4),
            8 => Some(5),
            9 => Some(6),
            10 => Some(7),
            _ => None,
        },
        3 => match key {
            1 => Some(0),
            10 | 11 => Some(7),
            2 => Some(1),
            3 => Some(2),
            4..=5 => Some(3),
            6..=7 => Some(4),
            8 => Some(5),
            9 => Some(6),
            _ => None,
        },
        _ => None,
    }
}
