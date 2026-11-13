import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync, rmSync } from "node:fs";
import { randomUUID } from "node:crypto";
// Keep relative module imports based at the final output directory.
const file = `.worker-types-${randomUUID()}.d.ts`;
try {
  const result = spawnSync(
    process.execPath,
    [
      "node_modules/wrangler/bin/wrangler.js",
      "types",
      file,
      "--env",
      "staging",
      "--strict-vars",
      "false",
    ],
    { stdio: "inherit" },
  );
  if (result.status !== 0) process.exitCode = result.status ?? 1;
  else
    writeFileSync(
      "worker-configuration.d.ts",
      readFileSync(file, "utf8")
        .split("\n")
        .filter(
          (line) =>
            !line.startsWith("// Generated") &&
            !line.startsWith("// Runtime types generated"),
        )
        .join("\n"),
    );
} finally {
  rmSync(file, { force: true });
}
