# RELEASE — CI, installers, updater, signing

Code: `.github/workflows/release.yml`, `src/updates.rs`, `src-tauri/tauri.conf.json`, `src-tauri/build.rs`, `src-tauri/Info.plist`.

## Channels

- **Stable**: branch `main`, tags `vX.Y.Z`. **Experimental**: branch `m1`, tags `vX.Y.Z-exp.N` (N numeric), GitHub pre-releases.
- `draft` job enforces the branch rule (checkout `fetch-depth: 0`, `git merge-base --is-ancestor $GITHUB_SHA origin/main|origin/m1`)
  and the tag shape; a tag with a dash gets `--prerelease`. `workflow_dispatch` skips it (job is tag-only).
- Prerelease versions in tauri.conf.json/Cargo.toml (`0.1.3-exp.1`) are valid semver. Bundler 2.10 (read in source, not yet
  run in CI): NSIS uses only major.minor.patch(.build) for `VIProductVersion`, pre part ignored, file name keeps it;
  macOS Info.plist gets it verbatim in CFBundleShortVersionString/CFBundleVersion (fine outside the App Store); deb
  `Version: 0.1.3-exp.1` is valid (hyphen = revision); AppImage name keeps it. Only the **msi** target rejects non-numeric
  pre-release, and we don't build msi. If a target breaks, run the dry run and fix there.
- Ordering (`version_key`): `0.1.3-exp.2 > 0.1.3-exp.1`, `0.1.3 > 0.1.3-exp.9`, `0.1.4-exp.1 > 0.1.3`.
- Cut experimental: on `m1` set version `X.Y.Z-exp.N` in both files → commit → push m1 → `git tag vX.Y.Z-exp.N && git push origin vX.Y.Z-exp.N`.
- Cut stable: merge to `main`, set version `X.Y.Z` in both files → commit on main → push → `git tag vX.Y.Z && git push origin vX.Y.Z`.
- Dry run: Actions → Release → Run workflow, any branch (builds only; no branch/tag checks, no release).

## Release workflow (tag `v*`)

1. `draft` (ubuntu): branch rule above; tag must equal version in `tauri.conf.json` **and** `Cargo.toml` → create draft release + notes
   (download table per OS, security-warnings table).
2. `build` matrix (needs draft): windows-latest `--bundles nsis` · macos-latest `--target universal-apple-darwin --bundles dmg`
   (ad-hoc signed `signingIdentity "-"`, not notarized, min macOS 11) · ubuntu-22.04 `--bundles appimage,deb` (deb depends
   `libasound2`). Each: install deps → `cargo test` → `tauri-apps/tauri-action`.
3. `publish` (needs all builds): add `SHA256SUMS.txt`, un-draft.
`workflow_dispatch` = dry run (artifacts uploaded, nothing published). Bundled resources: `prompts/ dossiers/ sidecar/`.


## Updater (`updates.rs`) — pull only (user presses Check)

- `ONSITE_RELEASES` = `api.github.com/repos/Prateesh-Sulikeri/Onsite/releases?per_page=30`; drafts skipped;
  `version_key` (pre-release < release, then by trailing number). `parse_releases` → `UpdateInfo` (current, newest, all, asset per package).
- Channel: settings `update_channel` (`stable` default; an `-exp` build defaults to `experimental`; commands
  `get_update_channel`/`set_update_channel`). Stable `latest` = newest non-prerelease above current; Experimental = newest of all.
  Settings > Updates has the picker; pre-releases show an "Experimental" pill.
- `Package::current()`: Windows `Nsis` (`*-setup.exe`) · macOS `Dmg` (`*universal*.dmg` or arch) · `$APPIMAGE` set →
  `AppImage` (`*amd64|arm64*.AppImage`) · else `Deb`.
- `install(tag)`: refused while a round or research job runs → download (github.com URLs only) to `%TEMP%/onsite-update`
  with `update_progress` events → Nsis: run installer + quit · Dmg: open · AppImage: replace `$APPIMAGE` + restart · Deb:
  copy to Downloads. Any version installable (downgrade allowed; UI warns older may not read newer data).
- VoiceStudio: installed version from `/health.version` else `resources/pyproject.toml` in its install dir; latest from
  `debpalash/VoiceStudio` releases. Never updated by Onsite.

## Code signing

Windows: **unsigned**; SignPath Foundation application pending (2026-10-30). `CODE_SIGNING.md` = policy page SignPath
requires; `SECURITY.md` ("not code-signed yet", verify with SHA256SUMS) is the true state. On approval: add signing step
to `release.yml`, update `SECURITY.md` + release-notes warnings table. macOS: ad-hoc only (notarization needs paid account).

## Platform quirks

- macOS/Linux GUI launches get a bare PATH → `lib.rs::path_from_login_shell` (`$SHELL -ilc`, 3 s cap).
- Windows children: `CREATE_NO_WINDOW` (no console flashes).
- `build.rs` watches `icons/` (else new logos never reach the exe without a clean build).
- Running app locks `onsite.exe` → test builds with `--target-dir target/<other>`.
- macOS mic permission string: `Info.plist`.

## Status / gaps

Windows used daily. macOS + Linux builds never run by a person (2026-10-29) — treat reports as first contact.
