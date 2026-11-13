// Copy only public source data. No model weights, tokenizer or evaluation cache is read.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";

const source = resolve(import.meta.dirname, "../../src-tauri/src");
const output = resolve(import.meta.dirname, "../src/text/data");
mkdirSync(output, { recursive: true });
const read = (file) => readFileSync(resolve(source, file), "utf8");
const write = (name, data) => writeFileSync(resolve(output, name + ".json"), JSON.stringify(data) + "\n");
const literals = (text) => [...text.matchAll(/"([^"\n]*)"/g)].map((match) => match[1]);

for (const name of ["models", "terms"]) write(name, JSON.parse(read(`written_form/${name}.json`)));
write("lowercase", read("punctuation_lowercase.txt").trimEnd().split(/\r?\n/));
const lists = {};
for (const [file, names] of [
  ["cleanup.rs", ["function_word", "protected_lexeme"]],
  ["structured_clean.rs", ["literal_word", "uncertainty_word", "negation_word", "instruction_word", "prose_boundary"]],
]) {
  const text = read(file);
  for (const name of names) {
    const start = text.indexOf(`fn ${name}(`);
    const end = text.indexOf("\n}", start);
    if (start < 0 || end < 0) throw new Error(`Missing Rust function ${name}`);
    lists[name] = literals(text.slice(start, end));
  }
}
for (const [file, names] of [
  ["structured_clean.rs", ["STRUCTURAL_WORDS", "TLDS", "CONTINUATION_WORDS"]],
  ["written_form/numbers.rs", ["SMALL", "TENS", "ORDINALS", "MONTHS", "AMP_NAMES", "CLAUSE_WORDS"]],
]) {
  const text = read(file);
  for (const name of names) {
    const start = text.indexOf(`const ${name}:`);
    const end = text.indexOf("];", start);
    if (start < 0 || end < 0) throw new Error(`Missing Rust list ${name}`);
    lists[name] = literals(text.slice(start, end));
  }
}
const numbers = read("written_form/numbers.rs");
for (const name of ["UNITS", "DURATIONS"]) {
  const match = numbers.match(new RegExp(`const ${name}: &str\\s*=\\s*"([^"]*)"`));
  if (!match) throw new Error(`Missing Rust word list ${name}`);
  lists[name] = match[1].split(" ");
}
write("words", lists);
