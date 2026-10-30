//! Text rules shared by the checks and the scorer: normalising, whole-term matching,
//! section headings, contact details and dates. Ported from Resume-Matcher
//! (`services/text_match.py`, `services/ats_parse/{normalize,headings,checks}.py`), changed for Onsite.
//! This file is under the Apache License 2.0 (`LICENSES/Apache-2.0.txt`), not MIT.

use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

/// NFKC (so "ﬁ" ligatures from PDFs become "fi"), hyphen line-breaks rejoined,
/// "S U M M A R Y" rejoined, spaces collapsed, lower case.
pub fn normalize(text: &str) -> String {
    static HYPHEN_BREAK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\w)-\n\s*(\w)").unwrap());
    static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\u{a0}]+").unwrap());
    let t: String = text.nfkc().collect();
    let t = HYPHEN_BREAK.replace_all(&t, "$1-$2");
    let t = t.lines().map(rejoin_letter_spacing).collect::<Vec<_>>().join("\n");
    SPACES.replace_all(&t, " ").to_lowercase().trim().to_string()
}

/// Three or more single letters split by single spaces, five chars or longer: "S U M M A R Y".
fn rejoin_letter_spacing(line: &str) -> String {
    let words: Vec<&str> = line.split(' ').collect();
    let mut out: Vec<String> = vec![];
    let mut run: Vec<&str> = vec![];
    let single = |w: &str| w.chars().count() == 1 && w.chars().all(char::is_alphanumeric);
    let flush = |run: &mut Vec<&str>, out: &mut Vec<String>| {
        if run.len() >= 3 {
            out.push(run.concat());
        } else {
            out.extend(run.iter().map(|s| s.to_string()));
        }
        run.clear();
    };
    for w in words {
        if single(w) {
            run.push(w);
        } else {
            flush(&mut run, &mut out);
            out.push(w.to_string());
        }
    }
    flush(&mut run, &mut out);
    out.join(" ")
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x3040..=0x30FF | 0x3130..=0x318F | 0x3400..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF)
}

/// Whole-term, case-insensitive. "java" does not match "javascript"; "c++" matches "C++,".
/// `text` must already be lower case (callers lower it once for many keywords).
pub fn keyword_in_text(keyword: &str, text: &str) -> bool {
    let kw: String = keyword.trim().nfkc().collect::<String>().to_lowercase();
    if kw.is_empty() {
        return false;
    }
    if kw.chars().any(is_cjk) {
        return text.contains(&kw);
    }
    let mut from = 0;
    while let Some(off) = text[from..].find(&kw) {
        let start = from + off;
        let end = start + kw.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        let ok_before = before.is_none_or(|c| !is_word(c) || is_cjk(c) || !kw.starts_with(is_word));
        // "C" must not match "C++" or "C#".
        let ok_after = after.is_none_or(|c| !is_word(c) && !(matches!(c, '+' | '#') && kw.ends_with(is_word)) || is_cjk(c) || !kw.ends_with(is_word));
        if ok_before && ok_after {
            return true;
        }
        from = start + kw.chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// Crude English stem so "microservices" ~ "microservice" and "deployed" ~ "deploying".
/// Only has to be consistent: both sides go through it.
pub fn stem(word: &str) -> String {
    let w = word.to_lowercase();
    if w.chars().count() <= 3 {
        return w;
    }
    let cut = |suf: &str| w.strip_suffix(suf).filter(|b| b.chars().count() >= 3);
    let mut s = if let Some(b) = cut("ies").or_else(|| cut("ied")) {
        format!("{b}y")
    } else if let Some(b) = cut("ings").or_else(|| cut("ing")).or_else(|| cut("ed")) {
        b.to_string()
    } else if w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") && !(w.ends_with("is") && w.len() > 4) {
        // "apis" -> "api" but "redis", "analysis" stay.
        w[..w.len() - 1].to_string()
    } else {
        w.clone()
    };
    if s.chars().count() > 4 && s.ends_with('e') {
        s.pop();
    }
    s
}

/// Word stems in order. Symbols that matter in tech terms (c++, c#, .net, node.js) stay in the word.
pub fn stems(text: &str) -> Vec<String> {
    static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\w][\w+#.]*[\w+#]|[\w+#]+|\.[\w]+").unwrap());
    TOKEN.find_iter(&text.to_lowercase()).map(|m| stem(m.as_str())).collect()
}

/// The term's stems appear in a row somewhere in the text.
pub fn stem_match(term: &str, text_stems: &[String]) -> bool {
    let t = stems(term);
    !t.is_empty() && text_stems.windows(t.len()).any(|w| w == t.as_slice())
}

// ---------------------------------------------------------------- sections

pub const REQUIRED_SECTIONS: [&str; 3] = ["experience", "education", "skills"];

/// English headings an ATS maps to each section.
pub fn heading_names(kind: &str) -> &'static [&'static str] {
    match kind {
        "summary" => &["summary", "profile", "professional summary", "about me", "objective", "career objective", "professional profile", "career summary"],
        "experience" => &[
            "experience", "work experience", "professional experience", "employment history", "work history",
            "employment", "career history", "relevant experience", "industry experience", "internships", "internship experience",
        ],
        "education" => &["education", "academic background", "qualifications", "education and training", "academics"],
        "skills" => &["skills", "technical skills", "core competencies", "competencies", "technologies", "tech stack", "key skills", "tools and technologies", "skills summary"],
        "projects" => &["projects", "personal projects", "key projects", "side projects", "selected projects", "academic projects"],
        "certifications" => &["certifications", "certificates", "licenses and certifications", "courses"],
        "achievements" => &["achievements", "awards", "honors", "accomplishments", "awards and achievements"],
        _ => &[],
    }
}

pub const SECTION_KINDS: [&str; 7] = ["summary", "experience", "education", "skills", "projects", "certifications", "achievements"];

/// A synonym alone, or joined to more heading text ("Skills & Tools"). Prose that only
/// starts with one ("experience in payments") is not a heading.
fn is_heading(line: &str, names: &[&str]) -> bool {
    names.iter().any(|name| {
        line.strip_prefix(name).is_some_and(|rest| {
            let rest = rest.trim_start();
            rest.is_empty() || names.contains(&rest) || rest.starts_with(|c: char| !c.is_alphanumeric() && !c.is_whitespace()) || rest.starts_with("and ")
        })
    })
}

fn heading_line(line: &str) -> Option<String> {
    let l = line.trim().trim_matches(|c: char| matches!(c, ':' | '•' | '|' | '#' | '*' | '-' | '=' | '_') || c.is_whitespace());
    let n = l.split_whitespace().count();
    (n > 0 && n <= 6 && l.chars().count() <= 60).then(|| l.to_string())
}

/// The section a line heads, if it is a heading. `line` must be normalised.
pub fn section_of(line: &str) -> Option<&'static str> {
    let l = heading_line(line)?;
    SECTION_KINDS.into_iter().find(|k| is_heading(&l, heading_names(k)))
}

/// Normalised text split at headings. `"_top"` holds whatever sits above the first heading
/// (name, headline, contact line).
pub fn sections(norm_text: &str) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = vec![("_top", String::new())];
    for line in norm_text.lines() {
        if let Some(kind) = section_of(line) {
            out.push((kind, String::new()));
            continue;
        }
        let body = &mut out.last_mut().expect("starts non-empty").1;
        body.push_str(line);
        body.push('\n');
    }
    out
}

pub fn section_text(secs: &[(&'static str, String)], kind: &str) -> String {
    secs.iter().filter(|(k, _)| *k == kind).map(|(_, b)| b.as_str()).collect::<Vec<_>>().join("\n")
}

// ---------------------------------------------------------------- contact and dates

pub static EMAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\w.+-]{1,64}@[\w-]+(?:\.[\w-]+)+").unwrap());
static PHONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\+?\d[\d\s().-]{5,}\d").unwrap());
static DATE_PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:(?:19|20)\d{2}|0?[1-9]|1[0-2])$").unwrap());
pub static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:19|20)\d{2}\b").unwrap());

/// Seven or more digits that are not all year or month parts ("2019 - 2022" is not a phone).
pub fn has_phone(text: &str) -> bool {
    static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").unwrap());
    PHONE.find_iter(text).any(|m| {
        let groups: Vec<&str> = DIGITS.find_iter(m.as_str()).map(|g| g.as_str()).collect();
        groups.iter().map(|g| g.len()).sum::<usize>() >= 7 && !groups.iter().all(|g| DATE_PART.is_match(g))
    })
}

const MONTH: &str = r"(?:jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)";

/// Date styles, in the order Resume-Matcher tries them at each position.
pub static DATE: LazyLock<Regex> = LazyLock::new(|| {
    let y = r"(?:19|20)\d{2}";
    Regex::new(&format!(
        r"(?i)(?P<iso>\b{y}[-/]\d{{1,2}}\b)|(?P<numeric>\b\d{{1,2}}[/.-]{y}\b)|(?P<name_year>\b{MONTH}\.?,?\s+{y}\b)|(?P<name_short_year>\b{MONTH}\.?\s+'\d{{2}}\b)|(?P<year>\b{y}\b)"
    ))
    .unwrap()
});

/// Share of dates written in the most common style, 0-100. `None` under two dates.
pub fn date_consistency(text: &str) -> Option<f64> {
    let mut counts = std::collections::HashMap::<&str, usize>::new();
    for c in DATE.captures_iter(text) {
        for name in ["iso", "numeric", "name_year", "name_short_year", "year"] {
            if c.name(name).is_some() {
                *counts.entry(name).or_default() += 1;
                break;
            }
        }
    }
    let total: usize = counts.values().sum();
    (total >= 2).then(|| (*counts.values().max().unwrap_or(&0) as f64 / total as f64 * 1000.0).round() / 10.0)
}

/// Months of experience from date ranges ("Jan 2020 - Present", "03/2019 – 2021"),
/// overlapping jobs counted once. `today` is (year, month).
pub fn experience_months(text: &str, today: (i32, u32)) -> Option<u32> {
    static RANGE: LazyLock<Regex> = LazyLock::new(|| {
        let point = format!(r"(?:{MONTH}\.?,?\s+(?:19|20)\d{{2}}|\d{{1,2}}[/.-](?:19|20)\d{{2}}|(?:19|20)\d{{2}}[/-]\d{{1,2}}|(?:19|20)\d{{2}})");
        Regex::new(&format!(r"(?i)(?P<a>{point})\s*(?:-|–|—|to|until|till)\s*(?P<b>{point}|present|current|now|today|date|ongoing)")).unwrap()
    });
    let mut spans: Vec<(i32, i32)> = vec![];
    let now = today.0 * 12 + today.1 as i32 - 1;
    for c in RANGE.captures_iter(text) {
        let Some(a) = parse_point(&c["a"], false) else { continue };
        let Some(b) = (if is_open_end(&c["b"]) { Some(now) } else { parse_point(&c["b"], true) }) else { continue };
        if b >= a && b <= now + 1 && b - a < 50 * 12 {
            spans.push((a, b + 1));
        }
    }
    if spans.is_empty() {
        return None;
    }
    spans.sort();
    let (mut total, mut cur) = (0, spans[0]);
    for s in spans.into_iter().skip(1) {
        if s.0 <= cur.1 {
            cur.1 = cur.1.max(s.1);
        } else {
            total += cur.1 - cur.0;
            cur = s;
        }
    }
    total += cur.1 - cur.0;
    Some(total as u32)
}

fn is_open_end(s: &str) -> bool {
    let s = s.to_lowercase();
    ["present", "current", "now", "today", "date", "ongoing"].contains(&s.as_str())
}

/// Month index (year*12 + month-1). A bare year starts in January, or ends in December.
fn parse_point(s: &str, end: bool) -> Option<i32> {
    static Y: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:19|20)\d{2}").unwrap());
    let s = s.to_lowercase();
    let year: i32 = Y.find(&s)?.as_str().parse().ok()?;
    let months = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let month = if let Some(i) = months.iter().position(|m| s.starts_with(m)) {
        i as i32 + 1
    } else {
        let rest = s.replace(&year.to_string(), "");
        match rest.trim_matches(|c: char| !c.is_ascii_digit()).parse::<i32>() {
            Ok(m) if (1..=12).contains(&m) => m,
            _ => if end { 12 } else { 1 },
        }
    };
    Some(year * 12 + month - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_term_matching() {
        let t = "java, javascript and c++. node.js on aws; ci/cd";
        assert!(keyword_in_text("Java", t));
        assert!(keyword_in_text("C++", t));
        assert!(keyword_in_text("Node.js", t));
        assert!(keyword_in_text("CI/CD", t));
        assert!(!keyword_in_text("Go", t));
        assert!(!keyword_in_text("script", t));
        assert!(!keyword_in_text("C", "c++ only"));
        assert!(keyword_in_text("C#", "c#/.net"));
        assert!(keyword_in_text("java", "javajava java"));
    }

    #[test]
    fn normalizes_pdf_artifacts() {
        assert_eq!(normalize("E X P E R I E N C E\nconﬁg  man-\n agement"), "experience\nconfig man-agement");
        assert_eq!(normalize("A B test"), "a b test");
    }

    #[test]
    fn stems_line_up() {
        let s = stems("Built event-driven microservices; deployed services");
        assert!(stem_match("microservice", &s));
        assert!(stem_match("deploying", &s));
        assert!(stem_match("Event-driven", &s) || stem_match("event driven", &s));
        assert!(!stem_match("kubernetes", &s));
        assert_eq!(stem("aws"), "aws");
        assert_eq!(stem("apis"), "api");
        assert_eq!(stem("engineers"), stem("engineer"));
        assert_eq!(stem("processes"), stem("process"));
        assert_eq!(stem("caches"), stem("cache"));
        assert_eq!(stem("redis"), "redis");
    }

    #[test]
    fn finds_sections() {
        let t = normalize("Jane Doe\nSenior Backend Engineer\nWORK EXPERIENCE\nAcme\nExperience in payments is key\nSkills & Tools\nJava\nEducation:\nBITS");
        let s = sections(&t);
        let kinds: Vec<&str> = s.iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, ["_top", "experience", "skills", "education"]);
        assert!(section_text(&s, "experience").contains("experience in payments"));
        assert!(section_text(&s, "_top").contains("senior backend engineer"));
    }

    #[test]
    fn phones_not_dates() {
        assert!(has_phone("+91 98765 43210"));
        assert!(has_phone("(555) 123-4567"));
        assert!(!has_phone("2019 - 2022"));
        assert!(!has_phone("01/2019 - 12/2022"));
    }

    #[test]
    fn date_styles() {
        assert_eq!(date_consistency("Jan 2020 - Mar 2023, Feb 2018 - Dec 2019"), Some(100.0));
        assert_eq!(date_consistency("Jan 2020 - 03/2023"), Some(50.0));
        assert_eq!(date_consistency("2020"), None);
    }

    #[test]
    fn experience_months_merge_overlaps() {
        let t = "Acme  Jan 2020 – Present\nBeta 06/2018 - 12/2020\nSchool 2014 - 2018";
        // One unbroken stretch: Jan 2014 through Oct 2026.
        assert_eq!(experience_months(t, (2026, 10)), Some(12 * 12 + 10));
        assert_eq!(experience_months("Acme Jan 2020 - Dec 2020\nBeta Jan 2022 - Jun 2022", (2026, 10)), Some(18));
        assert_eq!(experience_months("no dates here", (2026, 10)), None);
    }
}
