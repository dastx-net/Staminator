use chrono::TimeZone;
use egui::{Align, Color32, Context, FontId, Grid, Pos2, Rect, RichText, Stroke, Vec2, Window};

/// Shows saved results for the selected profile. Records are expected in save order;
/// the newest 100 are plotted from oldest to newest.
pub fn show(
    ctx: &Context,
    profile: &str,
    records: &[crate::user_store::ResultRecord],
    open: &mut bool,
) {
    Window::new("Прогресс")
        .open(open)
        .default_width(620.0)
        .default_height(540.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.heading(format!("Результаты профиля: {profile}"));
            ui.add_space(6.0);

            let recent: Vec<_> = records.iter().rev().take(100).collect();
            if recent.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label("Для этого профиля пока нет сохранённых занятий.");
                });
                return;
            }

            let mut chronological = recent;
            chronological.reverse();
            ui.label(RichText::new("Скорость по занятиям (знаков в минуту)").strong());
            draw_chart(
                ui,
                &chronological
                    .iter()
                    .map(|r| r.cpm as f32)
                    .collect::<Vec<_>>(),
            );
            ui.add_space(8.0);
            ui.label(RichText::new("Последние занятия").strong());
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .max_height(230.0)
                .show(ui, |ui| {
                    Grid::new("progress_recent_records")
                        .striped(true)
                        .num_columns(6)
                        .spacing([10.0, 5.0])
                        .show(ui, |ui| {
                            for title in [
                                "Дата",
                                "Режим",
                                "Раскладка",
                                "Упражнение",
                                "Скорость",
                                "Ошибки",
                            ] {
                                ui.label(RichText::new(title).strong());
                            }
                            ui.end_row();
                            for record in chronological.iter().rev() {
                                let date = chrono::Local
                                    .timestamp_opt(record.timestamp as i64, 0)
                                    .single()
                                    .map(|date| date.format("%d.%m.%Y %H:%M").to_string())
                                    .unwrap_or_else(|| "—".into());
                                ui.label(date);
                                ui.label(record.mode.to_string());
                                ui.label(record.layout.to_string());
                                ui.label(record.exercise.chars().take(42).collect::<String>());
                                ui.label(format!("{} зн./мин", record.cpm));
                                ui.label(format!(
                                    "{} ({:.1}%)",
                                    record.errors, record.error_percent
                                ));
                                ui.end_row();
                            }
                        });
                });
        });
}

fn draw_chart(ui: &mut egui::Ui, values: &[f32]) {
    let desired = Vec2::new(ui.available_width().max(280.0), 180.0);
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let plot = Rect::from_min_max(
        Pos2::new(rect.left() + 42.0, rect.top() + 8.0),
        Pos2::new(rect.right() - 12.0, rect.bottom() - 22.0),
    );
    let max = values.iter().copied().fold(0.0_f32, f32::max).max(10.0);
    let step = (max / 4.0).ceil().max(1.0);
    for i in 0..=4 {
        let value = step * i as f32;
        let y = plot.bottom() - (value / (step * 4.0)) * plot.height();
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0, Color32::from_gray(65)),
        );
        painter.text(
            Pos2::new(plot.left() - 6.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{value:.0}"),
            FontId::proportional(10.0),
            ui.visuals().text_color(),
        );
    }

    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let x = if values.len() <= 1 {
                plot.center().x
            } else {
                plot.left() + i as f32 / (values.len() - 1) as f32 * plot.width()
            };
            let y = plot.bottom() - value.max(0.0) / (step * 4.0) * plot.height();
            Pos2::new(x, y)
        })
        .collect();
    if points.len() == 1 {
        painter.circle_filled(points[0], 3.0, Color32::from_rgb(90, 190, 245));
    } else {
        painter.add(egui::Shape::line(
            points,
            Stroke::new(2.0, Color32::from_rgb(90, 190, 245)),
        ));
    }
    painter.line_segment(
        [plot.left_bottom(), plot.right_bottom()],
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.fg_stroke.color),
    );
    ui.with_layout(egui::Layout::left_to_right(Align::Min), |ui| {
        ui.add_space(plot.left() - rect.left());
        ui.label(RichText::new(format!("{} занятий", values.len())).small());
    });
}
