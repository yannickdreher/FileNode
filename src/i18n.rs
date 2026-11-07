//! Presentation-only localization. Protocol fields and persisted sync paths never depend on locale.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
pub enum Language {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "en")]
    #[value(name = "en")]
    English,
    #[serde(rename = "de")]
    #[value(name = "de")]
    German,
}

impl Language {
    pub fn resolve(self) -> Self {
        match self {
            Self::Auto => Self::from_locale(sys_locale::get_locale().as_deref().unwrap_or("en")),
            value => value,
        }
    }

    fn from_locale(locale: &str) -> Self {
        if locale
            .split(['-', '_', '.'])
            .next()
            .is_some_and(|v| v.eq_ignore_ascii_case("de"))
        {
            Self::German
        } else {
            Self::English
        }
    }
}

fn catalogs() -> &'static [BTreeMap<String, String>; 2] {
    static CATALOGS: OnceLock<[BTreeMap<String, String>; 2]> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        [
            serde_json::from_str(include_str!("../locales/en.json"))
                .expect("valid English catalog"),
            serde_json::from_str(include_str!("../locales/de.json")).expect("valid German catalog"),
        ]
    })
}

pub fn text(language: Language, key: &str) -> &'static str {
    let catalogs = catalogs();
    let index = usize::from(language.resolve() == Language::German);
    catalogs[index]
        .get(key)
        .or_else(|| catalogs[0].get(key))
        .map(String::as_str)
        .unwrap_or("[missing translation]")
}

/// Keep events unrendered so switching languages also translates past activity.
#[derive(Clone, Debug, Serialize)]
pub struct Message {
    key: &'static str,
    arguments: Vec<(&'static str, String)>,
}

impl Message {
    pub fn argument(&self, key: &str) -> Option<&str> {
        self.arguments
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.as_str())
    }
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            arguments: Vec::new(),
        }
    }

    pub fn with(mut self, key: &'static str, value: impl ToString) -> Self {
        self.arguments.push((key, value.to_string()));
        self
    }

    pub fn render(&self, language: Language) -> String {
        // Substitute in a single pass: braces in a user-supplied file name are literal.
        let template = text(language, self.key);
        let mut output = String::new();
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            output.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}').map(|v| v + start) else {
                break;
            };
            let key = &rest[start + 1..end];
            if let Some((_, value)) = self.arguments.iter().find(|(name, _)| *name == key) {
                output.push_str(value);
            } else {
                output.push_str(&rest[start..=end]);
            }
            rest = &rest[end + 1..];
        }
        output.push_str(rest);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn placeholders(value: &str) -> BTreeSet<&str> {
        value
            .split('{')
            .skip(1)
            .map(|s| s.split('}').next().unwrap())
            .collect()
    }

    #[test]
    fn catalogs_have_identical_keys_and_placeholders() {
        let [english, german] = catalogs();
        assert_eq!(
            english.keys().collect::<Vec<_>>(),
            german.keys().collect::<Vec<_>>()
        );
        for (key, value) in english {
            assert!(
                !value.trim().is_empty() && !german[key].trim().is_empty(),
                "{key}"
            );
            assert_eq!(placeholders(value), placeholders(&german[key]), "{key}");
        }
    }

    #[test]
    fn language_switch_renders_existing_events_without_changing_arguments() {
        let event = Message::new("log.local_rename")
            .with("from", "{to}.txt")
            .with("to", "notes.txt");
        assert_eq!(
            event.render(Language::English),
            "Local rename applied: {to}.txt → notes.txt"
        );
        assert_eq!(
            event.render(Language::German),
            "Lokale Umbenennung übernommen: {to}.txt → notes.txt"
        );
    }

    #[test]
    fn system_locales_and_persisted_preference_are_stable() {
        assert_eq!(Language::from_locale("de-DE"), Language::German);
        assert_eq!(Language::from_locale("de_CH.UTF-8"), Language::German);
        assert_eq!(Language::from_locale("fr-FR"), Language::English);
        let config = crate::config::Config {
            language: Language::German,
            ..Default::default()
        };
        let encoded = toml::to_string(&config).unwrap();
        assert_eq!(
            toml::from_str::<crate::config::Config>(&encoded)
                .unwrap()
                .language,
            Language::German
        );
    }
}
