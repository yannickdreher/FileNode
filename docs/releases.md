# Releases

The GitLab project is `noritec/noridrive`. The default and release branch is **main**. Merge requests run Rust, Linux desktop and release-tooling checks. A successful push/merge to main starts the serialized release child pipeline; tags do not start duplicate pipelines.

1. **plan** runs semantic-release in dry-run mode and records the candidate version, source commit and release notes. `feat:` increments the minor version; `fix:` and all other changes increment the patch version; `!` or `BREAKING CHANGE:` increments the major version. Documentation and CI-only merges also produce a release. With no existing version tag, semantic-release starts at 1.0.0.
2. **linux-build** and **windows-build** build that exact version and source commit. Cargo, Cargo.lock, Qt, the WinUI assembly and MSIX identity are stamped in their disposable job workspaces. No version-only commit is pushed to main.
3. **publish** checks both platform manifests, source commits and SHA-256 checksums before semantic-release creates the tag and GitLab release. It uploads the assets to GitLab's Generic Package Registry and attaches permanent links to the release, alongside generated change notes. The release notes include conventional and non-conventional changes. No automatic issue/MR comments are posted.

The parent job holds `resource_group: noridrive-release` through the complete child pipeline (`strategy: mirror`), preventing concurrent candidates from publishing the same version. Publishing recomputes the release from Git history and refuses a mismatched candidate. A branch that has advanced is not released from stale binaries.

## Downloads

- `noridrive-X.Y.Z-linux-x86_64.tar.gz`: Linux CLI, worker and Qt desktop frontend, documentation and release notes. Requires Qt 6.4+, glibc compatible with Debian 12, D-Bus and a Secret Service. It is not a standalone AppImage.
- `noridrive-X.Y.Z-windows-x86_64.zip`: complete self-contained WinUI app, Rust worker and dependencies. Extract the archive and run `noridrive-ui.exe`.
- `noridrive-X.Y.Z-windows-x86_64.msix`: **unsigned** package with development identity. Signing/trust or the exact Microsoft Store identity is still required for normal installation/submission. The pipeline does not claim Store publication or code signing.
- `noridrive-X.Y.Z-windows-x86_64-cli.exe`: command-line executable. Use `--headless` for command-line operation; the standalone file is not the WinUI app.
- `SHA256SUMS` and `BUILD-INFO.json`: checksums, source commit and release version for the published files.

Intermediate job artifacts expire after 90 days. Published Generic Package Registry files remain until explicitly removed under the project's package retention policies.

## GitLab configuration

`main` is protected. The existing protected **RELEASE_TOKEN** variable is used only by the release tooling as `GITLAB_TOKEN`; it needs API and repository write access for this project. `plan` and `publish` declare the `release` environment. Install scripts are disabled during `npm ci`. Source builds do not need release API credentials.

The instance currently has Linux runners only. The required **windows-build** job is intentionally pending until a Windows shell runner tagged **windows** is configured, as requested. Linux can build in parallel; publication waits for both platforms. No incomplete Windows release is published.

The future Windows runner needs Git, Node.js 24 LTS, .NET 10 SDK, rustup with the repository-pinned MSVC toolchain, Visual Studio C++ x64 build tools and a Windows SDK. It must have access to NuGet, the Rust/npm registries and this GitLab project. Configure it as protected for main releases. `ci/package-windows.ps1` builds the matching Rust core, WinUI portable package and unsigned MSIX; it neither installs the app nor starts synchronization.

Only register the runner on a build machine intended to execute trusted CI code. No runner service was installed on the developer laptop by this change. Once registered, retry the pending Windows job if needed; the original release plan and Linux artifacts must still be available. Otherwise rerun the main pipeline.

## Local verification

```sh
npm ci --ignore-scripts
npm test
```

The release-tooling tests verify SemVer rules, notes for documentation/CI/non-conventional commits, consistent stamping across all build projects and rejection of missing, modified or mismatched binaries. The native Windows offline UI smoke test is described in [development](development.md).
