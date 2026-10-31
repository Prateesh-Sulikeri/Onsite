# Onsite website

Static site, deployed to GitHub Pages by `.github/workflows/site.yml`. Plain HTML/CSS, no framework, no build deps.

- `src/`: page templates (`{{placeholders}}`), `style.css` (the app's night-violet tokens), `site.js` (picks the
  Download file for the visitor's system).
- `build.mjs`: fetches the published releases from the GitHub API, adds "what changed" from `git log` between
  release tags, fills the templates, and copies the logo, backdrop and screenshots (`ui/assets/`,
  `docs/engineering/img/`) into `dist/`.

Local: `node site/build.mjs`, then open `site/dist/index.html`. It uses `gh auth token` if `GITHUB_TOKEN` is unset.

The site rebuilds on a push to `main` touching `site/**`, after every stable and experimental release, and daily.
It loads nothing from third parties and has no analytics; keep it that way (see PRIVACY.md).
