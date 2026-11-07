//! Local profile and lesson result persistence.

use serde::{Deserialize, Serialize};
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserStore {
    #[serde(default)]
    profiles: Vec<Profile>,
    #[serde(default)]
    active_profile_id: String,
    #[serde(skip)]
    path: PathBuf,
}

impl UserStore {
    /// Load the store from `%APPDATA%/Staminator/profiles.json` on Windows,
    /// or the platform's conventional config directory elsewhere.
    pub fn load() -> io::Result<Self> {
        Self::load_from(default_path())
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
        };
        store.normalize();
        store
    }

    pub fn save(&self) -> io::Result<()> {
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

    pub fn select(&mut self, id: &str) -> io::Result<()> {
        if !self.profiles.iter().any(|p| p.id == id) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "profile not found"));
        }
        self.active_profile_id = id.to_owned();
        self.save()
    }

    pub fn create(&mut self, name: &str) -> io::Result<&Profile> {
        let name = valid_name(name)?;
        ensure_unique(&self.profiles, &name, None)?;
        let id = fresh_id();
        self.profiles.push(Profile {
            id: id.clone(),
            name,
            results: Vec::new(),
        });
        self.active_profile_id = id;
        self.save()?;
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
        self.profiles[index].name = name;
        self.save()
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
        self.profiles.remove(index);
        if self.active_profile_id == id {
            self.active_profile_id = self.profiles[0].id.clone();
        }
        self.save()
    }

    pub fn record(&mut self, input: ResultInput<'_>) -> io::Result<&ResultRecord> {
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
        });
        if profile.results.len() > MAX_RESULTS_PER_PROFILE {
            profile
                .results
                .drain(..profile.results.len() - MAX_RESULTS_PER_PROFILE);
        }
        self.save()?;
        Ok(self
            .profiles
            .iter()
            .find(|p| p.id == self.active_profile_id)
            .unwrap()
            .results
            .last()
            .unwrap())
    }

    pub fn active_records(&self) -> &[ResultRecord] {
        self.active_profile()
            .map(|p| p.results.as_slice())
            .unwrap_or(&[])
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
            self.profiles[i].results.truncate(MAX_RESULTS_PER_PROFILE);
            i += 1;
        }
        if self.profiles.is_empty() {
            let id = fresh_id();
            self.profiles.push(Profile {
                id: id.clone(),
                name: DEFAULT_PROFILE_NAME.into(),
                results: Vec::new(),
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
}
