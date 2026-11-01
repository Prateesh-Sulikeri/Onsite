//! "Send feedback": the user's answers go to the owner's Google Form, only when they press Send.
//! Google Forms takes a plain urlencoded POST to `formResponse` with one `entry.<id>` per question, no sign-in.
//! The ids and choice labels below are read from the live form (`FB_PUBLIC_LOAD_DATA_` in the viewform page);
//! if the form is edited, re-read them or submissions fail with 400.

use crate::error::{AppError, Result};
use serde::Deserialize;
use std::time::Duration;

pub const FORM_VIEW: &str = "https://docs.google.com/forms/d/e/1FAIpQLScMqpdKkD61KqPK4fkNTqm_uqS2hahUYbVda2iHHzX0EzVaRw/viewform";
const FORM_POST: &str = "https://docs.google.com/forms/d/e/1FAIpQLScMqpdKkD61KqPK4fkNTqm_uqS2hahUYbVda2iHHzX0EzVaRw/formResponse";

const E_OVERALL: &str = "entry.2027240377";
const E_REALISTIC: &str = "entry.1617850619";
const E_SETUP: &str = "entry.1691130292";
const E_OS: &str = "entry.353701081";
const E_MODE: &str = "entry.69846117";
const E_TRIED: &str = "entry.1250363713";
const E_USEFUL: &str = "entry.1729278150";
const E_MISSING: &str = "entry.1527212274";
const E_EXPERIENCE: &str = "entry.127933366";
const E_WRONG: &str = "entry.1216287094";
const E_WOULD_USE: &str = "entry.1845437555";
const E_EMAIL: &str = "entry.1938927202";

pub const OS: &[&str] = &["Windows", "macOS", "Linux"];
pub const MODES: &[&str] = &["Spoken \u{2014} built-in voice", "Spoken \u{2014} VoiceStudio", "Typed", "I didn't finish a round"];
pub const FEATURES: &[&str] = &[
    "Mock interview rounds",
    "Coding editor / Run code",
    "Whiteboard",
    "Round report & feedback",
    "Resume check",
    "Company research (dossiers)",
];
pub const WOULD_USE: &[&str] = &["Definitely", "Maybe", "Depends on the company", "Probably not"];

const MAX_TEXT: usize = 4000;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Feedback {
    pub overall: u8,
    pub realistic: u8,
    pub setup: u8,
    pub os: String,
    pub mode: String,
    pub tried: Vec<String>,
    pub useful: String,
    pub missing: String,
    pub experience: String,
    pub wrong: String,
    pub would_use: String,
    pub email: String,
}

fn scale(v: u8, what: &str) -> Result<String> {
    if (1..=5).contains(&v) {
        Ok(v.to_string())
    } else {
        Err(AppError::Invalid(format!("Rate {what} from 1 to 5.")))
    }
}

fn pick(v: &str, from: &[&str], what: &str) -> Result<String> {
    from.iter().find(|c| **c == v).map(|c| c.to_string()).ok_or_else(|| AppError::Invalid(format!("Pick {what}.")))
}

fn text(v: &str) -> String {
    let t = v.trim();
    t.chars().take(MAX_TEXT).collect()
}

/// A loose check: Google rejects anything that isn't an email address, so catch the obvious cases here.
fn email_ok(e: &str) -> bool {
    let Some((user, host)) = e.split_once('@') else { return false };
    !user.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.') && !e.contains(char::is_whitespace)
}

/// Checks the answers and returns the form fields in submission order.
pub fn fields(f: &Feedback) -> Result<Vec<(&'static str, String)>> {
    let mut out = vec![
        (E_OVERALL, scale(f.overall, "your overall experience")?),
        (E_REALISTIC, scale(f.realistic, "how realistic it felt")?),
        (E_SETUP, scale(f.setup, "how easy setup was")?),
        (E_OS, pick(&f.os, OS, "your computer")?),
        (E_MODE, pick(&f.mode, MODES, "how you ran your rounds")?),
    ];
    for t in &f.tried {
        out.push((E_TRIED, pick(t, FEATURES, "only listed features")?));
    }
    if !f.useful.is_empty() {
        out.push((E_USEFUL, pick(&f.useful, FEATURES, "the most useful feature")?));
    }
    let experience = text(&f.experience);
    if experience.is_empty() {
        return Err(AppError::Invalid("Tell us a little about your experience.".into()));
    }
    out.push((E_MISSING, text(&f.missing)));
    out.push((E_EXPERIENCE, experience));
    out.push((E_WRONG, text(&f.wrong)));
    out.push((E_WOULD_USE, pick(&f.would_use, WOULD_USE, "whether you'd use Onsite before an interview")?));
    let email = f.email.trim();
    if !email.is_empty() && !email_ok(email) {
        return Err(AppError::Invalid("That email doesn't look right. Fix it or leave it empty.".into()));
    }
    out.push((E_EMAIL, email.to_string()));
    Ok(out)
}

pub async fn submit(f: &Feedback) -> Result<()> {
    let mut body = fields(f)?;
    body.push(("fvv", "1".into()));
    body.push(("pageHistory", "0".into()));
    let resp = reqwest::Client::builder()
        .user_agent(concat!("Onsite/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
        .post(FORM_POST)
        .form(&body)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!("feedback: {e}");
            AppError::Invalid("Couldn't reach Google Forms. Check your internet connection, or open the form in your browser.".into())
        })?;
    let status = resp.status();
    if status.is_success() {
        tracing::info!("feedback sent");
        Ok(())
    } else {
        tracing::warn!("feedback: form returned {status}");
        Err(AppError::Invalid(format!("The form didn't accept it ({status}). Open the form in your browser instead.")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Feedback {
        Feedback {
            overall: 4,
            realistic: 5,
            setup: 3,
            os: "Windows".into(),
            mode: MODES[0].into(),
            tried: vec![FEATURES[0].into(), FEATURES[4].into()],
            useful: FEATURES[0].into(),
            experience: "  Felt like a real call.  ".into(),
            would_use: "Definitely".into(),
            ..Default::default()
        }
    }

    #[test]
    fn builds_fields() {
        let f = fields(&good()).unwrap();
        assert_eq!(f[0], (E_OVERALL, "4".into()));
        assert_eq!(f.iter().filter(|(k, _)| *k == E_TRIED).count(), 2);
        assert!(f.contains(&(E_EXPERIENCE, "Felt like a real call.".into())));
        assert!(f.contains(&(E_MODE, "Spoken \u{2014} built-in voice".into())));
    }

    #[test]
    fn rejects_bad_answers() {
        assert!(fields(&Feedback { overall: 0, ..good() }).is_err());
        assert!(fields(&Feedback { os: "BeOS".into(), ..good() }).is_err());
        assert!(fields(&Feedback { experience: "   ".into(), ..good() }).is_err());
        assert!(fields(&Feedback { tried: vec!["Hacking".into()], ..good() }).is_err());
        assert!(fields(&Feedback { email: "not an email".into(), ..good() }).is_err());
        assert!(fields(&Feedback { email: "a@b.co".into(), ..good() }).is_ok());
        assert!(fields(&Feedback { useful: String::new(), ..good() }).is_ok());
    }
}
