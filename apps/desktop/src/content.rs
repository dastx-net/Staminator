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
    use super::phrase_segment;

    #[test]
    fn takes_first_sentence_and_one_following_space() {
        assert_eq!(phrase_segment("  First. Second!  "), "First. ");
        assert_eq!(phrase_segment("Без знака"), "Без знака ");
    }
}
