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

// Historic storms: the frontmatter is the archive deep link, spelled out in fields rather than a
// pasted URL, so the app's #goto vocabulary lives in one place (storms/[...slug].astro).
const storms = defineCollection({
  loader: glob({ pattern: "**/*.md", base: "./src/content/storms" }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    /** When the storm happened — used for sorting and for the page's dateline. */
    date: z.coerce.date(),
    /** NEXRAD site that watched it, e.g. "KTLX". Must have a page under /radar/. */
    site: z.string(),
    lon: z.number(),
    lat: z.number(),
    zoom: z.number(),
    /** RFC3339 volume time the archive opens on. */
    at: z.string(),
    /** Extra #goto tokens: a moment code (VEL, CC…), a tilt number, `srv`. */
    extras: z.array(z.string()).default([]),
    image: z.string().optional(),
  }),
});

// One page per term. The definitions used to live as bold lines inside the FAQ, where they were
// unlinkable and were also being scraped into the page's FAQPage JSON-LD as if they were questions.
const glossary = defineCollection({
  loader: glob({ pattern: "**/*.md", base: "./src/content/glossary" }),
  schema: z.object({
    /** Display name, e.g. "CC (correlation coefficient)". The file name is the URL. */
    term: z.string(),
    description: z.string(),
    /** Slugs of other terms worth reading next. */
    related: z.array(z.string()).default([]),
  }),
});

export const collections = { docs, blog, storms, glossary };
