mod background;
mod content;
mod content_editor;
mod external_text;
mod generated_modes;
mod keyboard;
mod lesson_generator;
mod localization;
mod progress_view;
mod result_view;
mod sound;
mod storage_transfer;
mod training_view;
mod user_store;

use content::{ContentCatalog, ContentLanguage, Exercise};
use eframe::egui;
use result_view::ResultAction;
use sound::SoundEvent;
use staminator_core::{FormatArgs, Keystroke, Session, SessionState, StopReason};
use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
use user_store::UserStore;

fn background_mode(mode: user_store::BackgroundPlacement) -> &'static str {
    match mode {
        user_store::BackgroundPlacement::Fit => "fit",
        user_store::BackgroundPlacement::Fill => "fill",
        user_store::BackgroundPlacement::Tile => "tile",
    }
}

fn localized_error(key: &str, error: impl std::fmt::Display) -> String {
    localization::format(key, &FormatArgs::new().insert("error", error))
}

fn keyboard_setting_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("Staminator").join("settings.txt"))
}

fn load_keyboard_setting_from(path: Option<&std::path::Path>) -> bool {
    path.and_then(|path| fs::read_to_string(path).ok())
        .is_none_or(|value| value.trim() != "false")
}

fn save_keyboard_setting_to(path: Option<&std::path::Path>, show: bool) {
    if let Some(path) = path
        && let Some(parent) = path.parent()
        && fs::create_dir_all(parent).is_ok()
    {
        let _ = fs::write(path, show.to_string());
    }
}

#[derive(Default)]
struct KeyboardInputQueue {
    events: Vec<egui::Event>,
    close_requested: bool,
}

fn queue_keyboard_input(
    queue: &Arc<Mutex<KeyboardInputQueue>>,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    close_requested: bool,
) {
    let events: Vec<_> = events
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Copy
            )
        })
        .collect();
    if !events.is_empty() || close_requested {
        let mut pending = queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.events.extend(events);
        pending.close_requested |= close_requested;
        drop(pending);
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
}

fn sync_catalog_with_database(
    catalog: &mut ContentCatalog,
    database: &mut stamina_data::storage::Database,
) -> Result<(), stamina_data::storage::StorageError> {
    if !catalog.stored_lessons().is_empty() && catalog.lesson_diagnostics().is_empty() {
        database.import_lessons_once(catalog.stored_lessons())?;
        let builtin = catalog
            .stored_lessons()
            .iter()
            .filter(|(_, built_in)| *built_in)
            .map(|(lesson, _)| lesson.clone())
            .collect::<Vec<_>>();
        if !builtin.is_empty() {
            database.sync_builtin_lessons(&builtin)?;
        }
    }
    let lessons = database.list_lessons()?;
    if lessons.is_empty() && !catalog.lesson_diagnostics().is_empty() {
        return Ok(());
    }
    let mut groups = std::collections::HashMap::new();
    for (lesson, built_in) in &lessons {
        let key = format!("lesson_group:{}", lesson.id);
        let source_group = catalog.lesson_groups().get(&lesson.id).cloned();
        let saved_group = database.get_app_setting(&key)?;
        let group = if *built_in && source_group.is_some() {
            if source_group != saved_group {
                database.set_app_setting(&key, source_group.as_deref().unwrap_or_default())?;
            }
            source_group
        } else {
            match saved_group {
                Some(group) => Some(group),
                None => {
                    if let Some(group) = &source_group {
                        database.set_app_setting(&key, group)?;
                    }
                    source_group
                }
            }
        };
        if let Some(group) = group {
            groups.insert(lesson.id.clone(), group);
        }
    }
    catalog.set_lesson_groups(groups);
    catalog.set_stored_lessons(lessons);
    Ok(())
}

fn append_next_phrase(
    session: &mut Session,
    phrases: &[Exercise],
    sequence: &mut generated_modes::PhraseSequence,
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
    let next = sequence.next()?;
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
    const ALL: [Self; 5] = [
        Self::Lessons,
        Self::Phrases,
        Self::Letters,
        Self::AllKeys,
        Self::ExternalText,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Lessons => "mode.lessons",
            Self::Phrases => "mode.phrases",
            Self::Letters => "mode.letters",
            Self::AllKeys => "mode.all-keys",
            Self::ExternalText => "mode.external",
        }
    }
}

struct StaminatorApp {
    play_sound: fn(SoundEvent),
    selected_mode: TrainingMode,
    english_layout: bool,
    content_language: ContentLanguage,
    catalog: ContentCatalog,
    selected_content: [usize; 4],
    selected_lessons: [Option<String>; 2],
    chooser_open: bool,
    chooser_filter: String,
    chooser_preview: Option<usize>,
    lesson_editor: Option<content_editor::LessonEditor>,
    phrase_editor: Option<content_editor::PhraseEditor>,
    external_path: Option<PathBuf>,
    external_text: String,
    external_error: String,
    external_file_changed: bool,
    external_source: Option<external_text::ExternalFile>,
    external_reader: Option<external_text::ExternalTextStream<fs::File>>,
    external_positions: VecDeque<std::ops::Range<usize>>,
    external_start: usize,
    generation: u64,
    session: Option<Session>,
    session_text: String,
    phrase_sequence: Option<generated_modes::PhraseSequence>,
    started_at: Instant,
    last_result: Option<staminator_core::SessionStats>,
    show_keyboard: bool,
    keyboard_input: Arc<Mutex<KeyboardInputQueue>>,
    last_keyboard_state: Option<(keyboard::KeyboardLayout, Option<char>)>,
    keyboard_setting_path: Option<PathBuf>,
    result_open: bool,
    result_completed: bool,
    result_title: String,
    result_recorded: bool,
    users: UserStore,
    profiles_open: bool,
    progress_open: bool,
    profile_name_input: String,
    profile_error: String,
    lesson_transfer_error: String,
    background_open: bool,
    background_selection: Option<(bool, Option<PathBuf>)>,
    background_texture: Option<egui::TextureHandle>,
    background_error: String,
    background_files: Vec<PathBuf>,
}

impl StaminatorApp {
    fn sync_profile_language(&mut self) {
        localization::set_language(
            self.users
                .active_profile()
                .map_or("RU", |profile| profile.interface_language.as_str()),
        );
        self.catalog.refresh_diagnostics();
        self.content_language = if self
            .users
            .active_profile()
            .is_some_and(|profile| profile.content_language == "EN")
        {
            ContentLanguage::English
        } else {
            ContentLanguage::Russian
        };
    }
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        training_view::install_text_font(&cc.egui_ctx);
        let keyboard_setting_path = keyboard_setting_path();
        let mut show_keyboard = load_keyboard_setting_from(keyboard_setting_path.as_deref());
        let (users, load_error) = match UserStore::load() {
            Ok(users) => (users, None),
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
                (users, Some(error.to_string()))
            }
        };
        localization::set_language(
            users
                .active_profile()
                .map_or("RU", |profile| profile.interface_language.as_str()),
        );
        let mut profile_error = load_error
            .map(|error| localized_error("profile.load_error", error))
            .unwrap_or_default();
        let mut catalog = ContentCatalog::load();
        match users.database() {
            Ok(Some(mut database)) => {
                match database.get_app_setting("show_keyboard") {
                    Ok(Some(value)) => show_keyboard = value != "false",
                    Ok(None) => {
                        if let Err(error) =
                            database.set_app_setting("show_keyboard", &show_keyboard.to_string())
                        {
                            profile_error = localized_error(
                                "settings.save_error",
                                localization::storage_error(&error),
                            );
                        }
                    }
                    Err(error) => {
                        profile_error = localized_error(
                            "settings.load_error",
                            localization::storage_error(&error),
                        )
                    }
                }
                if let Err(error) = sync_catalog_with_database(&mut catalog, &mut database) {
                    profile_error =
                        localized_error("lessons.load_error", localization::storage_error(&error));
                }
            }
            Ok(None) => {}
            Err(error) => profile_error = localized_error("database.open_error", error),
        }
        let mut app = Self::from_parts(
            catalog,
            show_keyboard,
            keyboard_setting_path,
            users,
            profile_error,
            sound::play,
        );
        app.restore_external_text();
        app
    }

    fn from_parts(
        catalog: ContentCatalog,
        show_keyboard: bool,
        keyboard_setting_path: Option<PathBuf>,
        users: UserStore,
        profile_error: String,
        play_sound: fn(SoundEvent),
    ) -> Self {
        Self {
            play_sound,
            selected_mode: TrainingMode::Lessons,
            english_layout: false,
            content_language: users
                .active_profile()
                .map_or(ContentLanguage::Russian, |profile| {
                    if profile.content_language == "EN" {
                        ContentLanguage::English
                    } else {
                        ContentLanguage::Russian
                    }
                }),
            catalog,
            selected_content: [0; 4],
            selected_lessons: [None, None],
            chooser_open: false,
            chooser_filter: String::new(),
            chooser_preview: None,
            lesson_editor: None,
            phrase_editor: None,
            external_path: None,
            external_text: String::new(),
            external_error: String::new(),
            external_file_changed: false,
            external_source: None,
            external_reader: None,
            external_positions: VecDeque::new(),
            external_start: 0,
            generation: 0,
            session: None,
            session_text: String::new(),
            phrase_sequence: None,
            started_at: Instant::now(),
            last_result: None,
            show_keyboard,
            keyboard_input: Arc::default(),
            last_keyboard_state: None,
            keyboard_setting_path,
            result_open: false,
            result_completed: false,
            result_title: String::new(),
            result_recorded: false,
            users,
            profiles_open: false,
            progress_open: false,
            profile_name_input: String::new(),
            profile_error,
            lesson_transfer_error: String::new(),
            background_open: false,
            background_selection: None,
            background_texture: None,
            background_error: String::new(),
            background_files: Vec::new(),
        }
    }

    fn refresh_background(&mut self, ctx: &egui::Context) {
        let settings = self
            .users
            .background_settings()
            .cloned()
            .unwrap_or_default();
        let selection = (settings.use_builtin, settings.image_path);
        if self.background_selection.as_ref() != Some(&selection) {
            self.background_selection = Some(selection.clone());
            self.background_texture = None;
            self.background_error.clear();
            let loaded = if selection.0 {
                Some(background::load_builtin(ctx))
            } else {
                selection
                    .1
                    .as_deref()
                    .map(|path| background::load(ctx, path))
            };
            if let Some(result) = loaded {
                match result {
                    Ok(texture) => self.background_texture = Some(texture),
                    Err(error) => {
                        self.background_error =
                            format!("{}: {}", localization::tr("background.error"), error)
                    }
                }
            }
        }
    }

    fn show_background_window(&mut self, ctx: &egui::Context) {
        let mut open = self.background_open;
        let mut settings = self
            .users
            .background_settings()
            .cloned()
            .unwrap_or_default();
        let mut changed = false;
        egui::Window::new(localization::tr("background.title"))
            .open(&mut open)
            .show(ctx, |ui| {
                if ui
                    .selectable_label(settings.use_builtin, localization::tr("background.builtin"))
                    .clicked()
                {
                    settings.use_builtin = true;
                    settings.image_path = None;
                    changed = true;
                }
                if ui
                    .selectable_label(
                        !settings.use_builtin && settings.image_path.is_none(),
                        localization::tr("background.color"),
                    )
                    .clicked()
                {
                    settings.image_path = None;
                    settings.use_builtin = false;
                    changed = true;
                }
                if ui.button(localization::tr("background.choose")).clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("PNG / JPEG", &["png", "jpg", "jpeg"])
                        .pick_file()
                {
                    settings.image_path = Some(path);
                    settings.use_builtin = false;
                    changed = true;
                }
                if let Some(path) = &settings.image_path {
                    ui.label(path.display().to_string());
                }
                ui.horizontal(|ui| {
                    if ui.button(localization::tr("background.refresh")).clicked() {
                        let _ = fs::create_dir_all(background::directory());
                        self.background_files = background::images_in(&background::directory());
                        self.background_selection = None;
                    }
                    ui.small(background::directory().display().to_string());
                });
                for path in &self.background_files {
                    if ui
                        .selectable_label(
                            settings.image_path.as_ref() == Some(path),
                            path.file_name().unwrap_or_default().to_string_lossy(),
                        )
                        .clicked()
                    {
                        settings.image_path = Some(path.clone());
                        settings.use_builtin = false;
                        changed = true;
                    }
                }
                ui.horizontal(|ui| {
                    for (value, key) in [
                        (user_store::BackgroundPlacement::Fit, "background.fit"),
                        (user_store::BackgroundPlacement::Fill, "background.fill"),
                        (user_store::BackgroundPlacement::Tile, "background.tile"),
                    ] {
                        changed |= ui
                            .selectable_value(&mut settings.placement, value, localization::tr(key))
                            .changed();
                    }
                });
                changed |= ui
                    .add(
                        egui::Slider::new(&mut settings.dim_percent, 0..=100)
                            .text(localization::tr("background.dim")),
                    )
                    .changed();
                if !self.background_error.is_empty() {
                    ui.colored_label(egui::Color32::RED, &self.background_error);
                }
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(320.0, 120.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 0.0, egui::Color32::from_rgb(67, 76, 82));
                if let Some(texture) = &self.background_texture {
                    background::paint(
                        ui,
                        rect,
                        texture,
                        background_mode(settings.placement),
                        (u16::from(settings.dim_percent.min(100)) * 255 / 100) as u8,
                    );
                }
            });
        self.background_open = open;
        if changed {
            if let Err(error) = self.users.set_background_settings(settings) {
                self.profile_error = error.to_string();
            }
            self.refresh_background(ctx);
        }
    }

    fn persist_keyboard_setting(&mut self) {
        match self.users.database() {
            Ok(Some(mut database)) => {
                if let Err(error) =
                    database.set_app_setting("show_keyboard", &self.show_keyboard.to_string())
                {
                    self.profile_error =
                        localized_error("settings.save_error", localization::storage_error(&error));
                }
            }
            Ok(None) => {
                save_keyboard_setting_to(self.keyboard_setting_path.as_deref(), self.show_keyboard)
            }
            Err(error) => self.profile_error = localized_error("settings.save_error", error),
        }
    }

    fn take_keyboard_input(&mut self) -> Vec<egui::Event> {
        let (events, close_requested) = {
            let mut queue = self
                .keyboard_input
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                std::mem::take(&mut queue.events),
                std::mem::take(&mut queue.close_requested),
            )
        };
        if close_requested && self.show_keyboard {
            self.show_keyboard = false;
            self.persist_keyboard_setting();
        }
        events
    }

    fn language(&self) -> ContentLanguage {
        self.content_language
    }

    fn content_slot(&self) -> usize {
        usize::from(self.content_language == ContentLanguage::English) * 2
            + usize::from(self.selected_mode == TrainingMode::Phrases)
    }

    fn items(&self) -> &[Exercise] {
        match self.selected_mode {
            TrainingMode::Phrases => self.catalog.phrases(self.language()),
            _ => self.catalog.lessons(self.language()),
        }
    }

    fn current_index(&self) -> usize {
        if self.selected_mode == TrainingMode::Lessons {
            let language = usize::from(self.language() == ContentLanguage::English);
            self.selected_lessons[language]
                .as_deref()
                .and_then(|id| self.catalog.lesson_index(self.language(), id))
                .unwrap_or(0)
        } else {
            self.selected_content[self.content_slot()].min(self.items().len().saturating_sub(1))
        }
    }

    fn current_exercise(&self) -> Option<&Exercise> {
        self.items().get(self.current_index())
    }

    fn exercise(&self) -> &str {
        match self.selected_mode {
            TrainingMode::Lessons | TrainingMode::Phrases => {
                self.current_exercise().map_or("", |item| &item.text)
            }
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
            TrainingMode::Lessons | TrainingMode::Phrases => self.current_exercise().map_or_else(
                || localization::tr("mode.empty_text"),
                |item| item.name.clone(),
            ),
            TrainingMode::Letters if self.catalog.phrases(self.language()).is_empty() => {
                localization::tr("mode.empty_phrases")
            }
            TrainingMode::Letters => localization::tr("mode.letters"),
            TrainingMode::AllKeys if self.catalog.all_keys(self.english_layout).is_empty() => {
                localization::tr("mode.empty_keys")
            }
            TrainingMode::AllKeys => localization::tr("mode.all-keys"),
            TrainingMode::ExternalText => self
                .external_path
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| localization::tr("mode.external_unselected")),
        }
    }

    fn generated_text(&mut self) -> String {
        self.generation = self.generation.wrapping_add(1);
        let seed = self.now_ms().wrapping_add(self.generation);
        match self.selected_mode {
            TrainingMode::Letters => {
                let phrases = self.catalog.phrases(self.language());
                if phrases.is_empty() {
                    String::new()
                } else {
                    generated_modes::letters_from_phrases(phrases, seed)
                }
            }
            TrainingMode::AllKeys => {
                let keys = self.catalog.all_keys(self.english_layout);
                if keys.is_empty() {
                    String::new()
                } else {
                    generated_modes::all_keys(&keys, seed)
                }
            }
            _ => String::new(),
        }
    }

    fn choose_external_text(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title(localization::tr("file.open_training_text"))
            .pick_file()
        {
            self.load_external_text(path);
        }
    }

    fn load_external_text(&mut self, path: PathBuf) {
        self.external_file_changed = false;
        match external_text::ExternalFile::open(&path) {
            Ok(source) => {
                if source.char_count() == 0 {
                    self.external_error = localization::tr("file.no_training_text");
                } else {
                    self.external_path = Some(path);
                    self.external_source = Some(source);
                    self.external_start = 0;
                    self.external_error.clear();
                    self.external_file_changed = false;
                    self.session = None;
                    self.last_result = None;
                    self.result_open = false;
                    match self.prepare_external() {
                        Ok(()) => self.save_external_progress(),
                        Err(error) => {
                            self.external_error = localized_error("file.read_error", error)
                        }
                    }
                }
            }
            Err(error) => self.external_error = localized_error("file.open_error", error),
        }
    }

    fn restore_external_text(&mut self) {
        self.content_language = if self
            .users
            .active_profile()
            .is_some_and(|p| p.content_language == "EN")
        {
            ContentLanguage::English
        } else {
            ContentLanguage::Russian
        };
        self.external_path = None;
        self.external_text.clear();
        self.external_source = None;
        self.external_reader = None;
        self.external_positions.clear();
        self.external_start = 0;
        self.external_error.clear();
        self.external_file_changed = false;
        let Some(saved) = self
            .users
            .active_profile()
            .and_then(|p| p.external_text.clone())
        else {
            return;
        };
        match external_text::ExternalFile::open(&saved.path) {
            Ok(source)
                if source.file_id().size == saved.size
                    && source.file_id().content_hash == saved.hash
                    && saved.encoding == "Utf8" =>
            {
                if let Ok(index) = source.index_at_byte(saved.position) {
                    self.external_start = index;
                    self.external_path = Some(saved.path);
                    self.external_source = Some(source);
                    if let Err(error) = self.prepare_external() {
                        self.external_error = localized_error("file.read_error", error);
                    }
                } else {
                    self.external_error = localization::tr("file.invalid_position");
                }
            }
            Ok(_) => {
                self.external_error = localization::tr("file.changed");
                self.external_file_changed = true;
            }
            Err(error) => self.external_error = localized_error("file.saved_open_error", error),
        }
    }

    fn save_external_progress(&mut self) {
        let Some(progress) = self.current_external_progress() else {
            return;
        };
        if let Err(error) = self.users.set_external_progress(progress) {
            self.external_error = localized_error("file.save_position_error", error);
        } else if let Err(error) = self.prepare_external() {
            self.external_error = localized_error("file.read_error", error);
        }
    }

    fn current_external_progress(&mut self) -> Option<user_store::ExternalProgress> {
        if self.selected_mode != TrainingMode::ExternalText {
            return None;
        }
        let Some(source) = &self.external_source else {
            return None;
        };
        let id = source.file_id();
        let position = self
            .external_positions
            .front()
            .map_or(id.size as usize, |range| range.start);
        match external_text::ExternalFile::open(id.path()) {
            Ok(current) if current.file_id() == id => {}
            Ok(_) => {
                self.external_error = localization::tr("file.changed");
                self.external_file_changed = true;
                return None;
            }
            Err(error) => {
                self.external_error = localized_error("file.validate_error", error);
                return None;
            }
        }
        Some(user_store::ExternalProgress {
            path: id.path().to_path_buf(),
            size: id.size,
            hash: id.content_hash,
            encoding: "Utf8".into(),
            position,
        })
    }

    fn set_external_position(&mut self, index: usize) {
        let Some(source) = &self.external_source else {
            return;
        };
        self.external_start = index.min(source.char_count());
        self.session = None;
        self.last_result = None;
        self.result_open = false;
        match self.prepare_external() {
            Ok(()) => self.save_external_progress(),
            Err(error) => self.external_error = localized_error("file.read_error", error),
        }
    }

    fn prepare_external(&mut self) -> std::io::Result<()> {
        self.external_reader = None;
        self.external_positions.clear();
        self.external_text.clear();
        let source = self
            .external_source
            .as_ref()
            .expect("external source exists");
        let mut reader = source.stream_at_index(self.external_start)?;
        let fragment = reader.next_fragment(512)?;
        if let Some(fragment) = fragment {
            self.external_text = fragment.text;
            self.external_positions.extend(fragment.byte_positions);
        }
        self.external_reader = Some(reader);
        Ok(())
    }

    fn select_content(&mut self, index: usize) {
        let index = index.min(self.items().len().saturating_sub(1));
        if self.selected_mode == TrainingMode::Lessons {
            let language = usize::from(self.language() == ContentLanguage::English);
            self.selected_lessons[language] = self
                .catalog
                .lesson_id(self.language(), index)
                .map(str::to_owned);
        } else {
            self.selected_content[self.content_slot()] = index;
        }
        self.session = None;
        self.last_result = None;
        self.result_open = false;
    }

    fn advance_content(&mut self) -> bool {
        let next = self.current_index() + 1;
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
        if self.selected_mode == TrainingMode::Phrases {
            self.generation = self.generation.wrapping_add(1);
            let slot = self.content_slot();
            self.phrase_sequence = Some(generated_modes::PhraseSequence::new(
                self.catalog.phrases(self.language()).len(),
                self.selected_content[slot],
                self.now_ms().wrapping_add(self.generation),
            ));
        } else {
            self.phrase_sequence = None;
        }
        self.session_text = match self.selected_mode {
            TrainingMode::Letters | TrainingMode::AllKeys => self.generated_text(),
            _ => self.exercise().to_owned(),
        };
        if self.session_text.is_empty() {
            return;
        }
        self.session = Some(Session::new(self.session_text.clone()));
        self.last_result = None;
        self.result_open = false;
        self.result_completed = false;
        self.result_recorded = false;
        (self.play_sound)(SoundEvent::Start);
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
    }

    fn check_time_limit(&mut self, now: u64) {
        if self.selected_mode != TrainingMode::Lessons
            && self
                .session
                .as_mut()
                .is_some_and(|session| session.check_time_limit(now, 300_000))
        {
            self.open_stopped_result();
        }
    }

    fn open_stopped_result(&mut self) {
        self.last_result = self.session.as_ref().map(Session::stats);
        self.result_completed = self
            .session
            .as_ref()
            .is_some_and(|s| s.remaining_chars() == 0);
        self.result_title = self.session_result_title();
        self.result_open = true;
    }

    fn accept_result(&mut self) -> bool {
        let progress = if !self.result_recorded {
            self.current_external_progress()
        } else {
            None
        };
        let refresh_external = progress.is_some();
        self.record_current_result(progress);
        if !self.result_recorded {
            return false;
        }
        if refresh_external && let Err(error) = self.prepare_external() {
            self.external_error = localized_error("file.read_error", error);
        }
        self.result_open = false;
        true
    }

    fn record_current_result(&mut self, progress: Option<user_store::ExternalProgress>) {
        if self.result_recorded {
            return;
        }
        let Some(stats) = &self.last_result else {
            return;
        };
        if stats.correct == 0 && stats.errors == 0 {
            if let Some(progress) = progress
                && let Err(error) = self.users.set_external_progress(progress)
            {
                self.profile_error = localized_error("file.save_position_error", error);
                return;
            }
            self.result_recorded = true;
            return;
        }
        let mode = self.selected_mode.key();
        let layout = if self.english_layout {
            "layout.en"
        } else {
            "layout.ru"
        };
        if let Err(error) = self.users.record_with_progress(
            user_store::ResultInput {
                mode,
                layout,
                exercise: &self.result_title,
                correct: stats.correct as u64,
                errors: stats.errors as u64,
                elapsed_ms: stats.elapsed_ms,
                cpm: stats.cpm as f64,
                error_percent: stats.error_percent,
            },
            progress,
            match self.session.as_ref().and_then(Session::stop_reason) {
                Some(StopReason::Manual) => "manual",
                Some(StopReason::TimeLimit) => "time_limit",
                Some(StopReason::Completed) => "completed",
                None => "unknown",
            },
        ) {
            self.profile_error = localized_error("result.save_error", error);
        } else {
            self.result_recorded = true;
        }
    }

    fn session_result_title(&self) -> String {
        if self.selected_mode == TrainingMode::Phrases {
            localization::tr("mode.phrases")
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
            if let Some(session) = &mut self.session
                && let Some(sequence) = &mut self.phrase_sequence
                && let Some(next_text) = append_next_phrase(
                    session,
                    phrases,
                    sequence,
                    &mut self.selected_content[slot],
                    ch,
                    now,
                )
            {
                self.session_text.push_str(&next_text);
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
        } else if at_boundary
            && self.selected_mode == TrainingMode::ExternalText
            && let Some(reader) = &mut self.external_reader
        {
            match reader.next_fragment(512) {
                Ok(Some(fragment)) => {
                    if let Some(session) = &mut self.session
                        && session.append_expected(&fragment.text)
                    {
                        self.session_text.push_str(&fragment.text);
                        self.external_positions.extend(fragment.byte_positions);
                    }
                }
                Ok(None) => {}
                Err(error) => self.external_error = localized_error("file.read_error", error),
            }
        }
        if let Some(session) = &mut self.session {
            match session.type_char(ch, now) {
                Keystroke::Incorrect => (self.play_sound)(SoundEvent::Error),
                Keystroke::Correct
                    if self.selected_mode == TrainingMode::ExternalText
                        && self.external_positions.pop_front().is_some() =>
                {
                    self.external_start += 1;
                }
                _ => {}
            }
        }
        if let Some(session) = &mut self.session
            && session.retain_recent(128) > 0
        {
            self.session_text = session.expected_text();
        }
    }

    fn toggle_pause(&mut self) {
        let now = self.now_ms();
        if let Some(session) = &mut self.session {
            match session.snapshot(now).state {
                SessionState::Ready | SessionState::Running => {
                    session.pause(now);
                    (self.play_sound)(SoundEvent::Pause);
                }
                SessionState::Paused => {
                    session.resume(now);
                    (self.play_sound)(SoundEvent::Start);
                }
                _ => {}
            }
        }
    }

    #[cfg(test)]
    fn handle_input(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|input| input.events.clone());
        self.handle_input_events(ctx, events);
    }

    fn handle_input_events(&mut self, _ctx: &egui::Context, events: Vec<egui::Event>) {
        if self.result_open
            || self.background_open
            || self.chooser_open
            || self.profiles_open
            || self.lesson_editor.is_some()
            || self.phrase_editor.is_some()
        {
            return;
        }
        let now = self.now_ms();
        self.check_time_limit(now);
        if self.result_open {
            return;
        }
        let mut consume_space_text = false;
        for event in events {
            match event {
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    repeat: false,
                    ..
                } => {
                    self.stop();
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
                    key: egui::Key::Space | egui::Key::Enter,
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
                            (self.play_sound)(SoundEvent::Start);
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
                    if self
                        .session
                        .as_ref()
                        .is_some_and(|s| s.snapshot(now).state == SessionState::Paused)
                    {
                        self.session.as_mut().unwrap().resume(now);
                        (self.play_sound)(SoundEvent::Start);
                        continue;
                    }
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
            self.open_stopped_result();
        }
    }

    fn import_lesson_from(&mut self, path: &std::path::Path) -> Result<(), String> {
        let lesson =
            storage_transfer::import_json_lesson(path).map_err(|error| error.to_string())?;
        let mut editor = content_editor::LessonEditor::new(&lesson.language);
        editor.lesson = lesson;
        self.lesson_editor = Some(editor);
        self.chooser_open = false;
        self.lesson_transfer_error.clear();
        Ok(())
    }

    fn export_lesson_to(&self, index: usize, path: &std::path::Path) -> Result<(), String> {
        let lesson = self
            .catalog
            .lesson_id(self.language(), index)
            .and_then(|id| self.catalog.lesson_document(id))
            .map(|(lesson, _)| lesson)
            .ok_or_else(|| localization::tr("lesson.unselected"))?;
        storage_transfer::export_json_lesson(lesson, path).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn export_backup_to(&self, path: &std::path::Path) -> Result<(), String> {
        let database = self
            .users
            .database()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| localization::tr("database.unavailable"))?;
        storage_transfer::export_database_backup(&database, path)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn show_content_chooser(&mut self, ctx: &egui::Context) {
        let mut open = self.chooser_open;
        let mut chosen = None;
        let mut refresh = false;
        let mut create = false;
        let mut edit = false;
        let mut add_phrases = false;
        let mut import_lesson = false;
        let mut export_lesson = false;
        let mut clear_error = false;
        let filter = self.chooser_filter.trim().to_lowercase();
        let matches: Vec<_> = self
            .items()
            .iter()
            .enumerate()
            .filter(|(_, item)| filter.is_empty() || item.name.to_lowercase().contains(&filter))
            .map(|(index, item)| (index, item.name.chars().take(85).collect::<String>()))
            .collect();
        let current = self
            .chooser_preview
            .or_else(|| Some(self.current_index()))
            .filter(|index| matches.iter().any(|(candidate, _)| candidate == index))
            .or_else(|| matches.first().map(|(index, _)| *index));
        egui::Window::new(if self.selected_mode == TrainingMode::Lessons {
            localization::tr("chooser.lessons_title")
        } else {
            localization::tr("chooser.phrases_title")
        })
        .open(&mut open)
        .default_size([620.0, 470.0])
        .show(ctx, |ui| {
            if self.selected_mode == TrainingMode::Lessons {
                if !self.lesson_transfer_error.is_empty() {
                    ui.colored_label(egui::Color32::RED, &self.lesson_transfer_error);
                    clear_error = ui.button(localization::tr("clear-message")).clicked();
                }
                ui.horizontal(|ui| {
                    refresh = ui.button(localization::tr("refresh-lessons")).clicked();
                    create = ui.button(localization::tr("create-lesson")).clicked();
                    edit = ui
                        .add_enabled(
                            current.is_some(),
                            egui::Button::new(localization::tr("edit-lesson")),
                        )
                        .clicked();
                });
                ui.horizontal(|ui| {
                    import_lesson = ui.button(localization::tr("lesson-import")).clicked();
                    export_lesson = ui
                        .add_enabled(
                            current.is_some(),
                            egui::Button::new(localization::tr("lesson-export")),
                        )
                        .clicked();
                });
                if let Some(path) = self.catalog.user_lesson_path() {
                    ui.label(localization::format(
                        "chooser.user_folder",
                        &FormatArgs::new().insert("path", path.display()),
                    ));
                }
                if !self.catalog.lesson_diagnostics().is_empty() {
                    ui.colored_label(egui::Color32::RED, self.catalog.lesson_diagnostics());
                }
            }
            if self.selected_mode == TrainingMode::Phrases {
                add_phrases = ui.button(localization::tr("add-phrases")).clicked();
            }
            ui.text_edit_singleline(&mut self.chooser_filter);
            ui.label(localization::format(
                "chooser.matches",
                &FormatArgs::new().count(matches.len() as i64),
            ));
            ui.columns(2, |columns| {
                egui::ScrollArea::vertical()
                    .id_salt("chooser_list")
                    .max_height(300.0)
                    .show_rows(&mut columns[0], 24.0, matches.len(), |ui, range| {
                        for (index, name) in &matches[range] {
                            let response = ui.selectable_label(
                                Some(*index) == current,
                                format!("{}. {}", index + 1, name),
                            );
                            if response.double_clicked() {
                                chosen = Some(*index);
                            } else if response.clicked() {
                                self.chooser_preview = Some(*index);
                            }
                        }
                    });
                if let Some((current, item)) =
                    current.and_then(|index| self.items().get(index).map(|item| (index, item)))
                {
                    columns[1].heading(&item.name);
                    if self.selected_mode == TrainingMode::Lessons {
                        if let Some((lesson, _)) = self
                            .catalog
                            .lesson_id(self.language(), current)
                            .and_then(|id| self.catalog.lesson_document(id))
                        {
                            if let Some(description) = &lesson.description {
                                columns[1].label(description);
                            }
                        }
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("chooser_preview")
                        .max_height(240.0)
                        .show(&mut columns[1], |ui| {
                            ui.label(&item.text);
                        });
                    if columns[1].button(localization::tr("choose")).clicked() {
                        chosen = Some(current);
                    }
                }
            });
        });
        self.chooser_open = open;
        if clear_error {
            self.lesson_transfer_error.clear();
        }
        if import_lesson
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("JSON", &["json"])
                .pick_file()
        {
            match self.import_lesson_from(&path) {
                Ok(()) => self.lesson_transfer_error.clear(),
                Err(error) => {
                    self.lesson_transfer_error = localized_error("lesson.import_error", error)
                }
            }
        }
        if export_lesson
            && let Some(current) = current
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("JSON", &["json"])
                .set_file_name("lesson.json")
                .save_file()
        {
            match self.export_lesson_to(current, &path) {
                Ok(()) => self.lesson_transfer_error.clear(),
                Err(error) => {
                    self.lesson_transfer_error = localized_error("lesson.export_error", error)
                }
            }
        }
        if create {
            self.lesson_editor = Some(content_editor::LessonEditor::new(
                if self.language() == ContentLanguage::English {
                    "en"
                } else {
                    "ru"
                },
            ));
            self.chooser_open = false;
        }
        if edit && let Some(current) = current {
            let id = self.catalog.lesson_id(self.language(), current);
            if let Ok(Some(_)) = self.users.database() {
                let mut editor = content_editor::LessonEditor::new(
                    if self.language() == ContentLanguage::English {
                        "en"
                    } else {
                        "ru"
                    },
                );
                match id.and_then(|id| self.catalog.lesson_document(id)) {
                    Some((lesson, _)) => {
                        editor.lesson = lesson.clone();
                        editor.path = Some(PathBuf::from(&lesson.id));
                    }
                    None => editor.error = localization::tr("lesson.choose_first"),
                }
                self.lesson_editor = Some(editor);
                self.chooser_open = false;
                return;
            }
            let item = stamina_data::load_lesson_catalog(self.catalog.lesson_roots())
                .lessons
                .into_iter()
                .find(|item| Some(item.lesson.id.as_str()) == id);
            let mut editor =
                content_editor::LessonEditor::new(if self.language() == ContentLanguage::English {
                    "en"
                } else {
                    "ru"
                });
            if let Some(item) = item {
                if self
                    .catalog
                    .user_lesson_path()
                    .is_some_and(|root| item.path.starts_with(root))
                {
                    editor.lesson = item.lesson;
                    editor.path = Some(item.path);
                } else {
                    editor.error = localization::tr("lesson.builtin_edit");
                }
            } else {
                editor.error = localization::tr("lesson.choose_first");
            }
            self.lesson_editor = Some(editor);
            self.chooser_open = false;
        }
        if add_phrases {
            self.phrase_editor = Some(content_editor::PhraseEditor::default());
            self.chooser_open = false;
        }
        if refresh {
            for (index, language) in [ContentLanguage::Russian, ContentLanguage::English]
                .into_iter()
                .enumerate()
            {
                if self.selected_lessons[index].is_none() {
                    self.selected_lessons[index] =
                        self.catalog.lesson_id(language, 0).map(str::to_owned);
                }
            }
            match self.users.database() {
                Ok(Some(mut database)) => {
                    let mut refreshed_catalog = self.catalog.clone();
                    refreshed_catalog.refresh_lessons();
                    match sync_catalog_with_database(&mut refreshed_catalog, &mut database) {
                        Ok(()) => {
                            self.catalog = refreshed_catalog;
                            self.lesson_transfer_error.clear();
                        }
                        Err(error) => {
                            self.lesson_transfer_error = localized_error(
                                "lessons.load_error",
                                localization::storage_error(&error),
                            );
                        }
                    }
                }
                Ok(None) => {
                    self.catalog.refresh_lessons();
                    self.lesson_transfer_error.clear();
                }
                Err(error) => {
                    self.lesson_transfer_error = localized_error("database.open_error", error);
                }
            }
            self.session = None;
            self.last_result = None;
            self.result_open = false;
        }
        if let Some(index) = chosen {
            self.select_content(index);
            self.chooser_open = false;
            self.chooser_filter.clear();
            self.chooser_preview = None;
        }
    }

    fn show_content_editors(&mut self, ctx: &egui::Context) {
        if let Some(mut editor) = self.lesson_editor.take() {
            let mut open = true;
            if editor.show(ctx, &mut open) {
                if !["ru", "en"].iter().any(|lang| {
                    editor
                        .lesson
                        .language
                        .split('-')
                        .next()
                        .is_some_and(|value| value.eq_ignore_ascii_case(lang))
                }) {
                    editor.error = localization::tr("lesson.language_error");
                    self.lesson_editor = Some(editor);
                    return;
                }
                let database = match self.users.database() {
                    Ok(database) => database,
                    Err(error) => {
                        editor.error = localized_error("database.open_error", error);
                        self.lesson_editor = Some(editor);
                        return;
                    }
                };
                let result = if let Some(mut database) = database {
                    stamina_data::validate_json_lesson_typed(&editor.lesson)
                        .map_err(|e| localization::lesson_validation(&e))
                        .and_then(|()| {
                            let old_id = editor.path.as_ref().and_then(|p| p.to_str());
                            if old_id.is_some_and(|id| id != editor.lesson.id) {
                                return Err(localization::tr("lesson.id_change_error"));
                            }
                            match database
                                .lesson(&editor.lesson.id)
                                .map_err(|e| localization::storage_error(&e))?
                            {
                                Some(_) if old_id.is_none() => {
                                    return Err(localization::tr("lesson.id_exists"));
                                }
                                None if old_id.is_some() => {
                                    return Err(localization::tr("lesson.not_found"));
                                }
                                _ => {}
                            }
                            if let Some(id) = old_id {
                                if !database
                                    .update_user_lesson(id, &editor.lesson)
                                    .map_err(|e| localization::storage_error(&e))?
                                {
                                    return Err(localization::tr("lesson.not_editable"));
                                }
                                Ok(())
                            } else {
                                database
                                    .create_user_lesson(&editor.lesson)
                                    .map_err(|e| localization::storage_error(&e))
                            }
                        })
                        .map(|()| PathBuf::new())
                } else if let Some(root) = self.catalog.user_lesson_path() {
                    if let Some(path) = &editor.path {
                        stamina_data::edit_user_lesson(
                            root,
                            path,
                            self.catalog.lesson_roots(),
                            &editor.lesson,
                        )
                        .map_err(|e| localization::lesson_write_error(&e))
                    } else {
                        stamina_data::create_user_lesson(
                            root,
                            self.catalog.lesson_roots(),
                            &editor.lesson,
                        )
                        .map_err(|e| localization::lesson_write_error(&e))
                    }
                } else {
                    editor.error = localization::tr("lesson.user_folder_missing");
                    self.lesson_editor = Some(editor);
                    return;
                };
                match result {
                    Ok(_) => {
                        let language = if editor
                            .lesson
                            .language
                            .split('-')
                            .next()
                            .is_some_and(|v| v.eq_ignore_ascii_case("en"))
                        {
                            ContentLanguage::English
                        } else {
                            ContentLanguage::Russian
                        };
                        if let Ok(Some(database)) = self.users.database() {
                            match database.list_lessons() {
                                Ok(lessons) => self.catalog.set_stored_lessons(lessons),
                                Err(error) => {
                                    self.profile_error = localized_error(
                                        "lessons.load_error",
                                        localization::storage_error(&error),
                                    )
                                }
                            }
                        } else {
                            self.catalog.refresh_lessons();
                        }
                        self.content_language = language;
                        self.selected_mode = TrainingMode::Lessons;
                        self.selected_lessons[usize::from(language == ContentLanguage::English)] =
                            Some(editor.lesson.id.clone());
                        self.session = None;
                        self.last_result = None;
                        open = false;
                    }
                    Err(error) => editor.error = localized_error("lesson.save_error", error),
                }
            }
            if open {
                self.lesson_editor = Some(editor);
            }
        }
        if let Some(mut editor) = self.phrase_editor.take() {
            let mut open = true;
            if editor.show(ctx, &mut open) {
                let phrases = editor.parsed_phrases();
                match self.catalog.add_user_phrases(self.language(), &phrases) {
                    Ok(_) => {
                        self.session = None;
                        self.last_result = None;
                        open = false;
                    }
                    Err(error) => editor.error = localized_error("phrases.save_error", error),
                }
            }
            if open {
                self.phrase_editor = Some(editor);
            }
        }
    }

    fn import_profile_from(&mut self, path: &std::path::Path) -> Result<(), String> {
        let profile = storage_transfer::import_profile_json(path).map_err(|e| e.to_string())?;
        let value = serde_json::to_value(profile).map_err(|e| e.to_string())?;
        self.users
            .import_profile(serde_json::from_value(value).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        self.sync_profile_language();
        self.restore_external_text();
        self.profile_error.clear();
        Ok(())
    }

    fn export_profile_to(&self, path: &std::path::Path) -> Result<(), String> {
        let profile = self
            .users
            .active_profile()
            .ok_or_else(|| localization::tr("profile.unselected"))?;
        let value = serde_json::to_value(profile).map_err(|e| e.to_string())?;
        let profile = serde_json::from_value(value).map_err(|e| e.to_string())?;
        storage_transfer::export_profile_json(&profile, path).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn restore_backup_from(&mut self, path: &std::path::Path) -> Result<PathBuf, String> {
        let safety = self.users.restore_backup(path).map_err(|e| e.to_string())?;
        self.sync_profile_language();
        self.session = None;
        self.last_result = None;
        self.result_open = false;
        self.restore_external_text();
        let database = self
            .users
            .database()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| localization::tr("database.unavailable"))?;
        let lessons = database
            .list_lessons()
            .map_err(|e| localization::storage_error(&e))?;
        let mut groups = std::collections::HashMap::new();
        for (lesson, _) in &lessons {
            if let Some(group) = database
                .get_app_setting(&format!("lesson_group:{}", lesson.id))
                .map_err(|e| localization::storage_error(&e))?
            {
                groups.insert(lesson.id.clone(), group);
            }
        }
        self.catalog.set_lesson_groups(groups);
        self.catalog.set_stored_lessons(lessons);
        self.show_keyboard = database
            .get_app_setting("show_keyboard")
            .map_err(|e| localization::storage_error(&e))?
            .is_none_or(|value| value != "false");
        Ok(safety)
    }

    fn show_profiles(&mut self, ctx: &egui::Context) {
        let mut open = self.profiles_open;
        egui::Window::new(localization::tr("users"))
            .open(&mut open)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.heading(localization::tr("profiles"));
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
                                self.sync_profile_language();
                                self.restore_external_text();
                            }
                            Err(error) => self.profile_error = error.to_string(),
                        }
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(localization::tr("profile-import")).clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("JSON", &["json"])
                            .pick_file()
                        && let Err(error) = self.import_profile_from(&path)
                    {
                        self.profile_error = localized_error("profile.import_error", error);
                    }
                    if ui.button(localization::tr("profile-export")).clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("JSON", &["json"])
                            .set_file_name("profile.json")
                            .save_file()
                        && let Err(error) = self.export_profile_to(&path)
                    {
                        self.profile_error = localized_error("profile.export_error", error);
                    }
                });
                ui.separator();
                ui.label(localization::tr("profile-name"));
                ui.text_edit_singleline(&mut self.profile_name_input);
                ui.horizontal(|ui| {
                    if ui.button(localization::tr("create")).clicked() {
                        match self.users.create(&self.profile_name_input) {
                            Ok(_) => {
                                self.profile_name_input.clear();
                                self.profile_error.clear();
                                self.session = None;
                                self.sync_profile_language();
                                self.restore_external_text();
                            }
                            Err(error) => self.profile_error = error.to_string(),
                        }
                    }
                    if ui.button(localization::tr("rename")).clicked() {
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
                            egui::Button::new(localization::tr("delete-current")),
                        )
                        .clicked()
                    {
                        let id = self.users.active_profile_id().to_owned();
                        match self.users.delete(&id) {
                            Ok(()) => {
                                self.profile_error.clear();
                                self.session = None;
                                self.sync_profile_language();
                                self.restore_external_text();
                            }
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

impl StaminatorApp {
    fn draw(&mut self, root: &mut egui::Ui) {
        localization::set_language(
            self.users
                .active_profile()
                .map_or("RU", |p| p.interface_language.as_str()),
        );
        let mut events = root.ctx().input(|input| input.events.clone());
        events.extend(self.take_keyboard_input());
        self.handle_input_events(root.ctx(), events);
        let now = self.now_ms();
        let snapshot = self.session.as_ref().map(|s| s.snapshot(now));
        let active = snapshot.as_ref().is_some_and(|s| {
            matches!(
                s.state,
                SessionState::Ready | SessionState::Running | SessionState::Paused
            )
        }) || self.result_open;
        let position = self.session.as_ref().map_or(0, Session::retained_position);
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
            self.current_index()
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
                if ui
                    .button(localization::tr(if active { "stop" } else { "start" }))
                    .clicked()
                {
                    if active {
                        self.stop();
                    } else {
                        self.start();
                    }
                }
                ui.menu_button(localization::tr("mode"), |ui| {
                    ui.add_enabled_ui(!active, |ui| {
                        for mode in TrainingMode::ALL {
                            if ui
                                .add(
                                    egui::Button::new(localization::tr(mode.key()))
                                        .selected(self.selected_mode == mode),
                                )
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
                        ui.menu_button(localization::tr("content-language"), |ui| {
                            let russian = ui
                                .selectable_value(
                                    &mut self.content_language,
                                    ContentLanguage::Russian,
                                    localization::tr("language.russian"),
                                )
                                .clicked();
                            let english = ui
                                .selectable_value(
                                    &mut self.content_language,
                                    ContentLanguage::English,
                                    "English",
                                )
                                .clicked();
                            if russian || english {
                                self.chooser_preview = None;
                                let value = if self.content_language == ContentLanguage::English {
                                    "EN"
                                } else {
                                    "RU"
                                };
                                if let Err(error) = self.users.set_content_language(value) {
                                    self.profile_error = error.to_string();
                                }
                                self.session = None;
                                self.last_result = None;
                                self.result_open = false;
                                ui.close();
                            }
                        });
                        ui.menu_button(localization::tr("layout"), |ui| {
                            let russian = ui
                                .selectable_value(
                                    &mut self.english_layout,
                                    false,
                                    localization::tr("layout.russian"),
                                )
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
                if self.content_mode() {
                    if ui
                        .add_enabled(!active, egui::Button::new(localization::tr("exercise")))
                        .clicked()
                    {
                        self.chooser_open = true;
                        self.chooser_filter.clear();
                        self.chooser_preview = None;
                    }
                } else {
                    ui.menu_button(localization::tr("exercise"), |ui| {
                        ui.add_enabled_ui(!active, |ui| {
                            if self.selected_mode == TrainingMode::ExternalText {
                                if ui.button(localization::tr("open-text")).clicked() {
                                    self.choose_external_text();
                                    ui.close();
                                }
                                if self.external_file_changed {
                                    if ui
                                        .button(localization::tr("file.restart_changed"))
                                        .clicked()
                                        && let Some(path) = self
                                            .users
                                            .active_profile()
                                            .and_then(|p| p.external_text.as_ref())
                                            .map(|p| p.path.clone())
                                    {
                                        self.load_external_text(path);
                                        ui.close();
                                    }
                                } else if let Some(source) = &self.external_source {
                                    let mut position = self.external_start;
                                    let length = source.char_count();
                                    if ui
                                        .add(
                                            egui::Slider::new(&mut position, 0..=length)
                                                .text(localization::tr("position")),
                                        )
                                        .changed()
                                    {
                                        self.set_external_position(position);
                                    }
                                }
                                if !self.external_error.is_empty() {
                                    ui.colored_label(egui::Color32::RED, &self.external_error);
                                }
                            } else {
                                ui.label(localization::tr("generated-on-start"));
                            }
                        });
                    });
                }
                if ui.button(localization::tr("progress")).clicked() {
                    self.progress_open = true;
                }
                ui.menu_button(localization::tr("options"), |ui| {
                    ui.menu_button(localization::tr("interface-language"), |ui| {
                        let language = self
                            .users
                            .active_profile()
                            .map_or("RU", |p| p.interface_language.as_str())
                            .to_owned();
                        for (code, name) in [
                            ("RU", localization::tr("language.russian")),
                            ("EN", "English".to_owned()),
                        ] {
                            if ui.selectable_label(language == code, name).clicked() {
                                match self.users.set_interface_language(code) {
                                    Ok(()) => {
                                        localization::set_language(code);
                                        self.catalog.refresh_diagnostics();
                                    }
                                    Err(error) => self.profile_error = error.to_string(),
                                }
                                ui.close();
                            }
                        }
                    });
                    if ui.button(localization::tr("background.title")).clicked() {
                        self.background_open = true;
                        let _ = fs::create_dir_all(background::directory());
                        self.background_files = background::images_in(&background::directory());
                        ui.close();
                    }
                    if ui.button(localization::tr("backup-create")).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("SQLite", &["sqlite3"])
                            .set_file_name("staminator-backup.sqlite3")
                            .save_file()
                            && let Err(error) = self.export_backup_to(&path)
                        {
                            self.profile_error = localized_error("backup.create_error", error);
                        }
                        ui.close();
                    }
                    if ui.button(localization::tr("backup-check")).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("SQLite", &["sqlite3"])
                            .pick_file()
                        {
                            match storage_transfer::prepare_database_restore(path) {
                                Ok(()) => self.profile_error = localization::tr("backup.checked"),
                                Err(error) => {
                                    self.profile_error = localized_error("backup.error", error)
                                }
                            }
                        }
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            !active,
                            egui::Button::new(localization::tr("backup-restore")),
                        )
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("SQLite", &["sqlite3"])
                            .pick_file()
                        {
                            let confirmed =
                                storage_transfer::prepare_database_restore(&path).map(|()| {
                                    let answer = rfd::MessageDialog::new()
                                        .set_title(localization::tr("backup.restore_title"))
                                        .set_description(localization::tr("backup.restore_confirm"))
                                        .set_buttons(rfd::MessageButtons::YesNo)
                                        .show();
                                    answer == rfd::MessageDialogResult::Yes
                                });
                            match confirmed {
                                Ok(true) => match self.restore_backup_from(&path) {
                                    Ok(safety) => {
                                        self.profile_error = localization::format(
                                            "backup.restored",
                                            &FormatArgs::new().insert("path", safety.display()),
                                        );
                                    }
                                    Err(error) => {
                                        self.profile_error =
                                            localized_error("backup.restore_error", error)
                                    }
                                },
                                Ok(false) => {}
                                Err(error) => {
                                    self.profile_error = localized_error("backup.error", error)
                                }
                            }
                        }
                        ui.close();
                    }
                    if ui
                        .checkbox(&mut self.show_keyboard, localization::tr("show-keyboard"))
                        .changed()
                    {
                        self.persist_keyboard_setting();
                    }
                    ui.separator();
                    ui.label(localization::format(
                        "content.label",
                        &FormatArgs::new().insert("label", &current_label),
                    ));
                    if self.catalog.diagnostics().is_empty() {
                        ui.small(localization::tr("data.loaded"));
                    } else {
                        ui.small(self.catalog.diagnostics());
                    }
                    if !self.profile_error.is_empty() {
                        ui.colored_label(egui::Color32::RED, &self.profile_error);
                    }
                });
                if ui
                    .add_enabled(!active, egui::Button::new(localization::tr("users")))
                    .clicked()
                {
                    self.profiles_open = true;
                }
                ui.menu_button(localization::tr("help"), |ui| {
                    ui.label(localization::tr("app-description"));
                });
                ui.separator();
                ui.label(format!(
                    "{} / {} · {}",
                    if content_count == 0 {
                        0
                    } else {
                        current_index + 1
                    },
                    content_count,
                    current_label.chars().take(44).collect::<String>()
                ));
            });
        });

        self.refresh_background(root.ctx());
        egui::Panel::bottom("metrics").show(root, |ui| {
            ui.horizontal(|ui| {
                if !self.background_error.is_empty() {
                    ui.colored_label(egui::Color32::RED, &self.background_error);
                    ui.separator();
                }
                ui.label(if active {
                    if snapshot
                        .as_ref()
                        .is_some_and(|s| s.state == SessionState::Paused)
                    {
                        localization::tr("status.paused")
                    } else {
                        localization::tr("status.typing")
                    }
                } else if snapshot.is_some() {
                    localization::tr("status.completed")
                } else {
                    localization::tr("status.waiting")
                });
                ui.separator();
                if let Some(stats) = &stats {
                    ui.label(localization::format(
                        "metric.speed",
                        &FormatArgs::new().insert("value", stats.cpm),
                    ));
                    ui.separator();
                    ui.label(localization::format(
                        "metric.errors",
                        &FormatArgs::new()
                            .count(stats.errors as i64)
                            .insert("percent", format!("{:.1}", stats.error_percent)),
                    ));
                    ui.separator();
                    ui.label(localization::format(
                        "metric.time",
                        &FormatArgs::new()
                            .insert("minutes", stats.elapsed_ms / 60_000)
                            .insert("seconds", format!("{:02}", (stats.elapsed_ms / 1_000) % 60)),
                    ));
                } else {
                    ui.label(localization::tr("metric.empty"));
                }
            });
        });

        let background_settings = self
            .users
            .background_settings()
            .cloned()
            .unwrap_or_default();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| {
                training_view::draw_training(
                    ui,
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
                        Some(SessionState::Paused) => Some(localization::tr("hint.paused")),
                        Some(SessionState::Stopped) => Some(localization::tr("hint.finished")),
                        None if self.selected_mode == TrainingMode::ExternalText
                            && self.external_text.is_empty() =>
                        {
                            Some(localization::tr("hint.open-text"))
                        }
                        None => Some(localization::tr("hint.start")),
                        _ => None,
                    }
                    .as_deref(),
                    self.background_texture.as_ref().map(|texture| {
                        (
                            texture,
                            background_mode(background_settings.placement),
                            (u16::from(background_settings.dim_percent.min(100)) * 255 / 100) as u8,
                        )
                    }),
                );
            });

        if self.show_keyboard {
            let viewport_id = egui::ViewportId::from_hash_of("keyboard-window");
            let builder = egui::ViewportBuilder::default()
                .with_title(localization::tr("keyboard"))
                .with_inner_size([1100.0, 420.0])
                .with_min_inner_size([640.0, 250.0]);
            let layout = if self.english_layout {
                keyboard::KeyboardLayout::English
            } else {
                keyboard::KeyboardLayout::Russian
            };
            let state = (layout, next_char);
            if self.last_keyboard_state != Some(state) {
                root.ctx().request_repaint_of(viewport_id);
                self.last_keyboard_state = Some(state);
            }
            let input_queue = Arc::clone(&self.keyboard_input);
            root.ctx()
                .show_viewport_deferred(viewport_id, builder, move |ui, _class| {
                    let ctx = ui.ctx();
                    let events = ctx.input(|input| input.events.clone());
                    let close_requested = ctx.input(|input| input.viewport().close_requested());
                    queue_keyboard_input(&input_queue, ctx, events, close_requested);
                    keyboard::draw_keyboard(ui, layout, next_char);
                });
        } else {
            self.last_keyboard_state = None;
        }
        if self.chooser_open {
            self.show_content_chooser(root.ctx());
        }
        self.show_content_editors(root.ctx());
        if self.background_open {
            self.show_background_window(root.ctx());
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
        if self.result_open
            && let Some(stats) = self.last_result.clone()
        {
            let has_next = self.result_completed
                && self.content_mode()
                && self.current_index() + 1 < self.items().len();
            let can_continue = self.session.as_ref().is_some_and(|s| {
                s.stop_reason() == Some(StopReason::Manual) && s.remaining_chars() > 0
            });
            match result_view::show(
                root.ctx(),
                &self.result_title,
                &stats,
                self.result_completed,
                has_next,
                can_continue,
            ) {
                ResultAction::Stay => {}
                ResultAction::Continue => {
                    if self
                        .session
                        .as_mut()
                        .is_some_and(Session::continue_after_stop)
                    {
                        self.result_open = false;
                        self.last_result = None;
                    }
                }
                ResultAction::Retry => {
                    if self.accept_result() {
                        self.start();
                    }
                }
                ResultAction::Next => {
                    if self.accept_result() && self.advance_content() {
                        self.start();
                    }
                }
                ResultAction::Close => {
                    if self.accept_result()
                        && self.result_completed
                        && self.selected_mode == TrainingMode::Lessons
                    {
                        self.advance_content();
                    }
                }
            }
        }
        if active {
            root.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

impl eframe::App for StaminatorApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if root.ctx().input(|input| input.viewport().close_requested()) && !self.result_recorded {
            self.save_external_progress();
        }
        self.draw(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "staminator-app-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn sqlite_catalog_preserves_groups_and_updates_builtin_lessons() {
        let root = test_path().with_extension("directory");
        let builtin = root.join("builtin");
        let user = root.join("user");
        fs::create_dir_all(builtin.join("basics")).unwrap();
        fs::create_dir_all(user.join("practice")).unwrap();
        let builtin_path = builtin.join("basics/first.json");
        let user_path = user.join("practice/custom.json");
        let builtin_lesson = stamina_data::JsonLesson {
            version: 1,
            id: "first".into(),
            title: "First".into(),
            language: "en".into(),
            description: None,
            text: "abc".into(),
        };
        let user_lesson = stamina_data::JsonLesson {
            id: "custom".into(),
            title: "Custom".into(),
            ..builtin_lesson.clone()
        };
        fs::write(&builtin_path, serde_json::to_vec(&builtin_lesson).unwrap()).unwrap();
        fs::write(&user_path, serde_json::to_vec(&user_lesson).unwrap()).unwrap();
        let mut database =
            stamina_data::storage::Database::open(root.join("data.sqlite3")).unwrap();
        let mut catalog = ContentCatalog::fixture_empty();
        catalog.set_lesson_roots(vec![builtin.clone(), user.clone()]);
        sync_catalog_with_database(&mut catalog, &mut database).unwrap();
        assert!(
            catalog
                .lessons(ContentLanguage::English)
                .iter()
                .any(|lesson| lesson.name == "practice / Custom")
        );
        let mut updated = builtin_lesson;
        updated.text = "new".into();
        fs::write(&builtin_path, serde_json::to_vec(&updated).unwrap()).unwrap();
        catalog.set_lesson_roots(vec![builtin, user]);
        sync_catalog_with_database(&mut catalog, &mut database).unwrap();
        assert_eq!(database.lesson("first").unwrap().unwrap().0.text, "new");
        assert_eq!(database.lesson("custom").unwrap().unwrap().0, user_lesson);
        assert_eq!(
            catalog.lesson_groups().get("custom").map(String::as_str),
            Some("practice")
        );
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_builtin_directory_preserves_stored_lessons() {
        let root = test_path().with_extension("missing-builtin");
        let user = root.join("user");
        fs::create_dir_all(&user).unwrap();
        let lesson = stamina_data::JsonLesson {
            version: 1,
            id: "old".into(),
            title: "Old".into(),
            language: "en".into(),
            description: None,
            text: "abc".into(),
        };
        let mut database = stamina_data::storage::Database::open(root.join("db.sqlite3")).unwrap();
        database.save_lesson(&lesson, true).unwrap();
        let custom = stamina_data::JsonLesson {
            id: "custom".into(),
            ..lesson
        };
        fs::write(
            user.join("custom.json"),
            serde_json::to_vec(&custom).unwrap(),
        )
        .unwrap();
        let mut catalog = ContentCatalog::fixture_empty();
        catalog.set_lesson_roots(vec![root.join("missing"), user]);
        sync_catalog_with_database(&mut catalog, &mut database).unwrap();
        assert!(database.lesson("old").unwrap().is_some());
        assert!(database.lesson("custom").unwrap().is_some());
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn app_exports_lesson_and_backup_and_opens_lesson_for_import() {
        let root = test_path().with_extension("transfer");
        fs::create_dir_all(&root).unwrap();
        let mut app = test_app(root.join("fixture.json"));
        let unavailable = root.join("unavailable.sqlite3");
        assert!(app.export_backup_to(&unavailable).is_err());
        assert!(!unavailable.exists());
        app.users =
            UserStore::load_database(root.join("app.sqlite3"), &root.join("legacy.json")).unwrap();
        let lesson = stamina_data::JsonLesson {
            version: 1,
            id: "exported".into(),
            title: "Exported".into(),
            language: "ru".into(),
            description: None,
            text: "текст".into(),
        };
        app.catalog
            .set_stored_lessons(vec![(lesson.clone(), false)]);
        let lesson_path = root.join("lesson.json");
        app.export_lesson_to(0, &lesson_path).unwrap();
        assert!(app.export_lesson_to(0, &lesson_path).is_err());
        app.chooser_open = true;
        app.lesson_transfer_error = "Ошибка переноса".into();
        app.import_lesson_from(&lesson_path).unwrap();
        assert_eq!(app.lesson_editor.as_ref().unwrap().lesson, lesson);
        assert!(!app.chooser_open);
        assert!(app.lesson_transfer_error.is_empty());
        let missing = root.join("missing.json");
        assert!(app.import_lesson_from(&missing).is_err());
        assert_eq!(app.lesson_editor.as_ref().unwrap().lesson.id, "exported");
        let malformed = root.join("malformed.json");
        fs::write(&malformed, "{bad json}").unwrap();
        assert!(app.import_lesson_from(&malformed).is_err());
        assert_eq!(app.lesson_editor.as_ref().unwrap().lesson.text, "текст");
        assert!(app.export_lesson_to(10, &root.join("absent.json")).is_err());
        assert!(!root.join("absent.json").exists());
        let backup = root.join("backup.sqlite3");
        app.export_backup_to(&backup).unwrap();
        storage_transfer::prepare_database_restore(&backup).unwrap();
        assert!(app.export_backup_to(&backup).is_err());
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catalog_sync_updates_builtin_group_and_keeps_user_lesson_id_conflicts() {
        let root = test_path().with_extension("sync-conflicts");
        let builtin = root.join("builtin");
        fs::create_dir_all(builtin.join("Basics")).unwrap();
        let builtin_path = builtin.join("Basics/shared.json");
        let document = |title: &str, text: &str| stamina_data::JsonLesson {
            version: 1,
            id: "shared".into(),
            title: title.into(),
            language: "en".into(),
            description: None,
            text: text.into(),
        };
        fs::write(
            &builtin_path,
            serde_json::to_vec(&document("Bundled", "bundle text")).unwrap(),
        )
        .unwrap();

        let mut database =
            stamina_data::storage::Database::open(root.join("data.sqlite3")).unwrap();
        let user_lesson = stamina_data::storage::LessonDocument {
            version: 1,
            id: "shared".into(),
            title: "My copy".into(),
            language: "en".into(),
            description: None,
            text: "user text".into(),
        };
        database.create_user_lesson(&user_lesson).unwrap();

        let mut catalog = ContentCatalog::fixture_empty();
        catalog.set_lesson_roots(vec![builtin.clone()]);
        sync_catalog_with_database(&mut catalog, &mut database).unwrap();
        assert_eq!(
            database.lesson("shared").unwrap().unwrap(),
            (user_lesson.clone(), false)
        );
        assert_eq!(
            database
                .get_app_setting("lesson_group:shared")
                .unwrap()
                .as_deref(),
            Some("Basics")
        );
        assert_eq!(
            catalog.lessons(ContentLanguage::English)[0].name,
            "Basics / My copy"
        );

        fs::create_dir_all(builtin.join("Advanced")).unwrap();
        let new_path = builtin.join("Advanced/shared.json");
        fs::write(
            &new_path,
            serde_json::to_vec(&document("Bundled", "changed bundled text")).unwrap(),
        )
        .unwrap();
        fs::remove_file(&builtin_path).unwrap();
        catalog.set_lesson_roots(vec![builtin]);
        sync_catalog_with_database(&mut catalog, &mut database).unwrap();

        // An ID already owned by a user remains theirs, including its saved group.
        assert_eq!(
            database.lesson("shared").unwrap().unwrap(),
            (user_lesson, false)
        );
        assert_eq!(
            database
                .get_app_setting("lesson_group:shared")
                .unwrap()
                .as_deref(),
            Some("Basics")
        );
        assert_eq!(
            catalog.lessons(ContentLanguage::English)[0].name,
            "Basics / My copy"
        );
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catalog_sync_moves_builtin_group_and_refreshes_display_name() {
        let root = test_path().with_extension("sync-groups");
        let builtin = root.join("builtin");
        fs::create_dir_all(builtin.join("Basics")).unwrap();
        let old_path = builtin.join("Basics/lesson.json");
        let new_path = builtin.join("Advanced/lesson.json");
        let lesson = stamina_data::JsonLesson {
            version: 1,
            id: "grouped".into(),
            title: "Lesson".into(),
            language: "en".into(),
            description: None,
            text: "text".into(),
        };
        fs::write(&old_path, serde_json::to_vec(&lesson).unwrap()).unwrap();

        let mut database =
            stamina_data::storage::Database::open(root.join("data.sqlite3")).unwrap();
        let mut catalog = ContentCatalog::fixture_empty();
        catalog.set_lesson_roots(vec![builtin.clone()]);
        sync_catalog_with_database(&mut catalog, &mut database).unwrap();
        assert_eq!(
            catalog.lessons(ContentLanguage::English)[0].name,
            "Basics / Lesson"
        );

        fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        fs::write(&new_path, serde_json::to_vec(&lesson).unwrap()).unwrap();
        fs::remove_file(&old_path).unwrap();
        catalog.set_lesson_roots(vec![builtin]);
        sync_catalog_with_database(&mut catalog, &mut database).unwrap();

        assert_eq!(
            database
                .get_app_setting("lesson_group:grouped")
                .unwrap()
                .as_deref(),
            Some("Advanced")
        );
        assert_eq!(
            catalog.lessons(ContentLanguage::English)[0].name,
            "Advanced / Lesson"
        );
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn input(app: &mut StaminatorApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let released = events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key { key, .. } => Some(egui::Event::Key {
                    key: *key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }),
                _ => None,
            })
            .collect();
        let raw = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| app.handle_input(ui.ctx()));
        output.textures_delta.clear();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: released,
                ..Default::default()
            },
            |_| {},
        );
        output.textures_delta.clear();
    }

    fn draw_accessible(
        app: &mut StaminatorApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<(accesskit::NodeId, accesskit::Node)> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| app.draw(ui),
        );
        output.textures_delta.clear();
        output
            .platform_output
            .accesskit_update
            .take()
            .unwrap()
            .nodes
    }

    fn accessible_click(id: accesskit::NodeId) -> egui::Event {
        egui::Event::AccessKitActionRequest(accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: id,
            data: None,
        })
    }

    fn test_app(path: PathBuf) -> StaminatorApp {
        let mut app = StaminatorApp::from_parts(
            ContentCatalog::fixture(),
            true,
            None,
            UserStore::new_at(path),
            String::new(),
            |_| {},
        );
        app.selected_mode = TrainingMode::ExternalText;
        app.english_layout = true;
        app.external_path = Some(PathBuf::from("example.txt"));
        app.external_text = "ab".into();
        app
    }

    #[test]
    fn background_follows_profile_and_missing_file_falls_back() {
        let path = test_path();
        let image_path = path.with_extension("png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 40, 60, 255]))
            .save(&image_path)
            .unwrap();
        let mut app = test_app(path.clone());
        let original = app.users.active_profile_id().to_owned();
        let settings = user_store::BackgroundImageSettings {
            image_path: Some(image_path.clone()),
            use_builtin: false,
            placement: user_store::BackgroundPlacement::Fill,
            dim_percent: 50,
        };
        app.users.set_background_settings(settings.clone()).unwrap();
        let ctx = egui::Context::default();
        app.refresh_background(&ctx);
        assert!(app.background_texture.is_some());
        let other = app.users.create("Other").unwrap().id.clone();
        app.users.select(&other).unwrap();
        app.refresh_background(&ctx);
        assert!(app.background_texture.is_some());
        app.users.select(&original).unwrap();
        app.refresh_background(&ctx);
        assert!(app.background_texture.is_some());
        drop(app);
        let mut app = test_app(path.clone());
        app.users = UserStore::load_from(&path).unwrap();
        assert_eq!(app.users.background_settings(), Some(&settings));
        fs::remove_file(&image_path).unwrap();
        app.refresh_background(&ctx);
        assert!(app.background_texture.is_none());
        assert!(!app.background_error.is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn background_dialog_renders_options_with_the_builtin_art_selected() {
        let path = test_path().with_extension("background-dialog.json");
        let mut app = test_app(path);
        app.background_open = true;
        let ctx = egui::Context::default();
        app.refresh_background(&ctx);

        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| app.show_background_window(ui.ctx()),
        );
        assert!(app.background_texture.is_some());
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
    }

    fn database_test_app(path: PathBuf) -> StaminatorApp {
        let legacy = path.with_extension("legacy.json");
        let users = UserStore::load_database(path, &legacy).unwrap();
        StaminatorApp::from_parts(
            ContentCatalog::fixture_empty(),
            true,
            None,
            users,
            String::new(),
            |_| {},
        )
    }

    #[test]
    fn phrase_editor_saves_and_reopens_training_content() {
        let root = std::env::temp_dir().join(format!(
            "staminator-phrase-ui-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let mut app = test_app(root.join("profiles.json"));
        app.catalog.set_phrase_root(root.join("phrases"));
        app.selected_mode = TrainingMode::Phrases;
        app.chooser_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Добавить фразы"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        app.phrase_editor.as_mut().unwrap().text = "Новая фраза!".into();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить фразы"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        assert!(app.phrase_editor.is_none());
        let index = app.catalog.phrases(ContentLanguage::Russian).len() - 1;
        app.select_content(index);
        app.start();
        assert_eq!(app.session_text, "Новая фраза! ");
        for (index, ch) in "Новая фраза! ".chars().enumerate() {
            app.type_input_char(ch, index as u64);
        }
        let mut reopened = ContentCatalog::fixture();
        reopened.set_phrase_root(root.join("phrases"));
        assert!(
            reopened
                .phrases(ContentLanguage::Russian)
                .iter()
                .any(|p| p.text == "Новая фраза! ")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn phrase_editor_rejects_empty_input_and_can_retry() {
        let root = std::env::temp_dir().join(format!(
            "staminator-phrase-retry-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let mut app = test_app(root.join("profile"));
        app.catalog.set_phrase_root(root.join("phrases"));
        app.content_language = ContentLanguage::English;
        app.selected_mode = TrainingMode::Phrases;
        app.phrase_editor = Some(content_editor::PhraseEditor::default());
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить фразы"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        assert!(
            app.phrase_editor
                .as_ref()
                .unwrap()
                .error
                .contains("Не удалось")
        );
        app.phrase_editor.as_mut().unwrap().text = "Hello!".into();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить фразы"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        assert!(app.phrase_editor.is_none());
        assert!(
            app.catalog
                .phrases(ContentLanguage::English)
                .iter()
                .any(|p| p.text == "Hello! ")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn builtin_lesson_is_not_opened_for_overwriting() {
        let root = std::env::temp_dir().join(format!(
            "staminator-builtin-edit-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let builtin = root.join("builtin");
        fs::create_dir_all(&builtin).unwrap();
        let path = builtin.join("one.json");
        let original =
            r#"{"version":1,"id":"builtin","title":"Built in","language":"ru","text":"original"}"#;
        fs::write(&path, original).unwrap();
        let mut app = test_app(root.join("profile"));
        app.catalog
            .set_lesson_roots(vec![builtin, root.join("user")]);
        app.selected_mode = TrainingMode::Lessons;
        app.chooser_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Изменить выбранный урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        let editor = app.lesson_editor.as_ref().unwrap();
        assert!(editor.path.is_none());
        assert!(editor.error.contains("Встроенный урок"));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_keeps_invalid_content_on_validation_error() {
        let root = std::env::temp_dir().join(format!(
            "staminator-editor-invalid-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let builtin = root.join("builtin");
        let user = root.join("user");
        fs::create_dir_all(&builtin).unwrap();
        fs::write(
            builtin.join("one.json"),
            r#"{"version":1,"id":"used","title":"Existing","language":"ru","text":"original"}"#,
        )
        .unwrap();
        let mut app = test_app(root.join("profiles.json"));
        app.catalog.set_lesson_roots(vec![builtin.clone(), user]);
        app.lesson_editor = Some(content_editor::LessonEditor::new("fr"));
        let editor = app.lesson_editor.as_mut().unwrap();
        editor.lesson.id = "used".into();
        editor.lesson.title = "Edited".into();
        editor.lesson.text = "new".into();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        assert!(
            app.lesson_editor
                .as_ref()
                .unwrap()
                .error
                .contains("русском")
        );
        app.lesson_editor.as_mut().unwrap().lesson.language = "ru".into();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        let editor = app.lesson_editor.as_ref().unwrap();
        assert!(editor.error.contains("используется"));
        assert_eq!(editor.lesson.text, "new");
        assert!(
            fs::read_to_string(builtin.join("one.json"))
                .unwrap()
                .contains("original")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn english_lesson_validation_error_is_localized_and_preserves_editor_content() {
        let root = test_path().with_extension("english-validation");
        let mut app = database_test_app(root.join("profiles.sqlite3"));
        app.users.set_interface_language("EN").unwrap();
        app.sync_profile_language();
        let mut editor = content_editor::LessonEditor::new("en");
        editor.lesson.version = 2;
        editor.lesson.id = "valid-id".into();
        editor.lesson.title = "Valid title".into();
        editor.lesson.text = "Keep this text".into();
        app.lesson_editor = Some(editor);

        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let save = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Save lesson"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(save)]);

        let editor = app.lesson_editor.as_ref().unwrap();
        assert_eq!(
            editor.error,
            "Could not save lesson: unsupported lesson version 2"
        );
        assert_eq!(editor.lesson.text, "Keep this text");
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_reports_invalid_lesson_and_phrase_storage_errors() {
        let mut app = test_app(test_path());
        app.lesson_editor = Some(content_editor::LessonEditor::new("ru"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        assert!(
            app.lesson_editor
                .as_ref()
                .unwrap()
                .error
                .contains("каталог")
        );
        app.lesson_editor = None;
        app.phrase_editor = Some(content_editor::PhraseEditor {
            text: "Фраза".into(),
            error: String::new(),
        });
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить фразы"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        assert!(
            app.phrase_editor
                .as_ref()
                .unwrap()
                .error
                .contains("Не удалось сохранить")
        );
    }

    #[test]
    fn editor_generates_saves_edits_and_reloads_a_lesson() {
        let root = std::env::temp_dir().join(format!(
            "staminator-editor-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let builtin = root.join("builtin");
        let user = root.join("user");
        fs::create_dir_all(&builtin).unwrap();
        let mut app = test_app(root.join("profiles.json"));
        app.catalog
            .set_lesson_roots(vec![builtin.clone(), user.clone()]);
        app.selected_mode = TrainingMode::Lessons;
        app.chooser_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Создать урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        let editor = app.lesson_editor.as_mut().unwrap();
        editor.lesson.id = "my-lesson".into();
        editor.lesson.title = "Мой урок".into();
        editor.symbols = "аб".into();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Добавить упражнение"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        assert!(
            app.lesson_editor
                .as_ref()
                .unwrap()
                .lesson
                .text
                .starts_with("аааббб")
        );
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        assert!(app.lesson_editor.is_none());
        app.start();
        let generated = app.session_text.clone();
        for (index, ch) in generated.chars().enumerate() {
            app.type_input_char(ch, index as u64);
        }
        assert_eq!(app.session.as_ref().unwrap().remaining_chars(), 0);
        let catalog = stamina_data::load_lesson_catalog(&[builtin, user.clone()]);
        assert_eq!(catalog.lessons[0].lesson.id, "my-lesson");
        app.session = None;
        app.chooser_open = true;
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Изменить выбранный урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        app.lesson_editor.as_mut().unwrap().lesson.text = "новый текст".into();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let id = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(id)]);
        assert_eq!(app.exercise(), "новый текст");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_lesson_editor_create_edit_and_reopen_round_trip() {
        let root = std::env::temp_dir().join(format!(
            "staminator-sqlite-editor-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("staminator.sqlite3");
        let legacy_path = root.join("legacy.json");
        let mut app = test_app(root.join("test-profile.json"));
        app.users = UserStore::load_database(database_path.clone(), &legacy_path).unwrap();
        app.catalog.set_stored_lessons(vec![]);
        app.selected_mode = TrainingMode::Lessons;
        app.chooser_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let create = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Создать урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(create)]);
        let editor = app.lesson_editor.as_mut().unwrap();
        editor.lesson.id = "sqlite-round-trip".into();
        editor.lesson.title = "SQLite round trip".into();
        editor.lesson.text = "first text".into();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let save = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(save)]);
        assert!(app.lesson_editor.is_none());

        let restarted_users =
            UserStore::load_database(database_path.clone(), &legacy_path).unwrap();
        let database = restarted_users.database().unwrap().unwrap();
        let lessons = database.list_lessons().unwrap();
        assert_eq!(lessons.len(), 1);
        assert_eq!(lessons[0].0.text, "first text");
        assert!(!lessons[0].1);
        drop(database);
        let mut restarted = test_app(root.join("second-profile.json"));
        restarted.users = restarted_users;
        restarted.catalog.set_stored_lessons(lessons);
        restarted.selected_mode = TrainingMode::Lessons;
        restarted.chooser_open = true;
        draw_accessible(&mut restarted, &ctx, vec![]);
        let nodes = draw_accessible(&mut restarted, &ctx, vec![]);
        let edit = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Изменить выбранный урок"))
            .unwrap()
            .0;
        draw_accessible(&mut restarted, &ctx, vec![accessible_click(edit)]);
        assert_eq!(
            restarted.lesson_editor.as_ref().unwrap().lesson.id,
            "sqlite-round-trip"
        );
        restarted.lesson_editor.as_mut().unwrap().lesson.text = "edited text".into();
        let nodes = draw_accessible(&mut restarted, &ctx, vec![]);
        let save = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut restarted, &ctx, vec![accessible_click(save)]);
        assert!(restarted.lesson_editor.is_none());

        let reopened = stamina_data::storage::Database::open(&database_path).unwrap();
        let saved = reopened.lesson("sqlite-round-trip").unwrap().unwrap();
        assert_eq!(saved.0.text, "edited text");
        assert!(!saved.1);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_lesson_editor_rejects_builtin_duplicate_and_invalid_documents() {
        let root = std::env::temp_dir().join(format!(
            "staminator-sqlite-editor-errors-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("staminator.sqlite3");
        let legacy_path = root.join("legacy.json");
        let users = UserStore::load_database(database_path.clone(), &legacy_path).unwrap();
        let mut database = users.database().unwrap().unwrap();
        let built_in = stamina_data::storage::LessonDocument {
            version: 1,
            id: "protected".into(),
            title: "Built in".into(),
            language: "ru".into(),
            description: None,
            text: "original".into(),
        };
        let user_lesson = stamina_data::storage::LessonDocument {
            version: 1,
            id: "already-used".into(),
            title: "Existing user lesson".into(),
            language: "ru".into(),
            description: None,
            text: "existing".into(),
        };
        database.save_lesson(&built_in, true).unwrap();
        database.save_lesson(&user_lesson, false).unwrap();
        let stored = database.list_lessons().unwrap();
        drop(database);

        let mut app = test_app(root.join("unused-profile.json"));
        app.users = users;
        app.catalog.set_stored_lessons(stored);
        app.selected_mode = TrainingMode::Lessons;
        app.selected_lessons[0] = Some("protected".into());
        app.chooser_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let edit = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Изменить выбранный урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(edit)]);
        assert_eq!(app.lesson_editor.as_ref().unwrap().lesson.text, "original");
        app.lesson_editor.as_mut().unwrap().lesson.text = "user edit".into();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let save = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(save)]);
        let mut db = app.users.database().unwrap().unwrap();
        db.sync_builtin_lessons(&[built_in]).unwrap();
        assert_eq!(db.lesson("protected").unwrap().unwrap().0.text, "user edit");
        drop(db);
        let reopened = stamina_data::storage::Database::open(&database_path).unwrap();
        let (saved, built_in) = reopened.lesson("protected").unwrap().unwrap();
        assert_eq!(saved.text, "user edit");
        assert!(!built_in);
        drop(reopened);
        app.lesson_editor = None;

        // A new document cannot take the ID of an existing user document.
        app.chooser_open = true;
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let create = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Создать урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(create)]);
        let editor = app.lesson_editor.as_mut().unwrap();
        editor.lesson.id = "already-used".into();
        editor.lesson.title = "Duplicate".into();
        editor.lesson.text = "new".into();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let save = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(save)]);
        assert!(
            app.lesson_editor
                .as_ref()
                .unwrap()
                .error
                .contains("уже существует")
        );

        // Invalid documents are rejected while the editor keeps the entered content.
        let editor = app.lesson_editor.as_mut().unwrap();
        editor.lesson.title.clear();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let save = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(save)]);
        let editor = app.lesson_editor.as_ref().unwrap();
        assert!(editor.error.contains("название"));
        assert_eq!(editor.lesson.id, "already-used");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_lesson_editor_reports_database_open_failure() {
        let root = std::env::temp_dir().join(format!(
            "staminator-sqlite-editor-db-error-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("staminator.sqlite3");
        let legacy_path = root.join("legacy.json");
        let mut app = test_app(root.join("unused-profile.json"));
        app.users = UserStore::load_database(database_path.clone(), &legacy_path).unwrap();
        fs::remove_file(&database_path).unwrap();
        fs::create_dir(&database_path).unwrap();
        app.lesson_editor = Some(content_editor::LessonEditor::new("ru"));
        let editor = app.lesson_editor.as_mut().unwrap();
        editor.lesson.id = "lesson".into();
        editor.lesson.title = "Lesson".into();
        editor.lesson.text = "text".into();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let save = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(save)]);
        assert!(
            app.lesson_editor
                .as_ref()
                .unwrap()
                .error
                .contains("Не удалось открыть базу")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_keyboard_visibility_option_persists_as_global_setting() {
        let root = std::env::temp_dir().join(format!(
            "staminator-sqlite-keyboard-setting-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("staminator.sqlite3");
        let legacy_path = root.join("legacy.json");
        let mut app = test_app(root.join("unused-profile.json"));
        app.users = UserStore::load_database(database_path.clone(), &legacy_path).unwrap();
        app.show_keyboard = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let options = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Опции"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(options)]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let checkbox = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Показывать клавиатуру"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(checkbox)]);
        assert!(app.show_keyboard);
        let reopened = UserStore::load_database(database_path, &legacy_path).unwrap();
        let database = reopened.database().unwrap().unwrap();
        assert_eq!(
            database
                .get_app_setting("show_keyboard")
                .unwrap()
                .as_deref(),
            Some("true")
        );
        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_lesson_editor_rejects_changing_existing_id() {
        let root = std::env::temp_dir().join(format!(
            "staminator-sqlite-editor-id-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("staminator.sqlite3");
        let legacy_path = root.join("legacy.json");
        let users = UserStore::load_database(database_path, &legacy_path).unwrap();
        let mut database = users.database().unwrap().unwrap();
        let lesson = stamina_data::storage::LessonDocument {
            version: 1,
            id: "fixed-id".into(),
            title: "Lesson".into(),
            language: "ru".into(),
            description: None,
            text: "original".into(),
        };
        database.save_lesson(&lesson, false).unwrap();
        let stored = database.list_lessons().unwrap();
        drop(database);
        let mut app = test_app(root.join("unused-profile.json"));
        app.users = users;
        app.catalog.set_stored_lessons(stored);
        app.selected_mode = TrainingMode::Lessons;
        app.selected_lessons[0] = Some("fixed-id".into());
        app.chooser_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let edit = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Изменить выбранный урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(edit)]);
        let editor = app.lesson_editor.as_mut().unwrap();
        editor.lesson.id = "changed-id".into();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let save = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(save)]);
        assert!(
            app.lesson_editor
                .as_ref()
                .unwrap()
                .error
                .contains("Изменение ID")
        );
        assert_eq!(app.lesson_editor.as_ref().unwrap().lesson.id, "changed-id");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_lesson_editor_reports_missing_record_after_catalog_load() {
        let root = std::env::temp_dir().join(format!(
            "staminator-sqlite-editor-missing-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("staminator.sqlite3");
        let legacy_path = root.join("legacy.json");
        let users = UserStore::load_database(database_path.clone(), &legacy_path).unwrap();
        let mut database = users.database().unwrap().unwrap();
        let lesson = stamina_data::storage::LessonDocument {
            version: 1,
            id: "removed-after-load".into(),
            title: "Lesson".into(),
            language: "ru".into(),
            description: None,
            text: "original".into(),
        };
        database.save_lesson(&lesson, false).unwrap();
        let catalog_lessons = database.list_lessons().unwrap();
        database.delete_lesson(&lesson.id).unwrap();
        drop(database);

        let mut app = test_app(root.join("unused-profile.json"));
        app.users = users;
        app.catalog.set_stored_lessons(catalog_lessons);
        app.selected_mode = TrainingMode::Lessons;
        app.selected_lessons[0] = Some(lesson.id.clone());
        app.chooser_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let edit = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Изменить выбранный урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(edit)]);
        assert_eq!(app.lesson_editor.as_ref().unwrap().lesson.id, lesson.id);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let save = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Сохранить урок"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(save)]);
        let editor = app.lesson_editor.as_ref().unwrap();
        assert!(editor.error.contains("Урок не найден"));
        assert_eq!(editor.lesson.text, "original");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refresh_preserves_lesson_id_and_new_lesson_can_be_completed() {
        let root = std::env::temp_dir().join(format!(
            "staminator-refresh-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let lesson = |id: &str, text: &str| {
            format!(r#"{{"version":1,"id":"{id}","title":"{id}","language":"ru","text":"{text}"}}"#)
        };
        std::fs::write(root.join("one.json"), lesson("b", "ab")).unwrap();
        std::fs::write(root.join("two.json"), lesson("c", "cd")).unwrap();
        let mut app = test_app(root.join("profiles.json"));
        app.catalog.set_lesson_roots(vec![root.clone()]);
        app.selected_mode = TrainingMode::Lessons;
        app.content_language = ContentLanguage::Russian;
        app.select_content(1);
        assert_eq!(app.exercise(), "cd");
        std::fs::write(nested.join("new.json"), lesson("a", "xy")).unwrap();
        std::fs::write(nested.join("bad.json"), "{").unwrap();
        app.catalog.refresh_lessons();
        assert_eq!(app.exercise(), "cd");
        assert!(app.catalog.lesson_diagnostics().contains("bad.json"));
        app.select_content(0);
        app.start();
        app.type_input_char('x', 1);
        app.type_input_char('y', 2);
        assert_eq!(app.session.as_ref().unwrap().remaining_chars(), 0);
        assert!(app.advance_content());
        assert_eq!(app.exercise(), "ab");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stopped_session_is_saved_once_to_active_profile() {
        let path = std::env::temp_dir().join(format!(
            "staminator-controller-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut app = test_app(path.clone());
        app.start();
        app.type_input_char('x', 0);
        app.type_input_char('a', 0);
        app.toggle_pause();
        app.type_input_char('b', 0);
        assert_eq!(app.session.as_ref().unwrap().remaining_chars(), 1);
        app.stop();
        app.stop();
        app.record_current_result(None);
        assert!(app.result_open);
        assert!(!app.result_completed);
        let records = UserStore::load_from(&path).unwrap();
        assert_eq!(records.active_records().len(), 1);
        let result = &records.active_records()[0];
        assert_eq!((result.correct, result.errors), (1, 1));
        assert_eq!(result.exercise, "example.txt");
        assert_eq!(result.layout, "layout.en");
        assert_eq!(result.stop_reason, "manual");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn continue_button_keeps_one_result_until_final_acceptance() {
        let path = test_path();
        let mut app = test_app(path.clone());
        app.show_keyboard = false;
        app.start();
        app.type_input_char('x', 0);
        app.type_input_char('a', 0);
        app.stop();
        assert!(app.users.active_records().is_empty());
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let button = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Продолжить"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(button)]);
        assert!(!app.result_open);
        input(&mut app, &ctx, vec![egui::Event::Text("b".into())]);
        assert!(app.result_completed);
        app.accept_result();
        app.accept_result();
        let records = UserStore::load_from(&path).unwrap();
        assert_eq!(records.active_records().len(), 1);
        assert_eq!(
            (
                records.active_records()[0].correct,
                records.active_records()[0].errors
            ),
            (2, 1)
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn time_limit_stops_free_mode_but_not_lessons() {
        let mut app = test_app(test_path());
        app.start();
        app.type_input_char('a', 0);
        app.check_time_limit(300_000);
        assert_eq!(
            app.session.as_ref().unwrap().stop_reason(),
            Some(StopReason::TimeLimit)
        );
        assert!(app.result_open);
        app.selected_mode = TrainingMode::Lessons;
        app.start();
        let ch = app.session_text.chars().next().unwrap();
        app.type_input_char(ch, 0);
        app.check_time_limit(300_000);
        assert!(!app.result_open);
    }

    #[test]
    fn keyboard_events_start_pause_resume_and_complete() {
        let path = test_path();
        let mut app = test_app(path.clone());
        let ctx = egui::Context::default();
        input(
            &mut app,
            &ctx,
            vec![key(egui::Key::Space), egui::Event::Text(" ".into())],
        );
        assert_eq!(app.session.as_ref().unwrap().snapshot(0).position, 0);
        input(
            &mut app,
            &ctx,
            vec![egui::Event::Text("xa".into()), key(egui::Key::F9)],
        );
        let paused = app.session.as_ref().unwrap().snapshot(app.now_ms());
        assert_eq!(
            (paused.state, paused.correct, paused.errors),
            (SessionState::Paused, 1, 1)
        );
        input(
            &mut app,
            &ctx,
            vec![key(egui::Key::Space), egui::Event::Text(" ".into())],
        );
        assert_eq!(app.session.as_ref().unwrap().snapshot(0).position, 1);
        input(&mut app, &ctx, vec![egui::Event::Text("b".into())]);
        assert!(app.result_open && app.result_completed);
        assert_eq!(app.users.active_records().len(), 0);
        app.accept_result();
        assert_eq!(app.users.active_records().len(), 1);
        input(&mut app, &ctx, vec![egui::Event::Text("a".into())]);
        assert_eq!(app.users.active_records().len(), 1);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn deferred_keyboard_queue_delivers_text_and_hotkeys_to_the_parent() {
        let path = test_path();
        let mut app = test_app(path.clone());
        let ctx = egui::Context::default();

        queue_keyboard_input(
            &app.keyboard_input,
            &ctx,
            vec![key(egui::Key::Space), egui::Event::Text(" ".into())],
            false,
        );
        let events = app.take_keyboard_input();
        app.handle_input_events(&ctx, events);
        assert_eq!(
            app.session.as_ref().unwrap().snapshot(0).state,
            SessionState::Ready
        );
        assert_eq!(app.session.as_ref().unwrap().snapshot(0).position, 0);

        queue_keyboard_input(
            &app.keyboard_input,
            &ctx,
            vec![egui::Event::Text("a".into()), key(egui::Key::F9)],
            false,
        );
        let events = app.take_keyboard_input();
        app.handle_input_events(&ctx, events);
        let snapshot = app.session.as_ref().unwrap().snapshot(app.now_ms());
        assert_eq!(snapshot.state, SessionState::Paused);
        assert_eq!(snapshot.correct, 1);
        assert_eq!(snapshot.position, 1);

        drop(app);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn deferred_keyboard_close_updates_visibility_and_persists_setting() {
        let path = test_path();
        let settings = path.with_extension("settings.txt");
        let mut app = test_app(path.clone());
        app.keyboard_setting_path = Some(settings.clone());
        let ctx = egui::Context::default();

        queue_keyboard_input(&app.keyboard_input, &ctx, vec![], true);
        let events = app.take_keyboard_input();
        app.handle_input_events(&ctx, events);

        assert!(!app.show_keyboard);
        assert_eq!(fs::read_to_string(&settings).unwrap(), "false");
        drop(app);
        fs::remove_file(settings).unwrap();
        let _ = fs::remove_file(path);
    }

    #[test]
    fn escape_stops_and_modal_blocks_input() {
        let path = test_path();
        let mut app = test_app(path.clone());
        let ctx = egui::Context::default();
        input(
            &mut app,
            &ctx,
            vec![key(egui::Key::Space), egui::Event::Text("a".into())],
        );
        app.background_open = true;
        input(&mut app, &ctx, vec![egui::Event::Text("b".into())]);
        assert_eq!(app.session.as_ref().unwrap().snapshot(0).position, 1);
        app.background_open = false;
        app.chooser_open = true;
        input(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        assert_eq!(
            app.session.as_ref().unwrap().snapshot(0).state,
            SessionState::Running
        );
        app.chooser_open = false;
        input(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        assert!(app.result_open);
        assert!(!app.result_completed);
        app.accept_result();
        assert_eq!(app.users.active_records().len(), 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn copy_shortcut_stops_session_and_does_not_repeat_result() {
        let path = test_path();
        let mut app = test_app(path.clone());
        let ctx = egui::Context::default();
        input(
            &mut app,
            &ctx,
            vec![key(egui::Key::Space), egui::Event::Text("a".into())],
        );
        input(&mut app, &ctx, vec![egui::Event::Copy]);
        assert!(app.result_open);
        assert!(!app.result_completed);
        app.accept_result();
        assert_eq!(app.users.active_records().len(), 1);
        input(&mut app, &ctx, vec![egui::Event::Copy]);
        assert_eq!(app.users.active_records().len(), 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn pause_toggle_and_empty_external_text_are_safe() {
        let mut app = test_app(test_path());
        app.external_text.clear();
        app.start();
        assert!(app.session.is_none());
        app.toggle_pause();
        app.external_text = "ab".into();
        app.start();
        app.toggle_pause();
        assert_eq!(
            app.session.as_ref().unwrap().snapshot(0).state,
            SessionState::Paused
        );
        app.toggle_pause();
        assert_eq!(
            app.session.as_ref().unwrap().snapshot(0).state,
            SessionState::Ready
        );
    }

    #[test]
    fn content_selection_is_independent_for_language_and_mode() {
        let mut app = test_app(test_path());
        app.content_language = ContentLanguage::English;
        app.selected_mode = TrainingMode::Lessons;
        app.select_content(1);
        assert_eq!(app.current_label(), "EN words");
        assert!(!app.advance_content());
        app.content_language = ContentLanguage::Russian;
        assert_eq!(app.current_label(), "RU home row");
        assert!(app.advance_content());
        app.selected_mode = TrainingMode::Phrases;
        assert_eq!(app.current_label(), "Кот спит.");
        app.content_language = ContentLanguage::English;
        assert_eq!(app.current_label(), "Cats nap.");
        app.selected_mode = TrainingMode::Lessons;
        assert_eq!(app.current_label(), "EN words");
    }

    #[test]
    fn missing_content_is_reported_and_does_not_start_a_session() {
        let mut app = test_app(test_path());
        app.catalog = ContentCatalog::fixture_empty();
        for language in [ContentLanguage::Russian, ContentLanguage::English] {
            app.content_language = language;
            for mode in [
                TrainingMode::Lessons,
                TrainingMode::Phrases,
                TrainingMode::Letters,
            ] {
                app.selected_mode = mode;
                assert!(app.current_label().contains("Нет"));
                app.start();
                assert!(app.session.is_none());
            }
        }
        app.show_keyboard = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let _ = draw_accessible(&mut app, &ctx, vec![]);
    }

    #[test]
    fn each_mode_starts_in_both_languages_and_all_keys_follow_layout() {
        let mut app = test_app(test_path());
        for language in [ContentLanguage::Russian, ContentLanguage::English] {
            app.content_language = language;
            for mode in [
                TrainingMode::Lessons,
                TrainingMode::Phrases,
                TrainingMode::Letters,
                TrainingMode::AllKeys,
            ] {
                app.selected_mode = mode;
                app.session = None;
                app.start();
                let session = app.session.as_ref().expect("mode has content");
                assert!(session.next_expected().is_some());
                app.stop();
            }
        }
        app.selected_mode = TrainingMode::AllKeys;
        app.content_language = ContentLanguage::Russian;
        app.english_layout = true;
        let english = app.generated_text();
        assert!(english.chars().all(|c| "asdfjkl; ".contains(c)));
        app.content_language = ContentLanguage::English;
        app.english_layout = false;
        let russian = app.generated_text();
        assert!(russian.chars().all(|c| "фывапролдж ".contains(c)));
    }

    #[test]
    fn content_language_persists_per_profile_independently_of_keyboard_layout() {
        let path = test_path();
        let mut app = test_app(path.clone());
        app.selected_mode = TrainingMode::Lessons;
        app.content_language = ContentLanguage::English;
        app.users.set_content_language("EN").unwrap();
        app.users.set_interface_language("EN").unwrap();
        app.sync_profile_language();
        app.english_layout = false;
        assert_eq!(app.current_label(), "EN home row");
        assert_eq!(localization::tr("options"), "Options");
        let english_id = app.users.active_profile_id().to_owned();
        let russian_id = app.users.create("Русский профиль").unwrap().id.clone();
        assert_eq!(app.users.active_profile().unwrap().content_language, "RU");
        app.users.select(&english_id).unwrap();
        app.sync_profile_language();
        assert_eq!(app.content_language, ContentLanguage::English);
        assert_eq!(app.users.active_profile().unwrap().content_language, "EN");
        let loaded = UserStore::load_from(path.clone()).unwrap();
        assert_eq!(loaded.active_profile().unwrap().content_language, "EN");
        app.users.select(&russian_id).unwrap();
        app.sync_profile_language();
        assert_eq!(app.content_language, ContentLanguage::Russian);
        assert_eq!(localization::tr("options"), "Опции");
        assert_eq!(app.users.active_profile().unwrap().content_language, "RU");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn long_external_session_saves_absolute_position_after_trimming() {
        let profile_path = test_path();
        let text_path = test_path();
        fs::write(&text_path, format!("{} конец", "я".repeat(200))).unwrap();
        let mut app = test_app(profile_path.clone());
        app.selected_mode = TrainingMode::ExternalText;
        app.load_external_text(text_path.clone());
        app.start();
        for _ in 0..150 {
            app.type_input_char('я', 100);
        }
        let session = app.session.as_ref().unwrap();
        assert_eq!(session.snapshot(100).position, 150);
        assert_eq!(session.retained_position(), 128);
        app.stop();
        app.accept_result();
        assert_eq!(
            app.users
                .active_profile()
                .unwrap()
                .external_text
                .as_ref()
                .unwrap()
                .position,
            300
        );
        app.restore_external_text();
        assert_eq!(app.external_text, format!("{} конец", "я".repeat(50)));
        fs::remove_file(text_path).unwrap();
        fs::remove_file(profile_path).unwrap();
    }

    #[test]
    fn external_stream_crosses_fragments_without_losing_position_or_history() {
        let profile_path = test_path();
        let text_path = test_path();
        let content = format!("{}\r\n  конец", "я".repeat(1300));
        fs::write(&text_path, &content).unwrap();
        let mut app = test_app(profile_path.clone());
        app.selected_mode = TrainingMode::ExternalText;
        app.load_external_text(text_path.clone());
        assert!(app.external_text.chars().count() <= 512);
        app.start();
        for _ in 0..1100 {
            app.type_input_char('я', 100);
            assert!(app.session_text.chars().count() <= 640);
            assert!(app.external_positions.len() <= 512);
        }
        app.type_input_char('x', 100);
        assert_eq!(app.external_start, 1100);
        app.stop();
        app.accept_result();
        assert_eq!(
            app.users
                .active_profile()
                .unwrap()
                .external_text
                .as_ref()
                .unwrap()
                .position,
            2200
        );
        app.restore_external_text();
        assert_eq!(app.external_text, format!("{} конец", "я".repeat(200)));
        fs::remove_file(text_path).unwrap();
        fs::remove_file(profile_path).unwrap();
    }

    #[test]
    fn external_progress_does_not_advance_when_file_changes_during_session() {
        let profile_path = test_path();
        let text_path = test_path();
        fs::write(&text_path, "я текст").unwrap();
        let mut app = test_app(profile_path.clone());
        app.selected_mode = TrainingMode::ExternalText;
        app.load_external_text(text_path.clone());
        app.start();
        app.type_input_char('я', 100);
        fs::write(&text_path, "другой текст").unwrap();
        app.stop();
        app.accept_result();
        assert!(app.external_error.contains("изменился"));
        assert_eq!(
            app.users
                .active_profile()
                .unwrap()
                .external_text
                .as_ref()
                .unwrap()
                .position,
            0
        );
        fs::remove_file(text_path).unwrap();
        fs::remove_file(profile_path).unwrap();
    }

    #[test]
    fn changed_external_file_can_be_restarted_from_menu() {
        let profile_path = test_path();
        let text_path = test_path();
        fs::write(&text_path, "старый текст").unwrap();
        let mut app = test_app(profile_path.clone());
        app.show_keyboard = false;
        app.selected_mode = TrainingMode::ExternalText;
        app.load_external_text(text_path.clone());
        fs::write(&text_path, "новый текст").unwrap();
        app.restore_external_text();
        assert!(app.external_error.contains("изменился"));
        app.users.set_interface_language("EN").unwrap();
        app.sync_profile_language();

        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let menu = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Exercise"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(menu)]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let restart = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Restart from the changed file"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(restart)]);
        assert!(app.external_error.is_empty());
        assert_eq!(app.external_text, "новый текст");
        assert_eq!(
            app.users
                .active_profile()
                .unwrap()
                .external_text
                .as_ref()
                .unwrap()
                .position,
            0
        );
        fs::remove_file(text_path).unwrap();
        fs::remove_file(profile_path).unwrap();
    }

    #[test]
    fn selecting_external_position_survives_reload_with_utf8_and_whitespace() {
        let profile_path = test_path();
        let text_path = test_path();
        fs::write(&text_path, "я\r\n  бв").unwrap();
        let mut app = test_app(profile_path.clone());
        app.selected_mode = TrainingMode::ExternalText;
        app.load_external_text(text_path.clone());
        app.set_external_position(2);
        assert_eq!(app.external_text, "бв");
        assert_eq!(
            app.users
                .active_profile()
                .unwrap()
                .external_text
                .as_ref()
                .unwrap()
                .position,
            6
        );
        app.restore_external_text();
        assert_eq!(app.external_text, "бв");
        fs::remove_file(text_path).unwrap();
        fs::remove_file(profile_path).unwrap();
    }

    #[test]
    fn invalid_saved_offset_and_deleted_file_do_not_overwrite_progress() {
        let profile_path = test_path();
        let text_path = test_path();
        fs::write(&text_path, "я текст").unwrap();
        let mut app = test_app(profile_path.clone());
        app.selected_mode = TrainingMode::ExternalText;
        app.load_external_text(text_path.clone());
        let mut saved = app
            .users
            .active_profile()
            .unwrap()
            .external_text
            .clone()
            .unwrap();
        saved.position = 1;
        app.users.set_external_progress(saved).unwrap();
        app.restore_external_text();
        assert!(app.external_error.contains("Позиция"));
        app.load_external_text(text_path.clone());
        app.start();
        app.type_input_char('я', 100);
        fs::remove_file(text_path).unwrap();
        app.stop();
        app.accept_result();
        assert!(app.external_error.contains("проверить файл"));
        assert_eq!(
            app.users
                .active_profile()
                .unwrap()
                .external_text
                .as_ref()
                .unwrap()
                .position,
            0
        );
        fs::remove_file(profile_path).unwrap();
    }

    #[test]
    fn external_file_loading_preserves_previous_text_on_error() {
        let path = test_path();
        let mut app = test_app(test_path());
        app.start();
        fs::write(&path, "Привет\r\n !").unwrap();
        app.load_external_text(path.clone());
        assert_eq!(app.external_text, "Привет !");
        assert!(app.external_error.is_empty());
        assert!(app.session.is_none());
        fs::write(&path, [0xCF, 0xF0, 0xE8]).unwrap();
        app.load_external_text(path.clone());
        assert!(app.external_error.contains("UTF-8"));
        assert_eq!(app.external_text, "Привет !");
        fs::write(&path, " \r\n ").unwrap();
        app.load_external_text(path.clone());
        assert!(!app.external_error.is_empty());
        assert_eq!(app.external_text, "Привет !");
        fs::remove_file(&path).unwrap();
        app.load_external_text(path);
        assert!(app.external_error.contains("Не удалось открыть файл"));
        assert_eq!(app.external_text, "Привет !");
    }

    #[test]
    fn external_progress_roundtrips_per_profile_and_rejects_changed_file() {
        let store_path = test_path();
        let text_path = store_path.with_extension("txt");
        fs::write(&text_path, "я\r\n  бв").unwrap();
        let mut app = test_app(store_path.clone());
        app.load_external_text(text_path.clone());
        app.start();
        app.type_input_char('я', 1);
        app.type_input_char('x', 2);
        app.stop();
        app.accept_result();
        app.accept_result();
        assert_eq!(app.external_text, " бв");

        let mut reopened = test_app(store_path.clone());
        reopened.users = UserStore::load_from(&store_path).unwrap();
        reopened.restore_external_text();
        assert_eq!(reopened.external_text, " бв");
        assert_eq!(
            reopened
                .users
                .active_profile()
                .unwrap()
                .external_text
                .as_ref()
                .unwrap()
                .position,
            2
        );
        reopened.users.create("Другой").unwrap();
        reopened.restore_external_text();
        assert!(reopened.external_text.is_empty());
        let original = reopened.users.profiles()[0].id.clone();
        reopened.users.select(&original).unwrap();
        fs::write(&text_path, "я\r\n  бг").unwrap();
        reopened.restore_external_text();
        assert!(reopened.external_text.is_empty());
        assert!(reopened.external_error.contains("изменился"));
        fs::remove_file(text_path).unwrap();
        reopened.restore_external_text();
        assert!(reopened.external_error.contains("Не удалось открыть"));
    }

    #[test]
    fn keyboard_preference_roundtrips_and_missing_defaults_to_visible() {
        let path = test_path().with_extension("txt");
        assert!(load_keyboard_setting_from(Some(&path)));
        assert!(load_keyboard_setting_from(None));
        save_keyboard_setting_to(Some(&path), false);
        assert!(!load_keyboard_setting_from(Some(&path)));
        save_keyboard_setting_to(Some(&path), true);
        assert!(load_keyboard_setting_from(Some(&path)));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn default_keyboard_setting_path_follows_appdata() {
        let expected = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|base| base.join("Staminator").join("settings.txt"));
        assert_eq!(keyboard_setting_path(), expected);
    }

    #[test]
    fn generated_training_continues_and_phrases_advance() {
        let mut app = test_app(test_path());
        app.selected_mode = TrainingMode::AllKeys;
        app.start();
        let first = app.session_text.clone();
        for ch in first.chars() {
            app.type_input_char(ch, 0);
        }
        assert!(app.session_text.len() > first.len());
        assert_eq!(
            app.session.as_ref().unwrap().snapshot(0).state,
            SessionState::Running
        );
        app.selected_mode = TrainingMode::Letters;
        app.start();
        assert!(!app.session_text.is_empty());
        app.selected_mode = TrainingMode::Phrases;
        app.start();
        let first = app.session_text.clone();
        let first_phrase_index = app.selected_content[app.content_slot()];
        for ch in first.chars() {
            app.type_input_char(ch, 0);
        }
        assert_ne!(app.selected_content[app.content_slot()], first_phrase_index);
        assert!(
            app.selected_content[app.content_slot()] < app.catalog.phrases(app.language()).len()
        );
        assert!(app.session_text.len() > first.len());
    }

    #[test]
    fn main_view_renders_ready_active_paused_and_result_states() {
        let path = test_path();
        let mut app = test_app(path.clone());
        app.show_keyboard = false;
        let ctx = egui::Context::default();
        let render = |app: &mut StaminatorApp| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui));
            assert!(!output.shapes.is_empty());
            output.textures_delta.clear();
        };
        render(&mut app);
        app.start();
        render(&mut app);
        app.type_input_char('a', 0);
        app.toggle_pause();
        render(&mut app);
        app.stop();
        render(&mut app);
        assert!(app.result_open);
        app.accept_result();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn accessible_start_and_stop_buttons_record_the_session() {
        let path = test_path();
        let mut app = test_app(path.clone());
        app.show_keyboard = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let start = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("СТАРТ"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(start)]);
        assert!(app.session.is_some());
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let stop = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("СТОП"))
            .unwrap()
            .0;
        app.type_input_char('a', 0);
        draw_accessible(&mut app, &ctx, vec![accessible_click(stop)]);
        assert!(app.result_open);
        app.accept_result();
        assert_eq!(app.users.active_records().len(), 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn accessible_menu_opens_lesson_chooser_and_selects_lesson() {
        let mut app = test_app(test_path());
        app.show_keyboard = false;
        app.content_language = ContentLanguage::English;
        app.selected_mode = TrainingMode::Lessons;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let menu = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Занятие"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(menu)]);
        assert!(app.chooser_open);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let lesson = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("2. EN words"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(lesson)]);
        assert_eq!(app.current_label(), "EN home row");
        assert!(app.chooser_open);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        assert_eq!(app.chooser_preview, Some(1));
        let choose = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Выбрать…"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(choose)]);
        assert_eq!(app.current_label(), "EN words");
        assert!(!app.chooser_open);
    }

    #[test]
    fn double_click_selects_and_closes_chooser() {
        let mut app = test_app(test_path());
        app.show_keyboard = false;
        app.selected_mode = TrainingMode::Lessons;
        app.content_language = ContentLanguage::English;
        app.chooser_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let bounds = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("2. EN words"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let pos = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        for _ in 0..2 {
            draw_accessible(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            draw_accessible(
                &mut app,
                &ctx,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(app.current_label(), "EN words");
        assert!(!app.chooser_open);
        assert_eq!(app.chooser_preview, None);
    }

    #[test]
    fn chooser_filter_limits_preview_and_actions_to_matching_lessons() {
        let mut app = test_app(test_path());
        app.show_keyboard = false;
        app.selected_mode = TrainingMode::Lessons;
        app.content_language = ContentLanguage::English;
        app.chooser_open = true;
        app.chooser_filter = "words".into();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        assert!(
            nodes
                .iter()
                .any(|(_, node)| node.label() == Some("2. EN words"))
        );
        let choose = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Выбрать…"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(choose)]);
        assert_eq!(app.current_label(), "EN words");
        app.chooser_open = true;
        app.chooser_filter = "no matching lesson".into();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        assert!(
            !nodes
                .iter()
                .any(|(_, node)| node.label() == Some("Выбрать…"))
        );
        for label in ["Изменить выбранный урок", "Экспорт урока…"]
        {
            assert!(
                nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some(label))
                    .unwrap()
                    .1
                    .is_disabled()
            );
        }
        assert_eq!(app.current_label(), "EN words");
    }

    #[test]
    fn obsolete_lesson_selection_falls_back_and_starts_training() {
        let mut app = test_app(test_path());
        app.selected_mode = TrainingMode::Lessons;
        app.content_language = ContentLanguage::English;
        app.selected_lessons[1] = Some("obsolete.bundled.lesson".into());
        assert_eq!(app.current_index(), 0);
        assert_eq!(app.current_label(), "EN home row");
        app.start();
        assert!(app.session.is_some());
        assert_eq!(app.session_text, "asdf jkl;");
    }

    #[test]
    fn chooser_edits_the_previewed_lesson_without_selecting_it() {
        let root = test_path();
        let users = UserStore::load_database(root.with_extension("sqlite"), &root).unwrap();
        let mut database = users.database().unwrap().unwrap();
        let lesson = |id: &str, text: &str| stamina_data::JsonLesson {
            version: 1,
            id: id.into(),
            title: id.into(),
            language: "ru".into(),
            description: Some(format!("Description {id}")),
            text: text.into(),
        };
        database
            .save_lesson(&lesson("first", "first text"), true)
            .unwrap();
        database
            .save_lesson(&lesson("second", "second text"), true)
            .unwrap();
        let lessons = database.list_lessons().unwrap();
        drop(database);
        let mut app = test_app(test_path());
        app.users = users;
        app.show_keyboard = false;
        app.selected_mode = TrainingMode::Lessons;
        app.catalog.set_stored_lessons(lessons);
        app.selected_lessons[0] = Some("first".into());
        app.chooser_open = true;
        app.chooser_preview = Some(1);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let edit = nodes
            .iter()
            .find(|(_, n)| n.label() == Some(localization::tr("edit-lesson").as_str()))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(edit)]);
        let editor = app.lesson_editor.as_ref().unwrap();
        assert_eq!(editor.lesson.id, "second");
        assert_eq!(
            editor.lesson.description.as_deref(),
            Some("Description second")
        );
        assert_eq!(app.selected_lessons[0].as_deref(), Some("first"));
    }

    #[test]
    fn accessible_lesson_chooser_shows_and_clears_transfer_error() {
        let mut app = test_app(test_path());
        app.selected_mode = TrainingMode::Lessons;
        app.chooser_open = true;
        app.show_keyboard = false;
        app.lesson_transfer_error = "Не удалось импортировать урок: неверный JSON".into();
        app.profile_error = "Не удалось загрузить профиль".into();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let initial = draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        assert!(initial.iter().chain(nodes.iter()).any(|(_, node)| {
            node.value() == Some("Не удалось импортировать урок: неверный JSON")
        }));
        let clear = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Очистить сообщение"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(clear)]);

        assert!(app.lesson_transfer_error.is_empty());
        assert_eq!(app.profile_error, "Не удалось загрузить профиль");
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        assert!(
            !nodes
                .iter()
                .any(|(_, node)| node.value()
                    == Some("Не удалось импортировать урок: неверный JSON"))
        );
    }

    #[test]
    fn phrase_chooser_does_not_show_lesson_transfer_errors() {
        let mut app = test_app(test_path());
        app.selected_mode = TrainingMode::Phrases;
        app.chooser_open = true;
        app.show_keyboard = false;
        app.lesson_transfer_error = "Ошибка урока".into();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let first = draw_accessible(&mut app, &ctx, vec![]);
        let second = draw_accessible(&mut app, &ctx, vec![]);
        assert!(
            !first
                .iter()
                .chain(second.iter())
                .any(|(_, node)| node.value() == Some("Ошибка урока"))
        );
        assert_eq!(app.lesson_transfer_error, "Ошибка урока");
    }

    #[test]
    fn accessible_lesson_refresh_shows_errors_and_keeps_selection() {
        let root = test_path().with_extension("");
        fs::create_dir_all(&root).unwrap();
        let file = |id: &str| {
            format!(r#"{{"version":1,"id":"{id}","title":"{id}","language":"en","text":"abc"}}"#)
        };
        fs::write(root.join("b.json"), file("b")).unwrap();
        let mut app = test_app(test_path());
        app.catalog
            .set_lesson_roots(vec![root.clone(), root.join("user")]);
        app.selected_mode = TrainingMode::Lessons;
        app.content_language = ContentLanguage::English;
        app.chooser_open = true;
        app.lesson_transfer_error = "Старая ошибка".into();
        app.show_keyboard = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let refresh = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Обновить список уроков"))
            .unwrap()
            .0;
        fs::write(root.join("a.json"), file("a")).unwrap();
        fs::write(root.join("bad.json"), "{").unwrap();
        draw_accessible(&mut app, &ctx, vec![accessible_click(refresh)]);
        assert!(app.lesson_transfer_error.is_empty());
        assert_eq!(app.current_label(), "b");
        assert!(app.catalog.lesson_diagnostics().contains("bad.json"));
        draw_accessible(&mut app, &ctx, vec![]);
        assert_eq!(app.catalog.lessons(ContentLanguage::English).len(), 2);
        fs::write(root.join("bad.json"), file("bad")).unwrap();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let refresh = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Обновить список уроков"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(refresh)]);
        assert!(app.catalog.lesson_diagnostics().is_empty());
        assert_eq!(app.catalog.lessons(ContentLanguage::English).len(), 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_chooser_shows_file_error_until_successful_refresh() {
        let root = test_path().with_extension("sqlite-chooser-diagnostics");
        let lesson_dir = root.join("lessons");
        fs::create_dir_all(&lesson_dir).unwrap();
        let broken = lesson_dir.join("broken.json");
        fs::write(&broken, "{").unwrap();
        let mut app = database_test_app(root.join("profiles.sqlite3"));
        let mut db = app.users.database().unwrap().unwrap();
        let saved = stamina_data::JsonLesson {
            version: 1,
            id: "saved".into(),
            title: "Saved".into(),
            language: "en".into(),
            description: None,
            text: "abc".into(),
        };
        db.create_user_lesson(&saved).unwrap();
        app.catalog.set_lesson_roots(vec![lesson_dir.clone()]);
        sync_catalog_with_database(&mut app.catalog, &mut db).unwrap();
        assert!(app.catalog.lesson_diagnostics().contains("broken.json"));
        assert_eq!(
            app.catalog.lessons(ContentLanguage::English)[0].name,
            "Saved"
        );
        drop(db);
        app.selected_mode = TrainingMode::Lessons;
        app.content_language = ContentLanguage::English;
        app.chooser_open = true;
        app.show_keyboard = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let initial = draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        assert!(initial.iter().chain(nodes.iter()).any(|(_, node)| {
            node.value()
                .is_some_and(|label| label.contains("broken.json"))
        }));
        fs::write(
            &broken,
            r#"{"version":1,"id":"fixed","title":"Fixed","language":"en","text":"abc"}"#,
        )
        .unwrap();
        let refresh = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Обновить список уроков"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(refresh)]);
        assert!(app.catalog.lesson_diagnostics().is_empty());
        assert_eq!(app.catalog.lessons(ContentLanguage::English).len(), 2);
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_refresh_keeps_displayed_lessons_if_database_cannot_open() {
        let root = test_path().with_extension("sqlite-refresh-error");
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("profiles.sqlite3");
        let mut app = database_test_app(database_path.clone());
        let saved = stamina_data::JsonLesson {
            version: 1,
            id: "saved".into(),
            title: "Saved".into(),
            language: "en".into(),
            description: None,
            text: "abc".into(),
        };
        app.catalog.set_stored_lessons(vec![(saved, false)]);
        fs::remove_file(&database_path).unwrap();
        fs::create_dir(&database_path).unwrap();
        app.selected_mode = TrainingMode::Lessons;
        app.content_language = ContentLanguage::English;
        app.chooser_open = true;
        app.show_keyboard = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let refresh = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Обновить список уроков"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(refresh)]);
        assert_eq!(
            app.catalog.lessons(ContentLanguage::English)[0].name,
            "Saved"
        );
        assert!(
            app.lesson_transfer_error
                .contains("Не удалось открыть базу уроков")
        );
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refresh_after_lesson_removal_falls_back_and_recovers_selection() {
        let root = test_path().with_extension("");
        fs::create_dir_all(&root).unwrap();
        let lesson = |id: &str| {
            format!(r#"{{"version":1,"id":"{id}","title":"{id}","language":"en","text":"abc"}}"#)
        };
        fs::write(root.join("first.json"), lesson("first")).unwrap();
        fs::write(root.join("second.json"), lesson("second")).unwrap();
        let profile_path = test_path();
        let mut app = test_app(profile_path.clone());
        app.catalog.set_lesson_roots(vec![root.clone()]);
        app.selected_mode = TrainingMode::Lessons;
        app.content_language = ContentLanguage::English;
        app.select_content(1);
        fs::remove_file(root.join("second.json")).unwrap();
        app.catalog.refresh_lessons();
        assert_eq!(app.current_label(), "first");
        fs::write(root.join("second.json"), lesson("second")).unwrap();
        app.catalog.refresh_lessons();
        assert_eq!(app.current_label(), "second");
        app.start();
        app.type_input_char('a', 1);
        app.stop();
        app.accept_result();
        assert_eq!(app.users.active_records().len(), 1);
        fs::remove_file(profile_path).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accessible_mode_menu_changes_mode_and_layout() {
        let mut app = test_app(test_path());
        app.show_keyboard = false;
        app.content_language = ContentLanguage::English;
        app.selected_mode = TrainingMode::Lessons;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let menu = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Режим"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(menu)]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let phrases = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Фразы"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(phrases)]);
        assert_eq!(app.current_label(), "Cats nap.");
        assert!(app.selected_mode == TrainingMode::Phrases);
        draw_accessible(&mut app, &ctx, vec![accessible_click(menu)]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let layout = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Раскладка"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(layout)]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let russian = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Русская"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(russian)]);
        assert_eq!(app.current_label(), "Cats nap.");
        assert!(!app.english_layout);
    }

    #[test]
    fn interface_language_menu_preserves_session_and_reloads_profile() {
        let path = test_path();
        let mut app = test_app(path.clone());
        app.show_keyboard = false;
        app.start();
        let before = app.session_text.clone();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let options = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Опции"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(options)]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let language = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Язык интерфейса"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(language)]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let english = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("English"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(english)]);
        assert_eq!(app.session_text, before);
        assert!(app.session.is_some());
        assert!(app.english_layout);
        assert_eq!(app.content_language, ContentLanguage::Russian);
        assert_eq!(
            UserStore::load_from(path)
                .unwrap()
                .active_profile()
                .unwrap()
                .interface_language,
            "EN"
        );
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        assert!(nodes.iter().any(|(_, n)| n.label() == Some("Options")));
    }

    #[test]
    fn accessible_content_language_menu_saves_choice_and_locks_during_session() {
        let path = test_path();
        let mut app = test_app(path.clone());
        app.show_keyboard = false;
        app.selected_mode = TrainingMode::Lessons;
        app.chooser_open = true;
        app.chooser_preview = Some(1);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let mode = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Режим"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(mode)]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let language = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Язык текста"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(language)]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let english = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("English"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(english)]);

        assert_eq!(app.chooser_preview, None);
        assert_eq!(app.content_language, ContentLanguage::English);
        assert_eq!(app.users.active_profile().unwrap().content_language, "EN");
        assert_eq!(
            UserStore::load_from(path.clone())
                .unwrap()
                .active_profile()
                .unwrap()
                .content_language,
            "EN"
        );

        app.start();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let mode = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Режим"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(mode)]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        if let Some(language) = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Язык текста"))
            .map(|(id, _)| *id)
        {
            draw_accessible(&mut app, &ctx, vec![accessible_click(language)]);
            let nodes = draw_accessible(&mut app, &ctx, vec![]);
            if let Some(russian) = nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Русский"))
                .map(|(id, _)| *id)
            {
                draw_accessible(&mut app, &ctx, vec![accessible_click(russian)]);
            }
        }
        assert_eq!(app.content_language, ContentLanguage::English);
        assert_eq!(app.users.active_profile().unwrap().content_language, "EN");
        assert!(app.session.is_some());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn accessible_profiles_create_select_and_delete() {
        let path = test_path();
        let mut app = test_app(path.clone());
        app.show_keyboard = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let profiles = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Пользователи"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(profiles)]);
        assert!(app.profiles_open);
        app.profile_name_input = "Alice".into();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let create = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Создать"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(create)]);
        assert_eq!(app.users.active_name(), "Alice");
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let original = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Пользователь"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(original)]);
        assert_eq!(app.users.active_name(), "Пользователь");
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let delete = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Удалить текущий"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(delete)]);
        assert_eq!(app.users.active_name(), "Alice");
        assert_eq!(UserStore::load_from(&path).unwrap().profiles().len(), 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn accessible_profiles_validate_and_rename() {
        let path = test_path();
        let mut app = test_app(path.clone());
        app.show_keyboard = false;
        app.profiles_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let create = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Создать"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(create)]);
        assert!(!app.profile_error.is_empty());
        app.profile_name_input = "Renamed".into();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let rename = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Переименовать"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(rename)]);
        assert_eq!(app.users.active_name(), "Renamed");
        assert!(app.profile_error.is_empty());
        assert_eq!(
            UserStore::load_from(&path).unwrap().active_name(),
            "Renamed"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn accessible_keyboard_option_saves_visibility() {
        let settings = test_path().with_extension("txt");
        let mut app = test_app(test_path());
        app.show_keyboard = false;
        app.keyboard_setting_path = Some(settings.clone());
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let options = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Опции"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(options)]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let keyboard = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Показывать клавиатуру"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(keyboard)]);
        assert!(app.show_keyboard);
        assert!(load_keyboard_setting_from(Some(&settings)));
        fs::remove_file(settings).unwrap();
    }

    #[test]
    fn accessible_progress_opens_current_profile_history() {
        let mut app = test_app(test_path());
        app.show_keyboard = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let progress = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Прогресс"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(progress)]);
        assert!(app.progress_open);
        draw_accessible(&mut app, &ctx, vec![]);
        assert_eq!(app.users.active_name(), "Пользователь");
    }

    #[test]
    fn accessible_result_next_and_retry_start_expected_lesson() {
        let path = test_path();
        let mut app = test_app(path.clone());
        app.show_keyboard = false;
        app.content_language = ContentLanguage::English;
        app.selected_mode = TrainingMode::Lessons;
        app.start();
        for ch in app.session_text.clone().chars() {
            app.type_input_char(ch, 0);
        }
        app.open_stopped_result();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let next = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Далее"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(next)]);
        assert_eq!(app.current_label(), "EN words");
        assert!(!app.result_open);
        assert_eq!(
            app.session.as_ref().unwrap().snapshot(0).state,
            SessionState::Ready
        );
        app.stop();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let retry = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Повторить"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(retry)]);
        assert!(!app.result_open);
        assert_eq!(app.current_label(), "EN words");
        assert_eq!(
            app.session.as_ref().unwrap().snapshot(0).state,
            SessionState::Ready
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn accessible_result_close_preserves_record() {
        let path = test_path();
        let mut app = test_app(path.clone());
        app.show_keyboard = false;
        app.start();
        app.type_input_char('a', 0);
        app.stop();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        draw_accessible(&mut app, &ctx, vec![]);
        let nodes = draw_accessible(&mut app, &ctx, vec![]);
        let close = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Закрыть"))
            .unwrap()
            .0;
        draw_accessible(&mut app, &ctx, vec![accessible_click(close)]);
        assert!(!app.result_open);
        assert_eq!(
            UserStore::load_from(&path).unwrap().active_records().len(),
            1
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn failure_to_save_result_is_reported() {
        let blocker = test_path();
        fs::write(&blocker, "not a directory").unwrap();
        let mut app = test_app(blocker.join("profiles.json"));
        app.start();
        app.type_input_char('a', 0);
        app.stop();
        app.accept_result();
        assert!(app.profile_error.contains("Не удалось сохранить результат"));
        assert!(!app.result_recorded);
        assert!(app.result_open);
        fs::remove_file(&blocker).unwrap();
        fs::create_dir(&blocker).unwrap();
        assert!(app.accept_result());
        assert_eq!(app.users.active_records().len(), 1);
        fs::remove_file(blocker.join("profiles.json")).unwrap();
        fs::remove_dir(blocker).unwrap();
    }

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
            let mut sequence = generated_modes::PhraseSequence::new(2, 0, 1);
            let chars: Vec<_> = first.chars().collect();
            for ch in &chars[..chars.len() - 1] {
                assert_eq!(session.type_char(*ch, 100), Keystroke::Correct);
            }
            let last = *chars.last().unwrap();
            assert_eq!(
                append_next_phrase(&mut session, &phrases, &mut sequence, &mut index, last, 200),
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

    #[test]
    fn chosen_phrase_starts_each_language_sequence_and_cycles_without_stopping() {
        for phrases_text in [["а1 ", "б2 ", "в3 "], ["a1 ", "b2 ", "c3 "]] {
            let phrases: Vec<_> = phrases_text
                .iter()
                .enumerate()
                .map(|(index, text)| Exercise {
                    name: index.to_string(),
                    text: (*text).into(),
                })
                .collect();
            let start = 2;
            let mut current = start;
            let mut sequence = generated_modes::PhraseSequence::new(phrases.len(), start, 73);
            let mut session = Session::new(phrases[current].text.clone());
            let mut seen = vec![current];

            for _ in 0..5 {
                let chars: Vec<_> = phrases[current].text.chars().collect();
                for (position, ch) in chars.iter().enumerate() {
                    if position + 1 == chars.len() {
                        let next_text = append_next_phrase(
                            &mut session,
                            &phrases,
                            &mut sequence,
                            &mut current,
                            *ch,
                            100,
                        )
                        .expect("phrase source should append at each boundary");
                        seen.push(current);
                        assert_eq!(next_text, phrases[current].text);
                    }
                    assert_eq!(session.type_char(*ch, 100), Keystroke::Correct);
                }
            }
            assert_eq!(session.snapshot(100).state, SessionState::Running);
            for cycle in seen.chunks(3) {
                let mut cycle = cycle.to_vec();
                cycle.sort_unstable();
                assert_eq!(cycle, vec![0, 1, 2]);
            }
        }
    }

    #[test]
    fn profile_json_roundtrips_between_database_apps_and_rejects_duplicate() {
        let root = test_path().with_extension("profile-transfer");
        fs::create_dir_all(&root).unwrap();
        let mut source = database_test_app(root.join("source.sqlite3"));
        let source_id = source.users.active_profile_id().to_owned();
        source.users.rename(&source_id, "Transferred").unwrap();
        source.users.set_content_language("EN").unwrap();
        let profile_path = root.join("profile.json");
        source.export_profile_to(&profile_path).unwrap();
        let expected = source.users.active_profile().unwrap().clone();

        let mut destination = database_test_app(root.join("destination.sqlite3"));
        destination.import_profile_from(&profile_path).unwrap();
        assert_eq!(destination.users.active_profile(), Some(&expected));
        assert_eq!(destination.content_language, ContentLanguage::English);
        assert!(destination.import_profile_from(&profile_path).is_err());
        assert_eq!(destination.users.profiles().len(), 2);
        let invalid_path = root.join("invalid-profile.json");
        fs::write(&invalid_path, b"not json").unwrap();
        assert!(destination.import_profile_from(&invalid_path).is_err());
        assert_eq!(destination.users.active_profile(), Some(&expected));
        assert_eq!(destination.content_language, ContentLanguage::English);

        drop(source);
        drop(destination);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manual_session_reason_survives_database_restart_and_profile_export() {
        let root = test_path().with_extension("reason-transfer");
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("profile.sqlite3");
        let mut app = database_test_app(database_path.clone());
        app.selected_mode = TrainingMode::ExternalText;
        app.external_text = "ab".into();
        app.external_path = Some(PathBuf::from("example.txt"));
        app.start();
        app.type_input_char('a', 100);
        app.stop();
        assert!(app.accept_result());
        let profile = root.join("profile.json");
        app.export_profile_to(&profile).unwrap();
        assert_eq!(
            storage_transfer::import_profile_json(&profile)
                .unwrap()
                .results[0]
                .stop_reason,
            "manual"
        );
        let reopened = UserStore::load_database(database_path, &root.join("missing.json")).unwrap();
        assert_eq!(reopened.active_records()[0].stop_reason, "manual");
        app.start();
        app.type_input_char('a', 100);
        app.type_input_char('b', 200);
        app.open_stopped_result();
        assert!(app.accept_result());
        assert_eq!(app.users.active_records()[1].stop_reason, "completed");
        app.start();
        app.type_input_char('a', 0);
        app.check_time_limit(300_000);
        assert!(app.accept_result());
        assert_eq!(app.users.active_records()[2].stop_reason, "time_limit");
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restore_backup_loads_lessons_groups_settings_and_resets_session() {
        let root = test_path().with_extension("restore-backup");
        fs::create_dir_all(&root).unwrap();
        let mut source = database_test_app(root.join("source.sqlite3"));
        source.users.set_content_language("EN").unwrap();
        let mut database = source.users.database().unwrap().unwrap();
        let lesson = stamina_data::storage::LessonDocument {
            version: 1,
            id: "backup.lesson".into(),
            title: "Backup lesson".into(),
            language: "en".into(),
            description: None,
            text: "backup text".into(),
        };
        database.save_lesson(&lesson, true).unwrap();
        database
            .set_app_setting("lesson_group:backup.lesson", "Imported/From backup")
            .unwrap();
        database.set_app_setting("show_keyboard", "false").unwrap();
        let backup = root.join("backup.sqlite3");
        storage_transfer::export_database_backup(&database, &backup).unwrap();
        drop(database);

        let mut app = database_test_app(root.join("destination.sqlite3"));
        app.session = Some(Session::new("stale session"));
        app.result_open = true;
        let safety = app.restore_backup_from(&backup).unwrap();

        assert!(safety.is_file());
        assert!(app.session.is_none());
        assert!(!app.result_open);
        assert!(!app.show_keyboard);
        assert_eq!(app.content_language, ContentLanguage::English);
        assert_eq!(
            app.catalog
                .lesson_groups()
                .get("backup.lesson")
                .map(String::as_str),
            Some("Imported/From backup")
        );
        assert_eq!(
            app.catalog.lessons(ContentLanguage::English)[0].name,
            "Imported/From backup / Backup lesson"
        );

        drop(source);
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }
}
