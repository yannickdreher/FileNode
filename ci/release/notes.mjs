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
  return notes;
}
