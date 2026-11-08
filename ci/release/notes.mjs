import { generateNotes as conventionalNotes } from "@semantic-release/release-notes-generator";
import { notesOptions } from "./options.mjs";

export async function generateNotes(config, context) {
  let notes = await conventionalNotes(notesOptions, context);
  const known = new Set(notesOptions.presetConfig.types.map(type => type.type));
  const other = context.commits.filter(commit => {
    const header = commit.message.split("\n", 1)[0];
    if (/^Merge\b/.test(header)) return false;
    const type = /^([a-z]+)(?:\([^)]*\))?!?:\s/.exec(header)?.[1];
    return !known.has(type);
  });
  if (other.length) notes += "\n\n### Other changes\n\n" + other.map(commit => {
    const subject = commit.message.split("\n", 1)[0].replaceAll("<", "&lt;").replaceAll(">", "&gt;");
    return `* ${subject} (${commit.hash.slice(0, 7)})`;
  }).join("\n") + "\n";
  return notes + `\n\nThe Windows portable ZIP contains the complete desktop app and its dependencies. Extract it and run \`filenode-ui.exe\` without installing an MSIX. The MSIX is unsigned and uses the Windows 11 test-package identity. To install it on Windows 11, open Windows PowerShell as administrator in the download folder and run:\n\n\`\`\`powershell\nAdd-AppxPackage -Path ".\\filenode-${context.nextRelease.version}-windows-x86_64.msix" -AllowUnsigned\n\`\`\`\n\nThis test-installation route is available from 0.2.1 onward; earlier MSIX downloads lack the required publisher OID. Double-click installation requires a trusted signed package. Windows 10 users can use the portable ZIP. See the [installation instructions](https://github.com/yannickdreher/FileNode/blob/main/README.md#install-on-windows). The Linux archive requires Qt 6.4+ and a Secret Service.\n`;
}
