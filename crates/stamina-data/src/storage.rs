//! SQLite persistence for profiles, lesson results, settings, progress, and lesson documents.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

const SCHEMA_VERSION: i32 = 2;

/// Ordered migrations. Entry `n` upgrades schema version `n` to `n + 1`.
const MIGRATIONS: &[fn(&rusqlite::Transaction<'_>) -> rusqlite::Result<()>] =
    &[migrate_0_to_1, migrate_1_to_2];

fn migrate_0_to_1(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("CREATE TABLE profiles(id TEXT PRIMARY KEY,name TEXT NOT NULL,content_language TEXT NOT NULL,external_text TEXT); CREATE TABLE sessions(id INTEGER PRIMARY KEY,profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,timestamp INTEGER NOT NULL,mode TEXT NOT NULL,layout TEXT NOT NULL,exercise TEXT NOT NULL,correct INTEGER NOT NULL,errors INTEGER NOT NULL,elapsed_ms INTEGER NOT NULL,cpm REAL NOT NULL,error_percent REAL NOT NULL); CREATE INDEX sessions_profile_time ON sessions(profile_id,timestamp); CREATE TABLE settings(profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,key TEXT NOT NULL,value TEXT NOT NULL,PRIMARY KEY(profile_id,key)); CREATE TABLE book_positions(profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,identity TEXT NOT NULL,encoding TEXT NOT NULL,offset INTEGER NOT NULL,PRIMARY KEY(profile_id,identity)); CREATE TABLE lessons(id TEXT PRIMARY KEY,document TEXT NOT NULL,built_in INTEGER NOT NULL); CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);")
}
fn migrate_1_to_2(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("ALTER TABLE sessions ADD COLUMN stop_reason TEXT NOT NULL DEFAULT 'unknown';")
}

#[derive(Debug)]
pub enum StorageError {
    Sql(rusqlite::Error),
    Json(serde_json::Error),
    Io(std::io::Error),
    FutureSchema(i32),
    NonEmptyImport,
    RevisionConflict { expected: u64, actual: u64 },
    RestoreConflict,
}
impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sql(e) => write!(f, "sqlite: {e}"),
            Self::Json(e) => write!(f, "json: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::FutureSchema(v) => write!(
                f,
                "database schema version {v} is newer than supported version {SCHEMA_VERSION}"
            ),
            Self::NonEmptyImport => write!(
                f,
                "legacy import refused because the database already contains data"
            ),
            Self::RevisionConflict { expected, actual } => write!(
                f,
                "snapshot revision conflict: expected {expected}, found {actual}"
            ),
            Self::RestoreConflict => write!(f, "database changed while preparing restore"),
        }
    }
}
impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sql(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::FutureSchema(_)
            | Self::NonEmptyImport
            | Self::RevisionConflict { .. }
            | Self::RestoreConflict => None,
        }
    }
}
impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}
impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

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
    #[serde(default = "default_stop_reason")]
    pub stop_reason: String,
}
fn default_stop_reason() -> String {
    "unknown".into()
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalProgress {
    pub path: PathBuf,
    pub size: u64,
    pub hash: u64,
    pub encoding: String,
    pub position: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub results: Vec<ResultRecord>,
    #[serde(default)]
    pub external_text: Option<ExternalProgress>,
    #[serde(default = "default_language")]
    pub content_language: String,
}
fn default_language() -> String {
    "RU".into()
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub active_profile_id: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotWithRevision {
    pub snapshot: Snapshot,
    pub revision: u64,
}
pub type LessonDocument = crate::JsonLesson;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookPosition {
    pub identity: String,
    pub encoding: String,
    pub offset: u64,
}

pub struct Database {
    connection: Connection,
    path: PathBuf,
}
impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        let c = Connection::open(&path)?;
        let version: i32 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StorageError::FutureSchema(version));
        }
        let mut version = version;
        while version < SCHEMA_VERSION {
            let migration = MIGRATIONS
                .get(version as usize)
                .ok_or_else(|| rusqlite::Error::InvalidQuery)?;
            let tx = c.unchecked_transaction()?;
            migration(&tx)?;
            let next = version + 1;
            tx.pragma_update(None, "user_version", next)?;
            tx.commit()?;
            version = next;
        }
        c.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self {
            connection: c,
            path,
        })
    }
    /// Create a consistent SQLite backup, including uncheckpointed WAL content.
    pub fn backup_to(&self, path: impl AsRef<Path>) -> Result<(), StorageError> {
        let mut destination = Connection::open(path)?;
        let backup = rusqlite::backup::Backup::new(&self.connection, &mut destination)?;
        backup.run_to_completion(32, std::time::Duration::from_millis(10), None)?;
        Ok(())
    }
    /// Validate a backup without migrating or modifying it.
    pub fn validate_backup(path: impl AsRef<Path>) -> Result<(), StorageError> {
        let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let version: i32 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StorageError::FutureSchema(version));
        }
        if version < 1 {
            return Err(StorageError::Sql(rusqlite::Error::InvalidQuery));
        }
        let integrity: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(StorageError::Sql(rusqlite::Error::InvalidQuery));
        }
        let mut foreign_key_check = c.prepare("PRAGMA foreign_key_check")?;
        let mut violations = foreign_key_check.query_map([], |_| Ok(()))?;
        if let Some(violation) = violations.next() {
            violation?;
            return Err(StorageError::Sql(rusqlite::Error::InvalidQuery));
        }
        let required: &[(&str, &[&str])] = &[
            (
                "profiles",
                &["id", "name", "content_language", "external_text"],
            ),
            (
                "sessions",
                &[
                    "id",
                    "profile_id",
                    "timestamp",
                    "mode",
                    "layout",
                    "exercise",
                    "correct",
                    "errors",
                    "elapsed_ms",
                    "cpm",
                    "error_percent",
                ],
            ),
            ("settings", &["profile_id", "key", "value"]),
            (
                "book_positions",
                &["profile_id", "identity", "encoding", "offset"],
            ),
            ("lessons", &["id", "document", "built_in"]),
            ("metadata", &["key", "value"]),
        ];
        for (table, columns) in required {
            let sql = format!("PRAGMA table_info({table})");
            let mut stmt = c.prepare(&sql)?;
            let found = stmt
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            if columns
                .iter()
                .any(|column| !found.iter().any(|actual| actual == column))
            {
                return Err(StorageError::Sql(rusqlite::Error::InvalidQuery));
            }
        }
        if version == SCHEMA_VERSION {
            if !c
                .prepare("SELECT stop_reason FROM sessions LIMIT 0")?
                .column_names()
                .contains(&"stop_reason")
            {
                return Err(StorageError::Sql(rusqlite::Error::InvalidQuery));
            }
            let _ = load_snapshot_conn(&c)?;
            let mut lessons = c.prepare("SELECT document FROM lessons")?;
            let rows = lessons.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                let _: LessonDocument = serde_json::from_str(&row?)?;
            }
        }
        Ok(())
    }
    /// Restore a validated backup after checking the current database revision.
    /// A consistent safety copy of the current database is written first.
    pub fn restore_from(
        &mut self,
        source_path: impl AsRef<Path>,
        expected_revision: u64,
        safety_backup_path: impl AsRef<Path>,
    ) -> Result<u64, StorageError> {
        let source_path = source_path.as_ref();
        let safety_backup_path = safety_backup_path.as_ref();
        Self::validate_backup(source_path)?;
        let source_version: i32 =
            Connection::open_with_flags(source_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?
                .pragma_query_value(None, "user_version", |r| r.get(0))?;
        let staged = if source_version < SCHEMA_VERSION {
            Some(stage_migrated_backup(source_path)?)
        } else {
            None
        };
        let restore_path = staged
            .as_ref()
            .map(|p| p.0.as_path())
            .unwrap_or(source_path);
        let source_abs = std::fs::canonicalize(source_path)?;
        let target_abs = std::fs::canonicalize(&self.path)?;
        if source_abs == target_abs || safety_backup_path.exists() {
            return Err(StorageError::Sql(rusqlite::Error::InvalidQuery));
        }
        let safety_abs = if safety_backup_path.exists() {
            Some(std::fs::canonicalize(safety_backup_path)?)
        } else {
            None
        };
        if safety_abs.as_deref() == Some(source_abs.as_path())
            || safety_abs.as_deref() == Some(target_abs.as_path())
        {
            return Err(StorageError::Sql(rusqlite::Error::InvalidQuery));
        }
        let source =
            Connection::open_with_flags(restore_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let source_revision = snapshot_revision(&source)?;
        let data_version_before: i64 =
            self.connection
                .pragma_query_value(None, "data_version", |r| r.get(0))?;
        // The backup API can wait on a writer transaction when used as its
        // source. Create the consistent safety copy before acquiring the lock.
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(safety_backup_path)?;
        let safety_result = (|| -> Result<(), StorageError> {
            let mut safety = Connection::open(safety_backup_path)?;
            let backup = rusqlite::backup::Backup::new(&self.connection, &mut safety)?;
            backup.run_to_completion(32, std::time::Duration::from_millis(10), None)?;
            drop(backup);
            Ok(())
        })();
        if let Err(error) = safety_result {
            let _ = std::fs::remove_file(safety_backup_path);
            return Err(error);
        }
        // A separate connection holds SQLite's write reservation through the
        // check, safety backup, and transactional restore. Other writers wait.
        let mut locked = Connection::open(&self.path)?;
        locked.pragma_update(None, "foreign_keys", "ON")?;
        let tx = locked.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let data_version_after: i64 =
            self.connection
                .pragma_query_value(None, "data_version", |r| r.get(0))?;
        if data_version_before != data_version_after {
            drop(tx);
            std::fs::remove_file(safety_backup_path)?;
            return Err(StorageError::RestoreConflict);
        }
        let actual_revision = snapshot_revision(&tx)?;
        if actual_revision != expected_revision {
            drop(tx);
            std::fs::remove_file(safety_backup_path)?;
            return Err(StorageError::RevisionConflict {
                expected: expected_revision,
                actual: actual_revision,
            });
        }

        let source_uri = sqlite_readonly_uri(restore_path);
        tx.execute("ATTACH DATABASE ?1 AS restore_source", [source_uri])?;
        tx.execute_batch("DELETE FROM main.sessions; DELETE FROM main.settings; DELETE FROM main.book_positions; DELETE FROM main.lessons; DELETE FROM main.profiles; DELETE FROM main.metadata;
            INSERT INTO main.profiles SELECT * FROM restore_source.profiles;
            INSERT INTO main.sessions SELECT * FROM restore_source.sessions;
            INSERT INTO main.settings SELECT * FROM restore_source.settings;
            INSERT INTO main.book_positions SELECT * FROM restore_source.book_positions;
            INSERT INTO main.lessons SELECT * FROM restore_source.lessons;
            INSERT INTO main.metadata SELECT * FROM restore_source.metadata;")?;
        let restored_revision = actual_revision
            .max(source_revision)
            .checked_add(1)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        set_snapshot_revision(&tx, restored_revision)?;
        tx.commit()?;
        drop(locked);
        Ok(restored_revision)
    }
    pub fn save_snapshot(&mut self, s: &Snapshot) -> Result<(), StorageError> {
        let tx = self.connection.transaction()?;
        let current = snapshot_revision(&tx)?;
        let next = current
            .checked_add(1)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        save_snapshot_tx(&tx, s)?;
        set_snapshot_revision(&tx, next)?;
        tx.commit()?;
        Ok(())
    }
    /// Load the snapshot and its monotonic revision in one read transaction.
    pub fn load_snapshot_with_revision(&self) -> Result<SnapshotWithRevision, StorageError> {
        let tx = self.connection.unchecked_transaction()?;
        let snapshot = load_snapshot_conn(&tx)?;
        let revision = snapshot_revision(&tx)?;
        tx.commit()?;
        Ok(SnapshotWithRevision { snapshot, revision })
    }
    /// Save only if no other connection has changed the snapshot since it was loaded.
    pub fn save_snapshot_if_revision(
        &mut self,
        s: &Snapshot,
        expected: u64,
    ) -> Result<u64, StorageError> {
        let tx = self.connection.transaction()?;
        let actual = snapshot_revision(&tx)?;
        if actual != expected {
            return Err(StorageError::RevisionConflict { expected, actual });
        }
        save_snapshot_tx(&tx, s)?;
        let next = actual.checked_add(1).ok_or(rusqlite::Error::InvalidQuery)?;
        set_snapshot_revision(&tx, next)?;
        tx.commit()?;
        Ok(next)
    }
    pub fn load_snapshot(&self) -> Result<Snapshot, StorageError> {
        load_snapshot_conn(&self.connection)
    }
    pub fn import_profiles_json(&mut self, path: impl AsRef<Path>) -> Result<bool, StorageError> {
        let tx = self.connection.transaction()?;
        let done: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM metadata WHERE key='legacy_imported')",
            [],
            |r| r.get(0),
        )?;
        if done {
            return Ok(false);
        }
        let populated:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM profiles UNION SELECT 1 FROM lessons UNION SELECT 1 FROM settings)",[],|r|r.get(0))?;
        if populated {
            return Err(StorageError::NonEmptyImport);
        }
        let bytes = std::fs::read(path)?;
        let snapshot: Snapshot = serde_json::from_slice(&bytes)?;
        for p in &snapshot.profiles {
            let ext = p
                .external_text
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            tx.execute(
                "INSERT INTO profiles(id,name,content_language,external_text) VALUES(?1,?2,?3,?4)",
                params![p.id, p.name, p.content_language, ext],
            )?;
            for r in &p.results {
                insert_result(&tx, &p.id, r)?;
            }
        }
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES('active_profile_id',?1)",
            [snapshot.active_profile_id],
        )?;
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES('legacy_imported','1')",
            [],
        )?;
        tx.commit()?;
        Ok(true)
    }
    pub fn import_lessons_once(
        &mut self,
        lessons: &[(LessonDocument, bool)],
    ) -> Result<bool, StorageError> {
        let tx = self.connection.transaction()?;
        let done: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM metadata WHERE key='lessons_imported')",
            [],
            |r| r.get(0),
        )?;
        if done {
            return Ok(false);
        }
        for (lesson, built_in) in lessons {
            tx.execute(
                "INSERT OR IGNORE INTO lessons(id,document,built_in) VALUES(?1,?2,?3)",
                params![lesson.id, serde_json::to_string(lesson)?, built_in],
            )?;
        }
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES('lessons_imported','1')",
            [],
        )?;
        tx.commit()?;
        Ok(true)
    }
    pub fn set_app_setting(&mut self, key: &str, value: &str) -> Result<(), StorageError> {
        self.connection.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("setting:{key}"), value])?;
        Ok(())
    }

    pub fn get_app_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key=?1",
                [format!("setting:{key}")],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_setting(
        &mut self,
        profile: &str,
        key: &str,
        value: &str,
    ) -> Result<(), StorageError> {
        self.connection.execute("INSERT INTO settings(profile_id,key,value) VALUES(?1,?2,?3) ON CONFLICT(profile_id,key) DO UPDATE SET value=excluded.value",params![profile,key,value])?;
        Ok(())
    }
    pub fn get_setting(&self, profile: &str, key: &str) -> Result<Option<String>, StorageError> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM settings WHERE profile_id=?1 AND key=?2",
                params![profile, key],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn save_lesson(
        &mut self,
        lesson: &LessonDocument,
        built_in: bool,
    ) -> Result<(), StorageError> {
        self.connection.execute("INSERT INTO lessons(id,document,built_in) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET document=excluded.document,built_in=excluded.built_in",params![lesson.id,serde_json::to_string(lesson)?,built_in])?;
        Ok(())
    }
    /// Insert a user lesson without replacing an existing ID.
    pub fn create_user_lesson(&mut self, lesson: &LessonDocument) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO lessons(id,document,built_in) VALUES(?1,?2,0)",
            params![lesson.id, serde_json::to_string(lesson)?],
        )?;
        Ok(())
    }
    /// Replace an existing user lesson. Built-in lessons are never updated.
    pub fn update_user_lesson(
        &mut self,
        id: &str,
        lesson: &LessonDocument,
    ) -> Result<bool, StorageError> {
        if id != lesson.id {
            return Err(StorageError::Sql(rusqlite::Error::InvalidQuery));
        }
        let changed = self.connection.execute(
            "UPDATE lessons SET document=?1 WHERE id=?2 AND built_in=0",
            params![serde_json::to_string(lesson)?, id],
        )?;
        Ok(changed != 0)
    }
    /// Insert missing built-ins and refresh only rows already marked built-in.
    pub fn sync_builtin_lessons(&mut self, lessons: &[LessonDocument]) -> Result<(), StorageError> {
        let tx = self.connection.transaction()?;
        for lesson in lessons {
            let document = serde_json::to_string(lesson)?;
            tx.execute("INSERT INTO lessons(id,document,built_in) VALUES(?1,?2,1) ON CONFLICT(id) DO UPDATE SET document=excluded.document WHERE lessons.built_in=1", params![lesson.id, document])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn lesson(&self, id: &str) -> Result<Option<(LessonDocument, bool)>, StorageError> {
        let x = self
            .connection
            .query_row(
                "SELECT document,built_in FROM lessons WHERE id=?",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
            )
            .optional()?;
        x.map(|(s, b)| Ok((serde_json::from_str(&s)?, b)))
            .transpose()
    }
    pub fn list_lessons(&self) -> Result<Vec<(LessonDocument, bool)>, StorageError> {
        let mut q = self
            .connection
            .prepare("SELECT document,built_in FROM lessons ORDER BY id")?;
        let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?;
        rows.map(|row| {
            let (s, b) = row?;
            Ok((serde_json::from_str(&s)?, b))
        })
        .collect()
    }
    pub fn delete_lesson(&mut self, id: &str) -> Result<bool, StorageError> {
        Ok(self
            .connection
            .execute("DELETE FROM lessons WHERE id=? AND built_in=0", [id])?
            != 0)
    }
    pub fn set_book_position(
        &mut self,
        profile: &str,
        p: &BookPosition,
    ) -> Result<(), StorageError> {
        self.connection.execute("INSERT INTO book_positions(profile_id,identity,encoding,offset) VALUES(?1,?2,?3,?4) ON CONFLICT(profile_id,identity) DO UPDATE SET encoding=excluded.encoding,offset=excluded.offset",params![profile,p.identity,p.encoding,p.offset])?;
        Ok(())
    }
    pub fn book_position(
        &self,
        profile: &str,
        identity: &str,
    ) -> Result<Option<BookPosition>, StorageError> {
        Ok(self.connection.query_row("SELECT identity,encoding,offset FROM book_positions WHERE profile_id=?1 AND identity=?2",params![profile,identity],|r|Ok(BookPosition{identity:r.get(0)?,encoding:r.get(1)?,offset:r.get(2)?})).optional()?)
    }
}
fn sqlite_readonly_uri(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::with_capacity(normalized.len());
    for byte in normalized.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("file:{encoded}?mode=ro")
}
struct TemporaryDatabase(PathBuf);
impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn stage_migrated_backup(source_path: &Path) -> Result<TemporaryDatabase, StorageError> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temp_path = std::env::temp_dir().join(format!(
        "stamina-restore-stage-{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)?;
    let result = (|| -> Result<(), StorageError> {
        let source =
            Connection::open_with_flags(source_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut staged = Connection::open(&temp_path)?;
        let backup = rusqlite::backup::Backup::new(&source, &mut staged)?;
        backup.run_to_completion(32, std::time::Duration::from_millis(10), None)?;
        drop(backup);
        drop(staged);
        drop(source);
        let migrated = Database::open(&temp_path)?;
        drop(migrated);
        Database::validate_backup(&temp_path)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(TemporaryDatabase(temp_path))
}

fn snapshot_revision(c: &Connection) -> Result<u64, StorageError> {
    let raw: Option<String> = c
        .query_row(
            "SELECT value FROM metadata WHERE key='snapshot_revision'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    match raw {
        None => Ok(0),
        Some(v) => v
            .parse()
            .map_err(|_| StorageError::Sql(rusqlite::Error::InvalidQuery)),
    }
}
fn set_snapshot_revision(
    tx: &rusqlite::Transaction<'_>,
    revision: u64,
) -> Result<(), StorageError> {
    tx.execute("INSERT INTO metadata(key,value) VALUES('snapshot_revision',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [revision.to_string()])?;
    Ok(())
}

fn save_snapshot_tx(tx: &rusqlite::Transaction<'_>, s: &Snapshot) -> Result<(), StorageError> {
    for p in &s.profiles {
        let ext = p
            .external_text
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        tx.execute("INSERT INTO profiles(id,name,content_language,external_text) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET name=excluded.name,content_language=excluded.content_language,external_text=excluded.external_text", params![p.id,p.name,p.content_language,ext])?;
        let old_results = load_results(tx, &p.id)?;
        if p.results.starts_with(&old_results) {
            for r in &p.results[old_results.len()..] {
                insert_result(tx, &p.id, r)?;
            }
        } else {
            tx.execute("DELETE FROM sessions WHERE profile_id=?", [&p.id])?;
            for r in &p.results {
                insert_result(tx, &p.id, r)?;
            }
        }
    }
    let old = {
        let mut q = tx.prepare("SELECT id FROM profiles")?;
        let rows = q.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for id in old {
        if !s.profiles.iter().any(|p| p.id == id) {
            tx.execute("DELETE FROM profiles WHERE id=?", [id])?;
        }
    }
    tx.execute("INSERT INTO metadata(key,value) VALUES('active_profile_id',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&s.active_profile_id])?;
    Ok(())
}

fn load_results(c: &Connection, id: &str) -> Result<Vec<ResultRecord>, StorageError> {
    let mut q = c.prepare("SELECT timestamp,mode,layout,exercise,correct,errors,elapsed_ms,cpm,error_percent,stop_reason FROM sessions WHERE profile_id=? ORDER BY id")?;
    let rows = q.query_map([id], |r| {
        Ok(ResultRecord {
            timestamp: r.get::<_, i64>(0)? as u64,
            mode: r.get(1)?,
            layout: r.get(2)?,
            exercise: r.get(3)?,
            correct: r.get::<_, i64>(4)? as u64,
            errors: r.get::<_, i64>(5)? as u64,
            elapsed_ms: r.get::<_, i64>(6)? as u64,
            cpm: r.get(7)?,
            error_percent: r.get(8)?,
            stop_reason: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn load_snapshot_conn(c: &Connection) -> Result<Snapshot, StorageError> {
    let mut stmt =
        c.prepare("SELECT id,name,content_language,external_text FROM profiles ORDER BY rowid")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut profiles = Vec::new();
    for row in rows {
        let (id, name, content_language, ext) = row?;
        profiles.push(Profile {
            results: load_results(c, &id)?,
            id,
            name,
            content_language,
            external_text: ext.map(|x| serde_json::from_str(&x)).transpose()?,
        });
    }
    let active = c
        .query_row(
            "SELECT value FROM metadata WHERE key='active_profile_id'",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default();
    Ok(Snapshot {
        profiles,
        active_profile_id: active,
    })
}

fn insert_result(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    r: &ResultRecord,
) -> Result<(), rusqlite::Error> {
    tx.execute("INSERT INTO sessions(profile_id,timestamp,mode,layout,exercise,correct,errors,elapsed_ms,cpm,error_percent,stop_reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![id,r.timestamp,r.mode,r.layout,r.exercise,r.correct,r.errors,r.elapsed_ms,r.cpm,r.error_percent,r.stop_reason])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_failure_preserves_the_original_database() {
        let (mut database, path) = db();
        let snapshot = Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "Alice".into(),
                results: vec![],
                external_text: None,
                content_language: "RU".into(),
            }],
            active_profile_id: "p".into(),
        };
        database.save_snapshot(&snapshot).unwrap();
        assert!(database
            .backup_to(path.join("missing").join("backup.db"))
            .is_err());
        assert_eq!(database.load_snapshot().unwrap(), snapshot);
        drop(database);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn restore_checks_revision_and_keeps_safety_backup() {
        let (mut destination, target_path) = db();
        destination
            .save_snapshot(&Snapshot {
                profiles: vec![Profile {
                    id: "current".into(),
                    name: "Current".into(),
                    content_language: "RU".into(),
                    external_text: None,
                    results: vec![],
                }],
                active_profile_id: "current".into(),
            })
            .unwrap();
        let source_path = target_path.with_extension("source.db");
        let (mut source, source_temp) = db();
        source
            .save_snapshot(&Snapshot {
                profiles: vec![Profile {
                    id: "restored".into(),
                    name: "Restored".into(),
                    content_language: "RU".into(),
                    external_text: None,
                    results: vec![],
                }],
                active_profile_id: "restored".into(),
            })
            .unwrap();
        source.backup_to(&source_path).unwrap();
        let safety = target_path.with_extension("safety.db");
        assert!(matches!(
            destination.restore_from(&source_path, 0, &safety),
            Err(StorageError::RevisionConflict {
                expected: 0,
                actual: 1
            })
        ));
        assert_eq!(
            destination.load_snapshot().unwrap().profiles[0].id,
            "current"
        );
        assert_eq!(
            destination.restore_from(&source_path, 1, &safety).unwrap(),
            2
        );
        assert_eq!(
            destination.load_snapshot().unwrap().profiles[0].id,
            "restored"
        );
        assert_eq!(
            Database::open(&safety)
                .unwrap()
                .load_snapshot()
                .unwrap()
                .profiles[0]
                .id,
            "current"
        );
        drop(destination);
        drop(source);
        std::fs::remove_file(target_path).unwrap();
        std::fs::remove_file(source_temp).unwrap();
        std::fs::remove_file(source_path).unwrap();
        std::fs::remove_file(safety).unwrap();
    }

    #[test]
    fn restore_rejects_backup_with_foreign_key_violations_without_touching_destination() {
        let (mut destination, target_path) = db();
        destination
            .save_snapshot(&Snapshot {
                profiles: vec![Profile {
                    id: "keep".into(),
                    name: "Keep".into(),
                    content_language: "RU".into(),
                    external_text: None,
                    results: vec![],
                }],
                active_profile_id: "keep".into(),
            })
            .unwrap();
        let (source, source_temp) = db();
        source
            .connection
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        source.connection.execute("INSERT INTO sessions(profile_id,timestamp,mode,layout,exercise,correct,errors,elapsed_ms,cpm,error_percent,stop_reason) VALUES('missing',1,'m','l','e',1,0,1,1.0,0.0,'unknown')", []).unwrap();
        let source_path = target_path.with_extension("bad-fk.db");
        source.backup_to(&source_path).unwrap();
        let safety = target_path.with_extension("unused-safety.db");
        assert!(matches!(
            destination.restore_from(&source_path, 1, &safety),
            Err(StorageError::Sql(_))
        ));
        assert_eq!(destination.load_snapshot().unwrap().profiles[0].id, "keep");
        assert!(!safety.exists());
        drop(destination);
        drop(source);
        std::fs::remove_file(target_path).unwrap();
        std::fs::remove_file(source_temp).unwrap();
        std::fs::remove_file(source_path).unwrap();
    }

    #[test]
    fn restore_rejects_unsupported_and_malformed_backup_schemas_without_touching_destination() {
        let (mut destination, target_path) = db();
        destination
            .save_snapshot(&Snapshot {
                profiles: vec![Profile {
                    id: "keep".into(),
                    name: "Keep".into(),
                    content_language: "RU".into(),
                    external_text: None,
                    results: vec![],
                }],
                active_profile_id: "keep".into(),
            })
            .unwrap();
        for (label, version, expected_future) in [
            ("v0", 0, false),
            ("future", SCHEMA_VERSION + 1, true),
            ("missing-tables", SCHEMA_VERSION, false),
        ] {
            let source_path = target_path.with_extension(format!("{label}.db"));
            let source = Connection::open(&source_path).unwrap();
            source.pragma_update(None, "user_version", version).unwrap();
            drop(source);
            let safety = target_path.with_extension(format!("{label}-safety.db"));
            let result = destination.restore_from(&source_path, 1, &safety);
            if expected_future {
                assert!(matches!(result, Err(StorageError::FutureSchema(v)) if v == version));
            } else {
                assert!(matches!(result, Err(StorageError::Sql(_))));
            }
            assert_eq!(destination.load_snapshot().unwrap().profiles[0].id, "keep");
            assert!(!safety.exists());
            std::fs::remove_file(source_path).unwrap();
        }
        drop(destination);
        std::fs::remove_file(target_path).unwrap();
    }

    #[test]
    fn restore_rejects_malformed_lesson_json_without_touching_destination() {
        let (mut destination, target_path) = db();
        destination
            .save_snapshot(&Snapshot {
                profiles: vec![Profile {
                    id: "keep".into(),
                    name: "Keep".into(),
                    content_language: "RU".into(),
                    external_text: None,
                    results: vec![],
                }],
                active_profile_id: "keep".into(),
            })
            .unwrap();
        let (source, source_temp) = db();
        source
            .connection
            .execute(
                "INSERT INTO lessons(id,document,built_in) VALUES('bad','not-json',1)",
                [],
            )
            .unwrap();
        let source_path = target_path.with_extension("bad-json.db");
        source.backup_to(&source_path).unwrap();
        let safety = target_path.with_extension("bad-json-safety.db");
        assert!(matches!(
            destination.restore_from(&source_path, 1, &safety),
            Err(StorageError::Json(_))
        ));
        assert_eq!(destination.load_snapshot().unwrap().profiles[0].id, "keep");
        assert!(!safety.exists());
        drop(destination);
        drop(source);
        std::fs::remove_file(target_path).unwrap();
        std::fs::remove_file(source_temp).unwrap();
        std::fs::remove_file(source_path).unwrap();
    }

    #[test]
    fn restore_migrates_v1_copy_without_changing_original_backup() {
        let (mut destination, target_path) = db();
        destination
            .save_snapshot(&Snapshot {
                profiles: vec![Profile {
                    id: "old-current".into(),
                    name: "Current".into(),
                    content_language: "RU".into(),
                    external_text: None,
                    results: vec![],
                }],
                active_profile_id: "old-current".into(),
            })
            .unwrap();
        let source_path = target_path.with_extension("v1-source.db");
        let legacy = Connection::open(&source_path).unwrap();
        let tx = legacy.unchecked_transaction().unwrap();
        migrate_0_to_1(&tx).unwrap();
        tx.pragma_update(None, "user_version", 1).unwrap();
        tx.execute(
            "INSERT INTO profiles(id,name,content_language) VALUES('legacy','Legacy','RU')",
            [],
        )
        .unwrap();
        tx.execute("INSERT INTO sessions(profile_id,timestamp,mode,layout,exercise,correct,errors,elapsed_ms,cpm,error_percent) VALUES('legacy',1,'m','l','e',1,0,1,1.0,0.0)", []).unwrap();
        tx.commit().unwrap();
        drop(legacy);
        let safety = target_path.with_extension("v1-safety.db");
        assert_eq!(
            destination.restore_from(&source_path, 1, &safety).unwrap(),
            2
        );
        let restored = destination.load_snapshot().unwrap();
        assert_eq!(restored.profiles[0].id, "legacy");
        assert_eq!(restored.profiles[0].results[0].stop_reason, "unknown");
        let original =
            Connection::open_with_flags(&source_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let original_version: i32 = original
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(original_version, 1);
        assert!(original
            .prepare("SELECT stop_reason FROM sessions LIMIT 0")
            .is_err());
        drop(original);
        drop(destination);
        std::fs::remove_file(target_path).unwrap();
        std::fs::remove_file(source_path).unwrap();
        std::fs::remove_file(safety).unwrap();
    }

    #[test]
    fn oversized_counters_fail_without_replacing_existing_data() {
        let (mut database, path) = db();
        let mut snapshot = Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "Alice".into(),
                results: vec![],
                external_text: None,
                content_language: "RU".into(),
            }],
            active_profile_id: "p".into(),
        };
        database.save_snapshot(&snapshot).unwrap();
        snapshot.profiles[0].results.push(ResultRecord {
            timestamp: 1,
            mode: "lesson".into(),
            layout: "ru".into(),
            exercise: "one".into(),
            correct: u64::MAX,
            errors: 0,
            elapsed_ms: 100,
            cpm: 1.0,
            error_percent: 0.0,
            stop_reason: "unknown".into(),
        });
        assert!(database.save_snapshot(&snapshot).is_err());
        assert!(database.load_snapshot().unwrap().profiles[0]
            .results
            .is_empty());
        drop(database);
        std::fs::remove_file(path).unwrap();
    }
    fn db() -> (Database, PathBuf) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "stamina-data-{}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        (Database::open(&p).unwrap(), p)
    }
    #[test]
    fn snapshot_round_trip_keeps_profiles_results_and_active_id() {
        let (mut d, p) = db();
        let s = Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "Alice".into(),
                content_language: "RU".into(),
                external_text: None,
                results: vec![ResultRecord {
                    timestamp: 7,
                    mode: "lesson".into(),
                    layout: "ru".into(),
                    exercise: "x".into(),
                    correct: 4,
                    errors: 1,
                    elapsed_ms: 900,
                    cpm: 2.5,
                    error_percent: 20.0,
                    stop_reason: "unknown".into(),
                }],
            }],
            active_profile_id: "p".into(),
        };
        d.save_snapshot(&s).unwrap();
        assert_eq!(d.load_snapshot().unwrap(), s);
        drop(d);
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn revision_prevents_stale_connection_from_overwriting_snapshot() {
        let (mut first, path) = db();
        let mut second = Database::open(&path).unwrap();
        let initial = first.load_snapshot_with_revision().unwrap();
        let update = Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "first".into(),
                content_language: "RU".into(),
                external_text: None,
                results: vec![],
            }],
            active_profile_id: "p".into(),
        };
        assert_eq!(
            first
                .save_snapshot_if_revision(&update, initial.revision)
                .unwrap(),
            1
        );
        let stale = Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "stale".into(),
                content_language: "RU".into(),
                external_text: None,
                results: vec![],
            }],
            active_profile_id: "p".into(),
        };
        assert!(matches!(
            second.save_snapshot_if_revision(&stale, initial.revision),
            Err(StorageError::RevisionConflict {
                expected: 0,
                actual: 1
            })
        ));
        assert_eq!(first.load_snapshot().unwrap().profiles[0].name, "first");
        drop(first);
        drop(second);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn ordinary_snapshot_save_advances_revision() {
        let (mut d, path) = db();
        let before = d.load_snapshot_with_revision().unwrap().revision;
        d.save_snapshot(&Snapshot::default()).unwrap();
        assert_eq!(
            d.load_snapshot_with_revision().unwrap().revision,
            before + 1
        );
        drop(d);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn appending_snapshot_result_preserves_existing_session_row_ids() {
        let (mut d, path) = db();
        let mut snapshot = Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "P".into(),
                content_language: "RU".into(),
                external_text: None,
                results: vec![ResultRecord {
                    timestamp: 1,
                    mode: "m".into(),
                    layout: "l".into(),
                    exercise: "e".into(),
                    correct: 1,
                    errors: 0,
                    elapsed_ms: 1,
                    cpm: 1.0,
                    error_percent: 0.0,
                    stop_reason: "completed".into(),
                }],
            }],
            active_profile_id: "p".into(),
        };
        d.save_snapshot(&snapshot).unwrap();
        let first_id: i64 = d
            .connection
            .query_row("SELECT id FROM sessions", [], |r| r.get(0))
            .unwrap();
        let appended = ResultRecord {
            timestamp: 2,
            stop_reason: "user".into(),
            ..snapshot.profiles[0].results[0].clone()
        };
        snapshot.profiles[0].results.push(appended);
        d.save_snapshot(&snapshot).unwrap();
        let preserved_id: i64 = d
            .connection
            .query_row("SELECT id FROM sessions ORDER BY id LIMIT 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(preserved_id, first_id);
        drop(d);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn version_one_database_migrates_to_v2_with_legacy_stop_reason() {
        let path = std::env::temp_dir().join(format!("stamina-v1-{}.db", std::process::id()));
        let c = Connection::open(&path).unwrap();
        let tx = c.unchecked_transaction().unwrap();
        migrate_0_to_1(&tx).unwrap();
        tx.pragma_update(None, "user_version", 1).unwrap();
        tx.execute(
            "INSERT INTO profiles(id,name,content_language) VALUES('p','P','RU')",
            [],
        )
        .unwrap();
        tx.execute("INSERT INTO sessions(profile_id,timestamp,mode,layout,exercise,correct,errors,elapsed_ms,cpm,error_percent) VALUES('p',1,'m','l','e',1,0,1,1.0,0.0)", []).unwrap();
        tx.commit().unwrap();
        drop(c);
        let d = Database::open(&path).unwrap();
        assert_eq!(
            d.load_snapshot().unwrap().profiles[0].results[0].stop_reason,
            "unknown"
        );
        drop(d);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn version_zero_database_runs_every_registered_migration_in_order() {
        let path = std::env::temp_dir().join(format!("stamina-v0-{}.db", std::process::id()));
        drop(Connection::open(&path).unwrap());
        let d = Database::open(&path).unwrap();
        let version: i32 = d
            .connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let column_exists: bool = d.connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name='stop_reason')", [], |r| r.get(0)).unwrap();
        assert!(column_exists);
        drop(d);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn lesson_atomic_crud_protects_existing_and_builtin_ids() {
        let (mut d, path) = db();
        let lesson = LessonDocument {
            version: 1,
            id: "same".into(),
            title: "User".into(),
            language: "en".into(),
            description: None,
            text: "one".into(),
        };
        d.create_user_lesson(&lesson).unwrap();
        assert!(d.create_user_lesson(&lesson).is_err());
        let replacement = LessonDocument {
            text: "two".into(),
            ..lesson.clone()
        };
        assert!(d.update_user_lesson("same", &replacement).unwrap());
        assert_eq!(d.lesson("same").unwrap().unwrap().0.text, "two");
        let builtin = LessonDocument {
            id: "built".into(),
            ..lesson.clone()
        };
        d.save_lesson(&builtin, true).unwrap();
        let builtin_update = LessonDocument {
            id: "built".into(),
            ..replacement.clone()
        };
        assert!(!d.update_user_lesson("built", &builtin_update).unwrap());
        d.sync_builtin_lessons(&[LessonDocument {
            text: "refreshed".into(),
            ..builtin.clone()
        }])
        .unwrap();
        assert_eq!(d.lesson("built").unwrap().unwrap().0.text, "refreshed");
        d.sync_builtin_lessons(&[LessonDocument {
            text: "attacker".into(),
            ..lesson
        }])
        .unwrap();
        assert_eq!(d.lesson("same").unwrap().unwrap().0.text, "two");
        drop(d);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn snapshot_update_preserves_settings_and_positions_and_deletes_removed_profiles() {
        let (mut d, p) = db();
        let profile = Profile {
            id: "p".into(),
            name: "P".into(),
            content_language: "RU".into(),
            external_text: None,
            results: vec![],
        };
        let s = Snapshot {
            profiles: vec![profile.clone()],
            active_profile_id: "p".into(),
        };
        d.save_snapshot(&s).unwrap();
        d.set_setting("p", "theme", "dark").unwrap();
        let pos = BookPosition {
            identity: "book".into(),
            encoding: "utf-8".into(),
            offset: 12,
        };
        d.set_book_position("p", &pos).unwrap();
        d.save_snapshot(&s).unwrap();
        assert_eq!(
            d.get_setting("p", "theme").unwrap().as_deref(),
            Some("dark")
        );
        assert_eq!(d.book_position("p", "book").unwrap(), Some(pos));
        d.save_snapshot(&Snapshot::default()).unwrap();
        assert_eq!(d.get_setting("p", "theme").unwrap(), None);
        drop(d);
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn lesson_crud_respects_builtin_flag() {
        let (mut d, p) = db();
        let lesson = LessonDocument {
            version: 1,
            id: "l".into(),
            title: "Lesson".into(),
            language: "en".into(),
            description: None,
            text: "abc".into(),
        };
        d.save_lesson(&lesson, false).unwrap();
        assert_eq!(d.lesson("l").unwrap(), Some((lesson.clone(), false)));
        assert_eq!(d.list_lessons().unwrap().len(), 1);
        assert!(d.delete_lesson("l").unwrap());
        drop(d);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn invalid_json_import_rolls_back_and_can_be_retried() {
        let (mut d, db_path) = db();
        let source = db_path.with_extension("json");
        std::fs::write(&source, b"{ invalid").unwrap();
        assert!(matches!(
            d.import_profiles_json(&source),
            Err(StorageError::Json(_))
        ));
        assert!(d.load_snapshot().unwrap().profiles.is_empty());
        std::fs::write(
            &source,
            br#"{"profiles":[{"id":"p","name":"A","results":[]}],"active_profile_id":"p"}"#,
        )
        .unwrap();
        assert!(d.import_profiles_json(&source).unwrap());
        assert!(!d.import_profiles_json(&source).unwrap());
        assert_eq!(d.load_snapshot().unwrap().profiles.len(), 1);
        drop(d);
        std::fs::remove_file(source).unwrap();
        std::fs::remove_file(db_path).unwrap();
    }

    #[test]
    fn future_schema_is_rejected_without_rewriting_database() {
        let (_, path) = db();
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(connection);
        assert!(
            matches!(Database::open(&path), Err(StorageError::FutureSchema(v)) if v == SCHEMA_VERSION + 1)
        );
        let connection = Connection::open(&path).unwrap();
        let version: i32 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION + 1);
        drop(connection);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_snapshot_transaction_leaves_previous_snapshot_intact() {
        let (mut d, path) = db();
        let original = Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "Original".into(),
                content_language: "RU".into(),
                external_text: None,
                results: vec![],
            }],
            active_profile_id: "p".into(),
        };
        d.save_snapshot(&original).unwrap();
        d.connection.execute_batch("CREATE TRIGGER reject_sessions BEFORE INSERT ON sessions BEGIN SELECT RAISE(ABORT, 'forced failure'); END;").unwrap();
        let replacement = Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "Changed".into(),
                content_language: "RU".into(),
                external_text: None,
                results: vec![ResultRecord {
                    timestamp: 1,
                    mode: "m".into(),
                    layout: "l".into(),
                    exercise: "e".into(),
                    correct: 1,
                    errors: 0,
                    elapsed_ms: 10,
                    cpm: 6.0,
                    error_percent: 0.0,
                    stop_reason: "unknown".into(),
                }],
            }],
            active_profile_id: "p".into(),
        };
        assert!(d.save_snapshot(&replacement).is_err());
        assert_eq!(d.load_snapshot().unwrap(), original);
        drop(d);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn profile_import_refuses_existing_data_without_setting_import_marker() {
        let (mut d, db_path) = db();
        let json = db_path.with_extension("json");
        std::fs::write(&json, br#"{"profiles":[],"active_profile_id":""}"#).unwrap();
        let snapshot = Snapshot {
            profiles: vec![Profile {
                id: "existing".into(),
                name: "Existing".into(),
                content_language: "RU".into(),
                external_text: None,
                results: vec![],
            }],
            active_profile_id: "existing".into(),
        };
        d.save_snapshot(&snapshot).unwrap();
        assert!(matches!(
            d.import_profiles_json(&json),
            Err(StorageError::NonEmptyImport)
        ));
        assert_eq!(d.load_snapshot().unwrap(), snapshot);
        drop(d);
        let _ = std::fs::remove_file(json);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn lesson_import_is_idempotent_and_does_not_replace_existing_lessons() {
        let (mut d, path) = db();
        let original = LessonDocument {
            version: 1,
            id: "shared".into(),
            title: "User copy".into(),
            language: "en".into(),
            description: None,
            text: "custom".into(),
        };
        let bundled = LessonDocument {
            version: 1,
            id: "shared".into(),
            title: "Bundled".into(),
            language: "en".into(),
            description: None,
            text: "factory".into(),
        };
        d.save_lesson(&original, false).unwrap();
        assert!(d.import_lessons_once(&[(bundled, true)]).unwrap());
        assert!(!d.import_lessons_once(&[]).unwrap());
        assert_eq!(d.lesson("shared").unwrap(), Some((original, false)));
        let builtin = LessonDocument {
            version: 1,
            id: "builtin".into(),
            title: "Builtin".into(),
            language: "en".into(),
            description: None,
            text: "text".into(),
        };
        d.save_lesson(&builtin, true).unwrap();
        assert!(!d.delete_lesson("builtin").unwrap());
        assert_eq!(d.lesson("missing").unwrap(), None);
        drop(d);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn app_settings_profile_settings_and_positions_round_trip() {
        let (mut d, path) = db();
        assert_eq!(d.get_app_setting("window").unwrap(), None);
        d.set_app_setting("window", "maximized").unwrap();
        d.set_app_setting("window", "normal").unwrap();
        assert_eq!(
            d.get_app_setting("window").unwrap().as_deref(),
            Some("normal")
        );
        assert_eq!(d.get_setting("missing", "theme").unwrap(), None);
        let snapshot = Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "P".into(),
                content_language: "RU".into(),
                external_text: None,
                results: vec![],
            }],
            active_profile_id: "p".into(),
        };
        d.save_snapshot(&snapshot).unwrap();
        assert_eq!(d.get_setting("p", "theme").unwrap(), None);
        d.set_setting("p", "theme", "dark").unwrap();
        d.set_setting("p", "theme", "light").unwrap();
        assert_eq!(
            d.get_setting("p", "theme").unwrap().as_deref(),
            Some("light")
        );
        assert!(d.set_setting("ghost", "theme", "dark").is_err());
        let position = BookPosition {
            identity: "book".into(),
            encoding: "utf-8".into(),
            offset: 3,
        };
        assert_eq!(d.book_position("p", "book").unwrap(), None);
        d.set_book_position("p", &position).unwrap();
        assert_eq!(d.book_position("p", "book").unwrap(), Some(position));
        let updated = BookPosition {
            identity: "book".into(),
            encoding: "utf-8".into(),
            offset: 9,
        };
        d.set_book_position("p", &updated).unwrap();
        assert_eq!(d.book_position("p", "book").unwrap(), Some(updated));
        assert!(d
            .set_book_position(
                "ghost",
                &BookPosition {
                    identity: "x".into(),
                    encoding: "utf-8".into(),
                    offset: 0
                }
            )
            .is_err());
        drop(d);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn backup_restores_snapshot_and_settings() {
        let (mut d, path) = db();
        let backup_path = path.with_extension("backup.db");
        let snapshot = Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "Backup".into(),
                content_language: "RU".into(),
                external_text: None,
                results: vec![],
            }],
            active_profile_id: "p".into(),
        };
        d.save_snapshot(&snapshot).unwrap();
        d.set_setting("p", "theme", "dark").unwrap();
        d.backup_to(&backup_path).unwrap();
        drop(d);
        let restored = Database::open(&backup_path).unwrap();
        assert_eq!(restored.load_snapshot().unwrap(), snapshot);
        assert_eq!(
            restored.get_setting("p", "theme").unwrap().as_deref(),
            Some("dark")
        );
        drop(restored);
        std::fs::remove_file(backup_path).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn malformed_stored_json_is_reported_by_snapshot_and_lesson_reads() {
        let (mut d, path) = db();
        d.save_snapshot(&Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "P".into(),
                content_language: "RU".into(),
                external_text: None,
                results: vec![],
            }],
            active_profile_id: "p".into(),
        })
        .unwrap();
        d.connection
            .execute(
                "UPDATE profiles SET external_text='not-json' WHERE id='p'",
                [],
            )
            .unwrap();
        assert!(matches!(d.load_snapshot(), Err(StorageError::Json(_))));
        d.connection
            .execute(
                "INSERT INTO lessons(id,document,built_in) VALUES('bad','not-json',0)",
                [],
            )
            .unwrap();
        assert!(matches!(d.lesson("bad"), Err(StorageError::Json(_))));
        assert!(matches!(d.list_lessons(), Err(StorageError::Json(_))));
        drop(d);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn interrupted_schema_migration_rolls_back_and_reports_sql_error() {
        let path = std::env::temp_dir().join(format!(
            "stamina-migration-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE profiles(id TEXT);")
            .unwrap();
        drop(connection);
        assert!(matches!(Database::open(&path), Err(StorageError::Sql(_))));
        let connection = Connection::open(&path).unwrap();
        let version: i32 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 0);
        let sessions_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!sessions_exists);
        drop(connection);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn io_and_database_failures_keep_their_error_sources() {
        let missing =
            std::env::temp_dir().join(format!("missing-stamina-db-{}", std::process::id()));
        assert!(matches!(
            Database::open(missing.join("child.db")),
            Err(StorageError::Sql(_))
        ));
        let (d, path) = db();
        let invalid_backup = path.with_extension("backup-dir");
        std::fs::create_dir_all(&invalid_backup).unwrap();
        assert!(matches!(
            d.backup_to(&invalid_backup),
            Err(StorageError::Sql(_))
        ));
        let source = path.with_extension("missing.json");
        let mut d = d;
        assert!(matches!(
            d.import_profiles_json(&source),
            Err(StorageError::Io(_))
        ));
        drop(d);
        std::fs::remove_dir(invalid_backup).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn storage_errors_expose_sources_only_for_wrapped_errors() {
        let sql = StorageError::Sql(rusqlite::Error::InvalidQuery);
        assert!(sql.to_string().starts_with("sqlite: "));
        assert!(sql.source().is_some());

        let json = StorageError::Json(serde_json::from_str::<()>("{").unwrap_err());
        assert!(json.to_string().starts_with("json: "));
        assert!(json.source().is_some());

        let io = StorageError::Io(std::io::Error::other("disk unavailable"));
        assert!(io.to_string().starts_with("io: "));
        assert!(io.source().is_some());

        let future = StorageError::FutureSchema(SCHEMA_VERSION + 2);
        assert!(future.to_string().contains("newer than supported"));
        assert!(future.source().is_none());

        let non_empty = StorageError::NonEmptyImport;
        assert!(non_empty.to_string().contains("already contains data"));
        assert!(non_empty.source().is_none());
    }

    #[test]
    fn legacy_import_restores_external_progress_and_results() {
        let (mut database, db_path) = db();
        let source = db_path.with_extension("json");
        let external_text = ExternalProgress {
            path: PathBuf::from("practice.txt"),
            size: 42,
            hash: 17,
            encoding: "windows-1251".into(),
            position: 9,
        };
        let result = ResultRecord {
            timestamp: 12,
            mode: "lesson".into(),
            layout: "ru".into(),
            exercise: "home row".into(),
            correct: 30,
            errors: 2,
            elapsed_ms: 5000,
            cpm: 360.0,
            error_percent: 6.25,
            stop_reason: "unknown".into(),
        };
        let imported = Snapshot {
            profiles: vec![Profile {
                id: "p".into(),
                name: "Imported".into(),
                results: vec![result],
                external_text: Some(external_text),
                content_language: "RU".into(),
            }],
            active_profile_id: "p".into(),
        };
        std::fs::write(&source, serde_json::to_vec(&imported).unwrap()).unwrap();

        assert!(database.import_profiles_json(&source).unwrap());
        assert_eq!(database.load_snapshot().unwrap(), imported);
        assert!(!database.import_profiles_json(&source).unwrap());

        drop(database);
        std::fs::remove_file(source).unwrap();
        std::fs::remove_file(db_path).unwrap();
    }

    #[test]
    fn legacy_import_refuses_existing_lessons_or_settings() {
        for existing_data in ["lesson", "setting"] {
            let (mut database, db_path) = db();
            let source = db_path.with_extension("json");
            std::fs::write(&source, br#"{"profiles":[],"active_profile_id":""}"#).unwrap();
            match existing_data {
                "lesson" => database
                    .save_lesson(
                        &LessonDocument {
                            version: 1,
                            id: "custom".into(),
                            title: "Custom".into(),
                            language: "en".into(),
                            description: None,
                            text: "abc".into(),
                        },
                        false,
                    )
                    .unwrap(),
                "setting" => {
                    database
                        .save_snapshot(&Snapshot {
                            profiles: vec![Profile {
                                id: "p".into(),
                                name: "Existing".into(),
                                results: vec![],
                                external_text: None,
                                content_language: "RU".into(),
                            }],
                            active_profile_id: "p".into(),
                        })
                        .unwrap();
                    database.set_setting("p", "theme", "dark").unwrap();
                }
                _ => unreachable!(),
            }

            assert!(matches!(
                database.import_profiles_json(&source),
                Err(StorageError::NonEmptyImport)
            ));
            assert_eq!(
                database.load_snapshot().unwrap().profiles.is_empty(),
                existing_data == "lesson"
            );
            assert_eq!(
                database.get_setting("p", "theme").unwrap().as_deref(),
                (existing_data == "setting").then_some("dark")
            );

            drop(database);
            std::fs::remove_file(source).unwrap();
            std::fs::remove_file(db_path).unwrap();
        }
    }

    #[test]
    fn snapshot_json_defaults_older_missing_fields() {
        let snapshot: Snapshot =
            serde_json::from_str(r#"{"profiles":[{"id":"p","name":"Older profile"}]}"#).unwrap();
        assert_eq!(snapshot.active_profile_id, "");
        assert_eq!(snapshot.profiles[0].content_language, "RU");
        assert!(snapshot.profiles[0].results.is_empty());
        assert!(snapshot.profiles[0].external_text.is_none());
    }
}
