//! Local profile and lesson result persistence.

use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_PROFILE_NAME: &str = "Пользователь";
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
}

fn default_content_language() -> String {
    "RU".into()
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
}

impl UserStore {
    pub fn database(&self) -> io::Result<Option<stamina_data::storage::Database>> {
        if self.database {
            stamina_data::storage::Database::open(&self.path)
                .map(Some)
                .map_err(invalid_data)
        } else {
            Ok(None)
        }
    }

    /// Restore a checked backup and retain a copy of the previous database.
    pub fn restore_backup(&mut self, source: &Path) -> io::Result<PathBuf> {
        let mut database = self
            .database()?
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "База данных недоступна"))?;
        let safety_path = self
            .path
            .with_file_name(format!("pre-restore-{}.sqlite3", fresh_id()));
        database
            .restore_from(source, self.revision.get(), &safety_path)
            .map_err(invalid_data)?;
        let loaded = database
            .load_snapshot_with_revision()
            .map_err(invalid_data)?;
        let mut restored: Self =
            serde_json::from_value(serde_json::to_value(loaded.snapshot).map_err(invalid_data)?)
                .map_err(invalid_data)?;
        restored.path = self.path.clone();
        restored.database = true;
        restored.revision.set(loaded.revision);
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
        let mut database = stamina_data::storage::Database::open(&path).map_err(invalid_data)?;
        let backup = sibling_path(legacy, "bak");
        let source = if legacy.exists() {
            Some(legacy)
        } else if backup.exists() {
            Some(backup.as_path())
        } else {
            None
        };
        if let Some(source) = source {
            database
                .import_profiles_json(source)
                .map_err(invalid_data)?;
        }
        let loaded = database
            .load_snapshot_with_revision()
            .map_err(invalid_data)?;
        let before = serde_json::to_value(&loaded.snapshot).map_err(invalid_data)?;
        let mut store: Self =
            serde_json::from_value(serde_json::to_value(loaded.snapshot).map_err(invalid_data)?)
                .map_err(invalid_data)?;
        store.path = path;
        store.database = true;
        store.revision.set(loaded.revision);
        store.normalize();
        if serde_json::to_value(&store).map_err(invalid_data)? != before {
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
            }
        };
        store.path = path;
        store.normalize();
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
                .map_err(|error| match error {
                    stamina_data::storage::StorageError::RevisionConflict { .. } => io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "Данные изменены в другом экземпляре приложения; перезапустите приложение",
                    ),
                    error => invalid_data(error),
                })?;
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
                    "profile path has no parent directory",
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

    pub fn import_profile(&mut self, profile: Profile) -> io::Result<()> {
        if profile.id.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "profile id cannot be empty",
            ));
        }
        if profile.results.len() > MAX_RESULTS_PER_PROFILE
            || !matches!(profile.content_language.as_str(), "RU" | "EN")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid profile contents",
            ));
        }
        valid_name(&profile.name)?;
        ensure_unique(&self.profiles, &profile.name, None)?;
        if self
            .profiles
            .iter()
            .any(|existing| existing.id == profile.id)
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "profile id already exists",
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
            .unwrap_or(DEFAULT_PROFILE_NAME)
    }

    pub fn active_profile(&self) -> Option<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.id == self.active_profile_id)
    }

    pub fn set_content_language(&mut self, language: &str) -> io::Result<()> {
        if !matches!(language, "RU" | "EN") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid content language",
            ));
        }
        let original = self.clone();
        let profile = self
            .profiles
            .iter_mut()
            .find(|p| p.id == self.active_profile_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "profile not found"))?;
        profile.content_language = language.into();
        self.commit_or_restore(original)
    }

    pub fn select(&mut self, id: &str) -> io::Result<()> {
        if !self.profiles.iter().any(|p| p.id == id) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "profile not found"));
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
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "profile not found"))?;
        ensure_unique(&self.profiles, &name, Some(id))?;
        let original = self.clone();
        self.profiles[index].name = name;
        self.commit_or_restore(original)
    }

    pub fn delete(&mut self, id: &str) -> io::Result<()> {
        if self.profiles.len() <= 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot delete the final profile",
            ));
        }
        let index = self
            .profiles
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "profile not found"))?;
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
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "active profile not found"))?;
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
                    io::Error::new(io::ErrorKind::NotFound, "active profile not found")
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
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "active profile not found"))?;
        profile.external_text = Some(progress);
        if let Err(error) = self.save() {
            *self = original;
            return Err(error);
        }
        Ok(())
    }

    fn normalize(&mut self) {
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
            if !matches!(self.profiles[i].content_language.as_str(), "RU" | "EN") {
                self.profiles[i].content_language = default_content_language();
            }
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
                name: DEFAULT_PROFILE_NAME.into(),
                results: Vec::new(),
                external_text: None,
                content_language: default_content_language(),
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
            "profile name cannot be empty",
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
            "profile name already exists",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    type FailureOperation = (&'static str, Box<dyn Fn(&mut UserStore) -> io::Result<()>>);

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
        store
            .database()
            .unwrap()
            .unwrap()
            .backup_to(&backup_path)
            .unwrap();
        store.create("After backup").unwrap();
        let safety = store.restore_backup(&backup_path).unwrap();
        assert_eq!(store.active_name(), "Before backup");
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

        let failures: [FailureOperation; 4] = [
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
        let failures: [FailureOperation; 4] = [
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
}
