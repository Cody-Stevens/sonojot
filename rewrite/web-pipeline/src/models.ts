// Pinned model files. Every download is checked against these sizes and
// SHA-256 values before it is cached or loaded, whatever host serves it.

export interface ModelFile {
  /** Model folder on the host, e.g. "sonojot-punct-v3". */
  model: string;
  /** File name inside that folder. */
  name: string;
  bytes: number;
  sha256: string;
}

/**
 * SonoJot Parakeet v1 (CC BY 4.0): SonoJot's fine-tune of NVIDIA Parakeet TDT
 * 0.6B v2, the English model SonoJot ships on the desktop and phone. Browser
 * build: encoder weights in 4-bit MatMulNBits (block 64) so one file runs on
 * WebGPU and WASM; see rewrite/evaluation/web-pipeline-2026-11/REPORT.md for how
 * it was made and measured. On Hugging Face the files are in the `web/`
 * folder of `repo` at `revision`.
 */
export const PARAKEET = {
  id: "sonojot-parakeet-v1-web-q4",
  repo: "SonoJot/sonojot-parakeet-v1",
  /** Hugging Face commit that added the browser files. */
  revision: "d4702f0ad09acfbfd6517a37df98906c91423fec",
  path: "web",
  /** Longest audio piece the trimmed positional table supports (2,500 frames of 80 ms). */
  maxSeconds: 195,
  files: {
    preprocessor: {
      model: "sonojot-parakeet-v1-web-q4",
      name: "nemo128.onnx",
      bytes: 138_824,
      sha256: "5b4a84c52eeaa615dc46d781cc7e4598f9b432184831d57493c48adcd01371a9",
    },
    encoder: {
      model: "sonojot-parakeet-v1-web-q4",
      name: "encoder-model.q4.onnx",
      bytes: 366_128_171,
      sha256: "3a256d2bf773a22576000f70e8d7dd56018c708b361b27a59554a5f54bf76b53",
    },
    decoder: {
      model: "sonojot-parakeet-v1-web-q4",
      name: "decoder_joint-model.int8.onnx",
      bytes: 8_998_286,
      sha256: "21af23d179374499aff5b63a98ecab548f738c258fc55402cd8067f14e96dbdd",
    },
    vocab: {
      model: "sonojot-parakeet-v1-web-q4",
      name: "vocab.txt",
      bytes: 9_384,
      sha256: "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d",
    },
  },
} as const satisfies { id: string; repo: string; revision: string; path: string; maxSeconds: number; files: Record<string, ModelFile> };

/** SonoJot Punctuation v3 (CC BY-SA 4.0), unchanged from the desktop download. */
export const PUNCTUATION = {
  id: "sonojot-punct-v3",
  repo: "SonoJot/sonojot-punct-v3",
  /** Hugging Face commit the files come from. */
  revision: "e023ae245a00bac1785bb6a022b16d181e346e48",
  files: {
    model: {
      model: "sonojot-punct-v3",
      name: "model.int8.onnx",
      bytes: 268_364_003,
      sha256: "11f46c02f68a2a05912ed0c65a6dc063f199280b816307fa3ec17ce23a5e2182",
    },
    tokenizer: {
      model: "sonojot-punct-v3",
      name: "tokenizer.json",
      bytes: 34_363_188,
      sha256: "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f",
    },
    config: {
      model: "sonojot-punct-v3",
      name: "punct-config.json",
      bytes: 1_672,
      sha256: "8bbd52775cedbbcf33c6be4402c9cb77d7c6f581fbc10909518384425039c9e4",
    },
  },
} as const satisfies { id: string; repo: string; revision: string; files: Record<string, ModelFile> };

export const ALL_FILES: readonly ModelFile[] = [
  ...Object.values(PARAKEET.files),
  ...Object.values(PUNCTUATION.files),
];

/** Maps a pinned file to the URL it is fetched from. */
export type ModelUrl = (file: ModelFile) => string;

/** Files laid out as `<base>/<model>/<name>`, e.g. a SonoJot-owned host or the app's own origin. */
export function folderUrls(base: string): ModelUrl {
  const root = base.endsWith("/") ? base : `${base}/`;
  return (file) => new URL(`${file.model}/${file.name}`, root).href;
}

/** Where a model folder lives on Hugging Face; `path` is a subfolder of the repository, e.g. "web". */
export interface HuggingFaceSource {
  repo: string;
  revision: string;
  path?: string;
}

/**
 * Hugging Face `resolve` URLs pinned to a commit. Downloads redirect to
 * *.hf.co CDN hosts, so a CSP needs `connect-src https://huggingface.co https://*.hf.co`.
 */
export function huggingFaceUrls(repos: Record<string, HuggingFaceSource>): ModelUrl {
  return (file) => {
    const source = repos[file.model];
    if (!source) throw new Error(`No Hugging Face repository for ${file.model}.`);
    const folder = source.path?.replace(/^\/+|\/+$/g, "");
    return `https://huggingface.co/${source.repo}/resolve/${source.revision}/${folder ? `${folder}/` : ""}${file.name}`;
  };
}

/**
 * The default download source: each model folder at its pinned Hugging Face
 * commit. A page using it needs `connect-src https://huggingface.co https://*.hf.co`.
 */
export const HUGGING_FACE_MODEL_URLS: Record<string, string> = {
  [PARAKEET.id]: `https://huggingface.co/${PARAKEET.repo}/resolve/${PARAKEET.revision}/${PARAKEET.path}/`,
  [PUNCTUATION.id]: `https://huggingface.co/${PUNCTUATION.repo}/resolve/${PUNCTUATION.revision}/`,
};
