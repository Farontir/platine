//! Translations: every user-visible string comes from the Fluent files in `i18n/`.

use std::borrow::Cow;

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use unic_langid::LanguageIdentifier;

/// A supported interface language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    /// English, the fallback language.
    English,
    /// French.
    French,
}

impl Language {
    /// Every supported language.
    pub const ALL: [Language; 2] = [Language::English, Language::French];

    /// BCP 47 code of the language.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::French => "fr",
        }
    }

    /// Fluent source of the language, embedded at build time.
    pub fn source(self) -> &'static str {
        match self {
            Language::English => include_str!("../../../i18n/en/patine.ftl"),
            Language::French => include_str!("../../../i18n/fr/patine.ftl"),
        }
    }

    /// Language matching a locale such as `fr-FR`, falling back to English.
    pub fn from_locale(locale: &str) -> Self {
        let primary = locale.split(['-', '_']).next().unwrap_or_default();
        Self::ALL
            .into_iter()
            .find(|language| primary.eq_ignore_ascii_case(language.code()))
            .unwrap_or(Language::English)
    }

    /// Language of the operating system, falling back to English.
    pub fn system() -> Self {
        sys_locale::get_locale()
            .map(|locale| Self::from_locale(&locale))
            .unwrap_or(Language::English)
    }
}

/// Translated strings of the current language.
pub struct I18n {
    language: Language,
    bundle: FluentBundle<FluentResource>,
}

impl I18n {
    /// Loads the strings of `language`.
    pub fn new(language: Language) -> Self {
        let id: LanguageIdentifier = language
            .code()
            .parse()
            .expect("language codes are valid BCP 47 identifiers");
        let mut bundle = FluentBundle::new(vec![id]);
        // Unicode isolation marks would show up as boxes in egui.
        bundle.set_use_isolating(false);
        let resource = match FluentResource::try_new(language.source().to_owned()) {
            Ok(resource) => resource,
            Err((resource, errors)) => {
                // Unreachable in practice: a test parses every file.
                tracing::error!(
                    ?errors,
                    language = language.code(),
                    "translation file has errors"
                );
                resource
            }
        };
        if let Err(errors) = bundle.add_resource(resource) {
            tracing::error!(
                ?errors,
                language = language.code(),
                "duplicate translation keys"
            );
        }
        Self { language, bundle }
    }

    /// Current language.
    pub fn language(&self) -> Language {
        self.language
    }

    /// Translation of `key`. A missing key yields the key itself, so it is visible in the UI.
    pub fn tr(&self, key: &str) -> Cow<'_, str> {
        self.format(key, None)
    }

    /// Translation of `key` with Fluent arguments.
    pub fn tr_args(&self, key: &str, args: &FluentArgs<'_>) -> String {
        self.format(key, Some(args)).into_owned()
    }

    fn format(&self, key: &str, args: Option<&FluentArgs<'_>>) -> Cow<'_, str> {
        let Some(pattern) = self.bundle.get_message(key).and_then(|m| m.value()) else {
            tracing::warn!(key, language = self.language.code(), "missing translation");
            return Cow::Owned(key.to_owned());
        };
        let mut errors = Vec::new();
        let text = self.bundle.format_pattern(pattern, args, &mut errors);
        if !errors.is_empty() {
            tracing::warn!(key, ?errors, "translation formatting errors");
        }
        text
    }

    /// Whether `key` exists in the current language.
    pub fn has(&self, key: &str) -> bool {
        self.bundle.has_message(key)
    }
}

/// Message keys defined in a Fluent source, in file order.
pub fn message_keys(source: &str) -> Result<Vec<String>, String> {
    let resource =
        FluentResource::try_new(source.to_owned()).map_err(|(_, errors)| format!("{errors:?}"))?;
    Ok(resource
        .entries()
        .filter_map(|entry| match entry {
            fluent_syntax::ast::Entry::Message(message) => Some(message.id.name.to_owned()),
            _ => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_language_parses() {
        for language in Language::ALL {
            let keys = message_keys(language.source()).unwrap();
            assert!(!keys.is_empty(), "{language:?}");
            let unique: BTreeSet<_> = keys.iter().collect();
            assert_eq!(unique.len(), keys.len(), "duplicate keys in {language:?}");
        }
    }

    #[test]
    fn languages_have_the_same_keys() {
        let english: BTreeSet<_> = message_keys(Language::English.source())
            .unwrap()
            .into_iter()
            .collect();
        for language in Language::ALL {
            let keys: BTreeSet<_> = message_keys(language.source())
                .unwrap()
                .into_iter()
                .collect();
            let missing: Vec<_> = english.difference(&keys).collect();
            let extra: Vec<_> = keys.difference(&english).collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{language:?}: missing {missing:?}, extra {extra:?}"
            );
        }
    }

    #[test]
    fn locale_matching() {
        assert_eq!(Language::from_locale("fr-FR"), Language::French);
        assert_eq!(Language::from_locale("fr_CA"), Language::French);
        assert_eq!(Language::from_locale("en-US"), Language::English);
        assert_eq!(Language::from_locale("de-DE"), Language::English);
        assert_eq!(Language::from_locale(""), Language::English);
    }

    #[test]
    fn formats_with_arguments_and_without_isolation_marks() {
        let i18n = I18n::new(Language::French);
        let mut args = FluentArgs::new();
        args.set("version", "1.2.3");
        assert_eq!(i18n.tr_args("about-version", &args), "Version 1.2.3");
        assert_eq!(i18n.tr("panel-layers"), "Calques");
    }

    #[test]
    fn missing_key_is_visible() {
        let i18n = I18n::new(Language::English);
        assert_eq!(i18n.tr("no-such-key"), "no-such-key");
    }
}
