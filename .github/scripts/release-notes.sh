#!/usr/bin/env bash
# Prints the shared release-notes tables (download, first run, security warnings) for a tag.
# Usage: release-notes.sh <tag>      (needs GITHUB_REPOSITORY)
# File names are the ones Tauri gives each bundle; the links work once the builds upload.
set -euo pipefail
tag="$1"
cat <<'EOF2' | sed -e "s|{V}|${tag#v}|g" -e "s|{REPO}|$GITHUB_REPOSITORY|g" \
  -e "s|{DL}|https://github.com/$GITHUB_REPOSITORY/releases/download/$tag|g"
## Download

| Your system | Download |
| --- | --- |
| **Windows 10/11** (x64) | [Installer (.exe)]({DL}/Onsite_{V}_x64-setup.exe) |
| **macOS 11+** (Apple Silicon and Intel) | [Disk image (.dmg)]({DL}/Onsite_{V}_universal.dmg) |
| **Linux** (x86-64, any distro) | [AppImage]({DL}/Onsite_{V}_amd64.AppImage) |
| **Linux** (Debian, Ubuntu) | [.deb package]({DL}/Onsite_{V}_amd64.deb) |

Already installed? Settings → Updates → Check for updates.

## First run

- Pick an AI in Settings: Claude Code (Pro/Max), or paste a Claude, OpenAI, Gemini or Groq API key (Gemini and Groq have free tiers).
- For spoken rounds, download the voice models in Settings → Voice (about 580 MB, once). [VoiceStudio](https://github.com/debpalash/VoiceStudio/releases) also works. Typed rounds need neither.

## Security warnings

Onsite is free and not code-signed yet, so your system warns about it the first time.

| System | What you see | What to do |
| --- | --- | --- |
| Windows | "Windows protected your PC" (SmartScreen) | Click **More info**, then **Run anyway** |
| macOS | "Onsite is damaged" or "can't be opened" | Run `xattr -dr com.apple.quarantine /Applications/Onsite.app` once, then open it. Allow the microphone when asked. |
| Linux | Nothing; AppImage won't start | `chmod +x Onsite_*.AppImage` |

Check a download against `SHA256SUMS.txt`: `sha256sum -c SHA256SUMS.txt --ignore-missing` (Windows: `Get-FileHash <file>`).

[Code signing policy](https://github.com/{REPO}/blob/main/CODE_SIGNING.md) · [Privacy policy](https://github.com/{REPO}/blob/main/PRIVACY.md)
EOF2
