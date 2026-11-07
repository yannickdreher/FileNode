import { readFile, writeFile } from "node:fs/promises";
import { parseVersion } from "./release/artifacts.mjs";

const version = parseVersion(process.argv[2] ?? "");
async function replace(filename, pattern, replacement) {
  const text = await readFile(filename, "utf8");
  if (!pattern.test(text)) throw new Error(`Cannot stamp version in ${filename}`);
  await writeFile(filename, text.replace(pattern, replacement));
}
await replace("Cargo.toml", /^(version\s*=\s*)"[^"]+"/m, `$1"${version}"`);
await replace("Cargo.lock", /(\[\[package\]\]\r?\nname = "noridrive"\r?\nversion = )"[^"]+"/, `$1"${version}"`);
await replace("apps/linux/CMakeLists.txt", /(project\(NoriDriveLinux VERSION )\S+/, `$1${version}`);
await replace("apps/windows/NoriDrive.Windows.csproj", /<Version>[^<]+<\/Version>/, `<Version>${version}</Version>`);
await replace("apps/windows/Package.appxmanifest", /(<Identity\b[^>]*\bVersion=")[^"]+"/, `$1${version}.0"`);
