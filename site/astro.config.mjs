// @ts-check
import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";

// Static output, deployed to the Cloudflare Pages project `hookecho-site`. The wasm app itself
// is a separate deploy (project `hookecho`) and lives at app.hookecho.io — nothing here bundles it.
export default defineConfig({
  site: "https://hookecho.io",
  // Hover-prefetch every internal link. The pages are a few kB of static HTML, so the next one is
  // already there by the time the click lands; lists opt into viewport prefetching by hand.
  prefetch: true,
  integrations: [sitemap()],
});
