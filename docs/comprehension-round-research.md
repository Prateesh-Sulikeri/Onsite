# Code comprehension round: research notes (2026-10-30)

Status: notes only. Nothing built yet. Chosen direction (2026-10-31): `docs/plans/comprehension-workspace.md`
(multi-file projects from a separate repo, file tree, no Run, diffs sent to the AI).

## What Onsite does today

Defined in `prompts/catalog.json` → `ROUNDS.comprehension` ("Code comprehension and review", 45 min, `shared` tab).

- The interviewer pastes **one 60-120 line file** in a single code fence, built around a scenario from `openers`
  (rate limiter, TTL cache, job scheduler, flash-sale inventory, webhook retries, checkout flow, …).
- It plants 3-5 issues (correctness, concurrency, leak, performance, security, idempotency/retry, error handling,
  design smell) with no hints.
- Sequence: what does it do → trace a scenario → what problems, which matter most → how to fix and test → extend
  if time remains. Missed issues are not revealed (at most one or two at the end).
- Rubric: reading accuracy, bug finding, prioritisation, fix quality, testing mindset, communication.
- Phase bar in `ui/app.js` (`comprehension`): Introduction, Read the code, Trace a scenario, Find the problems,
  Fix and test, Your questions.
- The code is **read-only** and **cannot be run**. The whole round is talk-only.

## What big tech runs in 2026

Most of this comes from third-party prep sites, not official company pages. Details vary between sources.

### Meta: AI-enabled coding round
- ~60 min onsite round in CoderPad. Replaces one of the two standard coding rounds; the recruiter says which.
- Layout reported as: file explorer on the left, editor with several files open in the middle, AI chat + instructions on the right.
- One extended, real-world problem in **stages**: first a bug-fixing phase in the existing codebase, then a core
  implementation phase (100+ lines), where AI use is expected mainly in the second phase.
- AI use reportedly not scored directly; the candidate must understand and explain the solution. One source says
  same four competencies as a normal coding round.
- Eligibility reports conflict (E4-E5 only vs up to E7/M2; some say invite-only pilot). Model lists differ by source.

### Stripe: Bug Squash
- Stripe's own (older Atlas) description: candidate and interviewer work side by side on a **real historical bug
  in an open-source project**, in the candidate's language.
- Prep sites: 45-60 min, unfamiliar codebase, 3-5 bugs or failing unit tests. Reported as open-book.
- Graded on **method, not speed**: reproduce first, form a hypothesis, fix, add a regression test.
  Random edits with no hypothesis or check is the main way people fail.
- SQL variant reported for data engineering roles.
- No evidence of a separate "code review" round at Stripe. Its other practical round is API Integration.

### Common pattern
You work **in** the code instead of talking about it: several files, tests you can run, often an AI assistant,
debugging plus extending.

## Gaps

| Real rounds | Onsite now |
|---|---|
| Codebase with several files | One file in a chat message |
| Run code and tests, see failures | Can't run anything |
| Start from a failing test or bug report | "What problems do you see?" |
| Fix in the editor, then add a feature | Describe the fix out loud |
| AI assistant allowed (Meta) | None |
| Graded on debugging method | Graded on bug spotting |

## Ideas discussed

1. **Multi-file workspace.** The interviewer (or the bank) sends a small repo of 3-5 files plus a test file as data;
   the workspace shows a file tree and the files can be edited.
2. **Run tests.** Reuse `runner/` (Python/JS/Java/C++) to run the repo's tests; pass/fail goes into the
   `[Workspace]` block. Today the runner only knows single-function LeetCode problems, so this is the big piece.
3. **Staged script** (Bug Squash + Meta): (a) bug report + one failing test, (b) reproduce, find the cause, fix,
   add a regression test, (c) add a feature to the same code, (d) what else would you change.
   Rubric: add "Debugging method" and "Navigating unfamiliar code"; drop plain "Bug finding".
4. **Optional "AI assistant allowed" switch** for a Meta-style variant. Needs a second model session for the
   assistant; the interviewer itself must still get no tools (non-negotiable 4).
5. **Quick win, no new plumbing:** rewrite the prompt only: staged bug report → fix → extend, still one file.

## Where codebases could come from

Constraint: the runner uses plain `python`, `node`, `javac`, `g++`. No Maven/pip/npm installs on friends'
machines. A round needs a codebase you can learn in ~10 minutes (3-8 files).

### Real bugs with a failing test
| Source | Language | Contents | Fit |
|---|---|---|---|
| Defects4J (`rjust/defects4j`) | Java | ~850 real bugs, ~17 projects (Commons Lang, Joda-Time, Jackson…), fixing commit + failing test | Great bugs; projects big, need Ant/Maven. Inspiration, not shipped as-is. |
| BugsInPy (`soarsmu/BugsInPy`) | Python | ~490 real bugs, 17 projects (pandas, keras, black…) | Big projects, many dependencies. |
| SWE-bench / Verified / Lite (`SWE-bench/SWE-bench`) | Python | Real GitHub issues + failing tests (django, sympy, requests…) | Issue text is a good model for bug reports; repos too big for 45 min. |
| Multi-SWE-bench (ByteDance) | Java, TS, JS, Go, Rust, C, C++ | Same idea, more languages | Same problem. |
| QuixBugs (`jkoppel/QuixBugs`) | Python + Java | 40 small programs, one-line bug each, with tests | Dependency-free but one function each: too small. |
| HumanEvalFix (bigcode HumanEvalPack) | 6 incl. Java, Python, JS, C++ | Small buggy functions + tests | Function-sized. |

### Small apps to plant bugs in
- RealWorld (`gothinkster/realworld`): same Medium-like app in dozens of stacks. Good multi-file size, needs frameworks.
- Spring PetClinic: classic Java codebase; Spring + Maven.
- Mutation tools (PIT for Java, mutmut for Python) can plant bugs automatically in any repo with tests.

### Code review data
- CodeReviewer (Microsoft): real PR diffs with reviewer comments. Useful for a "review this PR" variant.

### Licensing
QuixBugs, SWE-bench, RealWorld: MIT. Defects4J framework: MIT, but each bug's code keeps its project's
license (mostly Apache-2.0). Copying any code means shipping its license; writing our own repos inspired by
these bugs avoids that.

## Recommendation so far

Build a curated set of ~20-30 small repos, the way `prompts/questions.csv` works for coding:
- 3-8 files, standard library only, Python / Java / JS / C++, each with a test file the runner can run.
- 1-2 planted bugs per repo, modelled on real bug types from Defects4J / BugsInPy: off-by-one at boundaries,
  wrong equality or null handling, timezone/locale, retry without idempotency, shared mutable state.
- Each repo comes with a SWE-bench-style bug report, one failing test, a hidden reference fix, and an
  "extend it" task for stage two.
- A build script checks that the tests fail before the fix and pass after it (like `cargo test -- --ignored` for the bank).
- Topics come from the current `openers` (payments/backend flavour), not from django or sympy.

Possible next step: draft the repo manifest format (files, bug report, tests, fix, extension) and build two or
three example repos to try.

## Open questions
- One staged round, or split into "Debugging" and "Code review (PR diff)" round types?
- Should the AI-assistant variant exist, and on which providers?
- How does the interviewer see the candidate's edits across several files without blowing the token budget
  (Groq free tier is 8K tokens/min)?
- Do company dossiers need a new round type key, or does `comprehension` cover both?

## Sources
- https://prachub.com/resources/meta-ai-coding-interview
- https://thita.ai/blog/interview/meta-ai-assisted-coding-interview
- https://copilotinterview.com/blog/meta-ai-enabled-coding-interview
- https://techscreen.app/articles/meta-technical-interview-process-2026
- https://nerddesignlab.substack.com/p/metas-ai-enabled-interview-everything
- https://www.coditioning.com/blog/804/stripe-swe-bug-squash-interview
- https://leonstaff.com/blogs/stripe-technical-interview-bug-squash-integration-guide/
- https://techscreen.app/articles/stripe-technical-interview-process-2026
- https://prepfully.com/interview-guides/stripe-software-engineer
