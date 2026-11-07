import semanticRelease from "semantic-release";
import { mkdir, writeFile } from "node:fs/promises";
import { options, analysisPlugins } from "./options.mjs";

if (!process.env.RELEASE_TOKEN) throw new Error("The protected RELEASE_TOKEN variable is required.");
process.env.GITLAB_TOKEN = process.env.RELEASE_TOKEN;
const result = await semanticRelease({ ...options, plugins: analysisPlugins, dryRun: true });
await mkdir(".release", { recursive: true });
const version = result ? result.nextRelease.version : "";
const plan = { version, commit: process.env.CI_COMMIT_SHA, notes: result ? result.nextRelease.notes : "" };
await writeFile(".release/plan.json", JSON.stringify(plan, null, 2) + "\n");
await writeFile(".release/plan.env", `RELEASE_VERSION=${version}\n`);
await writeFile(".release/RELEASE-NOTES.md", plan.notes);
