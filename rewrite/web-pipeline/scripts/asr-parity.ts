// Compare this package's Parakeet decoding (onnxruntime-node, CPU) with the
// onnx-asr reference transcripts written by rewrite/evaluation/web-pipeline-2026-11/asr_wer.py.
//
// usage: tsx scripts/asr-parity.ts MODEL_DIR ENCODER DECODER REFERENCE.jsonl CORPUS_INFERENCE.jsonl
import { readFileSync } from "node:fs";
import { join } from "node:path";
import * as ort from "onnxruntime-node";
import { Parakeet } from "../src/asr/parakeet.ts";
import { readWav16k } from "../test/wav.ts";

const [dir, encoder, decoder, referencePath, inferencePath] = process.argv.slice(2);
if (!dir || !encoder || !decoder || !referencePath || !inferencePath) {
  console.error("usage: tsx scripts/asr-parity.ts MODEL_DIR ENCODER DECODER REFERENCE.jsonl INFERENCE.jsonl");
  process.exit(2);
}
const options: ort.InferenceSession.SessionOptions = { intraOpNumThreads: 6, executionProviders: ["cpu"] };
const engine = new Parakeet(
  ort,
  {
    preprocessor: await ort.InferenceSession.create(join(dir, "nemo128.onnx"), options),
    encoder: await ort.InferenceSession.create(join(dir, encoder), options),
    decoder: await ort.InferenceSession.create(join(dir, decoder), options),
  },
  readFileSync(join(dir, "vocab.txt"), "utf8"),
);
const audio = new Map<string, string>();
for (const line of readFileSync(inferencePath, "utf8").split("\n")) {
  if (line.trim()) {
    const row = JSON.parse(line) as { id: string; audio_path: string };
    audio.set(row.id, row.audio_path);
  }
}
let same = 0;
let total = 0;
let seconds = 0;
let audioSeconds = 0;
for (const line of readFileSync(referencePath, "utf8").split("\n")) {
  if (!line.trim()) continue;
  const reference = JSON.parse(line) as { id: string; text: string };
  const samples = readWav16k(readFileSync(audio.get(reference.id)!));
  const start = performance.now();
  const text = await engine.transcribe(samples);
  seconds += (performance.now() - start) / 1000;
  audioSeconds += samples.length / 16_000;
  total++;
  if (text === reference.text) same++;
  else console.log(`DIFF ${reference.id}\n  ref: ${reference.text}\n  got: ${text}`);
}
console.log(`${same}/${total} identical; RTF ${(seconds / audioSeconds).toFixed(4)}`);
