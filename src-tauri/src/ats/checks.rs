//! Can an ATS read this file? Deterministic checks on the extracted text and layout.
//! Ported from Resume-Matcher `services/ats_parse/checks.py` (Apache-2.0).

use super::extract::{Document, Kind, PageLayout, TextBox};
use super::text;
use serde::Serialize;

const MIN_TEXT_CHARS: usize = 50;
const SPARSE_PAGE_CHARS: usize = 200;
const COLUMN_MIN_COVERAGE: f64 = 0.05;
const COLUMN_MIN_BOXES: usize = 3;
const RIGHT_BAND_MAX_WIDTH: f64 = 0.3;
const RIGHT_BAND_EDGE_TOLERANCE: f64 = 2.0;
const RIGHT_BAND_MIN_FLUSH: f64 = 0.8;
const MAX_COMFORTABLE_PAGES: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Extraction,
    Layout,
    Content,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub category: Category,
    pub severity: Severity,
    pub status: Status,
    /// One short line for the list.
    pub title: String,
    /// Why it matters / what to do. Empty on a pass.
    pub detail: String,
}

fn check(id: &'static str, category: Category, severity: Severity, status: Status, title: impl Into<String>, detail: impl Into<String>) -> Check {
    let detail = if status == Status::Pass { String::new() } else { detail.into() };
    Check { id, category, severity, status, title: title.into(), detail }
}

fn visible_chars(t: &str) -> usize {
    t.chars().filter(|c| !c.is_whitespace()).count()
}

fn pages_list(p: &[usize]) -> String {
    p.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
}

pub fn text_layer(doc: &Document) -> Check {
    let n = visible_chars(&doc.text);
    if n >= MIN_TEXT_CHARS {
        check("text_layer", Category::Extraction, Severity::High, Status::Pass, format!("Text can be read ({n} characters)"), "")
    } else {
        check(
            "text_layer", Category::Extraction, Severity::High, Status::Fail,
            "No readable text",
            "The ATS sees an empty page. This happens with scanned resumes or text saved as an image. Export it again as a text PDF or .docx.",
        )
    }
}

pub fn garbled_glyphs(doc: &Document) -> Check {
    let count = doc.text.matches("(cid:").count() + doc.text.chars().filter(|&c| c == '\u{fffd}' || ('\u{e000}'..='\u{f8ff}').contains(&c)).count();
    let ratio = count as f64 / visible_chars(&doc.text).max(1) as f64;
    let status = if ratio > 0.05 { Status::Fail } else if ratio > 0.005 { Status::Warn } else { Status::Pass };
    check(
        "garbled_glyphs", Category::Extraction, Severity::High, status,
        if status == Status::Pass { "No garbled characters".to_string() } else { format!("{count} characters come out garbled") },
        "Some fonts or icons turn into junk when read. Use a standard font and drop icon fonts.",
    )
}

pub fn text_coverage(doc: &Document) -> Option<Check> {
    if doc.kind != Kind::Pdf || doc.pages.is_empty() {
        return None;
    }
    let sparse: Vec<usize> = doc.pages.iter().enumerate().filter(|(_, p)| p.images > 0 && visible_chars(&p.text) < SPARSE_PAGE_CHARS).map(|(i, _)| i + 1).collect();
    let status = if sparse.is_empty() { Status::Pass } else if sparse.len() == doc.pages.len() { Status::Fail } else { Status::Warn };
    Some(check(
        "text_coverage", Category::Extraction, Severity::Medium, status,
        if sparse.is_empty() { "Every page has real text".to_string() } else { format!("Page {} looks like a picture", pages_list(&sparse)) },
        "Those pages are mostly images with little text. An ATS can't read text inside images.",
    ))
}

fn union_length(intervals: &mut [(f64, f64)]) -> f64 {
    intervals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let (mut total, mut end) = (0.0, f64::NEG_INFINITY);
    for &(lo, hi) in intervals.iter() {
        if hi > end {
            total += hi - lo.max(end);
            end = hi;
        }
    }
    total
}

fn free_bands(mut blocked: Vec<(f64, f64)>, height: f64) -> Vec<(f64, f64)> {
    blocked.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut bands = vec![];
    let mut cursor = 0.0;
    for (lo, hi) in blocked {
        if lo > cursor {
            bands.push((cursor, lo));
        }
        cursor = f64::max(cursor, hi);
    }
    if cursor < height {
        bands.push((cursor, height));
    }
    bands
}

/// Narrow boxes all ending at one right edge: dates beside entry titles, not a sidebar.
fn is_right_aligned_band(boxes: &[&TextBox], width: f64) -> bool {
    let edge = boxes.iter().map(|b| b.x1).fold(f64::MIN, f64::max);
    let flush = boxes.iter().filter(|b| edge - b.x1 <= RIGHT_BAND_EDGE_TOLERANCE).count();
    let widest = boxes.iter().map(|b| b.x1 - b.x0).fold(0.0, f64::max);
    flush as f64 >= RIGHT_BAND_MIN_FLUSH * boxes.len() as f64 && widest < RIGHT_BAND_MAX_WIDTH * width
}

/// A vertical gutter separating two text columns over a large span. Full-width lines cross
/// every gutter in a one-column layout, so right-aligned dates don't count as a column.
pub fn has_columns(page: &PageLayout) -> bool {
    if page.height <= 0.0 || page.boxes.len() < 4 {
        return false;
    }
    for step in (25..=75).step_by(2) {
        let gutter = page.width * step as f64 / 100.0;
        let blocked = page.boxes.iter().filter(|b| b.x0 < gutter && gutter < b.x1).map(|b| (b.y0, b.y1)).collect();
        for (lo, hi) in free_bands(blocked, page.height) {
            if hi - lo <= 0.0 {
                continue;
            }
            let mut left: Vec<(f64, f64)> = page.boxes.iter().filter(|b| b.x1 <= gutter && b.y1 > lo && b.y0 < hi).map(|b| (b.y0.max(lo), b.y1.min(hi))).collect();
            let right_boxes: Vec<&TextBox> = page.boxes.iter().filter(|b| b.x0 >= gutter && b.y1 > lo && b.y0 < hi).collect();
            if left.len().min(right_boxes.len()) < COLUMN_MIN_BOXES || is_right_aligned_band(&right_boxes, page.width) {
                continue;
            }
            let mut right: Vec<(f64, f64)> = right_boxes.iter().map(|b| (b.y0.max(lo), b.y1.min(hi))).collect();
            if union_length(&mut left).min(union_length(&mut right)) / page.height >= COLUMN_MIN_COVERAGE {
                return true;
            }
        }
    }
    false
}

pub fn multi_column(doc: &Document) -> Option<Check> {
    let pages: Vec<usize> = match doc.kind {
        Kind::Docx => if doc.columns > 1 { vec![1] } else { vec![] },
        Kind::Pdf => doc.pages.iter().enumerate().filter(|(_, p)| has_columns(p)).map(|(i, _)| i + 1).collect(),
        _ => return None,
    };
    let status = if pages.is_empty() { Status::Pass } else { Status::Fail };
    Some(check(
        "multi_column", Category::Layout, Severity::Medium, status,
        if pages.is_empty() { "Single column".to_string() } else if doc.kind == Kind::Pdf { format!("Two columns on page {}", pages_list(&pages)) } else { "Two columns".to_string() },
        "Many ATSs read straight across both columns, mixing lines from each. Use one column.",
    ))
}

pub fn tables(doc: &Document) -> Option<Check> {
    if !matches!(doc.kind, Kind::Docx | Kind::Rtf) {
        return None;
    }
    let status = if doc.tables > 0 { Status::Warn } else { Status::Pass };
    Some(check(
        "tables", Category::Layout, Severity::Medium, status,
        if doc.tables > 0 { format!("{} table(s)", doc.tables) } else { "No tables".to_string() },
        "Older ATSs (Taleo, some Workday setups) skip or scramble table cells. Use plain lines.",
    ))
}

pub fn text_boxes(doc: &Document) -> Option<Check> {
    if doc.kind != Kind::Docx {
        return None;
    }
    let n = visible_chars(&doc.textbox_text);
    let status = if n >= MIN_TEXT_CHARS { Status::Fail } else if n > 0 { Status::Warn } else { Status::Pass };
    Some(check(
        "text_boxes", Category::Layout, Severity::Medium, status,
        if n > 0 { format!("{n} characters sit in text boxes") } else { "No text boxes".to_string() },
        "Most ATSs never read text boxes. Move that text into the page body.",
    ))
}

fn repeated_margin_lines(pages: &[PageLayout]) -> usize {
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for p in pages {
        let set: std::collections::HashSet<String> = p.boxes.iter().filter(|b| b.y0 > p.height * 0.92 || b.y1 < p.height * 0.08).map(|b| text::normalize(&b.text)).filter(|t| !t.is_empty()).collect();
        for t in set {
            *seen.entry(t).or_default() += 1;
        }
    }
    seen.values().filter(|&&n| n >= 2).count()
}

pub fn header_footer(doc: &Document) -> Option<Check> {
    match doc.kind {
        Kind::Docx => {
            let only_there = text::EMAIL.is_match(&doc.header_footer_text) && !text::EMAIL.is_match(&doc.text);
            let status = if only_there { Status::Fail } else if !doc.header_footer_text.trim().is_empty() { Status::Warn } else { Status::Pass };
            Some(check(
                "header_footer", Category::Layout, Severity::Medium, status,
                if only_there { "Contact details only in the page header".to_string() } else if status == Status::Warn { "Text in the page header or footer".to_string() } else { "Nothing hidden in headers or footers".to_string() },
                "Workday and Taleo often drop page headers and footers. Keep your name and contact details in the body.",
            ))
        }
        Kind::Pdf if doc.pages.len() >= 2 => {
            let n = repeated_margin_lines(&doc.pages);
            let status = if n > 0 { Status::Warn } else { Status::Pass };
            Some(check(
                "header_footer", Category::Layout, Severity::Medium, status,
                if n > 0 { format!("{n} line(s) repeat in every page margin") } else { "No repeated header or footer".to_string() },
                "Repeated header or footer lines get mixed into your experience text. Keep them short, or drop them.",
            ))
        }
        _ => None,
    }
}

pub fn images(doc: &Document) -> Option<Check> {
    if doc.kind == Kind::Text {
        return None;
    }
    let status = if doc.images > 0 { Status::Warn } else { Status::Pass };
    Some(check(
        "images", Category::Layout, Severity::Low, status,
        if doc.images > 0 { format!("{} image(s)", doc.images) } else { "No images".to_string() },
        "Photos, logos and skill bars add nothing an ATS can read. Anything said only in an image is lost.",
    ))
}

pub fn page_count(doc: &Document) -> Option<Check> {
    let pages = doc.page_count?;
    let status = if doc.truncated || pages > MAX_COMFORTABLE_PAGES { Status::Warn } else { Status::Pass };
    Some(check(
        "page_count", Category::Layout, Severity::Low, status,
        format!("{pages} page{}", if pages == 1 { "" } else { "s" }),
        "Some ATSs cut long resumes off, and recruiters skim. Aim for one or two pages.",
    ))
}

pub fn contact_info(doc: &Document) -> Check {
    let email = text::EMAIL.find(&doc.text);
    let phone = text::has_phone(&doc.text);
    let late = email.is_some_and(|m| m.start() as f64 / doc.text.len().max(1) as f64 > 0.25);
    let (status, title) = match (email.is_some(), phone) {
        (false, _) => (Status::Fail, "No email address found"),
        (true, false) => (Status::Warn, "Email found, no phone number"),
        (true, true) if late => (Status::Warn, "Contact details are far down the page"),
        _ => (Status::Pass, "Email and phone found"),
    };
    check(
        "contact_info", Category::Content, Severity::High, status, title,
        "Put your email and phone in plain text at the top, not in an image, icon or page header.",
    )
}

pub fn section_headings(found: &[&str]) -> Check {
    let missing: Vec<&str> = text::REQUIRED_SECTIONS.into_iter().filter(|k| !found.contains(k)).collect();
    let status = if missing.is_empty() { Status::Pass } else if missing.contains(&"experience") || missing.len() > 1 { Status::Fail } else { Status::Warn };
    check(
        "section_headings", Category::Content, Severity::Medium, status,
        if missing.is_empty() { "Experience, Education and Skills headings found".to_string() } else { format!("No {} heading found", missing.join(" or ")) },
        "ATSs file your text by heading. Use plain names like \"Experience\", \"Education\" and \"Skills\".",
    )
}

pub fn dates(doc: &Document) -> Check {
    let years = text::YEAR.find_iter(&doc.text).count();
    let status = if years > 0 { Status::Pass } else { Status::Warn };
    check(
        "dates", Category::Content, Severity::Low, status,
        if years > 0 { "Dates found".to_string() } else { "No dates found".to_string() },
        "Without dates an ATS can't work out your years of experience. Add a start and end date to each role.",
    )
}

/// Every check that applies to this document, most important first.
pub fn run(doc: &Document, found_sections: &[&str]) -> Vec<Check> {
    let mut out = vec![text_layer(doc), garbled_glyphs(doc)];
    out.extend([text_coverage(doc), multi_column(doc), tables(doc), text_boxes(doc), header_footer(doc), images(doc), page_count(doc)].into_iter().flatten());
    out.extend([contact_info(doc), section_headings(found_sections), dates(doc)]);
    out
}

/// 100 minus severity penalties; a warning costs half a failure.
pub fn score(checks: &[Check], categories: &[Category]) -> u32 {
    let mut total: f64 = 100.0;
    for c in checks.iter().filter(|c| categories.contains(&c.category) && c.status != Status::Pass) {
        let p = match c.severity {
            Severity::High => 30.0,
            Severity::Medium => 12.0,
            Severity::Low => 4.0,
        };
        total -= if c.status == Status::Fail { p } else { p / 2.0 };
    }
    total.clamp(0.0, 100.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x0: f64, y0: f64, x1: f64, y1: f64) -> TextBox {
        TextBox { x0, y0, x1, y1, text: "x".into() }
    }

    #[test]
    fn single_column_with_right_dates_is_not_two_columns() {
        let mut boxes = vec![];
        for i in 0..10 {
            let y = 50.0 + i as f64 * 60.0;
            boxes.push(b(50.0, y, 300.0, y + 12.0)); // title
            boxes.push(b(480.0, y, 560.0, y + 12.0)); // date, flush right
            boxes.push(b(50.0, y + 20.0, 560.0, y + 32.0)); // full-width bullet
        }
        let page = PageLayout { width: 612.0, height: 792.0, boxes, ..Default::default() };
        assert!(!has_columns(&page));
    }

    #[test]
    fn sidebar_layout_is_two_columns() {
        let mut boxes = vec![];
        for i in 0..20 {
            let y = 40.0 + i as f64 * 30.0;
            boxes.push(b(30.0, y, 170.0 - (i % 3) as f64 * 20.0, y + 12.0)); // ragged sidebar
            boxes.push(b(220.0, y, 580.0, y + 12.0)); // main column
        }
        let page = PageLayout { width: 612.0, height: 792.0, boxes, ..Default::default() };
        assert!(has_columns(&page));
    }

    #[test]
    fn scoring_halves_warnings() {
        let doc = Document::from_text("tiny");
        let checks = run(&doc, &[]);
        let ids: Vec<&str> = checks.iter().map(|c| c.id).collect();
        assert_eq!(ids, ["text_layer", "garbled_glyphs", "contact_info", "section_headings", "dates"]);
        // text_layer fail (30) only, among extraction+layout.
        assert_eq!(score(&checks, &[Category::Extraction, Category::Layout]), 70);
        // contact fail 30, sections fail 12, dates warn 2.
        assert_eq!(score(&checks, &[Category::Content]), 56);
    }
}
