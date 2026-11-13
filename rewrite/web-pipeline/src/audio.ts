// Audio in: decode any format the browser can play, then 16 kHz mono float.
// Long recordings are cut at their quietest moment so each piece fits the
// encoder comfortably; voice notes are usually a single piece.
export const SAMPLE_RATE = 16_000;

/**
 * Decode a recording (WebM/Opus, MP4/AAC, WAV, MP3...) to 16 kHz mono.
 * Runs on the main thread: Web Workers have no AudioContext. In a browser
 * without Web Audio, only uncompressed WAV can be read.
 */
export async function decodeAudio(input: Blob | ArrayBuffer): Promise<Float32Array> {
  const bytes = input instanceof Blob ? await input.arrayBuffer() : input.slice(0);
  const Context =
    globalThis.OfflineAudioContext ?? (globalThis as { webkitOfflineAudioContext?: typeof OfflineAudioContext }).webkitOfflineAudioContext;
  if (!Context) {
    const wav = readWav(bytes);
    if (wav) return resample(wav.samples, wav.sampleRate);
    throw new Error("This browser cannot decode audio (no Web Audio); pass a WAV file or 16 kHz Float32Array samples.");
  }
  // decodeAudioData resamples to the context's rate.
  const context = new Context(1, 1, SAMPLE_RATE);
  const buffer = await context.decodeAudioData(bytes);
  return downmix(buffer);
}

/** Mono samples from an uncompressed WAV file (16-bit PCM or 32-bit float), or undefined for anything else. */
export function readWav(bytes: ArrayBuffer): { samples: Float32Array; sampleRate: number } | undefined {
  const view = new DataView(bytes);
  const tag = (at: number) => String.fromCharCode(view.getUint8(at), view.getUint8(at + 1), view.getUint8(at + 2), view.getUint8(at + 3));
  if (bytes.byteLength < 12 || tag(0) !== "RIFF" || tag(8) !== "WAVE") return undefined;
  let format = 0;
  let channels = 0;
  let sampleRate = 0;
  let bits = 0;
  for (let offset = 12; offset + 8 <= bytes.byteLength; ) {
    const size = view.getUint32(offset + 4, true);
    const body = offset + 8;
    if (tag(offset) === "fmt ") {
      format = view.getUint16(body, true);
      channels = view.getUint16(body + 2, true);
      sampleRate = view.getUint32(body + 4, true);
      bits = view.getUint16(body + 14, true);
      if (format === 0xfffe && size >= 26) format = view.getUint16(body + 24, true); // WAVE_FORMAT_EXTENSIBLE
    } else if (tag(offset) === "data") {
      const pcm16 = format === 1 && bits === 16;
      const float32 = format === 3 && bits === 32;
      if ((!pcm16 && !float32) || !channels || !sampleRate) return undefined;
      const width = bits / 8;
      const frames = Math.floor(Math.min(size, bytes.byteLength - body) / width / channels);
      const samples = new Float32Array(frames);
      for (let i = 0; i < frames; i++) {
        let sum = 0;
        for (let c = 0; c < channels; c++) {
          const at = body + (i * channels + c) * width;
          sum += pcm16 ? view.getInt16(at, true) / 32768 : view.getFloat32(at, true);
        }
        samples[i] = sum / channels;
      }
      return { samples, sampleRate };
    }
    offset = body + size + (size & 1);
  }
  return undefined;
}

function downmix(buffer: AudioBuffer): Float32Array {
  if (buffer.numberOfChannels === 1) return buffer.getChannelData(0).slice();
  const out = new Float32Array(buffer.length);
  for (let c = 0; c < buffer.numberOfChannels; c++) {
    const channel = buffer.getChannelData(c);
    for (let i = 0; i < out.length; i++) out[i]! += channel[i]!;
  }
  const scale = 1 / buffer.numberOfChannels;
  for (let i = 0; i < out.length; i++) out[i]! *= scale;
  return out;
}

/** Linear-interpolation resampler for callers that already hold PCM at another rate. */
export function resample(samples: Float32Array, fromRate: number): Float32Array {
  if (fromRate === SAMPLE_RATE) return samples;
  const ratio = fromRate / SAMPLE_RATE;
  const out = new Float32Array(Math.floor(samples.length / ratio));
  for (let i = 0; i < out.length; i++) {
    const position = i * ratio;
    const left = Math.floor(position);
    const right = Math.min(left + 1, samples.length - 1);
    const weight = position - left;
    out[i] = samples[left]! * (1 - weight) + samples[right]! * weight;
  }
  return out;
}

export interface SplitOptions {
  /** Longest piece sent to the encoder. */
  maxSeconds?: number;
  /** How far back from the limit to look for a pause. */
  searchSeconds?: number;
}

/** [start, end) sample ranges covering the whole input. */
export function splitForRecognition(samples: Float32Array, options: SplitOptions = {}): [number, number][] {
  const max = Math.round((options.maxSeconds ?? 120) * SAMPLE_RATE);
  const search = Math.round((options.searchSeconds ?? 15) * SAMPLE_RATE);
  const frame = SAMPLE_RATE / 50; // 20 ms
  const ranges: [number, number][] = [];
  let start = 0;
  while (samples.length - start > max) {
    const from = start + max - Math.min(search, max / 2);
    const to = start + max;
    let best = to;
    let quietest = Infinity;
    for (let at = from; at + frame <= to; at += frame) {
      let energy = 0;
      for (let i = at; i < at + frame; i++) energy += samples[i]! * samples[i]!;
      if (energy < quietest) {
        quietest = energy;
        best = at + frame / 2;
      }
    }
    ranges.push([start, best]);
    start = best;
  }
  if (samples.length > start || ranges.length === 0) ranges.push([start, samples.length]);
  return ranges;
}
