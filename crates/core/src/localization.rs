//! Small JSON-backed localization catalog for desktop UI strings.
use serde_json::Value;
use std::collections::BTreeMap;

const RU: &str = include_str!("../resources/ru.json");
const EN: &str = include_str!("../resources/en.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    Ru,
    En,
}

#[derive(Debug, Clone, Default)]
pub struct FormatArgs {
    values: BTreeMap<String, String>,
    pub count: Option<i64>,
}
impl FormatArgs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(mut self, name: impl Into<String>, value: impl ToString) -> Self {
        self.values.insert(name.into(), value.to_string());
        self
    }
    pub fn count(mut self, count: i64) -> Self {
        self.count = Some(count);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalizationDiagnostic {
    MissingKey(String),
    MissingArgument { key: String, argument: String },
    MissingPluralForm { key: String, category: String },
    InvalidValue { key: String },
}
#[derive(Debug)]
pub enum LocalizationError {
    InvalidJson(String),
    InvalidRoot,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    selected: Value,
    fallback: Value,
    diagnostics: Vec<LocalizationDiagnostic>,
}
impl Catalog {
    pub fn embedded(locale: Locale) -> Self {
        Self::from_resources(
            match locale {
                Locale::Ru => RU,
                Locale::En => EN,
            },
            RU,
        )
        .expect("embedded localization resources are valid")
    }
    pub fn from_resources(selected: &str, fallback: &str) -> Result<Self, LocalizationError> {
        let selected = parse(selected)?;
        let fallback = parse(fallback)?;
        Ok(Self {
            selected,
            fallback,
            diagnostics: Vec::new(),
        })
    }
    pub fn format(&mut self, key: &str, args: &FormatArgs) -> String {
        let (value, locale) = if let Some(value) = self.selected.get(key) {
            (Some(value), &self.selected)
        } else {
            self.diagnostics
                .push(LocalizationDiagnostic::MissingKey(key.into()));
            (self.fallback.get(key), &self.fallback)
        };
        let Some(value) = value else {
            return format!("⟦{key}⟧");
        };
        let value = if let Some(forms) = value.as_object() {
            let count = args.count.unwrap_or(0);
            let category = plural_category(locale, count);
            if !forms.keys().any(|k| {
                matches!(
                    k.as_str(),
                    "zero" | "one" | "two" | "few" | "many" | "other"
                )
            }) {
                self.diagnostics
                    .push(LocalizationDiagnostic::InvalidValue { key: key.into() });
                return format!("⟦{key}⟧");
            }
            match forms.get(category).or_else(|| forms.get("other")) {
                Some(v) => Some(v),
                None => {
                    self.diagnostics
                        .push(LocalizationDiagnostic::MissingPluralForm {
                            key: key.into(),
                            category: category.into(),
                        });
                    return format!("⟦{key}⟧");
                }
            }
        } else {
            Some(value)
        };
        let Some(Value::String(template)) = value else {
            self.diagnostics
                .push(LocalizationDiagnostic::InvalidValue { key: key.into() });
            return format!("⟦{key}⟧");
        };
        interpolate(template, key, args, &mut self.diagnostics)
    }
    pub fn diagnostics(&self) -> &[LocalizationDiagnostic] {
        &self.diagnostics
    }
    pub fn take_diagnostics(&mut self) -> Vec<LocalizationDiagnostic> {
        std::mem::take(&mut self.diagnostics)
    }
}
fn parse(s: &str) -> Result<Value, LocalizationError> {
    let value: Value =
        serde_json::from_str(s).map_err(|e| LocalizationError::InvalidJson(e.to_string()))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(LocalizationError::InvalidRoot)
    }
}
fn interpolate(
    template: &str,
    key: &str,
    args: &FormatArgs,
    diagnostics: &mut Vec<LocalizationDiagnostic>,
) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('}') {
            let name = after[..end].trim();
            match if name == "count" {
                args.count.map(|n| n.to_string())
            } else {
                args.values.get(name).cloned()
            } {
                Some(v) => out.push_str(&v),
                None => {
                    diagnostics.push(LocalizationDiagnostic::MissingArgument {
                        key: key.into(),
                        argument: name.into(),
                    });
                    out.push_str(&format!("{{{name}}}"));
                }
            }
            rest = &after[end + 1..];
        } else {
            out.push_str(&rest[start..]);
            return out;
        }
    }
    out.push_str(rest);
    out
}
fn plural_category(locale: &Value, n: i64) -> &'static str {
    let language = locale
        .get("_locale")
        .and_then(Value::as_str)
        .unwrap_or("ru");
    if language.starts_with("ru") {
        let magnitude = n.unsigned_abs();
        let m10 = magnitude % 10;
        let m100 = magnitude % 100;
        if m10 == 1 && m100 != 11 {
            "one"
        } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
            "few"
        } else {
            "many"
        }
    } else if n == 1 {
        "one"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_catalogs_have_matching_keys_and_placeholders() {
        fn placeholders(value: &Value) -> std::collections::BTreeSet<String> {
            let mut result = std::collections::BTreeSet::new();
            let strings: Vec<&str> = match value {
                Value::String(s) => vec![s],
                Value::Object(forms) => forms
                    .values()
                    .map(|v| v.as_str().expect("plural form must be text"))
                    .collect(),
                _ => panic!("translation must be text or plural forms"),
            };
            for text in strings {
                let mut rest = text;
                while let Some(start) = rest.find('{') {
                    rest = &rest[start + 1..];
                    let end = rest.find('}').expect("unclosed placeholder");
                    result.insert(rest[..end].to_owned());
                    rest = &rest[end + 1..];
                }
            }
            result
        }
        let ru = parse(RU).unwrap();
        let en = parse(EN).unwrap();
        let ru = ru.as_object().unwrap();
        let en = en.as_object().unwrap();
        assert_eq!(
            ru.keys().collect::<Vec<_>>(),
            en.keys().collect::<Vec<_>>(),
            "translation keys must match"
        );
        for (key, value) in ru.iter().filter(|(key, _)| *key != "_locale") {
            assert_eq!(
                placeholders(value),
                placeholders(&en[key]),
                "placeholders differ for {key}"
            );
        }
    }
    #[test]
    fn long_unicode_translation_and_arguments_are_preserved() {
        let text = "Длинная строка 🦀 ".repeat(200);
        let resource =
            serde_json::json!({"_locale":"ru", "long":format!("{text}{{name}}")}).to_string();
        let mut catalog = Catalog::from_resources(&resource, &resource).unwrap();
        assert_eq!(
            catalog.format("long", &FormatArgs::new().insert("name", "Кириллица")),
            format!("{text}Кириллица")
        );
        assert!(catalog.diagnostics().is_empty());
    }
    #[test]
    fn missing_key_is_reported_and_safe() {
        let mut c = Catalog::embedded(Locale::En);
        assert_eq!(c.format("absent", &FormatArgs::new()), "⟦absent⟧");
        assert_eq!(
            c.diagnostics(),
            &[LocalizationDiagnostic::MissingKey("absent".into())]
        );
    }
    #[test]
    fn malformed_resource_is_rejected() {
        assert!(matches!(
            Catalog::from_resources("{bad", "{}"),
            Err(LocalizationError::InvalidJson(_))
        ));
    }
    #[test]
    fn substitutions_work_and_missing_ones_are_reported() {
        let mut c = Catalog::embedded(Locale::En);
        let out = c.format("lesson.progress", &FormatArgs::new().insert("current", 3));
        assert_eq!(out, "Lesson 3 of {total}");
        assert_eq!(
            c.diagnostics(),
            &[LocalizationDiagnostic::MissingArgument {
                key: "lesson.progress".into(),
                argument: "total".into()
            }]
        );
    }
    #[test]
    fn plural_rules_support_ru_and_en() {
        let mut ru = Catalog::embedded(Locale::Ru);
        assert_eq!(
            ru.format("lesson.complete", &FormatArgs::new().count(1)),
            "Завершено 1 упражнение"
        );
        assert_eq!(
            ru.format("lesson.complete", &FormatArgs::new().count(3)),
            "Завершено 3 упражнения"
        );
        assert_eq!(
            ru.format("lesson.complete", &FormatArgs::new().count(5)),
            "Завершено 5 упражнений"
        );
        let mut en = Catalog::embedded(Locale::En);
        assert_eq!(
            en.format("lesson.complete", &FormatArgs::new().count(2)),
            "Completed 2 lessons"
        );
    }
    #[test]
    fn falls_back_to_russian_key() {
        let mut c = Catalog::embedded(Locale::En);
        assert_eq!(
            c.format("error.generic", &FormatArgs::new()),
            "Something went wrong"
        );
        let mut custom = Catalog::from_resources("{\"_locale\":\"en\"}", RU).unwrap();
        assert_eq!(
            custom.format(
                "lesson.progress",
                &FormatArgs::new().insert("current", 1).insert("total", 2)
            ),
            "Упражнение 1 из 2"
        );
    }
    #[test]
    fn fallback_is_diagnosed_and_uses_fallback_plural_locale() {
        let fallback = r#"{"_locale":"ru","n":{"one":"one {count}","few":"few {count}","many":"many {count}"}}"#;
        let mut c = Catalog::from_resources(r#"{"_locale":"en"}"#, fallback).unwrap();
        assert_eq!(c.format("n", &FormatArgs::new().count(2)), "few 2");
        assert_eq!(
            c.diagnostics(),
            &[LocalizationDiagnostic::MissingKey("n".into())]
        );
    }
    #[test]
    fn missing_plural_form_and_wrong_value_type_are_diagnosed() {
        let resources = r#"{"_locale":"en","n":{"one":"one"},"bad":42}"#;
        let mut c = Catalog::from_resources(resources, "{}").unwrap();
        assert_eq!(c.format("n", &FormatArgs::new().count(2)), "⟦n⟧");
        assert!(c
            .diagnostics()
            .contains(&LocalizationDiagnostic::MissingPluralForm {
                key: "n".into(),
                category: "other".into()
            }));
        assert_eq!(c.format("bad", &FormatArgs::new()), "⟦bad⟧");
        assert!(c
            .diagnostics()
            .contains(&LocalizationDiagnostic::InvalidValue { key: "bad".into() }));
    }
    #[test]
    fn minimum_i64_plural_count_is_safe() {
        let mut c = Catalog::embedded(Locale::Ru);
        assert_eq!(
            c.format("lesson.complete", &FormatArgs::new().count(i64::MIN)),
            "Завершено -9223372036854775808 упражнений"
        );
    }
}
