# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by
[SignPath Foundation](https://signpath.org/).

## What is signed
Only the Windows installer (`Onsite_<version>_x64-setup.exe`) built by this repo's GitHub Actions release workflow
(`.github/workflows/release.yml`) from a version tag. Nothing built on a personal machine is signed.

## Team roles and their members
- **Committers and reviewers:** [Prateesh Sulikeri](https://github.com/Prateesh-Sulikeri)
- **Approvers:** [Prateesh Sulikeri](https://github.com/Prateesh-Sulikeri)

Changes from outside contributors are reviewed by a committer before they are merged. Every signing request is
approved by an approver.

## Privacy policy
This program will not transfer any information to other networked systems unless specifically requested by the
user or the person installing or operating it. Details: [PRIVACY.md](PRIVACY.md).
