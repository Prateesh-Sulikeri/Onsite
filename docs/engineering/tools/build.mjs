// Builds human-readable Markdown docs into styled PDFs in docs/engineering/pdf/.
// The .md files next to this folder are agent docs, so the human sources come from --src: a folder of
// Markdown with its img/ beside it, e.g. the tagged snapshot:
//   git worktree add ../onsite-docs-human docs-human-2026-10-30
//   npm run build -- --src ../onsite-docs-human/docs/engineering [FILE.md …]
// Mermaid blocks are rendered to SVG in headless Chromium, so diagrams look the same as on GitHub.
// Chromium: $CHROME, else Playwright's cache (~/.cache/ms-playwright), else an installed Chrome/Edge.
import { readFileSync, writeFileSync, readdirSync, mkdirSync, existsSync } from "node:fs";
import { join, dirname, basename, resolve } from "node:path";
import { homedir } from "node:os";
import { fileURLToPath, pathToFileURL } from "node:url";
import { marked } from "marked";
import { chromium } from "playwright-core";

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const srcAt = args.indexOf("--src");
if (srcAt < 0 || !args[srcAt + 1]) {
  console.error("Pass --src <folder of human-readable .md>. The .md files in docs/engineering are agent docs.");
  process.exit(2);
}
const docs = resolve(args[srcAt + 1]);
args.splice(srcAt, 2);
process.argv = [...process.argv.slice(0, 2), ...args];
const out = join(here, "..", "pdf");
const mermaidJs = join(here, "node_modules/mermaid/dist/mermaid.min.js");

function findChrome() {
  if (process.env.CHROME && existsSync(process.env.CHROME)) return process.env.CHROME;
  const cache = join(homedir(), ".cache/ms-playwright");
  if (existsSync(cache)) {
    for (const d of readdirSync(cache).filter(d => d.startsWith("chromium-")).sort().reverse()) {
      for (const sub of ["chrome-linux/chrome", "chrome-linux64/chrome", "chrome-win/chrome.exe", "chrome-mac/Chromium.app/Contents/MacOS/Chromium"]) {
        const p = join(cache, d, sub);
        if (existsSync(p)) return p;
      }
    }
  }
  for (const p of [
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser",
  ]) if (existsSync(p)) return p;
  throw new Error("No Chromium found. Set CHROME=/path/to/chrome.");
}

// ```mermaid blocks become <pre class="mermaid">; links to sibling .md files point at the PDFs.
marked.use({
  gfm: true,
  walkTokens(tok) {
    if (tok.type === "link" && /^[\w-]+\.md(#.*)?$/.test(tok.href)) tok.href = tok.href.replace(/\.md/, ".pdf");
  },
  renderer: {
    // Returning false falls back to marked's own renderer.
    code(tok) {
      return tok.lang === "mermaid" ? `<pre class="mermaid">${tok.text.replace(/</g, "&lt;")}</pre>` : false;
    },
  },
});

const CSS = `
@page { size: A4; margin: 18mm 16mm 20mm; }
:root { --ink:#1d1a2e; --ink-2:#4a4560; --muted:#6f6987; --line:#e3dff0; --accent:#6d3fd8; --accent-soft:#f1ecfd; --warm:#c2410c; --code:#f6f4fb; }
* { box-sizing: border-box; }
body { font: 10.5pt/1.55 "Segoe UI", "Inter", system-ui, sans-serif; color: var(--ink); margin: 0; }
h1 { font-size: 24pt; line-height: 1.15; margin: 0 0 6pt; color: #140f2b; letter-spacing: -0.01em; }
h1 + blockquote { border: 0; background: var(--accent-soft); color: var(--ink-2); padding: 7pt 10pt; border-radius: 6pt; margin: 6pt 0 16pt; font-size: 9.5pt; }
h2 { font-size: 15pt; margin: 22pt 0 6pt; padding-bottom: 3pt; border-bottom: 1.5pt solid var(--accent); color: #140f2b; break-after: avoid; }
h3 { font-size: 12pt; margin: 14pt 0 4pt; color: var(--accent); break-after: avoid; }
h4 { font-size: 10.5pt; margin: 10pt 0 3pt; break-after: avoid; }
p, li { orphans: 3; widows: 3; }
a { color: var(--accent); text-decoration: none; }
code { font: 9pt "Cascadia Code", Consolas, monospace; background: var(--code); padding: 0.5pt 3pt; border-radius: 3pt; }
pre { background: var(--code); border: 1px solid var(--line); border-radius: 6pt; padding: 8pt 10pt; overflow: hidden; white-space: pre-wrap; word-break: break-word; break-inside: avoid; }
pre code { background: none; padding: 0; font-size: 8.5pt; line-height: 1.45; }
pre.mermaid { background: #fff; border: 1px solid var(--line); text-align: center; padding: 10pt; }
pre.mermaid svg { max-width: 100%; height: auto; }
table { border-collapse: collapse; width: 100%; margin: 8pt 0; font-size: 9.2pt; break-inside: auto; }
th, td { border: 1px solid var(--line); padding: 4pt 6pt; text-align: left; vertical-align: top; }
th { background: var(--accent-soft); color: #140f2b; }
tr { break-inside: avoid; }
blockquote { margin: 8pt 0; padding: 6pt 10pt; border-left: 3pt solid var(--accent); background: #faf9fd; color: var(--ink-2); }
blockquote p { margin: 2pt 0; }
hr { border: 0; border-top: 1px solid var(--line); margin: 16pt 0; }
img { max-width: 100%; }
.cover { font-size: 8.5pt; color: var(--muted); text-transform: uppercase; letter-spacing: .12em; margin-bottom: 4pt; }
`;

const files = (process.argv.slice(2).length ? process.argv.slice(2) : readdirSync(docs).filter(f => f.endsWith(".md"))).sort();
mkdirSync(out, { recursive: true });
const browser = await chromium.launch({ executablePath: findChrome() });
const page = await browser.newPage();
for (const f of files) {
  const md = readFileSync(join(docs, f), "utf8");
  const title = (md.match(/^# (.+)$/m) || [, basename(f, ".md")])[1];
  const html = `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title><style>${CSS}</style></head>
<body><div class="cover">Onsite · Engineering docs</div>${marked.parse(md)}</body></html>`;
  // Next to the .md files, so relative image paths (img/…) resolve.
  const tmp = join(docs, `.${basename(f, ".md")}.html`);
  writeFileSync(tmp, html);
  await page.goto(pathToFileURL(tmp).href);
  if (html.includes('class="mermaid"')) {
    await page.addScriptTag({ path: mermaidJs });
    // One diagram at a time, so a syntax error names the diagram instead of failing the whole page.
    const errors = await page.evaluate(async () => {
      window.mermaid.initialize({
        startOnLoad: false, theme: "base", securityLevel: "strict", fontFamily: "Segoe UI, system-ui, sans-serif",
        themeVariables: { primaryColor: "#efe9fd", primaryBorderColor: "#7c4ddb", primaryTextColor: "#1d1a2e",
          lineColor: "#6f6987", secondaryColor: "#fdf0e8", tertiaryColor: "#f6f4fb", fontSize: "13px" },
      });
      const errs = [];
      for (const [i, el] of [...document.querySelectorAll(".mermaid")].entries()) {
        try { await window.mermaid.run({ nodes: [el] }); }
        catch (e) { errs.push(`diagram ${i + 1} (${el.textContent.trim().split("\n")[0]}): ${e.message || e}`); }
      }
      return errs;
    });
    if (errors.length) {
      console.error(`${f}:\n  ${errors.join("\n  ")}`);
      process.exitCode = 1;
      continue;
    }
  }
  const pdf = join(out, `${basename(f, ".md")}.pdf`);
  await page.pdf({
    path: pdf, format: "A4", printBackground: true, displayHeaderFooter: true,
    headerTemplate: "<span></span>",
    footerTemplate: `<div style="font:8px Segoe UI,sans-serif;color:#8a84a3;width:100%;padding:0 16mm;display:flex;justify-content:space-between">
      <span>${title.replace(/</g, "&lt;")}</span><span><span class="pageNumber"></span> / <span class="totalPages"></span></span></div>`,
    margin: { top: "18mm", bottom: "20mm", left: "16mm", right: "16mm" },
  });
  if (process.env.KEEP_HTML) await page.screenshot({ path: join(process.env.KEEP_HTML, `${basename(f, ".md")}.png`), fullPage: true });
  (await import("node:fs")).rmSync(tmp);
  console.log(`pdf/${basename(pdf)}`);
}
await browser.close();
