import { defineConfig } from "@playwright/test";

// SONOJOT_UI_PORT lets a second checkout run the suite without reusing (and
// silently testing) another checkout's dev server on the default port.
const port = Number(process.env.SONOJOT_UI_PORT ?? 1420);
const baseURL = `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: "./src/tests",
  testIgnore: "**/typography-paint.spec.js", // Dedicated two-engine/site config.
  outputDir: "../artifacts/rewrite-20260910/frontend/test-results",
  workers: 1,
  reporter: "list",
  use: {
    baseURL,
    browserName: "chromium",
    channel: process.platform === "win32" ? "msedge" : undefined,
    viewport: { width: 1220, height: 860 },
    screenshot: "only-on-failure",
  },
  webServer: {
    command: `node node_modules/vite/bin/vite.js --host 127.0.0.1 --port ${port} --strictPort`,
    url: baseURL,
    reuseExistingServer: true,
  },
});
