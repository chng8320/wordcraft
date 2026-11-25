//! Review › Proofing › Readability Statistics: how hard the document (or the selection) is to
//! read. Counts (sentences, words, syllables, characters), the averages, and the published
//! readability formulas that fit its language: Flesch Reading Ease and Flesch–Kincaid grade,
//! Gunning fog, SMOG, Coleman–Liau, ARI and LIX for English; the language's own Flesch variant
//! for Spanish, French, German and Italian; LIX and ARI, which need no syllables, for other
//! alphabetic languages. Vietnamese writes one syllable per word, so it gets the counts and
//! sentence length only.

use super::{CmdResult, Session, Value, json, p};

/// The counts readability formulas start from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextStats {
    pub sentences: usize,
    pub words: usize,
    pub syllables: usize,
    /// Letters and digits in words.
    pub letters: usize,
    /// Words of three or more syllables (Gunning fog, SMOG).
    pub polysyllables: usize,
    /// Words of more than six letters (LIX).
    pub long_words: usize,
    pub paragraphs: usize,
}

/// A language the formulas know: its subtag, as written in the document or given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    English,
    Spanish,
    French,
    German,
    Italian,
    Portuguese,
    Dutch,
    Vietnamese,
    /// Any other language written with spaces between words.
    Other,
}

impl Lang {
    pub fn from_tag(tag: &str) -> Lang {
        match tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase().as_str() {
            "en" => Lang::English,
            "es" => Lang::Spanish,
            "fr" => Lang::French,
            "de" => Lang::German,
            "it" => Lang::Italian,
            "pt" => Lang::Portuguese,
            "nl" => Lang::Dutch,
            "vi" => Lang::Vietnamese,
            _ => Lang::Other,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::Spanish => "es",
            Lang::French => "fr",
            Lang::German => "de",
            Lang::Italian => "it",
            Lang::Portuguese => "pt",
            Lang::Dutch => "nl",
            Lang::Vietnamese => "vi",
            Lang::Other => "other",
        }
    }
}

/// Letters only Vietnamese uses among the Latin alphabets: enough to tell it from the others.
fn looks_vietnamese(text: &str) -> bool {
    let marks = text.chars().filter(|c| "ăâđêôơưĂÂĐÊÔƠƯạảấầẩẫậắằẳẵặẹẻẽếềểễệỉịọỏốồổỗộớờởỡợụủứừửữựỳỵỷỹ".contains(*c)).count();
    marks * 40 >= text.split_whitespace().count().max(1)
}

fn is_vowel(c: char, lang: Lang) -> bool {
    let base = match c.to_lowercase().next().unwrap_or(c) {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => 'a',
        'è' | 'é' | 'ê' | 'ë' | 'ē' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'æ' | 'œ' => 'e',
        x => x,
    };
    matches!(base, 'a' | 'e' | 'i' | 'o' | 'u') || (base == 'y' && lang != Lang::Spanish)
}

/// Syllables in one word: groups of vowels, with English's silent final e and -le, and at least
/// one per word. An estimate, as every readability tool's is; it is right for most words.
pub fn syllables(word: &str, lang: Lang) -> usize {
    if lang == Lang::Vietnamese {
        return 1;
    }
    let w: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).flat_map(char::to_lowercase).collect();
    if w.is_empty() {
        return 0;
    }
    let mut groups = 0;
    let mut prev = false;
    for &c in &w {
        let v = is_vowel(c, lang);
        if v && !prev {
            groups += 1;
        }
        prev = v;
    }
    if lang == Lang::English && w.len() > 2 {
        let n = w.len();
        let ends = |s: &str| w.iter().rev().take(s.len()).rev().copied().eq(s.chars());
        // a final silent e ("make"), but not "-le" after a consonant ("table"), "-ee" or "-ye"
        if w[n - 1] == 'e' && !is_vowel(w[n - 2], lang) && !(ends("le") && !is_vowel(w[n - 3], lang)) {
            groups = groups.saturating_sub(1);
        }
        // "-es"/"-ed" that add no sound ("makes", "liked"), not after t, d, s, z, x, ch, sh
        if (ends("es") || ends("ed")) && n > 3 && !is_vowel(w[n - 3], lang) && !matches!(w[n - 3], 't' | 'd' | 's' | 'z' | 'x' | 'h') {
            groups = groups.saturating_sub(1);
        }
    }
    groups.max(1)
}

/// The counts for `text`: sentences end at . ! ? … (and their full-width forms), not inside
/// numbers like 3.14; words are runs with a letter or digit.
pub fn text_stats(text: &str, lang: Lang) -> TextStats {
    let mut st = TextStats { paragraphs: text.split('\n').filter(|l| !l.trim().is_empty()).count(), ..TextStats::default() };
    for para in text.split('\n').filter(|l| !l.trim().is_empty()) {
        let mut open = false;
        let chars: Vec<char> = para.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            let end = matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？');
            let in_number = c == '.' && i > 0 && chars[i - 1].is_ascii_digit() && chars.get(i + 1).is_some_and(char::is_ascii_digit);
            if end && !in_number {
                if open {
                    st.sentences += 1;
                    open = false;
                }
            } else if c.is_alphanumeric() {
                open = true;
            }
        }
        if open {
            st.sentences += 1;
        }
        for word in para.split_whitespace() {
            let letters = word.chars().filter(|c| c.is_alphanumeric()).count();
            if letters == 0 {
                continue;
            }
            let syl = if word.chars().any(char::is_alphabetic) { syllables(word, lang) } else { 1 };
            st.words += 1;
            st.letters += letters;
            st.syllables += syl;
            if syl >= 3 {
                st.polysyllables += 1;
            }
            if letters > 6 {
                st.long_words += 1;
            }
        }
    }
    st
}

/// The sentences of `text`, split where [`text_stats`] counts them.
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n').filter(|l| !l.trim().is_empty()) {
        let chars: Vec<char> = para.chars().collect();
        let mut cur = String::new();
        for (i, &c) in chars.iter().enumerate() {
            cur.push(c);
            let end = matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？');
            let in_number = c == '.' && i > 0 && chars[i - 1].is_ascii_digit() && chars.get(i + 1).is_some_and(char::is_ascii_digit);
            if end && !in_number && cur.chars().any(char::is_alphanumeric) {
                out.push(cur.trim().to_string());
                cur.clear();
            }
        }
        if cur.chars().any(char::is_alphanumeric) {
            out.push(cur.trim().to_string());
        }
    }
    out
}

/// The hardest sentences of four words or more, hardest first: by the Flesch–Kincaid grade of
/// each, or by length where syllables mean nothing (Vietnamese).
pub fn hardest_sentences(text: &str, lang: Lang, n: usize) -> Vec<String> {
    let mut scored: Vec<(f64, String)> = split_sentences(text)
        .into_iter()
        .filter_map(|t| {
            let st = text_stats(&t, lang);
            (st.words >= 4).then(|| {
                let w = st.words as f64;
                let k = if lang == Lang::Vietnamese { w } else { 0.39 * w + 11.8 * st.syllables as f64 / w };
                (k, t)
            })
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(n).map(|(_, t)| t).collect()
}

fn round(x: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    (x * k).round() / k
}

/// The scores that fit `lang`, each with what it means; none when there is too little text.
pub fn scores(st: &TextStats, lang: Lang) -> Value {
    if st.words == 0 || st.sentences == 0 {
        return json!({});
    }
    let (w, s) = (st.words as f64, st.sentences as f64);
    let asl = w / s; // words per sentence
    let asw = st.syllables as f64 / w; // syllables per word
    let lix = asl + 100.0 * st.long_words as f64 / w;
    let ari = 4.71 * (st.letters as f64 / w) + 0.5 * asl - 21.43;
    let mut out = serde_json::Map::new();
    let mut put = |k: &str, v: f64, scale: &str| {
        out.insert(k.to_string(), json!({"score": round(v, 1), "scale": scale}));
    };
    match lang {
        Lang::English => {
            put("fleschReadingEase", 206.835 - 1.015 * asl - 84.6 * asw, "0–100, higher is easier; 60–70 is plain English");
            put("fleschKincaidGrade", 0.39 * asl + 11.8 * asw - 15.59, "US school grade");
            put("gunningFog", 0.4 * (asl + 100.0 * st.polysyllables as f64 / w), "years of schooling");
            if st.sentences >= 3 {
                put("smog", 1.043 * (st.polysyllables as f64 * 30.0 / s).sqrt() + 3.1291, "US school grade (meant for 30 sentences or more)");
            }
            put("colemanLiau", 0.0588 * (st.letters as f64 / w * 100.0) - 0.296 * (s / w * 100.0) - 15.8, "US school grade");
            put("automatedReadabilityIndex", ari, "US school grade");
        }
        Lang::Spanish => put("fernandezHuerta", 206.84 - 0.60 * asw * 100.0 - 1.02 * asl, "0–100, higher is easier (Flesch for Spanish)"),
        Lang::French => put("kandelMoles", 207.0 - 1.015 * asl - 73.6 * asw, "0–100, higher is easier (Flesch for French)"),
        Lang::German => put("amstad", 180.0 - asl - 58.5 * asw, "0–100, higher is easier (Flesch for German)"),
        Lang::Italian => put("fleschVacca", 217.0 - 1.3 * asl - 0.6 * asw * 100.0, "0–100, higher is easier (Flesch for Italian)"),
        Lang::Portuguese | Lang::Dutch | Lang::Other | Lang::Vietnamese => {}
    }
    if lang != Lang::Vietnamese {
        put("lix", lix, "under 30 very easy, 40 normal, over 60 very hard");
        if lang != Lang::English {
            put("automatedReadabilityIndex", ari, "school grade (letters per word and words per sentence)");
        }
    }
    Value::Object(out)
}

/// `review.readability`: the selection when there is one, else the document (as Word Count counts).
pub(super) fn readability(s: &mut Session, v: &Value) -> CmdResult {
    let whole = s.sel.is_collapsed() || crate::cmd::objects::object_selection(s).is_some();
    let text = if whole { s.counted_text() } else { s.selected_text() };
    let lang = match p::str(v, "language").filter(|t| !t.is_empty() && *t != "auto") {
        Some(t) => Lang::from_tag(t),
        None if looks_vietnamese(&text) => Lang::Vietnamese,
        None => Lang::from_tag(crate::cmd::default_lang(s).as_deref().unwrap_or("en")),
    };
    let st = text_stats(&text, lang);
    let per = |a: usize, b: usize| if b == 0 { 0.0 } else { round(a as f64 / b as f64, 2) };
    Ok(json!({
        "scope": if whole { "document" } else { "selection" },
        "language": lang.tag(),
        "counts": {"paragraphs": st.paragraphs, "sentences": st.sentences, "words": st.words, "syllables": st.syllables, "letters": st.letters,
                   "polysyllabicWords": st.polysyllables, "longWords": st.long_words},
        "averages": {"sentencesPerParagraph": per(st.sentences, st.paragraphs), "wordsPerSentence": per(st.words, st.sentences),
                     "syllablesPerWord": per(st.syllables, st.words), "lettersPerWord": per(st.letters, st.words)},
        "scores": scores(&st, lang),
        "readingMinutes": round(st.words as f64 / 230.0, 1),
        "hardestSentences": hardest_sentences(&text, lang, p::u64(v, "hardest").map_or(5, |n| n.min(50) as usize)),
        "note": "Syllables are estimated from spelling, as readability tools do; the scores are a guide, not a measure of quality.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syllables_follow_the_spelling_rules_well_enough() {
        for (w, n) in [("cat", 1), ("make", 1), ("makes", 1), ("table", 2), ("liked", 1), ("wanted", 2), ("beautiful", 3), ("I", 1)] {
            assert_eq!(syllables(w, Lang::English), n, "{w}");
        }
        assert_eq!(syllables("nước", Lang::Vietnamese), 1);
        assert!(syllables("Universität", Lang::German) >= 4);
    }

    #[test]
    fn sentences_end_at_their_marks_but_not_inside_numbers() {
        let st = text_stats("Pi is 3.14 today. Is it? Yes!\nA new line", Lang::English);
        assert_eq!((st.sentences, st.paragraphs), (4, 2));
    }

    #[test]
    fn harder_text_scores_as_harder() {
        let easy = text_stats("The cat sat on the mat. The dog ran to the park. We had fun.", Lang::English);
        let hard = text_stats(
            "Notwithstanding considerable methodological heterogeneity, the investigators characterised the longitudinal relationship as unequivocally significant.",
            Lang::English,
        );
        let (e, h) = (scores(&easy, Lang::English), scores(&hard, Lang::English));
        let f = |v: &Value, k: &str| v[k]["score"].as_f64().unwrap_or(f64::NAN);
        assert!(f(&e, "fleschReadingEase") > f(&h, "fleschReadingEase"));
        assert!(f(&e, "fleschKincaidGrade") < f(&h, "fleschKincaidGrade"));
        assert!(f(&e, "lix") < f(&h, "lix"));
    }

    #[test]
    fn the_hardest_sentence_comes_first() {
        let text = "The cat sat on a mat. Notwithstanding considerable methodological heterogeneity, investigators characterised it unequivocally. A dog ran home fast.";
        let h = hardest_sentences(text, Lang::English, 2);
        assert!(h.first().is_some_and(|s| s.starts_with("Notwithstanding")), "{h:?}");
        assert!(h.len() == 2 && h.iter().all(|s| s.ends_with('.')));
    }

    #[test]
    fn each_language_gets_the_formulas_that_fit_it() {
        let st = text_stats("Hola amigo. Esto es una prueba sencilla.", Lang::Spanish);
        let s = scores(&st, Lang::Spanish);
        assert!(s.get("fernandezHuerta").is_some() && s.get("fleschReadingEase").is_none());
        let vi = scores(&text_stats("Tôi đi học. Trời hôm nay đẹp.", Lang::Vietnamese), Lang::Vietnamese);
        assert!(vi.as_object().is_some_and(|o| o.is_empty()), "Vietnamese gets counts, not syllable formulas: {vi}");
        assert!(looks_vietnamese("Tôi đi học ở trường"));
        assert!(!looks_vietnamese("The quick brown fox"));
        assert_eq!(scores(&TextStats::default(), Lang::English), json!({}));
    }
}
