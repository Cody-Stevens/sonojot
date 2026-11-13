// SentencePiece pieces to text, matching onnx-asr's DECODE_SPACE_PATTERN
// r"\A\s|\s\B|(\s)\b" with Python's Unicode \w: a space survives only when a
// word character follows it and it is not at the start.
const WORD = /[\p{L}\p{N}_]/u;
const SPACE = /\s/u;

export function detokenize(pieces: readonly string[]): string {
  const text = pieces.join("").replaceAll("▁", " ");
  let out = "";
  const chars = Array.from(text);
  for (let i = 0; i < chars.length; i++) {
    const ch = chars[i]!;
    if (!SPACE.test(ch)) {
      out += ch;
      continue;
    }
    const next = chars[i + 1];
    if (i > 0 && next !== undefined && WORD.test(next)) out += " ";
  }
  return out;
}
