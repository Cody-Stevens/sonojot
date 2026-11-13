// Minimal PCM16 WAV reader for tests and scripts (16 kHz expected).
export function readWav16k(bytes: Uint8Array): Float32Array {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let offset = 12;
  let channels = 1;
  let rate = 0;
  let bits = 0;
  while (offset + 8 <= bytes.byteLength) {
    const id = String.fromCharCode(...bytes.subarray(offset, offset + 4));
    const size = view.getUint32(offset + 4, true);
    const body = offset + 8;
    if (id === "fmt ") {
      channels = view.getUint16(body + 2, true);
      rate = view.getUint32(body + 4, true);
      bits = view.getUint16(body + 14, true);
    } else if (id === "data") {
      if (rate !== 16_000 || bits !== 16) throw new Error(`Expected 16 kHz PCM16, got ${rate} Hz ${bits}-bit`);
      const frames = Math.floor(size / 2 / channels);
      const out = new Float32Array(frames);
      for (let i = 0; i < frames; i++) {
        let sum = 0;
        for (let c = 0; c < channels; c++) sum += view.getInt16(body + (i * channels + c) * 2, true);
        out[i] = sum / channels / 32768;
      }
      return out;
    }
    offset = body + size + (size & 1);
  }
  throw new Error("No data chunk in WAV file.");
}
