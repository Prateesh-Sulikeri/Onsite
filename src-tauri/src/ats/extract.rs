//! What an ATS gets out of a resume file: plain text plus the layout facts that break parsers.
//! The type comes from the file's bytes, not its name, so a renamed file is still read right.
//! Resume text never leaves this module except to the checks; it is not sent to Claude.

use crate::error::{AppError, Result};
use pdf_extract::{Document as PdfDoc, MediaBox, OutputDev, OutputError, Transform};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Serialize;
use std::io::Read;

/// Bigger than any real resume; keeps a stray video or archive from stalling the app.
pub const MAX_BYTES: usize = 10 * 1024 * 1024;
/// Pages read for checks. Page counting covers the whole file.
const MAX_PAGES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Pdf,
    Docx,
    Rtf,
    Text,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Pdf => "PDF",
            Kind::Docx => "Word (.docx)",
            Kind::Rtf => "Rich text (.rtf)",
            Kind::Text => "Plain text",
        }
    }
}

/// One run of text on a PDF page, in top-down page coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct PageLayout {
    pub width: f64,
    pub height: f64,
    pub boxes: Vec<TextBox>,
    pub images: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub kind: Kind,
    pub text: String,
    pub page_count: Option<usize>,
    pub truncated: bool,
    pub pages: Vec<PageLayout>,
    pub images: usize,
    pub tables: usize,
    pub columns: usize,
    pub header_footer_text: String,
    pub textbox_text: String,
}

impl Document {
    fn new(kind: Kind, text: String) -> Self {
        Self {
            kind,
            text,
            page_count: None,
            truncated: false,
            pages: vec![],
            images: 0,
            tables: 0,
            columns: 1,
            header_footer_text: String::new(),
            textbox_text: String::new(),
        }
    }

    /// Text typed or pasted into the app: no layout to check.
    pub fn from_text(text: &str) -> Self {
        Self::new(Kind::Text, clean_text(text))
    }
}

fn unsupported(msg: &str) -> AppError {
    AppError::Invalid(msg.into())
}

/// Reads a resume file. `name` is only used to word error messages.
pub fn extract(name: &str, bytes: &[u8]) -> Result<Document> {
    if bytes.is_empty() {
        return Err(unsupported("That file is empty."));
    }
    if bytes.len() > MAX_BYTES {
        return Err(unsupported("That file is over 10 MB. A resume should be far smaller."));
    }
    let head = &bytes[..bytes.len().min(1024)];
    if find(head, b"%PDF-").is_some() {
        return extract_pdf(bytes);
    }
    if bytes.starts_with(b"PK\x03\x04") {
        return extract_zip(name, bytes);
    }
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        return Err(unsupported("This is an old Word .doc file. Save it as .docx or PDF and try again."));
    }
    if bytes.starts_with(b"{\\rtf") {
        let mut doc = Document::new(Kind::Rtf, clean_text(&rtf_to_text(&String::from_utf8_lossy(bytes))));
        doc.tables = count(bytes, b"\\trowd");
        doc.images = count(bytes, b"\\pict");
        return Ok(doc);
    }
    if bytes.starts_with(b"\x89PNG") || bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || bytes.starts_with(b"GIF8") {
        return Err(unsupported("This is an image. An ATS can't read text in a picture. Use the PDF or Word file."));
    }
    let text = decode_text(bytes).ok_or_else(|| {
        unsupported("Onsite can't read this file type. Use PDF, Word (.docx), .rtf, .txt or .md.")
    })?;
    let lower = name.to_ascii_lowercase();
    let text = if lower.ends_with(".html") || lower.ends_with(".htm") { strip_tags(&text) } else { text };
    Ok(Document::new(Kind::Text, clean_text(&text)))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn count(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

/// UTF-8 (with or without BOM) or UTF-16 with a BOM. Binary data is refused.
fn decode_text(bytes: &[u8]) -> Option<String> {
    let utf16 = |b: &[u8], le: bool| {
        let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) }).collect();
        String::from_utf16(&units).ok()
    };
    let text = if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        utf16(rest, true)?
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest, false)?
    } else {
        let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        String::from_utf8(b.to_vec()).ok()?
    };
    let control = text.chars().filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\x0c')).count();
    (control * 100 <= text.chars().count().max(1)).then_some(text)
}

/// Unix line ends, no trailing spaces, no runs of blank lines.
fn clean_text(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace(['\r', '\x0c'], "\n");
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

fn strip_tags(html: &str) -> String {
    let re = regex::Regex::new(r"(?is)<(script|style)\b.*?</(script|style)>|<br\s*/?>|</(p|div|li|h\d|tr)>|<[^>]+>").expect("valid regex");
    let text = re.replace_all(html, |c: &regex::Captures| {
        let m = c[0].to_ascii_lowercase();
        if m.starts_with("<br") || m.starts_with("</") && !m.starts_with("</script") && !m.starts_with("</style") { "\n" } else { "" }
    });
    text.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'")
}

// ---------------------------------------------------------------- PDF

/// Collects text the way pdf-extract's plain-text output does, plus one box per run of
/// adjacent characters, so the checks can see columns and margins.
#[derive(Default)]
struct Collector {
    pages: Vec<PageLayout>,
    flip: Option<Transform>,
    text: String,
    boxes: Vec<TextBox>,
    run: Option<TextBox>,
    last_end: f64,
    last_y: f64,
    first_char: bool,
    started: bool,
}

impl Collector {
    fn close_run(&mut self) {
        if let Some(b) = self.run.take() {
            if !b.text.trim().is_empty() {
                self.boxes.push(b);
            }
        }
    }
}

impl OutputDev for Collector {
    fn begin_page(&mut self, _n: u32, mb: &MediaBox, _art: Option<(f64, f64, f64, f64)>) -> std::result::Result<(), OutputError> {
        self.flip = Some(Transform::row_major(1., 0., 0., -1., 0., mb.ury - mb.lly));
        self.pages.push(PageLayout { width: (mb.urx - mb.llx).abs(), height: (mb.ury - mb.lly).abs(), ..Default::default() });
        self.text.clear();
        self.boxes.clear();
        self.started = false;
        self.last_end = 100_000.;
        Ok(())
    }

    fn end_page(&mut self) -> std::result::Result<(), OutputError> {
        self.close_run();
        if let Some(p) = self.pages.last_mut() {
            p.text = std::mem::take(&mut self.text);
            p.boxes = std::mem::take(&mut self.boxes);
        }
        Ok(())
    }

    fn output_character(&mut self, trm: &Transform, width: f64, _spacing: f64, font_size: f64, ch: &str) -> std::result::Result<(), OutputError> {
        let flip = self.flip.unwrap_or_else(Transform::identity);
        let pos = trm.post_transform(&flip);
        // Font size in page units: the square root of the area of the transformed em square.
        let (vx, vy) = (font_size * (trm.m11 + trm.m21), font_size * (trm.m12 + trm.m22));
        let fs = (vx * vy).abs().sqrt().max(0.1);
        let (x, y) = (pos.m31, pos.m32);
        if self.first_char && self.started {
            let dy = (y - self.last_y).abs();
            if dy > fs * 1.5 || (x < self.last_end && dy > fs * 0.5) {
                self.text.push('\n');
            } else if x > self.last_end + fs * 0.1 {
                self.text.push(' ');
            }
        }
        self.text.push_str(ch);
        let end = x + width * fs;
        // Same line and no wide gap: extend the run. A gap of a few em is a column or a tab stop.
        let joins = self.run.as_ref().is_some_and(|r| (y - self.last_y).abs() <= fs * 0.5 && x <= r.x1 + fs * 2.0 && x >= r.x0 - fs);
        if joins {
            let r = self.run.as_mut().expect("checked");
            if self.first_char && x > r.x1 + fs * 0.1 {
                r.text.push(' ');
            }
            r.text.push_str(ch);
            r.x1 = r.x1.max(end);
            r.y0 = r.y0.min(y - fs);
            r.y1 = r.y1.max(y);
        } else {
            self.close_run();
            self.run = Some(TextBox { x0: x, y0: y - fs, x1: end, y1: y, text: ch.to_string() });
        }
        self.first_char = false;
        self.started = true;
        self.last_y = y;
        self.last_end = end;
        Ok(())
    }

    fn begin_word(&mut self) -> std::result::Result<(), OutputError> {
        self.first_char = true;
        Ok(())
    }
    fn end_word(&mut self) -> std::result::Result<(), OutputError> {
        Ok(())
    }
    fn end_line(&mut self) -> std::result::Result<(), OutputError> {
        Ok(())
    }
}

fn extract_pdf(bytes: &[u8]) -> Result<Document> {
    let mut pdf = PdfDoc::load_mem(bytes).map_err(|e| {
        tracing::warn!("PDF load failed: {e}");
        unsupported("This PDF is damaged or not a real PDF. Export it again from your editor.")
    })?;
    if pdf.is_encrypted() && pdf.decrypt("").is_err() {
        return Err(unsupported("This PDF is password-protected. An ATS can't open it. Save a copy without a password."));
    }
    let page_ids = pdf.get_pages();
    let page_count = page_ids.len();
    let mut out = Collector::default();
    for (&num, &id) in page_ids.iter().take(MAX_PAGES) {
        let before = out.pages.len();
        // pdf-extract panics on some broken fonts. Lose that page, not the whole check.
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdf_extract::output_doc_page(&pdf, &mut out, num)));
        match res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(page = num, "PDF page text failed: {e}"),
            Err(_) => tracing::warn!(page = num, "PDF page text panicked"),
        }
        if out.pages.len() == before {
            out.pages.push(PageLayout::default());
        } else if out.pages.len() > before + 1 {
            out.pages.truncate(before + 1);
        }
        let page = out.pages.last_mut().expect("pushed above");
        page.images = pdf.get_page_images(id).map(|v| v.len()).unwrap_or(0);
    }
    let text = clean_text(&out.pages.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n\n"));
    let mut doc = Document::new(Kind::Pdf, text);
    doc.images = out.pages.iter().map(|p| p.images).sum();
    doc.page_count = Some(page_count);
    doc.truncated = page_count > MAX_PAGES;
    doc.pages = out.pages;
    Ok(doc)
}

// ---------------------------------------------------------------- DOCX

fn extract_zip(name: &str, bytes: &[u8]) -> Result<Document> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| unsupported("This file looks like a damaged Word or zip file."))?;
    let mut read = |path: &str| -> Option<String> {
        let mut f = zip.by_name(path).ok()?;
        // A zip bomb inflates far past any resume; stop reading at 20 MB.
        let mut s = String::new();
        (&mut f).take(20 * 1024 * 1024).read_to_string(&mut s).ok()?;
        Some(s)
    };
    let Some(body) = read("word/document.xml") else {
        let lower = name.to_ascii_lowercase();
        let what = if lower.ends_with(".odt") { "an OpenDocument (.odt) file" } else if lower.ends_with(".pages") { "a Pages file" } else { "not a Word document" };
        return Err(unsupported(&format!("This is {what}. Save it as PDF or Word (.docx) and try again.")));
    };
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(String::from)).collect();
    let mut doc = parse_docx_body(&body)?;
    let mut margin = Vec::new();
    for n in names.iter().filter(|n| is_header_footer(n)) {
        let mut zip_read = || -> Option<String> {
            let mut f = zip.by_name(n).ok()?;
            let mut s = String::new();
            (&mut f).take(5 * 1024 * 1024).read_to_string(&mut s).ok()?;
            Some(s)
        };
        if let Some(xml) = zip_read() {
            let part = parse_docx_body(&xml)?;
            if !part.text.trim().is_empty() {
                margin.push(part.text);
            }
            doc.images += part.images;
        }
    }
    doc.header_footer_text = margin.join("\n");
    Ok(doc)
}

fn is_header_footer(n: &str) -> bool {
    let f = n.strip_prefix("word/").unwrap_or("");
    (f.starts_with("header") || f.starts_with("footer")) && f.ends_with(".xml") && !f.contains('/')
}

/// Body paragraphs and tables in order. Text boxes are kept apart: many ATSs skip them.
/// `mc:Fallback` repeats the `mc:Choice` content for old readers, so it is skipped.
fn parse_docx_body(xml: &str) -> Result<Document> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut lines: Vec<String> = vec![];
    let mut boxes: Vec<String> = vec![];
    let mut para = String::new();
    let mut cell = String::new();
    let mut row: Vec<String> = vec![];
    let (mut in_t, mut fallback, mut txbx, mut tbl) = (false, 0usize, 0usize, 0usize);
    let (mut tables, mut images, mut columns) = (0usize, 0usize, 1usize);
    loop {
        let ev = r.read_event().map_err(|e| {
            tracing::warn!("docx XML: {e}");
            unsupported("This Word file is damaged. Save it again from Word and retry.")
        })?;
        match ev {
            Event::Eof => break,
            Event::Start(e) if fallback > 0 => {
                if e.name().as_ref() == "mc:Fallback" {
                    fallback += 1;
                }
            }
            Event::End(e) if fallback > 0 => {
                if e.name().as_ref() == "mc:Fallback" {
                    fallback -= 1;
                }
            }
            _ if fallback > 0 => {}
            Event::Start(e) => match e.name().as_ref() {
                "mc:Fallback" => fallback += 1,
                "w:t" => in_t = true,
                "w:txbxContent" => txbx += 1,
                "w:tbl" => {
                    if tbl == 0 && txbx == 0 {
                        tables += 1;
                    }
                    tbl += 1;
                }
                "pic:pic" => images += 1,
                _ => {}
            },
            Event::Empty(e) => match e.name().as_ref() {
                "w:tab" => para.push('\t'),
                "w:br" | "w:cr" => para.push('\n'),
                "v:imagedata" => images += 1,
                "w:cols" => {
                    if let Ok(Some(a)) = e.try_get_attribute("w:num") {
                        columns = columns.max(a.value.parse().unwrap_or(1));
                    }
                }
                _ => {}
            },
            Event::Text(t) if in_t => para.push_str(&t),
            Event::GeneralRef(g) if in_t => {
                let name: &str = &g;
                match g.resolve_char_ref() {
                    Ok(Some(c)) => para.push(c),
                    _ => para.push_str(match name {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    }),
                }
            }
            Event::End(e) => match e.name().as_ref() {
                "w:t" => in_t = false,
                "w:p" => {
                    let p = std::mem::take(&mut para);
                    if txbx > 0 {
                        boxes.push(p);
                    } else if tbl > 0 {
                        if !cell.is_empty() && !p.trim().is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(p.trim());
                    } else {
                        lines.push(p);
                    }
                }
                "w:txbxContent" => txbx = txbx.saturating_sub(1),
                "w:tc" if tbl == 1 => {
                    let c = std::mem::take(&mut cell);
                    if !c.trim().is_empty() {
                        row.push(c);
                    }
                }
                "w:tr" if tbl == 1 => lines.push(std::mem::take(&mut row).join(" | ")),
                "w:tbl" => tbl = tbl.saturating_sub(1),
                _ => {}
            },
            _ => {}
        }
    }
    let mut doc = Document::new(Kind::Docx, clean_text(&lines.join("\n")));
    doc.tables = tables;
    doc.images = images;
    doc.columns = columns;
    doc.textbox_text = clean_text(&boxes.join("\n"));
    Ok(doc)
}

// ---------------------------------------------------------------- RTF

/// Good enough for resumes: drops control words and destination groups (fonts, colours,
/// pictures, metadata), keeps paragraph breaks, tabs, hex and Unicode escapes.
pub fn rtf_to_text(rtf: &str) -> String {
    const SKIP: [&str; 10] = ["fonttbl", "colortbl", "stylesheet", "info", "pict", "header", "footer", "listtable", "listoverridetable", "themedata"];
    let b = rtf.as_bytes();
    let mut out = String::new();
    // Per group: skipping?  The stack holds the state to restore at '}'.
    let mut stack: Vec<bool> = vec![];
    let mut skip = false;
    let mut uc_skip = 0usize;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' => {
                stack.push(skip);
                i += 1;
            }
            b'}' => {
                skip = stack.pop().unwrap_or(false);
                i += 1;
            }
            b'\\' => {
                i += 1;
                if i >= b.len() {
                    break;
                }
                let c = b[i];
                if c == b'*' {
                    skip = true;
                    i += 1;
                } else if c == b'\'' {
                    if let Some(h) = rtf.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                        if !skip {
                            // Windows-1252 covers the bullets and dashes resumes use.
                            out.push(cp1252(h));
                        }
                    }
                    i += 3;
                } else if c.is_ascii_alphabetic() {
                    let start = i;
                    while i < b.len() && b[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word = &rtf[start..i];
                    let nstart = i;
                    if i < b.len() && (b[i] == b'-' || b[i].is_ascii_digit()) {
                        i += 1;
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                    let num: Option<i32> = rtf[nstart..i].parse().ok();
                    if i < b.len() && b[i] == b' ' {
                        i += 1;
                    }
                    if SKIP.contains(&word) {
                        skip = true;
                    }
                    if skip {
                        continue;
                    }
                    match word {
                        "par" | "line" | "row" | "sect" | "page" => out.push('\n'),
                        "tab" | "cell" => out.push('\t'),
                        "emdash" => out.push('—'),
                        "endash" => out.push('–'),
                        "bullet" => out.push('•'),
                        "lquote" | "rquote" => out.push('\''),
                        "ldblquote" | "rdblquote" => out.push('"'),
                        "uc" => uc_skip = num.unwrap_or(1).max(0) as usize,
                        "u" => {
                            if let Some(n) = num {
                                let n = if n < 0 { n + 65536 } else { n } as u32;
                                if let Some(ch) = char::from_u32(n) {
                                    out.push(ch);
                                }
                                // Skip the ASCII fallback that follows \uN.
                                let mut k = 0;
                                while k < uc_skip.max(1) && i < b.len() && b[i] != b'\\' && b[i] != b'{' && b[i] != b'}' {
                                    i += 1;
                                    k += 1;
                                }
                            }
                        }
                        _ => {}
                    }
                } else {
                    // Escaped literal: \\ \{ \} or \~ (non-breaking space).
                    if !skip {
                        match c {
                            b'~' => out.push(' '),
                            b'\n' | b'\r' => out.push('\n'),
                            _ => out.push(c as char),
                        }
                    }
                    i += 1;
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                let start = i;
                while i < b.len() && !matches!(b[i], b'{' | b'}' | b'\\' | b'\r' | b'\n') {
                    i += 1;
                }
                if !skip {
                    out.push_str(&String::from_utf8_lossy(&b[start..i]));
                }
            }
        }
    }
    out
}

fn cp1252(b: u8) -> char {
    match b {
        0x91 | 0x92 => '\'',
        0x93 | 0x94 => '"',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x80 => '€',
        _ => b as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_by_content_not_name() {
        let doc = extract("resume.pdf", b"Jane Doe\nExperience\nBackend engineer").unwrap();
        assert_eq!(doc.kind, Kind::Text);
        assert!(doc.text.contains("Backend engineer"));
    }

    #[test]
    fn refuses_legacy_doc_and_images() {
        let doc = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        assert!(extract("cv.doc", &doc).unwrap_err().to_string().contains(".docx"));
        assert!(extract("cv.png", b"\x89PNG\r\n\x1a\n....").unwrap_err().to_string().contains("image"));
        assert!(extract("x.bin", &[0u8, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0]).is_err());
        assert!(extract("x.txt", b"").is_err());
    }

    #[test]
    fn utf16_and_bom_text() {
        let mut b = vec![0xFF, 0xFE];
        for u in "Résumé – Kafka".encode_utf16() {
            b.extend(u.to_le_bytes());
        }
        assert_eq!(extract("a.txt", &b).unwrap().text, "Résumé – Kafka");
        assert_eq!(extract("a.txt", b"\xEF\xBB\xBFhi\r\n\r\n\r\nthere").unwrap().text, "hi\n\nthere");
    }

    #[test]
    fn rtf_basics() {
        let rtf = r"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\*\generator Word;}\f0 Jane Doe\par Skills: Java, Kafka\tab Redis\par caf\'e9 \u8212? done}";
        let t = rtf_to_text(rtf);
        assert!(t.contains("Jane Doe\nSkills: Java, Kafka\tRedis\n"), "{t:?}");
        assert!(t.contains("café — done"), "{t:?}");
        assert!(!t.contains("Arial") && !t.contains("Word"));
    }

    #[test]
    fn html_tags_stripped() {
        let doc = extract("r.html", b"<html><style>p{}</style><p>Jane</p><p>Java &amp; Go</p></html>").unwrap();
        assert_eq!(doc.text, "Jane\nJava & Go");
    }
}
