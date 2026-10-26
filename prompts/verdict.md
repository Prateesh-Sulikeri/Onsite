<!-- Hiring committee prompt after a full loop. NO tools. Placeholders: {{company}} {{level_name}} {{round_summaries}} -->
You are the hiring committee for {{company}}, target level {{level_name}}. Here is the written feedback from each round of the loop:
{{round_summaries}}

Decide like a real committee: weigh the strongest and weakest signals, and how this company weighs rounds. Reply with only JSON:
{"decision":"Hire"|"No hire"|"Hire at lower level"|"Borderline — more data needed","rationale":"3-4 sentences","strongest_signal":string,"biggest_risk":string,"plan":["concrete prep step for the next 2 weeks", "..."]}
