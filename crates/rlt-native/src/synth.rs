//! Morphological synthesis: the inverse of the tagger. Maps `(lemma, tag)` to the surface forms the
//! lexicon records for that pair — e.g. `(sein, VER:3:SIN:PRÄ:NON) → ist` — so rule suggestions such
//! as "hat gegangen" → "ist gegangen" can name the right inflected form.
//!
//! Artifact: an `fst::Set` of `lemma \0 tag \0 form` keys, derived offline from the tagger artifact
//! (`build_synth`). A lookup is one prefix range scan.

use fst::{IntoStreamer, Set, SetBuilder, Streamer};
use rlt_core::Synthesizer;

use crate::tagger::{Tagger, TaggerError};

const SEP: u8 = 0;

/// Build the synthesis artifact (fst bytes) from a loaded tagger.
///
/// # Errors
/// Returns [`TaggerError`] if the fst cannot be built.
pub fn build_synth(tagger: &Tagger) -> Result<Vec<u8>, TaggerError> {
    let mut keys: Vec<Vec<u8>> = Vec::new();
    tagger.for_each_analysis(|word, lemma, tag| {
        let mut k = Vec::with_capacity(lemma.len() + tag.len() + word.len() + 2);
        k.extend_from_slice(lemma.as_bytes());
        k.push(SEP);
        k.extend_from_slice(tag.as_bytes());
        k.push(SEP);
        k.extend_from_slice(word.as_bytes());
        keys.push(k);
    });
    keys.sort_unstable();
    keys.dedup();
    let mut builder = SetBuilder::memory();
    for k in &keys {
        builder
            .insert(k)
            .map_err(|e| TaggerError::Fst(e.to_string()))?;
    }
    builder
        .into_inner()
        .map_err(|e| TaggerError::Fst(e.to_string()))
}

/// A loaded synthesis artifact.
pub struct FstSynthesizer {
    set: Set<Vec<u8>>,
}

impl FstSynthesizer {
    /// Load from the artifact bytes.
    ///
    /// # Errors
    /// Returns [`TaggerError`] if the bytes are not a valid fst.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, TaggerError> {
        Ok(Self {
            set: Set::new(bytes).map_err(|e| TaggerError::Fst(e.to_string()))?,
        })
    }
}

impl Synthesizer for FstSynthesizer {
    fn forms(&self, lemma: &str, tag: &str) -> Vec<String> {
        let mut lo = Vec::with_capacity(lemma.len() + tag.len() + 2);
        lo.extend_from_slice(lemma.as_bytes());
        lo.push(SEP);
        lo.extend_from_slice(tag.as_bytes());
        lo.push(SEP);
        let mut hi = lo.clone();
        *hi.last_mut().unwrap_or(&mut 0) = SEP + 1;
        let mut out = Vec::new();
        let mut stream = self.set.range().ge(&lo).lt(&hi).into_stream();
        while let Some(key) = stream.next() {
            if let Ok(form) = std::str::from_utf8(&key[lo.len()..]) {
                out.push(form.to_owned());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tagger::build_from_triples;

    #[test]
    fn synthesizes_forms_for_lemma_and_tag() {
        let t = |a: &str, b: &str, c: &str| (a.to_owned(), b.to_owned(), c.to_owned());
        let bytes = build_from_triples([
            t("ist", "sein", "VER:3:SIN:PRÄ:NON"),
            t("sind", "sein", "VER:3:PLU:PRÄ:NON"),
            t("hat", "haben", "VER:3:SIN:PRÄ:NON"),
        ])
        .expect("tagger");
        let tagger = Tagger::from_bytes(&bytes).expect("load");
        let synth = FstSynthesizer::from_bytes(build_synth(&tagger).expect("build")).expect("fst");
        assert_eq!(synth.forms("sein", "VER:3:SIN:PRÄ:NON"), vec!["ist"]);
        assert_eq!(synth.forms("sein", "VER:3:PLU:PRÄ:NON"), vec!["sind"]);
        assert!(synth.forms("sein", "VER:1:SIN:PRÄ:NON").is_empty());
    }
}
