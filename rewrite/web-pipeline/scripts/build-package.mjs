// Builds the published package: src/*.ts to dist/*.js with .d.ts files, so hosts
// need no TypeScript settings of their own and plain Node can import
// @sonojot/transcribe/language. npm runs this before `npm pack` and `npm publish`.
import { execFileSync } from "node:child_process";
import { readFileSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const dist = resolve(root, "dist");
rmSync(dist, { recursive: true, force: true });
execFileSync(process.execPath, [resolve(root, "node_modules/typescript/bin/tsc"), "-p", "tsconfig.build.json"], {
  cwd: root,
  stdio: "inherit",
});

// The compiler rewrites import specifiers but not the worker's URL string. Bundlers
// find the worker through this exact `new URL("./worker.js", import.meta.url)` form.
const index = resolve(dist, "index.js");
const source = readFileSync(index, "utf8");
const fixed = source.replace('new URL("./worker.ts", import.meta.url)', 'new URL("./worker.js", import.meta.url)');
if (fixed === source) throw new Error("dist/index.js does not refer to ./worker.ts; update build-package.mjs");
writeFileSync(index, fixed);
