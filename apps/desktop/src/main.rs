use eframe::egui;
use staminator_core::{Session, SessionState};
use std::{path::PathBuf, time::Instant};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([820.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Staminator",
        options,
        Box::new(|_cc| Ok(Box::new(StaminatorApp::new()))),
    )
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum TrainingMode {
    #[default]
    Lessons,
    Phrases,
    Letters,
    AllKeys,
    ExternalText,
}

impl TrainingMode {
    const ALL: [(Self, &'static str); 5] = [
        (Self::Lessons, "Уроки"),
        (Self::Phrases, "Фразы"),
        (Self::Letters, "Буквы из фраз"),
        (Self::AllKeys, "Все клавиши"),
        (Self::ExternalText, "Внешний текст"),
    ];
}

struct StaminatorApp {
    selected_mode: TrainingMode,
    lesson_name: String,
    lesson_text: String,
    phrase_text: String,
    data_message: String,
    session: Option<Session>,
    started_at: Instant,
    last_result: Option<staminator_core::SessionStats>,
}

impl StaminatorApp {
    fn new() -> Self {
        let data_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/stamina_setup/Data");
        let lessons = stamina_data::read_lessons(data_dir.join("lessons.ru")).ok();
        let phrases = stamina_data::read_phrases(data_dir.join("phrases.ru")).ok();
        let lesson = lessons.as_ref().and_then(|items| {
            items.iter().find(|item| {
                item.kind == ' ' && item.text.as_deref().is_some_and(|s| !s.trim().is_empty())
            })
        });
        let lesson_name = lesson
            .map(|item| item.name.clone())
            .unwrap_or_else(|| "Начальный урок".into());
        let lesson_text = lesson
            .and_then(|item| item.text.clone())
            .unwrap_or_else(|| "аа оо уу ыы ее мм нн".into());
        let phrase_text = phrases
            .as_ref()
            .and_then(|items| items.lines.iter().find(|line| !line.trim().is_empty()))
            .cloned()
            .unwrap_or_else(|| "Мама мыла раму. Саша шёл по дороге.".into());
        let data_message = if lessons.is_some() && phrases.is_some() {
            "Загружены локальные данные установки Stamina".into()
        } else {
            "Файлы установки не найдены; используются встроенные образцы".into()
        };
        Self {
            selected_mode: TrainingMode::Lessons,
            lesson_name,
            lesson_text,
            phrase_text,
            data_message,
            session: None,
            started_at: Instant::now(),
            last_result: None,
        }
    }

    fn exercise(&self) -> &str {
        match self.selected_mode {
            TrainingMode::Lessons => &self.lesson_text,
            TrainingMode::Phrases => &self.phrase_text,
            _ => "",
        }
    }

    fn now_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis().min(u64::MAX as u128) as u64
    }

    fn start(&mut self) {
        self.session = Some(Session::new(self.exercise().to_owned()));
        self.last_result = None;
    }

    fn stop(&mut self) {
        let now = self.now_ms();
        if let Some(session) = &mut self.session {
            self.last_result = Some(session.stop(now));
        }
    }

    fn toggle_pause(&mut self) {
        let now = self.now_ms();
        if let Some(session) = &mut self.session {
            match session.snapshot(now).state {
                SessionState::Running => session.pause(now),
                SessionState::Paused => session.resume(now),
                _ => {}
            }
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let now = self.now_ms();
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            match event {
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => self.stop(),
                egui::Event::Key {
                    key: egui::Key::F9,
                    pressed: true,
                    ..
                } => self.toggle_pause(),
                egui::Event::Text(text) => {
                    for ch in text.chars() {
                        if let Some(session) = &mut self.session {
                            let _ = session.type_char(ch, now);
                        }
                    }
                }
                _ => {}
            }
        }
        if self.last_result.is_none()
            && self
                .session
                .as_ref()
                .is_some_and(|session| session.snapshot(now).state == SessionState::Stopped)
        {
            self.last_result = self.session.as_ref().map(Session::stats);
        }
    }
}

impl eframe::App for StaminatorApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_input(root.ctx());
        let now = self.now_ms();
        let active = self.session.as_ref().is_some_and(|s| {
            matches!(
                s.snapshot(now).state,
                SessionState::Ready | SessionState::Running | SessionState::Paused
            )
        });

        egui::Panel::top("menu").show(root, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Файл", |ui| {
                    ui.label("Данные читаются из src/stamina_setup/Data");
                });
                ui.menu_button("Тренировка", |ui| {
                    if ui.button("Начать / заново").clicked() {
                        self.start();
                    }
                    if ui.button("Остановить (Esc)").clicked() {
                        self.stop();
                    }
                    if ui.button("Пауза / продолжить (F9)").clicked() {
                        self.toggle_pause();
                    }
                });
                ui.menu_button("Справка", |ui| {
                    ui.label("Первый сценарий набора Staminator");
                });
                ui.separator();
                ui.label(egui::RichText::new("Staminator").strong());
            });
        });

        let snapshot = self.session.as_ref().map(|s| s.snapshot(now));
        let stats = self
            .session
            .as_ref()
            .map(|s| s.stats_at(now))
            .or(self.last_result.clone());
        egui::Panel::bottom("status").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(if active {
                    match snapshot.as_ref().map(|s| s.state) {
                        Some(SessionState::Paused) => "Пауза",
                        Some(SessionState::Ready) => "Ожидание ввода",
                        _ => "Набор",
                    }
                } else {
                    "Готово"
                });
                ui.separator();
                if let Some(stats) = &stats {
                    ui.label(format!("Скорость: {} зн./мин", stats.cpm));
                    ui.separator();
                    ui.label(format!(
                        "Ошибки: {} ({:.1}%)",
                        stats.errors, stats.error_percent
                    ));
                    ui.separator();
                    ui.label(format!(
                        "Время: {}:{:02}",
                        stats.elapsed_ms / 60_000,
                        (stats.elapsed_ms / 1_000) % 60
                    ));
                } else {
                    ui.label("Скорость: — · Ошибки: — · Время: —");
                }
            });
        });

        egui::Panel::left("modes")
            .resizable(false)
            .default_size(190.0)
            .show(root, |ui| {
                ui.heading("Режимы");
                for (mode, label) in TrainingMode::ALL {
                    if matches!(mode, TrainingMode::Lessons | TrainingMode::Phrases) {
                        ui.add_enabled_ui(!active, |ui| {
                            if ui
                                .selectable_value(&mut self.selected_mode, mode, label)
                                .clicked()
                            {
                                self.session = None;
                                self.last_result = None;
                            }
                        });
                    } else {
                        ui.add_enabled(false, egui::Button::new(format!("{label} · скоро")));
                    }
                }
                ui.add_space(14.0);
                ui.separator();
                ui.label("Раскладка: Русская");
                ui.label(if self.selected_mode == TrainingMode::Lessons {
                    format!("Урок: {}", self.lesson_name)
                } else {
                    "Источник: первая фраза".into()
                });
                ui.add_space(10.0);
                if ui.button("Начать / заново").clicked() {
                    self.start();
                }
                if ui
                    .add_enabled(active, egui::Button::new("Пауза / продолжить · F9"))
                    .clicked()
                {
                    self.toggle_pause();
                }
                if ui
                    .add_enabled(active, egui::Button::new("Остановить · Esc"))
                    .clicked()
                {
                    self.stop();
                }
                ui.add_space(10.0);
                ui.small(&self.data_message);
            });

        egui::CentralPanel::default().show(root, |ui| {
            ui.heading("Тренировка");
            ui.label(if self.selected_mode == TrainingMode::Lessons {
                format!("Урок: {} · Русская раскладка", self.lesson_name)
            } else {
                "Фразы · Русская раскладка".into()
            });
            ui.add_space(12.0);
            let position = snapshot.as_ref().map_or(0, |s| s.position);
            let expected = self.exercise().chars().collect::<Vec<_>>();
            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| {
                    ui.set_min_height(150.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        ui.add_space(25.0);
                        let mut job = egui::text::LayoutJob::default();
                        let rest_color = ui.visuals().text_color();
                        for (index, ch) in expected.iter().enumerate() {
                            let color = if index < position {
                                egui::Color32::from_rgb(90, 175, 110)
                            } else if index == position {
                                egui::Color32::from_rgb(255, 190, 65)
                            } else {
                                rest_color
                            };
                            let format = egui::TextFormat {
                                font_id: egui::FontId::proportional(27.0),
                                color,
                                background: if index == position {
                                    egui::Color32::from_rgba_unmultiplied(255, 190, 65, 32)
                                } else {
                                    egui::Color32::TRANSPARENT
                                },
                                ..Default::default()
                            };
                            job.append(&ch.to_string(), 0.0, format);
                        }
                        ui.add(egui::Label::new(job).wrap());
                    });
                });
            let total = expected.len();
            let progress = if total == 0 {
                0.0
            } else {
                position as f32 / total as f32
            };
            ui.add_space(10.0);
            ui.add(
                egui::ProgressBar::new(progress)
                    .desired_width(ui.available_width())
                    .text(format!("{}%", (progress * 100.0) as u32)),
            );
            if let Some(result) = &self.last_result {
                ui.separator();
                ui.heading("Результат");
                ui.label(format!(
                    "Набрано: {} · ошибок: {} · скорость: {} зн./мин · время: {} сек.",
                    result.correct,
                    result.errors,
                    result.cpm,
                    result.elapsed_ms / 1000
                ));
            }
            ui.add_space(12.0);
            ui.separator();
            ui.label(egui::RichText::new("Экранная клавиатура · подсказка").weak());
            keyboard_row(
                ui,
                &[
                    "Ё", "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "-", "=",
                ],
            );
            keyboard_row(
                ui,
                &[
                    "Tab", "Й", "Ц", "У", "К", "Е", "Н", "Г", "Ш", "Щ", "З", "Х", "Ъ",
                ],
            );
            keyboard_row(
                ui,
                &[
                    "Caps", "Ф", "Ы", "В", "А", "П", "Р", "О", "Л", "Д", "Ж", "Э", "Enter",
                ],
            );
            keyboard_row(
                ui,
                &[
                    "Shift", "Я", "Ч", "С", "М", "И", "Т", "Ь", "Б", "Ю", ".", "Shift",
                ],
            );
            keyboard_row(ui, &["Ctrl", "Win", "Alt", "Пробел", "Alt", "Меню", "Ctrl"]);
        });
        if active {
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

fn keyboard_row(ui: &mut egui::Ui, keys: &[&str]) {
    ui.horizontal(|ui| {
        for key in keys {
            let width = if *key == "Пробел" { 170.0 } else { 47.0 };
            ui.add_enabled(
                false,
                egui::Button::new(*key).min_size([width, 34.0].into()),
            );
        }
    });
}
