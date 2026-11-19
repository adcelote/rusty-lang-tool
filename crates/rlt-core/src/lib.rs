//! Runtime checker core.
//!
//! This crate owns the **abstractions** every surface (CLI, WASM) and every cascade layer agree
//! on, but it does *not* depend on any concrete engine — the engine is injected. That dependency
//! inversion is the seam described in the plan: [`Engine`] is implemented by `rlt-engine` (a
//! vendored nlprule) today and can be swapped for a custom implementation later without touching
//! callers.
//!
//! ## Layout
//! - [`Engine`] — linguistic analysis seam (tokenize/tag/chunk → [`Analysis`]).
//! - [`Diagnostic`] & friends — the uniform output every layer emits.
//! - [`Checker`] — wires an [`Engine`] + L1 spelling + L2 rule matching and runs the cascade.
//!
//! M0 establishes these shapes; L1/L2 bodies arrive in M3/M4.

#![forbid(unsafe_code)]

mod disambig;
mod matcher;
mod spell;
#[doc(hidden)]
pub use spell::fuzz_edits1;

pub use disambig::ConfusionChecker;
pub use matcher::{Disambiguator, IrMatcher};

use serde::{Deserialize, Serialize};

/// Upper-case the first character of `s`, leaving the rest unchanged (`receive` → `Receive`).
pub fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Append `value` to `out` iff it is non-empty and not already present (order-preserving unique) —
/// taggers yield the same POS/lemma across a word's analyses, and downstream wants each once.
pub fn push_unique(out: &mut Vec<String>, value: &str) {
    if !value.is_empty() && !out.iter().any(|v| v == value) {
        out.push(value.to_owned());
    }
}

/// Re-case `candidate` to match `source`'s leading capitalization (so `Recieve` → `Receive`, not
/// `receive`); leaves it untouched when `source` starts lower-case.
pub(crate) fn recase(source: &str, candidate: &str) -> String {
    if source.chars().next().is_some_and(char::is_uppercase) {
        capitalize_first(candidate)
    } else {
        candidate.to_owned()
    }
}

/// Largest char-boundary offset `<= i` in `text` (`i` clamped to `text.len()` first).
///
/// `str::floor_char_boundary` is still unstable, so this is the stable stand-in.
#[must_use]
pub fn floor_char_boundary(text: &str, i: usize) -> usize {
    let mut i = i.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest char-boundary offset `>= i` in `text` (`i` clamped to `text.len()` first).
///
/// `str::ceil_char_boundary` is still unstable, so this is the stable stand-in.
#[must_use]
pub fn ceil_char_boundary(text: &str, i: usize) -> usize {
    let mut i = i.min(text.len());
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Whether `[start, end)` addresses `text` exactly: both ends in bounds, on char boundaries, and
/// ordered. Use this where a bad span must be **rejected** rather than repaired — the example
/// oracle, which would silently corrupt its own corpus if it spliced at a mid-character offset.
#[must_use]
pub fn is_valid_span(text: &str, start: usize, end: usize) -> bool {
    start <= end && end <= text.len() && text.is_char_boundary(start) && text.is_char_boundary(end)
}

/// `&text[start..end]`, with both ends snapped **inward** to char boundaries and clamped to
/// `text.len()` — never panics, whatever offsets it is handed.
///
/// Byte offsets that reach this crate are ENGINE-REPORTED (an [`Engine`] tokenizer, an L4 tagger's
/// word spans, a `Diagnostic` deserialized from wasm) and so are not trusted to land on a UTF-8 char
/// boundary. Raw indexing panics on the first multi-byte document; since every non-English language
/// this tool ships is full of those, that is a crash on ordinary input rather than an edge case.
/// Use this where a bad span should be **tolerated**; use [`is_valid_span`] where it must be caught.
#[must_use]
pub fn safe_slice(text: &str, start: usize, end: usize) -> &str {
    let end = floor_char_boundary(text, end);
    let start = floor_char_boundary(text, start.min(end));
    &text[start..end]
}

/// A half-open byte range `[start, end)` into the checked text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct Span {
    /// Byte offset of the first byte in the span.
    pub start: usize,
    /// Byte offset one past the last byte in the span.
    pub end: usize,
}

/// A proposed replacement for the text under a diagnostic's span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    /// The text to substitute for the span.
    pub replacement: String,
}

/// Which cascade layer produced a diagnostic. Lets the UI and tests attribute findings, and lets
/// later layers compose with (rather than override) earlier ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// L1 — FSA dictionary spell check.
    Spelling,
    /// L2 — LanguageTool rule grammar.
    Grammar,
    /// L3 — statistical confusion-pair disambiguation (real-word errors, e.g. their/there).
    Statistical,
    /// L4 — neural edit-tagger (GECToR-style; the long-tail grammatical errors L1–L3 miss).
    Neural,
}

/// One finding: a span, the rule/source that flagged it, a human message, and ordered fixes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// The text region the finding applies to.
    pub span: Span,
    /// Machine-readable code: an LT rule id for [`Source::Grammar`], or `"SPELL"` for spelling.
    pub code: String,
    /// Human-readable explanation of the issue.
    pub message: String,
    /// Ordered candidate fixes (best first); may be empty.
    pub suggestions: Vec<Suggestion>,
    /// Which cascade layer produced this finding.
    pub source: Source,
    /// Link and example from the rule that fired (LT rules only), for a fuller "why".
    pub details: Option<rlt_ir::RuleDetails>,
}

/// A single analysed token: surface text, its span, POS tags and lemmas.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct Token {
    /// The token's surface form as it appears in the source text.
    pub text: String,
    /// Where the token sits in the source text.
    pub span: Span,
    /// Part-of-speech tags from the engine's tagger (LT tagset).
    pub tags: Vec<String>,
    /// Lemmas (base forms) the tagger assigned — used by L2 `inflected` token matching.
    pub lemmas: Vec<String>,
}

/// The result of running an [`Engine`] over a piece of text: the token graph downstream layers walk.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct Analysis {
    /// Tokens in source order.
    pub tokens: Vec<Token>,
}

/// Linguistic analysis seam: tokenization, POS tagging, chunking and disambiguation.
///
/// The vendored nlprule lives behind this trait (`rlt-engine`); a future custom engine implements
/// the same trait and drops in unchanged. Nothing in `rlt-core` names a concrete engine type.
pub trait Engine {
    /// Tokenize, tag, chunk and disambiguate `text` into the token graph the cascade walks.
    fn analyze(&self, text: &str) -> Analysis;

    /// Whether `word` is in the engine's lexicon (any inflected form / casing). This is the L1
    /// spelling membership oracle and the validity filter for correction candidates. The future
    /// custom engine answers it from its own FSA dictionary.
    fn is_known(&self, word: &str) -> bool;

    /// How common `word` is on a 0 (unknown or rare) to about 25 (very common) scale, used to rank
    /// spelling suggestions. Engines without frequency data report 0 for everything.
    fn word_frequency(&self, _word: &str) -> u8 {
        0
    }
}

/// L2 — rule-based grammar/style checking seam.
///
/// The baseline (`rlt-engine`) wraps nlprule's rule engine; the on-thesis swap walks `rlt-ir`
/// rules (compiled from current LT) over the token graph. Either way it emits [`Source::Grammar`]
/// diagnostics, so the cascade and the example oracle are agnostic to which backs it.
pub trait GrammarChecker {
    /// Produce grammar/style diagnostics for `text`, given its already-computed [`Analysis`] (the
    /// IR matcher walks the token graph; the nlprule baseline uses `text` directly).
    fn grammar_diagnostics(&self, text: &str, analysis: &Analysis) -> Vec<Diagnostic>;
}

/// Wires the L1 spelling and L2 grammar layers and runs the cascade over text.
///
/// Generic over a backend providing both seams so the concrete type is a compile-time choice at
/// the binary, with no dynamic dispatch on the hot path.
pub struct Checker<B: Engine + GrammarChecker> {
    backend: B,
    /// The script's lower-case alphabet, driving L1 spell-check membership + edit generation.
    /// Defaults to [`spell::ASCII_ALPHABET`] (en/de); the CLI/wasm pass the language's alphabet.
    alphabet: &'static str,
    /// The L1 spelling-diagnostic message, in the checked language (LanguageTool's localized
    /// `spelling` string). Defaults to English; the CLI/wasm pass the language's variant.
    spell_message: &'static str,
    /// Sound-alike confusion pairs for suggestion candidates ([`rlt_lang::SpellConfig::replacement_pairs`]).
    replacement_pairs: &'static [(&'static str, &'static str)],
}

/// English default for the L1 spelling message when no language-specific one is supplied (the nlprule
/// baseline path); the native path passes the localized [`rlt_lang::SpellConfig::message`].
const DEFAULT_SPELL_MESSAGE: &str = "Possible spelling mistake found.";

impl<B: Engine + GrammarChecker> Checker<B> {
    /// Construct a checker over the given analysis + grammar backend, spell-checking against the
    /// ASCII alphabet (the historical English/German default).
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            alphabet: spell::ASCII_ALPHABET,
            spell_message: DEFAULT_SPELL_MESSAGE,
            replacement_pairs: &[],
        }
    }

    /// Construct a checker whose L1 spell-checker uses the active language's `alphabet` (e.g. Cyrillic
    /// for Russian) and localized `spell_message` — both from `rlt_lang::SpellConfig`.
    pub fn with_spell(backend: B, alphabet: &'static str, spell_message: &'static str) -> Self {
        Self {
            backend,
            alphabet,
            spell_message,
            replacement_pairs: &[],
        }
    }

    /// Add the language's sound-alike confusion pairs to the spelling suggestions.
    #[must_use]
    pub fn with_replacement_pairs(
        mut self,
        pairs: &'static [(&'static str, &'static str)],
    ) -> Self {
        self.replacement_pairs = pairs;
        self
    }

    /// Run the full cascade (L1 spelling + L2 grammar) over `text` and return all diagnostics,
    /// ordered by start position.
    #[must_use]
    pub fn check(&self, text: &str) -> Vec<Diagnostic> {
        let analysis = self.backend.analyze(text);
        let mut diagnostics = spell::spelling_diagnostics_with(
            &self.backend,
            &analysis,
            self.alphabet,
            self.spell_message,
            self.replacement_pairs,
        );
        diagnostics.extend(self.backend.grammar_diagnostics(text, &analysis));
        strip_noop_suggestions(text, &mut diagnostics);
        absorb_leading_space(text, &mut diagnostics);
        merge_equivalent(text, &mut diagnostics);
        diagnostics.sort_by_key(|d| d.span.start);
        diagnostics
    }
}

/// Drop "fixes" that would replace a span with the text it already contains. Such **no-op suggestions**
/// are the formal signature of a *cyclic correction*: the user applies the fix, the text is unchanged,
/// the same diagnostic re-fires, and the editor offers the identical no-op again. They arise from
/// false-positive matches (e.g. a spacing rule firing on already-correct text and reconstructing it) or
/// a suggestion template that rebuilds its own input.
///
/// Enforcing "every surfaced suggestion strictly changes its span" makes the cyclic state
/// *unrepresentable* at this single chokepoint (every concrete checker funnels through here): applying
/// any offered fix is guaranteed to change the text, so it always makes progress. A diagnostic whose
/// suggestions were *all* no-ops has no real change to offer and is itself spurious, so it is removed;
/// a diagnostic that never carried a suggestion is informational (nothing to apply, cannot cycle) and
/// is kept.
fn strip_noop_suggestions(text: &str, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.retain_mut(|d| {
        if d.suggestions.is_empty() {
            return true;
        }
        let spanned = text.get(d.span.start..d.span.end).unwrap_or_default();
        d.suggestions.retain(|s| s.replacement != spanned);
        !d.suggestions.is_empty()
    });
}

/// A comma-led fix (", dass") replacing a span that starts right after a space would leave that space
/// in front of the comma ("zeigt , dass"). LanguageTool's marker covers the space in such rules, but the
/// token-based span does not, so extend the span back over the whitespace it must absorb.
fn absorb_leading_space(text: &str, diagnostics: &mut [Diagnostic]) {
    const PUNCT: &[char] = &[',', '.', ';', ':', '!', '?'];
    for d in diagnostics
        .iter_mut()
        .filter(|d| d.source == Source::Grammar)
    {
        let all_punct_led = !d.suggestions.is_empty()
            && d.suggestions
                .iter()
                .all(|s| s.replacement.starts_with(PUNCT));
        if !all_punct_led {
            continue;
        }
        let Some(before) = text.get(..d.span.start) else {
            continue;
        };
        let trimmed = before.trim_end_matches(|c: char| c.is_whitespace() && c != '\n');
        d.span.start = trimmed.len();
    }
}

/// Collapse findings that point at the same problem: two diagnostics whose spans overlap and whose
/// first suggestion produces the *same corrected text* (e.g. LT's `KOMMA_ZWISCHEN_HAUPT_UND_NEBENSATZ_2`
/// "zeigt dass" -> "zeigt, dass" and `SUBJUNKTION_KOMMA` "dass" -> ", dass") are one issue to the
/// writer. The one with the narrower span stays (ties: the earlier one), so the editor marks as little
/// text as possible. Findings without suggestions, and spelling findings, are never merged.
fn merge_equivalent(text: &str, diagnostics: &mut Vec<Diagnostic>) {
    let fixed = |d: &Diagnostic| -> Option<String> {
        let s = d.suggestions.first()?;
        let (a, b) = (d.span.start, d.span.end);
        Some(format!(
            "{}{}{}",
            text.get(..a)?,
            s.replacement,
            text.get(b..)?
        ))
    };
    let results: Vec<Option<String>> = diagnostics
        .iter()
        .map(|d| (d.source == Source::Grammar).then(|| fixed(d)).flatten())
        .collect();
    let mut drop = vec![false; diagnostics.len()];
    for i in 0..diagnostics.len() {
        for j in (i + 1)..diagnostics.len() {
            if drop[i] || drop[j] || results[i].is_none() || results[i] != results[j] {
                continue;
            }
            let (a, b) = (&diagnostics[i].span, &diagnostics[j].span);
            if a.start >= b.end || b.start >= a.end {
                continue; // not overlapping
            }
            let (wa, wb) = (a.end - a.start, b.end - b.start);
            if wb < wa {
                drop[i] = true;
            } else {
                drop[j] = true;
            }
        }
    }
    let mut keep = drop.into_iter().map(|d| !d);
    diagnostics.retain(|_| keep.next().unwrap_or(true));
}

/// Composes a separate [`Engine`] and [`GrammarChecker`] into one backend — e.g. nlprule for
/// analysis (L0/L1) plus the IR matcher for L2 — so [`Checker`] can drive both.
pub struct Composite<E: Engine, G: GrammarChecker> {
    engine: E,
    grammar: G,
}

impl<E: Engine, G: GrammarChecker> Composite<E, G> {
    /// Combine an analysis engine with a grammar checker.
    pub fn new(engine: E, grammar: G) -> Self {
        Self { engine, grammar }
    }
}

impl<E: Engine, G: GrammarChecker> Engine for Composite<E, G> {
    fn analyze(&self, text: &str) -> Analysis {
        self.engine.analyze(text)
    }
    fn is_known(&self, word: &str) -> bool {
        self.engine.is_known(word)
    }
    fn word_frequency(&self, word: &str) -> u8 {
        self.engine.word_frequency(word)
    }
}

impl<E: Engine, G: GrammarChecker> GrammarChecker for Composite<E, G> {
    fn grammar_diagnostics(&self, text: &str, analysis: &Analysis) -> Vec<Diagnostic> {
        self.grammar.grammar_diagnostics(text, analysis)
    }
}

/// A [`GrammarChecker`] that contributes nothing — the L2 seam left deliberately empty. Composing it
/// with an [`Engine`] (via [`Composite`]) yields an L1-spelling-only backend, which the web demo's
/// progressive load uses to light up spelling the instant the tagger arrives, before the grammar IR
/// has finished downloading. Replacing it with a real [`IrMatcher`] later is a drop-in upgrade.
pub struct NoGrammar;

impl GrammarChecker for NoGrammar {
    fn grammar_diagnostics(&self, _text: &str, _analysis: &Analysis) -> Vec<Diagnostic> {
        Vec::new()
    }
}

/// Stacks an additional [`GrammarChecker`] `G` onto an [`Engine`] + [`GrammarChecker`] backend `B`,
/// **concatenating** both layers' diagnostics — additive composition where `G` never overrides `B`.
/// This is the seam every new cascade layer slots onto: analysis (`Engine`) is delegated to `B`,
/// and `grammar_diagnostics` runs `B` first, then appends `G`'s findings. L3 confusion ([`WithConfusion`])
/// and the L4 neural tagger are both just specialisations of this.
pub struct WithGrammar<B: Engine + GrammarChecker, G: GrammarChecker> {
    inner: B,
    extra: G,
}

impl<B: Engine + GrammarChecker, G: GrammarChecker> WithGrammar<B, G> {
    /// Wrap `inner`, appending `extra`'s diagnostics to the cascade.
    pub fn new(inner: B, extra: G) -> Self {
        Self { inner, extra }
    }
}

impl<B: Engine + GrammarChecker, G: GrammarChecker> Engine for WithGrammar<B, G> {
    fn analyze(&self, text: &str) -> Analysis {
        self.inner.analyze(text)
    }
    fn is_known(&self, word: &str) -> bool {
        self.inner.is_known(word)
    }
    fn word_frequency(&self, word: &str) -> u8 {
        self.inner.word_frequency(word)
    }
}

impl<B: Engine + GrammarChecker, G: GrammarChecker> GrammarChecker for WithGrammar<B, G> {
    fn grammar_diagnostics(&self, text: &str, analysis: &Analysis) -> Vec<Diagnostic> {
        let mut diagnostics = self.inner.grammar_diagnostics(text, analysis);
        diagnostics.extend(self.extra.grammar_diagnostics(text, analysis));
        diagnostics
    }
}

/// L3 real-word-error detection stacked onto any backend — the [`WithGrammar`] specialisation for
/// [`ConfusionChecker`]. With an empty model it is a no-op. Construct via `WithConfusion::new`.
pub type WithConfusion<B> = WithGrammar<B, ConfusionChecker>;

#[cfg(test)]
mod noop_guard_tests {
    use super::*;

    fn diag(start: usize, end: usize, reps: &[&str]) -> Diagnostic {
        Diagnostic {
            span: Span { start, end },
            code: "TEST".to_owned(),
            message: String::new(),
            suggestions: reps
                .iter()
                .map(|r| Suggestion {
                    replacement: (*r).to_owned(),
                })
                .collect(),
            source: Source::Grammar,
            details: None,
        }
    }

    #[test]
    fn drops_diagnostic_whose_only_suggestion_is_identity() {
        // ESPACIO firing on correctly-spaced text: span ". No" suggested as ". No".
        let text = "pan. No se";
        let mut d = vec![diag(3, 7, &[". No"])];
        strip_noop_suggestions(text, &mut d);
        assert!(
            d.is_empty(),
            "a sole no-op suggestion → drop the spurious diagnostic"
        );
    }

    #[test]
    fn keeps_real_fix_that_changes_the_span() {
        // ESPACIO firing on genuinely-missing space: span ".No" suggested as ". No".
        let text = "pan.No se";
        let mut d = vec![diag(3, 6, &[". No"])];
        strip_noop_suggestions(text, &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].suggestions.len(), 1);
    }

    #[test]
    fn strips_only_the_noop_when_mixed() {
        let text = "teh cat";
        let mut d = vec![diag(0, 3, &["teh", "the"])]; // identity + real
        strip_noop_suggestions(text, &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].suggestions,
            vec![Suggestion {
                replacement: "the".to_owned()
            }]
        );
    }

    #[test]
    fn keeps_informational_diagnostic_with_no_suggestions() {
        // A spell flag with no candidate (e.g. an OOV with no edit-1 hit) cannot cycle → keep it.
        let text = "xyzzy";
        let mut d = vec![diag(0, 5, &[])];
        strip_noop_suggestions(text, &mut d);
        assert_eq!(d.len(), 1);
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;

    // "naïve" — the 'ï' occupies bytes 2..4, so 3 is a mid-character offset.
    const S: &str = "naïve";

    #[test]
    fn floor_and_ceil_snap_off_a_mid_char_offset() {
        assert!(!S.is_char_boundary(3));
        assert_eq!(floor_char_boundary(S, 3), 2);
        assert_eq!(ceil_char_boundary(S, 3), 4);
    }

    #[test]
    fn boundaries_clamp_past_the_end() {
        assert_eq!(floor_char_boundary(S, 999), S.len());
        assert_eq!(ceil_char_boundary(S, 999), S.len());
    }

    #[test]
    fn safe_slice_does_not_panic_on_a_mid_char_span() {
        assert_eq!(safe_slice(S, 0, 3), "na");
        assert_eq!(safe_slice(S, 3, 999), "ïve");
        assert_eq!(safe_slice(S, 0, S.len()), S);
    }

    #[test]
    fn safe_slice_tolerates_inverted_and_out_of_range_spans() {
        assert_eq!(safe_slice(S, 4, 2), "");
        assert_eq!(safe_slice(S, 900, 999), "");
    }

    #[test]
    fn is_valid_span_rejects_what_safe_slice_repairs() {
        assert!(is_valid_span(S, 0, 2));
        assert!(!is_valid_span(S, 0, 3), "3 is mid-character");
        assert!(!is_valid_span(S, 0, 999), "past the end");
        assert!(!is_valid_span(S, 4, 2), "inverted");
    }

    #[test]
    fn merges_overlapping_findings_with_the_same_fix() {
        let text = "Wir wissen dass es geht";
        let mk = |start: usize, end: usize, rep: &str, code: &str| Diagnostic {
            span: Span { start, end },
            code: code.to_owned(),
            message: String::new(),
            suggestions: vec![Suggestion {
                replacement: rep.to_owned(),
            }],
            source: Source::Grammar,
            details: None,
        };
        let mut d = vec![
            mk(4, 15, "wissen, dass", "WIDE"),
            mk(11, 15, ", dass", "NARROW"),
            mk(16, 18, "xx", "OTHER"),
        ];
        absorb_leading_space(text, &mut d);
        assert_eq!(d[1].span.start, 10); // the space before "dass" is absorbed
        merge_equivalent(text, &mut d);
        let codes: Vec<_> = d.iter().map(|x| x.code.as_str()).collect();
        assert_eq!(codes, ["NARROW", "OTHER"]);
    }
}
