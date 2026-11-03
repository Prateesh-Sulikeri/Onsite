// Builds the Onsite website into site/dist: copies site/src, the logo, backdrop and screenshots, and bakes the
// GitHub releases (downloads, sizes, what changed) into the HTML. No dependencies; Node 20+.
//
//   node site/build.mjs
//
// Env: GITHUB_REPOSITORY (default Prateesh-Sulikeri/Onsite), GITHUB_TOKEN (optional; else `gh auth token`, else
// unauthenticated), SITE_RELEASES_FILE (a saved /releases JSON, for offline work), SITE_BASE (path the 404 page
// links from; default /<repo name>/, use / for a local server).
// "What changed" = commit subjects between a release's tag and the one before it, so run it in a clone with tags
// (CI: checkout with fetch-depth 0). Release bodies are not used: stable ones hold only the download tables.
import { readFile, writeFile, mkdir, cp, rm, readdir } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "..");
const src = path.join(here, "src");
const out = path.join(here, "dist");
const repo = process.env.GITHUB_REPOSITORY || "Prateesh-Sulikeri/Onsite";
const gh = `https://github.com/${repo}`;
// Where GitHub Pages serves the site. The 404 page needs absolute paths (it is served at any depth);
// SITE_BASE=/ for a local server at the root.
const [owner, name] = repo.split("/");
const siteUrl = `https://${owner.toLowerCase()}.github.io/${name}/`;
const base = process.env.SITE_BASE || `/${name}/`;

// YouTube links for the video slots. Empty = not up yet: the slot is left out. SITE_VIDEO_<NAME> overrides one
// (handy for a local preview). Templates mark slots with <!--video:name-->...<!--/video--> (shown when the video has
// a link; {{video_name}} inside is the link) and <!--novideo:name-->...<!--/novideo--> (shown when it hasn't).
const VIDEOS = { promo: "https://youtu.be/v9lG6l2rH18", setup: "https://www.youtube.com/watch?v=S1_G-g1jhgs", tour: "", ai: "", voice: "" };
for (const k of Object.keys(VIDEOS)) VIDEOS[k] = process.env[`SITE_VIDEO_${k.toUpperCase()}`] || VIDEOS[k];

const TAG = /^v(\d+)\.(\d+)\.(\d+)(?:-exp\.(\d+))?$/;
// Commit subjects that say nothing to someone downloading the app.
const NOISE = [/^Release\b/i, /^Version \d/i, /^Merge\b/i, /^Docs?:/i, /^Engineering docs\b/i, /^CI:/i,
  /^Experimental release\b/i, /^Website\b/i, /^Site:/i, /^Runner test\b/i, /^Remove dead code\b/i, /^Voice spike\b/i,
  /^Keep LF line endings\b/i, /^Plans?:/i, /^Tests?:/i, /^Scaffold\b/i];

const esc = s => String(s).replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

function versionKey(tag) {
  const m = TAG.exec(tag);
  if (!m) return null;
  return [+m[1], +m[2], +m[3], m[4] === undefined ? Infinity : +m[4]];
}
function cmp(a, b) {
  const ka = versionKey(a), kb = versionKey(b);
  for (let i = 0; i < 4; i++) if (ka[i] !== kb[i]) return ka[i] < kb[i] ? -1 : 1;
  return 0;
}
const isExp = tag => tag.includes("-exp.");

function git(args) {
  try {
    return execFileSync("git", args, { cwd: repoRoot, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  } catch { return null; }
}
const isAncestor = (a, b) => git(["merge-base", "--is-ancestor", a, b]) !== null;

// Previous release point: for stable, the newest older stable tag that is an ancestor; for experimental, the newest
// older tag of either kind that is an ancestor (main is merged back into m1, so a stable tag can be the base).
function previousTag(tag, allTags) {
  const older = allTags.filter(t => cmp(t, tag) < 0 && (isExp(tag) || !isExp(t))).sort(cmp).reverse();
  return older.find(t => isAncestor(t, tag)) || null;
}

// Hand-written notes (site/release-notes.json, {"v0.1.4": ["Area: what changed", ...]}) replace the commit subjects.
let NOTES = {};
async function loadNotes() {
  const all = JSON.parse(await readFile(path.join(here, "release-notes.json"), "utf8"));
  for (const [tag, lines] of Object.entries(all)) {
    if (tag.startsWith("_")) continue; // _about and other comments
    if (!TAG.test(tag)) throw new Error(`release-notes.json: ${tag} is not a release tag`);
    if (!Array.isArray(lines) || !lines.every(l => typeof l === "string" && l.trim()))
      throw new Error(`release-notes.json: ${tag} must be a list of lines`);
    NOTES[tag] = lines.map(l => l.trim());
  }
}

function changes(tag, allTags) {
  if (NOTES[tag]) return NOTES[tag];
  if (!allTags.includes(tag)) return null; // tag not in this clone
  const prev = previousTag(tag, allTags);
  const log = git(["log", "--no-merges", "--format=%s", "-n", prev ? "40" : "10", prev ? `${prev}..${tag}` : tag]);
  if (log === null) return null;
  return log.split("\n").map(s => s.replace(/\s*\[skip exp\]\s*/gi, " ").trim())
    .filter(s => s && !NOISE.some(re => re.test(s)));
}

function changeList(items, max = Infinity) {
  if (items === null) return `<p class="muted">Details are on the release page.</p>`;
  if (!items.length) return `<p class="muted">No changes worth listing. See the release page.</p>`;
  const shown = items.slice(0, max).map(s => {
    const m = /^([A-Z][\w+./ -]{0,22}):\s+(.+)$/.exec(s);
    return m ? `<li><span class="area">${esc(m[1])}</span>${esc(m[2])}</li>` : `<li>${esc(s)}</li>`;
  });
  const more = items.length > max ? `<li class="muted">and ${items.length - max} more</li>` : "";
  return `<ul class="changes">${shown.join("")}${more}</ul>`;
}

const FILES = {
  win: { re: /-setup\.exe$/, label: "Windows", kind: "Installer (.exe)", note: "Windows 10 and 11, x64" },
  mac: { re: /universal\.dmg$/, label: "macOS", kind: "Disk image (.dmg)", note: "macOS 11+, Apple Silicon and Intel" },
  appimage: { re: /\.AppImage$/, label: "Linux", kind: "AppImage", note: "x86-64, glibc 2.35+ (Ubuntu 22.04+)" },
  deb: { re: /_amd64\.deb$/, label: "Debian / Ubuntu", kind: ".deb package", note: "x86-64, Ubuntu 22.04+" },
};

function assets(rel) {
  const res = {};
  for (const [k, f] of Object.entries(FILES)) {
    const a = rel.assets.find(a => f.re.test(a.name) && String(a.browser_download_url).startsWith("https://github.com/"));
    res[k] = a ? { url: a.browser_download_url, name: a.name, mb: (a.size / 1048576).toFixed(0) } : null;
  }
  const sums = rel.assets.find(a => a.name === "SHA256SUMS.txt");
  res.sums = sums ? sums.browser_download_url : rel.html_url;
  return res;
}

const fmtDate = d => new Date(d).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });

async function token() {
  if (process.env.GITHUB_TOKEN) return process.env.GITHUB_TOKEN;
  try { return execFileSync("gh", ["auth", "token"], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim(); }
  catch { return ""; }
}

async function fetchReleases() {
  if (process.env.SITE_RELEASES_FILE) return JSON.parse(await readFile(process.env.SITE_RELEASES_FILE, "utf8"));
  const headers = { accept: "application/vnd.github+json", "user-agent": "onsite-site-build" };
  const t = await token();
  if (t) headers.authorization = `Bearer ${t}`;
  const r = await fetch(`https://api.github.com/repos/${repo}/releases?per_page=100`, { headers });
  if (!r.ok) throw new Error(`GitHub releases: HTTP ${r.status} ${await r.text()}`);
  return r.json();
}

function downloadCards(a, rel) {
  return Object.entries(FILES).map(([k, f]) => {
    const x = a[k];
    const href = x ? x.url : rel.html_url;
    const meta = x ? `${esc(f.kind)} · ${x.mb} MB` : "See the release page";
    return `<a class="dl" href="${esc(href)}" data-os="${k}" data-sums="${esc(a.sums)}">
  <span class="dl-os">${esc(f.label)}</span>
  <span class="dl-kind">${meta}</span>
  <span class="dl-note">${esc(f.note)}</span>
</a>`;
  }).join("\n");
}

function expCard(rel, allTags) {
  const tag = rel.tag_name;
  return `<article class="card news exp">
  <header><span class="badge warm">Experimental</span><h3>${esc(tag)}</h3><time datetime="${esc(rel.published_at)}">${fmtDate(rel.published_at)}</time></header>
  <p class="muted">The newest features first. It may break. Switch to it in Settings → Updates → Experimental, or download it from the release page.</p>
  ${changeList(changes(tag, allTags), 6)}
  <a class="link" href="${esc(rel.html_url)}">Experimental downloads →</a>
</article>`;
}

function changelogEntry(rel, allTags, latestStable) {
  const tag = rel.tag_name;
  const exp = isExp(tag);
  const a = assets(rel);
  const files = Object.entries(FILES).filter(([k]) => a[k])
    .map(([k, f]) => `<a href="${esc(a[k].url)}" data-sums="${esc(a.sums)}">${esc(f.label === "Debian / Ubuntu" ? ".deb" : f.label)}</a>`).join("");
  const badges = (exp ? `<span class="badge warm">Experimental</span>` : `<span class="badge">Stable</span>`)
    + (tag === latestStable ? `<span class="badge ok">Latest</span>` : "");
  return `<article class="card entry" id="${esc(tag)}">
  <header><h2><a href="#${esc(tag)}">${esc(tag)}</a></h2>${badges}<time datetime="${esc(rel.published_at)}">${fmtDate(rel.published_at)}</time></header>
  ${changeList(changes(tag, allTags))}
  <footer>${files ? `<span class="files">${files}</span>` : ""}<a class="link" href="${esc(rel.html_url)}">Release on GitHub →</a></footer>
</article>`;
}

// Copies each table's column headings onto its cells (data-label), so phones can show rows as labelled cards.
function labelCells(html) {
  return html.replace(/<table>[\s\S]*?<\/table>/g, table => {
    const heads = [...table.matchAll(/<th>([\s\S]*?)<\/th>/g)].map(m => m[1].replace(/<[^>]+>/g, "").trim());
    if (!heads.length) return table;
    return table.replace(/<tr>([\s\S]*?)<\/tr>/g, (row, cells) => {
      let i = 0;
      return `<tr>${cells.replace(/<td>/g, () => `<td data-label="${heads[i++] || ""}">`)}</tr>`;
    });
  });
}

function videoSlots(html) {
  const url = k => {
    if (!(k in VIDEOS)) throw new Error(`Unknown video slot: ${k}`);
    const u = VIDEOS[k];
    if (u && !/^https:\/\/(www\.youtube\.com|youtu\.be)\//.test(u)) throw new Error(`Video ${k} is not a YouTube link: ${u}`);
    return u;
  };
  return html
    .replace(/<!--video:(\w+)-->([\s\S]*?)<!--\/video-->/g, (m, k, inner) => url(k) ? inner.replaceAll(`{{video_${k}}}`, esc(url(k))) : "")
    .replace(/<!--novideo:(\w+)-->([\s\S]*?)<!--\/novideo-->/g, (m, k, inner) => url(k) ? "" : inner);
}

function fill(html, vars) {
  return html.replace(/\{\{(\w+)\}\}/g, (m, k) => {
    if (!(k in vars)) throw new Error(`No value for ${m}`);
    return vars[k];
  });
}

async function main() {
  await loadNotes();
  const all = (await fetchReleases()).filter(r => !r.draft && TAG.test(r.tag_name)).sort((a, b) => cmp(b.tag_name, a.tag_name));
  const stable = all.find(r => !isExp(r.tag_name));
  if (!stable) throw new Error("No published stable release found");
  const exp = all.find(r => isExp(r.tag_name) && cmp(r.tag_name, stable.tag_name) > 0);
  const tags = (git(["tag", "--list", "v*"]) || "").split("\n").filter(t => TAG.test(t));
  if (!tags.length) console.warn("No v* tags in this clone; 'what changed' falls back to the release pages.");

  const a = assets(stable);
  const link = k => esc(a[k] ? a[k].url : stable.html_url);
  const vars = {
    version: esc(stable.tag_name.slice(1)),
    tag: esc(stable.tag_name),
    date: fmtDate(stable.published_at),
    datetime: esc(stable.published_at),
    release_url: esc(stable.html_url),
    sums_url: esc(a.sums),
    dl_win: link("win"), dl_mac: link("mac"), dl_appimage: link("appimage"), dl_deb: link("deb"),
    download_cards: downloadCards(a, stable),
    stable_changes: changeList(changes(stable.tag_name, tags), 8),
    exp_card: exp ? expCard(exp, tags) : "",
    news_cols: exp ? "two" : "one",
    // Only releases with something to say, plus the latest stable (it carries the downloads).
    changelog: all.filter(r => r === stable || (changes(r.tag_name, tags) || []).length)
      .map(r => changelogEntry(r, tags, stable.tag_name)).join("\n"),
    gh: esc(gh),
    site_url: esc(siteUrl),
    base: esc(base),
    year: String(new Date().getUTCFullYear()),
  };
  vars.download_dialog = fill(await readFile(path.join(here, "partials/download-dialog.html"), "utf8"), vars);

  // Cache busting: pages ask for style.css?v=<hash>, so browsers never keep an old copy after a change.
  const ver = {};
  for (const f of ["style.css", "site.js"])
    ver[f] = createHash("sha256").update(await readFile(path.join(src, f))).digest("hex").slice(0, 10);
  const bust = html => html.replace(/(href|src)="(?:\{\{base\}\})?(style\.css|site\.js)"/g, (m, attr, f) => m.slice(0, -1) + `?v=${ver[f]}"`);

  await rm(out, { recursive: true, force: true });
  await mkdir(path.join(out, "assets"), { recursive: true });
  for (const name of await readdir(src)) {
    const from = path.join(src, name);
    if (name.endsWith(".html")) await writeFile(path.join(out, name), labelCells(fill(bust(videoSlots(await readFile(from, "utf8"))), vars)));
    else await cp(from, path.join(out, name), { recursive: true });
  }
  await cp(path.join(repoRoot, "ui/assets/logo.png"), path.join(out, "assets/logo.png"));
  await cp(path.join(repoRoot, "ui/assets/twilight.webp"), path.join(out, "assets/twilight.webp"));
  for (const shot of ["room", "dashboard", "dossiers", "resume", "feedback"])
    await cp(path.join(repoRoot, `docs/engineering/img/${shot}.jpg`), path.join(out, `assets/${shot}.jpg`));
  await writeFile(path.join(out, ".nojekyll"), "");

  console.log(`Built site/dist: stable ${stable.tag_name}${exp ? `, experimental ${exp.tag_name}` : ""}, ${all.length} releases.`);
}

main().catch(e => { console.error(e.message || e); process.exit(1); });
