use eframe::egui;
use stamina_data::JsonLesson;
use std::path::PathBuf;

use crate::localization::tr;

pub struct LessonEditor {
    pub lesson: JsonLesson,
    pub path: Option<PathBuf>,
    pub symbols: String,
    pub error: String,
}

impl LessonEditor {
    pub fn new(language: &str) -> Self {
        Self {
            lesson: JsonLesson {
                version: 1,
                id: String::new(),
                title: String::new(),
                language: language.into(),
                description: None,
                text: String::new(),
            },
            path: None,
            symbols: String::new(),
            error: String::new(),
        }
    }

    fn add_generated_exercise(&mut self) -> bool {
        let generated = if self.symbols.chars().any(char::is_whitespace) {
            String::new()
        } else {
            crate::lesson_generator::generate_lesson_text(&self.symbols)
        };
        if generated.is_empty() {
            self.error = tr("content_editor.generated_symbols_error");
            return false;
        }
        if !self.lesson.text.is_empty() && !self.lesson.text.ends_with(char::is_whitespace) {
            self.lesson.text.push(' ');
        }
        self.lesson.text.push_str(&generated);
        self.error.clear();
        true
    }

    pub fn show(&mut self, ctx: &egui::Context, open: &mut bool) -> bool {
        let mut save = false;
        egui::Window::new(tr("content_editor.lesson_title"))
            .open(open)
            .default_size([680.0, 600.0])
            .resizable(true)
            .show(ctx, |ui| {
                crate::ui_style::apply(ui);
                let footer_height = ui.spacing().interact_size.y + 24.0;
                egui::ScrollArea::vertical()
                    .max_height((ui.available_height() - footer_height).max(0.0))
                    .show(ui, |ui| {
                        ui.heading(tr("content_editor.lesson_title"));
                        ui.add_space(8.0);
                        egui::Grid::new("lesson_editor_metadata")
                            .num_columns(2)
                            .spacing([12.0, 8.0])
                            .show(ui, |ui| {
                                ui.label(tr("content_editor.lesson_id"));
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.lesson.id)
                                        .desired_width(f32::INFINITY),
                                );
                                ui.end_row();
                                ui.label(tr("content_editor.lesson_name"));
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.lesson.title)
                                        .desired_width(f32::INFINITY),
                                );
                                ui.end_row();
                                ui.label(tr("content_editor.language"));
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.lesson.language)
                                        .desired_width(f32::INFINITY),
                                );
                                ui.end_row();
                                ui.label(tr("content_editor.description"));
                                ui.add(
                                    egui::TextEdit::singleline(
                                        self.lesson.description.get_or_insert_with(String::new),
                                    )
                                    .desired_width(f32::INFINITY),
                                );
                                ui.end_row();
                            });
                        ui.add_space(12.0);
                        ui.heading(tr("content_editor.exercise_text"));
                        ui.add(
                            egui::TextEdit::multiline(&mut self.lesson.text)
                                .desired_rows(10)
                                .desired_width(f32::INFINITY),
                        );
                        ui.add_space(8.0);
                        ui.group(|ui| {
                            ui.label(tr("content_editor.generated_symbols"));
                            ui.horizontal_wrapped(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.symbols)
                                        .desired_width(220.0),
                                );
                                if ui.button(tr("content_editor.add_exercise")).clicked() {
                                    self.add_generated_exercise();
                                }
                            });
                        });
                        if !self.error.is_empty() {
                            ui.colored_label(ui.visuals().error_fg_color, &self.error);
                        }
                    });
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    save = ui.button(tr("content_editor.save_lesson")).clicked();
                });
            });
        save
    }
}

#[derive(Default)]
pub struct PhraseEditor {
    pub text: String,
    pub error: String,
}

impl PhraseEditor {
    pub fn parsed_phrases(&self) -> Vec<String> {
        self.text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }

    pub fn show(&mut self, ctx: &egui::Context, open: &mut bool) -> bool {
        let mut save = false;
        egui::Window::new(tr("content_editor.phrases_title"))
            .open(open)
            .default_size([600.0, 440.0])
            .resizable(true)
            .show(ctx, |ui| {
                crate::ui_style::apply(ui);
                let footer_height = ui.spacing().interact_size.y + 24.0;
                ui.heading(tr("content_editor.phrases_title"));
                ui.label(tr("content_editor.phrases_instructions"));
                ui.add_space(8.0);
                let content_height = (ui.available_height() - footer_height).max(0.0);
                egui::ScrollArea::vertical()
                    .max_height(content_height)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.text)
                                .desired_rows(12)
                                .desired_width(f32::INFINITY),
                        );
                        if !self.error.is_empty() {
                            ui.colored_label(ui.visuals().error_fg_color, &self.error);
                        }
                    });
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    save = ui.button(tr("content_editor.save_phrases")).clicked();
                });
            });
        save
    }
}

#[cfg(test)]
mod tests {
    use super::{LessonEditor, PhraseEditor};
    use egui::{Context, RawInput, Rect, Vec2, accesskit};

    fn accessible_button_id(
        ctx: &Context,
        label: &str,
        mut show: impl FnMut(&Context) -> bool,
    ) -> accesskit::NodeId {
        let screen = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let mut target = None;
        for _ in 0..3 {
            let mut output = ctx.run_pass(
                RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ctx| {
                    ctx.root_ui(|_| {
                        show(ctx);
                    })
                },
            );
            if let Some(update) = &output.platform_output.accesskit_update {
                target = update
                    .nodes
                    .iter()
                    .find_map(|(id, node)| {
                        (node.role() == accesskit::Role::Button && node.label() == Some(label))
                            .then_some(*id)
                    })
                    .or(target);
            }
            output.textures_delta.clear();
        }
        target.unwrap_or_else(|| panic!("AccessKit button not found: {label}"))
    }

    fn accesskit_click(target_node: accesskit::NodeId) -> egui::Event {
        egui::Event::AccessKitActionRequest(accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node,
            data: None,
        })
    }

    fn frame(
        ctx: &Context,
        events: Vec<egui::Event>,
        mut show: impl FnMut(&Context) -> bool,
    ) -> bool {
        let mut saved = false;
        let mut output = ctx.run_pass(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| ctx.root_ui(|_| saved = show(ctx)),
        );
        output.textures_delta.clear();
        saved
    }

    fn accessible_button_bounds(
        ctx: &Context,
        label: &str,
        screen_size: Vec2,
        mut show: impl FnMut(&Context) -> bool,
    ) -> accesskit::Rect {
        let screen = Rect::from_min_size(egui::Pos2::ZERO, screen_size);
        let mut bounds = None;
        for _ in 0..3 {
            let mut output = ctx.run_pass(
                RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ctx| {
                    ctx.root_ui(|_| {
                        show(ctx);
                    })
                },
            );
            if let Some(update) = &output.platform_output.accesskit_update {
                bounds = update
                    .nodes
                    .iter()
                    .find_map(|(_, node)| {
                        (node.role() == accesskit::Role::Button && node.label() == Some(label))
                            .then(|| node.bounds())
                            .flatten()
                    })
                    .or(bounds);
            }
            output.textures_delta.clear();
        }
        bounds.unwrap_or_else(|| panic!("Visible AccessKit button not found: {label}"))
    }

    fn click_at(
        ctx: &Context,
        pos: egui::Pos2,
        screen_size: Vec2,
        mut show: impl FnMut(&Context) -> bool,
    ) -> bool {
        let screen = Rect::from_min_size(egui::Pos2::ZERO, screen_size);
        let mut output = ctx.run_pass(
            RawInput {
                screen_rect: Some(screen),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                ctx.root_ui(|_| {
                    show(ctx);
                })
            },
        );
        output.textures_delta.clear();
        let mut saved = false;
        let mut output = ctx.run_pass(
            RawInput {
                screen_rect: Some(screen),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |ctx| ctx.root_ui(|_| saved = show(ctx)),
        );
        output.textures_delta.clear();
        saved
    }

    fn button_center_in_window(
        ctx: &Context,
        bounds: accesskit::Rect,
        screen_size: Vec2,
    ) -> egui::Pos2 {
        assert!(bounds.x0 >= 0.0 && bounds.y0 >= 0.0);
        assert!(bounds.x1 <= screen_size.x as f64 && bounds.y1 <= screen_size.y as f64);
        let center = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        let window_layer = ctx
            .layer_id_at(center)
            .expect("save button should be inside a window layer");
        let window_rect = ctx
            .memory(|memory| memory.area_rect(window_layer.id))
            .expect("save button window should have a client area");
        assert!(bounds.x0 as f32 >= window_rect.left());
        assert!(bounds.x1 as f32 <= window_rect.right());
        assert!(bounds.y0 as f32 >= window_rect.top() + 28.0);
        assert!(bounds.y1 as f32 <= window_rect.bottom());
        center
    }

    #[test]
    fn generated_exercise_rejects_empty_whitespace_and_unsupported_symbol_counts() {
        let mut editor = LessonEditor::new("en");
        for symbols in ["", "a b", "abcde"] {
            editor.symbols = symbols.into();
            assert!(!editor.add_generated_exercise(), "accepted {symbols:?}");
            assert_eq!(editor.error, "Укажите от 1 до 4 символов без пробелов");
            assert!(editor.lesson.text.is_empty());
        }
    }

    #[test]
    fn generated_exercise_appends_with_one_separator_and_clears_old_error() {
        let mut editor = LessonEditor::new("ru");
        editor.lesson.text = "existing".into();
        editor.symbols = "ab".into();
        editor.error = "old error".into();

        assert!(editor.add_generated_exercise());
        assert_eq!(
            editor.lesson.text,
            "existing aaabbb aabab baabb ababa bbaba abaab babab "
        );
        assert!(editor.error.is_empty());

        editor.lesson.text = "ends with space ".into();
        editor.symbols = "x".into();
        assert!(editor.add_generated_exercise());
        assert_eq!(editor.lesson.text, "ends with space xxxxx ");
    }

    #[test]
    fn phrase_editor_keeps_nonempty_trimmed_lines_as_separate_phrases() {
        let editor = PhraseEditor {
            text: "  first phrase  \n\n\t\nsecond phrase\r\n".into(),
            error: String::new(),
        };
        assert_eq!(editor.parsed_phrases(), ["first phrase", "second phrase"]);
    }

    #[test]
    fn phrase_editor_empty_input_produces_no_phrases() {
        let editor = PhraseEditor {
            text: " \n\t\n".into(),
            error: String::new(),
        };
        assert!(editor.parsed_phrases().is_empty());
    }

    #[test]
    fn lesson_editor_exposes_metadata_and_saves_through_accesskit() {
        let ctx = Context::default();
        ctx.enable_accesskit();
        let mut editor = LessonEditor::new("en");
        editor.lesson.id = "custom-lesson".into();
        editor.lesson.title = "Custom lesson".into();
        editor.lesson.description = Some("A short description".into());
        editor.lesson.text = "Practice text".into();
        let mut open = true;

        let save = accessible_button_id(
            &ctx,
            &crate::localization::tr("content_editor.save_lesson"),
            |ctx| editor.show(ctx, &mut open),
        );
        assert!(frame(&ctx, vec![accesskit_click(save)], |ctx| {
            editor.show(ctx, &mut open)
        }));

        assert_eq!(editor.lesson.id, "custom-lesson");
        assert_eq!(editor.lesson.title, "Custom lesson");
        assert_eq!(editor.lesson.language, "en");
        assert_eq!(
            editor.lesson.description.as_deref(),
            Some("A short description")
        );
        assert_eq!(editor.lesson.text, "Practice text");
    }

    #[test]
    fn phrase_editor_exposes_save_action_and_keeps_parsed_content() {
        let ctx = Context::default();
        ctx.enable_accesskit();
        let mut editor = PhraseEditor {
            text: "  first phrase  \n\nsecond phrase\n".into(),
            error: String::new(),
        };
        let mut open = true;

        let save = accessible_button_id(
            &ctx,
            &crate::localization::tr("content_editor.save_phrases"),
            |ctx| editor.show(ctx, &mut open),
        );
        assert!(frame(&ctx, vec![accesskit_click(save)], |ctx| {
            editor.show(ctx, &mut open)
        }));

        assert_eq!(editor.parsed_phrases(), ["first phrase", "second phrase"]);
    }

    #[test]
    fn lesson_save_stays_visible_and_clickable_with_long_content_in_short_window() {
        let ctx = Context::default();
        ctx.enable_accesskit();
        let mut editor = LessonEditor::new("en");
        editor.lesson.id = "long-lesson".into();
        editor.lesson.title = "Long lesson".into();
        editor.lesson.text = "Practice text. ".repeat(2_000);
        let mut open = true;
        let screen_size = Vec2::new(1000.0, 280.0);
        let save_label = crate::localization::tr("content_editor.save_lesson");
        let bounds = accessible_button_bounds(&ctx, &save_label, screen_size, |ctx| {
            editor.show(ctx, &mut open)
        });
        let center = button_center_in_window(&ctx, bounds, screen_size);
        assert!(click_at(&ctx, center, screen_size, |ctx| {
            editor.show(ctx, &mut open)
        }));
        assert_eq!(
            editor.lesson.text.chars().count(),
            "Practice text. ".repeat(2_000).chars().count()
        );
    }

    #[test]
    fn lesson_save_is_available_at_large_scales_with_long_error() {
        for scale in [1.5, 2.0] {
            let ctx = Context::default();
            ctx.enable_accesskit();
            ctx.set_zoom_factor(scale);
            let mut editor = LessonEditor::new("en");
            editor.lesson.id = "scaled-lesson".into();
            editor.lesson.title = "Scaled lesson".into();
            editor.lesson.text = "Retain this lesson content.".into();
            editor.error = format!(
                "{}: retry or correct the input",
                "Validation failed".repeat(80)
            );
            let mut open = true;
            let screen_size = Vec2::new(1000.0, 520.0);
            let bounds = accessible_button_bounds(
                &ctx,
                &crate::localization::tr("content_editor.save_lesson"),
                screen_size,
                |ctx| editor.show(ctx, &mut open),
            );
            let center = button_center_in_window(&ctx, bounds, screen_size);

            assert!(
                click_at(&ctx, center, screen_size, |ctx| {
                    editor.show(ctx, &mut open)
                }),
                "Save was not clickable at {scale}x"
            );
            assert_eq!(editor.lesson.id, "scaled-lesson");
            assert_eq!(editor.lesson.title, "Scaled lesson");
            assert_eq!(editor.lesson.text, "Retain this lesson content.");
        }
    }

    #[test]
    fn phrase_save_is_available_at_large_scales_with_long_error() {
        for scale in [1.5, 2.0] {
            let ctx = Context::default();
            ctx.enable_accesskit();
            ctx.set_zoom_factor(scale);
            let mut editor = PhraseEditor {
                text: "  first saved phrase  \nsecond saved phrase\n".into(),
                error: format!(
                    "{}: correct the phrase list",
                    "Validation failed".repeat(80)
                ),
            };
            let mut open = true;
            let screen_size = Vec2::new(1000.0, 520.0);
            let bounds = accessible_button_bounds(
                &ctx,
                &crate::localization::tr("content_editor.save_phrases"),
                screen_size,
                |ctx| editor.show(ctx, &mut open),
            );
            let center = button_center_in_window(&ctx, bounds, screen_size);

            assert!(
                click_at(&ctx, center, screen_size, |ctx| {
                    editor.show(ctx, &mut open)
                }),
                "Save was not clickable at {scale}x"
            );
            assert_eq!(
                editor.parsed_phrases(),
                ["first saved phrase", "second saved phrase"]
            );
        }
    }
}
