# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by
[SignPath Foundation](https://signpath.org).

## What is signed

Only the files of a Riflesso X release, built from this repository by GitHub Actions
([`.github/workflows/release.yml`](.github/workflows/release.yml)):

- `Riflesso X.exe`, the app (also inside the installer and the portable zip);
- `riflesso.exe`, the command line;
- `Riflesso-X_<version>_x64-setup.exe`, the installer.

Every signing request is approved by hand before a release is published.

## Team roles

| Role | Members |
|---|---|
| Committers and reviewers | [kannarozzi](https://github.com/kannarozzi) |
| Approvers | [kannarozzi](https://github.com/kannarozzi) |

Changes from other contributors are reviewed by a committer before they are merged.
All team members use multi-factor authentication on GitHub and SignPath.

## Privacy policy

Riflesso X collects no personal data. Apart from the update check described below,
which can be turned off, it will not transfer any information to other networked
systems unless specifically requested by the user or the person installing or
operating it.

Riflesso X receives the screen and sound of an iPhone, iPad or Mac over the local
network when someone chooses it on their device; nothing is sent beyond the local
network. Its links to the internet are:

- the update check: once a day the app (and the command line, when started) asks
  GitHub's public API for the number of the latest release. Nothing about the user or
  their devices is sent. It can be turned off in *Settings → App → Updates* (and with
  `--no-update-check` on the command line);
- "Update now", which downloads the new installer from GitHub when the user asks;
- the "Source code on GitHub" button, which opens this project's page in the browser;
- the command-line install script, which downloads the release from GitHub when run.
