//! Read-only parsers for Stamina's legacy lesson and phrase files.

#![forbid(unsafe_code)]

use encoding_rs::WINDOWS_1251;
use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

const MARKER: &str = "[#]";

#[derive(Debug)]
pub enum DataError {
    Io {
        path: String,
        source: std::io::Error,
    },
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "failed to read {path}: {source}"),
        }
    }
}
impl Error for DataError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
        }
    }
}

/// Decode UTF-8 when the entire byte sequence is valid; otherwise decode as Windows-1251.
pub fn decode_legacy(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => WINDOWS_1251.decode(bytes).0.into_owned(),
    }
}

fn read_text(path: &Path) -> Result<String, DataError> {
    let bytes = fs::read(path).map_err(|source| DataError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(decode_legacy(&bytes))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lesson {
    /// Marker type: space (lesson), `>` (submenu), `-` (separator), or `<` (close submenu).
    pub kind: char,
    pub name: String,
    /// Body for space and `>` entries, following the header line.
    pub text: Option<String>,
}

/// Parse marker records following `CLessons::find_next_lesson`'s header/body rules.
pub fn parse_lessons(input: &str) -> Vec<Lesson> {
    let mut lessons = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = input[cursor..].find(MARKER) {
        let after_marker = cursor + relative + MARKER.len();
        let Some(kind) = input[after_marker..].chars().next() else {
            break;
        };
        if !matches!(kind, ' ' | '>' | '-' | '<') {
            cursor = after_marker;
            continue;
        }
        let name_start = after_marker + kind.len_utf8();
        let rest = &input[name_start..];
        let (name, text, next) = match rest.find('\r') {
            None => (rest.trim_end_matches('\n').to_owned(), None, input.len()),
            Some(end) => {
                let name = rest[..end].to_owned();
                let line_ending = if rest[end..].starts_with("\r\n") {
                    2
                } else {
                    1
                };
                let body_start = name_start + end + line_ending;
                if kind == ' ' || kind == '>' {
                    let body_end = input[body_start..]
                        .find(MARKER)
                        .map(|n| body_start + n)
                        .unwrap_or(input.len());
                    let body = input[body_start..body_end]
                        .chars()
                        .filter(|&c| c != '\r' && c != '\n')
                        .collect();
                    (name, Some(body), body_end)
                } else {
                    (name, None, name_start + end + 1)
                }
            }
        };
        lessons.push(Lesson { kind, name, text });
        cursor = next.max(after_marker);
        if cursor >= input.len() {
            break;
        }
    }
    lessons
}

pub fn read_lessons(path: impl AsRef<Path>) -> Result<Vec<Lesson>, DataError> {
    Ok(parse_lessons(&read_text(path.as_ref())?))
}

/// Phrase files are exposed as decoded text lines; no phrase indexing assumptions are made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phrases {
    pub raw: String,
    pub lines: Vec<String>,
}

pub fn parse_phrases(input: &str) -> Phrases {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let lines = normalized.lines().map(str::to_owned).collect();
    Phrases {
        raw: input.to_owned(),
        lines,
    }
}

pub fn read_phrases(path: impl AsRef<Path>) -> Result<Phrases, DataError> {
    Ok(parse_phrases(&read_text(path.as_ref())?))
}

/// A keyboard layout record from the `[LAYOUTS]` section of `Layouts.ini`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub code: String,
    pub phrase_extension: String,
    pub lesson_extension: String,
    pub display_name: String,
    pub keys: String,
}

/// Parse active eight-hex-digit layout entries in `[LAYOUTS]`.
/// Semicolon comments and entries outside that section are ignored.
pub fn parse_layouts(input: &str) -> Vec<Layout> {
    let mut in_layouts = false;
    let mut layouts = Vec::new();
    for line in input.lines() {
        let line = line.trim();
        if line.starts_with(';') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_layouts = line[1..line.len() - 1].eq_ignore_ascii_case("LAYOUTS");
            continue;
        }
        if !in_layouts {
            continue;
        }
        let Some((code, value)) = line.split_once('=') else {
            continue;
        };
        let code = code.trim();
        if code.len() != 8 || !code.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let mut fields = value.splitn(4, ',');
        let (Some(phrase_extension), Some(lesson_extension), Some(display_name), Some(keys)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        layouts.push(Layout {
            code: code.to_owned(),
            phrase_extension: phrase_extension.to_owned(),
            lesson_extension: lesson_extension.to_owned(),
            display_name: display_name.to_owned(),
            keys: keys.to_owned(),
        });
    }
    layouts
}

pub fn read_layouts(path: impl AsRef<Path>) -> Result<Vec<Layout>, DataError> {
    Ok(parse_layouts(&read_text(path.as_ref())?))
}

/// A version 1 JSON lesson stored as one UTF-8 file.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JsonLesson {
    pub version: u32,
    pub id: String,
    pub title: String,
    pub language: String,
    #[serde(default)]
    pub description: Option<String>,
    pub text: String,
}

/// A valid lesson and the file that supplied it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogLesson {
    pub path: PathBuf,
    pub lesson: JsonLesson,
}

/// A per-path parsing, validation, or filesystem diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogDiagnostic {
    pub path: PathBuf,
    pub message: String,
}

/// Lessons from both roots, sorted by ID then path. Duplicate IDs are excluded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LessonCatalog {
    pub lessons: Vec<CatalogLesson>,
    pub diagnostics: Vec<CatalogDiagnostic>,
}

/// Recursively scan built-in and user roots. Symlinks are skipped, so traversal
/// cannot escape either root. Missing roots are allowed; other errors are reported
/// per path and do not stop scanning other directories.
pub fn load_lesson_catalog(roots: &[impl AsRef<Path>]) -> LessonCatalog {
    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    for root in roots {
        let root = root.as_ref();
        match fs::symlink_metadata(root) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                diagnostics.push(CatalogDiagnostic {
                    path: root.to_owned(),
                    message: "корень является ссылкой; обход пропущен".into(),
                })
            }
            Ok(metadata) if metadata.is_dir() => {
                collect_json_files(root, &mut files, &mut diagnostics)
            }
            Ok(_) => diagnostics.push(CatalogDiagnostic {
                path: root.to_owned(),
                message: "корень не является каталогом".into(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => diagnostics.push(CatalogDiagnostic {
                path: root.to_owned(),
                message: error.to_string(),
            }),
        }
    }
    files.sort();

    let mut by_id: BTreeMap<String, Vec<CatalogLesson>> = BTreeMap::new();
    for path in files {
        match read_json_lesson(&path) {
            Ok(lesson) => by_id
                .entry(lesson.id.clone())
                .or_default()
                .push(CatalogLesson { path, lesson }),
            Err(message) => diagnostics.push(CatalogDiagnostic { path, message }),
        }
    }

    let mut lessons = Vec::new();
    for (_, mut copies) in by_id {
        if copies.len() > 1 {
            let paths = copies
                .iter()
                .map(|item| item.path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            for item in copies {
                diagnostics.push(CatalogDiagnostic {
                    path: item.path,
                    message: format!("повторяется ID урока; конфликтующие пути: {paths}"),
                });
            }
        } else if let Some(item) = copies.pop() {
            lessons.push(item);
        }
    }
    lessons.sort_by(|a, b| {
        a.lesson
            .id
            .cmp(&b.lesson.id)
            .then_with(|| a.path.cmp(&b.path))
    });
    diagnostics.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.message.cmp(&b.message)));
    LessonCatalog {
        lessons,
        diagnostics,
    }
}

fn collect_json_files(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<CatalogDiagnostic>,
) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.push(CatalogDiagnostic {
                path: directory.to_owned(),
                message: error.to_string(),
            });
            return;
        }
    };
    let mut entries = entries
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry),
            Err(error) => {
                diagnostics.push(CatalogDiagnostic {
                    path: directory.to_owned(),
                    message: error.to_string(),
                });
                None
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let kind = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata.file_type(),
            Err(error) => {
                diagnostics.push(CatalogDiagnostic {
                    path,
                    message: error.to_string(),
                });
                continue;
            }
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect_json_files(&path, files, diagnostics);
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            files.push(path);
        }
    }
}

fn read_json_lesson(path: &Path) -> Result<JsonLesson, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let source =
        std::str::from_utf8(&bytes).map_err(|error| format!("файл не в UTF-8: {error}"))?;
    let lesson: JsonLesson =
        serde_json::from_str(source).map_err(|error| format!("ошибка JSON: {error}"))?;
    if lesson.version != 1 {
        return Err(format!("неподдерживаемая версия урока {}", lesson.version));
    }
    if lesson.id.trim().is_empty() {
        return Err("ID не должен быть пустым".into());
    }
    if lesson.title.trim().is_empty() {
        return Err("название не должно быть пустым".into());
    }
    if lesson.text.trim().is_empty() {
        return Err("текст не должен быть пустым".into());
    }
    if !valid_language_tag_subset(&lesson.language) {
        return Err("язык должен соответствовать поддерживаемому синтаксису BCP 47: 2–8 латинских букв, затем необязательные части из 1–8 букв или цифр".into());
    }
    Ok(lesson)
}

// Deliberately validate a lexical subset: primary subtag is 2-8 ASCII letters;
// subsequent subtags are 1-8 ASCII alphanumeric characters. This does not check
// registry validity or all BCP 47 extension/private-use rules.
fn valid_language_tag_subset(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let Some(primary) = parts.next() else {
        return false;
    };
    (2..=8).contains(&primary.len())
        && primary.bytes().all(|byte| byte.is_ascii_alphabetic())
        && parts.all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        static NEXT_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "stamina-data-{label}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_json(path: &Path, id: &str, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(r#"{{"version":1,"id":"{id}","title":"Title {id}","language":"ru","text":"{text}"}}"#),
        )
        .unwrap();
    }

    fn temp_file(label: &str) -> std::path::PathBuf {
        static NEXT_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "stamina-data-{label}-{}-{}.tmp",
            std::process::id(),
            NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    #[test]
    fn catalog_rejects_invalid_fields_and_unreadable_roots_without_losing_valid_lessons() {
        let root = temp_dir("invalid-fields");
        write_json(&root.join("valid.json"), "valid", "abc");
        for (name, json) in [
            (
                "id.json",
                r#"{"version":1,"id":" ","title":"ok","language":"ru","text":"abc"}"#,
            ),
            (
                "title.json",
                r#"{"version":1,"id":"t","title":" ","language":"ru","text":"abc"}"#,
            ),
            (
                "language.json",
                r#"{"version":1,"id":"l","title":"ok","language":"!","text":"abc"}"#,
            ),
        ] {
            fs::write(root.join(name), json).unwrap();
        }
        fs::write(root.join("encoding.json"), [0xff]).unwrap();
        let not_directory = root.join("plain-file");
        fs::write(&not_directory, "file").unwrap();
        let catalog =
            load_lesson_catalog(&[root.clone(), not_directory.clone(), root.join("absent")]);
        assert_eq!(catalog.lessons.len(), 1);
        for name in [
            "id.json",
            "title.json",
            "language.json",
            "encoding.json",
            "plain-file",
        ] {
            assert!(
                catalog.diagnostics.iter().any(|d| d.path.ends_with(name)),
                "{name}"
            );
        }
        assert!(!catalog
            .diagnostics
            .iter()
            .any(|d| d.path.ends_with("absent")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_lesson_types_headers_and_bodies() {
        let input = "[#]>Раздел\r\nописание\r\n[#] Урок\r\nАБ\r\nВ\r\n[#]-разделитель\r\n[#]<\r\n";
        assert_eq!(
            parse_lessons(input),
            vec![
                Lesson {
                    kind: '>',
                    name: "Раздел".into(),
                    text: Some("описание".into())
                },
                Lesson {
                    kind: ' ',
                    name: "Урок".into(),
                    text: Some("АБВ".into())
                },
                Lesson {
                    kind: '-',
                    name: "разделитель".into(),
                    text: None
                },
                Lesson {
                    kind: '<',
                    name: "".into(),
                    text: None
                },
            ]
        );
    }

    #[test]
    fn final_header_has_no_body_and_unknown_markers_are_skipped() {
        assert_eq!(
            parse_lessons("[#]xbad\n[#] Последний"),
            vec![Lesson {
                kind: ' ',
                name: "Последний".into(),
                text: None
            }]
        );
    }

    #[test]
    fn decodes_utf8_or_windows_1251() {
        assert_eq!(decode_legacy("Привет".as_bytes()), "Привет");
        assert_eq!(
            decode_legacy(&[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]),
            "Привет"
        );
    }

    #[test]
    fn phrases_preserve_raw_text_and_expose_normalized_lines() {
        assert_eq!(parse_phrases("один\r\nдва\r\n").lines, ["один", "два"]);
        assert_eq!(parse_phrases("a\nb").raw, "a\nb");
    }

    #[test]
    fn parses_active_layouts_and_keeps_commas_in_key_map() {
        let input = "; comment\r\n[OTHER]\r\n00000001=ignored,x,name,keys\r\n[LAYOUTS]\r\n00000409=en,us,English,abc,def\r\n;00000419=ru,ru,Disabled,xyz\r\nnot-a-code=ru,ru,Bad,no\r\n00000002=too,few\r\n[END]\r\n00000003=x,x,Outside,z";
        assert_eq!(
            parse_layouts(input),
            vec![Layout {
                code: "00000409".into(),
                phrase_extension: "en".into(),
                lesson_extension: "us".into(),
                display_name: "English".into(),
                keys: "abc,def".into(),
            }]
        );
    }

    #[test]
    fn read_functions_decode_legacy_bytes_before_parsing() {
        let lessons_path = temp_file("lessons");
        let phrases_path = temp_file("phrases");
        let layouts_path = temp_file("layouts");
        let lesson_name = "Урок";
        let mut lesson_bytes = b"[#] ".to_vec();
        lesson_bytes.extend_from_slice(&WINDOWS_1251.encode(lesson_name).0);
        lesson_bytes.extend_from_slice(b"\r\nABC\r\n");
        fs::write(&lessons_path, lesson_bytes).unwrap();
        fs::write(
            &phrases_path,
            [0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2, b'\r', b'\n'],
        )
        .unwrap();
        fs::write(
            &layouts_path,
            b"[LAYOUTS]\r\n00000409=en,us,English,abc\r\n",
        )
        .unwrap();

        let lessons = read_lessons(&lessons_path).unwrap();
        assert_eq!(lessons[0].name, "Урок");
        assert_eq!(lessons[0].text.as_deref(), Some("ABC"));
        assert_eq!(read_phrases(&phrases_path).unwrap().lines, ["Привет"]);
        assert_eq!(
            read_layouts(&layouts_path).unwrap()[0].display_name,
            "English"
        );

        fs::remove_file(lessons_path).unwrap();
        fs::remove_file(phrases_path).unwrap();
        fs::remove_file(layouts_path).unwrap();
    }

    #[test]
    fn read_error_reports_path_and_preserves_io_source() {
        let path = temp_file("missing");
        let error = read_phrases(&path).unwrap_err();

        assert!(error.to_string().contains(&path.display().to_string()));
        assert!(error
            .source()
            .is_some_and(|source| { source.downcast_ref::<std::io::Error>().is_some() }));
    }

    #[test]
    fn catalog_reads_two_roots_recursively_in_stable_order() {
        let built_in = temp_dir("catalog-built-in");
        let user = temp_dir("catalog-user");
        write_json(&built_in.join("z.json"), "z", "last");
        write_json(&built_in.join("group/a.json"), "a", "first");
        write_json(&user.join("nested/m.json"), "m", "middle");

        let catalog = load_lesson_catalog(&[built_in.clone(), user.clone()]);
        assert_eq!(
            catalog
                .lessons
                .iter()
                .map(|item| item.lesson.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "m", "z"]
        );
        assert!(catalog.diagnostics.is_empty());

        fs::remove_dir_all(built_in).unwrap();
        fs::remove_dir_all(user).unwrap();
    }

    #[test]
    fn catalog_reports_bad_files_and_every_duplicate_path() {
        let root = temp_dir("catalog-diagnostics");
        write_json(&root.join("one.json"), "same", "text");
        write_json(&root.join("sub/two.json"), "same", "text");
        fs::write(root.join("empty.json"), "{\"version\":1,broken").unwrap();
        fs::write(
            root.join("version.json"),
            r#"{"version":2,"id":"x","title":"X","language":"en","text":"x"}"#,
        )
        .unwrap();
        fs::write(
            root.join("blank.json"),
            r#"{"version":1,"id":"b","title":"B","language":"en","text":" "}"#,
        )
        .unwrap();
        fs::write(
            root.join("bad-language.json"),
            r#"{"version":1,"id":"lang","title":"Language","language":"e-","text":"text"}"#,
        )
        .unwrap();

        let catalog = load_lesson_catalog(std::slice::from_ref(&root));
        assert!(catalog.lessons.is_empty());
        assert_eq!(catalog.diagnostics.len(), 6);
        assert_eq!(
            catalog
                .diagnostics
                .iter()
                .filter(|item| item.message.contains("повторяется ID урока"))
                .count(),
            2
        );
        assert!(catalog
            .diagnostics
            .iter()
            .any(|item| item.path.ends_with("version.json")
                && item.message.contains("неподдерживаемая версия урока")));
        assert!(catalog
            .diagnostics
            .iter()
            .any(|item| item.path.ends_with("blank.json")
                && item.message.contains("текст не должен быть пустым")));
        assert!(catalog
            .diagnostics
            .iter()
            .any(|item| item.path.ends_with("bad-language.json")
                && item.message.contains("поддерживаемому синтаксису BCP 47")));

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn catalog_does_not_follow_symlinks_outside_root() {
        use std::os::unix::fs::symlink;
        let root = temp_dir("catalog-symlink-root");
        let outside = temp_dir("catalog-symlink-outside");
        write_json(&outside.join("hidden.json"), "hidden", "secret");
        symlink(&outside, root.join("outside")).unwrap();
        symlink(outside.join("hidden.json"), root.join("linked.json")).unwrap();
        let linked_root = root.with_extension("linked-root");
        symlink(&outside, &linked_root).unwrap();

        let missing_root = root.with_extension("missing");
        let catalog = load_lesson_catalog(&[root.clone(), linked_root.clone(), missing_root]);
        assert!(catalog.lessons.is_empty());
        assert_eq!(catalog.diagnostics.len(), 1);
        assert_eq!(catalog.diagnostics[0].path, linked_root);
        assert!(catalog.diagnostics[0]
            .message
            .contains("корень является ссылкой"));

        fs::remove_dir_all(root).unwrap();
        fs::remove_file(linked_root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
