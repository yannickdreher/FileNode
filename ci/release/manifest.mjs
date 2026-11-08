import { createManifest } from "./artifacts.mjs";
await createManifest("dist", process.argv[2], process.argv[3], process.env.GITHUB_SHA);
