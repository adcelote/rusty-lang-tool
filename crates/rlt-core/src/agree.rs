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
    "vom", "zum", "zur", "im", "am", "beim", "ins", "ans", "aufs", "durchs", "fürs", "ums", "übers", "unters", "vors",
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
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
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
    while j < toks.len() && has_adj(&toks[j]) && !is_cap(&toks[j]) && !toks[j].tags.iter().any(|t| t.starts_with("SUB:")) {
        if !transparent(&toks[j], kind) {
            adjs.push(&toks[j]);
        }
        j += 1;
    }
    let noun = toks.get(j)?;
    if !is_cap(noun) || noun.text.contains('-') || noun.tags.iter().any(|t| !t.starts_with("SUB:")) {
        return None;
    }
    if let Some(prev) = i.checked_sub(1).map(|k| &toks[k]) {
        let p = prev.text.to_lowercase();
        // "vom einen Ende", "dieses eine Mal": the article is a numeral/adjective here.
        if CONTRACTIONS.contains(&p.as_str()) || (art_readings(prev).is_some() || p.starts_with("dies") || p.starts_with("jen")) && !p.is_empty() {
            return None;
        }
    }
    // Multi-word names and compounds ("die Parkland High School", "Die letzte Developer Preview") and
    // hyphenated compounds ("Die CO2-Emission"): the token after the noun decides.
    if let Some(next) = toks.get(j + 1) {
        let after = next.text.starts_with('-') || next.text.starts_with('\'') || next.text.starts_with('’');
        let name = is_cap(next) && !next.tags.iter().any(|t| t == "SENT_START");
        if after || name {
            return None;
        }
    }
    if matches!(noun.text.as_str(), "Million" | "Millionen" | "Milliarde" | "Milliarden") {
        return None;
    }
    if pronoun_capable(&toks[i]) {
        // Relative/demonstrative pronoun: after a comma ("..., in dem Bücher"), after a noun or
        // "etwas/alles", or ("das"/"die") right after a finite verb.
        let near_comma = (1..=2).any(|d| i.checked_sub(d).is_some_and(|k| toks[k].text == ","));
        let prev = i.checked_sub(1).map(|k| &toks[k]);
        let after_noun = prev.is_some_and(|p| is_cap(p) && p.tags.iter().any(|t| t.starts_with("SUB:")) && !p.tags.iter().any(|t| t == "SENT_START"));
        let after_verb = prev.is_some_and(|p| {
            p.tags.iter().any(|t| t.starts_with("VER:"))
                && !matches!(p.text.to_lowercase().as_str(), "ist" | "sind" | "war" | "waren" | "wird" | "wurde")
        })
            && matches!(toks[i].text.to_lowercase().as_str(), "das" | "die");
        let after_indef = prev.is_some_and(|p| matches!(p.text.to_lowercase().as_str(), "etwas" | "alles" | "nichts" | "viel" | "vieles"));
        if near_comma || after_noun || after_verb || after_indef {
            return None;
        }
        // "das" before a non-neuter noun is almost always the pronoun.
        if toks[i].text.eq_ignore_ascii_case("das") && !noun_readings(noun).iter().any(|r| r.2 == "NEU" || r.1 == "PLU") {
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
            Span { start: toks[i].span.start, end: noun.span.end },
            Vec::new(),
        )
    } else {
        (toks[i].span, suggestions)
    };
    Some(Diagnostic {
        span,
        code: "DE_AGREEMENT_RLT".into(),
        message: "Artikel, Adjektiv und Nomen passen in Kasus, Numerus oder Genus nicht zusammen.".into(),
        suggestions: sugg.into_iter().map(|s| Suggestion { replacement: s }).collect(),
        source: Source::Grammar,
        details: None,
    })
}

/// Number readings of a finite verb: (person, number) pairs, or `None` unless every reading is verbal.
fn verb_readings(t: &Token) -> Option<Vec<(String, String)>> {
    if t.tags.is_empty() || !t.tags.iter().all(|g| g.starts_with("VER:") || g.starts_with("PA2:")) {
        return None;
    }
    let mut out = Vec::new();
    for tag in &t.tags {
        let f = fields(tag);
        let k = if matches!(f.get(1), Some(&"AUX" | &"MOD")) { 2 } else { 1 };
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
    let first = toks.iter().position(|t| !t.tags.iter().any(|g| g == "SENT_START") && !t.text.is_empty())?;
    let lower = toks[first].text.to_lowercase();
    let msg = "Subjekt und Verb stimmen in Person oder Numerus nicht überein.";
    let diag = |a: usize, b: usize| Diagnostic {
        span: Span { start: toks[a].span.start, end: toks[b].span.end },
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
        let ok = vr.iter().any(|(vp, vn)| (vp == p && vn == n) || (lower == "es" && vp == "3") || (lower == "ihr" && vp == "3"));
        return (!ok).then(|| diag(first, first + 1));
    }
    // Article (+ adjectives) + noun + verb.
    let (kind, art) = art_readings(&toks[first])?;
    let mut j = first + 1;
    let mut adjs: Vec<&Token> = Vec::new();
    while j < toks.len() && has_adj(&toks[j]) && !is_cap(&toks[j]) && !toks[j].tags.iter().any(|t| t.starts_with("SUB:")) {
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
    if toks.get(j + 2).is_some_and(|n| matches!(n.text.to_lowercase().as_str(), "man" | "er" | "sie" | "es" | "ich" | "du" | "wir" | "ihr")) {
        return None;
    }
    if vr.iter().any(|(p, n)| p == "3" && nums.contains(&n.as_str())) {
        return None;
    }
    // Object-fronting ("Die Ergebnisse zeigt der Autor"): a nominative word right after the verb may be the subject.
    if let Some(next) = toks.get(j + 2) {
        if next.tags.iter().any(|t| {
            let f = fields(t);
            f.contains(&"NOM") && matches!(f[0], "ART" | "SUB" | "PRO")
        }) && !next.tags.iter().any(|t| t == "SENT_END") {
            let nom_after = next.tags.iter().any(|t| {
                let f = fields(t);
                f.get(2) == Some(&"NOM") || f.get(1) == Some(&"NOM")
            });
            if nom_after && (is_cap(next) || art_readings(next).is_some() || pronoun_person(&next.text.to_lowercase()).is_some()) {
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
            span: Span { start, end: start + text.len() },
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

    const DIE: &[&str] = &["ART:DEF:NOM:SIN:FEM", "ART:DEF:AKK:SIN:FEM", "ART:DEF:NOM:PLU:MAS"];
    const HAUS: &[&str] = &["SUB:NOM:SIN:NEU", "SUB:AKK:SIN:NEU", "SUB:DAT:SIN:NEU"];
    const FRAU: &[&str] = &["SUB:NOM:SIN:FEM", "SUB:AKK:SIN:FEM"];

    #[test]
    fn flags_gender_mismatch_with_article_suggestion() {
        let a = analysis(&[("Ist", &["VER:3:SIN:PRÄ:NON"]), ("die", DIE), ("Haus", HAUS)]);
        let d = german_agreement(&a);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "DE_AGREEMENT_RLT");
        assert_eq!(d[0].suggestions[0].replacement, "das");
    }

    #[test]
    fn accepts_matching_article() {
        let a = analysis(&[("Ist", &["VER:3:SIN:PRÄ:NON"]), ("die", DIE), ("Frau", FRAU)]);
        assert!(german_agreement(&a).is_empty());
    }

    #[test]
    fn skips_hyphenated_compound() {
        let a = analysis(&[("Die", DIE), ("Haus", HAUS), ("-", &["PKT"]), ("Tür", HAUS)]);
        assert!(german_agreement(&a).iter().all(|d| d.code != "DE_AGREEMENT_RLT"));
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
