import { defineConfig } from "@playwright/test";
import base from "../../../playwright.config";

export default defineConfig({
  ...base,
  testDir: "..",
  use: { ...base.use, baseURL: "http://127.0.0.1:1432" },
  webServer: {
    command:
      "node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 1432 --strictPort",
    cwd: "../../..",
    url: "http://127.0.0.1:1432",
    reuseExistingServer: true,
  },
});
