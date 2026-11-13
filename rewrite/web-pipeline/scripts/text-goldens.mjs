/** Run from rewrite/web-pipeline: node scripts/text-goldens.mjs
 * Build (repository root):
 * (set CARGO_TARGET_DIR first to keep the build out of the source tree)
 * cargo build --manifest-path rewrite/src-tauri/Cargo.toml --features fidelity-eval --bin fidelity_eval --bin punctuation_probe --example text_tokenizer_goldens -j 6
 * Copy the built executables into .scratch/bin before running this script.
 * The exact evaluator commands and outputs are retained in a unique .scratch subfolder.
 */
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { syntheticInputs } from "./text-inputs.mjs";

const pkg = path.resolve(import.meta.dirname, "..");
const scratch = path.join(pkg, ".scratch");
const modelRoot = process.env.SONOJOT_MODELS_ROOT ?? path.join(process.env.LOCALAPPDATA, "SonoJot/models/rewrite");
const modelDir = process.env.SONOJOT_PUNCT_MODEL_DIR ?? path.join(modelRoot, "punctuation/sonojot-punct-v3");
fs.mkdirSync(scratch, { recursive: true });
const temp = fs.mkdtempSync(path.join(scratch, "text-goldens-"));
const inputs = syntheticInputs();
const rows = inputs.flatMap((row) => ["clean", "format"].map((mode) => ({ ...row, id: `${row.id}-${mode}`, mode })));
const writeLines = (name, values) => {
  const file = path.join(temp, name);
  fs.writeFileSync(file, values.map((v) => JSON.stringify(v)).join("\n") + "\n");
  return file;
};
const readLines = (file) => fs.readFileSync(file, "utf8").split(/\r?\n/).filter(Boolean).map(JSON.parse);
const commands = [];
function run(binary, args, label) {
  const command = path.join(scratch, "bin", binary + ".exe");
  commands.push({ command, args });
  fs.writeFileSync(path.join(temp, "commands.json"), JSON.stringify(commands, null, 2));
  console.log(`${label}: ${rows.length} synthetic mode rows`);
  const log = fs.openSync(path.join(temp, label + ".log"), "w");
  const result = spawnSync(command, args, {
    cwd: pkg,
    stdio: ["ignore", log, log],
    windowsHide: true,
    env: { ...process.env, SONOJOT_PUNCT_THREADS: "4" },
  });
  fs.closeSync(log);
  if (result.status !== 0) throw new Error(`${label} failed (${result.status}): ${result.error ?? "see scratch log"}`);
}
function evaluate(rules) {
  const input = writeLines(
    rules ? "rules-input.jsonl" : "input.jsonl",
    rows.map((row) => ({
      id: row.id,
      mode: row.mode === "format" ? "format_only" : "clean",
      input_text: row.input,
      ...(rules ? { scope: "meeting" } : {}),
    })),
  );
  const output = path.join(temp, rules ? "rules-output.jsonl" : "output.jsonl");
  run(
    "fidelity_eval",
    [
      "--input",
      input,
      "--output",
      output,
      "--models-root",
      modelRoot,
      "--variant",
      "host",
      "--cleanup-policy",
      "conservative",
      "--punctuation-model-dir",
      modelDir,
      "--written-form",
      "model",
    ],
    rules ? "rules" : "pipeline",
  );
  return new Map(readLines(output).map((row) => [row.id, row]));
}
const pipeline = evaluate(false),
  rules = evaluate(true);
const cases = rows.map((row) => {
  const result = pipeline.get(row.id),
    rule = rules.get(row.id);
  if (typeof result?.final_text !== "string" || typeof rule?.final_text !== "string")
    throw new Error(`Missing ${row.id}`);
  if (result.punctuation_stage && result.punctuation_stage !== "punctuation_model")
    throw new Error(`Fallback: ${row.id}`);
  return { ...row, expected: result.final_text, rulesExpected: rule.final_text };
});
const config = JSON.parse(fs.readFileSync(path.join(modelDir, "punct-config.json"), "utf8"));
const exceptions = new Set(config.lowercase_words_except);
const samples = new Set([
  ".",
  ",",
  "?",
  " café",
  " cafe\u0301",
  " naïve",
  " İstanbul",
  " Straße",
  " ٢٣",
  " 0123456789",
  " don't",
  " don’t",
  " 😀",
  " 👩🏽‍💻",
  " 中文",
  " 日本語",
  " 한글",
  " a\u200db",
  " <mask>",
  " \u00a0",
  " Ⅳ",
  " ﬁ",
  " \u0000",
]);
for (const row of cases)
  for (const match of row.rulesExpected.matchAll(/[\p{L}\p{N}]+(?:['’][\p{L}\p{N}]+)*/gu)) {
    const lower = match[0].toLowerCase();
    samples.add(config.word_prefix + (exceptions.has(lower) ? match[0] : lower));
  }
const tokenInput = path.join(temp, "token-input.json"),
  tokenOutput = path.join(temp, "token-output.json");
fs.writeFileSync(tokenInput, JSON.stringify([...samples]));
run("text_tokenizer_goldens", [path.join(modelDir, "tokenizer.json"), tokenInput, tokenOutput], "tokenizer");
const tokenizerCases = JSON.parse(fs.readFileSync(tokenOutput, "utf8"));
const sourceData = (file) =>
  JSON.parse(fs.readFileSync(path.resolve(pkg, "../src-tauri/src/written_form", file), "utf8"));
const classes = new Set(
  "CARDINAL DECIMAL MONEY PERCENT MAGNITUDE YEAR DATE TIME ORDINAL MEASURE VERSION MODEL AMP".split(" "),
);
const spanInputs = ["reference_cases.json", "round2_cases.json"].flatMap((file) =>
  sourceData(file)
    .filter((row) => classes.has(row[1]))
    .map((row) => ({ text: row[0], class: row[1] })),
);
for (const family of sourceData("models.json").families) {
  for (const example of family.examples ?? []) spanInputs.push({ text: example.spoken, class: "MODEL" });
}
const termData = sourceData("term_cases.json");
for (const rows of Object.values(termData)) {
  if (!Array.isArray(rows)) continue;
  for (const row of rows)
    if (Array.isArray(row) && classes.has(row[1])) {
      spanInputs.push({ text: row[0], class: row[1], ...(row.length >= 5 ? { before: row[2], after: row[3] } : {}) });
    }
}
const spanCases = spanInputs.map((row, i) => ({ id: `span-${i}`, ...row }));
const spanInput = writeLines("span-input.jsonl", spanCases),
  spanOutput = path.join(temp, "span-output.jsonl");
run(
  "fidelity_eval",
  ["--written-form-replay", "span", "--written-form", "rules", "--input", spanInput, "--output", spanOutput],
  "spans",
);
const spanResults = new Map(readLines(spanOutput).map((row) => [row.id, row]));
const writtenFormCases = spanCases.map((row) => ({ ...row, expected: spanResults.get(row.id).rendered }));
const fixtures = {
  policy: "conservative-v1; local-rules; sonojot-punct-v3; written-form model for Clean, off for Format",
  inputs: inputs.length,
  rows: cases,
  tokenizerCases,
  writtenFormCases,
  termCases: termData.render_terms,
};
fs.mkdirSync(path.join(pkg, "test/fixtures"), { recursive: true });
fs.writeFileSync(path.join(pkg, "test/fixtures/text-parity.json"), JSON.stringify(fixtures, null, 2) + "\n");
console.log(`Wrote ${cases.length} goldens, ${tokenizerCases.length} tokenizer probes. Evidence: ${temp}`);
