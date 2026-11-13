import { readFileSync, writeFileSync, unlinkSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

const directory = fileURLToPath(new URL("../", import.meta.url));

export function stagingConfig(template, settings) {
  const root = structuredClone(template);
  const c = root.env?.staging;
  if (
    !c ||
    root.vars?.SYNC_TEST_IDENTITY ||
    root.routes ||
    root.workers_dev !== false ||
    Object.keys(root.env ?? {}).some((name) => name !== "staging") ||
    c.name !== "sonojot-sync-staging" ||
    c.vars?.ENVIRONMENT !== "staging" ||
    c.vars?.SYNC_TEST_IDENTITY !== "true" ||
    c.routes ||
    c.env ||
    !Array.isArray(c.d1_databases) ||
    c.d1_databases.length !== 1 ||
    c.d1_databases[0].binding !== "DB" ||
    !c.d1_databases[0].database_name.endsWith("-staging") ||
    !Array.isArray(c.r2_buckets) ||
    c.r2_buckets.length !== 2 ||
    c.r2_buckets.some((b) => !b.bucket_name.endsWith("-staging"))
  )
    throw Error(
      "Staging resources must be configured. Production and routes are forbidden.",
    );

  // Only reviewed deployment values can cross from the private overlay.
  if (
    !settings ||
    ["origin", "accountId", "databaseId", "workosClientId"].some(
      (key) =>
        typeof settings[key] !== "string" ||
        settings[key] !== settings[key].trim(),
    ) ||
    !/^https:\/\/sonojot-sync-staging\.[a-z0-9-]+\.workers\.dev$/.test(
      settings.origin ?? "",
    ) ||
    settings.origin === "https://sonojot-sync-staging.example.workers.dev" ||
    !/^[a-f0-9]{32}$/.test(settings.accountId ?? "") ||
    !/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/.test(
      settings.databaseId ?? "",
    ) ||
    settings.databaseId.startsWith("00000000") ||
    !/^client_[A-Za-z0-9]+$/.test(settings.workosClientId ?? "")
  )
    throw Error(
      "Configure the staging origin, account, database and public AuthKit client ID in a private overlay.",
    );
  root.account_id = settings.accountId;
  c.vars.PUBLIC_ORIGIN = settings.origin;
  c.vars.WORKOS_CLIENT_ID = settings.workosClientId;
  c.d1_databases[0].database_id = settings.databaseId;
  return root;
}

export function runStaging(args) {
  // No arbitrary Wrangler arguments: callers cannot replace --env or --config.
  const commands = {
    build: ["deploy", "--dry-run", "--outdir", "dist"],
    deploy: ["deploy"],
    migrate: ["d1", "migrations", "apply", "DB", "--remote"],
    check: null,
  };
  if (args.length !== 1 || !Object.hasOwn(commands, args[0]))
    throw Error("Use staging-guard.mjs check, build, deploy or migrate.");
  const template = JSON.parse(
    readFileSync(resolve(directory, "wrangler.jsonc"), "utf8"),
  );
  const settingsPath =
    process.env.SONOJOT_SYNC_CONFIG ??
    resolve(directory, "../scripts/sync-staging.json");
  let settings;
  try {
    settings = JSON.parse(readFileSync(settingsPath, "utf8"));
  } catch {
    throw Error(
      "Missing private staging config. Set SONOJOT_SYNC_CONFIG to its JSON file.",
    );
  }
  const config = stagingConfig(template, settings);
  if (args[0] === "check") {
    console.log("Staging configuration validated; no deployment performed.");
    return 0;
  }
  // Beside the template so Wrangler's relative source/migration paths still resolve.
  const file = resolve(directory, `.staging-${randomUUID()}.json`);
  writeFileSync(file, JSON.stringify(config), { flag: "wx" });
  try {
    const result = spawnSync(
      process.execPath,
      [
        resolve(directory, "node_modules/wrangler/bin/wrangler.js"),
        ...commands[args[0]],
        "--env",
        "staging",
        "--config",
        file,
      ],
      { cwd: directory, stdio: "inherit", windowsHide: true },
    );
    return result.status ?? 1;
  } finally {
    unlinkSync(file);
  }
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
)
  process.exitCode = runStaging(process.argv.slice(2));
