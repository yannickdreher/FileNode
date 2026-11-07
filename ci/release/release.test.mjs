import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, readFile, copyFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { analyzeCommits } from "@semantic-release/commit-analyzer";
import { analyzer } from "./options.mjs";
import { generateNotes } from "./notes.mjs";
import { createManifest, verifyArtifacts, filenames, parseVersion } from "./artifacts.mjs";

const commit = "a".repeat(40);
const logger = { log() {}, error() {} };

test("every merged change releases with Conventional Commit version increments", async () => {
  for (const [message, expected] of [["feat: add a feature", "minor"], ["fix: repair sync", "patch"], ["docs: clarify setup", "patch"], ["ci: update runner", "patch"], ["Update translations", "patch"], ["feat!: change protocol", "major"]]) {
    assert.equal(await analyzeCommits(analyzer[1], { commits: [{ message, hash: commit }], cwd: process.cwd(), logger }), expected, message);
  }
});

test("notes include documentation, CI and non-conventional changes", async () => {
  const notes = await generateNotes({}, {
    commits: ["docs: clarify setup", "ci: update runner", "Update translations"].map(message => ({ message, hash: commit })),
    lastRelease: {}, nextRelease: { version: "1.0.0", gitTag: "v1.0.0", gitHead: commit },
    options: { repositoryUrl: "https://gitlab.example.com/noritec/noridrive.git" }, cwd: process.cwd(), logger,
  });
  for (const text of ["clarify setup", "update runner", "Update translations"]) assert.ok(notes.includes(text), text);
  assert.ok(notes.includes("/-/commit/"), "Release notes must link to GitLab commit pages.");
});

test("publishing requires both platforms, the same commit and intact binary checksums", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "noridrive-assets-"));
  try {
    for (const platform of ["linux", "windows"]) {
      for (const name of filenames("1.2.3", platform)) await writeFile(path.join(directory, name), "binary fixture");
      await createManifest(directory, "1.2.3", platform, commit);
    }
    assert.equal((await verifyArtifacts(directory, "1.2.3", commit)).length, 4);
    await assert.rejects(verifyArtifacts(directory, "1.2.3", "b".repeat(40)), /commit/);
    const executable = path.join(directory, filenames("1.2.3", "windows")[2]);
    await writeFile(executable, "modified binary");
    await assert.rejects(verifyArtifacts(directory, "1.2.3", commit), /checksum/);
    await rm(path.join(directory, "build-windows.json"));
    await assert.rejects(verifyArtifacts(directory, "1.2.3", commit), /ENOENT/);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test("one planned version stamps Cargo, lockfile, WinUI, MSIX and Qt together", async () => {
  const root = process.cwd();
  const directory = await mkdtemp(path.join(tmpdir(), "noridrive-stamp-"));
  const files = ["Cargo.toml", "Cargo.lock", "apps/linux/CMakeLists.txt", "apps/windows/NoriDrive.Windows.csproj", "apps/windows/Package.appxmanifest"];
  try {
    for (const filename of files) {
      await mkdir(path.dirname(path.join(directory, filename)), { recursive: true });
      await copyFile(path.join(root, filename), path.join(directory, filename));
    }
    execFileSync(process.execPath, [path.join(root, "ci/stamp-version.mjs"), "12.34.56"], { cwd: directory });
    for (const filename of files) assert.ok((await readFile(path.join(directory, filename), "utf8")).includes("12.34.56"), filename);
    assert.throws(() => parseVersion("../bad"));
    assert.throws(() => parseVersion("65536.0.0"));
  } finally { await rm(directory, { recursive: true, force: true }); }
});
