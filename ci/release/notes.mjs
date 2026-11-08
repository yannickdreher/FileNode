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
  return notes + `\n\nOn Windows 10/11, run \`filenode-${context.nextRelease.version}-windows-x86_64-setup.exe\` for a per-user installation with a Start menu shortcut and uninstaller. The setup executable is unsigned; Windows may display an unknown-publisher/SmartScreen notice. It does not require an MSIX certificate import. Alternatively, extract the complete portable ZIP and start \`filenode-ui.exe\`. The MSIX is unsigned and requires trusted signing or Microsoft Store signing before installation. FileNode's executable activations cannot use the unsigned hosted-package route; the earlier 0.2.1 installation guidance was incorrect. See the [installation instructions](https://github.com/yannickdreher/FileNode/blob/main/README.md#install-on-windows). The Linux archive requires Qt 6.4+ and a Secret Service.\n`;
}
