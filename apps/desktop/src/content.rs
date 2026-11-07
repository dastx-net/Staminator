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
    russian_lesson_ids: Vec<String>,
    english_lesson_ids: Vec<String>,
    lesson_roots: Vec<PathBuf>,
    lesson_diagnostics: String,
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
        let executable_lessons = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("lessons")));
        let development_lessons = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lessons");
        let builtin = executable_lessons
            .filter(|p| p.is_dir())
            .unwrap_or(development_lessons);
        let user_root = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
            .map(|p| p.join("Staminator/lessons"));
        Self::load_with_roots(&data_dir, builtin, user_root)
    }

    fn load_with_roots(
        data_dir: &std::path::Path,
        builtin: PathBuf,
        user_root: Option<PathBuf>,
    ) -> Self {
        let mut catalog = Self::load_from(data_dir);
        catalog.lesson_roots = vec![builtin];
        if let Some(root) = user_root {
            catalog.lesson_roots.push(root);
        }
        catalog.refresh_lessons();
        catalog
    }

    fn load_from(data_dir: &std::path::Path) -> Self {
        let mut diagnostics = Vec::new();

        let russian_lessons: Vec<Exercise> = Vec::new();
        let english_lessons: Vec<Exercise> = Vec::new();
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
            russian_lesson_ids: (0..russian_lessons.len())
                .map(|i| format!("legacy.ru.{i}"))
                .collect(),
            english_lesson_ids: (0..english_lessons.len())
                .map(|i| format!("legacy.en.{i}"))
                .collect(),
            lesson_roots: Vec::new(),
            lesson_diagnostics: String::new(),
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
            russian_lesson_ids: vec!["ru.1".into(), "ru.2".into()],
            english_lesson_ids: vec!["en.1".into(), "en.2".into()],
            lesson_roots: Vec::new(),
            lesson_diagnostics: String::new(),
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
            russian_lesson_ids: Vec::new(),
            english_lesson_ids: Vec::new(),
            lesson_roots: Vec::new(),
            lesson_diagnostics: String::new(),
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

    #[cfg(test)]
    pub(crate) fn set_lesson_roots(&mut self, roots: Vec<PathBuf>) {
        self.lesson_roots = roots;
        self.refresh_lessons();
    }

    pub fn refresh_lessons(&mut self) {
        let catalog = stamina_data::load_lesson_catalog(&self.lesson_roots);
        self.russian_lessons.clear();
        self.english_lessons.clear();
        self.russian_lesson_ids.clear();
        self.english_lesson_ids.clear();
        for entry in catalog.lessons {
            let group = self
                .lesson_roots
                .iter()
                .find_map(|root| entry.path.strip_prefix(root).ok())
                .and_then(|path| path.parent())
                .filter(|path| !path.as_os_str().is_empty())
                .map(|path| path.display().to_string());
            let lesson = entry.lesson;
            let language = lesson.language.split('-').next().unwrap_or("");
            let (items, ids) = if language.eq_ignore_ascii_case("ru") {
                (&mut self.russian_lessons, &mut self.russian_lesson_ids)
            } else if language.eq_ignore_ascii_case("en") {
                (&mut self.english_lessons, &mut self.english_lesson_ids)
            } else {
                continue;
            };
            ids.push(lesson.id);
            items.push(Exercise {
                name: group.map_or_else(
                    || lesson.title.clone(),
                    |group| format!("{group} / {}", lesson.title),
                ),
                text: lesson.text,
            });
        }
        self.lesson_diagnostics = catalog
            .diagnostics
            .iter()
            .map(|d| format!("{}: {}", d.path.display(), d.message))
            .collect::<Vec<_>>()
            .join("\n");
    }

    pub fn lesson_id(&self, language: ContentLanguage, index: usize) -> Option<&str> {
        let ids = match language {
            ContentLanguage::Russian => &self.russian_lesson_ids,
            ContentLanguage::English => &self.english_lesson_ids,
        };
        ids.get(index).map(String::as_str)
    }

    pub fn lesson_index(&self, language: ContentLanguage, id: &str) -> Option<usize> {
        let ids = match language {
            ContentLanguage::Russian => &self.russian_lesson_ids,
            ContentLanguage::English => &self.english_lesson_ids,
        };
        ids.iter().position(|value| value == id)
    }

    pub fn lesson_diagnostics(&self) -> &str {
        &self.lesson_diagnostics
    }

    pub fn user_lesson_path(&self) -> Option<&std::path::Path> {
        self.lesson_roots.get(1).map(PathBuf::as_path)
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

#[cfg(test)]
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
    fn startup_loads_built_in_and_user_lessons_without_changing_user_files() {
        let root = temp_file("startup");
        let built_in = root.join("lessons");
        let user = root.join("user-lessons");
        fs::create_dir_all(&built_in).unwrap();
        fs::create_dir_all(&user).unwrap();
        let content =
            r#"{"version":1,"id":"user.one","title":"One","language":"en","text":"hello"}"#;
        let user_file = user.join("one.json");
        fs::write(&user_file, content).unwrap();
        let catalog =
            ContentCatalog::load_with_roots(&root.join("data"), built_in, Some(user.clone()));
        assert_eq!(catalog.lessons(ContentLanguage::English)[0].text, "hello");
        assert_eq!(catalog.user_lesson_path(), Some(user.as_path()));
        assert_eq!(fs::read_to_string(user_file).unwrap(), content);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refresh_filters_languages_and_removes_conflicting_lessons() {
        let root = temp_file("json-catalog");
        let built_in = root.join("built-in");
        let user = root.join("user");
        fs::create_dir_all(&built_in).unwrap();
        fs::create_dir_all(&user).unwrap();
        let json = |id: &str, language: &str| {
            format!(
                r#"{{"version":1,"id":"{id}","title":"{id}","language":"{language}","text":"abc"}}"#
            )
        };
        fs::write(built_in.join("en.json"), json("english", "en-US")).unwrap();
        fs::write(built_in.join("fr.json"), json("french", "fr")).unwrap();
        fs::write(user.join("ru.json"), json("russian", "ru-RU")).unwrap();
        let mut catalog = ContentCatalog::fixture();
        catalog.set_lesson_roots(vec![built_in.clone(), user.clone()]);
        assert_eq!(
            catalog.lesson_id(ContentLanguage::English, 0),
            Some("english")
        );
        assert_eq!(
            catalog.lesson_index(ContentLanguage::Russian, "russian"),
            Some(0)
        );
        assert_eq!(catalog.lessons(ContentLanguage::English).len(), 1);
        fs::write(user.join("duplicate.json"), json("english", "en")).unwrap();
        catalog.refresh_lessons();
        assert!(catalog.lessons(ContentLanguage::English).is_empty());
        assert!(catalog.lesson_diagnostics().contains("duplicate.json"));
        assert_eq!(catalog.lessons(ContentLanguage::Russian).len(), 1);
        fs::remove_dir_all(root).unwrap();
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
        assert!(catalog.lessons(ContentLanguage::Russian).is_empty());
        assert!(catalog.lessons(ContentLanguage::English).is_empty());
        assert_eq!(catalog.phrases(ContentLanguage::English)[0].text, "First. ");
        assert!(catalog.phrases(ContentLanguage::Russian).is_empty());
        assert_eq!(catalog.keys(ContentLanguage::Russian), "abc");
        assert_eq!(catalog.keys(ContentLanguage::English), "xyz");
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
