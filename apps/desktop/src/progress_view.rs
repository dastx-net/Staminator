use chrono::TimeZone;
use egui::{Align, Color32, Context, FontId, Grid, Pos2, Rect, RichText, Stroke, Vec2, Window};

use crate::localization::tr;

/// Shows saved results for the selected profile. Records are expected in save order;
/// the newest 100 are plotted from oldest to newest.
pub fn show(
    ctx: &Context,
    profile: &str,
    records: &[crate::user_store::ResultRecord],
    open: &mut bool,
) {
    Window::new(tr("progress.title"))
        .open(open)
        .default_width(620.0)
        .default_height(540.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.heading(crate::localization::format(
                "progress.profile_results",
                &staminator_core::FormatArgs::new().insert("profile", profile),
            ));
            ui.add_space(6.0);

            let recent: Vec<_> = records.iter().rev().take(100).collect();
            if recent.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(tr("progress.empty"));
                });
                return;
            }

            let mut chronological = recent;
            chronological.reverse();
            ui.label(RichText::new(tr("progress.speed_chart_title")).strong());
            draw_chart(
                ui,
                &chronological
                    .iter()
                    .map(|r| r.cpm as f32)
                    .collect::<Vec<_>>(),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(tr("progress.recent_sessions")).strong());
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
                                tr("progress.column.date"),
                                tr("progress.column.mode"),
                                tr("progress.column.layout"),
                                tr("progress.column.exercise"),
                                tr("progress.column.speed"),
                                tr("progress.column.errors"),
                            ] {
                                ui.label(RichText::new(title).strong());
                            }
                            ui.end_row();
                            for record in chronological.iter().rev() {
                                let date = format_timestamp(record.timestamp);
                                ui.label(date);
                                ui.label(display_mode(&record.mode));
                                ui.label(display_layout(&record.layout));
                                ui.label(record.exercise.chars().take(42).collect::<String>());
                                ui.label(crate::localization::format(
                                    "unit.characters_per_minute",
                                    &staminator_core::FormatArgs::new().insert("value", record.cpm),
                                ));
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

fn display_mode(value: &str) -> String {
    let key = match value {
        "Уроки" | "mode.lessons" => "mode.lessons",
        "Фразы" | "mode.phrases" => "mode.phrases",
        "Буквы из фраз" | "mode.letters" => "mode.letters",
        "Все клавиши" | "mode.all-keys" => "mode.all-keys",
        "Внешний текст" | "mode.external" => "mode.external",
        _ => return value.to_owned(),
    };
    tr(key)
}

fn display_layout(value: &str) -> String {
    match value {
        "Русская" | "layout.ru" => tr("layout-russian"),
        "English" | "layout.en" => "English".to_owned(),
        _ => value.to_owned(),
    }
}

fn format_timestamp(timestamp: u64) -> String {
    let Ok(timestamp) = i64::try_from(timestamp) else {
        return "—".into();
    };
    let Some(date) = chrono::Local.timestamp_opt(timestamp, 0).single() else {
        return "—".into();
    };
    localized_datetime(&date).unwrap_or_else(|| date.format("%Y-%m-%d %H:%M").to_string())
}

#[cfg(windows)]
fn localized_datetime(date: &chrono::DateTime<chrono::Local>) -> Option<String> {
    use chrono::{Datelike, Timelike};
    use windows_sys::Win32::{
        Foundation::SYSTEMTIME,
        Globalization::{GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS},
    };
    let time = SYSTEMTIME {
        wYear: date.year().try_into().ok()?,
        wMonth: date.month() as u16,
        wDayOfWeek: date.weekday().num_days_from_sunday() as u16,
        wDay: date.day() as u16,
        wHour: date.hour() as u16,
        wMinute: date.minute() as u16,
        wSecond: date.second() as u16,
        wMilliseconds: 0,
    };
    let mut day = [0u16; 128];
    let mut clock = [0u16; 128];
    // SAFETY: SYSTEMTIME and output buffers are valid for the stated lengths; null selects user defaults.
    let (day_len, clock_len) = unsafe {
        (
            GetDateFormatEx(
                std::ptr::null(),
                0,
                &time,
                std::ptr::null(),
                day.as_mut_ptr(),
                day.len() as i32,
                std::ptr::null(),
            ),
            GetTimeFormatEx(
                std::ptr::null(),
                TIME_NOSECONDS,
                &time,
                std::ptr::null(),
                clock.as_mut_ptr(),
                clock.len() as i32,
            ),
        )
    };
    if day_len <= 1 || clock_len <= 1 {
        return None;
    }
    Some(format!(
        "{} {}",
        String::from_utf16_lossy(&day[..day_len as usize - 1]),
        String::from_utf16_lossy(&clock[..clock_len as usize - 1])
    ))
}

#[cfg(not(windows))]
fn localized_datetime(_date: &chrono::DateTime<chrono::Local>) -> Option<String> {
    None
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
        ui.label(
            RichText::new(crate::localization::format(
                "progress.chart_session_count",
                &staminator_core::FormatArgs::new().count(values.len() as i64),
            ))
            .small(),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_handles_out_of_range_and_uses_local_time() {
        assert_eq!(format_timestamp(u64::MAX), "—");
        let rendered = format_timestamp(1_700_000_000);
        assert_ne!(rendered, "—");
        assert!(rendered.chars().any(|c| c.is_ascii_digit()));
    }

    #[test]
    fn history_translates_known_values_and_preserves_custom_values() {
        crate::localization::set_language("EN");
        assert_eq!(display_mode("Фразы"), "Phrases");
        assert_eq!(display_mode("mode.lessons"), "Lessons");
        assert_eq!(display_mode("custom"), "custom");
        assert_eq!(display_layout("Русская"), "Russian");
        assert_eq!(display_layout("custom"), "custom");
        crate::localization::set_language("RU");
    }

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
            stop_reason: "unknown".into(),
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
