# RESUME-CHECK — `src/ats/`

Reads a resume like an ATS parser, scores six ATS profiles, matches a posting, optional AI screen. Deterministic except
two AI calls. Ported: checks + match score from Resume-Matcher (Apache-2.0, `services/ats.py`, `ats_parse/`); profiles
from ats-screener (MIT). See `THIRD_PARTY_NOTICES.md`.

## Map

| File | Contents |
|---|---|
| `extract.rs` | `extract(name, bytes) -> Document {text, kind, page_count, pages[PageLayout{boxes}], truncated, …}`; type by magic bytes; PDF via pdf-extract/lopdf + `Collector` (`OutputDev`, text boxes); DOCX via zip + quick-xml (tables, text boxes, skip `mc:Fallback`, headers/footers kept apart); `rtf_to_text` (cp1252); UTF-8/16 text; HTML strip. Refuses .doc, images, .odt/.pages, password PDFs (owner-only-locked OK) |
| `text.rs` | `normalize`, `stem(s)`, `stem_match`, `keyword_in_text`, `sections`, `section_of`, `heading_names`, `has_phone`, `EMAIL`, `date_consistency`, `experience_months` |
| `checks.rs` | `Check {id, category, severity, status, title, detail}`; `run`; `score(checks, categories)`; `has_columns`/`free_bands`/`is_right_aligned_band` |
| `score.rs` | `Facts::new`, `TermHit {exact, stem, alias, required}`, `term_hits`, `match_score` (`SubScores`: keywords, required, title_match, section_completeness, date_consistency), `Strategy`, `PROFILES`, `ats_scores`, `tips` |
| `job.rs` | `JobKeywords {company, role, required_skills, preferred_skills, keywords, aliases, experience_years, seniority}`; `BUILTIN_ALIASES`; `keywords(state, jd)` cached `ats_job:<sha256(CACHE_VERSION\nnorm jd)[..16]>`; `CACHE_VERSION = "2"`; ≤ 20 terms, ≤ 60 chars |
| `screen.rs` | `AiScreen {grade A-D, fit, advance, level below/at/above, level_note, summary, must_haves[{skill, verdict proven/claimed/missing, evidence}], strengths, red_flags, recruiter_take, model, duration_ms}`; cached `ats_screen:<sha256(version, resume, jd)>` |
| `mod.rs` | `Report {file, extractability full/partial/none, parse_score, content_score, checks, job, matched, keywords, ats, tips, sections, experience_years, preview, checked_at}`; `analyze` (pure); `run`; `run_screen`; `read_file`; `LAST_KEY = "ats_last"` |

## Pipeline

`extract` → `Facts` + `checks::run` → `parse_score` (Extraction+Layout; 0 if no text layer) / `content_score` (Content)
→ `term_hits(job)` → `match_score` → `ats_scores` → `tips`. AI: posting-only keywords (interviewer model, ~4–7 s);
screen on **extracted** text + posting (report model, ~15–22 s), auto-run after the rule check when a posting exists.

Checks by category — Extraction: `text_layer garbled_glyphs text_coverage`; Layout: `multi_column tables text_boxes
header_footer images page_count`; Content: `contact_info section_headings dates`.
Extractability: none (no text layer) / partial (garbled, low coverage, truncated) / full.

## Profiles

`parse' = 100 − (100 − parse_score) × strictness`; score = weighted(parse', credit(all), credit(required), title, sections,
dates) − profile issues. Weights order: parse, keywords, required, title, sections, dates.

| id | strategy (credit) | strictness | weights | pass |
|---|---|---|---|---|
| workday | Exact (exact only) | 1.00 | .25 .30 .15 .10 .12 .08 | 70 |
| taleo | Exact | 0.95 | .20 .30 .20 .10 .12 .08 | 65 |
| icims | Semantic (stem/alias 0.9) | 0.60 | .15 .35 .15 .15 .12 .08 | 60 |
| greenhouse | Semantic | 0.60 | .20 .30 .15 .15 .12 .08 | 60 |
| lever | Stem | 0.75 | .20 .35 .15 .10 .12 .08 | 60 |
| successfactors | Taxonomy (alias) | 0.85 | .20 .30 .20 .10 .12 .08 | 65 |

Profile issues e.g. Workday: >2 pages, contact only in header, two columns; Taleo: tables.

## Rules

- AI never sees the resume when reading the posting. Screen grades: A 85+, B 70–84, C 50–69, D < 50; quote resume only;
  **never infer gender/age/nationality from the name** (`ats_screen.md`).
- Bump `CACHE_VERSION` when `ats_job.md` or `JobKeywords` changes.
- Loop start runs the check; no/unreadable resume blocks the loop; result shown on Get ready.
- UI copy must not claim the profiles are the vendors' real behaviour (simulations).

## Tests / tools

Fixtures `src-tauri/tests/fixtures/ats/` (clean/messy docx, one/two-column + scan pdf, rtf, html sources);
`tests/ats_files.rs`; `analyze` is pure → unit tests. CLI: `cargo run --example ats_check -- resume.pdf [job.json] [--json]`
(app running locks the exe → `--target-dir target/ats-test`).

## Requirements

ATS-R1 formats by bytes · R2 refusals with plain message · R3 extracted preview · R4 checks list · R5 six profiles + issues ·
R6 keyword hits exact/stem/alias + required · R7 AI screen shape · R8 no trait guessing · R9 caching · R10 loop gate · R11 CLI.
