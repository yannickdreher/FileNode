import { readFile, writeFile } from "node:fs/promises";
import { verifyConditions as verifyGitlab, publish as publishGitlab } from "@semantic-release/gitlab";
import { verifyArtifacts } from "./artifacts.mjs";

function gitlabConfig(version) {
  return {
    gitlabUrl: process.env.CI_SERVER_URL,
    assets: [
      { path: `dist/noridrive-${version}-linux-x86_64.tar.gz`, label: "Linux x86_64", target: "generic_package", type: "package" },
      { path: `dist/noridrive-${version}-windows-x86_64.zip`, label: "Windows x64 portable app", target: "generic_package", type: "package" },
      { path: `dist/noridrive-${version}-windows-x86_64.msix`, label: "Windows x64 MSIX (unsigned)", target: "generic_package", type: "package" },
      { path: `dist/noridrive-${version}-windows-x86_64-cli.exe`, label: "Windows x64 command-line executable", target: "generic_package", type: "other" },
      { path: "dist/SHA256SUMS", label: "SHA-256 checksums", target: "generic_package", type: "other" },
      { path: "dist/BUILD-INFO.json", label: "Build provenance", target: "generic_package", type: "other" },
    ],
  };
}

export async function verifyConditions(config, context) {
  if (!process.env.CI_SERVER_URL) throw new Error("CI_SERVER_URL is required for GitLab publishing.");
  return verifyGitlab(gitlabConfig("pending"), context);
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
  return publishGitlab(gitlabConfig(context.nextRelease.version), context);
}
// No success/fail hooks: release automation does not post issue or MR comments.
