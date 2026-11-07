use egui::{Align, Context, Grid, RichText, Window};
use staminator_core::SessionStats;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultAction {
    Stay,
    Continue,
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
    can_continue: bool,
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
                if can_continue && ui.button("Продолжить").clicked() {
                    action = ResultAction::Continue;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{RawInput, Rect, Vec2};

    fn accessible_action(label: &str, has_next: bool) -> ResultAction {
        let ctx = Context::default();
        ctx.enable_accesskit();
        let stats = SessionStats {
            correct: 1,
            errors: 0,
            elapsed_ms: 1_000,
            cpm: 60,
            error_percent: 0.0,
        };
        let screen = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 700.0));
        let mut target = None;
        for _ in 0..2 {
            let mut output = ctx.run_pass(
                RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ctx| {
                    ctx.root_ui(|_| {
                        show(ctx, "Доступность", &stats, true, has_next, false);
                    })
                },
            );
            if let Some(update) = &output.platform_output.accesskit_update {
                target = update
                    .nodes
                    .iter()
                    .find_map(|(id, node)| (node.label() == Some(label)).then_some(*id))
                    .or(target);
            }
            output.textures_delta.clear();
        }
        let target = target.unwrap_or_else(|| panic!("AccessKit button not found: {label}"));
        let request = egui::Event::AccessKitActionRequest(accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: target,
            data: None,
        });
        let mut action = ResultAction::Stay;
        let mut output = ctx.run_pass(
            RawInput {
                screen_rect: Some(screen),
                events: vec![request],
                ..Default::default()
            },
            |ctx| ctx.root_ui(|_| action = show(ctx, "Доступность", &stats, true, has_next, false)),
        );
        output.textures_delta.clear();
        action
    }

    fn frame(ctx: &Context, mut f: impl FnMut(&Context)) -> String {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(900.0, 700.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_pass(input.clone(), |ctx| ctx.root_ui(|_| f(ctx)));
        output.textures_delta.clear();
        let mut output = ctx.run_pass(input, |ctx| ctx.root_ui(|_| f(ctx)));
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

    #[test]
    fn result_shows_completed_metrics_and_only_available_actions() {
        let ctx = Context::default();
        let stats = SessionStats {
            correct: 123,
            errors: 7,
            elapsed_ms: 125_000,
            cpm: 61,
            error_percent: 5.0,
        };
        let mut action = ResultAction::Stay;
        let shapes = frame(&ctx, |ctx| {
            action = show(ctx, "Урок 1", &stats, true, false, false);
        });

        assert_eq!(action, ResultAction::Stay);
        for text in [
            "Урок 1",
            "Упражнение завершено",
            "2:05",
            "123",
            "7",
            "95.0%",
            "61 зн./мин",
            "Повторить",
            "Закрыть",
        ] {
            assert!(
                shapes.contains(text),
                "missing visible result text: {text}; got {shapes:?}"
            );
        }
        assert!(!shapes.contains("Далее"));
    }

    #[test]
    fn stopped_result_clamps_accuracy_and_includes_next_action() {
        let ctx = Context::default();
        let stats = SessionStats {
            correct: 0,
            errors: 1,
            elapsed_ms: 0,
            cpm: 0,
            error_percent: 140.0,
        };
        let shapes = frame(&ctx, |ctx| {
            assert_eq!(
                show(ctx, "Свободный режим", &stats, false, true, true),
                ResultAction::Stay
            );
        });
        assert!(shapes.contains("Занятие остановлено"));
        assert!(shapes.contains("0.0%"));
        assert!(shapes.contains("Далее"));
    }

    #[test]
    fn accessible_button_actions_select_retry_next_and_close() {
        assert_eq!(accessible_action("Повторить", false), ResultAction::Retry);
        assert_eq!(accessible_action("Далее", true), ResultAction::Next);
        assert_eq!(accessible_action("Закрыть", false), ResultAction::Close);
    }
}
