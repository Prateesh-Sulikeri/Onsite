# RELEASE — CI, installers, updater, signing

Code: `.github/workflows/release.yml` (tags), `build.yml` (shared build matrix + publish), `experimental.yml` (push to m1), `.github/scripts/release-notes.sh`, `src/updates.rs`, `src-tauri/tauri.conf.json`, `src-tauri/build.rs`, `src-tauri/Info.plist`.

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

## Release workflow (tag `v*`; steps 2–3 are build.yml)

1. `draft` (ubuntu): branch rule above; tag must equal version in `tauri.conf.json` **and** `Cargo.toml` → create draft release + notes
   (download table per OS, security-warnings table).
2. `build` matrix (needs draft): windows-latest `--bundles nsis` · macos-latest `--target universal-apple-darwin --bundles dmg`
   (ad-hoc signed `signingIdentity "-"`, not notarized, min macOS 11) · ubuntu-22.04 `--bundles appimage,deb` (deb depends
   `libasound2`). Each: install deps → `cargo test` → `tauri-apps/tauri-action`.
3. `publish` (needs all builds): add `SHA256SUMS.txt`, un-draft.
`workflow_dispatch` = dry run (artifacts uploaded, nothing published). Bundled resources: `prompts/ dossiers/ sidecar/`.
NSIS hooks: `src-tauri/windows/hooks.nsh` (uninstall clean-up, `/PURGE`; see STORAGE.md). Only the Windows CI build compiles them.


## Automatic experimental releases (`experimental.yml`)

- Trigger: push to `m1` (paths-ignore `docs/**`, `**/*.md`, `graphify-out/**`) or manual run; skipped if the head commit message has `[skip exp]`.
  `concurrency: experimental`, cancel-in-progress.
- Version: core of `src-tauri/Cargo.toml` (pre-release part stripped); while tag/release `v<core>` exists, patch+1; then `-exp.<github.run_number>`
  (monotonic, so `version_key` orders them). Written to tauri.conf.json + Cargo.toml in CI only (build.yml `version` input, perl); never committed. Cargo.lock is
  rewritten by the build (no `--locked`).
- Flow: `prepare` (delete stale `-exp.` drafts, make draft pre-release `v<version>`, `--target $GITHUB_SHA`, `--latest=false`, notes = sha + commit subjects since the
  previous exp release (else last 10) + shared tables) → `build` (build.yml: 3 builds, SHA256SUMS, un-draft; the tag appears on publish) → `prune`
  (keep newest 3 published `vX.Y.Z-exp.N` pre-releases, `gh release delete --cleanup-tag`; stable never matches). `cleanup` deletes this run's draft on failure/cancel.
- Tags/releases made with `GITHUB_TOKEN` do not trigger workflows, so release.yml does not fire on them (it would fail the version check anyway).
- The manual path still works: commit `X.Y.Z-exp.N` in both files on m1, push the `vX.Y.Z-exp.N` tag → release.yml. Prefer the automatic flow; do not mix
  both for the same core version (the auto run number could be lower than a hand-picked N and sort below it).

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
- macOS mic: `Info.plist` (prompt text) **and** `Entitlements.plist` (`com.apple.security.device.audio-input`). Tauri signs with the
  hardened runtime by default (`--options runtime`, even ad-hoc); without the entitlement the mic is silently blocked. sherpa-onnx is
  linked statically, so no library-validation entitlement is needed.
  build.yml checks the signed app for the entitlement after the macOS build and fails (release stays a draft) without it.
- Linux: `lib.rs::run` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` unless the user set it. Without it WebKitGTK draws a blank white
  window on NVIDIA drivers, VMs and WSLg (seen 2026-10-31 on the exp.4 AppImage under WSLg).
- The AppImage runs without libfuse2 (static runtime); the .deb's `libasound2` resolves to `libasound2t64` on Ubuntu 24.04.

## Status / gaps

Windows used daily. Linux AppImage launched under WSLg 2026-10-31 (UI, DB, dossier import OK; no mic or round tested).
macOS never run by a person — treat reports as first contact.
Known for later: a hand-run newer setup.exe over an install with these hooks offers "uninstall first"; that runs the old
uninstaller without `/UPDATE`, so the voice models are deleted and download again (user data stays).
