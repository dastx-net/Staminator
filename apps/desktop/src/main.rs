mod content;
mod generated_modes;
mod keyboard;
mod progress_view;
mod result_view;
mod sound;
mod training_view;
mod user_store;

use content::{ContentCatalog, ContentLanguage, Exercise};
use eframe::egui;
use result_view::ResultAction;
use sound::SoundEvent;
use staminator_core::{Keystroke, Session, SessionState};
use std::{fs, path::PathBuf, time::Instant};
use user_store::UserStore;

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

fn append_next_phrase(
    session: &mut Session,
    phrases: &[Exercise],
    index: &mut usize,
    ch: char,
    now: u64,
) -> Option<String> {
    if !matches!(
        session.snapshot(now).state,
        SessionState::Ready | SessionState::Running
    ) || session.remaining_chars() != 1
        || session.next_expected() != Some(ch)
    {
        return None;
    }
    let next = *index + 1;
    let text = phrases.get(next)?.text.clone();
    if session.append_expected(&text) {
        *index = next;
        Some(text)
    } else {
        None
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
    catalog: ContentCatalog,
    selected_content: [usize; 4],
    chooser_open: bool,
    chooser_filter: String,
    external_path: Option<PathBuf>,
    external_text: String,
    external_error: String,
    generation: u64,
    session: Option<Session>,
    session_text: String,
    phrase_start_index: usize,
    started_at: Instant,
    last_result: Option<staminator_core::SessionStats>,
    background: Option<egui::TextureHandle>,
    show_keyboard: bool,
    escape_pending: bool,
    result_open: bool,
    result_completed: bool,
    result_title: String,
    result_recorded: bool,
    users: UserStore,
    profiles_open: bool,
    progress_open: bool,
    profile_name_input: String,
    profile_error: String,
}

impl StaminatorApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        training_view::install_text_font(&cc.egui_ctx);
        let catalog = ContentCatalog::load();
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
        let (users, profile_error) = match UserStore::load() {
            Ok(users) => (users, String::new()),
            Err(error) => {
                let recovered_path = std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join("Staminator")
                    .join("profiles-recovered.json");
                let users = UserStore::load_from(&recovered_path).unwrap_or_else(|_| {
                    let unique = recovered_path
                        .with_file_name(format!("profiles-recovered-{}.json", std::process::id()));
                    UserStore::new_at(unique)
                });
                (users, format!("Не удалось загрузить профили: {error}"))
            }
        };
        Self {
            selected_mode: TrainingMode::Lessons,
            english_layout: false,
            catalog,
            selected_content: [0; 4],
            chooser_open: false,
            chooser_filter: String::new(),
            external_path: None,
            external_text: String::new(),
            external_error: String::new(),
            generation: 0,
            session: None,
            session_text: String::new(),
            phrase_start_index: 0,
            started_at: Instant::now(),
            last_result: None,
            background,
            show_keyboard,
            escape_pending: false,
            result_open: false,
            result_completed: false,
            result_title: String::new(),
            result_recorded: false,
            users,
            profiles_open: false,
            progress_open: false,
            profile_name_input: String::new(),
            profile_error,
        }
    }

    fn language(&self) -> ContentLanguage {
        if self.english_layout {
            ContentLanguage::English
        } else {
            ContentLanguage::Russian
        }
    }

    fn content_slot(&self) -> usize {
        usize::from(self.english_layout) * 2
            + usize::from(self.selected_mode == TrainingMode::Phrases)
    }

    fn items(&self) -> &[Exercise] {
        match self.selected_mode {
            TrainingMode::Phrases => self.catalog.phrases(self.language()),
            _ => self.catalog.lessons(self.language()),
        }
    }

    fn current_exercise(&self) -> &Exercise {
        &self.items()[self.selected_content[self.content_slot()].min(self.items().len() - 1)]
    }

    fn exercise(&self) -> &str {
        match self.selected_mode {
            TrainingMode::Lessons | TrainingMode::Phrases => &self.current_exercise().text,
            TrainingMode::ExternalText => &self.external_text,
            _ => "",
        }
    }

    fn content_mode(&self) -> bool {
        matches!(
            self.selected_mode,
            TrainingMode::Lessons | TrainingMode::Phrases
        )
    }

    fn current_label(&self) -> String {
        match self.selected_mode {
            TrainingMode::Lessons | TrainingMode::Phrases => self.current_exercise().name.clone(),
            TrainingMode::Letters => "Буквы из фраз".into(),
            TrainingMode::AllKeys => "Все клавиши".into(),
            TrainingMode::ExternalText => self
                .external_path
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Внешний текст не выбран".into()),
        }
    }

    fn generated_text(&mut self) -> String {
        self.generation = self.generation.wrapping_add(1);
        let seed = self.now_ms().wrapping_add(self.generation);
        match self.selected_mode {
            TrainingMode::Letters => {
                generated_modes::letters_from_phrases(self.catalog.phrases(self.language()), seed)
            }
            TrainingMode::AllKeys => {
                generated_modes::all_keys(self.catalog.keys(self.language()), seed)
            }
            _ => String::new(),
        }
    }

    fn choose_external_text(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Открыть текст для тренировки")
            .pick_file()
        {
            match fs::read(&path) {
                Ok(bytes) => {
                    let decoded = stamina_data::decode_legacy(&bytes);
                    let text = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
                    if text.is_empty() {
                        self.external_error = "В выбранном файле нет текста для набора".into();
                    } else {
                        self.external_path = Some(path);
                        self.external_text = text;
                        self.external_error.clear();
                        self.session = None;
                        self.last_result = None;
                        self.result_open = false;
                    }
                }
                Err(error) => self.external_error = format!("Не удалось открыть файл: {error}"),
            }
        }
    }

    fn select_content(&mut self, index: usize) {
        self.selected_content[self.content_slot()] = index.min(self.items().len() - 1);
        self.session = None;
        self.last_result = None;
        self.result_open = false;
        self.escape_pending = false;
    }

    fn advance_content(&mut self) -> bool {
        let next = self.selected_content[self.content_slot()] + 1;
        if next < self.items().len() {
            self.select_content(next);
            true
        } else {
            false
        }
    }

    fn now_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis().min(u64::MAX as u128) as u64
    }

    fn start(&mut self) {
        self.session_text = match self.selected_mode {
            TrainingMode::Letters | TrainingMode::AllKeys => self.generated_text(),
            _ => self.exercise().to_owned(),
        };
        if self.session_text.is_empty() {
            return;
        }
        self.phrase_start_index = self.selected_content[self.content_slot()];
        self.session = Some(Session::new(self.session_text.clone()));
        self.last_result = None;
        self.escape_pending = false;
        self.result_open = false;
        self.result_completed = false;
        self.result_recorded = false;
        sound::play(SoundEvent::Start);
    }

    fn stop(&mut self) {
        let now = self.now_ms();
        if !self.session.as_ref().is_some_and(|session| {
            matches!(
                session.snapshot(now).state,
                SessionState::Ready | SessionState::Running | SessionState::Paused
            )
        }) {
            return;
        }
        self.result_title = self.session_result_title();
        if let Some(session) = &mut self.session {
            self.result_completed = session.remaining_chars() == 0;
            self.last_result = Some(session.stop(now));
            self.result_open = true;
        }
        self.escape_pending = false;
        self.record_current_result();
    }

    fn record_current_result(&mut self) {
        if self.result_recorded {
            return;
        }
        let Some(stats) = &self.last_result else {
            return;
        };
        self.result_recorded = true;
        if stats.correct == 0 && stats.errors == 0 {
            return;
        }
        let mode = TrainingMode::ALL
            .iter()
            .find(|(kind, _)| *kind == self.selected_mode)
            .map(|(_, label)| *label)
            .unwrap_or("Занятие");
        let layout = if self.english_layout {
            "English"
        } else {
            "Русская"
        };
        if let Err(error) = self.users.record(
            mode,
            layout,
            &self.result_title,
            stats.correct as u64,
            stats.errors as u64,
            stats.elapsed_ms,
            stats.cpm as f64,
            stats.error_percent,
        ) {
            self.profile_error = format!("Не удалось сохранить результат: {error}");
        }
    }

    fn session_result_title(&self) -> String {
        if self.selected_mode == TrainingMode::Phrases {
            format!(
                "Фразы {}–{}",
                self.phrase_start_index + 1,
                self.selected_content[self.content_slot()] + 1
            )
        } else {
            self.current_label()
        }
    }

    fn type_input_char(&mut self, ch: char, now: u64) {
        let at_boundary = self.session.as_ref().is_some_and(|session| {
            matches!(
                session.snapshot(now).state,
                SessionState::Ready | SessionState::Running
            ) && session.remaining_chars() == 1
                && session.next_expected() == Some(ch)
        });
        if self.selected_mode == TrainingMode::Phrases {
            let slot = self.content_slot();
            let phrases = self.catalog.phrases(self.language());
            if let Some(session) = &mut self.session {
                if let Some(next_text) =
                    append_next_phrase(session, phrases, &mut self.selected_content[slot], ch, now)
                {
                    self.session_text.push_str(&next_text);
                }
            }
        } else if at_boundary
            && matches!(
                self.selected_mode,
                TrainingMode::Letters | TrainingMode::AllKeys
            )
        {
            let next_text = format!(" {}", self.generated_text());
            if self
                .session
                .as_mut()
                .is_some_and(|session| session.append_expected(&next_text))
            {
                self.session_text.push_str(&next_text);
            }
        }
        if let Some(session) = &mut self.session {
            if session.type_char(ch, now) == Keystroke::Incorrect {
                sound::play(SoundEvent::Error);
            }
        }
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
        if self.result_open || self.chooser_open || self.profiles_open {
            return;
        }
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
                        self.type_input_char(ch, now);
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
            self.result_open = true;
            self.result_completed = true;
            self.result_title = self.session_result_title();
            self.record_current_result();
        }
    }

    fn show_content_chooser(&mut self, ctx: &egui::Context) {
        let mut open = self.chooser_open;
        let mut chosen = None;
        let current = self.selected_content[self.content_slot()];
        egui::Window::new(if self.selected_mode == TrainingMode::Lessons {
            "Выбор урока"
        } else {
            "Выбор фразы"
        })
        .open(&mut open)
        .default_size([620.0, 470.0])
        .show(ctx, |ui| {
            ui.text_edit_singleline(&mut self.chooser_filter);
            let filter = self.chooser_filter.trim().to_lowercase();
            let matches: Vec<_> = self
                .items()
                .iter()
                .enumerate()
                .filter(|(_, item)| filter.is_empty() || item.name.to_lowercase().contains(&filter))
                .map(|(index, item)| (index, item.name.chars().take(85).collect::<String>()))
                .collect();
            ui.label(format!("Найдено: {}", matches.len()));
            egui::ScrollArea::vertical().max_height(385.0).show_rows(
                ui,
                24.0,
                matches.len(),
                |ui, range| {
                    for (index, name) in &matches[range] {
                        if ui
                            .selectable_label(*index == current, format!("{}. {}", index + 1, name))
                            .clicked()
                        {
                            chosen = Some(*index);
                        }
                    }
                },
            );
        });
        self.chooser_open = open;
        if let Some(index) = chosen {
            self.select_content(index);
            self.chooser_open = false;
            self.chooser_filter.clear();
        }
    }

    fn show_profiles(&mut self, ctx: &egui::Context) {
        let mut open = self.profiles_open;
        egui::Window::new("Пользователи")
            .open(&mut open)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.heading("Профили");
                let profiles: Vec<_> = self
                    .users
                    .profiles()
                    .iter()
                    .map(|profile| (profile.id.clone(), profile.name.clone()))
                    .collect();
                for (id, name) in profiles {
                    if ui
                        .selectable_label(self.users.active_profile_id() == id, name)
                        .clicked()
                    {
                        match self.users.select(&id) {
                            Ok(()) => {
                                self.session = None;
                                self.last_result = None;
                                self.result_open = false;
                                self.profile_error.clear();
                            }
                            Err(error) => self.profile_error = error.to_string(),
                        }
                    }
                }
                ui.separator();
                ui.label("Имя профиля");
                ui.text_edit_singleline(&mut self.profile_name_input);
                ui.horizontal(|ui| {
                    if ui.button("Создать").clicked() {
                        match self.users.create(&self.profile_name_input) {
                            Ok(_) => {
                                self.profile_name_input.clear();
                                self.profile_error.clear();
                            }
                            Err(error) => self.profile_error = error.to_string(),
                        }
                    }
                    if ui.button("Переименовать").clicked() {
                        let id = self.users.active_profile_id().to_owned();
                        match self.users.rename(&id, &self.profile_name_input) {
                            Ok(()) => {
                                self.profile_name_input.clear();
                                self.profile_error.clear();
                            }
                            Err(error) => self.profile_error = error.to_string(),
                        }
                    }
                    if ui
                        .add_enabled(
                            self.users.profiles().len() > 1,
                            egui::Button::new("Удалить текущий"),
                        )
                        .clicked()
                    {
                        let id = self.users.active_profile_id().to_owned();
                        match self.users.delete(&id) {
                            Ok(()) => self.profile_error.clear(),
                            Err(error) => self.profile_error = error.to_string(),
                        }
                    }
                });
                if !self.profile_error.is_empty() {
                    ui.colored_label(egui::Color32::RED, &self.profile_error);
                }
            });
        self.profiles_open = open;
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
            self.session_text.chars().nth(position)
        } else {
            None
        };
        let stats = self
            .session
            .as_ref()
            .map(|s| s.stats_at(now))
            .or(self.last_result.clone());
        let current_label = self.current_label();
        let current_index = if self.content_mode() {
            self.selected_content[self.content_slot()]
        } else {
            0
        };
        let content_count = if self.content_mode() {
            self.items().len()
        } else {
            1
        };

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
                            if ui
                                .add(egui::Button::new(label).selected(self.selected_mode == mode))
                                .clicked()
                            {
                                self.selected_mode = mode;
                                self.session = None;
                                self.last_result = None;
                                self.result_open = false;
                                self.chooser_open = false;
                                if mode == TrainingMode::ExternalText
                                    && self.external_text.is_empty()
                                {
                                    self.choose_external_text();
                                }
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
                                self.result_open = false;
                                ui.close();
                            }
                        });
                    });
                });
                ui.menu_button("Занятие", |ui| {
                    ui.add_enabled_ui(!active, |ui| {
                        if self.content_mode() {
                            if ui.button("Выбрать…").clicked() {
                                self.chooser_open = true;
                                self.chooser_filter.clear();
                                ui.close();
                            }
                            if ui
                                .add_enabled(current_index > 0, egui::Button::new("Предыдущее"))
                                .clicked()
                            {
                                self.select_content(current_index - 1);
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    current_index + 1 < content_count,
                                    egui::Button::new("Следующее"),
                                )
                                .clicked()
                            {
                                self.select_content(current_index + 1);
                                ui.close();
                            }
                            ui.separator();
                            ui.label(format!("{} из {}", current_index + 1, content_count));
                        } else if self.selected_mode == TrainingMode::ExternalText {
                            if ui.button("Открыть текстовый файл…").clicked()
                            {
                                self.choose_external_text();
                                ui.close();
                            }
                            if !self.external_error.is_empty() {
                                ui.colored_label(egui::Color32::RED, &self.external_error);
                            }
                        } else {
                            ui.label("Новый текст создаётся при старте");
                        }
                    });
                });
                if ui.button("Прогресс").clicked() {
                    self.progress_open = true;
                }
                ui.menu_button("Опции", |ui| {
                    if ui
                        .checkbox(&mut self.show_keyboard, "Показывать клавиатуру")
                        .changed()
                    {
                        save_keyboard_setting(self.show_keyboard);
                    }
                    ui.separator();
                    ui.label(format!("Занятие: {current_label}"));
                    ui.small(if self.catalog.diagnostics().is_empty() {
                        "Загружены локальные данные Stamina"
                    } else {
                        "Часть файлов не найдена; используются встроенные образцы"
                    });
                    if !self.profile_error.is_empty() {
                        ui.colored_label(egui::Color32::RED, &self.profile_error);
                    }
                });
                if ui
                    .add_enabled(!active, egui::Button::new("Пользователи"))
                    .clicked()
                {
                    self.profiles_open = true;
                }
                ui.menu_button("Справка", |ui| {
                    ui.label("Staminator · клавиатурный тренажёр");
                });
                ui.separator();
                ui.label(format!(
                    "{} / {} · {}",
                    current_index + 1,
                    content_count,
                    current_label.chars().take(44).collect::<String>()
                ));
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
                        &self.session_text
                    } else {
                        ""
                    },
                    position,
                    elapsed,
                    self.users.active_name(),
                    match snapshot.as_ref().map(|s| s.state) {
                        Some(SessionState::Paused) if self.escape_pending => {
                            Some("Пауза · пробел — продолжить · Esc / Ctrl+C — завершить")
                        }
                        Some(SessionState::Paused) => Some("Пауза · пробел — продолжить"),
                        Some(SessionState::Stopped) => Some("Занятие завершено · пробел — заново"),
                        None if self.selected_mode == TrainingMode::ExternalText
                            && self.external_text.is_empty() =>
                        {
                            Some("Занятие → Открыть текстовый файл")
                        }
                        None => Some("Пробел — начать занятие"),
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
        if self.chooser_open {
            self.show_content_chooser(root.ctx());
        }
        if self.profiles_open {
            self.show_profiles(root.ctx());
        }
        if self.progress_open {
            progress_view::show(
                root.ctx(),
                self.users.active_name(),
                self.users.active_records(),
                &mut self.progress_open,
            );
        }
        if self.result_open {
            if let Some(stats) = self.last_result.clone() {
                let has_next = self.content_mode()
                    && self.selected_content[self.content_slot()] + 1 < self.items().len();
                match result_view::show(
                    root.ctx(),
                    &self.result_title,
                    &stats,
                    self.result_completed,
                    has_next,
                ) {
                    ResultAction::Stay => {}
                    ResultAction::Retry => self.start(),
                    ResultAction::Next => {
                        if self.advance_content() {
                            self.start();
                        }
                    }
                    ResultAction::Close => self.result_open = false,
                }
            }
        }
        if active {
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_and_english_phrases_continue_in_one_session() {
        for (first, second) in [("аб ", "вг "), ("ab ", "cd ")] {
            let phrases = vec![
                Exercise {
                    name: "1".into(),
                    text: first.into(),
                },
                Exercise {
                    name: "2".into(),
                    text: second.into(),
                },
            ];
            let mut session = Session::new(first);
            let mut index = 0;
            let chars: Vec<_> = first.chars().collect();
            for ch in &chars[..chars.len() - 1] {
                assert_eq!(session.type_char(*ch, 100), Keystroke::Correct);
            }
            let last = *chars.last().unwrap();
            assert_eq!(
                append_next_phrase(&mut session, &phrases, &mut index, last, 200),
                Some(second.into())
            );
            assert_eq!(session.type_char(last, 200), Keystroke::Correct);
            assert_eq!(session.snapshot(200).state, SessionState::Running);
            assert_eq!(index, 1);
            for ch in second.chars() {
                assert_eq!(session.type_char(ch, 300), Keystroke::Correct);
            }
            assert_eq!(session.snapshot(300).state, SessionState::Stopped);
            assert_eq!(
                session.stats().correct,
                first.chars().count() + second.chars().count()
            );
        }
    }
}
