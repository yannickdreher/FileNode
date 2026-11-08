import { createHash } from "node:crypto";
import { readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";

export function parseVersion(version) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) throw new Error("A stable X.Y.Z release version is required.");
  if (version.split(".").some(part => Number(part) > 65535)) throw new Error("Release version exceeds MSIX version limits.");
  return version;
}
export function filenames(version, platform) {
  parseVersion(version);
  const prefix = `filenode-${version}`;
  if (platform === "linux") return [`${prefix}-linux-x86_64.tar.gz`];
  if (platform === "windows") return [`${prefix}-windows-x86_64.zip`, `${prefix}-windows-x86_64.msix`, `${prefix}-windows-x86_64-cli.exe`];
  throw new Error("Unsupported release platform");
}
export async function checksum(filename) {
  const metadata = await stat(filename);
  if (!metadata.isFile() || metadata.size === 0) throw new Error(`Missing or empty release asset: ${path.basename(filename)}`);
  return createHash("sha256").update(await readFile(filename)).digest("hex");
}
export async function createManifest(directory, version, platform, commit) {
  if (!/^[a-f0-9]{40}$/.test(commit ?? "")) throw new Error("The source commit SHA is required.");
  const files = [];
  for (const name of filenames(version, platform)) files.push({ name, sha256: await checksum(path.join(directory, name)) });
  await writeFile(path.join(directory, `build-${platform}.json`), JSON.stringify({ version, platform, commit, files }, null, 2) + "\n");
}
export async function verifyArtifacts(directory, version, commit) {
  const files = [];
  for (const platform of ["linux", "windows"]) {
    const manifest = JSON.parse(await readFile(path.join(directory, `build-${platform}.json`), "utf8"));
    if (manifest.version !== version || manifest.commit !== commit || manifest.platform !== platform) throw new Error(`Mismatched ${platform} build version or commit`);
    const expected = filenames(version, platform);
    if (!Array.isArray(manifest.files) || manifest.files.length !== expected.length) throw new Error(`Incomplete ${platform} release assets`);
    for (const name of expected) {
      const entry = manifest.files.find(file => file.name === name);
      if (!entry || entry.sha256 !== await checksum(path.join(directory, name))) throw new Error(`Release asset checksum mismatch: ${name}`);
      files.push(entry);
    }
  }
  return files;
}
