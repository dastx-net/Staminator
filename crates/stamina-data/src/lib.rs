//! Read-only parsers for Stamina's legacy lesson and phrase files.

#![forbid(unsafe_code)]

use encoding_rs::WINDOWS_1251;
use std::{error::Error, fmt, fs, path::Path};

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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(label: &str) -> std::path::PathBuf {
        static NEXT_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "stamina-data-{label}-{}-{}.tmp",
            std::process::id(),
            NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
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
}
