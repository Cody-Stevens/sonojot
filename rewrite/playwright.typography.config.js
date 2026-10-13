import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./src/tests",
  testMatch: "typography-paint.spec.js",
  outputDir: "../artifacts/typography-safari/test-results",
  workers: 1,
  reporter: "list",
  projects: [
    { name: "webkit", use: { browserName: "webkit" } },
    { name: "chromium", use: { browserName: "chromium", channel: process.platform === "win32" ? "msedge" : undefined } },
  ],
  use: { reducedMotion: "reduce" },
  webServer: [
    {
      command: "npm run dev",
      url: "http://127.0.0.1:1420",
      reuseExistingServer: true,
    },
    {
      command:
        "node node_modules/astro/astro.js dev --host 127.0.0.1 --port 4321",
      cwd: "../web",
      url: "http://127.0.0.1:4321",
      reuseExistingServer: true,
    },
  ],
});
