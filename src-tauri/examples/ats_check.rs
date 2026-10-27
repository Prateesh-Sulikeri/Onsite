//! Runs the resume check on a file, offline: `cargo run --example ats_check -- resume.pdf [job.json]`.
//! `job.json` is a `JobKeywords` object (what Claude would return for a posting). `--json` prints the full report.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: ats_check <resume file> [job.json]");
        std::process::exit(2);
    };
    let bytes = std::fs::read(path).expect("read resume");
    let doc = match onsite_lib::ats::extract::extract(path, &bytes) {
        Ok(d) => d,
        Err(e) => {
            println!("refused: {e}");
            return;
        }
    };
    let job = args.get(1).map(|p| {
        let raw: onsite_lib::ats::job::JobKeywords = serde_json::from_str(&std::fs::read_to_string(p).expect("read job")).expect("job json");
        raw.sanitize()
    });
    let r = onsite_lib::ats::analyze(path, &doc, job.as_ref(), onsite_lib::ats::today());
    if args.iter().any(|a| a == "--json") {
        println!("{}", serde_json::to_string_pretty(&r).expect("report json"));
        return;
    }
    println!("{} | {} page(s) | {} words | text: {}", r.file.kind_label, r.file.pages.map_or("-".into(), |p| p.to_string()), r.file.words, r.extractability);
    println!("parse {}  content {}  match {}  sections {:?}  years {:?}", r.parse_score, r.content_score, r.matched.overall, r.sections, r.experience_years);
    for c in &r.checks {
        println!("  [{:?}] {} — {}", c.status, c.id, c.title);
    }
    for a in &r.ats {
        println!("  {:<15} {:>3} {}  {}", a.name, a.score, if a.passes { "pass" } else { "BELOW" }, a.issues.join(" / "));
    }
    for k in &r.keywords {
        println!("  term {:<22} exact={} stem={} alias={:?} req={}", k.term, k.exact, k.stem, k.alias, k.required);
    }
    for t in &r.tips {
        println!("  tip: {t}");
    }
    if std::env::var_os("ATS_PREVIEW").is_some() {
        println!("---- text ----\n{}", r.preview);
    }
}
