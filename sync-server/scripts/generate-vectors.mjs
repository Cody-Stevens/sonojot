// Rebuild the public synthetic Rust fixtures, never an account's runtime keys.
import { spawnSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
const root = fileURLToPath(new URL("../../", import.meta.url));
const result = spawnSync(
  "cargo",
  [
    "run",
    "--quiet",
    "--manifest-path",
    "rewrite/src-tauri/Cargo.toml",
    "--example",
    "sync_vectors",
    "--features",
    "sync-test-vectors",
  ],
  { cwd: root, encoding: "utf8", windowsHide: true, timeout: 300000 },
);
if (result.status !== 0) {
  process.stderr.write(result.stderr ?? "Vector generator failed");
  process.exit(1);
}
const data = JSON.parse(result.stdout);
writeFileSync(
  new URL("../../rewrite/src-tauri/tests/fixtures/sync-security-vectors.json", import.meta.url),
  JSON.stringify(data, null, 2) + "\n",
);
console.log("Regenerated five public synthetic interoperability vectors.");
