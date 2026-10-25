use egui::{Align2, Color32, FontId, Rect, TextureHandle, Ui, Vec2};

/// Draws the focused typing view across the full available egui panel.
pub fn draw_training(
    ui: &mut Ui,
    background: Option<&TextureHandle>,
    text: &str,
    position: usize,
    elapsed_ms: u64,
    profile_name: &str,
) {
    let rect = ui.max_rect();
    let painter = ui.painter().with_clip_rect(rect);

    if let Some(texture) = background {
        // Crop the texture to cover the viewport without distorting its aspect ratio.
        let size = texture.size_vec2();
        if size.x > 0.0 && size.y > 0.0 {
            let view_aspect = rect.width() / rect.height().max(1.0);
            let image_aspect = size.x / size.y;
            let uv = if image_aspect > view_aspect {
                let width = view_aspect / image_aspect;
                Rect::from_min_max(
                    egui::pos2((1.0 - width) * 0.5, 0.0),
                    egui::pos2((1.0 + width) * 0.5, 1.0),
                )
            } else {
                let height = image_aspect / view_aspect;
                Rect::from_min_max(
                    egui::pos2(0.0, (1.0 - height) * 0.5),
                    egui::pos2(1.0, (1.0 + height) * 0.5),
                )
            };
            painter.image(texture.id(), rect, uv, Color32::WHITE);
        } else {
            painter.rect_filled(rect, 0.0, Color32::from_rgb(67, 76, 82));
        }
    } else {
        painter.rect_filled(rect, 0.0, Color32::from_rgb(67, 76, 82));
    }

    let band_height = 100.0_f32.min(rect.height().max(1.0));
    let band = Rect::from_center_size(
        rect.center(),
        Vec2::new((rect.width() - 20.0).max(0.0), band_height),
    );
    painter.rect_filled(band, 10.0, Color32::from_rgb(232, 235, 237));
    painter.rect_stroke(
        band,
        10.0,
        egui::Stroke::new(4.0, Color32::from_rgb(153, 184, 210)),
        egui::StrokeKind::Inside,
    );

    let chars: Vec<char> = text.chars().collect();
    let font_size = (band_height * 0.55).clamp(28.0, 58.0);
    let font = FontId::monospace(font_size);
    // Measure the chosen font so the painted glyphs and fixed cursor position
    // stay aligned when the window or system font changes.
    let advance = ui
        .ctx()
        .fonts_mut(|fonts| fonts.glyph_width(&font, 'M'))
        .max(1.0);
    let margin = (band.width() * 0.045).clamp(20.0, 72.0);
    let text_left = band.left() + margin;
    let text_right = band.right() - margin;
    let text_width = (text_right - text_left).max(advance);
    let visible_count = ((text_width / advance).ceil() as usize + 2).max(1);
    let (start, end) = visible_range(chars.len(), position, visible_count);
    let current_x = rect.center().x;
    let scroll = current_x - text_left - position.saturating_sub(start) as f32 * advance;
    let text_painter = painter.with_clip_rect(band.shrink(6.0));
    let baseline = band.center().y;
    let mut x = text_left + scroll;

    for index in start..end {
        let color = if index < position {
            Color32::from_rgb(164, 169, 173)
        } else {
            Color32::BLACK
        };
        text_painter.text(
            egui::pos2(x, baseline),
            Align2::LEFT_CENTER,
            chars[index],
            font.clone(),
            color,
        );
        x += advance;
    }

    let caret_x = current_x;
    let caret_top = band.center().y - font_size * 0.48;
    let caret_bottom = band.center().y + font_size * 0.48;
    text_painter.line_segment(
        [
            egui::pos2(caret_x, caret_top),
            egui::pos2(caret_x, caret_bottom),
        ],
        (2.0, Color32::from_rgb(24, 28, 31)),
    );

    let elapsed = elapsed_ms / 1000;
    painter.text(
        egui::pos2(rect.right() - 12.0, rect.top() + 12.0),
        Align2::RIGHT_TOP,
        format!("{}:{:02}", elapsed / 60, elapsed % 60),
        FontId::monospace(22.0),
        Color32::WHITE,
    );
    painter.text(
        egui::pos2(rect.left() + 12.0, rect.bottom() - 10.0),
        Align2::LEFT_BOTTOM,
        profile_name,
        FontId::proportional(16.0),
        Color32::WHITE,
    );

    ui.allocate_rect(rect, egui::Sense::hover());
}

fn visible_range(text_len: usize, position: usize, visible_count: usize) -> (usize, usize) {
    let count = visible_count.max(1);
    let start = position.saturating_sub(count / 2).min(text_len);
    (start, start.saturating_add(count).min(text_len))
}

#[cfg(test)]
mod tests {
    use super::visible_range;

    #[test]
    fn visible_range_keeps_cursor_near_center_and_bounds_indices() {
        assert_eq!(visible_range(100, 0, 10), (0, 10));
        assert_eq!(visible_range(100, 50, 10), (45, 55));
        assert_eq!(visible_range(100, 99, 10), (94, 100));
        assert_eq!(visible_range(3, 99, 10), (3, 3));
    }
}
