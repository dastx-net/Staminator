use std::collections::BTreeMap;

use egui::epaint::PathShape;
use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Shape, Stroke, Ui, Vec2,
};
use staminator_keyboard_schema::{Canvas, Finger, Geometry, Key, KeyboardSchema};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardLayout {
    Russian,
    English,
}

/// Paints schema geometry; physical ids select the Russian or English legend.
pub fn draw_keyboard(
    ui: &mut Ui,
    schema: &KeyboardSchema,
    layout: KeyboardLayout,
    assignments: &BTreeMap<String, Finger>,
    active: Option<char>,
) {
    if let Some(background) = schema
        .style
        .as_ref()
        .and_then(|style| style.background_color.as_deref())
        .and_then(parse_color)
    {
        ui.painter()
            .rect_filled(ui.available_rect_before_wrap(), 0, background);
    }
    let width = ui.available_width().max(1.0);
    let height = ui.available_height().max(1.0);
    let scale = (width / schema.canvas.width as f32).min(height / schema.canvas.height as f32);
    let size = Vec2::new(
        schema.canvas.width as f32 * scale,
        schema.canvas.height as f32 * scale,
    );
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let style = schema.style.as_ref();
    let style_color = |get: fn(&staminator_keyboard_schema::KeyboardStyle) -> &Option<String>,
                       fallback: Color32| {
        style
            .and_then(|s| get(s).as_deref())
            .and_then(parse_color)
            .unwrap_or(fallback)
    };
    let background = style_color(|s| &s.background_color, Color32::from_rgb(24, 29, 36));
    let text_color = style_color(|s| &s.text_color, Color32::WHITE);
    let active_color = style_color(|s| &s.active_color, Color32::from_rgb(205, 112, 43));
    let active_border = style_color(|s| &s.active_border_color, Color32::from_rgb(255, 173, 92));
    let shadow = style
        .and_then(|s| s.shadow_color.as_deref())
        .and_then(parse_color);
    let font_family =
        if style.is_some_and(|s| s.font == Some(staminator_keyboard_schema::KeyboardFont::Soft)) {
            FontFamily::Name(crate::training_view::KEYBOARD_SOFT_FONT.into())
        } else {
            FontFamily::Proportional
        };
    painter.rect_filled(rect, 10, background);
    let target = active.and_then(|c| matched_key(schema, layout, c));
    let shifted = target.filter(|k| active.is_some_and(|c| shifted_char(k, layout, c)));
    let shift_code = shifted
        .and_then(|k| assignments.get(&k.physical_id))
        .map(|f| opposite_hand_shift(*f));
    for key in &schema.keys {
        let pts = points(&key.geometry, schema.canvas, rect);
        if pts.len() < 3 {
            continue;
        }
        let is_active = target.is_some_and(|k| k.physical_id == key.physical_id)
            || shift_code == Some(key.physical_id.as_str());
        let finger = assignments.get(&key.physical_id).copied();
        let fill = if is_active {
            active_color
        } else {
            key.color
                .as_deref()
                .and_then(parse_color)
                .or_else(|| {
                    schema
                        .show_finger_colors
                        .then(|| finger.map(finger_color))
                        .flatten()
                })
                .unwrap_or(Color32::from_rgb(76, 84, 94))
        };
        match &key.geometry {
            Geometry::Rect { .. } => {
                let key_rect = Rect::from_min_max(pts[0], pts[2]);
                let radius = style.and_then(|s| s.key_corner_radius).unwrap_or(0.0);
                let rounding = CornerRadius::same(
                    (key_rect.width().min(key_rect.height()) * radius)
                        .round()
                        .clamp(0.0, 255.0) as u8,
                );
                if let Some(shadow) = shadow {
                    painter.rect_filled(
                        key_rect
                            .translate(Vec2::new(0.0, (key_rect.height() * 0.045).clamp(1.0, 3.0))),
                        rounding,
                        shadow,
                    );
                }
                painter.rect_filled(key_rect, rounding, fill);
                if radius > 0.0 {
                    painter.rect_stroke(
                        key_rect.shrink(1.0),
                        rounding,
                        Stroke::new(1.0, Color32::from_white_alpha(45)),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            Geometry::Polygon { .. } => {
                painter.add(polygon_fill_mesh(&pts, fill));
            }
        }
        let border = Stroke::new(
            if is_active { 2.0 } else { 1.0 },
            if is_active {
                active_border
            } else {
                shadow.unwrap_or(Color32::from_rgb(111, 124, 136))
            },
        );
        match &key.geometry {
            Geometry::Rect { .. } => {
                let key_rect = Rect::from_min_max(pts[0], pts[2]);
                let radius = style.and_then(|s| s.key_corner_radius).unwrap_or(0.0);
                let rounding = CornerRadius::same(
                    (key_rect.width().min(key_rect.height()) * radius)
                        .round()
                        .clamp(0.0, 255.0) as u8,
                );
                painter.rect_stroke(key_rect, rounding, border, egui::StrokeKind::Inside);
            }
            Geometry::Polygon { .. } => {
                painter.add(Shape::Path(PathShape {
                    points: pts.clone(),
                    closed: true,
                    fill: Color32::TRANSPARENT,
                    stroke: border.into(),
                }));
            }
        };
        if schema.show_finger_colors
            && !is_active
            && let Some(finger) = finger
        {
            painter.add(Shape::Path(PathShape {
                points: pts.clone(),
                closed: true,
                fill: Color32::TRANSPARENT,
                stroke: Stroke::new(2.0, finger_color(finger)).into(),
            }));
        }
        let [base, shifted] = labels(key, layout);
        let base = base.filter(|s| !s.is_empty());
        let shifted = shifted.filter(|s| !s.is_empty());
        match (base, shifted) {
            (Some(base), Some(shifted)) if base != shifted && !case_pair(base, shifted) => {
                let font = (rect.height() * 0.10).clamp(8.0, 24.0);
                painter.text(
                    Pos2::new(
                        center(&pts).x,
                        pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min)
                            + rect.height() * 0.018,
                    ),
                    Align2::CENTER_TOP,
                    shifted,
                    FontId::new(font * 0.72, font_family.clone()),
                    if style.is_some() {
                        text_color
                    } else {
                        Color32::from_rgb(206, 214, 221)
                    },
                );
                painter.text(
                    Pos2::new(center(&pts).x, center(&pts).y + font * 0.36),
                    Align2::CENTER_CENTER,
                    base,
                    FontId::new(font, font_family.clone()),
                    text_color,
                );
            }
            (Some(base), _) => {
                painter.text(
                    center(&pts),
                    Align2::CENTER_CENTER,
                    base,
                    FontId::new((rect.height() * 0.10).clamp(8.0, 24.0), font_family.clone()),
                    text_color,
                );
            }
            (None, Some(shifted)) => {
                painter.text(
                    center(&pts),
                    Align2::CENTER_CENTER,
                    shifted,
                    FontId::new((rect.height() * 0.10).clamp(8.0, 24.0), font_family.clone()),
                    text_color,
                );
            }
            (None, None) => {}
        }
    }
}

/// Finger indices: left little, ring, middle, index, right index, middle, ring, little, left thumb, right thumb.
pub fn active_fingers(
    schema: &KeyboardSchema,
    layout: KeyboardLayout,
    assignments: &BTreeMap<String, Finger>,
    active: Option<char>,
) -> Vec<usize> {
    let Some(ch) = active else { return vec![] };
    if ch == ' ' {
        return schema
            .keys
            .iter()
            .find(|k| k.physical_id == "Space")
            .and_then(|k| finger(k, assignments))
            .map(finger_index)
            .into_iter()
            .collect();
    }
    let Some(k) = matched_key(schema, layout, ch) else {
        return vec![];
    };
    let Some(main_finger) = finger(k, assignments) else {
        return vec![];
    };
    let mut result = vec![finger_index(main_finger)];
    if shifted_char(k, layout, ch) {
        let code = opposite_hand_shift(main_finger);
        if let Some(f) = schema
            .keys
            .iter()
            .find(|k| k.physical_id == code)
            .and_then(|k| finger(k, assignments))
        {
            result.push(finger_index(f));
        }
    }
    result
}
fn matched_key(schema: &KeyboardSchema, layout: KeyboardLayout, ch: char) -> Option<&Key> {
    let mut found = schema.keys.iter().filter(|k| {
        legend(&k.physical_id, layout)
            .into_iter()
            .flat_map(|(a, b)| [a, b])
            .any(|s| contains_char(s, ch))
    });
    let first = found.next()?;
    if found.next().is_some() {
        None
    } else {
        Some(first)
    }
}
fn finger(k: &Key, a: &BTreeMap<String, Finger>) -> Option<Finger> {
    a.get(&k.physical_id).copied()
}
fn opposite_hand_shift(f: Finger) -> &'static str {
    match f {
        Finger::LeftLittle
        | Finger::LeftRing
        | Finger::LeftMiddle
        | Finger::LeftIndex
        | Finger::LeftThumb => "ShiftRight",
        Finger::RightLittle
        | Finger::RightRing
        | Finger::RightMiddle
        | Finger::RightIndex
        | Finger::RightThumb => "ShiftLeft",
    }
}
fn labels(k: &Key, l: KeyboardLayout) -> [Option<&str>; 2] {
    let mapped = legend(&k.physical_id, l);
    let base = k.label.as_deref().or_else(|| {
        if k.physical_id == "Space" {
            special_auto(&k.physical_id)
        } else {
            mapped.map(|x| x.0).or_else(|| special_auto(&k.physical_id))
        }
    });
    let shifted = k.shifted_label.as_deref().or_else(|| {
        // An explicitly blank base is a deliberate request to hide this key's
        // legend; don't reveal the automatic shifted glyph as a fallback.
        (k.physical_id != "Space" && k.label.as_deref() != Some(""))
            .then(|| mapped.map(|x| x.1))
            .flatten()
    });
    [base, shifted]
}
fn special_auto(id: &str) -> Option<&'static str> {
    Some(match id {
        "Backspace" => "Backspace",
        "Tab" => "Tab",
        "CapsLock" => "Caps Lock",
        "Enter" | "NumpadEnter" => "Enter",
        "ShiftLeft" | "ShiftRight" => "Shift",
        "ControlLeft" | "ControlRight" => "Ctrl",
        "AltLeft" | "AltRight" => "Alt",
        "MetaLeft" | "MetaRight" => "Win",
        "ContextMenu" => "Menu",
        "Space" => "Space",
        "Escape" => "Esc",
        "Delete" => "Del",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "Home" => "Home",
        "End" => "End",
        "PageUp" => "PgUp",
        "PageDown" => "PgDn",
        "Insert" => "Ins",
        "PrintScreen" => "PrtSc",
        "NumLock" => "Num Lock",
        "NumpadAdd" => "+",
        "NumpadSubtract" => "−",
        "NumpadMultiply" => "×",
        "NumpadDivide" => "÷",
        "NumpadDecimal" => ".",
        _ => return None,
    })
}
fn case_pair(base: &str, shifted: &str) -> bool {
    let mut a = base.chars();
    let mut b = shifted.chars();
    matches!((a.next(),b.next()),(Some(x),Some(y)) if a.next().is_none() && b.next().is_none() && x.is_alphabetic() && y.is_alphabetic() && x.to_lowercase().to_string()==y.to_lowercase().to_string())
}
fn contains_char(s: &str, c: char) -> bool {
    s.chars().any(|v| {
        v == c
            || (v.is_alphabetic()
                && c.is_alphabetic()
                && v.to_lowercase().to_string() == c.to_lowercase().to_string())
    })
}
fn shifted_char(k: &Key, l: KeyboardLayout, c: char) -> bool {
    legend(&k.physical_id, l)
        .is_some_and(|(base, shift)| shift.chars().any(|v| v == c) && !base.chars().any(|v| v == c))
}
fn legend(id: &str, l: KeyboardLayout) -> Option<(&'static str, &'static str)> {
    let ru = l == KeyboardLayout::Russian;
    let v = match id {
        "KeyQ" => {
            if ru {
                ("й", "Й")
            } else {
                ("q", "Q")
            }
        }
        "KeyW" => {
            if ru {
                ("ц", "Ц")
            } else {
                ("w", "W")
            }
        }
        "KeyE" => {
            if ru {
                ("у", "У")
            } else {
                ("e", "E")
            }
        }
        "KeyR" => {
            if ru {
                ("к", "К")
            } else {
                ("r", "R")
            }
        }
        "KeyT" => {
            if ru {
                ("е", "Е")
            } else {
                ("t", "T")
            }
        }
        "KeyY" => {
            if ru {
                ("н", "Н")
            } else {
                ("y", "Y")
            }
        }
        "KeyU" => {
            if ru {
                ("г", "Г")
            } else {
                ("u", "U")
            }
        }
        "KeyI" => {
            if ru {
                ("ш", "Ш")
            } else {
                ("i", "I")
            }
        }
        "KeyO" => {
            if ru {
                ("щ", "Щ")
            } else {
                ("o", "O")
            }
        }
        "KeyP" => {
            if ru {
                ("з", "З")
            } else {
                ("p", "P")
            }
        }
        "KeyA" => {
            if ru {
                ("ф", "Ф")
            } else {
                ("a", "A")
            }
        }
        "KeyS" => {
            if ru {
                ("ы", "Ы")
            } else {
                ("s", "S")
            }
        }
        "KeyD" => {
            if ru {
                ("в", "В")
            } else {
                ("d", "D")
            }
        }
        "KeyF" => {
            if ru {
                ("а", "А")
            } else {
                ("f", "F")
            }
        }
        "KeyG" => {
            if ru {
                ("п", "П")
            } else {
                ("g", "G")
            }
        }
        "KeyH" => {
            if ru {
                ("р", "Р")
            } else {
                ("h", "H")
            }
        }
        "KeyJ" => {
            if ru {
                ("о", "О")
            } else {
                ("j", "J")
            }
        }
        "KeyK" => {
            if ru {
                ("л", "Л")
            } else {
                ("k", "K")
            }
        }
        "KeyL" => {
            if ru {
                ("д", "Д")
            } else {
                ("l", "L")
            }
        }
        "KeyZ" => {
            if ru {
                ("я", "Я")
            } else {
                ("z", "Z")
            }
        }
        "KeyX" => {
            if ru {
                ("ч", "Ч")
            } else {
                ("x", "X")
            }
        }
        "KeyC" => {
            if ru {
                ("с", "С")
            } else {
                ("c", "C")
            }
        }
        "KeyV" => {
            if ru {
                ("м", "М")
            } else {
                ("v", "V")
            }
        }
        "KeyB" => {
            if ru {
                ("и", "И")
            } else {
                ("b", "B")
            }
        }
        "KeyN" => {
            if ru {
                ("т", "Т")
            } else {
                ("n", "N")
            }
        }
        "KeyM" => {
            if ru {
                ("ь", "Ь")
            } else {
                ("m", "M")
            }
        }
        "Backquote" => {
            if ru {
                ("ё", "Ё")
            } else {
                ("`", "~")
            }
        }
        "Digit1" => ("1", "!"),
        "Digit2" => {
            if ru {
                ("2", "\"")
            } else {
                ("2", "@")
            }
        }
        "Digit3" => {
            if ru {
                ("3", "№")
            } else {
                ("3", "#")
            }
        }
        "Digit4" => {
            if ru {
                ("4", ";")
            } else {
                ("4", "$")
            }
        }
        "Digit5" => ("5", "%"),
        "Digit6" => {
            if ru {
                ("6", ":")
            } else {
                ("6", "^")
            }
        }
        "Digit7" => {
            if ru {
                ("7", "?")
            } else {
                ("7", "&")
            }
        }
        "Digit8" => ("8", "*"),
        "Digit9" => ("9", "("),
        "Digit0" => ("0", ")"),
        "Minus" => ("-", "_"),
        "Equal" => ("=", "+"),
        "BracketLeft" => {
            if ru {
                ("х", "Х")
            } else {
                ("[", "{")
            }
        }
        "BracketRight" => {
            if ru {
                ("ъ", "Ъ")
            } else {
                ("]", "}")
            }
        }
        "Backslash" => {
            if ru {
                ("\\", "/")
            } else {
                ("\\", "|")
            }
        }
        "Semicolon" => {
            if ru {
                ("ж", "Ж")
            } else {
                (";", ":")
            }
        }
        "Quote" => {
            if ru {
                ("э", "Э")
            } else {
                ("'", "\"")
            }
        }
        "Comma" => {
            if ru {
                ("б", "Б")
            } else {
                (",", "<")
            }
        }
        "Period" => {
            if ru {
                ("ю", "Ю")
            } else {
                (".", ">")
            }
        }
        "Slash" => {
            if ru {
                (".", ",")
            } else {
                ("/", "?")
            }
        }
        "Space" => (" ", " "),
        _ => return None,
    };
    Some(v)
}

/// Default ANSI physical layout with the original NAA number-row finger zones.
pub fn default_schema() -> KeyboardSchema {
    make_schema()
}
/// Exact geometry produced by the earlier default generator, for persisted-schema migration.
pub fn legacy_default_schema() -> KeyboardSchema {
    make_schema_legacy()
}
pub fn default_assignments() -> BTreeMap<String, Finger> {
    make_assignments(false)
}
/// NAA mapping uses the little fingers for digits 1/2, ring for 3, middle for 4, index for 5/6.
pub fn alternative_assignments() -> BTreeMap<String, Finger> {
    make_assignments(true)
}
fn make_assignments(naa: bool) -> BTreeMap<String, Finger> {
    make_schema()
        .keys
        .into_iter()
        .filter_map(|k| {
            k.finger.map(|f| {
                let f = if naa {
                    match k.physical_id.as_str() {
                        "Digit1" | "Digit2" => Finger::LeftLittle,
                        "Digit3" => Finger::LeftRing,
                        "Digit4" => Finger::LeftMiddle,
                        "Digit5" | "Digit6" => Finger::LeftIndex,
                        "Digit7" => Finger::RightIndex,
                        "Digit8" => Finger::RightMiddle,
                        "Digit9" => Finger::RightRing,
                        "Digit0" => Finger::RightLittle,
                        _ => f,
                    }
                } else {
                    f
                };
                (k.physical_id, f)
            })
        })
        .collect()
}
fn make_schema() -> KeyboardSchema {
    make_schema_impl(false)
}
fn make_schema_legacy() -> KeyboardSchema {
    make_schema_impl(true)
}
fn make_schema_impl(legacy: bool) -> KeyboardSchema {
    let rows: [&[&str]; 5] = [
        &[
            "Backquote",
            "Digit1",
            "Digit2",
            "Digit3",
            "Digit4",
            "Digit5",
            "Digit6",
            "Digit7",
            "Digit8",
            "Digit9",
            "Digit0",
            "Minus",
            "Equal",
            "Backspace",
        ],
        &[
            "Tab",
            "KeyQ",
            "KeyW",
            "KeyE",
            "KeyR",
            "KeyT",
            "KeyY",
            "KeyU",
            "KeyI",
            "KeyO",
            "KeyP",
            "BracketLeft",
            "BracketRight",
            "Backslash",
        ],
        &[
            "CapsLock",
            "KeyA",
            "KeyS",
            "KeyD",
            "KeyF",
            "KeyG",
            "KeyH",
            "KeyJ",
            "KeyK",
            "KeyL",
            "Semicolon",
            "Quote",
            "Enter",
        ],
        &[
            "ShiftLeft",
            "KeyZ",
            "KeyX",
            "KeyC",
            "KeyV",
            "KeyB",
            "KeyN",
            "KeyM",
            "Comma",
            "Period",
            "Slash",
            "ShiftRight",
        ],
        &[
            "ControlLeft",
            "MetaLeft",
            "AltLeft",
            "Space",
            "AltRight",
            "MetaRight",
            "ContextMenu",
            "ControlRight",
        ],
    ];
    let mut keys = Vec::new();
    let gap = 0.10;
    let canvas = Canvas {
        width: 16.0,
        height: 5.0,
    };
    for (r, row) in rows.iter().enumerate() {
        let offset = 0.0;
        let units = row.iter().map(|id| unit(id)).sum::<f64>();
        let scale = if legacy {
            canvas.width / (units + gap * (row.len() - 1) as f64)
        } else {
            (canvas.width - gap * (row.len() - 1) as f64) / units
        };
        let mut x = offset;
        for (c, id) in row.iter().enumerate() {
            let w = unit(id) * scale;
            let (label, shifted) = legend(id, KeyboardLayout::English)
                .map(|_| (None, None))
                .unwrap_or_else(|| (Some(special(id).into()), None));
            keys.push(Key {
                physical_id: (*id).into(),
                geometry: Geometry::Rect {
                    x,
                    y: r as f64 + 0.08,
                    width: if legacy {
                        (w - gap).max(0.1)
                    } else if c + 1 == row.len() {
                        canvas.width - x
                    } else {
                        w
                    },
                    height: 0.84,
                },
                label,
                shifted_label: shifted,
                color: None,
                finger: Some(assign(id, r, c)),
            });
            x += if legacy { w } else { w + gap };
        }
    }
    KeyboardSchema {
        show_finger_colors: true,
        style: None,
        version: 1,
        name: "ANSI QWERTY".into(),
        canvas,
        keys,
    }
}
fn unit(id: &str) -> f64 {
    match id {
        "Backspace" | "Enter" => 2.0,
        "Tab" | "Backslash" => 1.5,
        "CapsLock" => 1.8,
        "ShiftLeft" => 2.2,
        "ShiftRight" => 2.8,
        "Space" => 6.0,
        "ControlLeft" | "ControlRight" => 1.3,
        "MetaLeft" | "MetaRight" | "AltLeft" | "AltRight" | "ContextMenu" => 1.2,
        _ => 1.0,
    }
}
fn special(id: &str) -> &'static str {
    match id {
        "Backspace" => "Backspace",
        "Tab" => "Tab",
        "CapsLock" => "Caps",
        "Enter" => "Enter",
        "ShiftLeft" | "ShiftRight" => "Shift",
        "Space" => "Space",
        "ControlLeft" | "ControlRight" => "Ctrl",
        "MetaLeft" | "MetaRight" => "Win",
        "AltLeft" | "AltRight" => "Alt",
        "ContextMenu" => "Menu",
        _ => "",
    }
}
fn assign(id: &str, row: usize, col: usize) -> Finger {
    use Finger::*;
    match id {
        "Space" => RightThumb,
        "AltLeft" | "MetaLeft" => LeftThumb,
        "AltRight" | "MetaRight" => RightThumb,
        "ControlLeft" | "ShiftLeft" | "Tab" | "CapsLock" => LeftLittle,
        "ControlRight" | "ShiftRight" | "Backspace" | "Enter" | "Backslash" | "ContextMenu" => {
            RightLittle
        }
        _ => match row {
            0 => match col {
                0..=1 => LeftLittle,
                2 => LeftRing,
                3 => LeftMiddle,
                4..=5 => LeftIndex,
                6..=7 => RightIndex,
                8 => RightMiddle,
                9 => RightRing,
                _ => RightLittle,
            },
            _ => match col {
                0..=1 => LeftLittle,
                2 => LeftRing,
                3 => LeftMiddle,
                4..=5 => LeftIndex,
                6..=7 => RightIndex,
                8 => RightMiddle,
                9 => RightRing,
                _ => RightLittle,
            },
        },
    }
}
fn finger_index(f: Finger) -> usize {
    match f {
        Finger::LeftLittle => 0,
        Finger::LeftRing => 1,
        Finger::LeftMiddle => 2,
        Finger::LeftIndex => 3,
        Finger::RightIndex => 4,
        Finger::RightMiddle => 5,
        Finger::RightRing => 6,
        Finger::RightLittle => 7,
        Finger::LeftThumb => 8,
        Finger::RightThumb => 9,
    }
}
fn finger_color(f: Finger) -> Color32 {
    let c = [
        (190, 119, 125),
        (198, 145, 102),
        (178, 169, 103),
        (111, 164, 139),
        (102, 154, 177),
        (128, 139, 190),
        (165, 128, 184),
        (190, 119, 157),
        (137, 181, 181),
        (207, 186, 126),
    ][finger_index(f)];
    Color32::from_rgb(c.0, c.1, c.2)
}
fn parse_color(s: &str) -> Option<Color32> {
    let h = s.strip_prefix('#')?;
    if !matches!(h.len(), 6 | 8) || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    let a = if h.len() == 8 {
        u8::from_str_radix(&h[6..8], 16).ok()?
    } else {
        255
    };
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}
fn points(g: &Geometry, c: Canvas, r: Rect) -> Vec<Pos2> {
    let f = |x: f64, y: f64| {
        Pos2::new(
            r.left() + x as f32 / c.width as f32 * r.width(),
            r.top() + y as f32 / c.height as f32 * r.height(),
        )
    };
    match g {
        Geometry::Rect {
            x,
            y,
            width,
            height,
        } => vec![
            f(*x, *y),
            f(x + width, *y),
            f(x + width, y + height),
            f(*x, y + height),
        ],
        Geometry::Polygon { points } => points.iter().map(|p| f(p.x, p.y)).collect(),
    }
}
fn center(p: &[Pos2]) -> Pos2 {
    let n = p.len() as f32;
    Pos2::new(
        p.iter().map(|p| p.x).sum::<f32>() / n,
        p.iter().map(|p| p.y).sum::<f32>() / n,
    )
}
// Ear clipping handles concave editor polygons without filling their convex hull.
pub(crate) fn triangulate(points: &[Pos2]) -> Vec<Vec<Pos2>> {
    if points.len() < 3 {
        return vec![];
    }
    let mut ids: Vec<usize> = (0..points.len()).collect();
    let area: f32 = (0..points.len())
        .map(|i| {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            a.x * b.y - b.x * a.y
        })
        .sum();
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let cross = |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let mut out = Vec::new();
    let mut guard = points.len() * points.len();
    while ids.len() > 3 && guard > 0 {
        guard -= 1;
        let mut ear = None;
        for i in 0..ids.len() {
            let (a, b, c) = (
                points[ids[(i + ids.len() - 1) % ids.len()]],
                points[ids[i]],
                points[ids[(i + 1) % ids.len()]],
            );
            if cross(a, b, c) * sign <= 0.0001 {
                continue;
            }
            let inside = ids
                .iter()
                .copied()
                .filter(|id| {
                    *id != ids[(i + ids.len() - 1) % ids.len()]
                        && *id != ids[i]
                        && *id != ids[(i + 1) % ids.len()]
                })
                .any(|id| {
                    let p = points[id];
                    cross(a, b, p) * sign >= 0.0
                        && cross(b, c, p) * sign >= 0.0
                        && cross(c, a, p) * sign >= 0.0
                });
            if !inside {
                ear = Some((i, vec![a, b, c]));
                break;
            }
        }
        if let Some((i, t)) = ear {
            out.push(t);
            ids.remove(i);
        } else {
            return vec![];
        }
    }
    if ids.len() == 3 {
        out.push(ids.iter().map(|i| points[*i]).collect());
    }
    out
}

/// Build a single un-antialiased mesh for a possibly concave polygon fill.
/// Keeping all ear-clipped triangles in one mesh prevents visible seams.
pub fn polygon_fill_mesh(points: &[Pos2], color: Color32) -> Shape {
    let mut mesh = egui::epaint::Mesh::default();
    for triangle in triangulate(points) {
        let first = mesh.vertices.len() as u32;
        mesh.vertices
            .extend(triangle.into_iter().map(|pos| egui::epaint::Vertex {
                pos,
                uv: egui::epaint::WHITE_UV,
                color,
            }));
        mesh.indices.extend([first, first + 1, first + 2]);
    }
    Shape::mesh(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, RawInput};
    use staminator_keyboard_schema::Point;
    #[test]
    fn bundled_black_keyboard_has_two_straight_blocks_and_working_highlights() {
        let schema: KeyboardSchema = serde_json::from_str(include_str!(
            "../../../assets/keyboards/split-black/keyboard.json"
        ))
        .unwrap();
        schema.validate().unwrap();
        assert_eq!(schema.name, "Split Black ANSI");
        assert!(!schema.show_finger_colors);
        assert_eq!(schema.keys.len(), 75);
        for y in [0.2, 1.3, 2.4, 3.5, 4.6, 5.7] {
            let mut left = false;
            let mut right = false;
            for key in &schema.keys {
                let Geometry::Rect {
                    x, y: key_y, width, ..
                } = key.geometry
                else {
                    panic!("the keys must not be tilted")
                };
                if (key_y - y).abs() < 1e-6 {
                    if x < 9.0 {
                        assert!(x + width <= 8.0 + 1e-6);
                        left = true;
                    } else {
                        assert!(x >= 10.0);
                        right = true;
                    }
                }
            }
            assert!(left && right, "each row needs a gap between both blocks");
        }
        let standard = default_assignments();
        assert_eq!(
            active_fingers(&schema, KeyboardLayout::Russian, &standard, Some('ф')),
            vec![0]
        );
        assert_eq!(
            active_fingers(&schema, KeyboardLayout::Russian, &standard, Some('н')),
            vec![4]
        );
        assert_eq!(
            active_fingers(&schema, KeyboardLayout::English, &standard, Some('Q')),
            vec![0, 7]
        );
        assert_eq!(
            active_fingers(&schema, KeyboardLayout::English, &standard, Some(' ')),
            vec![9]
        );
        let ctx = Context::default();
        crate::training_view::install_text_font(&ctx);
        let mut output = ctx.run_ui(RawInput::default(), |ui| {
            draw_keyboard(ui, &schema, KeyboardLayout::English, &standard, Some('Q'))
        });
        let active = parse_color(
            schema
                .style
                .as_ref()
                .unwrap()
                .active_color
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            output
                .shapes
                .iter()
                .filter(
                    |shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == active)
                )
                .count(),
            2
        );
        output.textures_delta.clear();
    }

    #[test]
    fn pink_keyboard_hides_finger_zones_but_keeps_target_and_shift_highlights() {
        let schema: KeyboardSchema =
            serde_json::from_str(include_str!("../../../assets/keyboards/pink/keyboard.json"))
                .unwrap();
        schema.validate().unwrap();
        assert!(!schema.show_finger_colors);
        for y in [0.1, 1.2, 2.3, 3.4, 4.5] {
            let right_edge = schema
                .keys
                .iter()
                .filter_map(|key| match key.geometry {
                    Geometry::Rect {
                        x, y: key_y, width, ..
                    } if (key_y - y).abs() < 1e-6 => Some(x + width),
                    _ => None,
                })
                .fold(0.0_f64, f64::max);
            assert!(
                (right_edge - 16.4).abs() < 1e-6,
                "each row must share its right edge"
            );
        }
        let style = schema.style.as_ref().unwrap();
        let active_color = parse_color(style.active_color.as_deref().unwrap()).unwrap();
        let ctx = Context::default();
        crate::training_view::install_text_font(&ctx);
        let paint = |assignments: &BTreeMap<String, Finger>, active| {
            let mut out = ctx.run_ui(RawInput::default(), |ui| {
                draw_keyboard(ui, &schema, KeyboardLayout::Russian, assignments, active);
            });
            let fills: Vec<Color32> = out
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill != Color32::TRANSPARENT => Some(rect.fill),
                    _ => None,
                })
                .collect();
            out.textures_delta.clear();
            fills
        };
        let standard = default_assignments();
        let alternate = alternative_assignments();
        assert_eq!(
            paint(&standard, None),
            paint(&alternate, None),
            "finger schemes must not recolor keys"
        );
        assert_eq!(
            paint(&standard, None)
                .iter()
                .filter(|color| **color == active_color)
                .count(),
            0
        );
        assert_eq!(
            paint(&standard, Some('й'))
                .iter()
                .filter(|color| **color == active_color)
                .count(),
            1
        );
        assert_eq!(
            paint(&standard, Some('Й'))
                .iter()
                .filter(|color| **color == active_color)
                .count(),
            2
        );
        assert_eq!(
            active_fingers(&schema, KeyboardLayout::Russian, &standard, Some('Й')),
            vec![0, 7]
        );
        assert_eq!(
            paint(&standard, Some('🦀'))
                .iter()
                .filter(|color| **color == active_color)
                .count(),
            0
        );
    }

    #[test]
    fn custom_key_color_preserves_alpha_and_rejects_invalid_text() {
        assert_eq!(parse_color("#4488AA80").unwrap().a(), 128);
        assert_eq!(parse_color("#4488AA").unwrap().a(), 255);
        assert!(parse_color("#ééé").is_none());
        assert!(parse_color("#12345678ff").is_none());
    }

    #[test]
    fn default_layouts_validate_and_map_languages() {
        let s = default_schema();
        s.validate().unwrap();
        let a = default_assignments();
        assert_eq!(
            active_fingers(&s, KeyboardLayout::English, &a, Some('q')),
            [0]
        );
        assert_eq!(
            active_fingers(&s, KeyboardLayout::Russian, &a, Some('й')),
            [0]
        );
        assert_eq!(
            active_fingers(&s, KeyboardLayout::Russian, &a, Some('Й')),
            [0, 7]
        );
        assert_eq!(
            active_fingers(&s, KeyboardLayout::English, &a, Some('!')),
            [0, 7]
        );
        assert_eq!(
            active_fingers(&s, KeyboardLayout::English, &a, Some('1')),
            [0]
        );
        assert!(active_fingers(&s, KeyboardLayout::English, &a, Some('🦀')).is_empty());
        let alt = alternative_assignments();
        assert_ne!(alt, a);
        assert_eq!(alt["Digit3"], Finger::LeftRing);
        assert_eq!(alt["Digit0"], Finger::RightLittle);
    }
    #[test]
    fn default_geometry_aligns_rows_and_legacy_geometry_is_preserved() {
        let current = default_schema();
        current.validate().unwrap();
        let encoded = serde_json::to_string(&current).unwrap();
        let decoded: KeyboardSchema = serde_json::from_str(&encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, current);
        for (before, after) in current.keys.iter().zip(decoded.keys.iter()) {
            match (&before.geometry, &after.geometry) {
                (
                    Geometry::Rect {
                        x: x0,
                        y: y0,
                        width: w0,
                        height: h0,
                    },
                    Geometry::Rect {
                        x: x1,
                        y: y1,
                        width: w1,
                        height: h1,
                    },
                ) => assert!(
                    (x0 - x1).abs() < 1e-12
                        && (y0 - y1).abs() < 1e-12
                        && (w0 - w1).abs() < 1e-12
                        && (h0 - h1).abs() < 1e-12
                ),
                _ => panic!("default geometry must round trip as rectangles"),
            }
        }
        let legacy = legacy_default_schema();
        let mut right_edges = [0.0; 5];
        for (row_index, edge) in right_edges.iter_mut().enumerate() {
            let row = current
                .keys
                .iter()
                .filter_map(|key| match &key.geometry {
                    Geometry::Rect { x, y, width, .. } if y.floor() as usize == row_index => {
                        Some((*x, *width))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(!row.is_empty());
            assert!((row.iter().map(|(x, w)| x + w).fold(0.0, f64::max) - 16.0).abs() < 1e-9);
            *edge = row.iter().map(|(x, w)| x + w).fold(0.0, f64::max);
            for pair in row.windows(2) {
                let gap = pair[1].0 - (pair[0].0 + pair[0].1);
                assert!((gap - 0.10).abs() < 1e-9);
            }
        }
        assert!(right_edges.iter().all(|edge| (*edge - 16.0).abs() < 1e-9));
        assert_ne!(current.keys[0].geometry, legacy.keys[0].geometry);
    }
    #[test]
    fn honors_overrides_and_paints_polygon_empty_labels() {
        let mut s = default_schema();
        s.keys[0].label = None;
        s.keys[0].shifted_label = None;
        s.keys.push(Key {
            physical_id: "NumpadEnter".into(),
            geometry: Geometry::Polygon {
                points: vec![
                    Point { x: 14., y: 3. },
                    Point { x: 15., y: 3. },
                    Point { x: 15., y: 4. },
                    Point { x: 14., y: 4. },
                ],
            },
            label: None,
            shifted_label: None,
            color: None,
            finger: None,
        });
        let mut a = default_assignments();
        a.insert("KeyQ".into(), Finger::RightLittle);
        assert_eq!(
            active_fingers(&s, KeyboardLayout::English, &a, Some('Q')),
            [7, 0]
        );
        a.remove("KeyQ");
        assert!(active_fingers(&s, KeyboardLayout::English, &a, Some('Q')).is_empty());
        let ctx = Context::default();
        let mut out = ctx.run_ui(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                draw_keyboard(ui, &s, KeyboardLayout::English, &a, None)
            });
        });
        assert!(!out.shapes.is_empty());
        assert!(
            out.shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Rect(_)))
        );
        assert!(
            out.shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Mesh(_)))
        );
        out.textures_delta.clear();
    }

    #[test]
    fn ambiguous_physical_character_has_no_highlight_fingers() {
        let mut schema = default_schema();
        let duplicate = schema
            .keys
            .iter()
            .find(|k| k.physical_id == "KeyQ")
            .unwrap()
            .clone();
        let mut duplicate = duplicate;
        duplicate.physical_id = "KeyQ".into();
        schema.keys.push(duplicate);
        let assignments = default_assignments();
        assert!(
            active_fingers(&schema, KeyboardLayout::English, &assignments, Some('q')).is_empty()
        );
    }

    #[test]
    fn shifted_keys_use_the_opposite_hand_for_thumb_assignments() {
        let schema = default_schema();
        let mut assignments = default_assignments();
        assignments.insert("KeyA".into(), Finger::LeftThumb);
        assert_eq!(
            active_fingers(&schema, KeyboardLayout::English, &assignments, Some('A')),
            [8, 7]
        );
        assert_eq!(opposite_hand_shift(Finger::LeftThumb), "ShiftRight");
        assignments.insert("KeyA".into(), Finger::RightThumb);
        assert_eq!(
            active_fingers(&schema, KeyboardLayout::English, &assignments, Some('A')),
            [9, 0]
        );
        assert_eq!(opposite_hand_shift(Finger::RightThumb), "ShiftLeft");
    }

    #[test]
    fn labels_render_independently_with_russian_symbols_and_special_key_fallbacks() {
        let mut schema = default_schema();
        let slash = schema
            .keys
            .iter_mut()
            .find(|k| k.physical_id == "Slash")
            .unwrap();
        slash.label = Some(String::new());
        slash.shifted_label = None;
        assert_eq!(labels(slash, KeyboardLayout::Russian), [Some(""), None]);
        slash.shifted_label = Some(",".into());
        assert_eq!(
            labels(slash, KeyboardLayout::Russian),
            [Some(""), Some(",")]
        );
        slash.shifted_label = None;
        let enter = schema
            .keys
            .iter_mut()
            .find(|k| k.physical_id == "Enter")
            .unwrap();
        enter.label = None;
        enter.shifted_label = None;
        assert_eq!(
            labels(enter, KeyboardLayout::English),
            [Some("Enter"), None]
        );
        enter.label = Some(String::new());
        assert_eq!(labels(enter, KeyboardLayout::English), [Some(""), None]);

        let key = schema
            .keys
            .iter_mut()
            .find(|k| k.physical_id == "KeyA")
            .unwrap();
        key.label = None;
        key.shifted_label = None;
        assert_eq!(labels(key, KeyboardLayout::English), [Some("a"), Some("A")]);
        key.label = Some("custom".into());
        key.shifted_label = None;
        assert_eq!(
            labels(key, KeyboardLayout::English),
            [Some("custom"), Some("A")]
        );
        key.label = None;
        key.shifted_label = Some("CUSTOM".into());
        assert_eq!(
            labels(key, KeyboardLayout::English),
            [Some("a"), Some("CUSTOM")]
        );
        key.label = Some(String::new());
        key.shifted_label = None;
        assert_eq!(labels(key, KeyboardLayout::English), [Some(""), None]);
        key.shifted_label = Some("CUSTOM".into());
        assert_eq!(
            labels(key, KeyboardLayout::English),
            [Some(""), Some("CUSTOM")]
        );

        let space = schema
            .keys
            .iter_mut()
            .find(|k| k.physical_id == "Space")
            .unwrap();
        space.label = None;
        space.shifted_label = None;
        assert_eq!(
            labels(space, KeyboardLayout::English),
            [Some("Space"), None]
        );
        let _ = space;
        assert_eq!(
            active_fingers(
                &schema,
                KeyboardLayout::English,
                &default_assignments(),
                Some(' ')
            ),
            [9]
        );
        let space = schema
            .keys
            .iter_mut()
            .find(|k| k.physical_id == "Space")
            .unwrap();
        space.label = Some(String::new());
        assert_eq!(labels(space, KeyboardLayout::English), [Some(""), None]);
        space.label = Some("Spacebar".into());
        assert_eq!(
            labels(space, KeyboardLayout::English),
            [Some("Spacebar"), None]
        );

        let ctx = Context::default();
        let mut out = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 500.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    draw_keyboard(
                        ui,
                        &schema,
                        KeyboardLayout::Russian,
                        &default_assignments(),
                        None,
                    )
                });
            },
        );
        let text = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.job.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains(':'),
            "automatic Russian shifted punctuation must be visible: {text}"
        );
        assert!(
            !text.contains("Enter"),
            "explicitly blank special label must stay hidden: {text}"
        );
        out.textures_delta.clear();
    }
}
