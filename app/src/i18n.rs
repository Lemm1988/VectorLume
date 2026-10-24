//! Sprache der Oberfläche. Texte stehen im Quelltext auf Deutsch und werden zur Laufzeit übersetzt:
//! `tr("…")` für feste Texte, `trs(&format!(…))` für zusammengesetzte. Zusammengesetzte Texte (auch Meldungen aus
//! den Bibliotheken) werden über Vorlagen mit `{}` erkannt, z. B. `„{}“ geöffnet.` → `Opened "{}".`; die
//! eingesetzten Werte bleiben erhalten (und werden ihrerseits übersetzt, falls es bekannte Texte sind).
//! Fehlt eine Übersetzung, erscheint der deutsche Text.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    German,
    English,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::German, Language::English];

    /// Name in der eigenen Sprache (wird nicht übersetzt).
    pub fn label(self) -> &'static str {
        match self {
            Language::German => "Deutsch",
            Language::English => "English",
        }
    }

    /// Sprache des Systems: Deutsch, wenn die Benutzeroberfläche deutsch ist, sonst Englisch.
    pub fn from_system() -> Self {
        match sys_locale::get_locale() {
            Some(l) if !l.to_ascii_lowercase().starts_with("de") => Language::English,
            _ => Language::German,
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_language(l: Language) {
    LANG.store(l as u8, Ordering::Relaxed);
    CACHE.with(|c| c.borrow_mut().clear());
}

pub fn language() -> Language {
    if LANG.load(Ordering::Relaxed) == Language::English as u8 { Language::English } else { Language::German }
}

struct Template {
    /// Deutsche Vorlage, an den Platzhaltern geteilt.
    parts: Vec<&'static str>,
    en: &'static str,
}

impl Template {
    /// Eingesetzte Werte, falls `s` zur Vorlage passt.
    fn matches<'s>(&self, s: &'s str) -> Option<Vec<&'s str>> {
        let mut rest = s.strip_prefix(self.parts[0])?;
        let mut args = Vec::with_capacity(self.parts.len() - 1);
        let last = self.parts.len() - 1;
        for (i, part) in self.parts.iter().enumerate().skip(1) {
            if i == last {
                let value = rest.strip_suffix(part)?;
                args.push(value);
                rest = "";
            } else {
                // Mehrdeutig, wenn zwei Platzhalter direkt aufeinander folgen – solche Vorlagen gibt es nicht
                let at = rest.find(part)?;
                args.push(&rest[..at]);
                rest = &rest[at + part.len()..];
            }
        }
        (rest.is_empty() && args.iter().all(|a| !a.is_empty())).then_some(args)
    }
}

struct Tables {
    exact: HashMap<&'static str, &'static str>,
    /// Längste (genaueste) Vorlagen zuerst.
    templates: Vec<Template>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let mut exact = HashMap::new();
        let mut templates = Vec::new();
        for &(de, en) in crate::i18n_en::EN {
            if de.contains("{}") {
                templates.push(Template { parts: de.split("{}").collect(), en });
            } else {
                exact.insert(de, en);
            }
        }
        templates.sort_by_key(|t| std::cmp::Reverse(t.parts.iter().map(|p| p.len()).sum::<usize>()));
        Tables { exact, templates }
    })
}

thread_local! {
    static CACHE: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// Fester Text.
pub fn tr(de: &'static str) -> &'static str {
    if language() == Language::German {
        return de;
    }
    tables().exact.get(de).copied().unwrap_or(de)
}

/// Beliebiger (auch zusammengesetzter) Text.
pub fn trs(s: &str) -> String {
    if language() == Language::German {
        return s.to_string();
    }
    translate(s).unwrap_or_else(|| s.to_string())
}

fn translate(s: &str) -> Option<String> {
    let tb = tables();
    if let Some(en) = tb.exact.get(s) {
        return Some((*en).to_string());
    }
    if let Some(hit) = CACHE.with(|c| c.borrow().get(s).cloned()) {
        return hit;
    }
    let result = tb.templates.iter().find_map(|tpl| tpl.matches(s).map(|args| fill(tpl.en, &args)));
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 4000 {
            c.clear();
        }
        c.insert(s.to_string(), result.clone());
    });
    result
}

/// Tastenkürzel wie „Strg+Umschalt+S“ mit den Tastennamen der Sprache.
pub fn shortcut(keys: &str) -> String {
    if language() == Language::German {
        return keys.to_string();
    }
    keys.split('+')
        .map(|k| match k {
            "Strg" => "Ctrl",
            "Umschalt" => "Shift",
            "Entf" => "Del",
            "Leertaste" => "Space",
            k => k,
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Setzt die Werte in die englische Vorlage ein: `{}` der Reihe nach, `{0}`, `{1}` … gezielt.
fn fill(en: &str, args: &[&str]) -> String {
    // Werte sind kürzer als der ganze Text, die Rekursion endet also
    let value = |i: usize| args.get(i).map(|a| translate(a).unwrap_or_else(|| a.to_string())).unwrap_or_default();
    let mut out = String::with_capacity(en.len() + 16);
    let mut next = 0;
    let mut rest = en;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) if after[..close].chars().all(|c| c.is_ascii_digit()) => {
                let idx = if close == 0 {
                    next += 1;
                    next - 1
                } else {
                    after[..close].parse().unwrap_or(0)
                };
                out.push_str(&value(idx));
                rest = &after[close + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_and_values() {
        let tpl = Template {
            parts: "„{}“ geöffnet ({} Objekte).".split("{}").collect(),
            en: "Opened \"{}\" ({1} objects).",
        };
        let args = tpl.matches("„Bayern.lbrn2“ geöffnet (12 Objekte).").unwrap();
        assert_eq!(args, vec!["Bayern.lbrn2", "12"]);
        assert_eq!(fill(tpl.en, &args), "Opened \"Bayern.lbrn2\" (12 objects).");
        assert!(tpl.matches("„x“ geöffnet.").is_none());
        assert!(tpl.matches("„“ geöffnet ( Objekte).").is_none(), "leere Werte passen nicht");
        assert_eq!(fill("{1} before {0}", &["a", "b"]), "b before a");
        assert_eq!(fill("literal {braces", &[]), "literal {braces");
    }

    #[test]
    fn english_table() {
        assert_eq!(translate("Datei").as_deref(), Some("File"));
        assert_eq!(translate("„Bayern.lbrn2“ geöffnet.").as_deref(), Some("Opened \"Bayern.lbrn2\"."));
        // Werte werden ihrerseits übersetzt, auch verschachtelt
        assert_eq!(
            translate("2× Text als Umrisse übernommen (nicht mehr als Text bearbeitbar)").as_deref(),
            Some("2× Text imported as outlines (no longer editable as text)")
        );
        assert_eq!(
            translate("Verbindung fehlgeschlagen: Lesefehler: x").as_deref(),
            Some("Connection failed: Read error: x")
        );
        assert_eq!(translate("Mein eigener Ebenenname"), None);
        for (de, en) in crate::i18n_en::EN {
            assert!(!en.is_empty() && !de.is_empty());
        }
    }

    /// Jeder Text, der im Quelltext mit `tr("…")` oder `trs(&format!("…"` übersetzt wird, braucht einen Eintrag.
    #[test]
    fn every_source_text_is_translated() {
        let keys: std::collections::HashSet<&str> = crate::i18n_en::EN.iter().map(|(de, _)| *de).collect();
        let mut missing = Vec::new();
        for (file, src) in crate::i18n_en::SOURCES {
            for text in source_texts(src) {
                if !keys.contains(text.as_str()) {
                    missing.push(format!("{file}: {text:?}"));
                }
            }
        }
        assert!(missing.is_empty(), "{} Texte ohne Übersetzung:\n{}", missing.len(), missing.join("\n"));
        let mut seen = std::collections::HashSet::new();
        let dup: Vec<_> = crate::i18n_en::EN.iter().filter(|(de, _)| !seen.insert(*de)).collect();
        assert!(dup.is_empty(), "doppelt: {dup:?}");
    }

    /// Texte aus `tr("…")` und Vorlagen aus `trs(&format!("…"` (Platzhalter vereinheitlicht zu `{}`).
    pub(super) fn source_texts(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (marker, is_format) in [("tr(", false), ("trs(&format!(", true)] {
            let mut rest = src;
            while let Some(at) = rest.find(marker) {
                // nicht Teil eines längeren Namens (z. B. `attr("`)
                let before = rest[..at].chars().next_back();
                rest = &rest[at + marker.len()..];
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let trimmed = rest.trim_start();
                let Some(literal) = trimmed.strip_prefix('"') else { continue };
                rest = literal;
                let (text, len) = unescape(rest);
                rest = &rest[len..];
                out.push(if is_format { placeholders(&text) } else { text });
            }
        }
        out
    }

    /// Inhalt eines Rust-Zeichenkettenliterals bis zum schließenden `"`.
    fn unescape(s: &str) -> (String, usize) {
        let mut out = String::new();
        let mut chars = s.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return (out, i + 1),
                '\\' => match chars.next() {
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 't')) => out.push('\t'),
                    Some((_, '"')) => out.push('"'),
                    Some((_, '\\')) => out.push('\\'),
                    Some((_, '\n')) | Some((_, '\r')) => {
                        while chars.peek().is_some_and(|(_, c)| c.is_whitespace()) {
                            chars.next();
                        }
                    }
                    Some((_, 'u')) => {
                        let mut hex = String::new();
                        for (_, c) in chars.by_ref() {
                            match c {
                                '{' => {}
                                '}' => break,
                                c => hex.push(c),
                            }
                        }
                        if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            out.push(c);
                        }
                    }
                    Some((_, c)) => out.push(c),
                    None => {}
                },
                c => out.push(c),
            }
        }
        (out, s.len())
    }

    /// `{name}`, `{:.1}`, `{x:>7.2}` … → `{}`; `{{`/`}}` → Klammern.
    fn placeholders(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    out.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    out.push('}');
                }
                '{' => {
                    for c in chars.by_ref() {
                        if c == '}' {
                            break;
                        }
                    }
                    out.push_str("{}");
                }
                c => out.push(c),
            }
        }
        out
    }

    #[test]
    fn source_text_extraction() {
        let src = "ui.label(tr(\"Ein \\\"Text\\\"\")); x.attr(\"nein\"); trs(&format!(\n  \"{n} von {:.1} mm {{x}}\", a)); tr(\"lang \\\n        weiter\"); tr(name)";
        assert_eq!(source_texts(src), vec!["Ein \"Text\"", "lang weiter", "{} von {} mm {x}"]);
    }
}
