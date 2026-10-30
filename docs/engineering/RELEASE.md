# RELEASE — CI, installers, updater, signing

Code: `.github/workflows/release.yml`, `src/updates.rs`, `src-tauri/tauri.conf.json`, `src-tauri/build.rs`, `src-tauri/Info.plist`.

## Release workflow (tag `v*`)

1. `draft` (ubuntu): tag must equal version in `tauri.conf.json` **and** `Cargo.toml` → create draft release + notes
   (download table per OS, security-warnings table).
2. `build` matrix (needs draft): windows-latest `--bundles nsis` · macos-latest `--target universal-apple-darwin --bundles dmg`
   (ad-hoc signed `signingIdentity "-"`, not notarized, min macOS 11) · ubuntu-22.04 `--bundles appimage,deb` (deb depends
   `libasound2`). Each: install deps → `cargo test` → `tauri-apps/tauri-action`.
3. `publish` (needs all builds): add `SHA256SUMS.txt`, un-draft.
`workflow_dispatch` = dry run (artifacts uploaded, nothing published). Bundled resources: `prompts/ dossiers/ sidecar/`.

To release: bump version in both files → commit → `git tag vX.Y.Z && git push --tags`.

## Updater (`updates.rs`) — pull only (user presses Check)

- `ONSITE_RELEASES` = `api.github.com/repos/Prateesh-Sulikeri/Onsite/releases?per_page=30`; drafts skipped;
  `version_key` (pre-release < release). `parse_releases` → `UpdateInfo` (current, newest, all, asset per package).
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
