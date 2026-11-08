# Releases and website

The public repository is [yannickdreher/NoriDrive](https://github.com/yannickdreher/NoriDrive). The default and release branch is **main**. The commit history is retained from GitLab, with deployment URLs replaced by reserved example domains before publication. Sanitizing history changes the old commit hashes. GitLab is no longer the release destination.

## GitHub Actions

`.github/workflows/ci.yml` handles pull requests, pushes to main and manual runs. PR code runs with read-only repository access. Official GitHub actions are pinned to commit SHAs.

1. **Release tooling**, **Linux core and desktop**, and **Windows core and WinUI** must pass. Linux uses Debian 12 in a container for a stable glibc baseline and an isolated D-Bus/Xvfb native tray test. Windows uses GitHub's **windows-2025** hosted runner, Rust MSVC and .NET 10 to run native Rust checks and compile WinUI. It does not need a self-hosted runner. The hosted Windows job does not claim the interactive desktop smoke test; run `scripts/windows-smoke.ps1` in a signed-in desktop session for that check.
2. On main, **Plan release** runs semantic-release in dry-run mode and records the candidate version, source commit and notes. `feat:` increments minor, `fix:` and other changes increment patch, and `!` or `BREAKING CHANGE:` increments major. With no existing version tag, the first release is 1.0.0.
3. **Package Linux** and **Package Windows** build that same version and source commit. Cargo, Cargo.lock, Qt, the WinUI assembly and MSIX identity are stamped only in disposable build workspaces. No version-only commit is pushed.
4. **Publish GitHub release** verifies both platform manifests, source commits and SHA-256 checksums, then semantic-release creates the tag, GitHub release, notes and attached downloads. Missing or inconsistent assets prevent publication. No automatic issue/PR comments or issue creation occurs.

The main workflow is serialized using a concurrency group with cancellation disabled. Publishing recomputes the release and rejects a mismatched plan. semantic-release also refuses to publish stale main history. Tag pushes do not start another build. A failed job can be rerun from Actions; if main has advanced, use the newer complete run.

Only planning and publishing have `contents: write`. They use the run's short-lived **GITHUB_TOKEN**. No personal access token, imported GitLab RELEASE_TOKEN, manual secret or npm publishing credential is needed. Build jobs have read-only contents permission. Node installs use `npm ci --ignore-scripts`.

## Downloads

- `noridrive-X.Y.Z-linux-x86_64.tar.gz`: Linux CLI, worker, Qt desktop frontend, docs and release notes. Requires Qt 6.4+, glibc compatible with Debian 12, D-Bus and a Secret Service. This is not a standalone AppImage.
- `noridrive-X.Y.Z-windows-x86_64.zip`: complete self-contained WinUI app, worker and dependencies. Extract and start `noridrive-ui.exe`.
- `noridrive-X.Y.Z-windows-x86_64.msix`: **unsigned** package with development identity. Signing/trust or a Microsoft Store identity is still required for normal installation/submission. The workflow does not publish to the Store.
- `noridrive-X.Y.Z-windows-x86_64-cli.exe`: command-line executable, not the WinUI app.
- `SHA256SUMS` and `BUILD-INFO.json`: checksums and provenance for all published files.

Intermediate Actions artifacts expire after 90 days. Release assets remain attached to the GitHub release until removed.

## GitHub Pages

`.github/workflows/pages.yml` deploys the static `website/` folder on website changes to main or manual dispatch. Pages must use **GitHub Actions** as its build source. The deployment uses `pages: write` and `id-token: write`; no separate hosting credential or external hosting service is used.

The site is at [yannickdreher.github.io/NoriDrive](https://yannickdreher.github.io/NoriDrive/). It supports English/German, works under the project subpath and links to GitHub Releases. It retrieves public release assets to update download buttons; until a release exists or if the API is unavailable, those buttons open the release list. Language preference is stored locally. No analytics, external fonts, accounts or tracking cookies are included.

### Microsoft Store

The Store button is initially disabled and labeled **coming soon**. After a real listing is live, set `microsoftStoreProductId` in `website/config.js` to its 12-character Partner Center product ID. On Windows the button opens `ms-windows-store://pdp/?ProductId=...`; on other systems it opens the web listing. A separate browser link is also available. See [Microsoft's Store URI documentation](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-store-app).

This is separate from sideloading. The repository currently builds an unsigned development MSIX. A public sideload download needs a trusted signature and optionally an `.appinstaller` update manifest. No certificate, signing credentials, invented Store ID or automatic Store submission is included. The `ms-appinstaller` web protocol is disabled by default on current Windows installations; it is not used here. See [Microsoft's distribution guidance](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/publish-first-app).

## Local verification

```sh
npm ci --ignore-scripts
npm test
```

Release tests cover SemVer rules, notes for conventional and other commits, version stamping and rejection of missing, modified or mismatched assets. Serve `website/` through a local static server to preview it. Native desktop verification is described in [development](development.md).
