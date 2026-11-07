import { options, analysisPlugins } from "./ci/release/options.mjs";

export default { ...options, plugins: [...analysisPlugins, "./ci/release/plugin.mjs"] };
