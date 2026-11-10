//! Local profile and lesson result persistence.

use crate::localization;
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_RESULTS_PER_PROFILE: usize = 10_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResultRecord {
    pub timestamp: u64,
    pub mode: String,
    pub layout: String,
    pub exercise: String,
    pub correct: u64,
    pub errors: u64,
    pub elapsed_ms: u64,
    pub cpm: f64,
    pub error_percent: f64,
    #[serde(default = "unknown_reason")]
    pub stop_reason: String,
}

fn unknown_reason() -> String {
    "unknown".into()
}

pub struct ResultInput<'a> {
    pub mode: &'a str,
    pub layout: &'a str,
    pub exercise: &'a str,
    pub correct: u64,
    pub errors: u64,
    pub elapsed_ms: u64,
    pub cpm: f64,
    pub error_percent: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub results: Vec<ResultRecord>,
    #[serde(default)]
    pub external_text: Option<ExternalProgress>,
    #[serde(default = "default_content_language")]
    pub content_language: String,
    #[serde(default = "default_content_language")]
    pub interface_language: String,
    /// Last selected lesson, keyed by content language code.
    #[serde(default)]
    pub selected_lessons: BTreeMap<String, String>,
    #[serde(default)]
    pub selected_mode: Option<String>,
    #[serde(default)]
    pub selected_layout: Option<String>,
    #[serde(default, deserialize_with = "deserialize_background")]
    pub background: BackgroundImageSettings,
    /// JSON fallback for window geometry; SQLite stores these in profile settings.
    #[serde(default, deserialize_with = "deserialize_window_geometry")]
    pub window_geometry: BTreeMap<String, crate::window_geometry::SavedWindowGeometry>,
    /// Per-window scale factors, stored in JSON or profile settings in SQLite.
    #[serde(default, deserialize_with = "deserialize_window_scales")]
    pub window_scales: BTreeMap<String, f32>,
}

fn deserialize_window_scales<'de, D>(deserializer: D) -> Result<BTreeMap<String, f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let mut result = BTreeMap::new();
    if let Some(entries) = value.as_object() {
        for (id, value) in entries {
            if id.trim().is_empty() {
                continue;
            }
            if let Some(scale) = value.as_f64().map(|scale| scale as f32)
                && scale.is_finite()
                && (0.5..=3.0).contains(&scale)
            {
                result.insert(id.clone(), scale);
            }
        }
    }
    Ok(result)
}

fn deserialize_window_geometry<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, crate::window_geometry::SavedWindowGeometry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let mut result = BTreeMap::new();
    if let Some(entries) = value.as_object() {
        for (id, value) in entries {
            if id.trim().is_empty() {
                continue;
            }
            if let Ok(geometry) =
                serde_json::from_value::<crate::window_geometry::SavedWindowGeometry>(value.clone())
                && crate::window_geometry::is_valid_saved_geometry(&geometry)
            {
                result.insert(id.clone(), geometry);
            }
        }
    }
    Ok(result)
}

fn deserialize_background<'de, D>(deserializer: D) -> Result<BackgroundImageSettings, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut value = serde_json::Value::deserialize(deserializer)?;
    if let Some(object) = value.as_object_mut()
        && !object.contains_key("use_builtin")
    {
        let use_builtin = object
            .get("image_path")
            .is_none_or(serde_json::Value::is_null);
        object.insert("use_builtin".into(), use_builtin.into());
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

/// Per-profile selection for the optional training background image.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BackgroundImageSettings {
    pub image_path: Option<PathBuf>,
    #[serde(default)]
    pub use_builtin: bool,
    pub placement: BackgroundPlacement,
    /// Black overlay opacity, from 0 (none) to 100 (opaque).
    pub dim_percent: u8,
}

impl Default for BackgroundImageSettings {
    fn default() -> Self {
        Self {
            image_path: None,
            use_builtin: true,
            placement: BackgroundPlacement::Fill,
            dim_percent: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundPlacement {
    #[default]
    Fit,
    Fill,
    Tile,
}

fn default_content_language() -> String {
    "RU".into()
}

fn is_supported_language(language: &str) -> bool {
    crate::content::ContentLanguage::parse(language).is_some()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalProgress {
    pub path: PathBuf,
    pub size: u64,
    pub hash: u64,
    pub encoding: String,
    pub position: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserStore {
    #[serde(default)]
    profiles: Vec<Profile>,
    #[serde(default)]
    active_profile_id: String,
    #[serde(skip)]
    path: PathBuf,
    #[serde(skip)]
    database: bool,
    #[serde(skip)]
    revision: Cell<u64>,
    /// Version of the one-time built-in background default migration.
    #[serde(default)]
    background_default_version: u8,
}

impl UserStore {
    /// Read the scale for a stable window ID in the active profile.
    pub fn window_scale(&self, id: &str) -> io::Result<f32> {
        if id.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "window ID must not be empty",
            ));
        }
        let profile = self.active_profile().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                localization::tr("profile.not_found"),
            )
        })?;
        if self.database {
            let key = format!("window_scale:{id}");
            if let Some(value) = self
                .database()?
                .expect("database flag")
                .get_setting(&profile.id, &key)
                .map_err(storage_io_error)?
                && let Ok(scale) = serde_json::from_str::<f32>(&value)
                && scale.is_finite()
                && (0.5..=3.0).contains(&scale)
            {
                return Ok(scale);
            }
        }
        Ok(profile.window_scales.get(id).copied().unwrap_or(1.0))
    }

    /// Save the scale for a stable window ID in the active profile.
    pub fn set_window_scale(&mut self, id: &str, value: f32) -> io::Result<()> {
        if id.trim().is_empty() || !value.is_finite() || !(0.5..=3.0).contains(&value) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid window scale",
            ));
        }
        if self.active_profile().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                localization::tr("profile.not_found"),
            ));
        }
        let profile_id = self.active_profile_id.clone();
        if self.database {
            self.database()?
                .expect("database flag")
                .set_setting(
                    &profile_id,
                    &format!("window_scale:{id}"),
                    &serde_json::to_string(&value).map_err(invalid_data)?,
                )
                .map_err(storage_io_error)?;
            return Ok(());
        }
        let original = self.clone();
        let profile = self
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    localization::tr("profile.not_found"),
                )
            })?;
        profile.window_scales.insert(id.to_owned(), value);
        self.commit_or_restore(original)
    }

    /// Read geometry for a stable window ID in the active profile.
    pub fn window_geometry(
        &self,
        id: &str,
    ) -> io::Result<Option<crate::window_geometry::SavedWindowGeometry>> {
        if id.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "window ID must not be empty",
            ));
        }
        let profile = self.active_profile().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                localization::tr("profile.not_found"),
            )
        })?;
        if self.database {
            let key = format!("window_geometry:{id}");
            if let Some(value) = self
                .database()?
                .expect("database flag")
                .get_setting(&profile.id, &key)
                .map_err(storage_io_error)?
            {
                // A null marker records an explicit reset and prevents stale JSON fallback.
                return Ok(serde_json::from_str::<
                    Option<crate::window_geometry::SavedWindowGeometry>,
                >(&value)
                .ok()
                .flatten()
                .filter(crate::window_geometry::is_valid_saved_geometry));
            }
        }
        Ok(profile
            .window_geometry
            .get(id)
            .copied()
            .filter(crate::window_geometry::is_valid_saved_geometry))
    }

    /// Save or reset geometry for a stable window ID in the active profile.
    pub fn set_window_geometry(
        &mut self,
        id: &str,
        value: Option<crate::window_geometry::SavedWindowGeometry>,
    ) -> io::Result<()> {
        if id.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "window ID must not be empty",
            ));
        }
        if value.is_some_and(|geometry| !crate::window_geometry::is_valid_saved_geometry(&geometry))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid window geometry",
            ));
        }
        if self.active_profile().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                localization::tr("profile.not_found"),
            ));
        }
        let profile_id = self.active_profile_id.clone();
        if self.database {
            let key = format!("window_geometry:{id}");
            let serialized = serde_json::to_string(&value).map_err(invalid_data)?;
            self.database()?
                .expect("database flag")
                .set_setting(&profile_id, &key, &serialized)
                .map_err(storage_io_error)?;
            return Ok(());
        }
        let original = self.clone();
        let profile = self
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    localization::tr("profile.not_found"),
                )
            })?;
        if let Some(geometry) = value {
            profile.window_geometry.insert(id.to_owned(), geometry);
        } else {
            profile.window_geometry.remove(id);
        }
        self.commit_or_restore(original)
    }

    pub fn background_settings(&self) -> Option<&BackgroundImageSettings> {
        self.active_profile().map(|profile| &profile.background)
    }

    pub fn set_background_settings(&mut self, settings: BackgroundImageSettings) -> io::Result<()> {
        if settings.dim_percent > 100 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "background dim must be between 0 and 100",
            ));
        }
        let original = self.clone();
        let profile = self
            .profiles
            .iter_mut()
            .find(|p| p.id == self.active_profile_id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    localization::tr("profile.not_found"),
                )
            })?;
        profile.background = settings;
        self.commit_or_restore(original)
    }
    pub fn database(&self) -> io::Result<Option<stamina_data::storage::Database>> {
        if self.database {
            stamina_data::storage::Database::open(&self.path)
                .map(Some)
                .map_err(storage_io_error)
        } else {
            Ok(None)
        }
    }

    /// Restore a checked backup and retain a copy of the previous database.
    pub fn restore_backup(&mut self, source: &Path) -> io::Result<PathBuf> {
        let mut database = self.database()?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                localization::tr("profile.database_unavailable"),
            )
        })?;
        let safety_path = self
            .path
            .with_file_name(format!("pre-restore-{}.sqlite3", fresh_id()));
        database
            .restore_from(source, self.revision.get(), &safety_path)
            .map_err(storage_io_error)?;
        let loaded = database
            .load_snapshot_with_revision()
            .map_err(storage_io_error)?;
        let mut restored: Self =
            serde_json::from_value(serde_json::to_value(loaded.snapshot).map_err(invalid_data)?)
                .map_err(invalid_data)?;
        restored.path = self.path.clone();
        restored.database = true;
        restored.background_default_version = 1;
        restored.revision.set(loaded.revision);
        for profile in &mut restored.profiles {
            if let Some(value) = database
                .get_setting(&profile.id, "selected_lessons")
                .map_err(storage_io_error)?
            {
                profile.selected_lessons = serde_json::from_str(&value).map_err(invalid_data)?;
            }
            profile.selected_mode = database
                .get_setting(&profile.id, "selected_mode")
                .map_err(storage_io_error)?;
            profile.selected_layout = database
                .get_setting(&profile.id, "selected_layout")
                .map_err(storage_io_error)?;
            if let Some(value) = database
                .get_setting(&profile.id, "training_selection")
                .map_err(storage_io_error)?
                && let Ok((mode, layout)) = serde_json::from_str::<(String, String)>(&value)
            {
                profile.selected_mode = Some(mode);
                profile.selected_layout = Some(layout);
            }
        }
        *self = restored;
        Ok(safety_path)
    }

    /// Load SQLite and import the legacy JSON store once when present.
    pub fn load() -> io::Result<Self> {
        let legacy = default_path();
        let path = legacy.with_file_name("staminator.sqlite3");
        Self::load_database(path, &legacy)
    }

    pub(crate) fn load_database(path: PathBuf, legacy: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut legacy_selections = BTreeMap::new();
        let mut database =
            stamina_data::storage::Database::open(&path).map_err(storage_io_error)?;
        let backup = sibling_path(legacy, "bak");
        let source = if legacy.exists() {
            Some(legacy)
        } else if backup.exists() {
            Some(backup.as_path())
        } else {
            None
        };
        if let Some(source) = source {
            if let Ok(bytes) = fs::read(source)
                && let Ok(legacy_store) = serde_json::from_slice::<Self>(&bytes)
            {
                legacy_selections = legacy_store
                    .profiles
                    .into_iter()
                    .map(|profile| {
                        (
                            profile.id,
                            (
                                profile.selected_lessons,
                                profile.selected_mode,
                                profile.selected_layout,
                            ),
                        )
                    })
                    .collect();
            }
            database
                .import_profiles_json(source)
                .map_err(storage_io_error)?;
        }
        let loaded = database
            .load_snapshot_with_revision()
            .map_err(storage_io_error)?;
        let before = serde_json::to_value(&loaded.snapshot).map_err(invalid_data)?;
        let mut store: Self =
            serde_json::from_value(serde_json::to_value(loaded.snapshot).map_err(invalid_data)?)
                .map_err(invalid_data)?;
        store.path = path;
        store.database = true;
        store.background_default_version = 1;
        store.revision.set(loaded.revision);
        for profile in &mut store.profiles {
            let key = "selected_lessons";
            let stored = database
                .get_setting(&profile.id, key)
                .map_err(storage_io_error)?;
            if let Some(value) = stored {
                if let Ok(selected) = serde_json::from_str::<BTreeMap<String, String>>(&value) {
                    profile.selected_lessons = selected;
                }
            } else if let Some((selected, _, _)) = legacy_selections.get(&profile.id) {
                profile.selected_lessons = selected.clone();
                database
                    .set_setting(
                        &profile.id,
                        key,
                        &serde_json::to_string(selected).map_err(invalid_data)?,
                    )
                    .map_err(storage_io_error)?;
            }
            if let Some(value) = database
                .get_setting(&profile.id, "training_selection")
                .map_err(storage_io_error)?
            {
                if let Ok((mode, layout)) = serde_json::from_str::<(String, String)>(&value) {
                    profile.selected_mode = Some(mode);
                    profile.selected_layout = Some(layout);
                }
            } else if let Some((_, Some(mode), Some(layout))) = legacy_selections.get(&profile.id) {
                profile.selected_mode = Some(mode.clone());
                profile.selected_layout = Some(layout.clone());
                database
                    .set_setting(
                        &profile.id,
                        "training_selection",
                        &serde_json::to_string(&(mode, layout)).map_err(invalid_data)?,
                    )
                    .map_err(storage_io_error)?;
            }
        }
        store.normalize();
        let after: stamina_data::storage::Snapshot =
            serde_json::from_value(serde_json::to_value(&store).map_err(invalid_data)?)
                .map_err(invalid_data)?;
        if serde_json::to_value(after).map_err(invalid_data)? != before {
            store.save()?;
        }
        Ok(store)
    }

    /// Load from an explicit path; useful for isolated tests and portable setups.
    pub fn load_from(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let backup = sibling_path(&path, "bak");
        let source = if path.exists() {
            Some(path.as_path())
        } else if backup.exists() {
            Some(backup.as_path())
        } else {
            None
        };
        let had_source = source.is_some();
        let mut store = if let Some(source) = source {
            let bytes = fs::read(source)?;
            serde_json::from_slice::<Self>(&bytes).map_err(invalid_data)?
        } else {
            Self {
                profiles: Vec::new(),
                active_profile_id: String::new(),
                path: path.clone(),
                database: false,
                revision: Cell::new(0),
                background_default_version: 1,
            }
        };
        store.path = path;
        let migrate_background_default = store.background_default_version == 0;
        store.normalize();
        if migrate_background_default && had_source {
            // Persist the conversion once so a later explicit Fit selection stays Fit.
            store.save()?;
        }
        Ok(store)
    }

    /// Create an in-memory default store that will save to the standard path.
    pub fn new() -> Self {
        Self::new_at(default_path())
    }

    pub fn new_at(path: impl Into<PathBuf>) -> Self {
        let mut store = Self {
            profiles: Vec::new(),
            active_profile_id: String::new(),
            path: path.into(),
            database: false,
            revision: Cell::new(0),
            background_default_version: 1,
        };
        store.normalize();
        store
    }

    pub fn save(&self) -> io::Result<()> {
        if self.database {
            let snapshot =
                serde_json::from_value(serde_json::to_value(self).map_err(invalid_data)?)
                    .map_err(invalid_data)?;
            let revision = stamina_data::storage::Database::open(&self.path)
                .and_then(|mut database| {
                    database.save_snapshot_if_revision(&snapshot, self.revision.get())
                })
                .map_err(storage_io_error)?;
            self.revision.set(revision);
            return Ok(());
        }
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    localization::tr("profile.path_no_parent"),
                )
            })?;
        fs::create_dir_all(parent)?;
        let bytes = serde_json::to_vec_pretty(self).map_err(invalid_data)?;
        let temp = sibling_path(&self.path, "tmp");
        fs::write(&temp, bytes)?;

        // Windows rename does not replace an existing destination. Move the old
        // file aside first and restore it if installing the new file fails.
        if self.path.exists() {
            let backup = sibling_path(&self.path, "bak");
            if backup.exists() {
                fs::remove_file(&backup)?;
            }
            fs::rename(&self.path, &backup)?;
            if let Err(error) = fs::rename(&temp, &self.path) {
                let _ = fs::rename(&backup, &self.path);
                let _ = fs::remove_file(&temp);
                return Err(error);
            }
            let _ = fs::remove_file(backup);
        } else if let Err(error) = fs::rename(&temp, &self.path) {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        Ok(())
    }

    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    pub fn import_profile(&mut self, mut profile: Profile) -> io::Result<()> {
        if profile.id.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                localization::tr("profile.id_empty"),
            ));
        }
        if profile.results.len() > MAX_RESULTS_PER_PROFILE
            || !is_supported_language(&profile.content_language)
            || !is_supported_language(&profile.interface_language)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                localization::tr("profile.invalid_contents"),
            ));
        }
        profile.content_language =
            crate::content::ContentLanguage::parse(&profile.content_language)
                .unwrap()
                .code()
                .into();
        profile.interface_language =
            crate::content::ContentLanguage::parse(&profile.interface_language)
                .unwrap()
                .code()
                .into();
        valid_name(&profile.name)?;
        ensure_unique(&self.profiles, &profile.name, None)?;
        if self
            .profiles
            .iter()
            .any(|existing| existing.id == profile.id)
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                localization::tr("profile.id_duplicate"),
            ));
        }
        let original = self.clone();
        self.active_profile_id = profile.id.clone();
        self.profiles.push(profile);
        self.commit_or_restore(original)
    }

    pub fn active_profile_id(&self) -> &str {
        &self.active_profile_id
    }

    pub fn active_name(&self) -> &str {
        self.active_profile()
            .map(|p| p.name.as_str())
            .unwrap_or("Пользователь")
    }

    pub fn active_profile(&self) -> Option<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.id == self.active_profile_id)
    }

    pub fn set_content_language(&mut self, language: &str) -> io::Result<()> {
        if !is_supported_language(language) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                localization::tr("profile.invalid_content_language"),
            ));
        }
        let original = self.clone();
        let profile = self
            .profiles
            .iter_mut()
            .find(|p| p.id == self.active_profile_id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    localization::tr("profile.not_found"),
                )
            })?;
        profile.content_language = crate::content::ContentLanguage::parse(language)
            .unwrap()
            .code()
            .into();
        self.commit_or_restore(original)
    }

    /// Set the interface language for the active profile.
    pub fn set_interface_language(&mut self, language: &str) -> io::Result<()> {
        if !is_supported_language(language) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                localization::tr("profile.invalid_interface_language"),
            ));
        }
        let original = self.clone();
        let profile = self
            .profiles
            .iter_mut()
            .find(|p| p.id == self.active_profile_id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    localization::tr("profile.not_found"),
                )
            })?;
        profile.interface_language = crate::content::ContentLanguage::parse(language)
            .unwrap()
            .code()
            .into();
        self.commit_or_restore(original)
    }

    /// Save the selected lesson for the active profile's current content language.
    pub fn set_selected_lesson(&mut self, lesson: &str) -> io::Result<()> {
        let lesson = lesson.trim();
        if lesson.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "lesson must not be empty",
            ));
        }
        let original = self.clone();
        let (profile_id, serialized) = {
            let profile = self
                .profiles
                .iter_mut()
                .find(|p| p.id == self.active_profile_id)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        localization::tr("profile.not_found"),
                    )
                })?;
            profile
                .selected_lessons
                .insert(profile.content_language.clone(), lesson.to_owned());
            (
                profile.id.clone(),
                serde_json::to_string(&profile.selected_lessons).map_err(invalid_data)?,
            )
        };
        if self.database {
            let result = self.database().and_then(|db| {
                db.expect("database flag")
                    .set_setting(&profile_id, "selected_lessons", &serialized)
                    .map_err(storage_io_error)
            });
            if let Err(error) = result {
                *self = original;
                return Err(error);
            }
            return Ok(());
        }
        self.commit_or_restore(original)
    }

    /// Save the selected training mode and keyboard layout for the active profile.
    pub fn set_training_selection(&mut self, mode: &str, layout: &str) -> io::Result<()> {
        let mode = mode.trim();
        let layout = layout.trim();
        if mode.is_empty() || layout.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mode and layout must not be empty",
            ));
        }
        let original = self.clone();
        let profile_id = self
            .active_profile()
            .map(|profile| profile.id.clone())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    localization::tr("profile.not_found"),
                )
            })?;
        if self.database {
            let mut database = self.database()?.expect("database flag");
            database
                .set_setting(
                    &profile_id,
                    "training_selection",
                    &serde_json::to_string(&(mode, layout)).map_err(invalid_data)?,
                )
                .map_err(storage_io_error)?;
        }
        let profile = self
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .expect("active profile was found");
        profile.selected_mode = Some(mode.to_owned());
        profile.selected_layout = Some(layout.to_owned());
        if self.database {
            return Ok(());
        }
        self.commit_or_restore(original)
    }

    pub fn select(&mut self, id: &str) -> io::Result<()> {
        if !self.profiles.iter().any(|p| p.id == id) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                localization::tr("profile.not_found"),
            ));
        }
        let original = self.clone();
        self.active_profile_id = id.to_owned();
        self.commit_or_restore(original)
    }

    pub fn create(&mut self, name: &str) -> io::Result<&Profile> {
        let name = valid_name(name)?;
        ensure_unique(&self.profiles, &name, None)?;
        let original = self.clone();
        let id = fresh_id();
        self.profiles.push(Profile {
            id: id.clone(),
            name,
            results: Vec::new(),
            external_text: None,
            content_language: default_content_language(),
            interface_language: default_content_language(),
            selected_lessons: BTreeMap::new(),
            selected_mode: None,
            selected_layout: None,
            background: BackgroundImageSettings::default(),
            window_geometry: BTreeMap::new(),
            window_scales: BTreeMap::new(),
        });
        self.active_profile_id = id;
        self.commit_or_restore(original)?;
        Ok(self.profiles.last().expect("profile was just inserted"))
    }

    pub fn rename(&mut self, id: &str, name: &str) -> io::Result<()> {
        let name = valid_name(name)?;
        let index = self
            .profiles
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    localization::tr("profile.not_found"),
                )
            })?;
        ensure_unique(&self.profiles, &name, Some(id))?;
        let original = self.clone();
        self.profiles[index].name = name;
        self.commit_or_restore(original)
    }

    pub fn delete(&mut self, id: &str) -> io::Result<()> {
        if self.profiles.len() <= 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                localization::tr("profile.cannot_delete_last"),
            ));
        }
        let index = self
            .profiles
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    localization::tr("profile.not_found"),
                )
            })?;
        let original = self.clone();
        self.profiles.remove(index);
        if self.active_profile_id == id {
            self.active_profile_id = self.profiles[0].id.clone();
        }
        self.commit_or_restore(original)
    }

    fn commit_or_restore(&mut self, original: Self) -> io::Result<()> {
        if let Err(error) = self.save() {
            *self = original;
            return Err(error);
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn record(&mut self, input: ResultInput<'_>) -> io::Result<&ResultRecord> {
        self.record_with_reason(input, "unknown")
    }

    fn record_with_reason(
        &mut self,
        input: ResultInput<'_>,
        reason: &str,
    ) -> io::Result<&ResultRecord> {
        let original = self.clone();
        let profile = self
            .profiles
            .iter_mut()
            .find(|p| p.id == self.active_profile_id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    localization::tr("profile.active_not_found"),
                )
            })?;
        profile.results.push(ResultRecord {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            mode: input.mode.to_owned(),
            layout: input.layout.to_owned(),
            exercise: input.exercise.to_owned(),
            correct: input.correct,
            errors: input.errors,
            elapsed_ms: input.elapsed_ms,
            cpm: input.cpm,
            error_percent: input.error_percent,
            stop_reason: reason.to_owned(),
        });
        if profile.results.len() > MAX_RESULTS_PER_PROFILE {
            profile
                .results
                .drain(..profile.results.len() - MAX_RESULTS_PER_PROFILE);
        }
        if let Err(error) = self.save() {
            *self = original;
            return Err(error);
        }
        Ok(self
            .profiles
            .iter()
            .find(|p| p.id == self.active_profile_id)
            .unwrap()
            .results
            .last()
            .unwrap())
    }

    pub fn record_with_progress(
        &mut self,
        input: ResultInput<'_>,
        progress: Option<ExternalProgress>,
        reason: &str,
    ) -> io::Result<()> {
        let original = self.clone();
        if let Some(progress) = progress {
            let profile = self
                .profiles
                .iter_mut()
                .find(|p| p.id == self.active_profile_id)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        localization::tr("profile.active_not_found"),
                    )
                })?;
            profile.external_text = Some(progress);
        }
        if let Err(error) = self.record_with_reason(input, reason) {
            *self = original;
            return Err(error);
        }
        Ok(())
    }

    pub fn active_records(&self) -> &[ResultRecord] {
        self.active_profile()
            .map(|p| p.results.as_slice())
            .unwrap_or(&[])
    }

    pub fn set_external_progress(&mut self, progress: ExternalProgress) -> io::Result<()> {
        let original = self.clone();
        let profile = self
            .profiles
            .iter_mut()
            .find(|p| p.id == self.active_profile_id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    localization::tr("profile.active_not_found"),
                )
            })?;
        profile.external_text = Some(progress);
        if let Err(error) = self.save() {
            *self = original;
            return Err(error);
        }
        Ok(())
    }

    fn normalize(&mut self) {
        if self.background_default_version == 0 {
            for profile in &mut self.profiles {
                let background = &mut profile.background;
                if background.use_builtin
                    && background.image_path.is_none()
                    && background.placement == BackgroundPlacement::Fit
                    && background.dim_percent == 0
                {
                    background.placement = BackgroundPlacement::Fill;
                }
            }
            self.background_default_version = 1;
        }
        self.profiles
            .retain(|p| !p.id.trim().is_empty() && !p.name.trim().is_empty());
        // Repair duplicate IDs/names from malformed or older store files.
        let mut i = 0;
        while i < self.profiles.len() {
            if self.profiles[..i]
                .iter()
                .any(|p| p.id == self.profiles[i].id)
            {
                self.profiles[i].id = fresh_id();
            }
            if self.profiles[..i]
                .iter()
                .any(|p| p.name.eq_ignore_ascii_case(&self.profiles[i].name))
            {
                let base = self.profiles[i].name.clone();
                let mut suffix = 2;
                loop {
                    let candidate = format!("{base} ({suffix})");
                    if !self.profiles[..i]
                        .iter()
                        .any(|p| p.name.eq_ignore_ascii_case(&candidate))
                    {
                        self.profiles[i].name = candidate;
                        break;
                    }
                    suffix += 1;
                }
            }
            self.profiles[i].content_language =
                crate::content::ContentLanguage::parse(&self.profiles[i].content_language)
                    .map_or("RU", |language| language.code())
                    .into();
            self.profiles[i].interface_language =
                crate::content::ContentLanguage::parse(&self.profiles[i].interface_language)
                    .map_or("RU", |language| language.code())
                    .into();
            let results = &mut self.profiles[i].results;
            if results.len() > MAX_RESULTS_PER_PROFILE {
                let excess = results.len() - MAX_RESULTS_PER_PROFILE;
                results.drain(..excess);
            }
            i += 1;
        }
        if self.profiles.is_empty() {
            let id = fresh_id();
            self.profiles.push(Profile {
                id: id.clone(),
                name: "Пользователь".into(),
                results: Vec::new(),
                external_text: None,
                content_language: default_content_language(),
                interface_language: "RU".into(),
                selected_lessons: BTreeMap::new(),
                selected_mode: None,
                selected_layout: None,
                background: BackgroundImageSettings::default(),
                window_geometry: BTreeMap::new(),
                window_scales: BTreeMap::new(),
            });
            self.active_profile_id = id;
        } else if !self.profiles.iter().any(|p| p.id == self.active_profile_id) {
            self.active_profile_id = self.profiles[0].id.clone();
        }
    }
}

impl Default for UserStore {
    fn default() -> Self {
        Self::new()
    }
}

fn valid_name(value: &str) -> io::Result<String> {
    let name = value.trim();
    if name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            localization::tr("profile.name_empty"),
        ));
    }
    Ok(name.to_owned())
}

fn ensure_unique(profiles: &[Profile], name: &str, except_id: Option<&str>) -> io::Result<()> {
    if profiles
        .iter()
        .any(|p| Some(p.id.as_str()) != except_id && p.name.eq_ignore_ascii_case(name))
    {
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            localization::tr("profile.name_duplicate"),
        ))
    } else {
        Ok(())
    }
}

fn default_path() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.unwrap_or_else(std::env::temp_dir)
        .join("Staminator")
        .join("profiles.json")
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{suffix}"));
    path.with_file_name(name)
}

fn fresh_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{now:x}-{:x}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

fn storage_io_error(error: stamina_data::storage::StorageError) -> io::Error {
    let kind = if matches!(
        &error,
        stamina_data::storage::StorageError::RevisionConflict { .. }
    ) {
        io::ErrorKind::WouldBlock
    } else {
        io::ErrorKind::InvalidData
    };
    io::Error::new(kind, localization::StorageErrorMessage(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    type FailureOperation = (&'static str, Box<dyn Fn(&mut UserStore) -> io::Result<()>>);

    #[test]
    fn storage_revision_conflict_keeps_kind_and_typed_source_with_localized_text() {
        let error = storage_io_error(stamina_data::storage::StorageError::RevisionConflict {
            expected: 3,
            actual: 5,
        });
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(
            error.to_string(),
            "Данные профиля изменились в другом окне (ожидалась версия 3, текущая версия 5)."
        );
        assert!(matches!(
            error
                .get_ref()
                .and_then(|wrapper| wrapper.source())
                .and_then(|source| source.downcast_ref::<stamina_data::storage::StorageError>()),
            Some(stamina_data::storage::StorageError::RevisionConflict {
                expected: 3,
                actual: 5
            })
        ));
    }

    #[test]
    fn future_schema_error_retains_kind_source_and_selected_language() {
        localization::set_language("EN");
        let error = storage_io_error(stamina_data::storage::StorageError::FutureSchema(99));
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "Database schema version 99 is newer than supported."
        );
        assert!(
            error
                .get_ref()
                .unwrap()
                .source()
                .unwrap()
                .is::<stamina_data::storage::StorageError>()
        );
        localization::set_language("RU");
    }

    #[test]
    fn imported_profile_keeps_id_and_rejects_conflicts() {
        let path = test_path();
        let mut store = UserStore::new_at(&path);
        let profile = Profile {
            id: "portable-id".into(),
            name: "Portable".into(),
            results: Vec::new(),
            external_text: None,
            content_language: "EN".into(),
            interface_language: "RU".into(),
            selected_lessons: BTreeMap::new(),
            selected_mode: None,
            selected_layout: None,
            background: BackgroundImageSettings::default(),
            window_geometry: BTreeMap::new(),
            window_scales: BTreeMap::new(),
        };
        store.import_profile(profile.clone()).unwrap();
        assert_eq!(store.active_profile(), Some(&profile));
        assert_eq!(
            store.import_profile(profile).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        let reloaded = UserStore::load_from(&path).unwrap();
        assert_eq!(reloaded.active_profile_id(), "portable-id");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn profile_validation_errors_use_selected_locale_without_mutating_store() {
        let path = test_path();
        let mut store = UserStore::new_at(&path);
        let active_id = store.active_profile_id().to_owned();

        localization::set_language("RU");
        assert_eq!(
            store.create("  ").unwrap_err().to_string(),
            "Введите имя профиля"
        );
        assert_eq!(
            store.create("Пользователь").unwrap_err().to_string(),
            "Профиль с таким именем уже существует"
        );
        assert_eq!(
            store.delete(&active_id).unwrap_err().to_string(),
            "Нельзя удалить последний профиль"
        );
        assert_eq!(
            store.set_content_language("XX").unwrap_err().to_string(),
            "В профиле указан неверный язык текста"
        );
        assert_eq!(
            store.set_interface_language("XX").unwrap_err().to_string(),
            "В профиле указан неверный язык интерфейса"
        );

        localization::set_language("EN");
        assert_eq!(
            store.select("missing").unwrap_err().to_string(),
            "The profile was not found"
        );
        let duplicate = Profile {
            id: active_id.clone(),
            name: "Another name".into(),
            results: Vec::new(),
            external_text: None,
            content_language: "RU".into(),
            interface_language: "RU".into(),
            selected_lessons: BTreeMap::new(),
            selected_mode: None,
            selected_layout: None,
            background: BackgroundImageSettings::default(),
            window_geometry: BTreeMap::new(),
            window_scales: BTreeMap::new(),
        };
        assert_eq!(
            store.import_profile(duplicate).unwrap_err().to_string(),
            "A profile with this ID already exists"
        );
        assert_eq!(store.profiles().len(), 1);
        assert_eq!(store.active_profile_id(), active_id);

        localization::set_language("RU");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn saving_profile_without_parent_reports_localized_error_and_rolls_back() {
        for (locale, expected) in [
            ("RU", "У пути к профилю нет родительской папки"),
            ("EN", "The profile path has no parent directory"),
        ] {
            localization::set_language(locale);
            let mut store = UserStore::new_at(PathBuf::from("profiles.json"));
            let error = store.create("Alice").unwrap_err();
            assert_eq!(error.to_string(), expected);
            assert_eq!(store.profiles().len(), 1);
            assert_eq!(store.active_name(), "Пользователь");
        }
        localization::set_language("RU");
    }

    #[test]
    fn concurrent_profile_stores_reject_stale_writes_without_losing_data() {
        let legacy = test_path();
        let database = legacy.with_extension("sqlite3");
        let mut first = UserStore::load_database(database.clone(), &legacy).unwrap();
        let mut second = UserStore::load_database(database.clone(), &legacy).unwrap();
        first.create("Alice").unwrap();
        assert!(second.create("Bob").is_err());
        assert_eq!(second.profiles().len(), 1);
        let reloaded = UserStore::load_database(database.clone(), &legacy).unwrap();
        assert!(
            reloaded
                .profiles()
                .iter()
                .any(|profile| profile.name == "Alice")
        );
        assert!(
            !reloaded
                .profiles()
                .iter()
                .any(|profile| profile.name == "Bob")
        );
        fs::remove_file(database).unwrap();
    }

    #[test]
    fn restore_reloads_data_and_preserves_previous_database() {
        let legacy = test_path();
        let database_path = legacy.with_extension("sqlite3");
        let backup_path = legacy.with_extension("backup.sqlite3");
        let mut store = UserStore::load_database(database_path.clone(), &legacy).unwrap();
        store.create("Before backup").unwrap();
        store.set_content_language("FA").unwrap();
        store.set_selected_lesson("ktouch.ir.002").unwrap();
        store.set_training_selection("Text", "FA").unwrap();
        store
            .database()
            .unwrap()
            .unwrap()
            .backup_to(&backup_path)
            .unwrap();
        store.create("After backup").unwrap();
        let safety = store.restore_backup(&backup_path).unwrap();
        assert_eq!(store.active_name(), "Before backup");
        assert_eq!(
            store
                .active_profile()
                .unwrap()
                .selected_lessons
                .get("FA")
                .map(String::as_str),
            Some("ktouch.ir.002")
        );
        assert_eq!(
            store.active_profile().unwrap().selected_mode.as_deref(),
            Some("Text")
        );
        assert_eq!(
            store.active_profile().unwrap().selected_layout.as_deref(),
            Some("FA")
        );
        let saved = stamina_data::storage::Database::open(&safety)
            .unwrap()
            .load_snapshot()
            .unwrap();
        assert!(
            saved
                .profiles
                .iter()
                .any(|profile| profile.name == "After backup")
        );
        store.create("New profile").unwrap();
        let reopened = UserStore::load_database(database_path.clone(), &legacy).unwrap();
        assert_eq!(reopened.active_name(), "New profile");
        fs::remove_file(safety).unwrap();
        fs::remove_file(backup_path).unwrap();
        fs::remove_file(database_path).unwrap();
    }

    #[test]
    fn sqlite_import_and_profile_changes_survive_restart() {
        let legacy = test_path();
        let database = legacy.with_extension("sqlite3");
        let mut old = UserStore::new_at(&legacy);
        let id = old.create("Alice").unwrap().id.clone();
        old.record(ResultInput {
            mode: "lesson",
            layout: "ru",
            exercise: "one",
            correct: 3,
            errors: 1,
            elapsed_ms: 1000,
            cpm: 180.0,
            error_percent: 25.0,
        })
        .unwrap();
        let source = fs::read(&legacy).unwrap();
        let mut store = UserStore::load_database(database.clone(), &legacy).unwrap();
        assert_eq!(store.active_profile_id(), id);
        assert_eq!(store.active_records().len(), 1);
        store.rename(&id, "Bob").unwrap();
        let progress = ExternalProgress {
            path: PathBuf::from("book.txt"),
            size: 20,
            hash: 12,
            encoding: "Utf8".into(),
            position: 7,
        };
        store
            .record_with_progress(
                ResultInput {
                    mode: "file",
                    layout: "ru",
                    exercise: "book",
                    correct: 5,
                    errors: 0,
                    elapsed_ms: 1000,
                    cpm: 300.0,
                    error_percent: 0.0,
                },
                Some(progress.clone()),
                "manual",
            )
            .unwrap();
        let reopened = UserStore::load_database(database.clone(), &legacy).unwrap();
        assert_eq!(reopened.active_name(), "Bob");
        assert_eq!(reopened.active_records().len(), 2);
        assert_eq!(
            reopened.active_profile().unwrap().external_text,
            Some(progress)
        );
        assert_eq!(fs::read(&legacy).unwrap(), source);
        fs::remove_file(legacy).unwrap();
        fs::remove_file(database).unwrap();
    }

    #[test]
    fn database_load_imports_backup_when_legacy_primary_is_missing() {
        let legacy = test_path();
        let backup = sibling_path(&legacy, "bak");
        let database = legacy.with_extension("sqlite3");
        let mut old = UserStore::new_at(&legacy);
        let id = old.create("Recovered backup profile").unwrap().id.clone();
        let source_bytes = fs::read(&legacy).unwrap();
        fs::rename(&legacy, &backup).unwrap();

        let recovered = UserStore::load_database(database.clone(), &legacy).unwrap();

        assert_eq!(recovered.active_profile_id(), id);
        assert_eq!(recovered.active_name(), "Recovered backup profile");
        assert_eq!(fs::read(&backup).unwrap(), source_bytes);
        let reopened = UserStore::load_database(database.clone(), &legacy).unwrap();
        assert_eq!(reopened.active_name(), "Recovered backup profile");
        assert_eq!(reopened.profiles().len(), 2);

        let _ = fs::remove_file(backup);
        let _ = fs::remove_file(database);
    }

    #[test]
    fn failed_legacy_import_preserves_corrupt_source_and_can_be_retried() {
        let legacy = test_path();
        let database = legacy.with_extension("sqlite3");
        let corrupt = b"{ broken legacy JSON";
        fs::write(&legacy, corrupt).unwrap();

        let error = UserStore::load_database(database.clone(), &legacy).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&legacy).unwrap(), corrupt);
        let valid = br#"{"active_profile_id":"restored","profiles":[{"id":"restored","name":"Restored profile","results":[]}] }"#;
        fs::write(&legacy, valid).unwrap();

        let imported = UserStore::load_database(database.clone(), &legacy).unwrap();

        assert_eq!(imported.active_profile_id(), "restored");
        assert_eq!(imported.active_name(), "Restored profile");
        assert_eq!(imported.profiles().len(), 1);
        assert_eq!(fs::read(&legacy).unwrap(), valid);
        let reopened = UserStore::load_database(database.clone(), &legacy).unwrap();
        assert_eq!(reopened.active_name(), "Restored profile");

        let _ = fs::remove_file(legacy);
        let _ = fs::remove_file(database);
    }

    #[test]
    fn failed_profile_mutations_restore_json_memory_and_file() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        let alice = store.create("Alice").unwrap().id.clone();
        let bob = store.create("Bob").unwrap().id.clone();
        store.select(&bob).unwrap();
        let before_memory = serde_json::to_value(&store).unwrap();
        let before_file = fs::read(&path).unwrap();
        let temp_blocker = sibling_path(&path, "tmp");
        fs::create_dir(&temp_blocker).unwrap();

        let failures: [FailureOperation; 5] = [
            (
                "select",
                Box::new({
                    let alice = alice.clone();
                    move |s| s.select(&alice)
                }),
            ),
            (
                "rename",
                Box::new({
                    let alice = alice.clone();
                    move |s| s.rename(&alice, "Alicia")
                }),
            ),
            (
                "delete",
                Box::new({
                    let alice = alice.clone();
                    move |s| s.delete(&alice)
                }),
            ),
            (
                "set_content_language",
                Box::new(|s| s.set_content_language("EN")),
            ),
            (
                "set_selected_lesson",
                Box::new(|s| s.set_selected_lesson("ktouch.ru1.001")),
            ),
        ];
        for (operation, mutate) in failures {
            let error = mutate(&mut store).expect_err(operation);
            assert_ne!(error.kind(), io::ErrorKind::NotFound, "{operation}");
            assert_eq!(
                serde_json::to_value(&store).unwrap(),
                before_memory,
                "{operation} memory rollback"
            );
            assert_eq!(
                fs::read(&path).unwrap(),
                before_file,
                "{operation} file preservation"
            );
        }

        fs::remove_dir(temp_blocker).unwrap();
        let reloaded = UserStore::load_from(&path).unwrap();
        assert_eq!(reloaded.active_profile_id(), bob);
        assert!(
            reloaded
                .profiles()
                .iter()
                .any(|p| p.id == alice && p.name == "Alice")
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn failed_database_profile_mutations_restore_memory_and_preserve_database() {
        let legacy = test_path();
        let database = legacy.with_extension("sqlite3");
        let preserved = legacy.with_extension("sqlite3.preserved");
        let mut initial = UserStore::new_at(&legacy);
        let alice = initial.create("Alice").unwrap().id.clone();
        let bob = initial.create("Bob").unwrap().id.clone();
        drop(initial);
        let mut store = UserStore::load_database(database.clone(), &legacy).unwrap();
        store.select(&bob).unwrap();
        drop(store);
        let mut store = UserStore::load_database(database.clone(), &legacy).unwrap();
        let before_memory = serde_json::to_value(&store).unwrap();
        fs::copy(&database, &preserved).unwrap();
        let failures: [FailureOperation; 5] = [
            (
                "select",
                Box::new({
                    let alice = alice.clone();
                    move |s| s.select(&alice)
                }),
            ),
            (
                "rename",
                Box::new({
                    let alice = alice.clone();
                    move |s| s.rename(&alice, "Alicia")
                }),
            ),
            (
                "delete",
                Box::new({
                    let alice = alice.clone();
                    move |s| s.delete(&alice)
                }),
            ),
            (
                "set_content_language",
                Box::new(|s| s.set_content_language("EN")),
            ),
            (
                "set_selected_lesson",
                Box::new(|s| s.set_selected_lesson("ktouch.ru1.001")),
            ),
        ];
        for (operation, mutate) in failures {
            fs::remove_file(&database).unwrap();
            fs::create_dir(&database).unwrap();
            let error = mutate(&mut store).expect_err(operation);
            assert_ne!(error.kind(), io::ErrorKind::NotFound, "{operation}");
            assert_eq!(
                serde_json::to_value(&store).unwrap(),
                before_memory,
                "{operation} memory rollback"
            );
            fs::remove_dir(&database).unwrap();
            fs::copy(&preserved, &database).unwrap();
            assert_eq!(
                fs::read(&database).unwrap(),
                fs::read(&preserved).unwrap(),
                "{operation} database preservation"
            );
        }
        let reloaded = UserStore::load_database(database.clone(), &legacy).unwrap();
        assert_eq!(reloaded.active_profile_id(), bob);
        assert!(
            reloaded
                .profiles()
                .iter()
                .any(|p| p.id == alice && p.name == "Alice")
        );
        drop(store);
        let _ = fs::remove_file(legacy);
        let _ = fs::remove_file(database);
        let _ = fs::remove_file(preserved);
    }

    #[test]
    fn failed_result_and_progress_leave_previous_state_for_retry() {
        let blocker = test_path();
        fs::write(&blocker, "not a directory").unwrap();
        let mut store = UserStore::new_at(blocker.join("profiles.json"));
        let progress = ExternalProgress {
            path: PathBuf::from("book.txt"),
            size: 10,
            hash: 1,
            encoding: "Utf8".into(),
            position: 3,
        };
        let result = store.record_with_progress(
            ResultInput {
                mode: "file",
                layout: "ru",
                exercise: "book",
                correct: 1,
                errors: 0,
                elapsed_ms: 100,
                cpm: 600.0,
                error_percent: 0.0,
            },
            Some(progress),
            "manual",
        );
        assert!(result.is_err());
        assert!(store.active_records().is_empty());
        assert!(store.active_profile().unwrap().external_text.is_none());
        fs::remove_file(blocker).unwrap();
    }

    #[test]
    fn result_with_progress_persists_stop_reason_and_keeps_progress_when_omitted() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        let progress = ExternalProgress {
            path: PathBuf::from("book.txt"),
            size: 128,
            hash: 42,
            encoding: "Utf8".into(),
            position: 17,
        };
        let input = |exercise| ResultInput {
            mode: "file",
            layout: "ru",
            exercise,
            correct: 12,
            errors: 2,
            elapsed_ms: 5_000,
            cpm: 144.0,
            error_percent: 14.3,
        };

        store
            .record_with_progress(input("book.txt"), Some(progress.clone()), "manual")
            .unwrap();
        store
            .record_with_progress(input("book.txt"), None, "completed")
            .unwrap();

        let reloaded = UserStore::load_from(&path).unwrap();
        assert_eq!(reloaded.active_records().len(), 2);
        assert_eq!(reloaded.active_records()[0].stop_reason, "manual");
        assert_eq!(reloaded.active_records()[1].stop_reason, "completed");
        assert_eq!(
            reloaded.active_profile().unwrap().external_text,
            Some(progress)
        );
        let _ = fs::remove_file(path);
    }

    fn test_path() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        std::env::temp_dir().join(format!(
            "staminator-user-store-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn create_select_delete_and_roundtrip() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        let default_id = store.active_profile_id().to_owned();
        let created_id = store.create("Alice").unwrap().id.clone();
        assert_eq!(store.active_name(), "Alice");
        store
            .record(ResultInput {
                mode: "Words",
                layout: "EN",
                exercise: "Home row",
                correct: 80,
                errors: 2,
                elapsed_ms: 12_000,
                cpm: 400.0,
                error_percent: 2.4,
            })
            .unwrap();
        store.create("Bob").unwrap();
        store.select(&created_id).unwrap();
        assert_eq!(store.active_records().len(), 1);
        store.select(&default_id).unwrap();
        store.delete(&created_id).unwrap();

        let loaded = UserStore::load_from(&path).unwrap();
        assert_eq!(loaded.profiles().len(), 2);
        assert_eq!(loaded.active_profile_id(), default_id);
        assert!(loaded.profiles().iter().all(|p| p.id != created_id));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn content_language_is_profile_scoped_and_legacy_profiles_default_to_russian() {
        let path = test_path();
        fs::write(&path, br#"{"active_profile_id":"legacy","profiles":[{"id":"legacy","name":"Legacy","results":[]},{"id":"english","name":"English","results":[]}] }"#).unwrap();
        let mut store = UserStore::load_from(&path).unwrap();
        assert_eq!(store.active_profile().unwrap().content_language, "RU");
        store.set_content_language("EN").unwrap();
        store.select("english").unwrap();
        assert_eq!(store.active_profile().unwrap().content_language, "RU");
        store.set_content_language("EN").unwrap();
        store.select("legacy").unwrap();
        assert_eq!(store.active_profile().unwrap().content_language, "EN");
        let loaded = UserStore::load_from(&path).unwrap();
        assert_eq!(loaded.active_profile().unwrap().content_language, "EN");
        assert_eq!(
            loaded
                .profiles()
                .iter()
                .find(|p| p.id == "english")
                .unwrap()
                .content_language,
            "EN"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn content_and_interface_languages_support_all_locales_independently_and_persist() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        let locales = [
            "RU", "EN", "ES", "AR", "FR", "PT-BR", "DE", "IT", "TR", "FA",
        ];

        for (index, locale) in locales.iter().enumerate() {
            store.set_content_language(locale).unwrap();
            let interface_locale = locales[(index + 1) % locales.len()];
            store.set_interface_language(interface_locale).unwrap();
            assert_eq!(store.active_profile().unwrap().content_language, *locale);
            assert_eq!(
                store.active_profile().unwrap().interface_language,
                interface_locale
            );
        }

        let reopened = UserStore::load_from(&path).unwrap();
        assert_eq!(reopened.active_profile().unwrap().content_language, "FA");
        assert_eq!(reopened.active_profile().unwrap().interface_language, "RU");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn language_aliases_are_canonicalized_before_saving_profile() {
        let path = test_path();
        let mut store = UserStore::new_at(&path);
        assert_eq!(
            store.set_selected_lesson("  ").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        store.set_content_language("pt_br").unwrap();
        store.set_interface_language("en").unwrap();
        store.set_selected_lesson("ktouch.br.001").unwrap();
        let loaded = UserStore::load_from(&path).unwrap();
        let profile = loaded.active_profile().unwrap();
        assert_eq!(profile.content_language, "PT-BR");
        assert_eq!(profile.interface_language, "EN");
        assert_eq!(
            profile.selected_lessons.get("PT-BR").map(String::as_str),
            Some("ktouch.br.001")
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn background_defaults_to_builtin_and_solid_color_selection_survives_restart() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        let profile = store.create("Background profile").unwrap().clone();
        assert_eq!(profile.background, BackgroundImageSettings::default());

        let solid_color = BackgroundImageSettings {
            image_path: None,
            use_builtin: false,
            placement: BackgroundPlacement::Fill,
            dim_percent: 35,
        };
        store.set_background_settings(solid_color.clone()).unwrap();
        let reopened = UserStore::load_from(&path).unwrap();

        assert_eq!(reopened.background_settings(), Some(&solid_color));
        assert!(reopened.background_settings().unwrap().image_path.is_none());
        assert!(!reopened.background_settings().unwrap().use_builtin);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_solid_default_without_source_enables_builtin_but_explicit_solid_is_preserved() {
        let path = test_path();
        let legacy = r#"{"profiles":[{"id":"p","name":"Default","background":{"image_path":null,"placement":"fit","dim_percent":0}}],"active_profile_id":"p"}"#;
        fs::write(&path, legacy).unwrap();
        let mut store = UserStore::load_from(&path).unwrap();
        assert!(store.background_settings().unwrap().use_builtin);
        assert_eq!(
            store.background_settings().unwrap().placement,
            BackgroundPlacement::Fill
        );
        let settings = BackgroundImageSettings {
            use_builtin: false,
            ..Default::default()
        };
        store.set_background_settings(settings.clone()).unwrap();
        assert_eq!(
            UserStore::load_from(&path).unwrap().background_settings(),
            Some(&settings)
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_builtin_fit_default_migrates_once_and_new_explicit_fit_persists() {
        let path = test_path();
        let legacy = r#"{"profiles":[{"id":"p","name":"Default","background":{"image_path":null,"use_builtin":true,"placement":"fit","dim_percent":0}}],"active_profile_id":"p"}"#;
        fs::write(&path, legacy).unwrap();

        let mut store = UserStore::load_from(&path).unwrap();
        assert_eq!(
            store.background_settings().unwrap().placement,
            BackgroundPlacement::Fill
        );
        let explicit_fit = BackgroundImageSettings {
            placement: BackgroundPlacement::Fit,
            ..BackgroundImageSettings::default()
        };
        store.set_background_settings(explicit_fit.clone()).unwrap();
        assert_eq!(
            UserStore::load_from(&path).unwrap().background_settings(),
            Some(&explicit_fit)
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reopening_unchanged_database_does_not_advance_revision_for_json_marker() {
        let path = test_path().with_extension("sqlite3");
        let legacy = path.with_extension("json");
        let store = UserStore::load_database(path.clone(), &legacy).unwrap();
        let first = store
            .database()
            .unwrap()
            .unwrap()
            .load_snapshot_with_revision()
            .unwrap()
            .revision;
        drop(store);

        let reopened = UserStore::load_database(path.clone(), &legacy).unwrap();
        let second = reopened
            .database()
            .unwrap()
            .unwrap()
            .load_snapshot_with_revision()
            .unwrap()
            .revision;
        assert_eq!(second, first);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_custom_background_without_use_builtin_remains_a_custom_image() {
        let path = test_path();
        fs::write(
            &path,
            br#"{"active_profile_id":"legacy","profiles":[{"id":"legacy","name":"Legacy","results":[],"background":{"image_path":"old-picture.jpg","placement":"fill","dim_percent":12}}]}"#,
        )
        .unwrap();

        let store = UserStore::load_from(&path).unwrap();
        let settings = store.background_settings().unwrap();
        assert!(!settings.use_builtin);
        assert_eq!(settings.image_path, Some(PathBuf::from("old-picture.jpg")));
        assert_eq!(settings.placement, BackgroundPlacement::Fill);
        assert_eq!(settings.dim_percent, 12);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn interface_language_is_profile_scoped_and_persists_across_switches() {
        let path = test_path().with_extension("sqlite3");
        let legacy = path.with_extension("json");
        let mut store = UserStore::load_database(path.clone(), &legacy).unwrap();
        let first = store.active_profile_id().to_owned();
        store.set_content_language("EN").unwrap();
        store.set_interface_language("EN").unwrap();
        store
            .set_background_settings(BackgroundImageSettings {
                image_path: Some(PathBuf::from("/photos/lesson.png")),
                use_builtin: false,
                placement: BackgroundPlacement::Fill,
                dim_percent: 45,
            })
            .unwrap();
        let second = store.create("Second").unwrap().id.clone();
        assert_eq!(store.active_profile().unwrap().interface_language, "RU");
        store.set_interface_language("EN").unwrap();
        store
            .set_background_settings(BackgroundImageSettings {
                image_path: Some(PathBuf::from("/photos/second.jpg")),
                use_builtin: false,
                placement: BackgroundPlacement::Tile,
                dim_percent: 20,
            })
            .unwrap();
        store.select(&first).unwrap();
        assert_eq!(store.active_profile().unwrap().content_language, "EN");
        assert_eq!(store.active_profile().unwrap().interface_language, "EN");
        store.select(&second).unwrap();
        assert_eq!(store.active_profile().unwrap().interface_language, "EN");

        let reopened = UserStore::load_database(path.clone(), &legacy).unwrap();
        assert_eq!(reopened.active_profile_id(), second);
        assert_eq!(reopened.active_profile().unwrap().interface_language, "EN");
        assert_eq!(
            reopened.active_profile().unwrap().background.placement,
            BackgroundPlacement::Tile
        );
        assert_eq!(
            reopened.active_profile().unwrap().background.dim_percent,
            20
        );
        assert_eq!(
            reopened
                .profiles()
                .iter()
                .find(|p| p.id == first)
                .unwrap()
                .content_language,
            "EN"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn selected_lessons_are_scoped_by_profile_and_content_language_in_sqlite() {
        let path = test_path().with_extension("sqlite3");
        let legacy = path.with_extension("json");
        let mut store = UserStore::load_database(path.clone(), &legacy).unwrap();
        let first = store.active_profile_id().to_owned();
        store.set_selected_lesson("RU basics").unwrap();
        store.set_content_language("EN").unwrap();
        store.set_selected_lesson("English basics").unwrap();

        let second = store.create("Second").unwrap().id.clone();
        store.set_content_language("EN").unwrap();
        store.set_selected_lesson("Second profile lesson").unwrap();
        store.select(&first).unwrap();
        store.set_content_language("RU").unwrap();
        assert_eq!(
            store
                .active_profile()
                .unwrap()
                .selected_lessons
                .get("RU")
                .map(String::as_str),
            Some("RU basics")
        );
        assert_eq!(
            store
                .active_profile()
                .unwrap()
                .selected_lessons
                .get("EN")
                .map(String::as_str),
            Some("English basics")
        );

        let mut reopened = UserStore::load_database(path.clone(), &legacy).unwrap();
        assert_eq!(
            reopened
                .active_profile()
                .unwrap()
                .selected_lessons
                .get("RU")
                .map(String::as_str),
            Some("RU basics")
        );
        assert_eq!(
            reopened
                .active_profile()
                .unwrap()
                .selected_lessons
                .get("EN")
                .map(String::as_str),
            Some("English basics")
        );
        reopened.select(&second).unwrap();
        assert_eq!(
            reopened
                .active_profile()
                .unwrap()
                .selected_lessons
                .get("EN")
                .map(String::as_str),
            Some("Second profile lesson")
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn selected_lessons_are_saved_per_language_and_profile_in_json() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        let first = store.active_profile_id().to_owned();
        store.set_selected_lesson("Урок RU").unwrap();
        store.set_content_language("EN").unwrap();
        store.set_selected_lesson("Lesson EN").unwrap();
        let second = store.create("Second").unwrap().id.clone();
        store.set_selected_lesson("Second lesson").unwrap();
        store.select(&first).unwrap();

        let mut reopened = UserStore::load_from(&path).unwrap();
        assert_eq!(
            reopened
                .active_profile()
                .unwrap()
                .selected_lessons
                .get("RU")
                .map(String::as_str),
            Some("Урок RU")
        );
        assert_eq!(
            reopened
                .active_profile()
                .unwrap()
                .selected_lessons
                .get("EN")
                .map(String::as_str),
            Some("Lesson EN")
        );
        reopened.select(&second).unwrap();
        assert_eq!(
            reopened
                .active_profile()
                .unwrap()
                .selected_lessons
                .get("RU")
                .map(String::as_str),
            Some("Second lesson")
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn training_selection_is_profile_scoped_and_persists_in_json() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        let first = store.active_profile_id().to_owned();
        store.set_training_selection("Text", "EN").unwrap();
        let second = store.create("Second").unwrap().id.clone();
        store.set_training_selection("Words", "RU").unwrap();
        store.select(&first).unwrap();

        let mut reopened = UserStore::load_from(&path).unwrap();
        assert_eq!(
            reopened.active_profile().unwrap().selected_mode.as_deref(),
            Some("Text")
        );
        assert_eq!(
            reopened
                .active_profile()
                .unwrap()
                .selected_layout
                .as_deref(),
            Some("EN")
        );
        reopened.select(&second).unwrap();
        assert_eq!(
            reopened.active_profile().unwrap().selected_mode.as_deref(),
            Some("Words")
        );
        assert_eq!(
            reopened
                .active_profile()
                .unwrap()
                .selected_layout
                .as_deref(),
            Some("RU")
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn training_selection_is_profile_scoped_and_persists_in_sqlite() {
        let path = test_path().with_extension("sqlite3");
        let legacy = path.with_extension("json");
        let mut store = UserStore::load_database(path.clone(), &legacy).unwrap();
        let first = store.active_profile_id().to_owned();
        store.set_training_selection("Text", "EN").unwrap();
        let second = store.create("Second").unwrap().id.clone();
        store.set_training_selection("Words", "RU").unwrap();

        let mut reopened = UserStore::load_database(path.clone(), &legacy).unwrap();
        assert_eq!(
            reopened.active_profile().unwrap().selected_mode.as_deref(),
            Some("Words")
        );
        reopened.select(&first).unwrap();
        assert_eq!(
            reopened.active_profile().unwrap().selected_mode.as_deref(),
            Some("Text")
        );
        assert_eq!(
            reopened
                .active_profile()
                .unwrap()
                .selected_layout
                .as_deref(),
            Some("EN")
        );
        reopened.select(&second).unwrap();
        assert_eq!(
            reopened.active_profile().unwrap().selected_mode.as_deref(),
            Some("Words")
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn imports_legacy_json_training_selection_into_sqlite_settings() {
        let legacy = test_path();
        let database = legacy.with_extension("sqlite3");
        let mut source = UserStore::new_at(&legacy);
        source.set_training_selection("Words", "DE").unwrap();
        source.set_content_language("DE").unwrap();
        source.set_selected_lesson("lesson-de-01").unwrap();
        let profile_id = source.active_profile_id().to_owned();

        let imported = UserStore::load_database(database.clone(), &legacy).unwrap();
        assert_eq!(imported.active_profile_id(), profile_id);
        assert_eq!(
            imported.active_profile().unwrap().selected_mode.as_deref(),
            Some("Words")
        );
        assert_eq!(
            imported
                .active_profile()
                .unwrap()
                .selected_layout
                .as_deref(),
            Some("DE")
        );
        assert_eq!(
            imported
                .active_profile()
                .unwrap()
                .selected_lessons
                .get("DE")
                .map(String::as_str),
            Some("lesson-de-01")
        );

        let reopened = UserStore::load_database(database.clone(), &legacy).unwrap();
        assert_eq!(
            reopened.active_profile().unwrap().selected_mode.as_deref(),
            Some("Words")
        );
        assert_eq!(
            reopened
                .active_profile()
                .unwrap()
                .selected_layout
                .as_deref(),
            Some("DE")
        );
        let _ = fs::remove_file(legacy);
        let _ = fs::remove_file(database);
    }

    #[test]
    fn failed_training_selection_write_restores_profile_state() {
        let blocker = test_path();
        fs::write(&blocker, b"not a directory").unwrap();
        let path = blocker.join("profiles.json");
        let mut store = UserStore::new_at(&path);

        assert!(store.set_training_selection("Text", "EN").is_err());
        assert_eq!(store.active_profile().unwrap().selected_mode, None);
        assert_eq!(store.active_profile().unwrap().selected_layout, None);
        let _ = fs::remove_file(blocker);
    }

    #[test]
    fn failed_sqlite_training_selection_open_leaves_profile_state_unchanged() {
        let database = test_path().with_extension("sqlite3");
        let legacy = database.with_extension("json");
        let mut store = UserStore::load_database(database.clone(), &legacy).unwrap();
        store.path = database.parent().unwrap().to_path_buf();

        assert!(store.set_training_selection("Text", "EN").is_err());
        assert_eq!(store.active_profile().unwrap().selected_mode, None);
        assert_eq!(store.active_profile().unwrap().selected_layout, None);
        let _ = fs::remove_file(database);
    }

    #[test]
    fn rejects_duplicate_empty_names_and_final_delete() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        assert!(store.create("  ").is_err());
        store.create("Alice").unwrap();
        assert!(store.create("alice").is_err());
        let ids: Vec<_> = store.profiles().iter().map(|p| p.id.clone()).collect();
        store.delete(&ids[0]).unwrap();
        assert!(store.delete(&ids[1]).is_err());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn recovers_profile_file_left_in_backup_after_interrupted_replace() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        store.create("Alice").unwrap();
        let backup = sibling_path(&path, "bak");
        fs::rename(&path, &backup).unwrap();
        let recovered = UserStore::load_from(&path).unwrap();
        assert_eq!(recovered.active_name(), "Alice");
        let _ = fs::remove_file(backup);
    }

    #[test]
    fn reports_corrupt_profile_file_without_replacing_it_with_defaults() {
        let path = test_path();
        fs::write(&path, b"{ not valid json").unwrap();

        let error = UserStore::load_from(&path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&path).unwrap(), b"{ not valid json");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn uses_backup_when_primary_file_is_missing_then_replaces_it_on_save() {
        let path = test_path();
        let backup = sibling_path(&path, "bak");
        let mut original = UserStore::load_from(&path).unwrap();
        original.create("Recovered").unwrap();
        fs::rename(&path, &backup).unwrap();

        let mut recovered = UserStore::load_from(&path).unwrap();
        assert_eq!(recovered.active_name(), "Recovered");
        recovered.create("After recovery").unwrap();

        let reloaded = UserStore::load_from(&path).unwrap();
        assert_eq!(reloaded.active_name(), "After recovery");
        assert_eq!(reloaded.profiles().len(), 3);
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(backup);
    }

    #[test]
    fn result_history_is_isolated_by_profile_and_survives_reload() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        let default_id = store.active_profile_id().to_owned();
        store
            .record(ResultInput {
                mode: "Words",
                layout: "EN",
                exercise: "Default exercise",
                correct: 10,
                errors: 1,
                elapsed_ms: 2_000,
                cpm: 300.0,
                error_percent: 9.1,
            })
            .unwrap();

        let alice_id = store.create("Alice").unwrap().id.clone();
        assert!(store.active_records().is_empty());
        store
            .record(ResultInput {
                mode: "Text",
                layout: "RU",
                exercise: "Alice exercise",
                correct: 50,
                errors: 0,
                elapsed_ms: 8_000,
                cpm: 375.0,
                error_percent: 0.0,
            })
            .unwrap();

        store.select(&default_id).unwrap();
        assert_eq!(store.active_records().len(), 1);
        assert_eq!(store.active_records()[0].exercise, "Default exercise");
        store.select(&alice_id).unwrap();
        assert_eq!(store.active_records().len(), 1);
        assert_eq!(store.active_records()[0].exercise, "Alice exercise");

        let reloaded = UserStore::load_from(&path).unwrap();
        let default = reloaded
            .profiles()
            .iter()
            .find(|p| p.id == default_id)
            .unwrap();
        let alice = reloaded
            .profiles()
            .iter()
            .find(|p| p.id == alice_id)
            .unwrap();
        assert_eq!(default.results.len(), 1);
        assert_eq!(default.results[0].exercise, "Default exercise");
        assert_eq!(alice.results.len(), 1);
        assert_eq!(alice.results[0].exercise, "Alice exercise");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn failed_create_save_returns_io_error_and_does_not_create_store_file() {
        let blocker = test_path();
        fs::write(&blocker, b"not a directory").unwrap();
        let path = blocker.join("profiles.json");
        let mut store = UserStore::new_at(&path);

        let error = store.create("Unsaved").unwrap_err();

        assert!(matches!(
            error.kind(),
            io::ErrorKind::AlreadyExists
                | io::ErrorKind::NotADirectory
                | io::ErrorKind::Other
                | io::ErrorKind::PermissionDenied
        ));
        assert!(!path.exists());
        assert_eq!(store.profiles().len(), 1);
        assert_ne!(store.active_name(), "Unsaved");
        let _ = fs::remove_file(blocker);
    }

    #[test]
    fn normalizes_duplicate_and_blank_profile_fields() {
        let path = test_path();
        let data = serde_json::json!({
            "active_profile_id": "missing",
            "profiles": [
                {"id": "same", "name": "Alice", "results": []},
                {"id": "same", "name": "alice", "results": []},
                {"id": "  ", "name": "Ignored id", "results": []},
                {"id": "empty-name", "name": " \t ", "results": []}
            ]
        });
        fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();

        let store = UserStore::load_from(&path).unwrap();

        assert_eq!(store.profiles().len(), 2);
        assert_eq!(store.profiles()[0].name, "Alice");
        assert_eq!(store.profiles()[1].name, "alice (2)");
        assert_ne!(store.profiles()[0].id, store.profiles()[1].id);
        assert_eq!(store.active_profile_id(), store.profiles()[0].id);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reports_not_found_and_invalid_input_for_profile_operations() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        let only_id = store.active_profile_id().to_owned();

        assert_eq!(
            store.select("missing").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            store.rename("missing", "Renamed").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            store.rename(&only_id, "  ").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );

        let second_id = store.create("Second").unwrap().id.clone();
        assert_eq!(
            store.rename(&second_id, "Пользователь").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            store.delete("missing").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn loading_and_recording_trim_history_to_latest_limit() {
        let path = test_path();
        let results: Vec<_> = (0..=MAX_RESULTS_PER_PROFILE)
            .map(|index| ResultRecord {
                timestamp: index as u64,
                mode: "Words".into(),
                layout: "EN".into(),
                exercise: format!("exercise-{index}"),
                correct: index as u64,
                errors: 0,
                elapsed_ms: 1_000,
                cpm: 60.0,
                error_percent: 0.0,
                stop_reason: "unknown".into(),
            })
            .collect();
        let data = serde_json::json!({
            "active_profile_id": "profile",
            "profiles": [{"id": "profile", "name": "History", "results": results}]
        });
        fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();

        let mut store = UserStore::load_from(&path).unwrap();
        assert_eq!(store.active_records().len(), MAX_RESULTS_PER_PROFILE);
        assert_eq!(
            store.active_records().first().unwrap().exercise,
            "exercise-1"
        );
        assert_eq!(
            store.active_records().last().unwrap().exercise,
            format!("exercise-{MAX_RESULTS_PER_PROFILE}")
        );

        store
            .record(ResultInput {
                mode: "Words",
                layout: "EN",
                exercise: "newest",
                correct: 1,
                errors: 0,
                elapsed_ms: 1_000,
                cpm: 60.0,
                error_percent: 0.0,
            })
            .unwrap();

        let records = store.active_records();
        assert_eq!(records.len(), MAX_RESULTS_PER_PROFILE);
        assert_eq!(records.first().unwrap().exercise, "exercise-2");
        assert_eq!(records.last().unwrap().exercise, "newest");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn window_geometry_is_scoped_by_window_and_profile_and_persists_in_json() {
        let path = test_path();
        let mut store = UserStore::new_at(&path);
        let first_id = store.active_profile_id().to_owned();
        let first = crate::window_geometry::SavedWindowGeometry {
            rect: crate::window_geometry::LogicalRect::new(20.0, 30.0, 800.0, 600.0),
            pixels_per_point: None,
            maximized: false,
            visible: true,
        };
        store.set_window_geometry("main", Some(first)).unwrap();
        let second_id = store.create("Second").unwrap().id.clone();
        let second = crate::window_geometry::SavedWindowGeometry {
            rect: crate::window_geometry::LogicalRect::new(100.0, 120.0, 900.0, 700.0),
            pixels_per_point: None,
            maximized: true,
            visible: false,
        };
        store.set_window_geometry("keyboard", Some(second)).unwrap();
        store.select(&first_id).unwrap();
        assert_eq!(store.window_geometry("main").unwrap(), Some(first));
        assert_eq!(store.window_geometry("keyboard").unwrap(), None);
        let mut reopened = UserStore::load_from(&path).unwrap();
        reopened.select(&first_id).unwrap();
        assert_eq!(reopened.window_geometry("main").unwrap(), Some(first));
        reopened.select(&second_id).unwrap();
        assert_eq!(reopened.window_geometry("keyboard").unwrap(), Some(second));
        reopened.set_window_geometry("keyboard", None).unwrap();
        let mut reopened = UserStore::load_from(&path).unwrap();
        reopened.select(&first_id).unwrap();
        assert_eq!(reopened.window_geometry("main").unwrap(), Some(first));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn sqlite_window_geometry_is_profile_scoped_and_survives_restart_and_reset() {
        let path = test_path().with_extension("sqlite3");
        let legacy = path.with_extension("json");
        let mut store = UserStore::load_database(path.clone(), &legacy).unwrap();
        let first_id = store.active_profile_id().to_owned();
        let first = crate::window_geometry::SavedWindowGeometry {
            rect: crate::window_geometry::LogicalRect::new(40.0, 50.0, 800.0, 600.0),
            pixels_per_point: None,
            maximized: true,
            visible: true,
        };
        store.set_window_geometry("main", Some(first)).unwrap();
        let second_id = store.create("Second").unwrap().id.clone();
        assert_eq!(store.window_geometry("main").unwrap(), None);
        store.set_window_geometry("main", None).unwrap();
        store.select(&first_id).unwrap();
        assert_eq!(store.window_geometry("main").unwrap(), Some(first));
        let mut reopened = UserStore::load_database(path.clone(), &legacy).unwrap();
        assert_eq!(reopened.window_geometry("main").unwrap(), Some(first));
        reopened.select(&second_id).unwrap();
        assert_eq!(reopened.window_geometry("main").unwrap(), None);
        reopened
            .set_window_geometry("keyboard", Some(first))
            .unwrap();
        let reopened = UserStore::load_database(path.clone(), &legacy).unwrap();
        assert_eq!(reopened.window_geometry("keyboard").unwrap(), Some(first));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn malformed_window_geometry_uses_safe_none_fallback() {
        let path = test_path();
        fs::write(&path, br#"{"active_profile_id":"p","profiles":[{"id":"p","name":"P","window_geometry":{"main":{"rect":{"x":0,"y":0,"width":-1,"height":600}}}}]}"#).unwrap();
        let mut store = UserStore::load_from(&path).unwrap();
        assert_eq!(store.window_geometry("main").unwrap(), None);
        assert_eq!(
            store
                .set_window_geometry(
                    "main",
                    Some(crate::window_geometry::SavedWindowGeometry {
                        rect: crate::window_geometry::LogicalRect::new(0.0, 0.0, 20.0, 20.0),
                        pixels_per_point: None,
                        maximized: false,
                        visible: true
                    })
                )
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn window_scale_is_scoped_by_window_and_profile_and_persists_in_json() {
        let path = test_path();
        let mut store = UserStore::new_at(&path);
        let first_id = store.active_profile_id().to_owned();
        assert_eq!(store.window_scale("main").unwrap(), 1.0);
        store.set_window_scale("main", 1.25).unwrap();
        let second_id = store.create("Second").unwrap().id.clone();
        assert_eq!(store.window_scale("main").unwrap(), 1.0);
        store.set_window_scale("keyboard", 2.0).unwrap();
        store.select(&first_id).unwrap();
        assert_eq!(store.window_scale("main").unwrap(), 1.25);
        assert_eq!(store.window_scale("keyboard").unwrap(), 1.0);
        let mut reopened = UserStore::load_from(&path).unwrap();
        reopened.select(&second_id).unwrap();
        assert_eq!(reopened.window_scale("keyboard").unwrap(), 2.0);
        assert_eq!(reopened.window_scale("main").unwrap(), 1.0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn sqlite_window_scale_is_profile_scoped_and_survives_restart() {
        let path = test_path().with_extension("sqlite3");
        let legacy = path.with_extension("json");
        let mut store = UserStore::load_database(path.clone(), &legacy).unwrap();
        let first_id = store.active_profile_id().to_owned();
        store.set_window_scale("main", 1.5).unwrap();
        let second_id = store.create("Second").unwrap().id.clone();
        assert_eq!(store.window_scale("main").unwrap(), 1.0);
        store.set_window_scale("main", 2.5).unwrap();
        let mut reopened = UserStore::load_database(path.clone(), &legacy).unwrap();
        assert_eq!(reopened.window_scale("main").unwrap(), 2.5);
        reopened.select(&first_id).unwrap();
        assert_eq!(reopened.window_scale("main").unwrap(), 1.5);
        reopened.select(&second_id).unwrap();
        assert_eq!(reopened.window_scale("main").unwrap(), 2.5);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn malformed_sqlite_window_scales_fall_back_to_default() {
        let path = test_path().with_extension("sqlite3");
        let legacy = path.with_extension("json");
        let store = UserStore::load_database(path.clone(), &legacy).unwrap();
        let profile_id = store.active_profile_id().to_owned();
        let mut db = stamina_data::storage::Database::open(&path).unwrap();
        for (id, value) in [
            ("malformed", "not json"),
            ("large", "3.1"),
            ("small", "0.4"),
        ] {
            db.set_setting(&profile_id, &format!("window_scale:{id}"), value)
                .unwrap();
            assert_eq!(store.window_scale(id).unwrap(), 1.0);
        }
        drop(db);
        drop(store);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn malformed_or_out_of_range_window_scales_use_default_and_are_rejected_on_write() {
        let path = test_path();
        fs::write(&path, br#"{"active_profile_id":"p","profiles":[{"id":"p","name":"P","window_scales":{"bad":"nope","too_small":0.1,"too_large":4.0,"ok":1.5}}]}"#).unwrap();
        let store = UserStore::load_from(&path).unwrap();
        assert_eq!(store.window_scale("bad").unwrap(), 1.0);
        assert_eq!(store.window_scale("too_small").unwrap(), 1.0);
        assert_eq!(store.window_scale("too_large").unwrap(), 1.0);
        assert_eq!(store.window_scale("ok").unwrap(), 1.5);

        let mut store = store;
        for invalid in [0.49, 3.01, f32::NAN, f32::INFINITY] {
            assert_eq!(
                store.set_window_scale("main", invalid).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert_eq!(
            store.set_window_scale(" ", 1.0).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn window_scale_accepts_inclusive_limits_and_failed_scale_or_geometry_save_rolls_back() {
        let path = test_path();
        let mut store = UserStore::load_from(&path).unwrap();
        store.set_window_scale("main", 0.5).unwrap();
        store.set_window_scale("keyboard", 3.0).unwrap();
        let geometry = crate::window_geometry::SavedWindowGeometry {
            rect: crate::window_geometry::LogicalRect::new(20.0, 30.0, 800.0, 600.0),
            pixels_per_point: None,
            maximized: false,
            visible: true,
        };
        store.set_window_geometry("main", Some(geometry)).unwrap();
        let before_memory = serde_json::to_value(&store).unwrap();
        let before_file = fs::read(&path).unwrap();
        let blocker = sibling_path(&path, "tmp");
        fs::create_dir(&blocker).unwrap();

        assert!(store.set_window_scale("main", 1.75).is_err());
        assert_eq!(serde_json::to_value(&store).unwrap(), before_memory);
        assert_eq!(fs::read(&path).unwrap(), before_file);
        assert!(store.set_window_geometry("main", None).is_err());
        assert_eq!(serde_json::to_value(&store).unwrap(), before_memory);
        assert_eq!(fs::read(&path).unwrap(), before_file);

        fs::remove_dir(blocker).unwrap();
        let reloaded = UserStore::load_from(&path).unwrap();
        assert_eq!(reloaded.window_scale("main").unwrap(), 0.5);
        assert_eq!(reloaded.window_scale("keyboard").unwrap(), 3.0);
        assert_eq!(reloaded.window_geometry("main").unwrap(), Some(geometry));
        let _ = fs::remove_file(path);
    }
}
