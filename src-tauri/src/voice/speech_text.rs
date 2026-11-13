//! Written text to spoken text, for the voice only. The screen and transcript keep the original.
//! Rules are applied in a fixed order; see `docs/plans/voice-pipeline-rebuild-impl.md` Task 4.
//! Text the lexicon replaced is wrapped in private-use marks so no later rule touches it.

use regex::{Captures, Regex};
use serde::Deserialize;
use std::path::Path;
use std::sync::LazyLock;

const OPEN: char = '\u{E000}';
const CLOSE: char = '\u{E001}';

/// Pronunciation lexicon from `prompts/pronounce.json`: whole terms said a fixed way.
#[derive(Default)]
pub struct Lexicon {
    /// (term, spoken), longest term first.
    say: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct LexFile {
    say: Vec<LexEntry>,
}

#[derive(Deserialize)]
struct LexEntry {
    term: String,
    #[serde(rename = "as")]
    spoken: String,
}

impl Lexicon {
    pub fn from_json(json: &str) -> Result<Lexicon, String> {
        let f: LexFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut say: Vec<(String, String)> = f.say.into_iter().filter(|e| !e.term.is_empty()).map(|e| (e.term, e.spoken)).collect();
        say.sort_by_key(|(t, _)| std::cmp::Reverse(t.chars().count()));
        Ok(Lexicon { say })
    }

    /// Reads `<prompts_dir>/pronounce.json`. Any error gives the empty lexicon and a warning.
    pub fn load(prompts_dir: &Path) -> Lexicon {
        match std::fs::read_to_string(prompts_dir.join("pronounce.json")).map_err(|e| e.to_string()).and_then(|s| Lexicon::from_json(&s)) {
            Ok(l) => l,
            Err(_) => {
                tracing::warn!("pronunciation lexicon not loaded; speaking without it");
                Lexicon::default()
            }
        }
    }
}

const ONES: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen",
    "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
];
const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
const ORDINALS: [&str; 32] = [
    "", "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth", "eleventh", "twelfth",
    "thirteenth", "fourteenth", "fifteenth", "sixteenth", "seventeenth", "eighteenth", "nineteenth", "twentieth",
    "twenty first", "twenty second", "twenty third", "twenty fourth", "twenty fifth", "twenty sixth", "twenty seventh",
    "twenty eighth", "twenty ninth", "thirtieth", "thirty first",
];
/// Largest number read as a whole; longer digit runs are read digit by digit.
const MAX_WORDS: u64 = 999_999_999_999;

fn below_thousand(n: u64, out: &mut Vec<String>) {
    let (h, rest) = (n / 100, n % 100);
    if h > 0 {
        out.push(format!("{} hundred", ONES[h as usize]));
    }
    if rest >= 20 {
        out.push(TENS[(rest / 10) as usize].to_string());
        if rest % 10 > 0 {
            out.push(ONES[(rest % 10) as usize].to_string());
        }
    } else if rest > 0 {
        out.push(ONES[rest as usize].to_string());
    }
}

/// `256` -> "two hundred fifty six". No "and", no hyphens.
pub fn int_words(n: u64) -> String {
    if n == 0 {
        return "zero".into();
    }
    let mut out = Vec::new();
    let mut rest = n;
    for (scale, name) in [(1_000_000_000_000u64, "trillion"), (1_000_000_000, "billion"), (1_000_000, "million"), (1_000, "thousand")] {
        if rest >= scale {
            below_thousand(rest / scale, &mut out);
            out.push(name.into());
            rest %= scale;
        }
    }
    below_thousand(rest, &mut out);
    out.join(" ")
}

fn digit_word(c: char, zero: &str) -> String {
    match c.to_digit(10) {
        Some(0) => zero.to_string(),
        Some(d) => ONES[d as usize].to_string(),
        None => c.to_string(),
    }
}

fn digits_one_by_one(d: &str, zero: &str) -> String {
    d.chars().map(|c| digit_word(c, zero)).collect::<Vec<_>>().join(" ")
}

/// A digit run said as a number, or digit by digit when it is too long.
fn integer(d: &str) -> String {
    match d.parse::<u64>() {
        Ok(n) if n <= MAX_WORDS => int_words(n),
        _ => digits_one_by_one(d, "zero"),
    }
}

/// "99.99" -> "ninety nine point nine nine"; "7" -> "seven".
fn number(s: &str) -> String {
    match s.split_once('.') {
        Some((whole, frac)) => format!("{} point {}", integer(whole), digits_one_by_one(frac, "zero")),
        None => integer(s),
    }
}

fn is_one(s: &str) -> bool {
    s.parse::<f64>().is_ok_and(|v| v == 1.0)
}

fn re(p: &str) -> Regex {
    Regex::new(p).unwrap()
}

/// Applies `f` to the text outside lexicon marks only.
fn outside_marks(s: &str, f: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(OPEN) {
        out.push_str(&f(&rest[..i]));
        let after = &rest[i..];
        let end = after.find(CLOSE).map_or(after.len(), |j| j + CLOSE.len_utf8());
        out.push_str(&after[..end]);
        rest = &after[end..];
    }
    out.push_str(&f(rest));
    out
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0xFE0F)
}

/// Rule 1.
fn clean(text: &str) -> String {
    static HASH: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^\s*#+\s*"));
    let s: String = text.chars().filter(|&c| c != '*' && c != '`' && !is_emoji(c)).collect();
    let s = HASH.replace_all(&s, "");
    s.replace(['\u{2014}', '\u{2013}'], ", ").replace("...", ",").replace('\u{2026}', ",")
}

/// Rules 2 and 3: links and very long tokens are never read out.
fn links_and_long_values(s: &str) -> String {
    static URL: LazyLock<Regex> = LazyLock::new(|| re(r"(?:https?://|www\.)\S+"));
    let s = URL.replace_all(s, |c: &Captures| {
        let url = &c[0];
        let kept = url.trim_end_matches(['.', ',', '?', '!']);
        format!("a link{}", &url[kept.len()..])
    });
    s.split(' ').map(|t| if t.chars().count() > 40 { "a long value" } else { t }).collect::<Vec<_>>().join(" ")
}

/// Rule 4: lexicon terms, longest first, whole terms only, case-sensitive.
fn apply_lexicon(s: &str, lex: &Lexicon) -> String {
    let mut s = s.to_string();
    for (term, spoken) in &lex.say {
        s = outside_marks(&s, |part| {
            let mut out = String::new();
            let mut i = 0;
            while let Some(j) = part[i..].find(term.as_str()) {
                let start = i + j;
                let end = start + term.len();
                let before_ok = part[..start].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
                let after_ok = part[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
                if before_ok && after_ok {
                    out.push_str(&part[i..start]);
                    out.push(OPEN);
                    out.push_str(spoken);
                    out.push(CLOSE);
                } else {
                    out.push_str(&part[i..end]);
                }
                i = end;
            }
            out.push_str(&part[i..]);
            out
        });
    }
    s
}

/// Rule 5: O(n log n) -> "O of n log n".
fn big_o(s: &str) -> String {
    static O: LazyLock<Regex> = LazyLock::new(|| re(r"\bO\(([^)]*)\)"));
    O.replace_all(s, |c: &Captures| {
        let inner = c[1].trim().replace("^2", " squared").replace("^3", " cubed").replace('*', " times ");
        let inner = if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()) { integer(&inner) } else { inner };
        format!("O of {inner}")
    })
    .into_owned()
}

/// Rule 6: generics become spaces, brackets go (their content stays).
fn brackets(s: &str) -> String {
    static GENERIC: LazyLock<Regex> = LazyLock::new(|| re(r"([A-Za-z0-9])<([^<>\s][^<>]*?)>"));
    let mut s = s.to_string();
    // Inner generics first, so nested ones (Map<String, List<Integer>>) unwrap too.
    loop {
        let next = GENERIC.replace_all(&s, "$1 $2 ").into_owned();
        if next == s {
            break;
        }
        s = next;
    }
    s.replace(['(', ')', '[', ']', '{', '}'], " ")
}

/// Rule 7: operators and symbols.
fn operators(s: &str) -> String {
    static OPS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        [
            (r"\s*(?:->|=>|\x{2192})\s*", "to"),
            (r"\s*>=\s*", "greater than or equal to"),
            (r"\s*<=\s*", "less than or equal to"),
            (r"\s*!=\s*", "not equal to"),
            (r"\s*==\s*", "equals"),
            (r"\s*&&\s*", "and"),
            (r"\s*\|\|\s*", "or"),
            (r" = ", "equals"),
            (r" < ", "less than"),
            (r" > ", "greater than"),
            (r" \+ ", "plus"),
            (r" & ", "and"),
            (r"\s*@\s*", "at"),
        ]
        .into_iter()
        .map(|(p, w)| (re(p), w))
        .collect()
    });
    static TILDE: LazyLock<Regex> = LazyLock::new(|| re(r"~(\d)"));
    let mut s = s.to_string();
    for (r, w) in OPS.iter() {
        s = r.replace_all(&s, format!(" {w} ").as_str()).into_owned();
    }
    TILDE.replace_all(&s, "about $1").replace('|', " ")
}

/// (suffix, singular, plural).
const UNITS: [(&str, &str, &str); 7] = [
    ("ms", "millisecond", "milliseconds"),
    ("KB", "kilobyte", "kilobytes"),
    ("MB", "megabyte", "megabytes"),
    ("GB", "gigabyte", "gigabytes"),
    ("TB", "terabyte", "terabytes"),
    ("QPS", "Q P S", "Q P S"),
    ("RPS", "R P S", "R P S"),
];

fn unit(suffix: &str, one: bool) -> String {
    UNITS.iter().find(|u| u.0 == suffix).map_or(suffix.to_string(), |u| if one { u.1 } else { u.2 }.to_string())
}

fn mixed_token(t: &str) -> String {
    let mut runs: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in t.chars() {
        if !cur.is_empty() && cur.chars().last().unwrap().is_ascii_digit() != c.is_ascii_digit() {
            runs.push(std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    runs.push(cur);
    runs.iter()
        .map(|r| {
            if r.chars().all(|c| c.is_ascii_digit()) {
                if r.len() <= 2 {
                    integer(r)
                } else {
                    digits_one_by_one(r, "oh")
                }
            } else if (2..=4).contains(&r.len()) && r.chars().all(|c| c.is_ascii_uppercase()) {
                r.chars().map(String::from).collect::<Vec<_>>().join(" ")
            } else {
                r.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rule 8: numbers, units, codes.
fn numbers(s: &str) -> String {
    static COMMA: LazyLock<Regex> = LazyLock::new(|| re(r"(\d),(\d)"));
    static MONEY: LazyLock<Regex> = LazyLock::new(|| re(r"\$(\d+(?:\.\d+)?)"));
    static PCTL: LazyLock<Regex> = LazyLock::new(|| re(r"\b[pP](\d+(?:\.\d+)?)\b"));
    static VERSION: LazyLock<Regex> = LazyLock::new(|| re(r"\bv(\d+(?:\.\d+)+)\b"));
    static ORDINAL: LazyLock<Regex> = LazyLock::new(|| re(r"\b(\d+)(st|nd|rd|th)\b"));
    static RANGE: LazyLock<Regex> = LazyLock::new(|| re(r"\b(\d+)-(\d+)\b"));
    static PERCENT: LazyLock<Regex> = LazyLock::new(|| re(r"\b(\d+(?:\.\d+)?)%"));
    // The trailing group stands in for a lookahead: a unit must not run into more letters or digits.
    static SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
        re(r"\b(\d+(?:\.\d+)?)(?:([kKMB])(?: (ms|KB|MB|GB|TB|QPS|RPS))?|(x|s)| ?(ms|KB|MB|GB|TB|QPS|RPS))([^A-Za-z0-9]|$)")
    });
    static DECIMAL: LazyLock<Regex> = LazyLock::new(|| re(r"\b(\d+)\.(\d+)\b"));
    static MIXED: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:[A-Za-z]+\d[A-Za-z0-9]*|\d+[A-Za-z][A-Za-z0-9]*)\b"));
    static INT: LazyLock<Regex> = LazyLock::new(|| re(r"\b\d+\b"));

    let mut s = s.to_string();
    while COMMA.is_match(&s) {
        s = COMMA.replace_all(&s, "$1$2").into_owned();
    }
    let s = MONEY.replace_all(&s, |c: &Captures| {
        if is_one(&c[1]) {
            "one dollar".to_string()
        } else {
            format!("{} dollars", number(&c[1]))
        }
    });
    let s = PCTL.replace_all(&s, |c: &Captures| format!("P {}", number(&c[1])));
    let s = VERSION.replace_all(&s, |c: &Captures| format!("version {}", c[1].split('.').map(integer).collect::<Vec<_>>().join(" point ")));
    let s = ORDINAL.replace_all(&s, |c: &Captures| match c[1].parse::<usize>() {
        Ok(n @ 1..=31) => ORDINALS[n].to_string(),
        _ => c[0].to_string(),
    });
    let s = RANGE.replace_all(&s, |c: &Captures| format!("{} to {}", integer(&c[1]), integer(&c[2])));
    let s = PERCENT.replace_all(&s, |c: &Captures| format!("{} percent", number(&c[1])));
    let s = SUFFIX.replace_all(&s, |c: &Captures| {
        let n = &c[1];
        let tail = &c[6];
        let said = if let Some(mult) = c.get(2) {
            let m = match mult.as_str() {
                "k" | "K" => "thousand",
                "M" => "million",
                _ => "billion",
            };
            match c.get(3) {
                Some(u) => format!("{} {m} {}", number(n), unit(u.as_str(), false)),
                None => format!("{} {m}", number(n)),
            }
        } else if let Some(xs) = c.get(4) {
            match xs.as_str() {
                "x" => format!("{} times", number(n)),
                _ if is_one(n) => format!("{} second", number(n)),
                _ => format!("{} seconds", number(n)),
            }
        } else {
            format!("{} {}", number(n), unit(&c[5], is_one(n)))
        };
        format!("{said}{tail}")
    });
    let s = DECIMAL.replace_all(&s, |c: &Captures| number(&format!("{}.{}", &c[1], &c[2])));
    let s = MIXED.replace_all(&s, |c: &Captures| mixed_token(&c[0]));
    INT.replace_all(&s, |c: &Captures| integer(&c[0])).into_owned()
}

/// Rule 9: dots, underscores, slashes and camel case inside identifiers.
fn identifiers(s: &str) -> String {
    static DOT: LazyLock<Regex> = LazyLock::new(|| re(r"(\p{L})\.(\p{L}|\d)"));
    static UNDER: LazyLock<Regex> = LazyLock::new(|| re(r"([\p{L}\d])_([\p{L}\d])"));
    static SLASH: LazyLock<Regex> = LazyLock::new(|| re(r"(\p{L})/(\p{L})"));
    static CAMEL: LazyLock<Regex> = LazyLock::new(|| re(r"(\p{Ll})(\p{Lu})"));
    let mut s = s.to_string();
    // Repeated so chains like a.b.c and a_b_c are all split (each match uses the shared letter).
    for (r, with) in [(&*DOT, "$1 dot $2"), (&*UNDER, "$1 $2"), (&*SLASH, "$1 $2"), (&*CAMEL, "$1 $2")] {
        loop {
            let next = r.replace_all(&s, with).into_owned();
            if next == s {
                break;
            }
            s = next;
        }
    }
    s
}

/// Rule 10.
fn tidy(s: &str) -> String {
    static SPACES: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));
    static BEFORE_PUNCT: LazyLock<Regex> = LazyLock::new(|| re(r" ([.,?!;:])"));
    static COMMAS: LazyLock<Regex> = LazyLock::new(|| re(r",(?: ?,)+"));
    let s: String = s.chars().filter(|&c| c != OPEN && c != CLOSE).collect();
    let s = SPACES.replace_all(&s, " ");
    let s = BEFORE_PUNCT.replace_all(&s, "$1");
    let s = COMMAS.replace_all(&s, ",");
    let s = s.trim();
    if s.chars().any(char::is_alphanumeric) {
        s.to_string()
    } else {
        String::new()
    }
}

/// The text the voice should say for `text`. Empty when nothing is left to say.
pub fn to_speech(text: &str, lex: &Lexicon) -> String {
    let s = clean(text);
    let s = links_and_long_values(&s);
    let s = apply_lexicon(&s, lex);
    let s = outside_marks(&s, |p| identifiers(&numbers(&operators(&brackets(&big_o(p))))));
    tidy(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex() -> Lexicon {
        Lexicon::from_json(include_str!("../../../prompts/pronounce.json")).unwrap()
    }

    #[test]
    fn int_words_basics() {
        for (n, w) in [(0, "zero"), (7, "seven"), (13, "thirteen"), (40, "forty"), (99, "ninety nine"),
            (100, "one hundred"), (256, "two hundred fifty six"), (1001, "one thousand one"),
            (10_000, "ten thousand"), (1_000_000, "one million"), (2_500_000_000, "two billion five hundred million")] {
            assert_eq!(int_words(n), w);
        }
    }

    #[test]
    fn fixture_lines_become_their_spoken_form() {
        let tsv = include_str!("../../tests/fixtures/voice/tts_tech_lines.tsv");
        let empty = Lexicon::default();
        for row in tsv.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let (written, spoken) = row.split_once('\t').unwrap();
            assert_eq!(to_speech(written, &empty), spoken, "{written}");
        }
    }

    #[test]
    fn lexicon_terms_win_over_rules() {
        let l = lex();
        assert_eq!(to_speech("We use PostgreSQL and gRPC, e.g. for C++ services.", &l),
            "We use Postgres Q L and G R P C, for example for C plus plus services.");
        assert_eq!(to_speech("It runs 24/7 on k8s.", &l), "It runs twenty four seven on kubernetes.");
        // A term inside a longer word is not touched.
        assert_eq!(to_speech("The SQLite file.", &l), "The SQLite file.");
    }

    #[test]
    fn codes_and_mixed_tokens() {
        let e = Lexicon::default();
        assert_eq!(to_speech("Send an MT103.", &e), "Send an M T one oh three.");
        assert_eq!(to_speech("Store it in S3 with sha256.", &e), "Store it in S three with sha two five six.");
        assert_eq!(to_speech("Use Node.js here.", &e), "Use Node dot js here.");
    }

    #[test]
    fn review_focus_inputs() {
        let e = Lexicon::default();
        // 1. Nothing left to say.
        assert_eq!(to_speech("**\u{1F600}** ()", &e), "");
        assert_eq!(to_speech("   ", &e), "");
        // 2. A huge token is never read out.
        let blob = "A".repeat(300);
        assert_eq!(to_speech(&format!("The key is {blob} today."), &e), "The key is a long value today.");
        // 3. Names with accents pass through.
        assert_eq!(to_speech("Thanks, Jos\u{e9} and Zo\u{eb}.", &e), "Thanks, Jos\u{e9} and Zo\u{eb}.");
        // Plain sentences are unchanged.
        let plain = "Walk me through a project you are proud of.";
        assert_eq!(to_speech(plain, &e), plain);
    }

    #[test]
    fn bad_lexicon_file_gives_an_empty_lexicon() {
        assert!(Lexicon::from_json("{not json").is_err());
        let dir = std::env::temp_dir().join("onsite-no-such-prompts");
        assert_eq!(to_speech("SQL", &Lexicon::load(&dir)), "SQL");
    }
}
