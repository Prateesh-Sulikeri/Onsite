# Onsite website

Static site, deployed to GitHub Pages by `.github/workflows/site.yml`. Plain HTML/CSS, no framework, no build deps.

- `src/`: pages (`index`, `guides` (setup per AI + voice engine), `how-it-works` for developers, `changelog`) as templates (`{{placeholders}}`), `style.css` (the app's night-violet tokens), `site.js` (picks the
  Download file for the visitor's system).
- `build.mjs`: fetches the published releases from the GitHub API, adds "what changed" from `git log` between
  release tags, fills the templates, and copies the logo, backdrop and screenshots (`ui/assets/`,
  `docs/engineering/img/`) into `dist/`.

Local: `node site/build.mjs`, then open `site/dist/index.html`. It uses `gh auth token` if `GITHUB_TOKEN` is unset.

The site rebuilds on a push to `main` touching `site/**`, after every stable and experimental release, and daily.
It loads nothing from third parties and has no analytics; keep it that way (see PRIVACY.md).

`how-it-works.html` summarises `docs/engineering/*.md` for outside developers. When a subsystem changes in a way
that makes it wrong (flags, engines, limits, numbers), update it in the same commit, like the engineering docs.

`guides.html` screenshots live in `src/img/guides/`. They are the real UI with a fake backend: retake them with
`cd docs/engineering/tools && npm run guide-shots` (one shot: `npm run guide-shots -- ai-groq-ready`). When a Settings
screen or setup step changes, update the guide and retake its shots in the same commit. The Docker section is
written from VoiceStudio's Docker docs and is marked untested until someone runs it end to end.
