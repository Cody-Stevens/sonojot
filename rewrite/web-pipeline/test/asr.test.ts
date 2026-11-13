import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { detokenize } from "../src/asr/detokenize.ts";
import { parseVocab } from "../src/asr/parakeet.ts";
import { resample, splitForRecognition, SAMPLE_RATE } from "../src/audio.ts";

test("detokenize matches onnx-asr spacing", () => {
  assert.equal(detokenize(["▁", "1", "3", ".", "3", "%", "▁be", "y", "o", "nd"]), "13.3% beyond");
  assert.equal(detokenize(["▁Hello", ",", "▁world", "."]), "Hello, world.");
  assert.equal(detokenize(["▁it", "'", "s", "▁fine"]), "it's fine");
  // onnx-asr's regex on "  a" drops the first space and keeps the second.
  assert.equal(detokenize(["▁", "▁a"]), " a");
  assert.equal(detokenize([]), "");
});

test("parseVocab keeps ids and the blank token", () => {
  const vocab = parseVocab("<unk> 0\n▁t 1\n▁th 2\n<blk> 3\n");
  assert.deepEqual(vocab, ["<unk>", "▁t", "▁th", "<blk>"]);
  assert.throws(() => parseVocab("broken"));
});

test("short audio is one piece", () => {
  assert.deepEqual(splitForRecognition(new Float32Array(SAMPLE_RATE * 30)), [[0, SAMPLE_RATE * 30]]);
  assert.deepEqual(splitForRecognition(new Float32Array(0)), [[0, 0]]);
});

test("long audio is cut at the quietest point before the limit", () => {
  const seconds = 50;
  const samples = new Float32Array(SAMPLE_RATE * seconds).map((_, i) => Math.sin(i / 7) * 0.5);
  // Silence from 17.0 to 17.5 s: inside the search window (10..20 s for a 20 s limit).
  samples.fill(0, SAMPLE_RATE * 17, SAMPLE_RATE * 17.5);
  const ranges = splitForRecognition(samples, { maxSeconds: 20, searchSeconds: 10 });
  assert.equal(ranges[0]![0], 0);
  const cut = ranges[0]![1] / SAMPLE_RATE;
  assert.ok(cut >= 17 && cut <= 17.5, `cut at ${cut}`);
  for (let i = 1; i < ranges.length; i++) assert.equal(ranges[i]![0], ranges[i - 1]![1]);
  assert.equal(ranges.at(-1)![1], samples.length);
  for (const [start, end] of ranges) assert.ok(end - start <= SAMPLE_RATE * 20);
});

test("resample keeps duration", () => {
  const input = new Float32Array(48_000).fill(0.25);
  const output = resample(input, 48_000);
  assert.equal(output.length, 16_000);
  assert.ok(Math.abs(output[100]! - 0.25) < 1e-6);
  assert.equal(resample(input, 16_000), input);
});

// Real-model check, skipped unless the local model folder and a public clip are present:
// SONOJOT_MODEL_DIR=<host folder> SONOJOT_TEST_WAV=<16 kHz wav> SONOJOT_TEST_TEXT=<expected raw text>
const modelDir = process.env.SONOJOT_MODEL_DIR;
const wav = process.env.SONOJOT_TEST_WAV;
test("Parakeet reproduces the reference transcript", { skip: !modelDir || !wav || !existsSync(wav) }, async () => {
  const ort = await import("onnxruntime-node");
  const { Parakeet } = await import("../src/asr/parakeet.ts");
  const { readWav16k } = await import("./wav.ts");
  const dir = join(modelDir!, "sonojot-parakeet-v1-web-q4");
  const engine = new Parakeet(
    ort,
    {
      preprocessor: await ort.InferenceSession.create(join(dir, "nemo128.onnx")),
      encoder: await ort.InferenceSession.create(join(dir, "encoder-model.q4.onnx")),
      decoder: await ort.InferenceSession.create(join(dir, "decoder_joint-model.int8.onnx")),
    },
    readFileSync(join(dir, "vocab.txt"), "utf8"),
  );
  const text = await engine.transcribe(readWav16k(readFileSync(wav!)));
  if (process.env.SONOJOT_TEST_TEXT) assert.equal(text, process.env.SONOJOT_TEST_TEXT);
  else assert.ok(text.length > 0);
});

test("readWav reads PCM16 and float WAV and refuses other formats", async () => {
  const { readWav } = await import("../src/audio.ts");
  const wav = (format: number, bits: number, channels: number, rate: number, write: (view: DataView, at: number) => void, frames: number) => {
    const width = bits / 8;
    const data = frames * channels * width;
    const view = new DataView(new ArrayBuffer(44 + data));
    const text = (at: number, value: string) => [...value].forEach((ch, i) => view.setUint8(at + i, ch.charCodeAt(0)));
    text(0, "RIFF"), view.setUint32(4, 36 + data, true), text(8, "WAVE"), text(12, "fmt "), view.setUint32(16, 16, true);
    view.setUint16(20, format, true), view.setUint16(22, channels, true), view.setUint32(24, rate, true);
    view.setUint32(28, rate * channels * width, true), view.setUint16(32, channels * width, true), view.setUint16(34, bits, true);
    text(36, "data"), view.setUint32(40, data, true);
    write(view, 44);
    return view.buffer;
  };
  const stereo = readWav(wav(1, 16, 2, 48_000, (v, at) => (v.setInt16(at, 16384, true), v.setInt16(at + 2, 0, true)), 1))!;
  assert.equal(stereo.sampleRate, 48_000);
  assert.deepEqual([...stereo.samples], [0.25]);
  const float = readWav(wav(3, 32, 1, 16_000, (v, at) => v.setFloat32(at, -0.5, true), 1))!;
  assert.deepEqual([...float.samples], [-0.5]);
  assert.equal(readWav(wav(1, 8, 1, 16_000, () => undefined, 1)), undefined);
  assert.equal(readWav(new TextEncoder().encode("OggS not a wav").buffer as ArrayBuffer), undefined);
});
