//! Review › Protect › Redact: remove words, phrases and patterns (e-mail addresses, phone and card
//! numbers…) from every part of the document, replacing them with a mark of the same length, so
//! the text is gone from the file and not only covered. Every story is searched: the body, headers,
//! footers, footnotes, endnotes, comments and text boxes, hidden text and tracked insertions and
//! deletions included; and the document properties. The redaction itself is never tracked.

use regex::Regex;

use super::{CmdError, CmdResult, Document, Pos, Session, StoryRef, Value, json, p};

/// A ready-made pattern: its name, the expression, and a check a match must pass.
struct Preset {
    name: &'static str,
    pattern: &'static str,
    check: fn(&str) -> bool,
}

fn any(_: &str) -> bool {
    true
}

/// Card numbers: 13 to 19 digits that pass the Luhn check, so order numbers aren't redacted.
fn luhn(s: &str) -> bool {
    let d: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&d.len()) {
        return false;
    }
    let sum: u32 = d.iter().rev().enumerate().map(|(i, &x)| if i % 2 == 1 { if x * 2 > 9 { x * 2 - 9 } else { x * 2 } } else { x }).sum();
    sum % 10 == 0
}

/// IBANs: the mod-97 check of ISO 13616.
fn iban(s: &str) -> bool {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_uppercase();
    if t.len() < 15 || t.len() > 34 {
        return false;
    }
    let (head, tail) = t.split_at(4);
    let mut rem: u64 = 0;
    for c in tail.chars().chain(head.chars()) {
        let v = match c.to_digit(36) {
            Some(v) => v as u64,
            None => return false,
        };
        rem = if v >= 10 { (rem * 100 + v) % 97 } else { (rem * 10 + v) % 97 };
    }
    rem == 1
}

fn ipv4(s: &str) -> bool {
    s.split('.').all(|o| o.parse::<u16>().is_ok_and(|n| n <= 255))
}

/// Ten digits or more (a phone number), not a year or a short count.
fn phone(s: &str) -> bool {
    s.chars().filter(char::is_ascii_digit).count() >= 9
}

const PRESETS: &[Preset] = &[
    Preset { name: "email", pattern: r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}", check: any },
    Preset { name: "phone", pattern: r"\+?\d[\d ()\-.]{7,}\d", check: phone },
    Preset { name: "creditCard", pattern: r"\b(?:\d[ \-]?){12,18}\d\b", check: luhn },
    Preset { name: "iban", pattern: r"\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]){11,30}\b", check: iban },
    Preset { name: "ipAddress", pattern: r"\b\d{1,3}(?:\.\d{1,3}){3}\b", check: ipv4 },
    Preset { name: "url", pattern: r"\bhttps?://[^\s<>\x22]+", check: any },
];

/// One thing to redact: a rule's name, its expression, and the check its matches must pass.
struct Rule {
    name: String,
    re: Regex,
    check: fn(&str) -> bool,
}

fn rules(v: &Value) -> Result<Vec<Rule>, CmdError> {
    let match_case = p::bool(v, "matchCase").unwrap_or(false);
    let whole = p::bool(v, "wholeWord").unwrap_or(true);
    let strings = |k: &str| -> Vec<String> {
        v.get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).filter(|t| !t.trim().is_empty()).collect())
            .unwrap_or_default()
    };
    let case = if match_case { "" } else { "(?i)" };
    let mut out = Vec::new();
    for t in strings("texts") {
        let t = t.trim();
        let body = regex::escape(t);
        // \b only where the text starts or ends with a letter or digit: "Ann" not in "Annex"
        let word = |c: Option<char>| whole && c.is_some_and(char::is_alphanumeric);
        let pat = format!(r"{case}{}{body}{}", if word(t.chars().next()) { r"\b" } else { "" }, if word(t.chars().last()) { r"\b" } else { "" });
        let re = Regex::new(&pat).map_err(|e| CmdError::Params(format!("texts: {e}")))?;
        out.push(Rule { name: format!("text:{t}"), re, check: any });
    }
    for (i, pat) in strings("patterns").into_iter().enumerate() {
        let re = Regex::new(&format!("{case}{pat}")).map_err(|e| CmdError::Params(format!("patterns[{i}]: {e}")))?;
        out.push(Rule { name: format!("pattern:{i}"), re, check: any });
    }
    for name in strings("presets") {
        let pr = PRESETS.iter().find(|p| p.name == name).ok_or_else(|| {
            CmdError::Params(format!("unknown preset {name:?} (one of: {})", PRESETS.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")))
        })?;
        let re = Regex::new(pr.pattern).map_err(|e| CmdError::Failed(e.to_string()))?;
        out.push(Rule { name: pr.name.to_string(), re, check: pr.check });
    }
    if out.is_empty() {
        return Err(CmdError::Params("say what to redact: texts, patterns or presets".into()));
    }
    Ok(out)
}

/// The byte ranges of `text` the rules match (merged where they overlap), with the rule of each.
fn matches(text: &str, rules: &[Rule]) -> Vec<(usize, usize, usize)> {
    let mut found: Vec<(usize, usize, usize)> = Vec::new();
    for (k, r) in rules.iter().enumerate() {
        for m in r.re.find_iter(text) {
            if m.end() > m.start() && (r.check)(m.as_str()) {
                found.push((m.start(), m.end(), k));
            }
        }
    }
    found.sort();
    let mut merged: Vec<(usize, usize, usize)> = Vec::new();
    for (a, b, k) in found {
        match merged.last_mut() {
            Some(last) if a < last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b, k)),
        }
    }
    merged
}

/// What replaces `found`: the mark once per character (spaces kept, so the line keeps its
/// shape), or the label as it is.
fn masked(found: &str, mark: &str, keep_length: bool) -> String {
    if keep_length { found.chars().map(|c| if c.is_whitespace() { c.to_string() } else { mark.to_string() }).collect() } else { mark.to_string() }
}

/// The same rules over the document properties' text (title, subject, keywords, author…,
/// custom properties); the names of the ones changed.
fn redact_properties(doc: &mut Document, rules: &[Rule], mark: &str, keep_length: bool) -> Vec<String> {
    let mut changed = Vec::new();
    let scrub = |t: &str| -> Option<String> {
        let m = matches(t, rules);
        if m.is_empty() {
            return None;
        }
        let mut s = t.to_string();
        for (a, b, _) in m.into_iter().rev() {
            let rep = masked(&s[a..b], mark, keep_length);
            s.replace_range(a..b, &rep);
        }
        Some(s)
    };
    if let Ok(Value::Object(mut core)) = serde_json::to_value(&doc.core) {
        for (k, v) in core.iter_mut() {
            if let Some(t) = v.as_str()
                && let Some(new) = scrub(t)
            {
                *v = Value::String(new);
                changed.push(k.clone());
            }
        }
        if !changed.is_empty()
            && let Ok(c) = serde_json::from_value(Value::Object(core))
        {
            doc.core = c;
        }
    }
    for cp in &mut doc.custom_props {
        if let Some(new) = scrub(&cp.value) {
            cp.value = new;
            changed.push(format!("custom:{}", cp.name));
        }
    }
    changed
}

/// `review.redact`.
pub(super) fn redact(s: &mut Session, v: &Value) -> CmdResult {
    let rules = rules(v)?;
    let mark = p::str(v, "mark").unwrap_or("█").to_string();
    let keep_length = p::bool(v, "keepLength").unwrap_or(true);
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect();
    // Redaction must not leave the text behind as a tracked deletion.
    let tracking = std::mem::replace(&mut s.doc.settings.track_changes, false);
    let mut by_rule = vec![0usize; rules.len()];
    let (mut total, mut touched) = (0usize, 0usize);
    let result = (|| -> Result<(), CmdError> {
        for story in &stories {
            let mut here = 0;
            for path in s.doc.para_paths(*story) {
                let Some(para) = s.doc.para(*story, &path) else { continue };
                let text = para.text.clone();
                for (a, b, k) in matches(&text, &rules).into_iter().rev() {
                    let (pa, pb) = (Pos { story: *story, path: path.clone(), off: a }, Pos { story: *story, path: path.clone(), off: b });
                    let props = s.doc.para_at(&pa).map(|p| p.props_of_char(a).clone().without_symbol()).unwrap_or_default();
                    let rep = masked(&text[a..b], &mark, keep_length);
                    let at = s.doc.delete_range(&pa, &pb)?;
                    // Inline objects (pictures, fields) inside a match go with it; a match is text.
                    s.doc.insert_text(&at, &rep, &props)?;
                    by_rule[k] += 1;
                    here += 1;
                }
            }
            total += here;
            touched += usize::from(here > 0);
        }
        Ok(())
    })();
    s.doc.settings.track_changes = tracking;
    result?;
    let properties = redact_properties(&mut s.doc, &rules, &mark, keep_length);
    s.clamp_selection();
    s.status = format!("Redacted {total} places.");
    Ok(json!({
        "redacted": total,
        "stories": touched,
        "byRule": rules.iter().zip(&by_rule).map(|(r, n)| json!({"rule": r.name, "count": n})).collect::<Vec<_>>(),
        "properties": properties,
        "notChecked": ["text inside pictures (scan them with OCR first)", "pictures' alternative text", "comment authors' names", "the file's earlier saved versions"],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::Document;

    #[test]
    fn checked_presets_skip_what_only_looks_like_their_data() {
        assert!(luhn("4111 1111 1111 1111") && !luhn("4111 1111 1111 1112"));
        assert!(iban("DE89 3704 0044 0532 0130 00") && !iban("DE89 3704 0044 0532 0130 01"));
        assert!(ipv4("192.168.1.20") && !ipv4("999.1.1.1"));
        assert!(phone("+84 901 234 567") && !phone("2024-11"));
    }

    #[test]
    fn overlapping_matches_become_one_range() {
        let rules = vec![
            Rule { name: "a".into(), re: Regex::new("Ann Lee").unwrap_or_else(|e| panic!("{e}")), check: any },
            Rule { name: "b".into(), re: Regex::new("Lee Smith").unwrap_or_else(|e| panic!("{e}")), check: any },
        ];
        assert_eq!(matches("Ann Lee Smith", &rules), vec![(0, 13, 0)]);
        assert_eq!(masked("Ann Lee", "█", true), "███ ███");
        assert_eq!(masked("Ann Lee", "[REDACTED]", false), "[REDACTED]");
    }

    #[test]
    fn redaction_removes_the_text_from_the_document_and_its_properties() {
        let mut s = Session::new(Document::new());
        s.run("document.setText", &json!({"text": "Ann Lee wrote to ann@example.com from +84 901 234 567. Annex stays."}))
            .unwrap_or_else(|e| panic!("{e}"));
        s.doc.core.title = "Notes by Ann Lee".into();
        let r = s.run("review.redact", &json!({"texts": ["ann lee"], "presets": ["email", "phone"]})).unwrap_or_else(|e| panic!("{e}"));
        let text: String = s.doc.para_paths(StoryRef::Body).iter().filter_map(|p| s.doc.para(StoryRef::Body, p)).map(|p| p.text.clone()).collect();
        assert!(!text.contains("Ann Lee") && !text.contains("example.com") && !text.contains("901"), "{text}");
        assert!(text.contains("Annex stays"), "a whole-word rule leaves longer words alone: {text}");
        assert!(text.contains('█'));
        assert!(!s.doc.core.title.contains("Ann Lee"), "{}", s.doc.core.title);
        assert!(r["redacted"].as_u64().is_some_and(|n| n >= 3), "{r}");
        assert!(s.run("review.redact", &json!({})).is_err(), "nothing to redact is refused");
        assert!(s.run("review.redact", &json!({"presets": ["nope"]})).is_err());
    }

    #[test]
    fn redaction_is_not_tracked_even_when_the_document_tracks_changes() {
        let mut s = Session::new(Document::new());
        s.run("document.setText", &json!({"text": "Secret code 4111 1111 1111 1111 here"})).unwrap_or_else(|e| panic!("{e}"));
        s.run("review.trackChanges", &json!({"value": true})).unwrap_or_else(|e| panic!("{e}"));
        s.run("review.redact", &json!({"presets": ["creditCard"]})).unwrap_or_else(|e| panic!("{e}"));
        let p = s.doc.para(StoryRef::Body, &s.doc.para_paths(StoryRef::Body)[0]).map(|p| p.text.clone()).unwrap_or_default();
        assert!(!p.contains("4111"), "the number is gone, not kept as a deletion: {p}");
        assert!(s.doc.settings.track_changes, "tracking is back on afterwards");
    }
}
