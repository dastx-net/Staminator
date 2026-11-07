use eframe::egui;
use stamina_data::JsonLesson;
use std::path::PathBuf;

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
            self.error = "Укажите от 1 до 4 символов без пробелов".into();
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
        egui::Window::new("Редактор урока")
            .open(open)
            .default_size([600.0, 500.0])
            .show(ctx, |ui| {
                ui.label("ID урока (для нового: латинские буквы, цифры, - или _)");
                ui.text_edit_singleline(&mut self.lesson.id);
                ui.label("Название");
                ui.text_edit_singleline(&mut self.lesson.title);
                ui.label("Язык (ru или en)");
                ui.text_edit_singleline(&mut self.lesson.language);
                ui.label("Описание");
                ui.text_edit_singleline(self.lesson.description.get_or_insert_with(String::new));
                ui.label("Текст упражнения");
                ui.add(
                    egui::TextEdit::multiline(&mut self.lesson.text)
                        .desired_rows(10)
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    ui.label("Символы для генерации (1–4)");
                    ui.text_edit_singleline(&mut self.symbols);
                    if ui.button("Добавить упражнение").clicked() {
                        self.add_generated_exercise();
                    }
                });
                if !self.error.is_empty() {
                    ui.colored_label(egui::Color32::RED, &self.error);
                }
                save = ui.button("Сохранить урок").clicked();
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
        egui::Window::new("Добавить фразы")
            .open(open)
            .default_size([550.0, 350.0])
            .show(ctx, |ui| {
                ui.label(
                    "Введите фразы: каждая непустая строка станет фразой для выбранного языка.",
                );
                ui.add(
                    egui::TextEdit::multiline(&mut self.text)
                        .desired_rows(10)
                        .desired_width(f32::INFINITY),
                );
                if !self.error.is_empty() {
                    ui.colored_label(egui::Color32::RED, &self.error);
                }
                save = ui.button("Сохранить фразы").clicked();
            });
        save
    }
}

#[cfg(test)]
mod tests {
    use super::{LessonEditor, PhraseEditor};

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
}
