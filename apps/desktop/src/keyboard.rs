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
        Color32::from_rgb(137, 181, 181), // left thumb
        Color32::from_rgb(207, 186, 126), // right thumb
    ];

    let rows = keyboard_rows(layout);

    let shift_key = active.and_then(|ch| {
        let (finger, shifted) = rows.iter().enumerate().find_map(|(r, row)| {
            row.iter().enumerate().find_map(|(k, &(top, bottom, _))| {
                key_matches_character(ch, top, bottom)
                    .then(|| (finger_for_key(r, k), requires_shift(ch, top, bottom)))
            })
        })?;
        shifted.then(|| finger.map(opposite_shift_key)).flatten()
    });
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
            let is_active = active.is_some_and(|ch| key_matches_character(ch, top, bottom))
                || shift_key == Some((row_index, key_index));
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
            if !is_active && let Some(color) = zone_color {
                let marker = Rect::from_min_max(
                    Pos2::new(key_rect.left() + 2.0, key_rect.bottom() - 4.0),
                    Pos2::new(key_rect.right() - 2.0, key_rect.bottom() - 1.0),
                );
                painter.rect_filled(marker, CornerRadius::same(1), color);
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
                let label = special_key_label(top, row_index, key_index);
                painter.text(
                    key_rect.center(),
                    Align2::CENTER_CENTER,
                    &label,
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
        ("keyboard.finger.left_little", 0),
        ("keyboard.finger.left_ring", 1),
        ("keyboard.finger.left_middle", 2),
        ("keyboard.finger.left_index", 3),
        ("keyboard.finger.right_index", 4),
        ("keyboard.finger.right_middle", 5),
        ("keyboard.finger.right_ring", 6),
        ("keyboard.finger.right_little", 7),
        ("keyboard.finger.left_thumb", 8),
        ("keyboard.finger.right_thumb", 9),
    ];
    let item_w = (width - 2.0 * pad_x) / legend.len() as f32;
    for (i, (label, color_index)) in legend.into_iter().enumerate() {
        let left = x0 + i as f32 * item_w;
        let swatch = Rect::from_min_size(Pos2::new(left, legend_y + 3.0), Vec2::new(7.0, 7.0));
        painter.rect_filled(swatch, CornerRadius::same(2), finger_colors[color_index]);
        painter.text(
            Pos2::new(left + 10.0, legend_y + 6.5),
            Align2::LEFT_CENTER,
            crate::localization::tr(label),
            FontId::proportional((height * 0.055).clamp(8.0, 11.0)),
            Color32::from_rgb(201, 208, 214),
        );
    }
}

fn keyboard_rows(layout: KeyboardLayout) -> [&'static [(&'static str, &'static str, f32)]; 5] {
    match layout {
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
                (",", ".", 1.),
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
    }
}

/// Finger assignments for the next character, including the opposite Shift.
pub fn active_fingers(layout: KeyboardLayout, active: Option<char>) -> Vec<usize> {
    let Some(ch) = active else { return Vec::new() };
    if ch == ' ' {
        return vec![8, 9];
    }
    let rows = keyboard_rows(layout);
    for (row_index, row) in rows.iter().enumerate() {
        for (key_index, &(top, bottom, _)) in row.iter().enumerate() {
            if key_matches_character(ch, top, bottom) {
                let Some(finger) = finger_for_key(row_index, key_index) else {
                    return Vec::new();
                };
                let mut fingers = vec![finger];
                if requires_shift(ch, top, bottom) {
                    let (row, key) = opposite_shift_key(finger);
                    if let Some(shift) = finger_for_key(row, key) {
                        fingers.push(shift);
                    }
                }
                return fingers;
            }
        }
    }
    Vec::new()
}

fn special_key_label(top: &str, row: usize, column: usize) -> String {
    let (scan, fallback) = match top {
        "Backspace" => (0x0e, "Backspace"),
        "Tab" => (0x0f, "Tab"),
        "Caps" => (0x3a, "Caps Lock"),
        "Enter" => (0x1c, "Enter"),
        "Shift" if row == 3 && column > 0 => (0x36, "Shift"),
        "Shift" => (0x2a, "Shift"),
        "Ctrl" if column > 3 => (0x11d, "Ctrl"),
        "Ctrl" => (0x1d, "Ctrl"),
        "Alt" if column > 3 => (0x138, "Alt"),
        "Alt" => (0x38, "Alt"),
        "Win" if column > 3 => (0x15c, "Win"),
        "Win" => (0x15b, "Win"),
        "Menu" => (0x15d, "Menu"),
        "" => (0x39, "Space"),
        _ => return top.to_owned(),
    };
    system_key_name(scan).unwrap_or_else(|| fallback.to_owned())
}

#[cfg(windows)]
fn system_key_name(scan: i32) -> Option<String> {
    let mut buffer = [0u16; 128];
    // SAFETY: the buffer is writable for the length supplied, and the scan code is a value.
    // Bit 8 of scan marks extended keys; shifting places it in Windows bit 24.
    let len = unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyNameTextW(
            scan << 16,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    (len > 0).then(|| String::from_utf16_lossy(&buffer[..len as usize]))
}

#[cfg(not(windows))]
fn system_key_name(_scan: i32) -> Option<String> {
    None
}

fn key_matches_character(ch: char, top: &str, bottom: &str) -> bool {
    if top.chars().count() > 1 || bottom.chars().count() > 1 {
        return false;
    }
    let target = ch.to_lowercase().next().unwrap_or(ch);
    top.chars()
        .chain(bottom.chars())
        .any(|c| c.to_lowercase().next() == Some(target))
        || (ch == ' ' && bottom == " ")
}

fn requires_shift(ch: char, top: &str, bottom: &str) -> bool {
    if ch == ' ' || ch == 'ё' || ch.is_ascii_lowercase() || ('а'..='я').contains(&ch) {
        return false;
    }
    if ch.is_ascii_uppercase() || ('А'..='Я').contains(&ch) {
        return true;
    }
    top.chars().any(|label| label == ch) && !bottom.chars().any(|label| label == ch)
}

fn opposite_shift_key(finger: usize) -> (usize, usize) {
    if finger < 4 { (3, 11) } else { (3, 0) }
}

/// Indexes follow physical QWERTY key positions; Russian legends use the same hardware.
fn finger_for_key(row: usize, key: usize) -> Option<usize> {
    match row {
        // Number row: `, 1-5 / 6-0, -, =. Function keys are excluded.
        0 => match key {
            0..=1 => Some(0),
            10..=13 => Some(7),
            2 => Some(1),
            9 => Some(6),
            3 => Some(2),
            8 => Some(5),
            4..=5 => Some(3),
            6..=7 => Some(4),
            _ => None,
        },
        // Tab and backslash use the left/right pinky respectively.
        1 => match key {
            0 => Some(0),
            1 => Some(0),
            11..=13 => Some(7),
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
            0 => Some(0),
            1 => Some(0),
            11..=12 => Some(7),
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
            0 => Some(0),
            11 => Some(7),
            1 => Some(0),
            10 => Some(7),
            2 => Some(1),
            3 => Some(2),
            4..=5 => Some(3),
            6..=7 => Some(4),
            8 => Some(5),
            9 => Some(6),
            _ => None,
        },
        // Ctrl/Menu use the nearby pinky, Alt/Win use the nearby thumb;
        // Space uses the right thumb (either thumb is also acceptable in practice).
        4 => match key {
            0 => Some(0),
            1..=2 => Some(8),
            3..=5 => Some(9),
            6..=7 => Some(7),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        KeyboardLayout, draw_keyboard, finger_for_key, opposite_shift_key, requires_shift,
    };
    use egui::{Context, RawInput, Rect, Vec2};

    fn paint(layout: KeyboardLayout, active: Option<char>) -> String {
        let ctx = Context::default();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(900.0, 500.0),
                )),
                ..Default::default()
            },
            |ui| draw_keyboard(ui, layout, active),
        );
        let shapes = format!("{:?}", output.shapes);
        output.textures_delta.clear();
        shapes
    }

    #[test]
    fn hands_follow_expected_character_and_opposite_shift() {
        use super::active_fingers;
        assert_eq!(active_fingers(KeyboardLayout::English, Some('q')), vec![0]);
        assert_eq!(
            active_fingers(KeyboardLayout::English, Some('Q')),
            vec![0, 7]
        );
        assert_eq!(
            active_fingers(KeyboardLayout::English, Some('P')),
            vec![7, 0]
        );
        assert_eq!(active_fingers(KeyboardLayout::Russian, Some('ф')), vec![0]);
        assert_eq!(
            active_fingers(KeyboardLayout::Russian, Some('Ф')),
            vec![0, 7]
        );
        assert_eq!(active_fingers(KeyboardLayout::Russian, Some('.')), vec![7]);
        assert_eq!(
            active_fingers(KeyboardLayout::Russian, Some(',')),
            vec![7, 0]
        );
        assert_eq!(
            active_fingers(KeyboardLayout::English, Some(' ')),
            vec![8, 9]
        );
        assert_eq!(active_fingers(KeyboardLayout::English, Some('4')), vec![3]);
        assert_eq!(active_fingers(KeyboardLayout::English, Some('6')), vec![4]);
        assert!(active_fingers(KeyboardLayout::English, Some('🦀')).is_empty());
        assert!(active_fingers(KeyboardLayout::English, None).is_empty());
    }

    #[test]
    fn headless_keyboard_paints_layout_and_active_key_changes() {
        let english = paint(KeyboardLayout::English, None);
        let russian = paint(KeyboardLayout::Russian, None);
        let active = paint(KeyboardLayout::English, Some('q'));
        assert!(english.contains("Q"));
        assert!(russian.contains("Й"));
        assert_ne!(english, russian);
        assert_ne!(english, active);
        assert_ne!(
            paint(KeyboardLayout::English, Some('A')),
            paint(KeyboardLayout::English, Some('a'))
        );
    }

    #[test]
    fn special_keys_have_names_and_character_labels_are_preserved() {
        assert_eq!(super::special_key_label("Й", 1, 1), "Й");
        assert!(!super::special_key_label("Backspace", 0, 13).is_empty());
        assert!(!super::special_key_label("", 4, 3).is_empty());
    }

    #[test]
    fn number_row_uses_the_documented_finger_zones() {
        // Physical positions are shared by the Russian and English layouts.
        let expected = [0, 0, 1, 2, 3, 3, 4, 4, 5, 6, 7, 7, 7, 7];
        for (key, finger) in expected.into_iter().enumerate() {
            assert_eq!(finger_for_key(0, key), Some(finger), "number row key {key}");
        }
        assert_eq!(finger_for_key(0, expected.len()), None);
    }

    #[test]
    fn assigns_both_shifts_and_control_keys() {
        assert_eq!(finger_for_key(3, 0), Some(0));
        assert_eq!(finger_for_key(3, 11), Some(7));
        assert_eq!(finger_for_key(0, 13), Some(7)); // Backspace
        assert_eq!(finger_for_key(1, 0), Some(0)); // Tab
        assert_eq!(finger_for_key(2, 0), Some(0)); // Caps Lock
        assert_eq!(finger_for_key(2, 12), Some(7)); // Enter
        assert_eq!(finger_for_key(4, 0), Some(0)); // left Ctrl
        assert_eq!(finger_for_key(4, 2), Some(8)); // left Alt
        assert_eq!(finger_for_key(4, 4), Some(9)); // right Alt
        assert_eq!(finger_for_key(4, 7), Some(7)); // right Ctrl
        assert_eq!(finger_for_key(4, 3), Some(9)); // Space thumb
    }

    #[test]
    fn detects_shifted_english_and_russian_characters() {
        assert!(requires_shift('A', "A", ""));
        assert!(!requires_shift('a', "A", ""));
        assert!(requires_shift('!', "!", "1"));
        assert!(!requires_shift('1', "!", "1"));
        assert!(requires_shift('Я', "Я", ""));
        assert!(!requires_shift('я', "Я", ""));
        assert!(requires_shift('Ё', "Ё", "`"));
        assert!(!requires_shift('ё', "Ё", "`"));
    }

    #[test]
    fn selects_shift_on_the_opposite_hand() {
        assert_eq!(opposite_shift_key(0), (3, 11));
        assert_eq!(opposite_shift_key(3), (3, 11));
        assert_eq!(opposite_shift_key(4), (3, 0));
        assert_eq!(opposite_shift_key(7), (3, 0));
    }
}
