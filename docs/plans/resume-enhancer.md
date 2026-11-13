# Resume Enhancement — Formatting Safety & Preservation

## 1. Objective

The Resume Enhancement feature must improve resume content without damaging the candidate's original document or its visual design.

Candidates may upload highly customised resumes containing:
- Multiple columns and sidebars
- Custom fonts, colours and typography
- Tables and text boxes
- Icons, graphics and visual elements
- Precisely aligned dates, headings and sections
- Complex spacing, margins and page layouts

A resume can be successfully parsed while still being impossible to reconstruct faithfully from the extracted content.

**Core product principle: Enhance the content without gambling with the original formatting.**

The original uploaded file must never be overwritten or silently replaced.

## 2. Important Technical Limitation

The existing ATS extraction pipeline can recover resume text and some layout information.

Relevant existing functionality includes:

- `src/ats/extract.rs` — extracts document text and page layout boxes.
- `src/ats/text.rs` — identifies sections, dates, contact information and experience.
- `src/ats/checks.rs` — detects extraction, layout and content issues.
- `src/ats/score.rs` — calculates keyword matches and compatibility scores.
- `src/ats/screen.rs` — provides AI-based recruiter assessment.

However, extracted text and layout boxes do not constitute a complete editable representation of the original document.

The existing extraction pipeline does not establish full-fidelity preservation of:
- Fonts and font sizes
- Colours and visual styles
- Exact positioning and alignment
- Margins and spacing
- Shapes, icons and graphics
- Complex tables and text boxes
- Original page composition

Therefore, reconstructing a fancy resume from extracted text may produce a visually different or broken document.

**Successful extraction must never be treated as proof of successful reconstruction.**

## 3. Three Resume Enhancement Modes

### Mode 1: Preserve Original — Default

The AI analyses the original resume and recommends content improvements without modifying the uploaded document.

It can suggest:
- Improved professional summaries
- Stronger experience bullet points
- Clearer descriptions of accomplishments
- Better presentation of existing skills
- Missing sections or information
- Corrections based on ATS feedback
- Job-description alignment

The candidate can selectively copy or apply suggested changes to their original document.

**Expected behaviour:** The original file remains completely untouched.

### Mode 2: Generate an ATS-Friendly Version

Generate a separate resume using a controlled, clean document template.

The candidate receives:
- Their original resume
- A newly generated ATS-friendly resume
- A comparison of content changes
- An explanation of improvements

The generated document should use predictable layouts, preferably:
- Single-column structure
- Standard section headings
- Conventional typography
- Consistent spacing
- Simple bullet points
- Minimal decorative elements

The original resume must remain available.

**Expected behaviour:** A new document is generated rather than attempting to reconstruct the original design.

### Mode 3: Preserve Original Design — Advanced

Offer an optional best-effort attempt to edit the original DOCX while retaining its existing formatting.

This mode may attempt to preserve:
- Existing styles
- Fonts and colours
- Paragraph formatting
- Tables
- Columns
- Headers and footers
- Text boxes and other layout elements

This is inherently more difficult and must not be presented as guaranteed.

Before enabling this mode, assess document complexity and explain the formatting risk.

**Expected behaviour:** The candidate sees a preview and explicitly approves the result before export.

## 4. Formatting Safety Pipeline

The enhancement workflow should follow these stages.

### Stage 1: Preserve the Original

- Store the original uploaded file unchanged.
- Never overwrite it.
- Keep the original available for comparison and recovery.
- Perform all enhancement operations on a separate working copy or generated output.

### Stage 2: Extract and Analyse

Reuse the existing ATS pipeline:

1. Extract document text and available layout information.
2. Identify resume sections and relevant facts.
3. Run extraction and layout checks.
4. Analyse dates, experience and contact information.
5. Match resume content against job-description requirements.
6. Collect deterministic ATS feedback.
7. Include AI recruiter-screening feedback when available.

### Stage 3: Generate Content Changes Separately

AI should generate structured content changes rather than directly manipulating the original document.

Suggested output structure:

- Professional summary
- Skills
- Experience
- Projects
- Education
- Certifications
- Suggested bullet-point rewrites
- Missing information
- Candidate confirmation requirements
- Warnings

Each proposed change should identify:

- Original text
- Suggested text
- Reason for the change
- Supporting resume evidence
- Whether candidate confirmation is required

AI must never fabricate:
- Skills
- Qualifications
- Experience
- Responsibilities
- Metrics
- Achievements
- Employment dates
- Project outcomes

If a job description requires a skill that is not evidenced in the resume, ask the candidate to confirm their actual experience. Do not silently insert it.

### Stage 4: Choose an Output Strategy

Based on document complexity and candidate preference, offer the appropriate enhancement mode.

Do not automatically reconstruct complex documents.

### Stage 5: Generate and Render

For generated documents:

1. Create the document.
2. Render it to PDF for inspection.
3. Inspect each page.
4. Compare the rendered output against the intended structure.
5. Run structural and visual validation.

DOCX should be the primary editable output format. PDF can be offered as a secondary output.

### Stage 6: Validate Formatting

Check for:

- Text overflow
- Clipped content
- Missing text
- Unexpected page breaks
- Unexpected blank pages
- Changed page count
- Broken columns
- Misaligned dates
- Broken tables
- Font substitutions
- Overlapping elements
- Inconsistent spacing
- Unexpected layout changes

A document containing all the original words can still be visually broken. Text extraction alone is therefore insufficient for validation.

### Stage 7: Approval Before Export

Show the candidate:
- Original resume
- Enhanced resume
- Page-by-page preview
- Content changes
- Formatting warnings
- Resolved ATS issues
- Remaining issues

The candidate must explicitly approve the generated document before treating it as their final resume.

If validation fails, block the export of the unsafe result and explain the problem.

## 5. Formatting-Risk Assessment

Introduce a formatting-risk assessment before attempting advanced document editing.

| Resume characteristic | Recommended behaviour |
|---|---|
| Simple single-column DOCX | Offer all enhancement modes |
| Two-column DOCX | Recommend original preservation or a new template |
| Heavy tables and text boxes | Warn before advanced editing |
| Graphics-heavy PDF | Offer content suggestions or a new template |
| Scanned/image-only PDF | Require OCR or manual review |
| Unsupported document format | Do not attempt reconstruction |
| Password-protected or corrupted document | Explain the limitation and stop safely |

The risk assessment should be based on actual extraction and layout checks, not merely the file extension.

A high-risk document should default to content-only suggestions or a separately generated template.

## 6. Formatting Integrity Gate

A formatting integrity gate should sit between document generation and export.

Its responsibilities:

1. Verify that document generation completed successfully.
2. Render the output.
3. Check that expected text is present.
4. Detect layout anomalies.
5. Verify that page composition is reasonable.
6. Flag potential formatting damage.
7. Prevent unsafe output from being presented as a finished resume.

The integrity gate should distinguish between:

- **Passed:** No significant formatting problems detected.
- **Warnings:** Possible layout differences require candidate review.
- **Failed:** Significant damage detected; block final export.

A successful integrity check does not guarantee pixel-perfect preservation. It only establishes that the generated document passed the available validation checks.

## 7. User Experience

The main ATS results page should provide an **Enhance My Resume** action.

The experience should communicate:

> Improve your resume using ATS feedback and AI-powered suggestions. Your original document will remain untouched.

After enhancement, provide:

- Original versus enhanced comparison
- Before-and-after text
- Highlighted additions, removals and rewrites
- Reasons for each suggested change
- Formatting warnings
- Candidate confirmation requests
- Preview and approval
- DOCX and PDF export options, where supported

The candidate should be able to reject individual suggestions rather than accepting every AI-generated change.

Do not promise that enhancement guarantees an ATS pass, interview or job offer.

## 8. AI Provider Unavailable

Resume parsing and deterministic ATS checks should remain useful without an AI provider.

AI-powered enhancement should be disabled until a provider is configured.

Suggested message:

> AI Resume Enhancement requires an AI provider. Configure one in Settings → AI Model to continue.

Do not silently fail or imply that AI enhancement has occurred when it has not.

## 9. Implementation Proposal

The following is proposed functionality, not existing functionality.

### Milestone 1: Safe Content Enhancement

- Reuse existing ATS extraction and feedback.
- Add `src/ats/enhance.rs`.
- Add `prompts/ats_enhance.md`.
- Reuse the existing AI one-shot infrastructure and model configuration.
- Generate structured enhancement suggestions.
- Validate AI output.
- Preserve the original uploaded file.
- Provide an editable enhanced-content preview.
- Generate a clean ATS-friendly DOCX.
- Add a formatting integrity gate.
- Require candidate approval before export.

### Milestone 2: Resume Studio

- Add multiple resume templates.
- Provide before-and-after comparison.
- Support selective section regeneration.
- Add iterative enhancement.
- Re-run ATS analysis against the enhanced resume.
- Show resolved and remaining issues.
- Add PDF export.
- Explore advanced original-DOCX formatting preservation.

## 10. Product Decision

The default Resume Enhancement experience must be content enhancement, not document replacement.

The original resume is the candidate's source of truth.

The product should offer three distinct choices:

1. Improve my content while keeping my original resume.
2. Create a new ATS-friendly resume.
3. Try to preserve and edit my existing design.

The first option should be the default.

The second should be the recommended route when a candidate has a complex or highly customised resume.

The third should remain optional and explicitly best-effort until reliable document-preservation technology is available.

**Final principle: Never sacrifice a candidate's carefully designed resume just to improve an ATS score.**
