# Onsite website

Static site, deployed to GitHub Pages by `.github/workflows/site.yml`. Plain HTML/CSS, no framework, no build deps.

- `src/`: pages (`index`, `guides` (setup per AI + voice engine), `how-it-works` for developers, `changelog`) as templates (`{{placeholders}}`), `style.css` (the app's night-violet tokens), `site.js` (picks the
  Download file for the visitor's system).
- `build.mjs`: fetches the published releases from the GitHub API, adds "what changed" from `git log` between
  release tags, fills the templates, and copies the logo, backdrop and screenshots (`ui/assets/`,
  `docs/engineering/img/`) into `dist/`.

Local: `node site/build.mjs`, then open `site/dist/index.html`. It uses `gh auth token` if `GITHUB_TOKEN` is unset.
Changelog: write lines for any release in `release-notes.json` (`{"v0.1.5": ["Voice: ...", "..."]}`); they replace the
commit subjects for that tag. Releases with nothing to list are left out, except the latest stable.
Videos: paste the YouTube links into `VIDEOS` at the top of `build.mjs` (`promo`, `setup`, `ai`, `voice`). A slot with
no link is left out of the page. The home hero and Guides link out to YouTube; nothing is embedded. Preview one
without editing: `SITE_VIDEO_DEMO=https://youtu.be/... node site/build.mjs`.
`404.html` links with absolute paths (`/Onsite/` by default, as on GitHub Pages); set `SITE_BASE=/` to try it on a
local server at the root (in Git Bash also `MSYS2_ENV_CONV_EXCL=SITE_BASE`, or it becomes a Windows path).

The site rebuilds on a push to `main` touching `site/**`, after every stable and experimental release, and daily.
Visit counts: GoatCounter, set by `GOATCOUNTER` in `site/build.mjs` (its site code). Empty = off: no script, the
CSP stays strict, and the "no trackers" copy stays. Set it, and the build adds the script, widens the CSP for the two
GoatCounter hosts and switches the copy (`<!--analytics-->` / `<!--noanalytics-->` blocks). Then update PRIVACY.md.
Nothing else on the site loads from third parties.

`how-it-works.html` summarises `docs/engineering/*.md` for outside developers. When a subsystem changes in a way
that makes it wrong (flags, engines, limits, numbers), update it in the same commit, like the engineering docs.

`guides.html` screenshots live in `src/img/guides/`. They are the real UI with a fake backend: retake them with
`cd docs/engineering/tools && npm run guide-shots` (one shot: `npm run guide-shots -- ai-groq-ready`). When a Settings
screen or setup step changes, update the guide and retake its shots in the same commit. The Docker section is
written from VoiceStudio's Docker docs and is marked untested until someone runs it end to end.
