// `@sonojot/transcribe/language` must load wherever TypeScript is only stripped,
// such as Node's built-in type stripping, which refuses syntax that emits code.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { test } from "node:test";

const source = new URL("../src/language.ts", import.meta.url);

test("language.ts survives strip-only type stripping", () => {
  // Default mode is "strip": it throws on parameter properties, enums and namespaces.
  assert.doesNotThrow(() => stripTypeScriptTypes(readFileSync(source, "utf8")));
});

test("language.ts runs under Node's own type stripping, without a loader", () => {
  const script = `
    const { supportsLanguage, UnsupportedLanguageError } = await import(${JSON.stringify(source.href)});
    const error = new UnsupportedLanguageError("de-DE");
    console.log(JSON.stringify([supportsLanguage("en-GB"), supportsLanguage("de"), error.language, error.name]));
  `;
  const out = execFileSync(process.execPath, ["--experimental-strip-types", "--no-warnings", "--input-type=module", "-e", script], {
    encoding: "utf8",
    // The test runner's loader (tsx) must not leak into the child.
    env: { ...process.env, NODE_OPTIONS: "" },
  });
  assert.deepEqual(JSON.parse(out), [true, false, "de-DE", "UnsupportedLanguageError"]);
});
