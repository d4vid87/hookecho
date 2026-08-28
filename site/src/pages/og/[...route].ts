import { getCollection } from "astro:content";
import { OGImageRoute } from "astro-og-canvas";
import sites from "../../data/nexrad-sites.json";

// Build-time social cards for the two page sets that are generated rather than written: every
// radar site and the storm archive. Everything else keeps the hand-made alltilts.jpg card, which is a
// screenshot of the actual product and beats anything drawn from text.
const storms = await getCollection("storms");

const pages = Object.fromEntries([
  ...sites.map((site) => [
    `radar/${site.id.toLowerCase()}`,
    {
      title: `${site.id} — ${site.city}, ${site.state}`,
      description:
        site.network === "tdwr"
          ? "Live terminal Doppler radar, every tilt of the volume, in HookEcho."
          : "Live NEXRAD radar, every tilt of the volume, in HookEcho.",
    },
  ]),
  ...storms.map((storm) => [
    `storms/${storm.id}`,
    { title: storm.data.title, description: `${storm.data.site} · replay the archived volumes in HookEcho.` },
  ]),
]);

// ponytail: named re-exports rather than `export const { ... } =` — Astro's route analysis only
// finds getStaticPaths as a plain named export, and the destructured form fails the build.
// OGImageRoute is async in 0.13 (it loads CanvasKit), so it has to be awaited before Astro can
// see the exports.
const route = await OGImageRoute({
  param: "route",
  pages,
  getImageOptions: (_path, page: { title: string; description: string }) => ({
    title: page.title,
    description: page.description,
    logo: { path: "./public/logo.png", size: [72] },
    bgGradient: [
      [17, 24, 21],
      [7, 11, 9],
    ],
    // The ramp's green end, matching --ramp-1 in global.css.
    border: { color: [48, 209, 88], width: 12, side: "inline-start" },
    padding: 60,
    font: {
      title: { size: 62, weight: "Bold", color: [228, 236, 230] },
      description: { size: 30, color: [144, 160, 150] },
    },
  }),
});

export const getStaticPaths = route.getStaticPaths;
export const GET = route.GET;
