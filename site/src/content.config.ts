import { defineCollection, z } from "astro:content";
import { glob } from "astro/loaders";

// `order` drives both the sidebar and prev/next — one number per page, no nested nav.
// ponytail: flat list; add sections when the page count outgrows a single column.
const docs = defineCollection({
  loader: glob({ pattern: "**/*.md", base: "./src/content/docs" }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    order: z.number(),
  }),
});

export const collections = { docs };
