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
    diagnostics: String,
}

impl ContentCatalog {
    /// Load catalog data from `src/stamina_setup/Data`, falling back per file if unavailable.
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
            Exercise {
                name: "Начальный урок".into(),
                text: "аа оо уу ыы ее мм нн".into(),
            },
        );
        let english_lessons = load_lessons(
            &data_dir.join("lessons.us"),
            "English lessons",
            &mut diagnostics,
            Exercise {
                name: "Basic lesson".into(),
                text: "asdf jkl; asdf jkl;".into(),
            },
        );
        let russian_phrases = load_phrases(
            &data_dir.join("phrases.ru"),
            "Russian phrases",
            &mut diagnostics,
            "Мама мыла раму. Саша шёл по дороге.",
        );
        let english_phrases = load_phrases(
            &data_dir.join("phrases.en"),
            "English phrases",
            &mut diagnostics,
            "The quick brown fox jumps over the lazy dog.",
        );
        let layouts =
            stamina_data::read_layouts(data_dir.join("../Layouts.ini")).unwrap_or_default();
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

        Self {
            russian_lessons,
            english_lessons,
            russian_phrases,
            english_phrases,
            russian_keys,
            english_keys,
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
            diagnostics: String::new(),
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

    pub fn keys(&self, language: ContentLanguage) -> &str {
        match language {
            ContentLanguage::Russian => &self.russian_keys,
            ContentLanguage::English => &self.english_keys,
        }
    }
}

fn load_lessons(
    path: &std::path::Path,
    label: &str,
    diagnostics: &mut Vec<String>,
    fallback: Exercise,
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
                vec![fallback]
            } else {
                exercises
            }
        }
        Err(error) => {
            diagnostics.push(format!("{label}: {error}; using built-in content"));
            vec![fallback]
        }
    }
}

fn load_phrases(
    path: &std::path::Path,
    label: &str,
    diagnostics: &mut Vec<String>,
    fallback: &str,
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
                vec![phrase_fallback(fallback)]
            } else {
                exercises
            }
        }
        Err(error) => {
            diagnostics.push(format!("{label}: {error}; using built-in content"));
            vec![phrase_fallback(fallback)]
        }
    }
}

fn phrase_fallback(text: &str) -> Exercise {
    Exercise {
        name: text.into(),
        text: phrase_segment(text),
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
    fn catalog_loads_both_languages_and_falls_back_per_missing_file() {
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
        assert_eq!(
            catalog.lessons(ContentLanguage::English)[0].name,
            "Basic lesson"
        );
        assert_eq!(catalog.phrases(ContentLanguage::English)[0].text, "First. ");
        assert!(!catalog.phrases(ContentLanguage::Russian).is_empty());
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

        let lessons = load_lessons(
            &path,
            "test lessons",
            &mut diagnostics,
            Exercise {
                name: "fallback".into(),
                text: "fallback".into(),
            },
        );

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
    fn load_lessons_uses_fallback_and_diagnostic_for_missing_or_empty_file() {
        let missing = temp_file("lessons-missing");
        let empty = temp_file("lessons-empty");
        fs::write(&empty, "[#]>Only a submenu\r\nbody\r\n[#] Blank\r\n \r\n").unwrap();
        let fallback = Exercise {
            name: "fallback lesson".into(),
            text: "xyz".into(),
        };

        let mut missing_diagnostics = Vec::new();
        assert_eq!(
            load_lessons(
                &missing,
                "test lessons",
                &mut missing_diagnostics,
                fallback.clone()
            ),
            std::slice::from_ref(&fallback)
        );
        assert!(missing_diagnostics[0].contains("test lessons:"));
        assert!(missing_diagnostics[0].contains("using built-in content"));

        let mut empty_diagnostics = Vec::new();
        assert_eq!(
            load_lessons(
                &empty,
                "test lessons",
                &mut empty_diagnostics,
                fallback.clone()
            ),
            [fallback]
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

        let phrases = load_phrases(&path, "test phrases", &mut diagnostics, "fallback");

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
    fn load_phrases_uses_fallback_and_diagnostic_for_missing_or_empty_file() {
        let missing = temp_file("phrases-missing");
        let empty = temp_file("phrases-empty");
        fs::write(&empty, " \r\n\t\n").unwrap();

        let mut missing_diagnostics = Vec::new();
        assert_eq!(
            load_phrases(
                &missing,
                "test phrases",
                &mut missing_diagnostics,
                "Backup. Later."
            ),
            [Exercise {
                name: "Backup. Later.".into(),
                text: "Backup. ".into()
            }]
        );
        assert!(missing_diagnostics[0].contains("using built-in content"));

        let mut empty_diagnostics = Vec::new();
        assert_eq!(
            load_phrases(
                &empty,
                "test phrases",
                &mut empty_diagnostics,
                "Backup. Later."
            ),
            [Exercise {
                name: "Backup. Later.".into(),
                text: "Backup. ".into()
            }]
        );
        assert!(empty_diagnostics[0].contains("no nonempty phrases"));
        fs::remove_file(empty).unwrap();
    }
}
