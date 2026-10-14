import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./src/tests",
  testMatch: ["onboarding-profiles.spec.js", "onboarding-fit.spec.js"],
  outputDir: "../artifacts/onboarding/test-results",
  workers: 1,
  timeout: 30000,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:1427",
    browserName: "chromium",
    channel: process.platform === "win32" ? "msedge" : undefined,
    viewport: { width: 1220, height: 860 },
  },
  webServer: {
    command:
      "node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 1427 --strictPort",
    url: "http://127.0.0.1:1427",
    reuseExistingServer: false,
  },
});
