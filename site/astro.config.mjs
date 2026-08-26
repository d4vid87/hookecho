// @ts-check
import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";

// Static output, deployed to the Cloudflare Pages project `hookecho-site`. The wasm app itself
// is a separate deploy (project `hookecho`) and lives at app.hookecho.io — nothing here bundles it.
export default defineConfig({
  site: "https://hookecho.io",
  integrations: [sitemap()],
});
