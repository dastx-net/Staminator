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

#[cfg(test)]
mod tests {
    use super::*;

    fn render(records: &[crate::user_store::ResultRecord]) -> String {
        let ctx = Context::default();
        let mut open = true;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 900.0))),
            ..Default::default()
        };
        let mut output = ctx.run_pass(input.clone(), |ctx| {
            ctx.root_ui(|_| show(ctx, "Тестовый профиль", records, &mut open))
        });
        output.textures_delta.clear();
        let mut output = ctx.run_pass(input, |ctx| {
            ctx.root_ui(|_| show(ctx, "Тестовый профиль", records, &mut open))
        });
        assert!(open);
        let shapes = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        output.textures_delta.clear();
        shapes
    }

    fn record(index: usize) -> crate::user_store::ResultRecord {
        crate::user_store::ResultRecord {
            timestamp: 1_700_000_000 + index as u64,
            mode: "Обычный".into(),
            layout: "Русская".into(),
            exercise: format!("Упражнение {index}"),
            correct: 100,
            errors: index as u64,
            elapsed_ms: 60_000,
            cpm: (40 + index) as f64,
            error_percent: 2.5,
        }
    }

    #[test]
    fn empty_history_has_profile_specific_empty_state() {
        let shapes = render(&[]);
        assert!(shapes.contains("Результаты профиля: Тестовый профиль"));
        assert!(shapes.contains("Для этого профиля пока нет сохранённых занятий."));
        assert!(!shapes.contains("Скорость по занятиям"));
    }

    #[test]
    fn history_shows_latest_first_and_limits_chart_to_latest_hundred() {
        let records: Vec<_> = (0..102).map(record).collect();
        let shapes = render(&records);
        assert!(shapes.contains("Скорость по занятиям (знаков в минуту)"));
        assert!(shapes.contains("100 занятий"));
        assert!(shapes.contains("Упражнение 101"));
        assert!(shapes.contains("Упражнение 100"));
        assert!(shapes.find("Упражнение 101").unwrap() < shapes.find("Упражнение 100").unwrap());
        assert!(!shapes.lines().any(|line| line == "Упражнение 0"));
        assert!(shapes.contains("Упражнение"));
        assert!(shapes.contains("Раскладка"));
    }
}
