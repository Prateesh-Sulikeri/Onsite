//! Resume check against real files: PDFs printed by Edge, .docx written as Word lays it out,
//! RTF, a scanned (image-only) PDF and a password-protected PDF. Sources in fixtures/ats/src.

use onsite_lib::ats::checks::Status;
use onsite_lib::ats::extract::{extract, Kind};
use onsite_lib::ats::job::JobKeywords;
use onsite_lib::ats::{analyze, Report};
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ats").join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn job() -> JobKeywords {
    serde_json::from_value::<JobKeywords>(serde_json::json!({
        "role": "Senior Backend Engineer",
        "required_skills": ["Java", "PostgreSQL", "Kafka", "Kubernetes"],
        "preferred_skills": ["Redis", "gRPC"],
        "keywords": ["payments", "idempotency"],
        "experience_years": 5
    }))
    .unwrap()
    .sanitize()
}

fn check(name: &str, with_job: bool) -> Report {
    let doc = extract(name, &fixture(name)).unwrap();
    let j = job();
    analyze(name, &doc, with_job.then_some(&j), (2026, 10))
}

fn status(r: &Report, id: &str) -> Option<Status> {
    r.checks.iter().find(|c| c.id == id).map(|c| c.status)
}

fn ats(r: &Report, id: &str) -> u32 {
    r.ats.iter().find(|a| a.id == id).unwrap().score
}

#[test]
fn clean_pdf_reads_fully() {
    let r = check("one.pdf", true);
    assert_eq!(r.file.kind, Kind::Pdf);
    assert_eq!(r.file.pages, Some(1));
    assert_eq!(r.extractability, "full");
    assert_eq!(r.parse_score, 100);
    assert_eq!(r.sections, ["summary", "experience", "education", "skills"]);
    assert_eq!(status(&r, "multi_column"), Some(Status::Pass));
    assert_eq!(status(&r, "contact_info"), Some(Status::Pass));
    assert_eq!(r.matched.sub_scores.title_match, Some(100.0));
    // Jun 2017 to Oct 2026, the two jobs back to back.
    assert_eq!(r.experience_years, Some(9.4));
    assert!(r.preview.contains("asha.rao@example.com"));
    assert!(r.ats.iter().all(|a| a.passes), "{:?}", r.ats.iter().map(|a| (a.id, a.score)).collect::<Vec<_>>());
}

#[test]
fn two_column_pdf_is_flagged_and_scrambles_the_title() {
    let r = check("two.pdf", true);
    assert_eq!(status(&r, "multi_column"), Some(Status::Fail));
    assert!(r.parse_score < 100);
    // The sidebar is read first, so the title lands under "Skills", not the headline.
    assert!(r.matched.sub_scores.title_match.unwrap() < 100.0);
    let clean = check("one.pdf", true);
    assert!(ats(&r, "workday") + 20 < ats(&clean, "workday"));
}

#[test]
fn scanned_pdf_reads_as_empty() {
    let r = check("scan.pdf", true);
    assert_eq!(r.extractability, "none");
    assert_eq!(r.parse_score, 0);
    assert_eq!(status(&r, "text_layer"), Some(Status::Fail));
    assert!(r.ats.iter().all(|a| a.score == 0 && !a.passes));
    assert_eq!(r.tips.len(), 1);
}

#[test]
fn password_pdf_is_refused_but_owner_only_lock_opens() {
    use pdf_extract::{EncryptionState, EncryptionVersion, Permissions};
    let lock = |user: &str| {
        let mut doc = pdf_extract::Document::load_mem(&fixture("one.pdf")).unwrap();
        // Encryption keys derive from the file ID, which Edge doesn't write.
        let id = pdf_extract::Object::string_literal(b"0123456789abcdef".to_vec());
        doc.trailer.set("ID", vec![id.clone(), id]);
        let state = EncryptionState::try_from(EncryptionVersion::V2 { document: &doc, owner_password: "owner", user_password: user, key_length: 128, permissions: Permissions::all() }).unwrap();
        doc.encrypt(&state).unwrap();
        let mut out = vec![];
        doc.save_to(&mut out).unwrap();
        out
    };
    let err = extract("locked.pdf", &lock("secret")).unwrap_err().to_string();
    assert!(err.contains("password"), "{err}");
    // Owner-only locks (no open password) are common from "protect" tools; ATSs read them.
    let doc = extract("owner.pdf", &lock("")).unwrap();
    assert!(doc.text.contains("Senior Backend Engineer"));
}

#[test]
fn clean_docx() {
    let r = check("clean.docx", true);
    assert_eq!(r.file.kind, Kind::Docx);
    assert_eq!(r.parse_score, 100);
    assert_eq!(r.sections, ["summary", "experience", "education", "skills"]);
    // Tabs between title and dates survive as whitespace, and "&amp;" decodes.
    assert!(r.preview.contains("PayFast\tJan 2021 – Present"));
    assert!(r.preview.contains("Redis caching & idempotency"));
    assert_eq!(r.experience_years, Some(9.4));
}

#[test]
fn messy_docx_shows_every_layout_problem() {
    let r = check("messy.docx", true);
    assert_eq!(status(&r, "header_footer"), Some(Status::Fail));
    assert_eq!(status(&r, "contact_info"), Some(Status::Fail));
    assert_eq!(status(&r, "tables"), Some(Status::Warn));
    assert_eq!(status(&r, "text_boxes"), Some(Status::Fail));
    assert_eq!(status(&r, "multi_column"), Some(Status::Fail));
    assert_eq!(status(&r, "images"), Some(Status::Warn));
    // The text box appears once (mc:Fallback skipped) and stays out of the body text.
    assert!(!r.preview.contains("seven years"));
    // Table cells come out as one line per row.
    assert!(r.preview.contains("Java | Kafka | PostgreSQL"));
    let workday = r.ats.iter().find(|a| a.id == "workday").unwrap();
    assert!(workday.issues.iter().any(|i| i.contains("page header")));
    assert!(!workday.passes);
}

#[test]
fn rtf_reads() {
    let r = check("clean.rtf", false);
    assert_eq!(r.file.kind, Kind::Rtf);
    assert_eq!(r.sections, ["experience", "education", "skills"]);
    assert!(r.preview.contains("PayFast  Jan 2021 – Present"));
    assert_eq!(status(&r, "contact_info"), Some(Status::Pass));
}

#[test]
fn file_type_comes_from_content() {
    // A .docx renamed to .pdf is still read as Word.
    let doc = extract("renamed.pdf", &fixture("clean.docx")).unwrap();
    assert_eq!(doc.kind, Kind::Docx);
    // A truncated PDF is refused with a plain message, not a panic.
    let pdf = fixture("one.pdf");
    let err = extract("cut.pdf", &pdf[..pdf.len() / 3]);
    if let Err(e) = err {
        assert!(e.to_string().contains("PDF"), "{e}");
    }
    // A zip that isn't a Word file.
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("content.xml", zip::write::SimpleFileOptions::default()).unwrap();
    std::io::Write::write_all(&mut zip, b"<x/>").unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    assert!(extract("cv.odt", &bytes).unwrap_err().to_string().contains("OpenDocument"));
}
