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

// Posts are dated rather than ordered; the index and the feed both sort on `date`.
const blog = defineCollection({
  loader: glob({ pattern: "**/*.md", base: "./src/content/blog" }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    date: z.coerce.date(),
    /** Absolute-from-root path to the social card image, if the post has one of its own. */
    image: z.string().optional(),
  }),
});

export const collections = { docs, blog };
