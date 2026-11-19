//! Word-list replacement rules: LanguageTool's `replace.txt`-style files (`wrong|wrong2=right|right2`),
//! one entry per line, `#` comments. Matches single words and short phrases case-insensitively.

use std::collections::HashMap;

use crate::{Analysis, Diagnostic, Source, Span, Suggestion, Token};

/// A parsed replacement list.
#[derive(Debug, Clone, Default)]
pub struct Wordlist {
    /// lower-cased wrong form (words joined by one space) → suggested replacements.
    entries: HashMap<String, Vec<String>>,
    /// Longest key in words, to bound the phrase scan.
    max_words: usize,
}

impl Wordlist {
    /// Parse LanguageTool replacement-list text. Unparseable lines are skipped.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut entries: HashMap<String, Vec<String>> = HashMap::new();
        let mut max_words = 1;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((wrong, right)) = line.split_once('=') else {
                continue;
            };
            let right: Vec<String> = right
                .split('|')
                .map(|s| s.split('#').next().unwrap_or("").trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            if right.is_empty() {
                continue;
            }
            for w in wrong.split('|') {
                let w = w.trim().to_lowercase();
                if w.is_empty() {
                    continue;
                }
                max_words = max_words.max(w.split(' ').count());
                entries.entry(w).or_insert_with(|| right.clone());
            }
        }
        Self { entries, max_words }
    }

    /// Number of wrong forms in the list.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the list has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn diagnostics(&self, analysis: &Analysis, message: &str) -> Vec<Diagnostic> {
        let toks: Vec<&Token> = analysis
            .tokens
            .iter()
            .filter(|t| t.text.chars().any(char::is_alphabetic))
            .collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            let mut hit = None;
            for n in (1..=self.max_words.min(toks.len() - i)).rev() {
                let window = &toks[i..i + n];
                // Words of a phrase must be adjacent (one space between).
                if window.windows(2).any(|w| w[1].span.start != w[0].span.end + 1) {
                    continue;
                }
                let key = window.iter().map(|t| t.text.to_lowercase()).collect::<Vec<_>>().join(" ");
                if let Some(r) = self.entries.get(&key) {
                    hit = Some((n, key, r));
                    break;
                }
            }
            let Some((n, key, right)) = hit else {
                i += 1;
                continue;
            };
            let first = toks[i];
            let upper = first.text.chars().next().is_some_and(char::is_uppercase);
            let suggestions = right
                .iter()
                .take(4)
                .map(|r| Suggestion {
                    replacement: if upper { capitalise(r) } else { r.clone() },
                })
                .collect();
            out.push(Diagnostic {
                span: Span {
                    start: first.span.start,
                    end: toks[i + n - 1].span.end,
                },
                code: "WORDLIST_REPLACE".into(),
                message: message.replace("{}", &key),
                suggestions,
                source: Source::Grammar,
                details: None,
            });
            i += n;
        }
        out
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analysis(words: &[&str]) -> Analysis {
        let mut pos = 0;
        let tokens = words
            .iter()
            .map(|w| {
                let t = Token {
                    text: (*w).into(),
                    span: Span { start: pos, end: pos + w.len() },
                    ..Default::default()
                };
                pos += w.len() + 1;
                t
            })
            .collect();
        Analysis { tokens }
    }

    #[test]
    fn parses_alternatives_and_matches_words_and_phrases() {
        let l = Wordlist::parse("# c\nsatisfacere|satisfacera=satisfaré|satisfará\na basto=abasto\n");
        assert_eq!(l.len(), 3);
        let d = l.diagnostics(&analysis(&["Satisfacera", "mucho"]), "«{}»");
        assert_eq!(d[0].suggestions[0].replacement, "Satisfaré");
        let p = l.diagnostics(&analysis(&["dar", "a", "basto"]), "«{}»");
        assert_eq!(p[0].span, Span { start: 4, end: 11 });
    }
}
