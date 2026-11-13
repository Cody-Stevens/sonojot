// Build the browser entry independently of the concurrent demo work.
import { build } from "vite";
import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { gzipSync } from "node:zlib";

const root = resolve(import.meta.dirname, "..");
await build({
  configFile: false,
  root,
  logLevel: "warn",
  build: {
    target: "es2022",
    outDir: resolve(root, ".scratch/text-bundle"),
    emptyOutDir: false,
    minify: true,
    lib: { entry: resolve(root, "src/text/index.ts"), formats: ["es"], fileName: "text" },
  },
});
const files = readdirSync(resolve(root, "src/text/data"))
  .filter((name) => name.endsWith(".json"))
  .sort();
const data = Buffer.concat(files.map((name) => readFileSync(resolve(root, "src/text/data", name))));
const bundle = readFileSync(resolve(root, ".scratch/text-bundle/text.js"));
const exports = await import(new URL("../.scratch/text-bundle/text.js", import.meta.url).href);
const result = await exports.postprocess("pay ten dollars", { mode: "clean" });
if (result.text !== "Pay $10.") throw new Error("Bundled rules smoke check failed");
console.log(
  JSON.stringify(
    {
      dataFiles: files,
      dataBytes: data.length,
      dataGzipBytes: gzipSync(data).length,
      browserBundleBytes: bundle.length,
      browserBundleGzipBytes: gzipSync(bundle).length,
      exportedFunctions: Object.keys(exports),
      bundledRulesSmoke: result.text,
    },
    null,
    2,
  ),
);
