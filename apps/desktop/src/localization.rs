use staminator_core::{Catalog, FormatArgs, Locale};
use std::{cell::RefCell, error::Error, fmt};

thread_local! {
    static CATALOGS: RefCell<(Locale, Vec<Catalog>)> = RefCell::new((Locale::Ru, [Locale::Ru, Locale::En, Locale::Es, Locale::Ar, Locale::Fr, Locale::PtBr, Locale::De, Locale::It, Locale::Tr, Locale::Fa].map(Catalog::embedded).into()));
}

pub fn set_language(language: &str) {
    CATALOGS.with_borrow_mut(|catalogs| {
        catalogs.0 = locale_from_code(language);
    });
}

fn locale_from_code(language: &str) -> Locale {
    match language
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase()
        .as_str()
    {
        "en" => Locale::En,
        "es" => Locale::Es,
        "ar" => Locale::Ar,
        "fr" => Locale::Fr,
        "pt" | "pt-br" => Locale::PtBr,
        "de" => Locale::De,
        "it" => Locale::It,
        "tr" => Locale::Tr,
        "fa" => Locale::Fa,
        _ => Locale::Ru,
    }
}

fn locale_index(locale: Locale) -> usize {
    match locale {
        Locale::Ru => 0,
        Locale::En => 1,
        Locale::Es => 2,
        Locale::Ar => 3,
        Locale::Fr => 4,
        Locale::PtBr => 5,
        Locale::De => 6,
        Locale::It => 7,
        Locale::Tr => 8,
        Locale::Fa => 9,
    }
}

pub fn tr(key: &str) -> String {
    format(key, &FormatArgs::new())
}

pub fn format(key: &str, args: &FormatArgs) -> String {
    CATALOGS.with_borrow_mut(|catalogs| {
        let index = locale_index(catalogs.0);
        let catalog = &mut catalogs.1[index];
        let result = catalog.format(key, args);
        for diagnostic in catalog.take_diagnostics() {
            eprintln!("Localization: {diagnostic:?}");
        }
        visual_text(&result)
    })
}

/// Prepare Arabic-script text for egui's left-to-right, unshaped text painter.
pub fn visual_text(text: &str) -> String {
    if !text
        .chars()
        .any(|ch| matches!(ch as u32, 0x0600..=0x06ff | 0x0750..=0x077f | 0x08a0..=0x08ff))
    {
        return text.to_owned();
    }
    let mut reshaper = arabic_reshaper::ArabicReshaper::new();
    reshaper.configuration.insert("delete_harakat", false);
    let shaped = reshaper.reshape(text);
    shaped
        .split_inclusive(['\n', '\r', '\u{2029}'])
        .map(|line| {
            let separator = line
                .chars()
                .last()
                .filter(|c| matches!(c, '\n' | '\r' | '\u{2029}'));
            let body = separator.map_or(line, |c| line.strip_suffix(c).unwrap_or(line));
            let bidi = unicode_bidi::BidiInfo::new(body, Some(unicode_bidi::Level::rtl()));
            let mut visual = bidi.paragraphs.first().map_or_else(String::new, |p| {
                bidi.reorder_line(p, p.range.clone()).into_owned()
            });
            if let Some(separator) = separator {
                visual.push(separator);
            }
            visual
        })
        .collect()
}

pub fn storage_error(error: &stamina_data::storage::StorageError) -> String {
    use stamina_data::storage::StorageError as E;
    let (key, args) = match error {
        E::FutureSchema(version) => (
            "storage.future_schema",
            FormatArgs::new().insert("version", version),
        ),
        E::NonEmptyImport => ("storage.non_empty_import", FormatArgs::new()),
        E::RevisionConflict { expected, actual } => (
            "storage.revision_conflict",
            FormatArgs::new()
                .insert("expected", expected)
                .insert("actual", actual),
        ),
        E::RestoreConflict => ("storage.restore_conflict", FormatArgs::new()),
        E::Sql(_) | E::Json(_) | E::Io(_) => return error.to_string(),
    };
    format(key, &args)
}

#[derive(Debug)]
pub struct StorageErrorMessage(pub stamina_data::storage::StorageError);

impl fmt::Display for StorageErrorMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&storage_error(&self.0))
    }
}

impl Error for StorageErrorMessage {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

pub fn lesson_validation(error: &stamina_data::LessonValidationError) -> String {
    let mut args = FormatArgs::new();
    if let stamina_data::LessonValidationError::UnsupportedVersion { version } = error {
        args = args.insert("version", version);
    }
    format(error.localization_key(), &args)
}

pub fn lesson_diagnostic(diagnostic: &stamina_data::CatalogDiagnostic) -> String {
    use stamina_data::CatalogDiagnosticCode;
    let args = match &diagnostic.code {
        CatalogDiagnosticCode::Io { reason }
        | CatalogDiagnosticCode::InvalidUtf8 { reason }
        | CatalogDiagnosticCode::InvalidJson { reason } => {
            FormatArgs::new().insert("reason", reason)
        }
        CatalogDiagnosticCode::DuplicateId { id, paths } => {
            FormatArgs::new().insert("id", id).insert(
                "paths",
                paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        }
        CatalogDiagnosticCode::InvalidLesson(error) => {
            return format!(
                "{}: {}",
                diagnostic.path.display(),
                lesson_validation(error)
            );
        }
        _ => FormatArgs::new(),
    };
    format!(
        "{}: {}",
        diagnostic.path.display(),
        format(diagnostic.code.localization_key(), &args)
    )
}

pub fn lesson_write_error(error: &stamina_data::LessonWriteError) -> String {
    use stamina_data::LessonWriteError;
    let args = match error {
        LessonWriteError::Invalid(message) => FormatArgs::new().insert("reason", message),
        LessonWriteError::InvalidLesson(error) => return lesson_validation(error),
        LessonWriteError::AlreadyExists(path) => FormatArgs::new().insert("path", path.display()),
        LessonWriteError::DuplicateId(id) => FormatArgs::new().insert("id", id),
        LessonWriteError::Io { path, source } => FormatArgs::new()
            .insert("path", path.display())
            .insert("reason", source),
        LessonWriteError::UnsafePath => FormatArgs::new(),
    };
    format(error.localization_key(), &args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_select_all_supported_locales_and_default_to_russian() {
        for (code, locale) in [
            ("ru", Locale::Ru),
            ("EN", Locale::En),
            ("es", Locale::Es),
            ("ar", Locale::Ar),
            ("fr", Locale::Fr),
            ("pt-BR", Locale::PtBr),
            ("de", Locale::De),
            ("it", Locale::It),
            ("tr", Locale::Tr),
            ("fa", Locale::Fa),
        ] {
            assert_eq!(locale_from_code(code), locale);
        }
        assert_eq!(locale_from_code("pt_BR"), Locale::PtBr);
        assert_eq!(locale_from_code("pt"), Locale::PtBr);
        assert_eq!(locale_from_code("unknown"), Locale::Ru);
    }

    #[test]
    fn visual_text_shapes_arabic_without_losing_other_lines() {
        assert_eq!(visual_text("English 123"), "English 123");
        let text = visual_text("سلام 123\rEnglish\u{2029}فارسی");
        assert!(text.contains('\r'));
        assert!(text.contains('\u{2029}'));
        assert!(text.contains("123"));
        assert!(text.contains("English"));
        assert!(text.chars().any(|c| ('\u{fb50}'..='\u{fefc}').contains(&c)));
    }

    #[test]
    fn lesson_errors_translate_typed_details_without_parsing_messages() {
        use stamina_data::{
            CatalogDiagnostic, CatalogDiagnosticCode as Code, LessonValidationError as Validation,
            LessonWriteError as Write,
        };
        set_language("EN");
        assert_eq!(
            lesson_validation(&Validation::UnsupportedVersion { version: 42 }),
            "unsupported lesson version 42"
        );
        let cases = [
            (Code::RootIsSymlink, "symbolic link"),
            (Code::RootNotDirectory, "not a directory"),
            (
                Code::Io {
                    reason: "blocked".into(),
                },
                "blocked",
            ),
            (
                Code::InvalidUtf8 {
                    reason: "byte 2".into(),
                },
                "byte 2",
            ),
            (
                Code::InvalidJson {
                    reason: "line 3".into(),
                },
                "line 3",
            ),
            (
                Code::InvalidLesson(Validation::EmptyTitle),
                "title must not be empty",
            ),
            (
                Code::DuplicateId {
                    id: "abc".into(),
                    paths: vec!["one".into(), "two".into()],
                },
                "abc; conflicting paths: one, two",
            ),
        ];
        for (code, expected) in cases {
            let entry = CatalogDiagnostic {
                path: "source.json".into(),
                code,
                message: "unrelated Russian text".into(),
            };
            let rendered = lesson_diagnostic(&entry);
            assert!(rendered.starts_with("source.json:"));
            assert!(rendered.contains(expected), "{rendered}");
        }
        let errors = [
            (Write::Invalid("details".into()), "details"),
            (
                Write::InvalidLesson(Validation::EmptyText),
                "text must not be empty",
            ),
            (Write::UnsafePath, "unsafe path"),
            (Write::AlreadyExists("used.json".into()), "used.json"),
            (Write::DuplicateId("abc".into()), "abc"),
            (
                Write::Io {
                    path: "target".into(),
                    source: std::io::Error::other("blocked"),
                },
                "target: blocked",
            ),
        ];
        for (error, expected) in errors {
            assert!(lesson_write_error(&error).contains(expected));
        }
        set_language("RU");
        assert_eq!(
            lesson_validation(&Validation::EmptyTitle),
            "название не должно быть пустым"
        );
    }

    #[test]
    fn storage_errors_translate_from_variant_data_in_both_locales() {
        use stamina_data::storage::StorageError as E;
        let cases = [
            (
                E::FutureSchema(99),
                "Версия базы данных 99 новее поддерживаемой.",
                "Database schema version 99 is newer than supported.",
            ),
            (
                E::NonEmptyImport,
                "Импорт старого профиля отменён: база данных уже содержит данные.",
                "Legacy profile import was refused because the database already contains data.",
            ),
            (
                E::RevisionConflict {
                    expected: 4,
                    actual: 7,
                },
                "Данные профиля изменились в другом окне (ожидалась версия 4, текущая версия 7).",
                "Profile data changed in another window (expected revision 4, current revision 7).",
            ),
            (
                E::RestoreConflict,
                "База данных изменилась во время подготовки восстановления.",
                "The database changed while restore was being prepared.",
            ),
        ];
        for (error, russian, english) in cases {
            set_language("RU");
            assert_eq!(storage_error(&error), russian);
            set_language("EN");
            assert_eq!(storage_error(&error), english);
        }
        let io = E::Io(std::io::Error::other("disk full"));
        assert!(storage_error(&io).contains("disk full"));
        set_language("RU");
    }

    #[test]
    fn literal_translation_keys_exist_in_both_catalogs() {
        let sources = [
            include_str!("main.rs"),
            include_str!("content.rs"),
            include_str!("content_editor.rs"),
            include_str!("external_text.rs"),
            include_str!("keyboard.rs"),
            include_str!("progress_view.rs"),
            include_str!("result_view.rs"),
            include_str!("storage_transfer.rs"),
            include_str!("user_store.rs"),
        ];
        for source in sources {
            for prefix in ["tr(\"", "localization::format(\""] {
                for part in source.split(prefix).skip(1) {
                    let Some((key, _)) = part.split_once('"') else {
                        continue;
                    };
                    for locale in [Locale::Ru, Locale::En] {
                        let mut catalog = Catalog::embedded(locale);
                        let _ = catalog.format(key, &FormatArgs::new());
                        assert!(
                            !catalog.diagnostics().iter().any(|diagnostic| matches!(
                                diagnostic,
                                staminator_core::LocalizationDiagnostic::MissingKey(_)
                            )),
                            "missing {key} in {locale:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn extra_entries_follow_the_selected_locale_and_interpolate() {
        set_language("EN");
        assert_eq!(
            tr("profile.database_unavailable"),
            "The database is unavailable"
        );
        assert_eq!(
            format(
                "external_text.invalid_utf8",
                &FormatArgs::new().insert("offset", 7)
            ),
            "The file must use UTF-8 encoding (byte 7)"
        );
        set_language("RU");
        assert_eq!(
            tr("external_text.index_past_end"),
            "Индекс находится за концом файла"
        );
    }
}
