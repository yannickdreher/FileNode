import semanticRelease from "semantic-release";
import { mkdir, writeFile } from "node:fs/promises";
import { options, analysisPlugins } from "./options.mjs";

if (!process.env.GITHUB_TOKEN) throw new Error("GITHUB_TOKEN with repository contents write permission is required.");
const result = await semanticRelease({ ...options, plugins: analysisPlugins, dryRun: true });
await mkdir(".release", { recursive: true });
const version = result ? result.nextRelease.version : "";
const plan = { version, commit: process.env.GITHUB_SHA, notes: result ? result.nextRelease.notes : "" };
await writeFile(".release/plan.json", JSON.stringify(plan, null, 2) + "\n");
await writeFile(".release/plan.env", `RELEASE_VERSION=${version}\n`);
await writeFile(".release/RELEASE-NOTES.md", plan.notes);
if (process.env.GITHUB_OUTPUT) {
  const { appendFile } = await import("node:fs/promises");
  await appendFile(process.env.GITHUB_OUTPUT, `version=${version}\n`);
}
