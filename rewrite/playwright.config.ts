import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./src/tests",
  testIgnore: "**/typography-paint.spec.js", // Dedicated two-engine/site config.
  outputDir: "../artifacts/rewrite-20260910/frontend/test-results",
  workers: 1,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:1420",
    browserName: "chromium",
    channel: process.platform === "win32" ? "msedge" : undefined,
    viewport: { width: 1220, height: 860 },
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "npm run dev",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: true,
  },
});
