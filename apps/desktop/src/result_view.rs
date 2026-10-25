use egui::{Align, Context, Grid, RichText, Window};
use staminator_core::SessionStats;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultAction {
    Stay,
    Retry,
    Next,
    Close,
}

/// Draw the session result dialog and report the action selected this frame.
pub fn show(
    ctx: &Context,
    title: &str,
    stats: &SessionStats,
    completed: bool,
    has_next: bool,
) -> ResultAction {
    let mut action = ResultAction::Stay;

    Window::new("Результат занятия")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(300.0);
            ui.label(RichText::new(title).strong());
            ui.add_space(8.0);
            ui.label(if completed {
                "Упражнение завершено"
            } else {
                "Занятие остановлено"
            });
            ui.add_space(8.0);

            Grid::new("session_result_metrics")
                .num_columns(2)
                .spacing([24.0, 6.0])
                .show(ui, |ui| {
                    metric(ui, "Время", format_duration(stats.elapsed_ms));
                    metric(ui, "Правильно", stats.correct.to_string());
                    metric(ui, "Ошибки", stats.errors.to_string());
                    metric(ui, "Точность", format!("{:.1}%", accuracy(stats)));
                    metric(ui, "Скорость", format!("{} зн./мин", stats.cpm));
                });

            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.button("Повторить").clicked() {
                    action = ResultAction::Retry;
                }
                if has_next && ui.button("Далее").clicked() {
                    action = ResultAction::Next;
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Закрыть").clicked() {
                        action = ResultAction::Close;
                    }
                });
            });
        });

    action
}

fn metric(ui: &mut egui::Ui, label: &str, value: String) {
    ui.label(label);
    ui.label(value);
    ui.end_row();
}

fn accuracy(stats: &SessionStats) -> f64 {
    (100.0 - stats.error_percent).clamp(0.0, 100.0)
}

fn format_duration(elapsed_ms: u64) -> String {
    let seconds = elapsed_ms / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
