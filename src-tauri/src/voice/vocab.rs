//! Per-round vocabulary: names, products and codes from the resume, job description, dossier and
//! round setup, so the transcript can restore them when the recognizer mishears them.

use std::collections::HashSet;

pub const MAX_TERMS: usize = 150;

/// All-caps words that are ordinary, not terms.
const STOP_CAPS: &[&str] =
    &["I", "A", "AND", "OR", "THE", "OF", "IN", "TO", "GPA", "CV", "USA", "UK", "PDF", "CEO", "CTO", "HR", "MBA", "BE", "BS", "MS", "PHD", "IT"];

/// Capitalised words that are ordinary resume words, not names.
const COMMON: &[&str] = &[
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
    "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday", "Developed", "Built", "Designed", "Led",
    "Worked", "Engineer", "Senior", "Software", "Experience", "Education", "Skills", "Projects", "University", "College",
    "Bachelor", "Master", "Technology", "Team", "India", "Present", "Remote",
];

#[derive(Default)]
pub struct Sources<'a> {
    pub resume: &'a str,
    pub jd: &'a str,
    pub notes: &'a str,
    /// Company, persona and problem names, already as terms.
    pub names: Vec<String>,
    /// Dossier text (company style, round names, reported questions), free text.
    pub dossier: &'a str,
}

/// Terms worth protecting in a transcript, most important first, at most MAX_TERMS.
pub fn round_terms(s: &Sources) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = super::stt::TECH_TERMS.iter().map(|t| t.to_lowercase()).collect();
    let mut add = |t: &str, out: &mut Vec<String>| {
        let t = t.trim();
        if out.len() < MAX_TERMS && !t.is_empty() && seen.insert(t.to_lowercase()) {
            out.push(t.to_string());
        }
    };
    if let Some(name) = candidate_name(s.resume) {
        add(&name, &mut out);
        if let Some(first) = name.split_whitespace().next() {
            add(first, &mut out);
        }
    }
    for n in &s.names {
        add(n, &mut out);
    }
    for text in [s.jd, s.dossier, s.resume, s.notes] {
        if out.len() >= MAX_TERMS {
            break;
        }
        scan(text, &mut |t| {
            add(&t, &mut out);
            out.len() < MAX_TERMS
        });
    }
    out
}

/// Rule 4: a capitalised word of 4+ letters (a likely name), not an ordinary resume word.
fn is_name_word(w: &str) -> bool {
    let mut cs = w.chars();
    let Some(first) = cs.next() else { return false };
    first.is_uppercase() && cs.clone().all(char::is_lowercase) && w.chars().count() >= 4 && !COMMON.contains(&w)
}

fn is_term(w: &str) -> bool {
    let chars: Vec<char> = w.chars().collect();
    let n = chars.len();
    let upper_after_first = chars.iter().skip(1).any(|c| c.is_uppercase());
    let has_lower = chars.iter().any(|c| c.is_lowercase());
    let has_letter = chars.iter().any(|c| c.is_alphabetic());
    let has_digit = chars.iter().any(|c| c.is_ascii_digit());
    let alnum = chars.iter().all(|c| c.is_alphanumeric());
    (upper_after_first && has_lower)
        || (has_letter && has_digit && alnum && (2..=12).contains(&n))
        || ((2..=6).contains(&n) && chars.iter().all(|c| c.is_uppercase()) && !STOP_CAPS.contains(&w))
}

/// Candidate terms in free text, in order of first appearance.
pub fn terms_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    scan(text, &mut |t| {
        if seen.insert(t.to_lowercase()) {
            out.push(t);
        }
        true
    });
    out
}

/// Calls `found` for each candidate term (repeats included) until it returns false.
fn scan(text: &str, found: &mut dyn FnMut(String) -> bool) {
    for line in text.lines() {
        // (previous word if it was a name word not followed by punctuation)
        let mut prev_name: Option<&str> = None;
        let mut sentence_start = true;
        for raw in line.split(|c: char| c.is_whitespace() || c == '/') {
            if raw.is_empty() {
                continue;
            }
            let w = raw.trim_matches(|c: char| !c.is_alphanumeric());
            let ends_sentence = raw.ends_with(['.', '!', '?', ':']);
            if w.is_empty() {
                prev_name = None;
                sentence_start = ends_sentence || sentence_start;
                continue;
            }
            let name = !sentence_start && is_name_word(w);
            if (name || is_term(w)) && !found(w.to_string()) {
                return;
            }
            if let (true, Some(p)) = (name, prev_name) {
                if !found(format!("{p} {w}")) {
                    return;
                }
            }
            // A pair needs the two words back to back with nothing between them.
            let clean_end = raw.ends_with(|c: char| c.is_alphanumeric());
            prev_name = (name && clean_end).then_some(w);
            sentence_start = ends_sentence;
        }
    }
}

/// The candidate's name when the resume's first non-empty line looks like one.
pub fn candidate_name(resume: &str) -> Option<String> {
    let line = resume.lines().map(str::trim).find(|l| !l.is_empty())?;
    let words: Vec<&str> = line.split_whitespace().collect();
    let ok = (2..=4).contains(&words.len())
        && line.chars().count() <= 40
        && words.iter().all(|w| w.chars().all(char::is_alphabetic) && w.chars().next().is_some_and(char::is_uppercase));
    ok.then(|| words.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESUME: &str = "Prateesh Sulikeri\nBackend Engineer\n\nBuilt payment services at Razorpay using Spring Boot, \
    Kafka and PostgreSQL. Moved settlement files to S3. Worked with Karthik on the Juspay integration.";

    #[test]
    fn finds_names_products_and_codes() {
        let t = terms_in(RESUME);
        for want in ["Razorpay", "Spring Boot", "PostgreSQL", "S3", "Karthik", "Juspay"] {
            assert!(t.iter().any(|x| x == want), "{want} missing from {t:?}");
        }
        for not in ["Built", "Moved", "Worked", "Engineer"] {
            assert!(!t.iter().any(|x| x == not), "{not} should not be a term");
        }
    }

    #[test]
    fn candidate_name_from_first_line() {
        assert_eq!(candidate_name(RESUME).as_deref(), Some("Prateesh Sulikeri"));
        assert_eq!(candidate_name("Jos\u{e9} Garc\u{ed}a\nEngineer").as_deref(), Some("Jos\u{e9} Garc\u{ed}a"));
        assert_eq!(candidate_name("RESUME\nName: x"), None);
        assert_eq!(candidate_name("Senior backend engineer with 6 years of experience"), None);
        assert_eq!(candidate_name(""), None);
    }

    #[test]
    fn round_terms_order_dedupe_and_cap() {
        let s = Sources { resume: RESUME, jd: "We use Kafka and Hazelcast.", names: vec!["Neha".into(), "Juspay".into()], ..Default::default() };
        let t = round_terms(&s);
        assert_eq!(&t[..3], ["Prateesh Sulikeri", "Prateesh", "Neha"]);
        // Juspay is in names and the resume: kept once. (Razorpay is in the static list, so it is dropped.)
        assert_eq!(t.iter().filter(|x| x.eq_ignore_ascii_case("juspay")).count(), 1);
        assert!(t.iter().any(|x| x == "Hazelcast"));
        assert!(!t.iter().any(|x| x == "Kafka"), "already in the static list");
        // Review focus 4: a huge resume is capped and fast.
        let big: String = (0..5000).map(|i| format!("we used Zorblat{i} daily. ")).collect();
        let started = std::time::Instant::now();
        let t = round_terms(&Sources { resume: &big, ..Default::default() });
        assert_eq!(t.len(), MAX_TERMS);
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
    }
}
