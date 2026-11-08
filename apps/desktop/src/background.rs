use egui::{Color32, Pos2, Rect, TextureHandle};
use std::{
    fs,
    io::BufReader,
    path::{Path, PathBuf},
};

const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_SIDE: u32 = 4096;
const MAX_ALLOC: u64 = 64 * 1024 * 1024;

pub fn load(ctx: &egui::Context, path: &Path) -> Result<TextureHandle, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE {
        return Err("File exceeds 32 MiB".into());
    }
    let mut reader = image::ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
    ) {
        return Err("Only PNG and JPEG are supported".into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| e.to_string())?;
    Ok(upload(
        ctx,
        "training-background",
        decoded,
        egui::TextureOptions::LINEAR,
    ))
}

pub fn load_builtin(ctx: &egui::Context) -> Result<TextureHandle, String> {
    let decoded = image::load_from_memory(include_bytes!("../assets/default-background-wide.png"))
        .map_err(|error| error.to_string())?;
    Ok(upload(
        ctx,
        "builtin-background",
        decoded,
        egui::TextureOptions::NEAREST,
    ))
}

fn upload(
    ctx: &egui::Context,
    name: &str,
    decoded: image::DynamicImage,
    options: egui::TextureOptions,
) -> TextureHandle {
    let limit = ctx
        .input(|input| input.max_texture_side)
        .clamp(1, u32::MAX as usize) as u32;
    let decoded = if decoded.width() > limit || decoded.height() > limit {
        decoded.resize(limit, limit, image::imageops::FilterType::Nearest)
    } else {
        decoded
    }
    .to_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    ctx.load_texture(name, image, options)
}

pub fn directory() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Staminator")
        .join("Backgrounds")
}

pub fn images_in(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    ["png", "jpg", "jpeg"]
                        .iter()
                        .any(|v| e.eq_ignore_ascii_case(v))
                })
        })
        .collect();
    paths.sort();
    paths
}

pub fn paint(ui: &egui::Ui, rect: Rect, texture: &TextureHandle, mode: &str, darkness: u8) {
    let painter = ui.painter().with_clip_rect(rect);
    let size = texture.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    if mode == "tile" {
        // Bound the number of draw calls for tiny images and large windows.
        let scale = (rect.width() / size.x / 128.0)
            .max(rect.height() / size.y / 128.0)
            .max(1.0);
        let tile = size * scale;
        let mut y = rect.top();
        while y < rect.bottom() {
            let mut x = rect.left();
            while x < rect.right() {
                painter.image(
                    texture.id(),
                    Rect::from_min_size(Pos2::new(x, y), tile),
                    uv,
                    Color32::WHITE,
                );
                x += tile.x;
            }
            y += tile.y;
        }
    } else {
        let scale = if mode == "fill" {
            (rect.width() / size.x).max(rect.height() / size.y)
        } else {
            (rect.width() / size.x).min(rect.height() / size.y)
        };
        painter.image(
            texture.id(),
            Rect::from_center_size(rect.center(), size * scale),
            uv,
            Color32::WHITE,
        );
    }
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(darkness));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Vec2;
    #[test]
    fn rejects_missing_corrupt_and_oversized_files() {
        let dir =
            std::env::temp_dir().join(format!("staminator-background-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let ctx = egui::Context::default();
        assert!(load(&ctx, &dir.join("missing.png")).is_err());
        let corrupt = dir.join("bad.png");
        fs::write(&corrupt, b"bad").unwrap();
        assert!(load(&ctx, &corrupt).is_err());
        let large = dir.join("large.jpg");
        fs::File::create(&large)
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap();
        assert!(load(&ctx, &large).is_err());
        assert_eq!(images_in(&dir).len(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn accepts_jpeg_and_rejects_excessive_dimensions() {
        let dir = std::env::temp_dir().join(format!(
            "staminator-background-dimensions-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let jpeg = dir.join("image.jpg");
        image::RgbImage::from_pixel(2, 3, image::Rgb([1, 2, 3]))
            .save(&jpeg)
            .unwrap();
        let ctx = egui::Context::default();
        assert!(load(&ctx, &jpeg).is_ok());
        let large = dir.join("wide.png");
        image::RgbaImage::new(MAX_SIDE + 1, 1).save(&large).unwrap();
        assert!(load(&ctx, &large).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn user_image_is_scaled_to_texture_limit_without_rejecting_valid_file() {
        let path = std::env::temp_dir().join(format!("staminator-wide-{}.png", std::process::id()));
        image::RgbaImage::new(2050, 10).save(&path).unwrap();
        let ctx = egui::Context::default();
        let texture = load(&ctx, &path).unwrap();
        assert_eq!(texture.size()[0], 2048);
        assert!(texture.size()[1] > 0);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn wide_builtin_covers_training_view_at_different_window_sizes() {
        let ctx = egui::Context::default();
        let texture = load_builtin(&ctx).unwrap();
        assert!(texture.size()[0] + 2 >= 3 * texture.size()[1]);
        assert!(texture.size()[0] <= ctx.input(|input| input.max_texture_side));
        for size in [
            Vec2::new(1400.0, 500.0),
            Vec2::new(800.0, 800.0),
            Vec2::new(2400.0, 400.0),
        ] {
            let rect = Rect::from_min_size(Pos2::ZERO, size);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                paint(ui, rect, &texture, "fill", 0)
            });
            let image = output
                .shapes
                .iter()
                .find(|shape| shape.shape.texture_id() == texture.id())
                .unwrap();
            assert!(image.shape.visual_bounding_rect().contains_rect(rect));
            output.textures_delta.clear();
        }
    }

    #[test]
    fn loads_png_and_paints_all_modes() {
        let path =
            std::env::temp_dir().join(format!("staminator-background-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();
        let ctx = egui::Context::default();
        let texture = load(&ctx, &path).unwrap();
        for mode in ["fit", "fill", "tile"] {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                paint(
                    ui,
                    Rect::from_min_size(Pos2::ZERO, Vec2::splat(100.0)),
                    &texture,
                    mode,
                    100,
                )
            });
            assert!(!output.shapes.is_empty());
            output.textures_delta.clear();
        }
        fs::remove_file(path).unwrap();
    }
}
