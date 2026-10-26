<!-- Research job prompt. Run with web search + web fetch ONLY.
     Placeholders: {{company}} {{role}} {{level_name}} {{jd}} {{round_types}} {{today}} -->

You are researching how {{company}} currently interviews software engineers for this role, so a mock-interview app can run a realistic loop.

Role: {{role}} ({{level_name}})
Today's date: {{today}}
Job description (may be empty):
{{jd}}

GOAL
Find how this company's loop for this role actually runs, based on recent candidate reports. Prefer 2024 onwards; weight the last 12 months most. Good sources: LeetCode Discuss, interviewexperiences.in, GeeksforGeeks interview experiences, Glassdoor interview reviews, Blind/Reddit threads, Naukri Code360, the company's own engineering blog and careers pages. Ignore SEO "guide" sites unless they confirm something seen elsewhere.

METHOD
1. Search broadly (company + role + "interview experience"), then narrow per round type (machine coding, LLD, system design, DSA, hiring manager, behavioral).
2. Open full write-ups, not just snippets. Read at least 8 distinct first-hand reports if they exist.
3. Find the company's stated values from its own site or job posts.
4. Note disagreements between reports (e.g. some loops have a DSA round, some don't) and say so in the summary.

PROGRESS (the app shows these to the user — print them as plain lines in your text output as you go)
PROGRESS stage=<1-6> pct=<0-100> note=<one short sentence>
Stages: 1 reading request, 2 finding the current process, 3 collecting candidate experiences, 4 extracting questions per round, 5 values and interviewer style, 6 building the dossier.

OUTPUT
Your FINAL message must be only one JSON object, no prose, matching this shape:
{
  "company": string, "role": string, "level": string,
  "summary": "3-5 sentences: typical round order, how strict, where people fail, any recent changes",
  "style": "how interviewers behave across the loop",
  "values": "company values, with source",
  "pitfalls": [string, ...],
  "process": [
    {"type": one of {{round_types}},
     "name": "round name as the company calls it",
     "minutes": integer,
     "what": "what happens in this round",
     "style": "how interviewers run it, what they push on",
     "questions": ["specific reported question, paraphrased in your words", ...]}
  ],
  "sources": [{"title": string, "url": string}]
}
Rules: only include questions that a source actually reported (paraphrase, never copy long passages). 4–10 questions per round. Order rounds as they usually happen. If evidence is thin, say so in "summary" instead of inventing.
