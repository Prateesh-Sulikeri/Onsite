//! Pure scoring for the voice bench: word error rate, missing words, term recall.

/// Lowercase words with every non-alphanumeric character removed. Hyphens split words.
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == '-')
        .map(|t| t.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wer {
    pub edits: usize,
    pub ref_words: usize,
    /// Reference words deleted or substituted in the best alignment, in order.
    pub missing: Vec<String>,
}

impl Wer {
    pub fn rate(&self) -> f32 {
        if self.ref_words == 0 {
            0.0
        } else {
            self.edits as f32 / self.ref_words as f32
        }
    }
}

/// Word-level Levenshtein distance between a reference and a hypothesis.
pub fn wer(reference: &str, hyp: &str) -> Wer {
    let r = words(reference);
    let h = words(hyp);
    let (n, m) = (r.len(), h.len());
    if n == 0 {
        return Wer { edits: 0, ref_words: 0, missing: Vec::new() };
    }
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let sub = d[i - 1][j - 1] + usize::from(r[i - 1] != h[j - 1]);
            d[i][j] = sub.min(d[i - 1][j] + 1).min(d[i][j - 1] + 1);
        }
    }
    let mut missing = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && d[i][j] == d[i - 1][j - 1] + usize::from(r[i - 1] != h[j - 1]) {
            if r[i - 1] != h[j - 1] {
                missing.push(r[i - 1].clone());
            }
            i -= 1;
            j -= 1;
        } else if i > 0 && d[i][j] == d[i - 1][j] + 1 {
            missing.push(r[i - 1].clone());
            i -= 1;
        } else {
            j -= 1;
        }
    }
    missing.reverse();
    Wer { edits: d[n][m], ref_words: n, missing }
}

/// (terms found, terms total). Case and spacing do not matter: "postgre sql" finds "PostgreSQL".
pub fn term_recall(hyp: &str, terms: &[String]) -> (usize, usize) {
    let h = words(hyp);
    let found = terms
        .iter()
        .filter(|term| {
            let tw = words(term);
            if tw.is_empty() {
                return false;
            }
            let want: String = tw.concat();
            let span = tw.len() + 1;
            (0..h.len()).any(|start| {
                (1..=span).any(|len| start + len <= h.len() && h[start..start + len].concat() == want)
            })
        })
        .count();
    (found, terms.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_strip_punctuation_and_case() {
        assert_eq!(words("Hello, World! It's read-only."), ["hello", "world", "its", "read", "only"]);
        assert!(words("  ... ").is_empty());
    }

    #[test]
    fn wer_counts_edits_and_missing_words() {
        let w = wer("the cache is in Redis today", "the cash is in today");
        assert_eq!((w.edits, w.ref_words), (2, 6));
        assert_eq!(w.missing, ["cache", "redis"]);
        assert!((w.rate() - 2.0 / 6.0).abs() < 1e-6);
        assert_eq!(wer("", "anything").rate(), 0.0);
        assert_eq!(wer("same words", "Same, words.").edits, 0);
    }

    #[test]
    fn term_recall_ignores_case_and_spacing() {
        let terms: Vec<String> = ["PostgreSQL", "Spring Boot", "Kafka"].map(String::from).to_vec();
        assert_eq!(term_recall("we use postgre sql and spring boot", &terms), (2, 3));
        assert_eq!(term_recall("", &terms), (0, 3));
    }
}
