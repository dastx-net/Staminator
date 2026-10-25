mod keyboard;
mod sound;
mod training_view;

use eframe::egui;
use sound::SoundEvent;
use staminator_core::{Keystroke, Session, SessionState};
use std::{fs, path::PathBuf, time::Instant};

fn keyboard_setting_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("Staminator").join("settings.txt"))
}

fn load_keyboard_setting() -> bool {
    keyboard_setting_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .is_none_or(|value| value.trim() != "false")
}

fn save_keyboard_setting(show: bool) {
    if let Some(path) = keyboard_setting_path() {
        if let Some(parent) = path.parent() {
            if fs::create_dir_all(parent).is_ok() {
                let _ = fs::write(path, show.to_string());
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 560.0])
            .with_min_inner_size([900.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Staminator",
        options,
        Box::new(|cc| Ok(Box::new(StaminatorApp::new(cc)))),
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
    english_layout: bool,
    lesson_name: String,
    lesson_text: String,
    phrase_text: String,
    english_lesson_name: String,
    english_lesson_text: String,
    english_phrase_text: String,
    data_message: String,
    session: Option<Session>,
    started_at: Instant,
    last_result: Option<staminator_core::SessionStats>,
    background: Option<egui::TextureHandle>,
    show_keyboard: bool,
    escape_pending: bool,
}

impl StaminatorApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        training_view::install_text_font(&cc.egui_ctx);
        let data_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/stamina_setup/Data");
        let lessons = stamina_data::read_lessons(data_dir.join("lessons.ru")).ok();
        let phrases = stamina_data::read_phrases(data_dir.join("phrases.ru")).ok();
        let english_lessons = stamina_data::read_lessons(data_dir.join("lessons.us")).ok();
        let english_phrases = stamina_data::read_phrases(data_dir.join("phrases.en")).ok();
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
        let english_lesson = english_lessons.as_ref().and_then(|items| {
            items.iter().find(|item| {
                item.kind == ' ' && item.text.as_deref().is_some_and(|s| !s.trim().is_empty())
            })
        });
        let english_lesson_name = english_lesson
            .map(|item| item.name.clone())
            .unwrap_or_else(|| "Basic lesson".into());
        let english_lesson_text = english_lesson
            .and_then(|item| item.text.clone())
            .unwrap_or_else(|| "asdf jkl; asdf jkl;".into());
        let english_phrase_text = english_phrases
            .as_ref()
            .and_then(|items| items.lines.iter().find(|line| !line.trim().is_empty()))
            .cloned()
            .unwrap_or_else(|| "Practice makes progress.".into());
        let data_message = if lessons.is_some() && phrases.is_some() {
            "Загружены локальные данные установки Stamina".into()
        } else {
            "Файлы установки не найдены; используются встроенные образцы".into()
        };
        let background_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/stamina_setup/Skins/Field.jpg");
        let background = image::open(background_path).ok().map(|image| {
            let image = image.to_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            cc.egui_ctx.load_texture(
                "training-background",
                color_image,
                egui::TextureOptions::LINEAR,
            )
        });
        let show_keyboard = load_keyboard_setting();
        Self {
            selected_mode: TrainingMode::Lessons,
            english_layout: false,
            lesson_name,
            lesson_text,
            phrase_text,
            english_lesson_name,
            english_lesson_text,
            english_phrase_text,
            data_message,
            session: None,
            started_at: Instant::now(),
            last_result: None,
            background,
            show_keyboard,
            escape_pending: false,
        }
    }

    fn exercise(&self) -> &str {
        match (self.english_layout, self.selected_mode) {
            (false, TrainingMode::Lessons) => &self.lesson_text,
            (false, TrainingMode::Phrases) => &self.phrase_text,
            (true, TrainingMode::Lessons) => &self.english_lesson_text,
            (true, TrainingMode::Phrases) => &self.english_phrase_text,
            _ => "",
        }
    }

    fn lesson_name(&self) -> &str {
        if self.english_layout {
            &self.english_lesson_name
        } else {
            &self.lesson_name
        }
    }

    fn now_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis().min(u64::MAX as u128) as u64
    }

    fn start(&mut self) {
        self.session = Some(Session::new(self.exercise().to_owned()));
        self.last_result = None;
        self.escape_pending = false;
        sound::play(SoundEvent::Start);
    }

    fn stop(&mut self) {
        let now = self.now_ms();
        if let Some(session) = &mut self.session {
            self.last_result = Some(session.stop(now));
        }
        self.escape_pending = false;
    }

    fn toggle_pause(&mut self) {
        let now = self.now_ms();
        if let Some(session) = &mut self.session {
            match session.snapshot(now).state {
                SessionState::Ready | SessionState::Running => {
                    session.pause(now);
                    sound::play(SoundEvent::Pause);
                }
                SessionState::Paused => {
                    session.resume(now);
                    self.escape_pending = false;
                    sound::play(SoundEvent::Start);
                }
                _ => {}
            }
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let now = self.now_ms();
        let mut consume_space_text = false;
        for event in ctx.input(|input| input.events.clone()) {
            match event {
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    repeat: false,
                    ..
                } => {
                    let state = self.session.as_ref().map(|s| s.snapshot(now).state);
                    match state {
                        Some(SessionState::Ready | SessionState::Running) => {
                            self.session.as_mut().unwrap().pause(now);
                            self.escape_pending = true;
                            sound::play(SoundEvent::Pause);
                        }
                        Some(SessionState::Paused) if self.escape_pending => self.stop(),
                        Some(SessionState::Paused) => self.escape_pending = true,
                        _ => {}
                    }
                }
                egui::Event::Key {
                    key: egui::Key::C,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if modifiers.ctrl => self.stop(),
                // egui-winit converts Ctrl+C into Copy before emitting a Key event.
                egui::Event::Copy => self.stop(),
                egui::Event::Key {
                    key: egui::Key::Space,
                    pressed: true,
                    repeat: false,
                    ..
                } => {
                    let state = self.session.as_ref().map(|s| s.snapshot(now).state);
                    match state {
                        None | Some(SessionState::Stopped) => {
                            self.start();
                            consume_space_text = true;
                        }
                        Some(SessionState::Paused) => {
                            self.session.as_mut().unwrap().resume(now);
                            self.escape_pending = false;
                            sound::play(SoundEvent::Start);
                            consume_space_text = true;
                        }
                        _ => {}
                    }
                }
                egui::Event::Key {
                    key: egui::Key::F9,
                    pressed: true,
                    ..
                } => self.toggle_pause(),
                egui::Event::Text(text) => {
                    for ch in text.chars() {
                        if ch == ' ' && consume_space_text {
                            consume_space_text = false;
                            continue;
                        }
                        if let Some(session) = &mut self.session {
                            if session.type_char(ch, now) == Keystroke::Incorrect {
                                sound::play(SoundEvent::Error);
                            }
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
        let snapshot = self.session.as_ref().map(|s| s.snapshot(now));
        let active = snapshot.as_ref().is_some_and(|s| {
            matches!(
                s.state,
                SessionState::Ready | SessionState::Running | SessionState::Paused
            )
        });
        let position = snapshot.as_ref().map_or(0, |s| s.position);
        let elapsed = snapshot.as_ref().map_or(0, |s| s.elapsed_ms);
        let next_char = if snapshot
            .as_ref()
            .is_some_and(|s| matches!(s.state, SessionState::Ready | SessionState::Running))
        {
            self.exercise().chars().nth(position)
        } else {
            None
        };
        let stats = self
            .session
            .as_ref()
            .map(|s| s.stats_at(now))
            .or(self.last_result.clone());

        egui::Panel::top("menu").show(root, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                if ui.button(if active { "СТОП" } else { "СТАРТ" }).clicked() {
                    if active {
                        self.stop();
                    } else {
                        self.start();
                    }
                }
                ui.menu_button("Режим", |ui| {
                    ui.add_enabled_ui(!active, |ui| {
                        for (mode, label) in TrainingMode::ALL {
                            let enabled =
                                matches!(mode, TrainingMode::Lessons | TrainingMode::Phrases);
                            if ui
                                .add_enabled(
                                    enabled,
                                    egui::Button::new(label).selected(self.selected_mode == mode),
                                )
                                .clicked()
                            {
                                self.selected_mode = mode;
                                self.session = None;
                                self.last_result = None;
                                ui.close();
                            }
                        }
                        ui.separator();
                        ui.menu_button("Раскладка", |ui| {
                            let russian = ui
                                .selectable_value(&mut self.english_layout, false, "Русская")
                                .clicked();
                            let english = ui
                                .selectable_value(&mut self.english_layout, true, "English")
                                .clicked();
                            if russian || english {
                                self.session = None;
                                self.last_result = None;
                                ui.close();
                            }
                        });
                    });
                });
                ui.add_enabled(false, egui::Button::new("Прогресс"));
                ui.menu_button("Опции", |ui| {
                    if ui
                        .checkbox(&mut self.show_keyboard, "Показывать клавиатуру")
                        .changed()
                    {
                        save_keyboard_setting(self.show_keyboard);
                    }
                    ui.separator();
                    ui.label(if self.selected_mode == TrainingMode::Lessons {
                        format!("Урок: {}", self.lesson_name())
                    } else {
                        "Источник: первая фраза".into()
                    });
                    ui.small(&self.data_message);
                });
                ui.add_enabled(false, egui::Button::new("Пользователи"));
                ui.menu_button("Справка", |ui| {
                    ui.label("Staminator · клавиатурный тренажёр");
                });
            });
        });

        egui::Panel::bottom("metrics").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(if active {
                    if snapshot
                        .as_ref()
                        .is_some_and(|s| s.state == SessionState::Paused)
                    {
                        "Пауза"
                    } else {
                        "Набор"
                    }
                } else if snapshot.is_some() {
                    "Завершено"
                } else {
                    "Ожидание"
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

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| {
                training_view::draw_training(
                    ui,
                    self.background.as_ref(),
                    if snapshot.as_ref().is_some_and(|s| {
                        matches!(s.state, SessionState::Ready | SessionState::Running)
                    }) {
                        self.exercise()
                    } else {
                        ""
                    },
                    position,
                    elapsed,
                    "Пользователь",
                    match snapshot.as_ref().map(|s| s.state) {
                        Some(SessionState::Paused) if self.escape_pending => {
                            Some("Пауза · пробел — продолжить · Esc / Ctrl+C — завершить")
                        }
                        Some(SessionState::Paused) => Some("Пауза · пробел — продолжить"),
                        Some(SessionState::Stopped) => Some("Урок завершён · пробел — заново"),
                        None => Some("Пробел — начать урок"),
                        _ => None,
                    },
                );
            });

        if self.show_keyboard {
            let viewport_id = egui::ViewportId::from_hash_of("keyboard-window");
            let builder = egui::ViewportBuilder::default()
                .with_title("Клавиатура")
                .with_inner_size([1100.0, 420.0])
                .with_min_inner_size([640.0, 250.0]);
            root.ctx()
                .show_viewport_immediate(viewport_id, builder, |ui, _class| {
                    let ctx = ui.ctx().clone();
                    if ctx.input(|input| input.viewport().close_requested()) {
                        self.show_keyboard = false;
                        save_keyboard_setting(false);
                    }
                    self.handle_input(&ctx);
                    let layout = if self.english_layout {
                        keyboard::KeyboardLayout::English
                    } else {
                        keyboard::KeyboardLayout::Russian
                    };
                    keyboard::draw_keyboard(ui, layout, next_char);
                });
        }
        if active {
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
