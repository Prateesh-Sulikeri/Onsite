# PRD-02 · Interview loops, rounds and feedback

> **Status:** Shipped (v0.1.2) · **Updated:** 2026-10-30 · **Design:** [DD-02 Interview engine](DD-02-interview-engine.md), [DD-01 AI brain](DD-01-ai-brain.md)

## 1. Summary

A **loop** is a set of rounds run in order, like a real onsite. Each **round** is a live conversation with one
interviewer persona, a clock, and a workspace. After each round the candidate gets a **report**. After the loop
they get a **verdict** from a simulated hiring committee.

## 2. User stories

| Id | As a… | I want to… | So that… |
|---|---|---|---|
| LOOP-S1 | candidate | pick the rounds, company style, level and interviewer style | practice matches the job I'm going for |
| LOOP-S2 | candidate | be interviewed by a consistent persona with a real voice and habits | it feels like a person, not a chat bot |
| LOOP-S3 | candidate | have the interviewer follow up, push back and keep time | I train for real pressure |
| LOOP-S4 | candidate | get a report with scores and evidence after each round | I know exactly what to fix |
| LOOP-S5 | candidate | get a hire / no-hire verdict after the loop | I know if I'm ready |
| LOOP-S6 | candidate | see my skill scores over time and what to practise next | I can plan the next week |
| LOOP-S7 | candidate | not hear the same opening question every time | repeated practice stays useful |

## 3. The setup screen

The *Interviews* screen walks through five numbered steps. The right side shows the loop as a timeline.

```text
┌──────────────────────────────────────────────┬───────────────────────────┐
│ ( General practice )  ( Company-specific )    │  Your interview loop      │
│                                              │  ① Resume screen   30 min │
│ 1 Set your context                           │  ② DSA coding      45 min │
│   Company style ▾   Interviewer style ▾      │  ③ System design   60 min │
│   Interviewers ▾    Level ▾   Language ▾     │  ④ Behavioral      45 min │
│ 2 Choose your rounds   [tiles × 9]           │                           │
│ 3 Add your resume      [drop] [JD drop]      │  What to expect           │
│ 4 Additional details   "Grill me on Kafka"   │  Tip: start with 2–3      │
│ 5 Interview settings   Round length ▾        │                           │
│                         [ ▶ Start interview → ]                          │
└──────────────────────────────────────────────┴───────────────────────────┘
```

### Choices

| Setting | Options | Source |
|---|---|---|
| Mode | General practice · Company-specific (needs a dossier, see [PRD-04](PRD-04-company-research.md)) | UI |
| Company style | 14: *Infer from the JD*, Google, Amazon, Meta, Microsoft, Apple, Netflix, Uber, Stripe, Atlassian, Flipkart, Indian fintech, Bank tech, Funded startup | `catalog.json` → `COMPANIES` |
| Interviewer style (temperament) | Supportive (hints when stuck) · Realistic (neutral, like a real panel) · Bar-raiser (terse, skeptical, rarely hints) | `TEMPER` |
| Interviewers | Mixed (a different named interviewer each round) · Indian Tech Panel · International Tech Panel | `panels.json` |
| Level | New grad · SDE-1 · SDE-2 · Senior · Staff | `LEVELS`, with a written bar per level in `LEVEL_BAR` |
| Coding language | Java, Python, C++, JavaScript, TypeScript, Go | UI |
| Round length | Real length per round, or 10 / 20 / 30 minutes each | UI |
| Resume | Required. PDF, Word, RTF, text. Runs the resume check first ([PRD-05](PRD-05-resume-check.md)). | UI |
| Job description | Optional, same formats | UI |
| Additional details | Free text to the panel, e.g. "Weak at DP" | UI |

## 4. Round types

| Key | Round | Real length | Default tab | Rubric dimensions |
|---|---|---|---|---|
| `resume` | Resume screen | 30 min | Notes | Clarity of story · Depth behind claims · Role fit · Motivation · Communication |
| `project` | Project deep dive | 45 | Notes (+ Whiteboard) | Ownership · Technical depth · Trade-off reasoning · Impact and metrics · Communication |
| `fundamentals` | Technical fundamentals | 45 | Notes | Accuracy · Depth · Applied reasoning · Communication |
| `coding` | DSA coding | 45 | Editor | Problem solving · Code quality · Testing and edge cases · Complexity analysis · Communication |
| `comprehension` | Code comprehension and review | 45 | From interviewer | Reading accuracy · Bug finding · Prioritisation and judgment · Fix quality · Testing mindset · Communication |
| `lld` | Low-level design | 60 | Notes (+ Whiteboard) | Requirements · Object modelling · Code quality and SOLID · Extensibility · Concurrency |
| `sysdesign` | System design | 60 | Notes (+ Whiteboard) | Requirements and scoping · Architecture · Deep dive · Scale and reliability · Trade-offs · Driving the discussion |
| `behavioral` | Behavioral | 45 | Notes | Specificity · Ownership · Self-awareness · Collaboration and conflict · Values alignment |
| `hm` | Hiring manager | 45 | Notes | Scope and impact · Judgment · Team fit · Motivation · Quality of their questions |

Each round in `catalog.json` also has a **guide** (how to run it, sent to the interviewer) and **openers**
(starting points that rotate, see LOOP-R7). The coding round has no openers: the question bank picks the problem.

The room shows a **round plan** for the candidate. Example for coding: Introduction 7% · Problem and clarifying
questions 13% · Approach and complexity 17% · Code it 38% · Dry run, edge cases, follow-up 18% · Your questions 7%.

## 5. Interviewer personas

Two panels ship in `prompts/panels.json`, five interviewers each:

| Indian Tech Panel | International Tech Panel |
|---|---|
| Arjun Mehta, Engineering Manager | Ethan Brooks, Principal Engineer / Architect |
| Neha Kapoor, Senior Technical Recruiter / HR | Olivia Chen, Technical Recruiter / HR |
| Karthik Reddy, Senior Technical Interviewer | Marcus Reed, Senior Technical Interviewer |
| Vikram Desai, Staff Engineer / Domain Specialist | Priya Shah, Staff Engineer / Domain Expert |
| Sameer Khanna, Director of Engineering | Daniel Foster, Engineering Manager |

Each persona has traits, a communication style, a strictness (1–5), a backstory, the round types they own, a
speaking-speed factor and a **voice design** (an instruction to VoiceStudio). The voice is built once per persona and
stays the same every round.

> **Rule:** when the chosen temperament and the persona's strictness disagree, the temperament wins for *how often
> to hint and how warm to sound*. The persona keeps its identity, habits and standards.

## 6. Requirements

| Id | Requirement | Status |
|---|---|---|
| LOOP-R1 | The interviewer speaks in short turns (1–4 sentences), one question at a time, with no markdown in speech. | Shipped (prompt rule) |
| LOOP-R2 | The interviewer paces the round from `[Elapsed: X of Y min]` on every candidate message, starts wrapping up at ~85%, and leaves time for the candidate's questions. | Shipped |
| LOOP-R3 | The interviewer closes the round itself and marks it with `[[END]]`. The app then shows "Get my feedback". | Shipped |
| LOOP-R4 | No feedback, scores or praise during the round ("never say great answer"). | Shipped (prompt rule) |
| LOOP-R5 | Code the interviewer shows goes to the workspace ("From interviewer" tab), not into speech. | Shipped |
| LOOP-R6 | The candidate's editor and whiteboard reach the interviewer when changed, inside a `[Workspace]` block. | Shipped |
| LOOP-R7 | Opening variety: each round gets an opener not used recently, and the openings of the last 4 rounds of that type as "do not reuse". | Shipped |
| LOOP-R8 | Only one round runs at a time. Starting a new one closes any round left open. | Shipped |
| LOOP-R9 | A round with no answers is deleted, so history only lists rounds that happened. | Shipped |
| LOOP-R10 | Terminate = stop now, no feedback, nothing saved (typed confirmation "terminate"). | Shipped |
| LOOP-R11 | A report needs at least 2 answers and 40 words. Otherwise: "Not enough to review yet." | Shipped |
| LOOP-R12 | If the interviewer process dies mid-round, the next answer reconnects to the same conversation. | Shipped (CLI: `--resume`; API: history in memory) |
| LOOP-R13 | Interviews are in English only. Accented speech mis-heard as Spanish is read as English. | Shipped |

## 7. The round report

Generated by the *report model* with no tools, from `prompts/report.md`. Sample (shortened):

```json
{
  "overall": 68,
  "decision": "Lean hire",
  "level_signal": "Near SDE-2 bar",
  "headline": "Solid hash-map approach and clean code; weak on edge cases and complexity proof.",
  "dimensions": [
    {"name": "Problem solving", "score": 3, "evidence": "Moved from O of n squared to a single pass after one nudge."},
    {"name": "Testing and edge cases", "score": 2, "evidence": "Did not test an empty array until asked."}
  ],
  "went_well": ["Asked about duplicates before coding"],
  "held_back": ["Stated complexity without explaining why"],
  "fix_first": "Before coding, list three edge cases out loud and say how the code handles each.",
  "moments": [{"question": "What if the array is huge?", "said": "Use a bigger hash map.", "better": "Discuss memory, streaming, and sorting plus two pointers."}],
  "practice_next": ["Two-pointer problems", "Explaining amortised complexity"]
}
```

- **Decision scale:** Strong hire · Hire · Lean hire · Lean no hire · No hire.
- **Dimension scores:** 1–4, or null when the round gave no signal.
- **Measured facts** are given to the model so it cannot invent them: number of answers, average words, longest
  answer, planned vs actual minutes.
- **Speech-to-text leniency:** garbled answers are judged on intent; the report may note a possible transcription problem.

## 8. The loop verdict

After a loop, `loop_feedback` writes any missing round reports. Once **two or more rounds** have a report, it asks
for a committee decision from `prompts/verdict.md`:

```json
{
  "decision": "Hire at lower level",
  "rationale": "Strong coding and fundamentals. System design needed heavy guidance…",
  "strongest_signal": "Payments idempotency deep dive",
  "biggest_risk": "Could not drive the design discussion",
  "plan": ["Practise 3 design problems end to end with estimates", "…"]
}
```

Decisions: Hire · No hire · Hire at lower level · Borderline — more data needed.

## 9. Dashboard

Everything is computed in the UI from `history` (each row carries the round key, minutes and report JSON).
No numbers are faked; an empty history shows empty states.

| Widget | What it shows | Rule |
|---|---|---|
| Continue | Review or get feedback on the last round | Last round in history |
| Weekly goal | Rounds per week, per round type | `weekly_goal` setting |
| Skill scores | Average 1–4 per rubric dimension | Grouped by dimension name across reports |
| Trend | Overall score over time | Sparkline per reviewed round |
| Practise next | Weakest skills | Needs a skill low in **2+ rounds** |
| Highlights | Strongest skills | Needs a skill high in **2+ rounds** |
| Recent interviews | Last few rounds | History |

## 10. Out of scope / open

- **Comprehension round redesign** (Planned): today the interviewer writes the code to review. Options under study:
  curated small repos, bug datasets (Defects4J, BugsInPy), mutation tools. See `docs/comprehension-round-research.md`.
- **Difficulty picker** for coding (today: New grad → Easy, Senior/Staff → Hard, else Medium).
- **Pausing a round** and coming back later.
