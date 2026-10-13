import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";
import { site } from "./src/site.config.mjs";
import { fileURLToPath } from "node:url";

// Static output. Cloudflare serves ./dist directly; no server adapter is used.
export default defineConfig({
  site: site.origin,
  output: "static",
  trailingSlash: "never",
  integrations: [sitemap()],
  build: {
    inlineStylesheets: "always",
  },
  devToolbar: { enabled: false },
  vite: {
    server: {
      fs: {
        allow: [
          fileURLToPath(new URL(".", import.meta.url)),
          fileURLToPath(new URL("../shared", import.meta.url)),
          // The isolated relief proof reuses the app's exact bundled font.
          // Allow this public asset only, not the rest of the app directory.
          fileURLToPath(
            new URL(
              "../rewrite/src/brand/fonts/SourceSerif4Variable-Roman.ttf.woff2",
              import.meta.url,
            ),
          ),
        ],
      },
    },
  },
});
