//! Zeilenpuffer: viele kurze Textzeilen (G-Code) in einem einzigen String mit Zeilenenden. Gegenüber einem
//! `Vec<String>` spart das je Zeile eine Speicheranforderung und rund 40 Byte – bei Millionen Zeilen großer
//! Rasterjobs Hunderte Megabyte.

use std::fmt;
use std::ops::Index;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct LineBuf {
    /// Alle Zeilen, getrennt durch `\n` (ohne abschließenden Zeilenumbruch).
    text: String,
    /// Ende jeder Zeile in `text` (exklusive).
    ends: Vec<usize>,
}

impl LineBuf {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(lines: usize, bytes: usize) -> Self {
        Self { text: String::with_capacity(bytes), ends: Vec::with_capacity(lines) }
    }

    /// Hängt eine Zeile an (ohne Zeilenumbruch darin).
    pub fn push(&mut self, line: &str) {
        debug_assert!(!line.contains('\n'), "Zeilenumbruch in einer Zeile");
        if !self.ends.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(line);
        self.ends.push(self.text.len());
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&str> {
        let end = *self.ends.get(i)?;
        let start = if i == 0 { 0 } else { self.ends[i - 1] + 1 };
        Some(&self.text[start..end])
    }

    pub fn last(&self) -> Option<&str> {
        self.len().checked_sub(1).and_then(|i| self.get(i))
    }

    pub fn iter(&self) -> Iter<'_> {
        Iter { buf: self, front: 0, back: self.len() }
    }

    /// Alle Zeilen, durch `\n` getrennt (ohne abschließenden Zeilenumbruch).
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn to_vec(&self) -> Vec<String> {
        self.iter().map(str::to_string).collect()
    }
}

impl Index<usize> for LineBuf {
    type Output = str;

    fn index(&self, i: usize) -> &str {
        self.get(i).unwrap_or_else(|| panic!("Zeile {i} von {}", self.len()))
    }
}

impl fmt::Debug for LineBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// Iterator über die Zeilen eines [`LineBuf`].
#[derive(Clone)]
pub struct Iter<'a> {
    buf: &'a LineBuf,
    front: usize,
    back: usize,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        (self.front < self.back).then(|| {
            self.front += 1;
            &self.buf[self.front - 1]
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.back - self.front;
        (n, Some(n))
    }
}

impl DoubleEndedIterator for Iter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        (self.front < self.back).then(|| {
            self.back -= 1;
            &self.buf[self.back]
        })
    }
}

impl ExactSizeIterator for Iter<'_> {}

impl<'a> IntoIterator for &'a LineBuf {
    type Item = &'a str;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl<S: AsRef<str>> FromIterator<S> for LineBuf {
    fn from_iter<I: IntoIterator<Item = S>>(iter: I) -> Self {
        let mut buf = LineBuf::new();
        buf.extend(iter);
        buf
    }
}

impl<S: AsRef<str>> Extend<S> for LineBuf {
    fn extend<I: IntoIterator<Item = S>>(&mut self, iter: I) {
        for l in iter {
            self.push(l.as_ref());
        }
    }
}

impl From<Vec<String>> for LineBuf {
    fn from(v: Vec<String>) -> Self {
        v.into_iter().collect()
    }
}

impl<const N: usize> From<[&str; N]> for LineBuf {
    fn from(v: [&str; N]) -> Self {
        v.into_iter().collect()
    }
}

/// Bequemer Vergleich in Tests: `assert_eq!(buf, ["G0 X1", "G1 X2"])`.
impl<S: AsRef<str>> PartialEq<[S]> for LineBuf {
    fn eq(&self, other: &[S]) -> bool {
        self.len() == other.len() && self.iter().zip(other).all(|(a, b)| a == b.as_ref())
    }
}

impl<S: AsRef<str>, const N: usize> PartialEq<[S; N]> for LineBuf {
    fn eq(&self, other: &[S; N]) -> bool {
        *self == other[..]
    }
}

impl<S: AsRef<str>> PartialEq<Vec<S>> for LineBuf {
    fn eq(&self, other: &Vec<S>) -> bool {
        *self == other[..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_index_iterate() {
        let mut b = LineBuf::new();
        assert!(b.is_empty() && b.last().is_none() && b.get(0).is_none());
        b.push("G0 X1");
        b.push("");
        b.push("G1 X2 S100");
        assert_eq!(b.len(), 3);
        assert_eq!((&b[0], &b[1], &b[2]), ("G0 X1", "", "G1 X2 S100"));
        assert_eq!(b.last(), Some("G1 X2 S100"));
        assert_eq!(b.text(), "G0 X1\n\nG1 X2 S100");
        assert_eq!(b.iter().rev().collect::<Vec<_>>(), ["G1 X2 S100", "", "G0 X1"]);
        let mut it = b.iter();
        assert_eq!((it.next(), it.next_back(), it.len()), (Some("G0 X1"), Some("G1 X2 S100"), 1));
        assert_eq!(format!("{b:?}"), r#"["G0 X1", "", "G1 X2 S100"]"#);
        assert_eq!(b, ["G0 X1", "", "G1 X2 S100"]);
        assert_eq!(b.to_vec(), vec!["G0 X1".to_string(), String::new(), "G1 X2 S100".to_string()]);
        let c: LineBuf = vec!["a".to_string(), "b".to_string()].into();
        assert_eq!(c, ["a", "b"]);
        assert_ne!(c, ["a"]);
        // eine einzelne leere Zeile ist etwas anderes als keine Zeile
        let e: LineBuf = [""].into();
        assert_eq!((e.len(), &e[0]), (1, ""));
    }
}
