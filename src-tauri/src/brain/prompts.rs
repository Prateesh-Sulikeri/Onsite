//! `{{placeholder}}` rendering for the templates in `prompts/`.

use std::collections::HashMap;
use std::path::Path;

pub type Vars<'a> = HashMap<&'a str, String>;

/// Loads a template and drops its leading `<!-- ... -->` doc comment.
pub fn load(prompts_dir: &Path, name: &str) -> crate::error::Result<String> {
    let text = std::fs::read_to_string(prompts_dir.join(name))?;
    Ok(strip_header(&text).to_string())
}

fn strip_header(text: &str) -> &str {
    let t = text.trim_start();
    match (t.starts_with("<!--"), t.find("-->")) {
        (true, Some(end)) => t[end + 3..].trim_start(),
        _ => t,
    }
}

/// Replaces every `{{key}}`. Unknown keys render empty and are logged.
pub fn render(template: &str, vars: &Vars) -> String {
    let mut out = String::with_capacity(template.len() + 1024);
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            out.push_str(&rest[open..]);
            return out;
        };
        let key = after[..close].trim();
        match vars.get(key) {
            Some(v) => out.push_str(v),
            None => tracing::warn!(key, "prompt placeholder has no value"),
        }
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_and_ignores_unknown() {
        let vars: Vars = [("a", "1".to_string())].into();
        assert_eq!(render("x {{a}} y {{ a }} {{b}} z {{", &vars), "x 1 y 1  z {{");
    }

    #[test]
    fn values_are_not_re_expanded() {
        let vars: Vars = [("a", "{{b}}".to_string()), ("b", "no".to_string())].into();
        assert_eq!(render("{{a}}", &vars), "{{b}}");
    }

    #[test]
    fn strips_doc_comment() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts");
        let t = load(&dir, "interviewer.md").unwrap();
        assert!(t.starts_with("You are {{persona}}"));
    }
}
