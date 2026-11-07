//! Read-only lesson and phrase catalog backed by the original Stamina data files.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentLanguage {
    Russian,
    English,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exercise {
    pub name: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct ContentCatalog {
    russian_lessons: Vec<Exercise>,
    english_lessons: Vec<Exercise>,
    russian_phrases: Vec<Exercise>,
    english_phrases: Vec<Exercise>,
    russian_keys: String,
    english_keys: String,
    russian_caps: String,
    english_caps: String,
    diagnostics: String,
}

impl ContentCatalog {
    /// Load catalog data from `src/stamina_setup/Data` and report missing files.
    pub fn load() -> Self {
        let data_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/stamina_setup/Data");
        Self::load_from(&data_dir)
    }

    fn load_from(data_dir: &std::path::Path) -> Self {
        let mut diagnostics = Vec::new();

        let russian_lessons = load_lessons(
            &data_dir.join("lessons.ru"),
            "Russian lessons",
            &mut diagnostics,
        );
        let english_lessons = load_lessons(
            &data_dir.join("lessons.us"),
            "English lessons",
            &mut diagnostics,
        );
        let russian_phrases = load_phrases(
            &data_dir.join("phrases.ru"),
            "Russian phrases",
            &mut diagnostics,
        );
        let english_phrases = load_phrases(
            &data_dir.join("phrases.en"),
            "English phrases",
            &mut diagnostics,
        );
        let layouts_path = data_dir.join("../Layouts.ini");
        let layouts = stamina_data::read_layouts(&layouts_path).unwrap_or_default();
        let caps = read_caps(&layouts_path);
        let russian_keys = layouts
            .iter()
            .find(|layout| layout.code == "00000419")
            .map(|layout| layout.keys.clone())
            .unwrap_or_else(|| "ёйцукенгшщзхъфывапролджэячсмитьбю1234567890".into());
        let english_keys = layouts
            .iter()
            .find(|layout| layout.code == "00000409")
            .map(|layout| layout.keys.clone())
            .unwrap_or_else(|| "qwertyuiopasdfghjklzxcvbnm1234567890".into());
        let russian_caps = caps.get("ru").cloned().unwrap_or_default();
        let english_caps = caps.get("en").cloned().unwrap_or_default();

        Self {
            russian_lessons,
            english_lessons,
            russian_phrases,
            english_phrases,
            russian_keys,
            english_keys,
            russian_caps,
            english_caps,
            diagnostics: diagnostics.join("\n"),
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture() -> Self {
        Self {
            russian_lessons: vec![
                Exercise {
                    name: "RU home row".into(),
                    text: "фыва олдж".into(),
                },
                Exercise {
                    name: "RU words".into(),
                    text: "мир дом".into(),
                },
            ],
            english_lessons: vec![
                Exercise {
                    name: "EN home row".into(),
                    text: "asdf jkl;".into(),
                },
                Exercise {
                    name: "EN words".into(),
                    text: "calm desk".into(),
                },
            ],
            russian_phrases: vec![
                Exercise {
                    name: "Кот спит.".into(),
                    text: "Кот спит. ".into(),
                },
                Exercise {
                    name: "Дом у реки.".into(),
                    text: "Дом у реки. ".into(),
                },
            ],
            english_phrases: vec![
                Exercise {
                    name: "Cats nap.".into(),
                    text: "Cats nap. ".into(),
                },
                Exercise {
                    name: "Birds fly.".into(),
                    text: "Birds fly. ".into(),
                },
            ],
            russian_keys: "фывапролдж".into(),
            english_keys: "asdfjkl;".into(),
            russian_caps: "ФЫВАПРОЛДЖ".into(),
            english_caps: "ASDFJKL".into(),
            diagnostics: String::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture_empty() -> Self {
        Self {
            russian_lessons: Vec::new(),
            english_lessons: Vec::new(),
            russian_phrases: Vec::new(),
            english_phrases: Vec::new(),
            russian_keys: String::new(),
            english_keys: String::new(),
            russian_caps: String::new(),
            english_caps: String::new(),
            diagnostics: "No training content available".into(),
        }
    }

    pub fn lessons(&self, language: ContentLanguage) -> &[Exercise] {
        match language {
            ContentLanguage::Russian => &self.russian_lessons,
            ContentLanguage::English => &self.english_lessons,
        }
    }

    pub fn phrases(&self, language: ContentLanguage) -> &[Exercise] {
        match language {
            ContentLanguage::Russian => &self.russian_phrases,
            ContentLanguage::English => &self.english_phrases,
        }
    }

    pub fn diagnostics(&self) -> &str {
        &self.diagnostics
    }

    #[cfg(test)]
    pub fn keys(&self, language: ContentLanguage) -> &str {
        match language {
            ContentLanguage::Russian => &self.russian_keys,
            ContentLanguage::English => &self.english_keys,
        }
    }

    /// Return printable system-layout keys with symbols listed in the matching
    /// Stamina `Caps` section removed. `english_layout` is the active OS layout,
    /// independent of the selected lesson/phrase language.
    pub fn all_keys(&self, english_layout: bool) -> String {
        let (keys, caps) = if english_layout {
            (&self.english_keys, &self.english_caps)
        } else {
            (&self.russian_keys, &self.russian_caps)
        };
        keys.chars().filter(|key| !caps.contains(*key)).collect()
    }
}

fn read_caps(path: &std::path::Path) -> std::collections::HashMap<String, String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Default::default();
    };
    let source = stamina_data::decode_legacy(&bytes);
    let mut section = String::new();
    let mut result = std::collections::HashMap::new();
    for line in source.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_ascii_lowercase();
        } else if let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("Caps")
            && !section.is_empty()
        {
            result.insert(section.clone(), value.trim().to_owned());
        }
    }
    result
}

fn load_lessons(
    path: &std::path::Path,
    label: &str,
    diagnostics: &mut Vec<String>,
) -> Vec<Exercise> {
    match stamina_data::read_lessons(path) {
        Ok(records) => {
            let exercises: Vec<_> = records
                .into_iter()
                .filter(|record| record.kind == ' ')
                .filter_map(|record| {
                    let text = record.text?;
                    (!text.trim().is_empty()).then_some(Exercise {
                        name: record.name,
                        text,
                    })
                })
                .collect();
            if exercises.is_empty() {
                diagnostics.push(format!(
                    "{label}: no nonempty lessons in {}",
                    path.display()
                ));
                Vec::new()
            } else {
                exercises
            }
        }
        Err(error) => {
            diagnostics.push(format!("{label}: {error}; no content available"));
            Vec::new()
        }
    }
}

fn load_phrases(
    path: &std::path::Path,
    label: &str,
    diagnostics: &mut Vec<String>,
) -> Vec<Exercise> {
    match stamina_data::read_phrases(path) {
        Ok(phrases) => {
            let exercises: Vec<_> = phrases
                .lines
                .into_iter()
                .filter(|line| !line.trim().is_empty())
                .map(|line| Exercise {
                    name: line.clone(),
                    text: phrase_segment(&line),
                })
                .collect();
            if exercises.is_empty() {
                diagnostics.push(format!(
                    "{label}: no nonempty phrases in {}",
                    path.display()
                ));
                Vec::new()
            } else {
                exercises
            }
        }
        Err(error) => {
            diagnostics.push(format!("{label}: {error}; no content available"));
            Vec::new()
        }
    }
}

fn phrase_segment(line: &str) -> String {
    let trimmed = line.trim();
    let end = trimmed
        .find(['.', '?', '!'])
        .map(|index| index + 1)
        .unwrap_or(trimmed.len());
    format!("{} ", trimmed[..end].trim_end())
}

#[cfg(test)]
mod tests {
    use super::{
        ContentCatalog, ContentLanguage, Exercise, load_lessons, load_phrases, phrase_segment,
        read_caps,
    };
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    fn temp_file(label: &str) -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "staminator-content-{label}-{}-{}.tmp",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn takes_first_sentence_and_one_following_space() {
        assert_eq!(phrase_segment("  First. Second!  "), "First. ");
        assert_eq!(phrase_segment("Без знака"), "Без знака ");
    }

    #[test]
    fn catalog_loads_available_data_and_reports_missing_files() {
        let root = temp_file("catalog");
        let data = root.join("Data");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("lessons.ru"), "[#] RU\r\nтест\r\n").unwrap();
        fs::write(data.join("phrases.en"), "First. Next!\r\n").unwrap();
        fs::write(
            root.join("Layouts.ini"),
            "[LAYOUTS]\n00000419=ru,ru,Russian,abc\n00000409=en,us,English,xyz\n",
        )
        .unwrap();

        let catalog = ContentCatalog::load_from(&data);
        assert_eq!(catalog.lessons(ContentLanguage::Russian)[0].text, "тест");
        assert!(catalog.lessons(ContentLanguage::English).is_empty());
        assert_eq!(catalog.phrases(ContentLanguage::English)[0].text, "First. ");
        assert!(catalog.phrases(ContentLanguage::Russian).is_empty());
        assert_eq!(catalog.keys(ContentLanguage::Russian), "abc");
        assert_eq!(catalog.keys(ContentLanguage::English), "xyz");
        assert!(catalog.diagnostics().contains("English lessons"));
        assert!(catalog.diagnostics().contains("Russian phrases"));

        fs::remove_file(data.join("lessons.ru")).unwrap();
        fs::remove_file(data.join("phrases.en")).unwrap();
        fs::remove_file(root.join("Layouts.ini")).unwrap();
        fs::remove_dir(data).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn load_lessons_keeps_only_named_nonempty_lesson_records() {
        let path = temp_file("lessons-filter");
        fs::write(
            &path,
            "[#]>Menu\r\nmenu body\r\n[#] Visible\r\nabc\r\n[#] Empty\r\n \r\n[#]-Separator\r\n[#]No body",
        )
        .unwrap();
        let mut diagnostics = Vec::new();

        let lessons = load_lessons(&path, "test lessons", &mut diagnostics);

        assert_eq!(
            lessons,
            [Exercise {
                name: "Visible".into(),
                text: "abc".into()
            }]
        );
        assert!(diagnostics.is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn load_lessons_returns_empty_and_diagnostic_for_missing_or_empty_file() {
        let missing = temp_file("lessons-missing");
        let empty = temp_file("lessons-empty");
        fs::write(&empty, "[#]>Only a submenu\r\nbody\r\n[#] Blank\r\n \r\n").unwrap();
        let mut missing_diagnostics = Vec::new();
        assert_eq!(
            load_lessons(&missing, "test lessons", &mut missing_diagnostics,),
            []
        );
        assert!(missing_diagnostics[0].contains("test lessons:"));
        assert!(missing_diagnostics[0].contains("no content available"));

        let mut empty_diagnostics = Vec::new();
        assert_eq!(
            load_lessons(&empty, "test lessons", &mut empty_diagnostics,),
            []
        );
        assert!(empty_diagnostics[0].contains("no nonempty lessons"));
        fs::remove_file(empty).unwrap();
    }

    #[test]
    fn load_phrases_filters_blank_lines_and_builds_exercises() {
        let path = temp_file("phrases-filter");
        fs::write(
            &path,
            "  First sentence. Second sentence!  \r\n \r\nNo punctuation\n",
        )
        .unwrap();
        let mut diagnostics = Vec::new();

        let phrases = load_phrases(&path, "test phrases", &mut diagnostics);

        assert_eq!(
            phrases,
            [
                Exercise {
                    name: "  First sentence. Second sentence!  ".into(),
                    text: "First sentence. ".into()
                },
                Exercise {
                    name: "No punctuation".into(),
                    text: "No punctuation ".into()
                },
            ]
        );
        assert!(diagnostics.is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn load_phrases_returns_empty_and_diagnostic_for_missing_or_empty_file() {
        let missing = temp_file("phrases-missing");
        let empty = temp_file("phrases-empty");
        fs::write(&empty, " \r\n\t\n").unwrap();

        let mut missing_diagnostics = Vec::new();
        assert_eq!(
            load_phrases(&missing, "test phrases", &mut missing_diagnostics,),
            []
        );
        assert!(missing_diagnostics[0].contains("no content available"));

        let mut empty_diagnostics = Vec::new();
        assert_eq!(
            load_phrases(&empty, "test phrases", &mut empty_diagnostics,),
            []
        );
        assert!(empty_diagnostics[0].contains("no nonempty phrases"));
        fs::remove_file(empty).unwrap();
    }

    #[test]
    fn all_keys_uses_system_layout_and_removes_caps_symbols() {
        let catalog = ContentCatalog::fixture();
        assert_eq!(catalog.all_keys(true), "asdfjkl;");
        assert_eq!(catalog.all_keys(false), "фывапролдж");
    }

    #[test]
    fn reads_caps_by_layout_extension_and_keeps_empty_when_content_is_absent() {
        let path = temp_file("caps");
        fs::write(
            &path,
            "[LAYOUTS]\n00000409=en,us,English,abcABC\n00000419=ru,ru,Russian,абвАБВ\n[EN]\nCaps=ABC\n[RU]\nCaps=АБВ\n",
        )
        .unwrap();
        let caps = read_caps(&path);
        assert_eq!(caps.get("en").map(String::as_str), Some("ABC"));
        assert_eq!(caps.get("ru").map(String::as_str), Some("АБВ"));
        fs::remove_file(path).unwrap();

        let missing_path = temp_file("caps-absent");
        let mut diagnostics = Vec::new();
        let lessons = load_lessons(&missing_path, "Russian lessons", &mut diagnostics);
        assert!(lessons.is_empty());
        assert!(diagnostics[0].contains("no content available"));
    }
}
