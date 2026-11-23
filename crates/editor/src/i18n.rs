//! The editor's texts in every language it speaks (docs/editor-i18n.md).
//!
//! All texts the user reads live in `crates/editor/i18n/texts.ron`, keyed
//! like `"toolbar.file"`, each with an English and a German version. Code
//! asks for them with `tr!`:
//!
//! ```ignore
//! tr!("toolbar.file")                         // &'static str
//! tr!("status.assets_registered", count = n)  // String, `{count}` filled in
//! ```
//!
//! The language is the user's choice (File > Settings), kept in the
//! editor's preferences; changing it rebuilds the editor's panels.

use std::{
    collections::HashMap,
    fmt::Display,
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        OnceLock,
    },
};

use ferrite::serde::Deserialize;

/// The texts, built into the editor.
const TEXTS: &str = include_str!("../i18n/texts.ron");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    German,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::German];

    /// Its name in files: `"en"`, `"de"`.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::German => "de",
        }
    }

    /// Its name in itself, as the language menu lists it.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::German => "Deutsch",
        }
    }

    pub fn from_code(code: &str) -> Option<Language> {
        Language::ALL.into_iter().find(|l| l.code() == code.trim().to_lowercase())
    }

    /// The system's language (`LC_ALL`, `LC_MESSAGES`, `LANG`), English
    /// for anything but German.
    pub fn from_system() -> Language {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        if locale.to_lowercase().starts_with("de") {
            Language::German
        } else {
            Language::English
        }
    }

    fn index(self) -> u8 {
        self as u8
    }

    fn from_index(index: u8) -> Language {
        if index == Language::German as u8 {
            Language::German
        } else {
            Language::English
        }
    }
}

/// One text in every language.
#[derive(Deserialize, Debug)]
#[serde(crate = "ferrite::serde")]
struct Entry {
    en: String,
    de: String,
}

struct Text {
    en: &'static str,
    de: &'static str,
}

static LANGUAGE: AtomicU8 = AtomicU8::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn parse(source: &str) -> Result<HashMap<String, Entry>, String> {
    ferrite::ron::from_str(source).map_err(|e| format!("i18n/texts.ron: {e}"))
}

fn table() -> &'static HashMap<&'static str, Text> {
    static TABLE: OnceLock<HashMap<&'static str, Text>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let entries = parse(TEXTS).unwrap_or_else(|e| {
            log::error!("{e}");
            HashMap::new()
        });
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        entries
            .into_iter()
            .map(|(key, entry)| (leak(key), Text { en: leak(entry.en), de: leak(entry.de) }))
            .collect()
    })
}

pub fn language() -> Language {
    Language::from_index(LANGUAGE.load(Ordering::Relaxed))
}

/// Speak `language` from now on; texts already shown stay until their
/// panel is built again (see `generation`).
pub fn set_language(language: Language) {
    if LANGUAGE.swap(language.index(), Ordering::Relaxed) != language.index() {
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Counts language changes: the editor rebuilds its panels when it moves.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// The text for `key` in the current language, if there is one (for keys
/// made at run time, like a graphics setting's).
pub fn lookup(key: &str) -> Option<&'static str> {
    let text = table().get(key)?;
    let chosen = match language() {
        Language::English => text.en,
        Language::German => text.de,
    };
    // A text not translated yet shows in English.
    Some(if chosen.is_empty() { text.en } else { chosen })
}

/// The text for `key` in the current language; the key itself if the
/// file has no such text (a test makes sure it has every one).
pub fn text(key: &'static str) -> &'static str {
    lookup(key).unwrap_or_else(|| {
        log::warn!("i18n: no text for \"{key}\"");
        key
    })
}

/// `text` with each `{name}` replaced by its value.
pub fn fill(text: &str, values: &[(&str, &dyn Display)]) -> String {
    let mut out = text.to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

/// A text of the editor in the current language: `tr!("key")` is a
/// `&'static str`, `tr!("key", name = value, ...)` a `String` with each
/// `{name}` replaced.
macro_rules! tr {
    ($key:literal) => {
        $crate::i18n::text($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fill(
            $crate::i18n::text($key),
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    };
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, path::Path};

    use super::*;

    /// Every `tr!("…")` key in the editor's sources.
    fn used_keys() -> BTreeSet<String> {
        fn visit(dir: &Path, keys: &mut BTreeSet<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(&path, keys);
                } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("i18n.rs") {
                    // Comments left out (this file's examples).
                    let source: String = std::fs::read_to_string(&path)
                        .unwrap()
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("//"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let mut rest = source.as_str();
                    while let Some(at) = rest.find("tr!(") {
                        // Not `include_str!(` and the like.
                        let macro_start = rest[..at].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
                        rest = rest[at + 4..].trim_start();
                        if !macro_start {
                            continue;
                        }
                        let Some(quoted) = rest.strip_prefix('"') else { continue };
                        let end = quoted.find('"').unwrap();
                        keys.insert(quoted[..end].to_string());
                        rest = &quoted[end..];
                    }
                }
            }
        }
        let mut keys = BTreeSet::new();
        visit(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut keys);
        keys
    }

    #[test]
    fn the_texts_file_parses_and_every_text_has_both_languages() {
        let entries = parse(TEXTS).unwrap();
        let missing: Vec<_> = entries
            .iter()
            .filter(|(_, e)| e.en.trim().is_empty() || e.de.trim().is_empty())
            .map(|(key, _)| key.clone())
            .collect();
        assert!(missing.is_empty(), "texts without a translation: {missing:?}");
    }

    #[test]
    fn every_key_the_code_asks_for_has_a_text() {
        let entries = parse(TEXTS).unwrap();
        let missing: Vec<_> = used_keys().into_iter().filter(|k| !entries.contains_key(k)).collect();
        assert!(missing.is_empty(), "keys missing from i18n/texts.ron: {missing:?}");
    }

    #[test]
    fn placeholders_match_between_the_languages() {
        let names = |text: &str| -> BTreeSet<String> {
            let mut names = BTreeSet::new();
            let mut rest = text;
            while let Some(open) = rest.find('{') {
                rest = &rest[open + 1..];
                if let Some(close) = rest.find('}') {
                    names.insert(rest[..close].to_string());
                    rest = &rest[close + 1..];
                }
            }
            names
        };
        for (key, entry) in parse(TEXTS).unwrap() {
            assert_eq!(names(&entry.en), names(&entry.de), "placeholders of \"{key}\" differ");
        }
    }

    #[test]
    fn fill_replaces_names() {
        assert_eq!(fill("{n} of {total}", &[("n", &3), ("total", &"7")]), "3 of 7");
    }
}
