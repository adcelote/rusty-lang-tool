//! Native German agreement checks that LanguageTool implements in Java (not in `grammar.xml`):
//! article–adjective–noun agreement (case, number, gender) and subject–verb agreement.
//!
//! Both are deliberately conservative: they only fire on short, adjacent patterns where every word is
//! fully tagged, and stay silent whenever the tagger could be missing a reading.

use crate::{Analysis, Diagnostic, Source, Span, Suggestion, Token};

const CASES: [&str; 4] = ["NOM", "AKK", "DAT", "GEN"];

/// One (case, number, gender) reading. Gender is `*` when the tag leaves it open (plural nouns, `NOG`).
type Reading = (String, String, String);

fn fields(tag: &str) -> Vec<&str> {
    tag.split(':').collect()
}

fn is_cap(t: &Token) -> bool {
    t.text.chars().next().is_some_and(char::is_uppercase)
}

fn gender_ok(a: &str, b: &str) -> bool {
    a == b || a == "*" || b == "*"
}

fn matches(a: &Reading, b: &Reading) -> bool {
    a.0 == b.0 && a.1 == b.1 && gender_ok(&a.2, &b.2)
}

/// Article readings, with the determiner kind (`DEF`/`IND`).
fn art_readings(t: &Token) -> Option<(&'static str, Vec<Reading>)> {
    let mut kind = None;
    let mut out = Vec::new();
    for tag in &t.tags {
        let f = fields(tag);
        if f.len() >= 5 && f[0] == "ART" {
            let k = match f[1] {
                "DEF" => "DEF",
                "IND" => "IND",
                _ => continue,
            };
            if kind.is_some_and(|x| x != k) {
                return None;
            }
            kind = Some(k);
            out.push((f[2].to_owned(), f[3].to_owned(), f[4].to_owned()));
        }
    }
    kind.map(|k| (k, out))
}

fn adj_readings(t: &Token, kind: &str) -> Vec<Reading> {
    t.tags
        .iter()
        .filter_map(|tag| {
            let f = fields(tag);
            (f.len() >= 6 && f[0] == "ADJ" && f[f.len() - 1] == kind)
                .then(|| (f[1].to_owned(), f[2].to_owned(), f[3].to_owned()))
        })
        .collect()
}

const CONTRACTIONS: &[&str] = &[
    "vom", "zum", "zur", "im", "am", "beim", "ins", "ans", "aufs", "durchs", "fürs", "ums",
    "übers", "unters", "vors",
];

fn has_adj(t: &Token) -> bool {
    t.tags.iter().any(|g| g.starts_with("ADJ:"))
}

/// Uninflected adjective/adverb-like words ("ganz", "recht", "laut") between article and noun: they carry
/// no agreement, so the check looks through them.
fn transparent(t: &Token, kind: &str) -> bool {
    t.tags.iter().any(|g| g.starts_with("ADV")) || adj_readings(t, kind).is_empty()
}

/// Forms that can also be demonstrative/relative pronouns, so a following noun phrase is less certain.
fn pronoun_capable(t: &Token) -> bool {
    t.tags.iter().any(|g| g.starts_with("PRO:"))
}

fn noun_readings(t: &Token) -> Vec<Reading> {
    t.tags
        .iter()
        .filter_map(|tag| {
            let f = fields(tag);
            (f.len() >= 4 && f[0] == "SUB").then(|| {
                let g = if matches!(f[3], "NOG" | "ALG") {
                    "*"
                } else {
                    f[3]
                };
                (f[1].to_owned(), f[2].to_owned(), g.to_owned())
            })
        })
        .collect()
}

fn article_form(kind: &str, r: &Reading) -> Option<&'static str> {
    let ci = CASES.iter().position(|c| *c == r.0)?;
    let g = match (r.1.as_str(), r.2.as_str()) {
        ("PLU", _) => 3,
        (_, "MAS") => 0,
        (_, "FEM") => 1,
        (_, "NEU") => 2,
        _ => return None,
    };
    const DEF: [[&str; 4]; 4] = [
        ["der", "die", "das", "die"],
        ["den", "die", "das", "die"],
        ["dem", "der", "dem", "den"],
        ["des", "der", "des", "der"],
    ];
    const IND: [[&str; 3]; 4] = [
        ["ein", "eine", "ein"],
        ["einen", "eine", "ein"],
        ["einem", "einer", "einem"],
        ["eines", "einer", "eines"],
    ];
    match kind {
        "DEF" => Some(DEF[ci][g]),
        "IND" if g < 3 => Some(IND[ci][g]),
        _ => None,
    }
}

fn capitalise(s: &str, like: &str) -> String {
    if like.chars().next().is_some_and(char::is_uppercase) {
        let mut c = s.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default()
    } else {
        s.to_owned()
    }
}

/// All agreement findings for a German analysis.
pub(crate) fn german_agreement(analysis: &Analysis) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let toks = &analysis.tokens;
    for i in 0..toks.len() {
        if let Some(d) = article_noun(toks, i) {
            out.push(d);
        }
    }
    for i in (0..toks.len()).filter(|&i| toks[i].tags.iter().any(|g| g == "SENT_START")) {
        // One sentence at a time: the slice runs to the next sentence start.
        let end = (i + 1..toks.len())
            .find(|&k| toks[k].tags.iter().any(|g| g == "SENT_START"))
            .unwrap_or(toks.len());
        if let Some(d) = subject_verb(&toks[i..end]) {
            out.push(d);
        }
    }
    out
}

fn article_noun(toks: &[Token], i: usize) -> Option<Diagnostic> {
    let (kind, art) = art_readings(&toks[i])?;
    let mut j = i + 1;
    let mut adjs: Vec<&Token> = Vec::new();
    while j < toks.len()
        && has_adj(&toks[j])
        && !is_cap(&toks[j])
        && !toks[j].tags.iter().any(|t| t.starts_with("SUB:"))
    {
        if !transparent(&toks[j], kind) {
            adjs.push(&toks[j]);
        }
        j += 1;
    }
    let noun = toks.get(j)?;
    if !is_cap(noun) || noun.text.contains('-') || noun.tags.iter().any(|t| !t.starts_with("SUB:"))
    {
        return None;
    }
    if let Some(prev) = i.checked_sub(1).map(|k| &toks[k]) {
        let p = prev.text.to_lowercase();
        // "vom einen Ende", "dieses eine Mal": the article is a numeral/adjective here.
        if CONTRACTIONS.contains(&p.as_str())
            || (art_readings(prev).is_some() || p.starts_with("dies") || p.starts_with("jen"))
                && !p.is_empty()
        {
            return None;
        }
    }
    // Multi-word names and compounds ("die Parkland High School", "Die letzte Developer Preview") and
    // hyphenated compounds ("Die CO2-Emission"): the token after the noun decides.
    if let Some(next) = toks.get(j + 1) {
        let after =
            next.text.starts_with('-') || next.text.starts_with('\'') || next.text.starts_with('’');
        let name = is_cap(next) && !next.tags.iter().any(|t| t == "SENT_START");
        if after || name {
            return None;
        }
    }
    if matches!(
        noun.text.as_str(),
        "Million" | "Millionen" | "Milliarde" | "Milliarden"
    ) {
        return None;
    }
    if pronoun_capable(&toks[i]) {
        // Relative/demonstrative pronoun: after a comma ("..., in dem Bücher"), after a noun or
        // "etwas/alles", or ("das"/"die") right after a finite verb.
        let near_comma = (1..=2).any(|d| i.checked_sub(d).is_some_and(|k| toks[k].text == ","));
        let prev = i.checked_sub(1).map(|k| &toks[k]);
        let after_noun = prev.is_some_and(|p| {
            is_cap(p)
                && p.tags.iter().any(|t| t.starts_with("SUB:"))
                && !p.tags.iter().any(|t| t == "SENT_START")
        });
        let after_verb = prev.is_some_and(|p| {
            p.tags.iter().any(|t| t.starts_with("VER:"))
                && !matches!(
                    p.text.to_lowercase().as_str(),
                    "ist" | "sind" | "war" | "waren" | "wird" | "wurde"
                )
        }) && matches!(toks[i].text.to_lowercase().as_str(), "das" | "die");
        let after_indef = prev.is_some_and(|p| {
            matches!(
                p.text.to_lowercase().as_str(),
                "etwas" | "alles" | "nichts" | "viel" | "vieles"
            )
        });
        if near_comma || after_noun || after_verb || after_indef {
            return None;
        }
        // "das" before a non-neuter noun is almost always the pronoun.
        if toks[i].text.eq_ignore_ascii_case("das")
            && !noun_readings(noun)
                .iter()
                .any(|r| r.2 == "NEU" || r.1 == "PLU")
        {
            return None;
        }
    }
    let nr = noun_readings(noun);
    if nr.is_empty() {
        return None;
    }
    // Readings the noun and every adjective agree on (independent of the article).
    let mut common: Vec<Reading> = nr;
    for a in &adjs {
        let ar = adj_readings(a, kind);
        common.retain(|c| ar.iter().any(|x| matches(x, c)));
    }
    let ok = art.iter().any(|a| common.iter().any(|c| matches(a, c)));
    if ok {
        return None;
    }
    let mut suggestions: Vec<String> = Vec::new();
    for case in CASES {
        for c in common.iter().filter(|c| c.0 == case) {
            if let Some(f) = article_form(kind, c) {
                let f = capitalise(f, &toks[i].text);
                if !suggestions.contains(&f) {
                    suggestions.push(f);
                }
            }
        }
    }
    suggestions.truncate(3);
    let (span, sugg) = if suggestions.is_empty() {
        (
            Span {
                start: toks[i].span.start,
                end: noun.span.end,
            },
            Vec::new(),
        )
    } else {
        (toks[i].span, suggestions)
    };
    Some(Diagnostic {
        span,
        code: "DE_AGREEMENT_RLT".into(),
        message: "Artikel, Adjektiv und Nomen passen in Kasus, Numerus oder Genus nicht zusammen."
            .into(),
        suggestions: sugg
            .into_iter()
            .map(|s| Suggestion { replacement: s })
            .collect(),
        source: Source::Grammar,
        details: None,
    })
}

/// Number readings of a finite verb: (person, number) pairs, or `None` unless every reading is verbal.
fn verb_readings(t: &Token) -> Option<Vec<(String, String)>> {
    if t.tags.is_empty()
        || !t
            .tags
            .iter()
            .all(|g| g.starts_with("VER:") || g.starts_with("PA2:"))
    {
        return None;
    }
    let mut out = Vec::new();
    for tag in &t.tags {
        let f = fields(tag);
        let k = if matches!(f.get(1), Some(&"AUX" | &"MOD")) {
            2
        } else {
            1
        };
        if let (Some(p), Some(n)) = (f.get(k), f.get(k + 1)) {
            if matches!(*p, "1" | "2" | "3") && matches!(*n, "SIN" | "PLU") {
                out.push(((*p).to_owned(), (*n).to_owned()));
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

fn pronoun_person(word: &str) -> Option<(&'static str, &'static str)> {
    match word {
        "ich" => Some(("1", "SIN")),
        "du" => Some(("2", "SIN")),
        "er" | "es" => Some(("3", "SIN")),
        "wir" => Some(("1", "PLU")),
        "ihr" => Some(("2", "PLU")),
        _ => None,
    }
}

/// Subject–verb agreement for a sentence-initial subject directly followed by the finite verb.
fn subject_verb(toks: &[Token]) -> Option<Diagnostic> {
    let first = toks
        .iter()
        .position(|t| !t.tags.iter().any(|g| g == "SENT_START") && !t.text.is_empty())?;
    let lower = toks[first].text.to_lowercase();
    let msg = "Subjekt und Verb stimmen in Person oder Numerus nicht überein.";
    let diag = |a: usize, b: usize| Diagnostic {
        span: Span {
            start: toks[a].span.start,
            end: toks[b].span.end,
        },
        code: "DE_SUBJECT_VERB_RLT".into(),
        message: msg.into(),
        suggestions: Vec::new(),
        source: Source::Grammar,
        details: None,
    };
    if let Some((p, n)) = pronoun_person(&lower) {
        let v = toks.get(first + 1)?;
        if is_cap(v) {
            return None;
        }
        let vr = verb_readings(v)?;
        // "Es kommen viele": existential `es` takes a plural verb.
        let ok = vr.iter().any(|(vp, vn)| {
            (vp == p && vn == n) || (lower == "es" && vp == "3") || (lower == "ihr" && vp == "3")
        });
        return (!ok).then(|| diag(first, first + 1));
    }
    // Article (+ adjectives) + noun + verb.
    let (kind, art) = art_readings(&toks[first])?;
    let mut j = first + 1;
    let mut adjs: Vec<&Token> = Vec::new();
    while j < toks.len()
        && has_adj(&toks[j])
        && !is_cap(&toks[j])
        && !toks[j].tags.iter().any(|t| t.starts_with("SUB:"))
    {
        if !transparent(&toks[j], kind) {
            adjs.push(&toks[j]);
        }
        j += 1;
    }
    let noun = toks.get(j)?;
    if !is_cap(noun) || noun.tags.iter().any(|t| !t.starts_with("SUB:")) {
        return None;
    }
    let mut common = noun_readings(noun);
    for a in &adjs {
        let ar = adj_readings(a, kind);
        common.retain(|c| ar.iter().any(|x| matches(x, c)));
    }
    let nums: Vec<&str> = common
        .iter()
        .filter(|c| c.0 == "NOM" && art.iter().any(|a| matches(a, c)))
        .map(|c| c.1.as_str())
        .collect();
    if nums.is_empty() {
        return None;
    }
    let v = toks.get(j + 1)?;
    if is_cap(v) {
        return None;
    }
    let vr = verb_readings(v)?;
    if toks.get(j + 2).is_some_and(|n| {
        matches!(
            n.text.to_lowercase().as_str(),
            "man" | "er" | "sie" | "es" | "ich" | "du" | "wir" | "ihr"
        )
    }) {
        return None;
    }
    if vr
        .iter()
        .any(|(p, n)| p == "3" && nums.contains(&n.as_str()))
    {
        return None;
    }
    // Object-fronting ("Die Ergebnisse zeigt der Autor"): a nominative word right after the verb may be the subject.
    if let Some(next) = toks.get(j + 2) {
        if next.tags.iter().any(|t| {
            let f = fields(t);
            f.contains(&"NOM") && matches!(f[0], "ART" | "SUB" | "PRO")
        }) && !next.tags.iter().any(|t| t == "SENT_END")
        {
            let nom_after = next.tags.iter().any(|t| {
                let f = fields(t);
                f.get(2) == Some(&"NOM") || f.get(1) == Some(&"NOM")
            });
            if nom_after
                && (is_cap(next)
                    || art_readings(next).is_some()
                    || pronoun_person(&next.text.to_lowercase()).is_some())
            {
                return None;
            }
        }
    }
    Some(diag(first, j + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(text: &str, start: usize, tags: &[&str]) -> Token {
        Token {
            text: text.into(),
            span: Span {
                start,
                end: start + text.len(),
            },
            tags: tags.iter().map(|s| (*s).to_owned()).collect(),
            lemmas: Vec::new(),
        }
    }

    fn analysis(words: &[(&str, &[&str])]) -> Analysis {
        let mut pos = 0;
        let mut tokens = vec![tok("", 0, &["SENT_START"])];
        for (w, tags) in words {
            tokens.push(tok(w, pos, tags));
            pos += w.len() + 1;
        }
        Analysis { tokens }
    }

    const DIE: &[&str] = &[
        "ART:DEF:NOM:SIN:FEM",
        "ART:DEF:AKK:SIN:FEM",
        "ART:DEF:NOM:PLU:MAS",
    ];
    const HAUS: &[&str] = &["SUB:NOM:SIN:NEU", "SUB:AKK:SIN:NEU", "SUB:DAT:SIN:NEU"];
    const FRAU: &[&str] = &["SUB:NOM:SIN:FEM", "SUB:AKK:SIN:FEM"];

    #[test]
    fn flags_gender_mismatch_with_article_suggestion() {
        let a = analysis(&[
            ("Ist", &["VER:3:SIN:PRÄ:NON"]),
            ("die", DIE),
            ("Haus", HAUS),
        ]);
        let d = german_agreement(&a);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "DE_AGREEMENT_RLT");
        assert_eq!(d[0].suggestions[0].replacement, "das");
    }

    #[test]
    fn accepts_matching_article() {
        let a = analysis(&[
            ("Ist", &["VER:3:SIN:PRÄ:NON"]),
            ("die", DIE),
            ("Frau", FRAU),
        ]);
        assert!(german_agreement(&a).is_empty());
    }

    #[test]
    fn skips_hyphenated_compound() {
        let a = analysis(&[("Die", DIE), ("Haus", HAUS), ("-", &["PKT"]), ("Tür", HAUS)]);
        assert!(
            german_agreement(&a)
                .iter()
                .all(|d| d.code != "DE_AGREEMENT_RLT")
        );
    }

    #[test]
    fn flags_subject_verb_mismatch_and_accepts_es_plural() {
        let wir = ["PRO:PER:NOM:PLU:ALG"];
        let hat = ["VER:3:SIN:PRÄ:NON"];
        let a = analysis(&[("Wir", &wir), ("hat", &hat)]);
        assert_eq!(german_agreement(&a)[0].code, "DE_SUBJECT_VERB_RLT");
        let es = ["PRO:PER:NOM:SIN:NEU"];
        let kommen = ["VER:3:PLU:PRÄ:NON"];
        assert!(german_agreement(&analysis(&[("Es", &es), ("kommen", &kommen)])).is_empty());
    }
}

// ---------------------------------------------------------------------------------------------
// Spanish (EAGLES tagset): determiner–adjective–noun agreement, subject–verb number, and
// subject–predicate-adjective agreement after ser/estar.
// ---------------------------------------------------------------------------------------------

/// (gender, number) with `*` for open gender (`C`, `0`) or invariable number (`N`).
type EsReading = (char, char);

fn es_gn(g: char, n: char) -> EsReading {
    let g = if matches!(g, 'M' | 'F') { g } else { '*' };
    let n = if matches!(n, 'S' | 'P') { n } else { '*' };
    (g, n)
}

fn es_match(a: EsReading, b: EsReading) -> bool {
    (a.0 == b.0 || a.0 == '*' || b.0 == '*') && (a.1 == b.1 || a.1 == '*' || b.1 == '*')
}

fn es_readings(t: &Token, prefix: &str) -> Vec<EsReading> {
    t.tags
        .iter()
        .filter(|g| g.starts_with(prefix))
        .filter_map(|g| {
            let c: Vec<char> = g.chars().collect();
            // Nouns are `NC<gender><number>000`; determiners/adjectives `DA0<g><n>0`, `AQ0<g><n>0`.
            let k = if prefix == "NC" { 2 } else { 3 };
            (c.len() > k + 1).then(|| es_gn(c[k], c[k + 1]))
        })
        .collect()
}

/// Determiner kind ("DA" definite / "DI" indefinite) readings of an article; `None` if it is not one.
fn es_det(t: &Token) -> Option<(&'static str, Vec<EsReading>)> {
    // Only real articles: other DI/DA-tagged words (todo, mucho, poco, demasiado, lo, ...) are
    // quantifiers or pronouns with their own agreement.
    if !matches!(
        t.text.to_lowercase().as_str(),
        "el" | "la" | "los" | "las" | "un" | "una" | "unos" | "unas"
    ) {
        return None;
    }
    let kind = if t.tags.iter().any(|g| g.starts_with("DA0")) {
        "DA"
    } else if t.tags.iter().any(|g| g.starts_with("DI0")) {
        "DI"
    } else {
        return None;
    };
    let r = es_readings(t, &format!("{kind}0"));
    (!r.is_empty()).then_some((kind, r))
}

fn es_form(kind: &str, r: EsReading) -> Option<&'static str> {
    match (kind, r.0, r.1) {
        ("DA", 'M', 'S') => Some("el"),
        ("DA", 'F', 'S') => Some("la"),
        ("DA", 'M', 'P') => Some("los"),
        ("DA", 'F', 'P') => Some("las"),
        ("DI", 'M', 'S') => Some("un"),
        ("DI", 'F', 'S') => Some("una"),
        ("DI", 'M', 'P') => Some("unos"),
        ("DI", 'F', 'P') => Some("unas"),
        _ => None,
    }
}

fn es_is_noun(t: &Token) -> bool {
    t.tags.iter().any(|g| g.starts_with("NC")) && !is_cap(t)
}

/// Determiner + adjectives + noun starting at `i`: (adjective count, noun index, readings both agree on).
fn es_np(
    toks: &[Token],
    i: usize,
) -> Option<(&'static str, Vec<EsReading>, usize, Vec<EsReading>)> {
    let (kind, det) = es_det(&toks[i])?;
    let mut j = i + 1;
    let mut common: Option<Vec<EsReading>> = None;
    while j < toks.len()
        && toks[j].tags.iter().any(|g| g.starts_with("AQ"))
        && (!es_is_noun(&toks[j]) || toks.get(j + 1).is_some_and(es_is_noun))
    {
        let ar = es_readings(&toks[j], "AQ0");
        common = Some(match common {
            None => ar,
            Some(c) => c
                .into_iter()
                .filter(|x| ar.iter().any(|y| es_match(*x, *y)))
                .collect(),
        });
        j += 1;
    }
    let noun = toks.get(j)?;
    if !es_is_noun(noun) || noun.text.contains('-') || noun.text.chars().count() < 2 {
        return None;
    }
    if matches!(
        noun.text.to_lowercase().as_str(),
        "mil" | "oeste" | "este" | "norte" | "sur" | "mates"
    ) {
        return None;
    }
    let nr = es_readings(noun, "NC");
    let mut all: Vec<EsReading> = nr
        .iter()
        .copied()
        .filter(|n| {
            common
                .as_ref()
                .is_none_or(|c| c.iter().any(|a| es_match(*a, *n)))
        })
        .collect();
    // "el agua", "un águila": feminine nouns with stressed initial a- take el/un.
    let low = noun.text.to_lowercase();
    if low.starts_with('a') || low.starts_with("ha") || low.starts_with('á') {
        if all.iter().any(|r| r.0 == 'F' && r.1 == 'S') {
            all.push(('M', 'S'));
        }
    }
    // Nothing both the adjectives and the noun accept: leave it to the tagger-gap caveat.
    Some((kind, det, j, all))
}

fn es_verb(t: &Token) -> Option<Vec<(char, char)>> {
    // "son" is also tagged as a noun; the copulas are unambiguous enough by lemma to allow that.
    let copula = t.lemmas.iter().any(|l| l == "ser" || l == "estar");
    if t.tags.is_empty() || !(copula || t.tags.iter().all(|g| g.starts_with('V'))) {
        return None;
    }
    let out: Vec<(char, char)> = t
        .tags
        .iter()
        .filter_map(|g| {
            let c: Vec<char> = g.chars().collect();
            (c.len() >= 6
                && matches!(c[2], 'I' | 'S' | 'M')
                && matches!(c[4], '1' | '2' | '3')
                && matches!(c[5], 'S' | 'P'))
            .then(|| (c[4], c[5]))
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

fn es_is_copula(t: &Token) -> bool {
    t.lemmas.iter().any(|l| l == "ser" || l == "estar") && es_verb(t).is_some()
}

const IMPERSONAL: &[&str] = &[
    "hace", "hay", "había", "hubo", "habrá", "creo", "supongo", "parece", "puede", "pueden", "dijo",
];

fn es_pronoun(w: &str) -> Option<(char, char)> {
    match w {
        "yo" => Some(('1', 'S')),
        "tú" => Some(('2', 'S')),
        "él" | "ella" | "usted" => Some(('3', 'S')),
        "nosotros" | "nosotras" => Some(('1', 'P')),
        "vosotros" | "vosotras" => Some(('2', 'P')),
        "ellos" | "ellas" | "ustedes" => Some(('3', 'P')),
        _ => None,
    }
}

/// All Spanish agreement findings for an analysis.
pub(crate) fn spanish_agreement(analysis: &Analysis) -> Vec<Diagnostic> {
    let toks = &analysis.tokens;
    let mut out = Vec::new();
    for i in 0..toks.len() {
        if let Some(d) = es_det_noun(toks, i) {
            out.push(d);
        }
    }
    for i in (0..toks.len()).filter(|&i| toks[i].tags.iter().any(|g| g == "SENT_START")) {
        let end = (i + 1..toks.len())
            .find(|&k| toks[k].tags.iter().any(|g| g == "SENT_START"))
            .unwrap_or(toks.len());
        if let Some(d) = es_sentence(&toks[i..end]) {
            out.push(d);
        }
    }
    out
}

fn es_det_noun(toks: &[Token], i: usize) -> Option<Diagnostic> {
    let (kind, det, j, all) = es_np(toks, i)?;
    // A preceding preposition-less determiner like "lo" / quantifier chains are not handled.
    if all.is_empty() || det.iter().any(|d| all.iter().any(|n| es_match(*d, *n))) {
        return None;
    }
    // "la/las/los" before a word that can also be a verb is often the clitic pronoun ("la saque").
    if matches!(toks[i].text.to_lowercase().as_str(), "la" | "las" | "los")
        && toks[j].tags.iter().any(|g| g.starts_with('V'))
    {
        return None;
    }
    // "cincuenta y una bombillas": numerals agree with their own counting noun.
    if i > 0
        && (matches!(toks[i - 1].text.to_lowercase().as_str(), "y" | "e")
            || toks[i - 1]
                .tags
                .iter()
                .any(|g| g == "Z" || g.starts_with("Z")))
    {
        return None;
    }
    let mut sugg: Vec<String> = Vec::new();
    for r in &all {
        if let Some(f) = es_form(kind, *r) {
            let f = capitalise(f, &toks[i].text);
            if !sugg.contains(&f) {
                sugg.push(f);
            }
        }
    }
    sugg.truncate(2);
    Some(Diagnostic {
        span: if sugg.is_empty() {
            Span {
                start: toks[i].span.start,
                end: toks[j].span.end,
            }
        } else {
            toks[i].span
        },
        code: "ES_AGREEMENT_RLT".into(),
        message: "El determinante, el adjetivo y el sustantivo no concuerdan en género o número."
            .into(),
        suggestions: sugg
            .into_iter()
            .map(|s| Suggestion { replacement: s })
            .collect(),
        source: Source::Grammar,
        details: None,
    })
}

fn es_sentence(toks: &[Token]) -> Option<Diagnostic> {
    let first = 1.min(toks.len().saturating_sub(1));
    let t0 = toks.get(first)?;
    let mk = |a: usize, b: usize, code: &str, msg: &str| Diagnostic {
        span: Span {
            start: toks[a].span.start,
            end: toks[b].span.end,
        },
        code: code.into(),
        message: msg.into(),
        suggestions: Vec::new(),
        source: Source::Grammar,
        details: None,
    };
    const SV: &str = "El sujeto y el verbo no concuerdan en número o persona.";
    if let Some((p, n)) = es_pronoun(&t0.text.to_lowercase()) {
        let v = toks.get(first + 1)?;
        let vr = es_verb(v)?;
        if IMPERSONAL.contains(&v.text.to_lowercase().as_str()) {
            return None;
        }
        return (!vr.contains(&(p, n))).then(|| mk(first, first + 1, "ES_SUBJECT_VERB_RLT", SV));
    }
    let (_, det, j, all) = es_np(toks, first)?;
    // Time expressions are adverbials, not subjects ("Una vez quise", "La próxima semana abren").
    if matches!(
        toks[j].text.to_lowercase().as_str(),
        "vez"
            | "veces"
            | "semana"
            | "semanas"
            | "mañana"
            | "tarde"
            | "noche"
            | "día"
            | "días"
            | "mes"
            | "meses"
            | "año"
            | "años"
            | "momento"
            | "rato"
            | "hora"
            | "horas"
    ) {
        return None;
    }
    let nums: Vec<char> = {
        let mut v: Vec<char> = all
            .iter()
            .filter(|n| det.iter().any(|d| es_match(*d, **n)))
            .map(|n| n.1)
            .filter(|c| *c != '*')
            .collect();
        v.dedup();
        v
    };
    if nums.len() != 1 {
        return None;
    }
    let num = nums[0];
    let v = toks.get(j + 1)?;
    let vr = es_verb(v)?;
    // Impersonal/parenthetical verbs and "El resto son …" (the verb agrees with the predicate).
    if IMPERSONAL.contains(&v.text.to_lowercase().as_str()) || (es_is_copula(v) && num == 'S') {
        return None;
    }
    if !vr.iter().any(|(p, n)| *p == '3' && *n == num) {
        return Some(mk(first, j + 1, "ES_SUBJECT_VERB_RLT", SV));
    }
    // Subject + ser/estar + (adverbs) + adjective that disagrees with the subject.
    if es_is_copula(v) {
        let mut k = j + 2;
        while toks
            .get(k)
            .is_some_and(|t| t.tags.iter().all(|g| g.starts_with("RG")))
            && !toks[k].tags.is_empty()
        {
            k += 1;
        }
        let adj = toks.get(k)?;
        if !adj
            .tags
            .iter()
            .all(|g| g.starts_with("AQ") || g.starts_with("_GN"))
            || es_readings(adj, "AQ0").is_empty()
        {
            return None;
        }
        let ar = es_readings(adj, "AQ0");
        // Only when the adjective ends the clause (or is coordinated with another adjective);
        // "hermosas y elegantes aves" continues into a noun phrase.
        let after = toks.get(k + 1);
        let clause_end =
            after.is_none_or(|n| n.tags.iter().any(|g| g == "SENT_END" || g == "_PUNCT"));
        let coord_adj = after.is_some_and(|n| n.tags.iter().any(|g| g == "CC"))
            && toks
                .get(k + 2)
                .is_some_and(|n| n.tags.iter().any(|g| g.starts_with("AQ")) && !es_is_noun(n));
        if !(clause_end || coord_adj) {
            return None;
        }
        let subj: Vec<EsReading> = all
            .iter()
            .copied()
            .filter(|n| det.iter().any(|d| es_match(*d, *n)))
            .collect();
        if !subj.iter().any(|s| ar.iter().any(|a| es_match(*s, *a))) {
            return Some(mk(
                first,
                k,
                "ES_AGREEMENT_RLT",
                "El adjetivo no concuerda con el sujeto en género o número.",
            ));
        }
    }
    None
}

#[cfg(test)]
mod es_tests {
    use super::*;

    fn tok(text: &str, start: usize, tags: &[&str], lemmas: &[&str]) -> Token {
        Token {
            text: text.into(),
            span: Span {
                start,
                end: start + text.len(),
            },
            tags: tags.iter().map(|s| (*s).to_owned()).collect(),
            lemmas: lemmas.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn analysis(words: &[(&str, &[&str], &[&str])]) -> Analysis {
        let mut pos = 0;
        let mut tokens = vec![tok("", 0, &["SENT_START"], &[])];
        for (w, t, l) in words {
            tokens.push(tok(w, pos, t, l));
            pos += w.len() + 1;
        }
        Analysis { tokens }
    }

    #[test]
    fn flags_article_gender_and_offers_form() {
        let a = analysis(&[
            ("La", &["DA0FS0"], &["el"]),
            ("problema", &["NCMS000"], &["problema"]),
        ]);
        let d = spanish_agreement(&a);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].suggestions[0].replacement, "El");
    }

    #[test]
    fn accepts_el_agua_and_matching_pairs() {
        let a = analysis(&[
            ("El", &["DA0MS0"], &["el"]),
            ("agua", &["NCFS000"], &["agua"]),
        ]);
        assert!(spanish_agreement(&a).is_empty());
        let b = analysis(&[
            ("Las", &["DA0FP0"], &["el"]),
            ("casas", &["NCFP000"], &["casa"]),
        ]);
        assert!(spanish_agreement(&b).is_empty());
    }

    #[test]
    fn flags_subject_verb_number() {
        let a = analysis(&[
            ("Los", &["DA0MP0"], &["el"]),
            ("resultados", &["NCMP000"], &["resultado"]),
            ("es", &["VSIP3S0"], &["ser"]),
        ]);
        assert_eq!(spanish_agreement(&a)[0].code, "ES_SUBJECT_VERB_RLT");
    }
}

// ---------------------------------------------------------------------------------------------
// English (Penn tagset): a/an and subject–verb agreement. LanguageTool does these in Java.
// ---------------------------------------------------------------------------------------------

/// Words whose first letter is a vowel but that start with a consonant sound ("a university").
const EN_CONSONANT_SOUND: &[&str] = &[
    "univers", "uniq", "unit", "union", "unif", "unic", "unilat", "unisex", "unipol", "unix",
    "use", "usu", "usa", "uti", "util", "eu", "one", "once", "ubiq", "ura", "uro", "ufo", "u-",
    "ewe", "euro",
];
/// Words starting with a silent h ("an hour").
const EN_SILENT_H: &[&str] = &["hour", "honest", "honor", "honour", "heir"];

fn en_starts_with_vowel_sound(word: &str) -> Option<bool> {
    let first = word.chars().next()?;
    if !first.is_alphabetic() {
        return None;
    }
    // Acronyms ("an MRI") and emphasis in capitals ("a LOT") cannot be told apart here: stay silent.
    let letters = word.chars().take_while(|c| c.is_alphabetic()).count();
    if letters < 2
        || word
            .chars()
            .take_while(|c| c.is_alphabetic())
            .all(char::is_uppercase)
    {
        return None;
    }
    let w = word.to_lowercase();
    if EN_SILENT_H.iter().any(|h| w.starts_with(h)) {
        return Some(true);
    }
    if EN_CONSONANT_SOUND.iter().any(|p| w.starts_with(p)) {
        return Some(false);
    }
    Some(matches!(w.chars().next()?, 'a' | 'e' | 'i' | 'o' | 'u'))
}

fn en_has(t: &Token, tag: &str) -> bool {
    t.tags.iter().any(|g| g == tag)
}

fn en_only_verbal_plural(t: &Token) -> bool {
    // Readings that are only base/non-3rd-person present forms ("go", "have").
    t.tags.iter().any(|g| g == "VBP" || g == "VB")
        && !t
            .tags
            .iter()
            .any(|g| matches!(g.as_str(), "VBZ" | "VBD" | "MD" | "NNS" | "VBN"))
}

/// All English findings: a/an everywhere, subject–verb at sentence start.
pub(crate) fn english_agreement(analysis: &Analysis) -> Vec<Diagnostic> {
    let toks = &analysis.tokens;
    let mut out = Vec::new();
    for i in 0..toks.len().saturating_sub(1) {
        let a = toks[i].text.as_str();
        if !(a.eq_ignore_ascii_case("a") || a.eq_ignore_ascii_case("an")) {
            continue;
        }
        // An article directly before a word (not a variable "a" in math or a list item).
        let next = &toks[i + 1];
        if next.span.start != toks[i].span.end + 1 {
            continue;
        }
        // The article must be a plain determiner (not a variable "a", grade "A", "Series A"), spelled
        // in lower case or opening the sentence, before a noun/adjective.
        let art = &toks[i];
        let sentence_start = i > 0 && en_has(&toks[i - 1], "SENT_START");
        let plain_det = art.tags.iter().any(|g| g == "DT")
            && !art.tags.iter().any(|g| g.starts_with("NN") || g == "FW");
        let before_noun = next.tags.iter().any(|g| {
            matches!(
                g.as_str(),
                "NN" | "NN:UN" | "NNS" | "JJ" | "JJR" | "JJS" | "NNP"
            )
        }) && !next.tags.iter().any(|g| {
            matches!(
                g.as_str(),
                "CC" | "IN" | "VBP" | "VBZ" | "MD" | "VBD" | "RB"
            )
        });
        if !plain_det
            || !before_noun
            || (art.text.starts_with(char::is_uppercase) && !sentence_start)
        {
            continue;
        }
        if toks
            .get(i + 2)
            .is_some_and(|t| t.text.starts_with('-') && t.span.start == next.span.end)
        {
            continue;
        }
        let Some(vowel) = en_starts_with_vowel_sound(&next.text) else {
            continue;
        };
        let want = if vowel { "an" } else { "a" };
        if a.eq_ignore_ascii_case(want) {
            continue;
        }
        out.push(Diagnostic {
            span: toks[i].span,
            code: "EN_A_VS_AN_RLT".into(),
            message: format!(
                "Use \"{want}\" before a word that starts with a {} sound.",
                if vowel { "vowel" } else { "consonant" }
            ),
            suggestions: vec![Suggestion {
                replacement: capitalise(want, a),
            }],
            source: Source::Grammar,
            details: None,
        });
    }
    for i in (0..toks.len()).filter(|&i| en_has(&toks[i], "SENT_START")) {
        let end = (i + 1..toks.len())
            .find(|&k| en_has(&toks[k], "SENT_START"))
            .unwrap_or(toks.len());
        if let Some(d) = en_subject_verb(&toks[i..end]) {
            out.push(d);
        }
    }
    out
}

fn en_subject_verb(toks: &[Token]) -> Option<Diagnostic> {
    let first = 1.min(toks.len().checked_sub(1)?);
    let s = toks.get(first)?;
    let sl = s.text.to_lowercase();
    let v = toks.get(first + 1)?;
    let vl = v.text.to_lowercase();
    if is_cap(v) {
        return None;
    }
    let singular = matches!(sl.as_str(), "he" | "she" | "it");
    let plural = matches!(sl.as_str(), "we" | "they" | "you");
    let bad = if singular {
        matches!(vl.as_str(), "are" | "were" | "have" | "do" | "don't" | "am")
            || (en_only_verbal_plural(v)
                && v.tags
                    .iter()
                    .all(|g| matches!(g.as_str(), "VB" | "VBP" | "NN" | "NN:UN"))
                && !matches!(vl.as_str(), "be" | "have" | "do")
                && !toks.get(first + 2).is_some_and(|n| {
                    n.text.starts_with('-')
                        || n.tags
                            .iter()
                            .any(|g| matches!(g.as_str(), "VBD" | "VBN" | "VBG"))
                }))
    } else if plural {
        matches!(
            vl.as_str(),
            "is" | "was" | "has" | "does" | "doesn't" | "am"
        ) || (en_has(v, "VBZ")
            && !v
                .tags
                .iter()
                .any(|g| matches!(g.as_str(), "VBP" | "VB" | "NN" | "NNS" | "VBD" | "MD")))
    } else if sl == "i" {
        matches!(vl.as_str(), "is" | "has" | "does" | "doesn't" | "are")
    } else {
        // Plural noun phrase "These results" + singular be/have, or "The results of the study is".
        let (np_start, noun_idx) = en_np(toks, first)?;
        let _ = np_start;
        let noun = &toks[noun_idx];
        let mut verb_idx = noun_idx + 1;
        let nl = noun.text.to_lowercase();
        let singular_s = nl.ends_with("ics")
            || nl.ends_with("works")
            || nl.ends_with("ss")
            || matches!(
                nl.as_str(),
                "news"
                    | "series"
                    | "species"
                    | "means"
                    | "headquarters"
                    | "barracks"
                    | "crossroads"
            );
        let plural_noun = noun.tags.iter().any(|g| g == "NNS")
            && !noun.tags.iter().any(|g| g == "NN" || g == "NN:UN")
            && !singular_s;
        // "The results of the study is clear": skip the "of …" modifier.
        if plural_noun
            && toks
                .get(verb_idx)
                .is_some_and(|t| t.text.eq_ignore_ascii_case("of"))
        {
            let mut k = verb_idx + 1;
            while k < toks.len()
                && !en_has(&toks[k], "VBZ")
                && !en_has(&toks[k], "VBD")
                && k < verb_idx + 5
            {
                if toks[k].text.eq_ignore_ascii_case("is")
                    || toks[k].text.eq_ignore_ascii_case("was")
                {
                    break;
                }
                k += 1;
            }
            verb_idx = k;
        }
        let v2 = toks.get(verb_idx)?;
        let v2l = v2.text.to_lowercase();
        let ok_plural =
            plural_noun && matches!(v2l.as_str(), "is" | "was" | "has" | "does" | "doesn't");
        return ok_plural.then(|| Diagnostic {
            span: v2.span,
            code: "EN_SUBJECT_VERB_RLT".into(),
            message: "The subject is plural but the verb is singular.".into(),
            suggestions: match v2l.as_str() {
                "is" => vec!["are"],
                "was" => vec!["were"],
                "has" => vec!["have"],
                "does" => vec!["do"],
                _ => vec!["don't"],
            }
            .into_iter()
            .map(|r| Suggestion {
                replacement: capitalise(r, &v2.text),
            })
            .collect(),
            source: Source::Grammar,
            details: None,
        });
    };
    bad.then(|| Diagnostic {
        span: Span {
            start: s.span.start,
            end: v.span.end,
        },
        code: "EN_SUBJECT_VERB_RLT".into(),
        message: "The subject and the verb do not agree in number or person.".into(),
        suggestions: Vec::new(),
        source: Source::Grammar,
        details: None,
    })
}

/// Determiner (these/those/the/…) + adjectives + noun at `i`: (start, noun index).
fn en_np(toks: &[Token], i: usize) -> Option<(usize, usize)> {
    let d = toks.get(i)?;
    if !matches!(
        d.text.to_lowercase().as_str(),
        "the" | "these" | "those" | "many" | "several" | "both"
    ) {
        return None;
    }
    let mut j = i + 1;
    while toks
        .get(j)
        .is_some_and(|t| en_has(t, "JJ") && !en_has(t, "NNS") && !en_has(t, "NN"))
    {
        j += 1;
    }
    let n = toks.get(j)?;
    (n.tags.iter().any(|g| g.starts_with("NN")) && !is_cap(n)).then_some((i, j))
}

#[cfg(test)]
mod en_tests {
    use super::*;

    fn tok(text: &str, start: usize, tags: &[&str]) -> Token {
        Token {
            text: text.into(),
            span: Span {
                start,
                end: start + text.len(),
            },
            tags: tags.iter().map(|s| (*s).to_owned()).collect(),
            lemmas: Vec::new(),
        }
    }

    fn analysis(words: &[(&str, &[&str])]) -> Analysis {
        let mut pos = 0;
        let mut tokens = vec![tok("", 0, &["SENT_START"])];
        for (w, t) in words {
            tokens.push(tok(w, pos, t));
            pos += w.len() + 1;
        }
        Analysis { tokens }
    }

    #[test]
    fn a_an_before_vowel_and_consonant_sounds() {
        let bad = analysis(&[("a", &["DT"]), ("apple", &["NN"])]);
        assert_eq!(english_agreement(&bad)[0].suggestions[0].replacement, "an");
        let uni = analysis(&[("an", &["DT"]), ("university", &["NN"])]);
        assert_eq!(english_agreement(&uni)[0].suggestions[0].replacement, "a");
        let ok = analysis(&[("an", &["DT"]), ("hour", &["NN"])]);
        assert!(english_agreement(&ok).is_empty());
        let var = analysis(&[("a", &["NN"]), ("and", &["CC"])]);
        assert!(english_agreement(&var).is_empty());
    }

    #[test]
    fn flags_pronoun_verb_mismatch() {
        let a = analysis(&[("He", &["PRP"]), ("go", &["NN", "VB", "VBP"])]);
        assert_eq!(english_agreement(&a)[0].code, "EN_SUBJECT_VERB_RLT");
        let ok = analysis(&[("He", &["PRP"]), ("goes", &["VBZ"])]);
        assert!(english_agreement(&ok).is_empty());
    }
}

// ---------------------------------------------------------------------------------------------
// German capitalisation (implemented from the observable behaviour of the rule, not from LT's Java):
// capitalised non-nouns inside a sentence ("Wir haben Gestern gearbeitet"). Lowercase nouns that are not
// valid lowercase words ("der hund") are reported by the spell checker, and "beim essen" by an XML rule.
// ---------------------------------------------------------------------------------------------

/// Capitalised words that are legitimate inside a sentence although they carry no noun reading.
const DE_ALWAYS_CAP: &[&str] = &[
    "sie", "ihr", "ihnen", "ihre", "ihrer", "ihrem", "ihren", "ihres", "du", "dich", "dir", "dein",
    "deine",
];

/// Words after which a capitalised adjective/verb is a nominalisation ("das Gehen", "im Folgenden").
fn nominalising_context(prev: &Token) -> bool {
    let p = prev.text.to_lowercase();
    prev.tags.iter().any(|g| {
        g.starts_with("ART:")
            || g.starts_with("PRP")
            || g.starts_with("PRO:POS")
            || g.starts_with("PRO:IND")
            || g.starts_with("PRO:DEM")
            || g.starts_with("ADJ:")
            || g.starts_with("ZAL")
            || g == "KON:UNT"
    }) || matches!(
        p.as_str(),
        "beim"
            | "im"
            | "zum"
            | "zur"
            | "vom"
            | "am"
            | "ans"
            | "ins"
            | "aufs"
            | "etwas"
            | "nichts"
            | "alles"
            | "viel"
            | "wenig"
            | "allerlei"
            | "das"
            | "dem"
            | "des"
            | "ein"
            | "eine"
            | "einem"
            | "einen"
            | "einer"
            | "eines"
            | "mein"
            | "dein"
            | "sein"
            | "unser"
            | "euer"
            | "und"
            | "oder"
            | "sowie"
            | "als"
            | "wie"
            | "bis"
            | "für"
            | "mit"
            | "von"
            | "zu"
            | "in"
            | "an"
            | "auf"
            | "aus"
            | "bei"
            | "nach"
            | "über"
            | "unter"
            | "um"
            | "durch"
            | "gegen"
            | "ohne"
            | "vor"
            | "hinter"
            | "neben"
            | "zwischen"
            | "statt"
            | "trotz"
            | "wegen"
            | "während"
            | "dank"
    )
}

pub(crate) fn german_case(analysis: &Analysis) -> Vec<Diagnostic> {
    let toks = &analysis.tokens;
    let mut out = Vec::new();
    for i in 1..toks.len() {
        let t = &toks[i];
        let prev = &toks[i - 1];
        if !t.text.chars().any(char::is_alphabetic) {
            continue;
        }
        let first_upper = t.text.chars().next().is_some_and(char::is_uppercase);
        let lower = t.text.to_lowercase();
        if first_upper {
            // Capitalised word inside a sentence without any noun/name reading.
            if en_has(prev, "SENT_START")
                || prev.text.chars().all(|c| !c.is_alphanumeric())
                || t.text.chars().count() < 2
                || t.text.chars().all(char::is_uppercase)
                || t.text.contains('-')
                || DE_ALWAYS_CAP.contains(&lower.as_str())
                || nominalising_context(prev)
                || t.tags.is_empty()
                // The tagger guesses unknown (compound) nouns as verbs and uses the capitalised
                // surface as lemma: not a real verb/adverb.
                || t.lemmas.iter().any(|l| l.chars().next().is_some_and(char::is_uppercase))
            {
                continue;
            }
            let non_noun = t.tags.iter().all(|g| {
                // Adjectives are left out on purpose: nominalised adjectives ("Neues", "Italienisch") and
                // names ("Lahm") are capitalised legitimately.
                g.starts_with("ADV")
                    || g.starts_with("VER:")
                    || g.starts_with("PA2")
                    || g.starts_with("KON")
                    || g.starts_with("PRP")
            });
            let after_cap_word = is_cap(prev) && !en_has(prev, "SENT_START");
            // The following word must not make it part of a name or compound ("Gesagt Getan").
            let next_cap = toks
                .get(i + 1)
                .is_some_and(|n| is_cap(n) && !en_has(n, "SENT_START"));
            if !non_noun || next_cap || after_cap_word {
                continue;
            }
            out.push(Diagnostic {
                span: t.span,
                code: "DE_CASE_RLT".into(),
                message: "Dieses Wort wird hier kleingeschrieben.".into(),
                suggestions: vec![Suggestion { replacement: lower }],
                source: Source::Grammar,
                details: None,
            });
        }
    }
    out
}
