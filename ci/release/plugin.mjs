import { readFile, writeFile } from "node:fs/promises";
import { verifyConditions as verifyGithub, publish as publishGithub } from "@semantic-release/github";
import { verifyArtifacts } from "./artifacts.mjs";

export function githubConfig(version) {
  return {
    successComment: false,
    failComment: false,
    failTitle: false,
    releasedLabels: false,
    assets: [
      { path: `dist/filenode-${version}-linux-x86_64.tar.gz`, label: "Linux x86_64" },
      { path: `dist/filenode-${version}-windows-x86_64.zip`, label: "Windows x64 portable app" },
      { path: `dist/filenode-${version}-windows-x86_64.msix`, label: "Windows x64 MSIX (unsigned)" },
      { path: `dist/filenode-${version}-windows-x86_64-cli.exe`, label: "Windows x64 command-line executable" },
      { path: "dist/SHA256SUMS", label: "SHA-256 checksums" },
      { path: "dist/BUILD-INFO.json", label: "Build provenance" },
    ],
  };
}

export async function verifyConditions(config, context) {
  return verifyGithub(githubConfig("pending"), context);
}
export async function verifyRelease(config, context) {
  const plan = JSON.parse(await readFile(".release/plan.json", "utf8"));
  if (plan.version !== context.nextRelease.version || plan.commit !== context.nextRelease.gitHead) throw new Error("Release plan no longer matches the version and source commit. Rebuild before publishing.");
  await verifyArtifacts("dist", plan.version, plan.commit);
}
export async function prepare(config, context) {
  const files = await verifyArtifacts("dist", context.nextRelease.version, context.nextRelease.gitHead);
  await writeFile("dist/SHA256SUMS", files.map(file => `${file.sha256}  ${file.name}\n`).join(""));
  await writeFile("dist/BUILD-INFO.json", JSON.stringify({ version: context.nextRelease.version, commit: context.nextRelease.gitHead, files }, null, 2) + "\n");
}
export async function publish(config, context) {
  return publishGithub(githubConfig(context.nextRelease.version), context);
}
// No success/fail hooks: release automation does not post issue or MR comments.
