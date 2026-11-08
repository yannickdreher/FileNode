# Releases and website

The public repository is [yannickdreher/FileNode](https://github.com/yannickdreher/FileNode). The default and release branch is **main**. The commit history is retained from GitLab, with deployment URLs replaced by reserved example domains before publication. Sanitizing history changes the old commit hashes. GitLab is no longer the release destination.

## GitHub Actions

`.github/workflows/ci.yml` handles pull requests, pushes to main and manual runs. PR code runs with read-only repository access. Official GitHub actions are pinned to commit SHAs.

1. **Release tooling**, **Linux core and desktop**, and **Windows core and WinUI** must pass. Linux uses Debian 12 in a container for a stable glibc baseline and an isolated D-Bus/Xvfb native tray test. Windows uses GitHub's **windows-2025** hosted runner, Rust MSVC and .NET 10 to run native Rust checks and compile WinUI. It does not need a self-hosted runner. The hosted Windows job does not claim the interactive desktop smoke test; run `scripts/windows-smoke.ps1` in a signed-in desktop session for that check.
2. On main, **Plan release** runs semantic-release in dry-run mode and records the candidate version, source commit and notes. `feat:` increments minor, `fix:` and other changes increment patch, and `!` or `BREAKING CHANGE:` increments major. The historical `v0.0.0` tag establishes the preview version baseline; it has no release assets. The first packaged release is **0.1.0**. The next feature release is **0.2.0**. The initial 1.0.0 candidate was canceled before publication.
3. **Package Linux** and **Package Windows** build that same version and source commit. Cargo, Cargo.lock, Qt, the WinUI assembly and MSIX identity are stamped only in disposable build workspaces. No version-only commit is pushed.
4. **Publish GitHub release** verifies both platform manifests, source commits and SHA-256 checksums, then semantic-release creates the tag, GitHub release, notes and attached downloads. Missing or inconsistent assets prevent publication. No automatic issue/PR comments or issue creation occurs.

The main workflow is serialized using a concurrency group with cancellation disabled. Publishing recomputes the release and rejects a mismatched plan. semantic-release also refuses to publish stale main history. Tag pushes do not start another build. A failed job can be rerun from Actions; if main has advanced, use the newer complete run.

For a follow-up such as 0.2.1 after 0.2.0, prepare and check the patch on a separate branch, then wait for the running main release to finish before merging. This preserves the previous release's source commit and publication plan, without canceling its workflow. A `fix:` commit after the published 0.2.0 tag plans 0.2.1.

Only planning and publishing have `contents: write`. They use the run's short-lived **GITHUB_TOKEN**. No personal access token, imported GitLab RELEASE_TOKEN, manual secret or npm publishing credential is needed. Build jobs have read-only contents permission. Node installs use `npm ci --ignore-scripts`.

## Downloads

- `filenode-X.Y.Z-linux-x86_64.tar.gz`: Linux CLI, worker, Qt desktop frontend, docs and release notes. Requires Qt 6.4+, glibc compatible with Debian 12, D-Bus and a Secret Service. This is not a standalone AppImage.
- `filenode-X.Y.Z-windows-x86_64.zip`: complete self-contained WinUI app, worker and dependencies. Extract and start `filenode-ui.exe`.
- `filenode-X.Y.Z-windows-x86_64-setup.exe`: per-user Windows installer from 0.2.2 onward, including Start menu/uninstall integration. No MSIX certificate import is needed; the unsigned EXE may still show Windows publisher/SmartScreen notices. Both the checked build and optimized release installer undergo install/uninstall smoke tests on a disposable Windows runner.
- `filenode-X.Y.Z-windows-x86_64.msix`: unsigned packaging artifact requiring trusted signing or Store signing before installation. The 0.2.1 OID/AllowUnsigned instructions were incorrect: executable activations are not permitted by the unsigned hosted-package route. See [installation instructions](../README.md#install-on-windows). The workflow does not publish to the Store.
- `filenode-X.Y.Z-windows-x86_64-cli.exe`: command-line executable, not the WinUI app.
- `SHA256SUMS` and `BUILD-INFO.json`: checksums and provenance for all published files.

Intermediate Actions artifacts expire after 90 days. Release assets remain attached to the GitHub release until removed.

## GitHub Pages

`.github/workflows/pages.yml` deploys the static `website/` folder on website changes to main or manual dispatch. Pages must use **GitHub Actions** as its build source. The deployment uses `pages: write` and `id-token: write`; no separate hosting credential or external hosting service is used.

The site is at [yannickdreher.github.io/FileNode](https://yannickdreher.github.io/FileNode/). It supports English/German, works under the project subpath and links to GitHub Releases. It retrieves public release assets to update download buttons; until a release exists or if the API is unavailable, those buttons open the release list. Language preference is stored locally. No analytics, external fonts, accounts or tracking cookies are included.

### Microsoft Store

The Store button is initially disabled and labeled **coming soon**. After a real listing is live, set `microsoftStoreProductId` in `website/config.js` to its 12-character Partner Center product ID. On Windows the button opens `ms-windows-store://pdp/?ProductId=...`; on other systems it opens the web listing. A separate browser link is also available. See [Microsoft's Store URI documentation](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-store-app).

This is separate from sideloading. The repository currently builds an unsigned development MSIX. A public sideload download needs a trusted signature and optionally an `.appinstaller` update manifest. No certificate, signing credentials, invented Store ID or automatic Store submission is included. The `ms-appinstaller` web protocol is disabled by default on current Windows installations; it is not used here. See [Microsoft's distribution guidance](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/publish-first-app).

## Local verification

```sh
npm ci --ignore-scripts
npm test
```

Release tests cover SemVer rules, notes for conventional and other commits, version stamping and rejection of missing, modified or mismatched assets. Serve `website/` through a local static server to preview it. Native desktop verification is described in [development](development.md).
