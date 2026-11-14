//! Keyboard collection and per-profile selection UI.
use crate::{
    keyboard,
    keyboard_editor::KeyboardEditor,
    keyboard_settings::{Assignments, FingerScheme, Preferences},
    user_store::UserStore,
};
use egui::Context;
use staminator_keyboard_schema::{
    Finger, KeyboardSchema,
    storage::{KeyboardCollection, KeyboardEntry},
};
use std::path::PathBuf;

pub struct KeyboardManager {
    root: PathBuf,
    collection: Option<KeyboardCollection>,
    entries: Vec<KeyboardEntry>,
    profile_id: String,
    prefs: Preferences,
    pub schema: KeyboardSchema,
    open: bool,
    editor: Option<KeyboardEditor>,
    editor_base_schema: Option<KeyboardSchema>,
    error: String,
    scheme_name: String,
    scheme_draft: Option<Assignments>,
}

impl KeyboardManager {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            collection: None,
            entries: Vec::new(),
            profile_id: String::new(),
            prefs: Preferences::default(),
            schema: keyboard::default_schema(),
            open: false,
            editor: None,
            editor_base_schema: None,
            error: String::new(),
            scheme_name: String::new(),
            scheme_draft: None,
        }
    }
    pub fn sync(&mut self, users: &mut UserStore) -> bool {
        if self.profile_id != users.active_profile_id() {
            self.error.clear();
        }
        if self.collection.is_none() {
            match KeyboardCollection::new(&self.root) {
                Ok(collection) => {
                    match collection.list() {
                        Ok(entries) if entries.is_empty() => {
                            if let Err(e) = collection.save_new(&keyboard::default_schema()) {
                                self.error = e.to_string();
                            }
                        }
                        Ok(_) => {}
                        Err(e) => self.error = e.to_string(),
                    }
                    if let Err(error) = crate::bundled_keyboards::ensure(&self.root, &collection) {
                        self.error = error;
                    }
                    self.collection = Some(collection);
                    self.refresh();
                }
                Err(e) => {
                    self.error = e.to_string();
                    return false;
                }
            }
        }
        if self.profile_id == users.active_profile_id() {
            return false;
        }
        self.editor = None;
        self.editor_base_schema = None;
        self.scheme_draft = None;
        self.scheme_name.clear();
        self.profile_id = users.active_profile_id().to_owned();
        self.prefs = match users.keyboard_settings() {
            Ok(s) => Preferences::decode(&s),
            Err(e) => {
                self.error = e.to_string();
                Preferences::default()
            }
        };
        self.load_selected();
        true
    }
    fn refresh(&mut self) {
        if let Some(c) = &self.collection {
            match c.list() {
                Ok(e) => self.entries = e,
                Err(e) => self.error = e.to_string(),
            }
        }
    }
    fn load_selected(&mut self) {
        if !self.entries.iter().any(|e| e.id == self.prefs.keyboard) {
            self.prefs.keyboard = self
                .entries
                .first()
                .map_or_else(String::new, |e| e.id.clone());
        }
        if let Some(c) = &self.collection {
            match c.load(&self.prefs.keyboard) {
                Ok(schema) => {
                    // The original bundled ANSI geometry was stored as a user file.
                    // Upgrade its exact contents in memory; edited schemas stay untouched.
                    self.schema = if is_original_ansi(&schema) {
                        keyboard::default_schema()
                    } else {
                        schema
                    };
                }
                Err(e) => {
                    self.error = e.to_string();
                    self.schema = keyboard::default_schema();
                }
            }
        }
    }
    fn persist(&mut self, users: &mut UserStore) {
        match self
            .prefs
            .encode()
            .and_then(|s| users.set_keyboard_settings(&s))
        {
            Ok(()) => {}
            Err(e) => self.error = e.to_string(),
        }
    }
    pub fn assignments(&self) -> Assignments {
        let builtin = self.prefs.finger_scheme.is_empty()
            || matches!(self.prefs.finger_scheme.as_str(), "default" | "alternate");
        let mut assignments = match self.prefs.finger_scheme.as_str() {
            "alternate" => keyboard::alternative_assignments(),
            "" | "default" => keyboard::default_assignments(),
            name => self
                .prefs
                .schemes
                .get(name)
                .map_or_else(keyboard::default_assignments, |s| s.assignments.clone()),
        };
        if builtin {
            for key in &self.schema.keys {
                if let Some(finger) = key.finger {
                    assignments.entry(key.physical_id.clone()).or_insert(finger);
                }
            }
        }
        assignments
    }
    pub fn open(&mut self) {
        self.refresh();
        self.open = true;
    }
    pub fn editing(&self) -> bool {
        self.open || self.editor.is_some()
    }
    pub fn show(&mut self, ctx: &Context, users: &mut UserStore) -> bool {
        let mut changed = false;
        if self.open {
            let mut open = self.open;
            egui::Window::new("Клавиатуры / Keyboards").open(&mut open).default_width(620.0).show(ctx, |ui| {
                ui.label(format!("Папка / Folder: {}", self.root.display()));
                ui.heading("Клавиатуры / Keyboards");
                egui::ScrollArea::vertical().id_salt("keyboard_entries").max_height(180.0).show(ui, |ui| {
                for entry in self.entries.clone() {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(self.prefs.keyboard == entry.id, &entry.name).clicked() {
                            self.error.clear(); self.prefs.keyboard = entry.id.clone(); self.load_selected(); self.persist(users); changed = true;
                        }
                        if ui.button("Экспорт ZIP / Export").clicked()
                            && let Some(path) = rfd::FileDialog::new().add_filter("ZIP", &["zip"]).set_file_name(format!("{}.zip", entry.id)).save_file()
                                && let Some(c) = &self.collection { match c.export_zip(&entry.id, path) { Ok(()) => self.error.clear(), Err(e) => self.error = e.to_string() } }
                        if ui.add_enabled(self.entries.len() > 1, egui::Button::new("Удалить / Delete")).clicked() {
                            let confirm = rfd::MessageDialog::new().set_title("Delete keyboard?").set_description(format!("Удалить клавиатуру / Delete keyboard: {}?", entry.name)).set_buttons(rfd::MessageButtons::YesNo).show();
                            if confirm == rfd::MessageDialogResult::Yes
                                && let Some(c) = &self.collection { if let Err(e) = c.remove(&entry.id) { self.error = e.to_string(); } else { self.error.clear(); self.refresh(); self.load_selected(); self.persist(users); changed = true; } }
                        }
                    });
                }
                });
                ui.horizontal(|ui| {
                    if ui.button("Импорт ZIP / Import ZIP").clicked()
                        && let Some(path) = rfd::FileDialog::new().add_filter("ZIP", &["zip"]).pick_file()
                            && let Some(c) = &self.collection { match c.import_zip(path) { Ok(entry) => { self.error.clear(); self.refresh(); self.prefs.keyboard = entry.id; self.load_selected(); self.persist(users); changed = true; }, Err(e) => self.error = e.to_string() } }
                    if ui.button("Новая / New").clicked() { let schema=crate::keyboard_editor::blank_schema("New keyboard", 16.0, 5.0); self.editor_base_schema=Some(schema.clone()); self.editor = Some(KeyboardEditor::new(schema)); }
                    if ui.button("Редактировать копию / Edit copy").clicked() { self.editor_base_schema=Some(self.schema.clone()); self.editor = Some(KeyboardEditor::new(self.schema.clone())); }
                });
                ui.separator();
                ui.heading("Пальцы / Finger assignments");
                ui.horizontal_wrapped(|ui| {
                    for (id, label) in [("default", "Обычная / Standard"), ("alternate", "Альтернативная / Alternative")] {
                        if ui.selectable_label((self.prefs.finger_scheme.is_empty() && id == "default") || self.prefs.finger_scheme == id, label).clicked() { self.error.clear(); self.prefs.finger_scheme = id.into(); self.persist(users); changed = true; }
                    }
                    for name in self.prefs.schemes.keys().cloned().collect::<Vec<_>>() {
                        if ui.selectable_label(self.prefs.finger_scheme == name, &name).clicked() { self.error.clear(); self.prefs.finger_scheme = name; self.persist(users); changed = true; }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Новая схема / New scheme").clicked() { self.scheme_name.clear(); self.scheme_draft = Some(self.assignments()); }
                    if ui.button("Импорт схемы / Import scheme").clicked()
                        && let Some(path) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                            match FingerScheme::load(&path) { Ok(s) if s.name != "default" && s.name != "alternate" && !self.prefs.schemes.contains_key(&s.name) => { self.error.clear(); self.prefs.finger_scheme = s.name.clone(); self.prefs.schemes.insert(s.name.clone(), s); self.persist(users); changed = true; }, Ok(_) => self.error = "Схема с таким именем уже существует / Scheme name already exists".into(), Err(e) => self.error = e }
                        }
                    if let Some(scheme) = self.prefs.schemes.get(&self.prefs.finger_scheme).cloned() {
                        if ui.button("Экспорт схемы / Export scheme").clicked() && let Some(path) = rfd::FileDialog::new().add_filter("JSON", &["json"]).set_file_name("fingers.json").save_file() { match scheme.export(&path) { Ok(()) => self.error.clear(), Err(e) => self.error = e } }
                        if ui.button("Удалить схему / Delete scheme").clicked() { self.error.clear(); self.prefs.schemes.remove(&scheme.name); self.prefs.finger_scheme = "default".into(); self.persist(users); changed = true; }
                    }
                });
                if let Some(draft) = &mut self.scheme_draft {
                    ui.horizontal(|ui| { ui.label("Имя / Name:"); ui.text_edit_singleline(&mut self.scheme_name); });
                    egui::ScrollArea::vertical().id_salt("finger_editor").max_height(200.0).show(ui, |ui| {
                        for key in &self.schema.keys {
                            ui.horizontal(|ui| {
                                ui.label(&key.physical_id);
                                let mut value = draft.get(&key.physical_id).copied();
                                egui::ComboBox::from_id_salt(("finger", &key.physical_id)).selected_text(finger_label(value)).show_ui(ui, |ui| {
                                    ui.selectable_value(&mut value, None, "—");
                                    for finger in fingers() { ui.selectable_value(&mut value, Some(finger), finger_label(Some(finger))); }
                                });
                                if let Some(f) = value { draft.insert(key.physical_id.clone(), f); } else { draft.remove(&key.physical_id); }
                            });
                        }
                    });
                    if ui.button("Сохранить схему / Save scheme").clicked() {
                        let s = FingerScheme { version: 1, name: self.scheme_name.trim().into(), assignments: draft.clone() };
                        if s.name == "default" || s.name == "alternate" || self.prefs.schemes.contains_key(&s.name) { self.error = "Имя уже занято / Name already exists".into(); }
                        else if let Err(e) = s.validate() { self.error = e; }
                        else { self.error.clear(); self.prefs.finger_scheme = s.name.clone(); self.prefs.schemes.insert(s.name.clone(), s); self.scheme_draft = None; self.persist(users); changed = true; }
                    }
                }
                if !self.error.is_empty() { ui.colored_label(egui::Color32::RED, &self.error); }
            });
            self.open = open;
        }
        let current_assignments = self.assignments();
        if let Some(editor) = &mut self.editor {
            if let Some(schema) = editor.show(ctx) {
                let finger_changes = self.editor_base_schema.as_ref().and_then(|base| {
                    changed_finger_assignments(base, &schema, current_assignments.clone())
                });
                if let Some(c) = &self.collection {
                    match c.save_new(&schema) {
                        Ok(entry) => {
                            self.error.clear();
                            self.refresh();
                            self.prefs.keyboard = entry.id;
                            self.load_selected();
                            if let Some(assignments) = finger_changes {
                                let base_name = "Edited keyboard fingers".to_owned();
                                let mut name = base_name.clone();
                                let mut suffix = 2;
                                while self.prefs.schemes.contains_key(&name)
                                    || matches!(name.as_str(), "default" | "alternate")
                                {
                                    name = format!("{base_name} {suffix}");
                                    suffix += 1;
                                }
                                let scheme = FingerScheme {
                                    version: 1,
                                    name: name.clone(),
                                    assignments,
                                };
                                self.prefs.schemes.insert(name.clone(), scheme);
                                self.prefs.finger_scheme = name;
                            }
                            self.persist(users);
                            self.editor = None;
                            self.editor_base_schema = None;
                            changed = true;
                        }
                        Err(e) => self.error = e.to_string(),
                    }
                }
            } else if !editor.is_open() {
                self.editor = None;
                self.editor_base_schema = None;
            }
        }
        changed
    }
}
fn is_original_ansi(schema: &KeyboardSchema) -> bool {
    use staminator_keyboard_schema::Geometry;
    let original = keyboard::legacy_default_schema();
    if schema.version != original.version
        || schema.name != original.name
        || schema.show_finger_colors != original.show_finger_colors
        || schema.style != original.style
        || schema.canvas != original.canvas
        || schema.keys.len() != original.keys.len()
    {
        return false;
    }
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-12;
    schema
        .keys
        .iter()
        .zip(&original.keys)
        .all(|(key, expected)| {
            if key.physical_id != expected.physical_id
                || key.label != expected.label
                || key.shifted_label != expected.shifted_label
                || key.color != expected.color
                || key.finger != expected.finger
            {
                return false;
            }
            match (&key.geometry, &expected.geometry) {
                (
                    Geometry::Rect {
                        x,
                        y,
                        width,
                        height,
                    },
                    Geometry::Rect {
                        x: ex,
                        y: ey,
                        width: ew,
                        height: eh,
                    },
                ) => close(*x, *ex) && close(*y, *ey) && close(*width, *ew) && close(*height, *eh),
                _ => false,
            }
        })
}

fn changed_finger_assignments(
    base: &KeyboardSchema,
    edited: &KeyboardSchema,
    mut assignments: Assignments,
) -> Option<Assignments> {
    let before = base
        .keys
        .iter()
        .map(|k| (k.physical_id.as_str(), k.finger))
        .collect::<std::collections::BTreeMap<_, _>>();
    let after = edited
        .keys
        .iter()
        .map(|k| (k.physical_id.as_str(), k.finger))
        .collect::<std::collections::BTreeMap<_, _>>();
    let ids = before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let mut changed = false;
    for id in ids {
        let old = before.get(id).copied().flatten();
        let new = after.get(id).copied().flatten();
        if old != new {
            changed = true;
            if let Some(f) = new {
                assignments.insert(id.to_owned(), f);
            } else {
                assignments.remove(id);
            }
        }
    }
    changed.then_some(assignments)
}
fn fingers() -> [Finger; 10] {
    [
        Finger::LeftLittle,
        Finger::LeftRing,
        Finger::LeftMiddle,
        Finger::LeftIndex,
        Finger::RightIndex,
        Finger::RightMiddle,
        Finger::RightRing,
        Finger::RightLittle,
        Finger::LeftThumb,
        Finger::RightThumb,
    ]
}
fn finger_label(f: Option<Finger>) -> &'static str {
    match f {
        None => "—",
        Some(Finger::LeftLittle) => "Л миз. / L little",
        Some(Finger::LeftRing) => "Л без. / L ring",
        Some(Finger::LeftMiddle) => "Л сред. / L middle",
        Some(Finger::LeftIndex) => "Л указ. / L index",
        Some(Finger::RightIndex) => "П указ. / R index",
        Some(Finger::RightMiddle) => "П сред. / R middle",
        Some(Finger::RightRing) => "П без. / R ring",
        Some(Finger::RightLittle) => "П миз. / R little",
        Some(Finger::LeftThumb) => "Л больш. / L thumb",
        Some(Finger::RightThumb) => "П больш. / R thumb",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{RawInput, Rect, Vec2};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn click_accessible(ctx: &Context, label: &str, mut frame: impl FnMut(&Context)) {
        ctx.enable_accesskit();
        let screen = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 900.0));
        let mut target = None;
        for _ in 0..3 {
            let mut output = ctx.run_pass(
                RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ctx| ctx.root_ui(|_| frame(ctx)),
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
        let target = target.unwrap_or_else(|| panic!("AccessKit control not found: {label}"));
        let request = egui::Event::AccessKitActionRequest(accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: target,
            data: None,
        });
        let mut output = ctx.run_pass(
            RawInput {
                screen_rect: Some(screen),
                events: vec![request],
                ..Default::default()
            },
            |ctx| ctx.root_ui(|_| frame(ctx)),
        );
        output.textures_delta.clear();
    }

    fn keyboard_schema(root: &std::path::Path, name: &str) -> KeyboardSchema {
        let manager = KeyboardManager::new(root.join("Keyboards"));
        let mut schema = manager.schema;
        schema.name = name.into();
        schema
    }
    #[test]
    fn saved_original_ansi_is_aligned_without_overwriting_custom_keyboards() {
        let root = std::env::temp_dir().join(format!(
            "staminator-keyboard-alignment-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let collection = KeyboardCollection::new(root.join("Keyboards")).unwrap();
        let original = keyboard::legacy_default_schema();
        let entry = collection.save_new(&original).unwrap();
        let mut styled = original.clone();
        styled.show_finger_colors = false;
        assert!(!is_original_ansi(&styled));
        styled.show_finger_colors = true;
        styled.style = Some(staminator_keyboard_schema::KeyboardStyle::default());
        assert!(!is_original_ansi(&styled));
        let mut custom = original.clone();
        custom.keys[0].label = Some("Custom".into());
        let custom_entry = collection.save_new(&custom).unwrap();
        let mut users = UserStore::new_at(root.join("profiles.json"));
        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        manager.sync(&mut users);
        manager.prefs.keyboard = entry.id.clone();
        manager.load_selected();
        assert_eq!(manager.schema, keyboard::default_schema());
        assert!(is_original_ansi(&collection.load(&entry.id).unwrap()));
        manager.prefs.keyboard = custom_entry.id;
        manager.load_selected();
        assert_eq!(manager.schema, custom);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundled_split_keyboard_can_be_selected_in_settings() {
        let root = std::env::temp_dir().join(format!(
            "staminator-split-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut users = UserStore::new_at(root.join("profiles.json"));
        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        manager.sync(&mut users);
        manager.open();
        let ctx = Context::default();
        click_accessible(&ctx, "Split Black ANSI", |ctx| {
            manager.show(ctx, &mut users);
        });
        assert_eq!(manager.schema.name, "Split Black ANSI");
        assert!(!manager.schema.show_finger_colors);
        drop(manager);
        let mut reopened = UserStore::load_from(root.join("profiles.json")).unwrap();
        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        manager.sync(&mut reopened);
        assert_eq!(manager.schema.name, "Split Black ANSI");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundled_pink_is_available_to_existing_profiles_and_stays_deleted() {
        let root = std::env::temp_dir().join(format!(
            "staminator-bundled-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut users = UserStore::new_at(root.join("profiles.json"));
        let collection = KeyboardCollection::new(root.join("Keyboards")).unwrap();
        let original = collection.save_new(&keyboard::default_schema()).unwrap();
        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        manager.sync(&mut users);
        assert!(manager.entries.iter().any(|entry| entry.id == original.id));
        let pink = manager
            .entries
            .iter()
            .find(|entry| entry.name == "Soft Pink ANSI")
            .unwrap()
            .clone();
        manager.open();
        let ctx = Context::default();
        click_accessible(&ctx, "Soft Pink ANSI", |ctx| {
            manager.show(ctx, &mut users);
        });
        assert_eq!(manager.schema.name, "Soft Pink ANSI");
        assert!(!manager.schema.show_finger_colors);
        manager
            .collection
            .as_ref()
            .unwrap()
            .remove(&pink.id)
            .unwrap();
        drop(manager);
        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        manager.sync(&mut users);
        assert!(
            !manager
                .entries
                .iter()
                .any(|entry| entry.name == "Soft Pink ANSI")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selection_and_finger_scheme_follow_profiles_and_restart() {
        let root = std::env::temp_dir().join(format!(
            "staminator-keyboards-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let profile_path = root.join("profiles.json");
        let mut users = UserStore::new_at(&profile_path);
        let first = users.active_profile_id().to_owned();
        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        assert!(manager.sync(&mut users));
        let mut copy = manager.schema.clone();
        copy.name = "Other".into();
        let entry = manager
            .collection
            .as_ref()
            .unwrap()
            .save_new(&copy)
            .unwrap();
        manager.refresh();
        manager.prefs.keyboard = entry.id;
        manager.prefs.finger_scheme = "alternate".into();
        manager.persist(&mut users);
        let second = users.create("Second").unwrap().id.clone();
        manager.sync(&mut users);
        assert_eq!(manager.prefs.finger_scheme, "");
        users.select(&first).unwrap();
        manager.sync(&mut users);
        assert_eq!(manager.schema.name, "Other");
        assert_eq!(
            manager.assignments().get("Digit4"),
            Some(&Finger::LeftMiddle)
        );
        drop(manager);
        let mut users = UserStore::load_from(&profile_path).unwrap();
        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        manager.sync(&mut users);
        assert_eq!(manager.schema.name, "Other");
        assert_eq!(manager.prefs.finger_scheme, "alternate");
        users.select(&second).unwrap();
        manager.sync(&mut users);
        assert_eq!(manager.prefs.finger_scheme, "");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accessible_controls_select_keyboard_save_scheme_and_persist_editor_copy() {
        let root = std::env::temp_dir().join(format!(
            "staminator-manager-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let profile_path = root.join("profiles.json");
        let mut users = UserStore::new_at(&profile_path);
        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        assert!(manager.sync(&mut users));
        let other = manager
            .collection
            .as_ref()
            .unwrap()
            .save_new(&keyboard_schema(&root, "Other board"))
            .unwrap();
        manager.refresh();
        manager.open = true;

        let ctx = Context::default();
        click_accessible(&ctx, &other.name, |ctx| {
            manager.show(ctx, &mut users);
        });
        assert_eq!(manager.prefs.keyboard, other.id);
        assert_eq!(manager.schema.name, "Other board");

        manager.error = "Previous failure".into();
        click_accessible(&ctx, "Альтернативная / Alternative", |ctx| {
            manager.show(ctx, &mut users);
        });
        assert!(manager.error.is_empty());
        assert_eq!(manager.prefs.finger_scheme, "alternate");
        assert_eq!(
            manager.assignments().get("Digit4"),
            Some(&Finger::LeftMiddle)
        );

        manager.scheme_name = "Custom scheme".into();
        let mut draft = manager.assignments();
        draft.insert("KeyQ".into(), Finger::RightLittle);
        manager.scheme_draft = Some(draft);
        click_accessible(&ctx, "Сохранить схему / Save scheme", |ctx| {
            manager.show(ctx, &mut users);
        });
        assert_eq!(manager.prefs.finger_scheme, "Custom scheme");
        assert_eq!(
            manager.assignments().get("KeyQ"),
            Some(&Finger::RightLittle)
        );

        let mut edited = manager.schema.clone();
        edited.name = "Saved editor copy".into();
        manager.editor = Some(KeyboardEditor::new(edited));
        click_accessible(
            &ctx,
            "Validate & save copy / Сохранить копию",
            |ctx| {
                manager.show(ctx, &mut users);
            },
        );
        assert_eq!(manager.schema.name, "Saved editor copy");
        assert!(manager.editor.is_none());
        drop(manager);
        let mut users = UserStore::load_from(&profile_path).unwrap();
        let mut reopened = KeyboardManager::new(root.join("Keyboards"));
        reopened.sync(&mut users);
        assert_eq!(reopened.schema.name, "Saved editor copy");
        assert_eq!(reopened.prefs.finger_scheme, "Custom scheme");
        assert_eq!(
            reopened.assignments().get("KeyQ"),
            Some(&Finger::RightLittle)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_keyboard_zip_is_shared_with_another_profile() {
        use staminator_keyboard_schema::Geometry;
        let root = std::env::temp_dir().join(format!(
            "staminator-share-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut users = UserStore::new_at(root.join("profiles.json"));
        let mut source = KeyboardManager::new(root.join("source"));
        source.sync(&mut users);
        let mut board = source.schema.clone();
        board.name = "Shared board".into();
        let a = board
            .keys
            .iter_mut()
            .find(|k| k.physical_id == "KeyA")
            .unwrap();
        a.geometry = Geometry::Rect {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: 0.5,
        };
        a.label = Some(String::new());
        a.shifted_label = Some(String::new());
        let entry = source
            .collection
            .as_ref()
            .unwrap()
            .save_new(&board)
            .unwrap();
        let archive = root.join("board.zip");
        source
            .collection
            .as_ref()
            .unwrap()
            .export_zip(&entry.id, &archive)
            .unwrap();
        let second = users.create("Other profile").unwrap().id.clone();
        let mut destination = KeyboardManager::new(root.join("destination"));
        destination.sync(&mut users);
        let imported = destination
            .collection
            .as_ref()
            .unwrap()
            .import_zip(&archive)
            .unwrap();
        destination.refresh();
        destination.prefs.keyboard = imported.id;
        destination.load_selected();
        destination.persist(&mut users);
        assert_eq!(destination.schema, board);
        assert_eq!(
            keyboard::active_fingers(
                &destination.schema,
                keyboard::KeyboardLayout::Russian,
                &destination.assignments(),
                Some('ф')
            ),
            vec![0]
        );
        assert_eq!(
            keyboard::active_fingers(
                &destination.schema,
                keyboard::KeyboardLayout::English,
                &destination.assignments(),
                Some('A')
            ),
            vec![0, 7]
        );
        drop(destination);
        let mut reopened = UserStore::load_from(root.join("profiles.json")).unwrap();
        assert_eq!(reopened.active_profile_id(), second);
        let mut destination = KeyboardManager::new(root.join("destination"));
        destination.sync(&mut reopened);
        assert_eq!(destination.schema, board);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saving_editor_finger_change_creates_scheme_that_survives_restart() {
        let root = std::env::temp_dir().join(format!(
            "staminator-editor-finger-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let profile_path = root.join("profiles.json");
        let mut users = UserStore::new_at(&profile_path);
        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        manager.sync(&mut users);
        let mut edited = manager.schema.clone();
        edited.name = "Board with custom finger".into();
        edited
            .keys
            .iter_mut()
            .find(|k| k.physical_id == "Digit4")
            .unwrap()
            .finger = Some(Finger::LeftMiddle);
        manager.editor_base_schema = Some(manager.schema.clone());
        manager.editor = Some(KeyboardEditor::new(edited));
        let ctx = Context::default();
        click_accessible(
            &ctx,
            "Validate & save copy / Сохранить копию",
            |ctx| {
                manager.show(ctx, &mut users);
            },
        );
        assert_eq!(manager.schema.name, "Board with custom finger");
        assert_eq!(
            manager.assignments().get("Digit4"),
            Some(&Finger::LeftMiddle)
        );
        assert!(
            manager
                .prefs
                .schemes
                .contains_key(&manager.prefs.finger_scheme)
        );
        drop(manager);
        let mut reopened = UserStore::load_from(&profile_path).unwrap();
        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        manager.sync(&mut reopened);
        assert_eq!(
            manager.assignments().get("Digit4"),
            Some(&Finger::LeftMiddle)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inaccessible_collection_reports_error_and_profile_switch_discards_drafts() {
        let root = std::env::temp_dir().join(format!(
            "staminator-manager-error-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let not_a_dir = root.join("not-a-directory");
        std::fs::write(&not_a_dir, "file").unwrap();
        let mut users = UserStore::new_at(root.join("profiles.json"));
        let mut broken = KeyboardManager::new(not_a_dir.join("Keyboards"));
        assert!(!broken.sync(&mut users));
        assert!(!broken.error.is_empty());

        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        manager.sync(&mut users);
        manager.scheme_draft = Some(manager.assignments());
        manager.scheme_name = "Unsaved".into();
        manager.editor = Some(KeyboardEditor::new(manager.schema.clone()));
        let second = users.create("Second").unwrap().id.clone();
        users.select(&second).unwrap();
        assert!(manager.sync(&mut users));
        assert!(manager.scheme_draft.is_none());
        assert!(manager.scheme_name.is_empty());
        assert!(manager.editor.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_finger_changes_create_a_persistable_scheme_without_changing_geometry_selection() {
        let root = std::env::temp_dir().join(format!(
            "staminator-manager-fingers-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let original = keyboard_schema(&root, "Before");
        let mut changed = original.clone();
        let key = changed
            .keys
            .iter_mut()
            .find(|k| k.physical_id == "KeyQ")
            .unwrap();
        key.finger = Some(Finger::RightLittle);
        let mut assignments = KeyboardManager::new(root.join("Keyboards")).assignments();
        let custom = changed_finger_assignments(&original, &changed, assignments.clone()).unwrap();
        assignments.insert("KeyQ".into(), Finger::RightLittle);
        assert_eq!(custom, assignments);
        assert_eq!(
            keyboard::active_fingers(
                &changed,
                keyboard::KeyboardLayout::English,
                &custom,
                Some('Q')
            ),
            [7, 0]
        );
        assert!(changed_finger_assignments(&changed, &changed, custom.clone()).is_none());

        let scheme = FingerScheme {
            version: 1,
            name: "Edited board fingers".into(),
            assignments: custom,
        };
        scheme.validate().unwrap();
        let path = root.join("fingers.json");
        scheme.export(&path).unwrap();
        assert_eq!(FingerScheme::load(&path).unwrap(), scheme);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finger_delta_handles_added_and_removed_assignments() {
        let base = keyboard::default_schema();
        let mut edited = base.clone();
        let q = edited
            .keys
            .iter_mut()
            .find(|k| k.physical_id == "KeyQ")
            .unwrap();
        q.finger = None;
        let mut added = base
            .keys
            .iter()
            .find(|k| k.physical_id == "Digit1")
            .unwrap()
            .clone();
        added.physical_id = "Numpad1".into();
        added.finger = Some(Finger::RightThumb);
        edited.keys.push(added);
        let mut assignments = keyboard::default_assignments();
        let result = changed_finger_assignments(&base, &edited, assignments.clone()).unwrap();
        assignments.remove("KeyQ");
        assignments.insert("Numpad1".into(), Finger::RightThumb);
        assert_eq!(result, assignments);
        assert_eq!(result.get("KeyQ"), None);
        assert_eq!(result.get("Numpad1"), Some(&Finger::RightThumb));
    }

    #[test]
    fn selection_fallback_and_invalid_custom_scheme_save_are_reported() {
        let root = std::env::temp_dir().join(format!(
            "staminator-manager-fallback-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut users = UserStore::new_at(root.join("profiles.json"));
        let mut manager = KeyboardManager::new(root.join("Keyboards"));
        manager.sync(&mut users);
        let first = manager.entries[0].id.clone();
        manager.prefs.keyboard = "missing-entry".into();
        manager.load_selected();
        assert_eq!(manager.prefs.keyboard, first);
        assert_eq!(manager.schema.name, "ANSI QWERTY");

        manager.open = true;
        manager.scheme_name = "default".into();
        manager.scheme_draft = Some(manager.assignments());
        let ctx = Context::default();
        click_accessible(&ctx, "Сохранить схему / Save scheme", |ctx| {
            manager.show(ctx, &mut users);
        });
        assert!(manager.error.contains("already"));
        assert!(manager.scheme_draft.is_some());
        assert!(!manager.prefs.schemes.contains_key("default"));

        let count = manager.entries.len();
        manager.editor_base_schema = Some(manager.schema.clone());
        manager.editor = Some(KeyboardEditor::new(manager.schema.clone()));
        click_accessible(&ctx, "Close / Закрыть", |ctx| {
            manager.show(ctx, &mut users);
        });
        assert!(manager.editor.is_none());
        assert_eq!(manager.entries.len(), count);
        std::fs::remove_dir_all(root).unwrap();
    }
}
