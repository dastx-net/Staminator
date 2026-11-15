use egui::{Align, Context, Grid, RichText, ScrollArea, Window};
use staminator_core::SessionStats;

use crate::localization::tr;

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

    Window::new(tr("result.title"))
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            crate::ui_style::apply(ui);
            ui.set_min_width(260.0);
            ui.set_max_width(520.0);
            ScrollArea::vertical()
                .max_height(ui.ctx().content_rect().height() * 0.8)
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title).heading().strong());
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(if completed {
                                tr("result.completed")
                            } else {
                                tr("result.stopped")
                            })
                            .weak(),
                        );

                        ui.add_space(20.0);
                        ui.separator();
                        ui.add_space(12.0);
                        Grid::new("session_result_metrics")
                            .num_columns(2)
                            .spacing([16.0, 10.0])
                            .show(ui, |ui| {
                                metric(ui, &tr("result.time"), format_duration(stats.elapsed_ms));
                                metric(ui, &tr("result.correct"), stats.correct.to_string());
                                metric(ui, &tr("result.errors"), stats.errors.to_string());
                                metric(
                                    ui,
                                    &tr("result.accuracy"),
                                    format!("{:.1}%", accuracy(stats)),
                                );
                                metric(
                                    ui,
                                    &tr("result.speed"),
                                    crate::localization::format(
                                        "unit.characters_per_minute",
                                        &staminator_core::FormatArgs::new()
                                            .insert("value", stats.cpm),
                                    ),
                                );
                            });
                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(12.0);
                        ui.horizontal_wrapped(|ui| {
                            if can_continue && ui.button(tr("result.continue")).clicked() {
                                action = ResultAction::Continue;
                            }
                            if ui.button(tr("result.retry")).clicked() {
                                action = ResultAction::Retry;
                            }
                            if has_next && ui.button(tr("result.next")).clicked() {
                                action = ResultAction::Next;
                            }
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if ui.button(tr("common.close")).clicked() {
                                    action = ResultAction::Close;
                                }
                            });
                        });
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

    fn accessible_action(
        label: &str,
        has_next: bool,
        can_continue: bool,
        dark_theme: bool,
    ) -> Option<ResultAction> {
        let ctx = Context::default();
        ctx.enable_accesskit();
        ctx.set_visuals(if dark_theme {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        let stats = SessionStats {
            correct: 1,
            errors: 0,
            elapsed_ms: 1_000,
            cpm: 60,
            error_percent: 0.0,
        };
        let screen = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(320.0, 600.0));
        let mut target = None;
        for _ in 0..2 {
            let mut output = ctx.run_pass(
                RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ctx| {
                    ctx.root_ui(|_| {
                        show(ctx, "Доступность", &stats, true, has_next, can_continue);
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
        let target = target?;
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
            |ctx| {
                ctx.root_ui(|_| {
                    action = show(ctx, "Доступность", &stats, true, has_next, can_continue)
                })
            },
        );
        output.textures_delta.clear();
        Some(action)
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
    fn accessible_actions_work_in_both_themes_at_narrow_width_and_respect_availability() {
        for dark_theme in [false, true] {
            for (label, has_next, can_continue, expected) in [
                ("Продолжить", true, true, ResultAction::Continue),
                ("Повторить", true, true, ResultAction::Retry),
                ("Далее", true, true, ResultAction::Next),
                ("Закрыть", true, true, ResultAction::Close),
            ] {
                assert_eq!(
                    accessible_action(label, has_next, can_continue, dark_theme),
                    Some(expected),
                    "action {label} failed in dark_theme={dark_theme}"
                );
            }

            assert_eq!(
                accessible_action("Продолжить", true, false, dark_theme),
                None,
                "Continue must be omitted when continuation is unavailable"
            );
            assert_eq!(
                accessible_action("Далее", false, true, dark_theme),
                None,
                "Next must be omitted when there is no next exercise"
            );
        }
    }
}
