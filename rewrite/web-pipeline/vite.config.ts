// Demo and benchmark server. Model files come from a local folder laid out like
// the model host (SONOJOT_MODEL_DIR/<model>/<file>); nothing is fetched from the
// internet unless the page is opened with ?models=hf. `vite preview` adds the strict CSP a host app such as Saidframe uses,
// plus the one addition onnxruntime-web needs ('wasm-unsafe-eval').
//
// SONOJOT_EXTERN_WASM=1 builds the way a host that serves onnxruntime's files
// itself would: the "onnxruntime-web-use-extern-wasm" condition picks a build that
// does not bundle the 28.7 MB .wasm and the pages pass `wasmPaths: "/ort/"`.
// SONOJOT_WASM_PATHS='{"wasm":"/ort/ort-wasm-simd-threaded.asyncify.wasm"}' (JSON)
// passes any other `wasmPaths`. With either, the servers map /ort/ to
// onnxruntime-web's dist folder.
import { appendFileSync, createReadStream, existsSync, statSync } from "node:fs";
import { extname, resolve, sep } from "node:path";
import { defaultClientConditions, defineConfig, type Connect, type Plugin } from "vite";

const modelDir = process.env.SONOJOT_MODEL_DIR;
const benchAudioDir = process.env.SONOJOT_BENCH_AUDIO_DIR;
const externWasm = !!process.env.SONOJOT_EXTERN_WASM;
const wasmPaths: unknown = process.env.SONOJOT_WASM_PATHS ? JSON.parse(process.env.SONOJOT_WASM_PATHS) : externWasm ? "/ort/" : undefined;
const ortDist = resolve(import.meta.dirname, "node_modules/onnxruntime-web/dist");
const TYPES: Record<string, string> = { ".mjs": "text/javascript", ".wasm": "application/wasm" };

export const CSP = [
  "default-src 'self'",
  "img-src 'self' blob: data:",
  "media-src 'self' blob:",
  "style-src 'self' 'unsafe-inline'",
  "script-src 'self' 'wasm-unsafe-eval'",
  "connect-src 'self' https://huggingface.co https://*.hf.co",
  "worker-src 'self'",
  "frame-ancestors 'none'",
  "base-uri 'self'",
  "object-src 'none'",
  "form-action 'self'",
].join("; ");

function serveFolder(prefix: string, root: string | undefined): Connect.NextHandleFunction {
  return (req, res, next) => {
    if (!root || !req.url?.startsWith(prefix)) return next();
    const relative = decodeURIComponent(req.url.slice(prefix.length).split("?")[0]!);
    const path = resolve(root, relative);
    if (!path.startsWith(resolve(root) + sep) || !existsSync(path) || !statSync(path).isFile()) {
      res.statusCode = 404;
      return res.end();
    }
    // Honour a single "bytes=start-[end]" range, as a CDN would, so resumed downloads can be tested.
    const size = statSync(path).size;
    const range = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? "");
    let start = 0;
    let end = size - 1;
    if (range) {
      start = Number(range[1]);
      end = range[2] ? Math.min(Number(range[2]), size - 1) : size - 1;
      if (start >= size || start > end) {
        logRequest(req.url, req.headers.range, 416, 0);
        res.statusCode = 416;
        res.setHeader("content-range", `bytes */${size}`);
        return res.end();
      }
      res.statusCode = 206;
      res.setHeader("content-range", `bytes ${start}-${end}/${size}`);
    }
    res.setHeader("accept-ranges", "bytes");
    res.setHeader("content-type", TYPES[extname(path)] ?? "application/octet-stream");
    res.setHeader("content-length", end - start + 1);
    // Count the bytes actually sent: an aborted download closes the socket early.
    let sent = 0;
    const stream = createReadStream(path, { start, end });
    stream.on("data", (chunk) => (sent += chunk.length));
    res.on("close", () => logRequest(req.url!, req.headers.range, res.statusCode, sent));
    stream.pipe(res);
  };
}

/** SONOJOT_REQUEST_LOG=<file>: one JSON line per model or audio request, for measure.ts. */
function logRequest(url: string, range: string | undefined, status: number, bytes: number): void {
  const log = process.env.SONOJOT_REQUEST_LOG;
  if (log) appendFileSync(log, `${JSON.stringify({ url, range: range ?? null, status, bytes })}\n`);
}

function localFiles(): Plugin {
  const install = (server: { middlewares: Connect.Server }) => {
    server.middlewares.use(serveFolder("/model-files/", modelDir));
    server.middlewares.use(serveFolder("/bench-audio/", benchAudioDir));
    if (wasmPaths) server.middlewares.use(serveFolder("/ort/", ortDist));
  };
  return { name: "sonojot-local-files", configureServer: install, configurePreviewServer: install };
}

export default defineConfig({
  root: resolve(import.meta.dirname, "demo"),
  plugins: [localFiles()],
  worker: { format: "es" },
  ...(externWasm ? { resolve: { conditions: ["onnxruntime-web-use-extern-wasm", ...defaultClientConditions] } } : {}),
  ...(wasmPaths ? { define: { "import.meta.env.VITE_WASM_PATHS": JSON.stringify(wasmPaths) } } : {}),
  optimizeDeps: { exclude: ["onnxruntime-web"] },
  build: {
    outDir: resolve(import.meta.dirname, "demo-build"),
    emptyOutDir: true,
    target: "es2022",
    rollupOptions: {
      input: {
        main: resolve(import.meta.dirname, "demo/index.html"),
        bench: resolve(import.meta.dirname, "demo/bench.html"),
      },
    },
  },
  preview: {
    headers: {
      "Content-Security-Policy": CSP,
      "Permissions-Policy": "microphone=(self)",
      // Optional: cross-origin isolation lets onnxruntime-web use WASM threads.
      ...(process.env.SONOJOT_ISOLATED
        ? { "Cross-Origin-Opener-Policy": "same-origin", "Cross-Origin-Embedder-Policy": "require-corp" }
        : {}),
    },
  },
  server: { fs: { allow: [resolve(import.meta.dirname)] } },
});
