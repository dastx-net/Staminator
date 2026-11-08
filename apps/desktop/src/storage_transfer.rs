//! Safe file transfer helpers for database backups and standalone lessons.
use std::{
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use crate::localization;
use stamina_data::{
    JsonLesson,
    storage::{Database, Profile},
};
use staminator_core::FormatArgs;

#[derive(Debug)]
pub enum TransferError {
    Io(io::Error),
    Storage(stamina_data::storage::StorageError),
    Json(serde_json::Error),
    InvalidLesson(String),
    InvalidBackup(String),
    InvalidProfile(String),
}

impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(
                f,
                "{}",
                localization::format("transfer.file_error", &FormatArgs::new().insert("error", e))
            ),
            Self::Storage(e) => f.write_str(&localization::storage_error(e)),
            Self::Json(e) => write!(
                f,
                "{}",
                localization::format("transfer.json_error", &FormatArgs::new().insert("error", e))
            ),
            Self::InvalidLesson(e) => write!(
                f,
                "{}",
                localization::format(
                    "transfer.invalid_lesson",
                    &FormatArgs::new().insert("error", e)
                )
            ),
            Self::InvalidBackup(e) => write!(
                f,
                "{}",
                localization::format(
                    "transfer.invalid_backup",
                    &FormatArgs::new().insert("error", e)
                )
            ),
            Self::InvalidProfile(e) => write!(
                f,
                "{}",
                localization::format(
                    "transfer.invalid_profile",
                    &FormatArgs::new().insert("error", e)
                )
            ),
        }
    }
}

fn validate_profile(profile: &Profile) -> Result<(), TransferError> {
    if profile.id.trim().is_empty() {
        return Err(TransferError::InvalidProfile(localization::tr(
            "transfer.profile_id_empty",
        )));
    }
    if profile.name.trim().is_empty() {
        return Err(TransferError::InvalidProfile(localization::tr(
            "transfer.profile_name_empty",
        )));
    }
    if profile.content_language.trim().is_empty() {
        return Err(TransferError::InvalidProfile(localization::tr(
            "transfer.profile_language_empty",
        )));
    }
    for (index, result) in profile.results.iter().enumerate() {
        if result.mode.trim().is_empty()
            || result.layout.trim().is_empty()
            || result.exercise.trim().is_empty()
        {
            return Err(TransferError::InvalidProfile(localization::format(
                "transfer.result_fields_empty",
                &FormatArgs::new().insert("index", index + 1),
            )));
        }
        if !result.cpm.is_finite() || result.cpm < 0.0 {
            return Err(TransferError::InvalidProfile(localization::format(
                "transfer.result_cpm_invalid",
                &FormatArgs::new().insert("index", index + 1),
            )));
        }
        if !result.error_percent.is_finite() || !(0.0..=100.0).contains(&result.error_percent) {
            return Err(TransferError::InvalidProfile(localization::format(
                "transfer.result_error_percent_invalid",
                &FormatArgs::new().insert("index", index + 1),
            )));
        }
    }
    Ok(())
}

/// Import one UTF-8 JSON profile while retaining its ID and complete result history.
pub fn import_profile_json(path: impl AsRef<Path>) -> Result<Profile, TransferError> {
    let bytes = fs::read(path)?;
    let source = std::str::from_utf8(&bytes).map_err(|error| {
        TransferError::InvalidProfile(localization::format(
            "transfer.invalid_utf8",
            &FormatArgs::new().insert("offset", error.valid_up_to()),
        ))
    })?;
    let profile: Profile = serde_json::from_str(source)?;
    validate_profile(&profile)?;
    Ok(profile)
}

/// Export a validated profile and all its result history as UTF-8 JSON without replacement.
pub fn export_profile_json(
    profile: &Profile,
    destination: impl AsRef<Path>,
) -> Result<PathBuf, TransferError> {
    validate_profile(profile)?;
    let bytes = serde_json::to_vec_pretty(profile)?;
    let path = destination.as_ref();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    use std::io::Write;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.write_all(b"\n")) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(path.to_owned())
}
impl Error for TransferError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Storage(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::InvalidLesson(_) | Self::InvalidBackup(_) | Self::InvalidProfile(_) => None,
        }
    }
}
impl From<io::Error> for TransferError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<stamina_data::storage::StorageError> for TransferError {
    fn from(e: stamina_data::storage::StorageError) -> Self {
        Self::Storage(e)
    }
}
impl From<serde_json::Error> for TransferError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// Export a consistent SQLite backup to a path that must not already exist.
pub fn export_database_backup(
    database: &Database,
    destination: impl AsRef<Path>,
) -> Result<PathBuf, TransferError> {
    let path = destination.as_ref();
    let reservation = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    drop(reservation);
    match database.backup_to(path) {
        Ok(()) => Ok(path.to_owned()),
        Err(error) => {
            let _ = fs::remove_file(path);
            Err(error.into())
        }
    }
}

/// Check a backup read-only before offering it for restoration.
/// The database layer verifies SQLite integrity and the supported schema version.
pub fn prepare_database_restore(path: impl AsRef<Path>) -> Result<(), TransferError> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(TransferError::InvalidBackup(localization::tr(
            "transfer.backup_missing",
        )));
    }
    Database::validate_backup(path).map_err(|error| {
        eprintln!("Backup validation: {error}");
        TransferError::InvalidBackup(localization::tr("transfer.backup_invalid"))
    })
}

/// Import one UTF-8 JSON lesson and apply the data crate's lesson validation.
pub fn import_json_lesson(path: impl AsRef<Path>) -> Result<JsonLesson, TransferError> {
    let bytes = fs::read(path)?;
    let source = std::str::from_utf8(&bytes).map_err(|error| {
        TransferError::InvalidLesson(localization::format(
            "transfer.invalid_utf8",
            &FormatArgs::new().insert("offset", error.valid_up_to()),
        ))
    })?;
    let lesson: JsonLesson = serde_json::from_str(source)?;
    stamina_data::validate_json_lesson_typed(&lesson)
        .map_err(|error| TransferError::InvalidLesson(localization::lesson_validation(&error)))?;
    Ok(lesson)
}

/// Export a validated lesson as UTF-8 JSON without replacing an existing file.
pub fn export_json_lesson(
    lesson: &JsonLesson,
    destination: impl AsRef<Path>,
) -> Result<PathBuf, TransferError> {
    stamina_data::validate_json_lesson_typed(lesson)
        .map_err(|error| TransferError::InvalidLesson(localization::lesson_validation(&error)))?;
    let bytes = serde_json::to_vec_pretty(lesson)?;
    let path = destination.as_ref();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    use std::io::Write;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.write_all(b"\n")) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn storage_transfer_error_keeps_storage_source_and_translates() {
        let error = TransferError::from(stamina_data::storage::StorageError::FutureSchema(12));
        localization::set_language("EN");
        assert_eq!(
            error.to_string(),
            "Database schema version 12 is newer than supported."
        );
        assert!(matches!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<stamina_data::storage::StorageError>()),
            Some(stamina_data::storage::StorageError::FutureSchema(12))
        ));
        localization::set_language("RU");
        assert_eq!(
            error.to_string(),
            "Версия базы данных 12 новее поддерживаемой."
        );
    }

    fn dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "staminator-transfer-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn lesson() -> JsonLesson {
        JsonLesson {
            version: 1,
            id: "test_lesson".into(),
            title: "Привет".into(),
            language: "ru".into(),
            description: None,
            text: "текст".into(),
        }
    }
    fn profile() -> Profile {
        Profile {
            id: "profile-7".into(),
            name: "Профиль".into(),
            results: vec![stamina_data::storage::ResultRecord {
                timestamp: 123,
                mode: "normal".into(),
                layout: "ru".into(),
                exercise: "lesson-1".into(),
                correct: 42,
                errors: 3,
                elapsed_ms: 9000,
                cpm: 280.0,
                error_percent: 6.25,
                stop_reason: "manual".into(),
            }],
            external_text: None,
            content_language: "RU".into(),
            interface_language: "RU".into(),
            background: stamina_data::storage::BackgroundImageSettings::default(),
        }
    }

    #[test]
    fn backup_export_validates_and_refuses_overwrite() {
        let root = dir();
        let source = root.join("source.db");
        let backup = root.join("copy.db");
        let db = Database::open(&source).unwrap();
        export_database_backup(&db, &backup).unwrap();
        let before = fs::read(&backup).unwrap();
        assert!(prepare_database_restore(&backup).is_ok());
        assert_eq!(fs::read(&backup).unwrap(), before);
        assert!(export_database_backup(&db, &backup).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn restore_validation_does_not_modify_invalid_sqlite_files() {
        let root = dir();
        let path = root.join("invalid.db");
        fs::write(&path, b"not sqlite").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(prepare_database_restore(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);

        let empty_sqlite = root.join("empty.db");
        fs::write(&empty_sqlite, []).unwrap();
        let before = fs::read(&empty_sqlite).unwrap();
        assert!(prepare_database_restore(&empty_sqlite).is_err());
        assert_eq!(fs::read(&empty_sqlite).unwrap(), before);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn lesson_round_trip_utf8_validation_and_no_overwrite() {
        let root = dir();
        let path = root.join("lesson.json");
        let expected = lesson();
        export_json_lesson(&expected, &path).unwrap();
        assert_eq!(import_json_lesson(&path).unwrap(), expected);
        assert!(export_json_lesson(&lesson(), &path).is_err());
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn invalid_or_non_utf8_lesson_is_rejected() {
        let root = dir();
        let path = root.join("bad.json");
        fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(matches!(
            import_json_lesson(&path),
            Err(TransferError::InvalidLesson(_))
        ));
        let mut bad = lesson();
        bad.text.clear();
        assert!(matches!(
            export_json_lesson(&bad, root.join("out.json")),
            Err(TransferError::InvalidLesson(_))
        ));
        let malformed = root.join("malformed.json");
        fs::write(&malformed, b"{").unwrap();
        assert!(matches!(
            import_json_lesson(&malformed),
            Err(TransferError::Json(_))
        ));
        assert!(matches!(
            import_json_lesson(root.join("missing.json")),
            Err(TransferError::Io(_))
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn profile_round_trip_keeps_id_history_and_refuses_overwrite() {
        let root = dir();
        let path = root.join("profile.json");
        let expected = profile();
        export_profile_json(&expected, &path).unwrap();
        let imported = import_profile_json(&path).unwrap();
        assert_eq!(imported.id, expected.id);
        assert_eq!(imported.results, expected.results);
        assert_eq!(imported, expected);
        assert!(export_profile_json(&expected, &path).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn profile_without_history_round_trips_and_exports_utf8_with_final_newline() {
        let root = dir();
        let mut expected = profile();
        expected.results.clear();
        let path = root.join("empty-history.json");

        assert_eq!(export_profile_json(&expected, &path).unwrap(), path);
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.ends_with(b"\n"));
        assert!(std::str::from_utf8(&bytes).is_ok());
        assert_eq!(import_profile_json(&path).unwrap(), expected);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_profile_values_are_rejected() {
        let root = dir();
        let mut invalid = profile();
        invalid.name = "  ".into();
        assert!(matches!(
            export_profile_json(&invalid, root.join("empty-name.json")),
            Err(TransferError::InvalidProfile(_))
        ));
        let path = root.join("bad-metric.json");
        let mut invalid = profile();
        invalid.results[0].error_percent = 101.0;
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(matches!(
            import_profile_json(&path),
            Err(TransferError::InvalidProfile(_))
        ));

        let malformed_utf8 = root.join("utf8-offset.json");
        fs::write(&malformed_utf8, [b'{', b' ', 0xff]).unwrap();
        assert!(matches!(
            import_profile_json(&malformed_utf8),
            Err(TransferError::InvalidProfile(_))
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn profile_validation_errors_are_localized_in_both_languages() {
        let root = dir();
        let mut cases = Vec::new();
        let mut value = profile();
        value.id.clear();
        cases.push((
            value,
            "Некорректный профиль: В профиле не указан ID",
            "Invalid profile: The profile ID is missing",
        ));
        let mut value = profile();
        value.name.clear();
        cases.push((
            value,
            "Некорректный профиль: В профиле не указано имя",
            "Invalid profile: The profile name is missing",
        ));
        let mut value = profile();
        value.content_language.clear();
        cases.push((
            value,
            "Некорректный профиль: В профиле не указан язык текста",
            "Invalid profile: The profile text language is missing",
        ));
        let mut value = profile();
        value.results[0].mode.clear();
        cases.push((
            value,
            "Некорректный профиль: В результате №1 не заполнены режим, раскладка или упражнение",
            "Invalid profile: Result 1 is missing its mode, layout, or exercise",
        ));
        let mut value = profile();
        value.results[0].cpm = f64::NAN;
        cases.push((
            value,
            "Некорректный профиль: В результате №1 неверная скорость",
            "Invalid profile: Result 1 has an invalid speed",
        ));
        let mut value = profile();
        value.results[0].error_percent = 101.0;
        cases.push((
            value,
            "Некорректный профиль: В результате №1 неверный процент ошибок",
            "Invalid profile: Result 1 has an invalid error percentage",
        ));

        for (index, (invalid, expected_ru, expected_en)) in cases.iter().enumerate() {
            localization::set_language("RU");
            let error =
                export_profile_json(invalid, root.join(format!("ru-{index}.json"))).unwrap_err();
            assert_eq!(error.to_string(), *expected_ru);
            localization::set_language("EN");
            let error =
                export_profile_json(invalid, root.join(format!("en-{index}.json"))).unwrap_err();
            assert_eq!(error.to_string(), *expected_en);
        }

        localization::set_language("RU");
        let utf8 = root.join("bad-utf8.json");
        fs::write(&utf8, [0xff]).unwrap();
        let error = import_profile_json(&utf8).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Некорректный профиль: Файл должен быть в кодировке UTF-8 (байт 0)"
        );
        localization::set_language("EN");
        let error = import_profile_json(&utf8).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Invalid profile: The file must use UTF-8 encoding (byte 0)"
        );
        localization::set_language("RU");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_and_missing_backups_report_localized_reasons() {
        let root = dir();
        let missing = root.join("missing.db");
        let invalid = root.join("invalid.db");
        fs::write(&invalid, b"not sqlite").unwrap();
        for (locale, expected_missing, expected_invalid) in [
            (
                "RU",
                "Некорректная резервная копия базы данных: Файл резервной копии не найден",
                "Некорректная резервная копия базы данных: Файл не является корректной резервной копией базы данных",
            ),
            (
                "EN",
                "Invalid database backup: The backup file was not found",
                "Invalid database backup: The file is not a valid database backup",
            ),
        ] {
            localization::set_language(locale);
            assert_eq!(
                prepare_database_restore(&missing).unwrap_err().to_string(),
                expected_missing
            );
            assert_eq!(
                prepare_database_restore(&invalid).unwrap_err().to_string(),
                expected_invalid
            );
        }
        localization::set_language("RU");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn profile_rejects_each_missing_identity_and_invalid_result_field() {
        let root = dir();
        let mut candidates = Vec::new();
        let mut value = profile();
        value.id = " \t".into();
        candidates.push(value);
        let mut value = profile();
        value.name.clear();
        candidates.push(value);
        let mut value = profile();
        value.content_language = " ".into();
        candidates.push(value);
        for field in 0..3 {
            let mut value = profile();
            match field {
                0 => value.results[0].mode.clear(),
                1 => value.results[0].layout.clear(),
                _ => value.results[0].exercise.clear(),
            }
            candidates.push(value);
        }
        for metric in [f64::NAN, f64::INFINITY, -0.01] {
            let mut value = profile();
            value.results[0].cpm = metric;
            candidates.push(value);
        }
        for metric in [f64::NAN, f64::NEG_INFINITY, -0.01, 100.01] {
            let mut value = profile();
            value.results[0].error_percent = metric;
            candidates.push(value);
        }
        for (index, invalid) in candidates.iter().enumerate() {
            assert!(matches!(
                export_profile_json(invalid, root.join(format!("invalid-{index}.json"))),
                Err(TransferError::InvalidProfile(_))
            ));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn transfer_errors_and_file_system_failures_are_reported() {
        let root = dir();
        let missing = root.join("missing.json");
        assert!(matches!(
            import_profile_json(&missing),
            Err(TransferError::Io(_))
        ));
        assert!(matches!(
            prepare_database_restore(&missing),
            Err(TransferError::InvalidBackup(_))
        ));
        let malformed = root.join("profile.json");
        fs::write(&malformed, b"not json").unwrap();
        assert!(matches!(
            import_profile_json(&malformed),
            Err(TransferError::Json(_))
        ));

        let db = Database::open(root.join("source.db")).unwrap();
        let occupied = root.join("occupied");
        fs::create_dir(&occupied).unwrap();
        assert!(matches!(
            export_database_backup(&db, &occupied),
            Err(TransferError::Io(_))
        ));
        assert!(matches!(
            export_json_lesson(&lesson(), &occupied),
            Err(TransferError::Io(_))
        ));
        assert!(matches!(
            export_profile_json(&profile(), &occupied),
            Err(TransferError::Io(_))
        ));
        assert!(!occupied.join("source.db").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lesson_validation_rejects_unsupported_version_empty_fields_and_bad_language() {
        let root = dir();
        let mut cases = Vec::new();
        let mut value = lesson();
        value.version = 2;
        cases.push(value);
        let mut value = lesson();
        value.id = "  ".into();
        cases.push(value);
        let mut value = lesson();
        value.title.clear();
        cases.push(value);
        let mut value = lesson();
        value.text = "\n ".into();
        cases.push(value);
        let mut value = lesson();
        value.language = "not a tag".into();
        cases.push(value);
        for (index, invalid) in cases.iter().enumerate() {
            assert!(matches!(
                export_json_lesson(invalid, root.join(format!("invalid-{index}.json"))),
                Err(TransferError::InvalidLesson(_))
            ));
        }
        let _ = fs::remove_dir_all(root);
    }
}
