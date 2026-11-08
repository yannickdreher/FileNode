export const analyzer = ["@semantic-release/commit-analyzer", {
  preset: "conventionalcommits",
  // Every merged change gets a release, including documentation and CI changes.
  releaseRules: [
    { breaking: true, release: "major" },
    { type: "feat", release: "minor" },
    { release: "patch" },
  ],
}];

export const notesOptions = {
  preset: "conventionalcommits",
  presetConfig: {
    commitUrlFormat: "{{host}}/{{owner}}/{{repository}}/commit/{{hash}}",
    issueUrlFormat: "{{host}}/{{owner}}/{{repository}}/issues/{{id}}",
    compareUrlFormat: "{{host}}/{{owner}}/{{repository}}/compare/{{previousTag}}...{{currentTag}}",
    types: [
    { type: "feat", section: "Features" }, { type: "fix", section: "Bug fixes" },
    { type: "perf", section: "Performance" }, { type: "refactor", section: "Refactoring" },
    { type: "docs", section: "Documentation" }, { type: "ci", section: "CI and releases" },
    { type: "build", section: "Build" }, { type: "test", section: "Tests" },
    { type: "chore", section: "Maintenance" }, { type: "style", section: "Style" },
  ] },
};
export const analysisPlugins = [analyzer, "./ci/release/notes.mjs"];

export const options = {
  branches: ["main"],
  tagFormat: "v${version}",
  repositoryUrl: "https://github.com/yannickdreher/FileNode.git",
};
