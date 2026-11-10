use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2};

const COLORS: [Color32; 10] = [
    Color32::from_rgb(190, 119, 125),
    Color32::from_rgb(198, 145, 102),
    Color32::from_rgb(178, 169, 103),
    Color32::from_rgb(111, 164, 139),
    Color32::from_rgb(102, 154, 177),
    Color32::from_rgb(128, 139, 190),
    Color32::from_rgb(165, 128, 184),
    Color32::from_rgb(190, 119, 157),
    Color32::from_rgb(137, 181, 181),
    Color32::from_rgb(207, 186, 126),
];

const LABELS: [&str; 10] = [
    "keyboard.finger.left_little",
    "keyboard.finger.left_ring",
    "keyboard.finger.left_middle",
    "keyboard.finger.left_index",
    "keyboard.finger.right_index",
    "keyboard.finger.right_middle",
    "keyboard.finger.right_ring",
    "keyboard.finger.right_little",
    "keyboard.finger.left_thumb",
    "keyboard.finger.right_thumb",
];

/// Paints a scalable pair of hands with color-coded finger regions.
pub fn draw_hands(ui: &mut Ui, active: &[usize]) {
    let width = ui.available_width().max(1.0);
    let height = (width * 0.58).min(ui.available_height()).max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect.shrink(1.0),
        CornerRadius::same(12),
        Color32::from_rgb(24, 29, 36),
    );

    let unit = (width * 0.052).min(height * 0.19).clamp(4.0, 42.0);
    let palm_w = unit * 4.25;
    let palm_h = height * 0.36;
    let palm_y = rect.top() + height * 0.55;
    let center_gap = width * 0.06;
    let hand_w = palm_w + unit * 1.2;
    let left_x = rect.center().x - center_gap / 2.0 - hand_w;
    let right_x = rect.center().x + center_gap / 2.0;

    for hand in 0..2 {
        let x = if hand == 0 { left_x } else { right_x };
        let offset = if hand == 0 { 0 } else { 4 };
        let palm_x = x + unit * 0.55;
        let palm = Rect::from_min_size(Pos2::new(palm_x, palm_y), Vec2::new(palm_w, palm_h));
        let base_color = Color32::from_rgb(75, 87, 99);
        painter.rect_filled(palm, CornerRadius::same(18), base_color);
        painter.rect_stroke(
            palm,
            CornerRadius::same(18),
            Stroke::new(1.5, Color32::from_rgb(139, 151, 163)),
            StrokeKind::Inside,
        );

        // Pinky to index: separate, gently staggered fingers attached to the palm.
        let finger_w = unit * 0.72;
        let heights = if hand == 0 {
            [0.30, 0.39, 0.45, 0.38]
        } else {
            [0.38, 0.45, 0.39, 0.30]
        };
        for (j, finger_height) in heights.into_iter().enumerate() {
            let index = offset + j;
            let fx = palm_x + unit * (0.12 + j as f32 * (0.84 + 0.12));
            let fh = height * finger_height;
            let finger = Rect::from_min_size(
                Pos2::new(fx, palm_y - fh + unit * 0.34),
                Vec2::new(finger_w, fh),
            );
            let is_active = active.contains(&index);
            let color = if is_active {
                COLORS[index]
            } else {
                COLORS[index].gamma_multiply(0.52)
            };
            painter.rect_filled(finger, CornerRadius::same(10), color);
            painter.rect_stroke(
                finger,
                CornerRadius::same(10),
                Stroke::new(
                    if is_active { 2.5 } else { 1.0 },
                    if is_active {
                        Color32::WHITE
                    } else {
                        Color32::from_rgb(137, 148, 157)
                    },
                ),
                StrokeKind::Inside,
            );
        }

        // Thumb sits diagonally outside the palm; its region is distinct and highlighted too.
        let thumb_index = if hand == 0 { 8 } else { 9 };
        let thumb_x = if hand == 0 {
            palm_x + palm_w - unit * 0.2
        } else {
            palm_x - unit * 0.9
        };
        let thumb = Rect::from_min_size(
            Pos2::new(thumb_x, palm_y + unit * 0.35),
            Vec2::new(unit * 0.8, unit * 1.85),
        );
        let is_active = active.contains(&thumb_index);
        let color = if is_active {
            COLORS[thumb_index]
        } else {
            COLORS[thumb_index].gamma_multiply(0.52)
        };
        painter.rect_filled(thumb, CornerRadius::same(12), color);
        painter.rect_stroke(
            thumb,
            CornerRadius::same(12),
            Stroke::new(
                if is_active { 2.5 } else { 1.0 },
                if is_active {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(137, 148, 157)
                },
            ),
            StrokeKind::Inside,
        );
    }
    let description = active
        .iter()
        .filter_map(|&i| LABELS.get(i))
        .map(|key| crate::localization::tr(key))
        .collect::<Vec<_>>()
        .join(" + ");
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - height * 0.05),
        Align2::CENTER_CENTER,
        description,
        FontId::proportional((height * 0.045).clamp(10.0, 18.0)),
        Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn highlights_change_the_rendered_hands() {
        let ctx = egui::Context::default();
        let paint = |active: &[usize]| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(720.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ui| super::draw_hands(ui, active),
            );
            let shapes = format!("{:?}", output.shapes);
            output.textures_delta.clear();
            shapes
        };
        assert_ne!(paint(&[]), paint(&[0, 7]));
        assert_ne!(paint(&[0]), paint(&[9]));
    }
}
