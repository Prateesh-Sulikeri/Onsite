//! Cuts streaming interviewer text into speakable sentences and fenced code blocks.
//! Code never goes to TTS. `[[END]]` is stripped and remembered.

#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Sentence(String),
    Code { lang: String, code: String },
}

pub const END_MARKER: &str = "[[END]]";
/// Case-sensitive, so "1.5 ms." still ends a sentence while "Ms." does not.
const ABBREVIATIONS: [&str; 10] = ["e.g", "i.e", "E.g", "I.e", "vs", "etc", "Mr", "Mrs", "Ms", "Dr"];

#[derive(Debug, Default)]
pub struct SentenceSplitter {
    buf: String,
    in_fence: bool,
    lang: String,
    code: String,
    ended: bool,
}

impl SentenceSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, delta: &str) -> Vec<Piece> {
        self.buf.push_str(delta);
        self.drain(false)
    }

    /// Flushes whatever is left. Returns the pieces and whether `[[END]]` was seen.
    pub fn finish(&mut self) -> (Vec<Piece>, bool) {
        let pieces = self.drain(true);
        let ended = std::mem::take(&mut self.ended);
        *self = Self::default();
        (pieces, ended)
    }

    fn drain(&mut self, eof: bool) -> Vec<Piece> {
        let mut out = Vec::new();
        loop {
            if self.in_fence {
                if let Some(nl) = self.buf.find('\n') {
                    let line: String = self.buf.drain(..=nl).collect();
                    if line.trim_start().starts_with("```") {
                        self.close_fence(&mut out);
                    } else {
                        self.code.push_str(&line);
                    }
                    continue;
                }
                if eof {
                    let rest = std::mem::take(&mut self.buf);
                    if !rest.trim_start().starts_with("```") {
                        self.code.push_str(&rest);
                    }
                    self.close_fence(&mut out);
                }
                break;
            }

            if let Some(i) = self.buf.find("```") {
                let prose: String = self.buf.drain(..i).collect();
                self.emit_prose(&prose, true, &mut out);
                match self.buf.find('\n') {
                    Some(nl) => {
                        let opener: String = self.buf.drain(..=nl).collect();
                        self.lang = opener.trim().trim_start_matches('`').trim().to_string();
                        self.in_fence = true;
                        continue;
                    }
                    None if eof => self.buf.clear(),
                    None => {}
                }
                break;
            }

            if eof {
                let rest = std::mem::take(&mut self.buf);
                self.emit_prose(&rest, true, &mut out);
                break;
            }
            // Keep trailing backticks: they may be the start of a fence.
            let scan_len = self.buf.trim_end_matches('`').len();
            let consumed = complete_prefix_len(&self.buf[..scan_len]);
            let prose: String = self.buf.drain(..consumed).collect();
            self.emit_prose(&prose, false, &mut out);
            break;
        }
        out
    }

    fn close_fence(&mut self, out: &mut Vec<Piece>) {
        let code = std::mem::take(&mut self.code);
        let lang = std::mem::take(&mut self.lang);
        self.in_fence = false;
        let code = code.trim_end_matches('\n').to_string();
        if !code.trim().is_empty() {
            out.push(Piece::Code { lang, code });
        }
    }

    /// `text` is either a run of complete sentences, or (when `force`) anything left over.
    fn emit_prose(&mut self, text: &str, force: bool, out: &mut Vec<Piece>) {
        let mut rest = text;
        loop {
            let n = complete_prefix_len_first(rest);
            let (sentence, tail) = match n {
                Some(n) => rest.split_at(n),
                None if force => (rest, ""),
                None => break,
            };
            self.push_sentence(sentence, out);
            rest = tail;
            if rest.is_empty() {
                break;
            }
        }
    }

    fn push_sentence(&mut self, raw: &str, out: &mut Vec<Piece>) {
        let mut text = raw.to_string();
        if text.contains(END_MARKER) {
            self.ended = true;
            text = text.replace(END_MARKER, "");
        }
        let text = speakable(&text);
        if !text.is_empty() {
            out.push(Piece::Sentence(text));
        }
    }
}

/// Removes light markdown that the model sometimes slips into speech.
fn speakable(s: &str) -> String {
    let s: String = s.chars().filter(|c| !matches!(c, '*' | '`')).collect();
    s.trim().trim_start_matches('#').trim().to_string()
}

/// Byte length of the first complete sentence in `s`, including trailing whitespace.
fn complete_prefix_len_first(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    for (i, c) in s.char_indices() {
        let end = match c {
            '\n' => i + 1,
            '.' | '!' | '?' => {
                let mut j = i + 1;
                while j < bytes.len() && matches!(bytes[j], b'"' | b'\'' | b')' | b']') {
                    j += 1;
                }
                let followed_by_space = j < bytes.len() && bytes[j].is_ascii_whitespace();
                if !followed_by_space || (c == '.' && is_abbreviation(&s[..i])) {
                    continue;
                }
                j
            }
            _ => continue,
        };
        let mut end = end;
        while end < bytes.len() && bytes[end].is_ascii_whitespace() {
            end += 1;
        }
        return Some(end);
    }
    None
}

/// Byte length of all complete sentences at the start of `s`.
fn complete_prefix_len(s: &str) -> usize {
    let mut total = 0;
    while let Some(n) = complete_prefix_len_first(&s[total..]) {
        total += n;
    }
    total
}

fn is_abbreviation(before_dot: &str) -> bool {
    let word = before_dot.rsplit(|c: char| c.is_whitespace() || c == '(').next().unwrap_or("");
    ABBREVIATIONS.contains(&word)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(chunks: &[&str]) -> (Vec<Piece>, bool) {
        let mut sp = SentenceSplitter::new();
        let mut out: Vec<Piece> = chunks.iter().flat_map(|c| sp.push(c)).collect();
        let (rest, ended) = sp.finish();
        out.extend(rest);
        (out, ended)
    }

    fn sentence(s: &str) -> Piece {
        Piece::Sentence(s.into())
    }

    #[test]
    fn emits_sentences_as_they_complete() {
        let mut sp = SentenceSplitter::new();
        assert_eq!(sp.push("Hi there. How"), vec![sentence("Hi there.")]);
        assert_eq!(sp.push(" are you"), vec![]);
        assert_eq!(sp.push("? Good"), vec![sentence("How are you?")]);
        assert_eq!(sp.finish(), (vec![sentence("Good")], false));
    }

    #[test]
    fn code_fence_goes_to_code_not_speech() {
        let (out, ended) = feed(&[
            "Here it is. Take a look.\n\n``",
            "`java\nint x = 1;\n",
            "return x;\n``",
            "`\nWhat does it return?",
        ]);
        assert_eq!(
            out,
            vec![
                sentence("Here it is."),
                sentence("Take a look."),
                Piece::Code { lang: "java".into(), code: "int x = 1;\nreturn x;".into() },
                sentence("What does it return?"),
            ]
        );
        assert!(!ended);
    }

    #[test]
    fn real_fixture_text() {
        let text = "Here is a quick example. It prints a greeting.\n\n```python\nprint(\"hi\")\n```\n\nThat code outputs the word hi when run.\n\n[[END]]";
        let chunks: Vec<String> = text.chars().collect::<Vec<_>>().chunks(5).map(|c| c.iter().collect()).collect();
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let (out, ended) = feed(&refs);
        assert_eq!(
            out,
            vec![
                sentence("Here is a quick example."),
                sentence("It prints a greeting."),
                Piece::Code { lang: "python".into(), code: "print(\"hi\")".into() },
                sentence("That code outputs the word hi when run."),
            ]
        );
        assert!(ended);
    }

    #[test]
    fn end_marker_split_across_chunks_and_inline() {
        assert_eq!(feed(&["Thanks for your time.\n[[EN", "D]]"]), (vec![sentence("Thanks for your time.")], true));
        assert_eq!(feed(&["Bye. [[END]]"]), (vec![sentence("Bye.")], true));
    }

    #[test]
    fn keeps_abbreviations_and_decimals_together() {
        let (out, _) = feed(&["Use a map, e.g. a HashMap. It costs 1.5 ms. Okay?"]);
        assert_eq!(
            out,
            vec![sentence("Use a map, e.g. a HashMap."), sentence("It costs 1.5 ms."), sentence("Okay?")]
        );
    }

    #[test]
    fn strips_markdown_and_handles_unclosed_fence() {
        let (out, _) = feed(&["**Note** this.\n```\nfoo()"]);
        assert_eq!(out, vec![sentence("Note this."), Piece::Code { lang: String::new(), code: "foo()".into() }]);
    }

    #[test]
    fn unicode_is_safe() {
        let (out, _) = feed(&["Namaste — welcome. ", "Let’s start…"]);
        assert_eq!(out, vec![sentence("Namaste — welcome."), sentence("Let’s start…")]);
    }
}
