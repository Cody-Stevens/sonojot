// SonoJot Parakeet (a fine-tune of NVIDIA Parakeet TDT 0.6B v2, CC BY 4.0) in ONNX Runtime: log-mel features,
// FastConformer encoder, then greedy token-and-duration (TDT) decoding.
// Decoding follows onnx-asr 0.12 (NemoConformerTdt + _AsrWithTransducerDecoding)
// so the text matches the Python reference used for the accuracy numbers.
import type { Ort, Session, Tensor } from "../ort.ts";
import { detokenize } from "./detokenize.ts";

export const SAMPLE_RATE = 16_000;

export interface ParakeetSessions {
  /** nemo128.onnx: waveforms [1, N] -> features [1, 128, T]. */
  preprocessor: Session;
  /** encoder: audio_signal [1, 128, T] -> outputs [1, 1024, T/8]. */
  encoder: Session;
  /** decoder_joint: one encoder frame + previous token + LSTM states -> logits. */
  decoder: Session;
}

export interface ParakeetOptions {
  /** onnx-asr's default; stops a runaway frame from emitting forever. */
  maxTokensPerStep?: number;
}

/** Thrown when an AbortSignal stops recognition between steps. */
export class AbortError extends Error {
  constructor() {
    super("Transcription was cancelled.");
    this.name = "AbortError";
  }
}

export function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted) throw new AbortError();
}

export class Parakeet {
  readonly #ort: Ort;
  readonly #s: ParakeetSessions;
  readonly #vocab: string[];
  readonly #blank: number;
  readonly #maxTokensPerStep: number;

  /** `vocabText` is vocab.txt: one "token id" pair per line, "<blk>" last. */
  constructor(ort: Ort, sessions: ParakeetSessions, vocabText: string, options: ParakeetOptions = {}) {
    this.#ort = ort;
    this.#s = sessions;
    this.#vocab = parseVocab(vocabText);
    const blank = this.#vocab.indexOf("<blk>");
    if (blank < 0) throw new Error("Parakeet vocabulary has no <blk> token.");
    this.#blank = blank;
    this.#maxTokensPerStep = options.maxTokensPerStep ?? 10;
  }

  /** Transcribe one stretch of 16 kHz mono audio (callers split long audio). */
  async transcribe(samples: Float32Array, signal?: AbortSignal): Promise<string> {
    if (samples.length === 0) return "";
    const { Tensor } = this.#ort;
    throwIfAborted(signal);
    const pre = await this.#s.preprocessor.run({
      waveforms: new Tensor("float32", samples, [1, samples.length]),
      waveforms_lens: new Tensor("int64", BigInt64Array.of(BigInt(samples.length)), [1]),
    });
    throwIfAborted(signal);
    const enc = await this.#s.encoder.run({ audio_signal: pre.features!, length: pre.features_lens! });
    pre.features!.dispose();
    const out = enc.outputs!;
    const frames = await frameMajor(out);
    const length = Number((await enc.encoded_lengths!.getData())[0]);
    out.dispose();
    const tokens = await this.#decode(frames, Math.min(length, frames.count), signal);
    return detokenize(tokens.map((id) => this.#vocab[id] ?? ""));
  }

  async #decode(frames: Frames, count: number, signal?: AbortSignal): Promise<number[]> {
    const { Tensor } = this.#ort;
    const vocabSize = this.#vocab.length;
    const targetLength = new Tensor("int32", Int32Array.of(1), [1]);
    let state1 = await this.#zeroState("input_states_1");
    let state2 = await this.#zeroState("input_states_2");
    const tokens: number[] = [];
    let t = 0;
    let emitted = 0;
    let steps = 0;
    while (t < count) {
      if ((++steps & 63) === 0) throwIfAborted(signal);
      const frame = frames.data.subarray(t * frames.dim, (t + 1) * frames.dim);
      const result = await this.#s.decoder.run({
        encoder_outputs: new Tensor("float32", frame, [1, frames.dim, 1]),
        targets: new Tensor("int32", Int32Array.of(tokens.length ? tokens[tokens.length - 1]! : this.#blank), [1, 1]),
        target_length: targetLength,
        input_states_1: state1,
        input_states_2: state2,
      });
      const logits = result.outputs!.data as Float32Array;
      const token = argmax(logits, 0, vocabSize);
      const step = argmax(logits, vocabSize, logits.length) - vocabSize;
      if (token !== this.#blank) {
        state1.dispose();
        state2.dispose();
        state1 = result.output_states_1!;
        state2 = result.output_states_2!;
        tokens.push(token);
        emitted++;
      } else {
        result.output_states_1!.dispose();
        result.output_states_2!.dispose();
      }
      result.outputs!.dispose();
      result.prednet_lengths?.dispose();
      if (step > 0) {
        t += step;
        emitted = 0;
      } else if (token === this.#blank || emitted === this.#maxTokensPerStep) {
        t += 1;
        emitted = 0;
      }
    }
    return tokens;
  }

  async #zeroState(name: string): Promise<Tensor> {
    // Shapes are [layers, batch, hidden]; the export declares layers and hidden.
    const meta = (this.#s.decoder as { inputMetadata?: readonly { name: string; shape?: readonly (number | string)[] }[] })
      .inputMetadata?.find((m) => m.name === name);
    const layers = typeof meta?.shape?.[0] === "number" ? meta.shape[0] : 2;
    const hidden = typeof meta?.shape?.[2] === "number" ? meta.shape[2] : 640;
    return new this.#ort.Tensor("float32", new Float32Array(layers * hidden), [layers, 1, hidden]);
  }
}

interface Frames {
  data: Float32Array;
  count: number;
  dim: number;
}

/** Encoder output is [1, dim, frames]; the decoder wants one contiguous frame at a time. */
async function frameMajor(output: Tensor): Promise<Frames> {
  const [, dim, count] = output.dims as [number, number, number];
  const source = (await output.getData()) as Float32Array;
  const data = new Float32Array(dim * count);
  for (let d = 0; d < dim; d++) {
    const row = d * count;
    for (let t = 0; t < count; t++) data[t * dim + d] = source[row + t]!;
  }
  return { data, count, dim };
}

/** First index of the maximum in [from, to), like numpy argmax. */
function argmax(values: Float32Array, from: number, to: number): number {
  let best = from;
  let max = values[from]!;
  for (let i = from + 1; i < to; i++) {
    const value = values[i]!;
    if (value > max) {
      max = value;
      best = i;
    }
  }
  return best;
}

export function parseVocab(text: string): string[] {
  const vocab: string[] = [];
  for (const line of text.split("\n")) {
    if (!line) continue;
    const space = line.lastIndexOf(" ");
    const id = Number(line.slice(space + 1));
    if (space < 0 || !Number.isInteger(id)) throw new Error(`Bad vocabulary line: ${line}`);
    vocab[id] = line.slice(0, space);
  }
  return vocab;
}
