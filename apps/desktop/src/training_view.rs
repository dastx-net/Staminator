use egui::{Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Rect, Ui, Vec2};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

const TEXT_FONT: &str = "Stamina Text";

pub fn install_text_font(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let font_path = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .map(|windir| windir.join("Fonts").join("cour.ttf"));
    add_text_font(&mut fonts, font_path.as_deref());
    ctx.set_fonts(fonts);
}

fn add_text_font(fonts: &mut FontDefinitions, font_path: Option<&Path>) -> bool {
    let mut family = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    let loaded = font_path
        .and_then(|path| fs::read(path).ok())
        .is_some_and(|bytes| {
            fonts
                .font_data
                .insert("Courier New".into(), Arc::new(FontData::from_owned(bytes)));
            family.insert(0, "Courier New".into());
            true
        });
    fonts
        .families
        .insert(FontFamily::Name(TEXT_FONT.into()), family);
    loaded
}

/// Draws the focused typing view across the full available egui panel.
pub fn draw_training(
    ui: &mut Ui,
    text: &str,
    position: usize,
    elapsed_ms: u64,
    profile_name: &str,
    prompt: Option<&str>,
    background: Option<(&egui::TextureHandle, &str, u8)>,
) {
    let rect = ui.max_rect();
    let painter = ui.painter().with_clip_rect(rect);

    painter.rect_filled(rect, 0.0, Color32::from_rgb(67, 76, 82));
    if let Some((texture, mode, darkness)) = background {
        crate::background::paint(ui, rect, texture, mode, darkness);
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

    let font_size = (band_height * 0.55).clamp(28.0, 58.0);
    let font = FontId::new(font_size, FontFamily::Name(TEXT_FONT.into()));
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
    // The text can be a large imported document. Only visit the visible
    // character range instead of allocating a Vec<char> for the whole lesson
    // on every repaint (including every resize frame).
    let start = position.saturating_sub(visible_count / 2);
    let current_x = rect.center().x;
    let scroll = current_x - text_left - position.saturating_sub(start) as f32 * advance;
    let text_painter = painter.with_clip_rect(band.shrink(6.0));
    let baseline = band.center().y;
    let mut x = text_left + scroll;

    for (offset, ch) in text.chars().skip(start).take(visible_count).enumerate() {
        let index = start + offset;
        let color = if index < position {
            Color32::from_rgb(164, 169, 173)
        } else {
            Color32::BLACK
        };
        text_painter.text(
            egui::pos2(x, baseline),
            Align2::LEFT_CENTER,
            ch,
            font.clone(),
            color,
        );
        x += advance;
    }

    if !text.is_empty() {
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
    }
    if let Some(prompt) = prompt {
        text_painter.text(
            band.center(),
            Align2::CENTER_CENTER,
            prompt,
            FontId::proportional(24.0),
            Color32::from_rgb(62, 72, 80),
        );
    }

    let elapsed = elapsed_ms / 1000;
    if background.is_some() {
        let timer = format!("{}:{:02}", elapsed / 60, elapsed % 60);
        let timer_galley = painter.layout_no_wrap(timer, FontId::monospace(22.0), Color32::WHITE);
        let name_galley = painter.layout_no_wrap(
            profile_name.to_owned(),
            FontId::proportional(16.0),
            Color32::WHITE,
        );
        painter.rect_filled(
            Rect::from_min_size(
                egui::pos2(
                    rect.right() - 16.0 - timer_galley.size().x,
                    rect.top() + 8.0,
                ),
                timer_galley.size() + Vec2::splat(8.0),
            ),
            3.0,
            Color32::from_black_alpha(190),
        );
        painter.rect_filled(
            Rect::from_min_size(
                egui::pos2(
                    rect.left() + 8.0,
                    rect.bottom() - 14.0 - name_galley.size().y,
                ),
                name_galley.size() + Vec2::splat(8.0),
            ),
            3.0,
            Color32::from_black_alpha(190),
        );
    }
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

#[cfg(test)]
fn visible_range(text_len: usize, position: usize, visible_count: usize) -> (usize, usize) {
    let count = visible_count.max(1);
    let start = position.saturating_sub(count / 2).min(text_len);
    (start, start.saturating_add(count).min(text_len))
}

#[cfg(test)]
mod tests {
    use super::{TEXT_FONT, add_text_font, draw_training, visible_range};
    use egui::{Context, FontDefinitions, FontFamily, RawInput, Rect, Vec2};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    fn temp_font_path() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        std::env::temp_dir().join(format!(
            "staminator-test-font-{}-{}.ttf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn custom_font_is_loaded_from_explicit_path_and_missing_file_is_ignored() {
        let missing = temp_font_path();
        let mut fonts = FontDefinitions::default();
        assert!(!add_text_font(&mut fonts, Some(&missing)));
        assert!(
            fonts
                .families
                .contains_key(&FontFamily::Name(TEXT_FONT.into()))
        );
        assert!(!fonts.font_data.contains_key("Courier New"));

        let path = temp_font_path();
        fs::write(&path, b"isolated test font bytes").unwrap();
        let mut fonts = FontDefinitions::default();
        assert!(add_text_font(&mut fonts, Some(&path)));
        assert!(fonts.font_data.contains_key("Courier New"));
        assert_eq!(
            fonts.families[&FontFamily::Name(TEXT_FONT.into())]
                .first()
                .map(String::as_str),
            Some("Courier New")
        );
        let _ = fs::remove_file(path);
    }

    fn paint(
        text: &str,
        position: usize,
        elapsed_ms: u64,
        profile: &str,
        prompt: Option<&str>,
    ) -> String {
        let ctx = Context::default();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 400.0),
                )),
                ..Default::default()
            },
            |ui| draw_training(ui, text, position, elapsed_ms, profile, prompt, None),
        );
        assert!(matches!(
            output.shapes.first().map(|clipped| &clipped.shape),
            Some(egui::Shape::Rect(rect))
                if rect.fill == egui::Color32::from_rgb(67, 76, 82)
                    && rect.rect.size() == Vec2::new(800.0, 400.0)
        ));
        let shapes = format!("{:?}", output.shapes);
        output.textures_delta.clear();
        shapes
    }

    #[test]
    fn headless_training_paints_session_state_and_prompt() {
        let initial = paint("abcdef", 0, 0, "Default", None);
        assert!(!initial.is_empty());
        assert_ne!(initial, paint("abcdef", 1, 0, "Default", None));
        assert_ne!(initial, paint("abcdef", 0, 61_000, "Default", None));
        assert_ne!(initial, paint("abcdef", 0, 0, "Другой", None));
        assert_ne!(initial, paint("abcdef", 0, 0, "Default", Some("Пауза")));
    }

    #[test]
    fn headless_training_handles_empty_text_and_tiny_viewport() {
        let ctx = Context::default();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1.0, 1.0))),
                ..Default::default()
            },
            |ui| draw_training(ui, "", usize::MAX, 999, "", Some(""), None),
        );
        output.textures_delta.clear();
        assert!(!output.shapes.is_empty());
    }

    #[test]
    fn headless_training_paints_large_text_without_expanding_to_all_glyphs() {
        let long_text = "x".repeat(1_000_000);
        let short_text = "x".repeat(1_000);
        assert_eq!(
            paint(&long_text, 500_000, 0, "Default", None),
            paint(&short_text, 500, 0, "Default", None)
        );
    }

    #[test]
    fn headless_training_paints_background_status_panels_and_cursor() {
        let ctx = Context::default();
        let texture = ctx.load_texture(
            "training-test-background",
            egui::ColorImage::filled([2, 2], egui::Color32::from_rgb(10, 20, 30)),
            egui::TextureOptions::LINEAR,
        );
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(640.0, 320.0),
                )),
                ..Default::default()
            },
            |ui| {
                draw_training(
                    ui,
                    "typing",
                    3,
                    3_661_000,
                    "Profile",
                    Some("Paused"),
                    Some((&texture, "fill", 120)),
                )
            },
        );

        let drew_background = output
            .shapes
            .iter()
            .any(|clipped| clipped.shape.texture_id() == texture.id());
        output.textures_delta.clear();
        assert!(drew_background);
        assert!(!output.shapes.is_empty());
    }

    #[test]
    fn visible_range_keeps_cursor_near_center_and_bounds_indices() {
        assert_eq!(visible_range(100, 0, 10), (0, 10));
        assert_eq!(visible_range(100, 50, 10), (45, 55));
        assert_eq!(visible_range(100, 99, 10), (94, 100));
        assert_eq!(visible_range(3, 99, 10), (3, 3));
    }
}
