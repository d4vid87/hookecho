# The default font faces, unbundled

These four faces are egui's own defaults, copied verbatim from `epaint_default_fonts` 0.35 (the
licenses beside them are that crate's, likewise verbatim: OFL for Ubuntu-Light and NotoEmoji, the
Hack license for Hack, MIT for emoji-icon-font).

The native and Android builds still get them the normal way, through eframe's `default_fonts`
feature. The browser build does not: compiled in, they were ~776 KB gzipped — a fifth of the whole
wasm — sitting on the critical path of a first visit for glyphs that are a fallback behind Inter.

So on the web they ship as four separate files, hashed into `/dist/` by `scripts/web/build.sh`
(which puts them under the year-long immutable `_headers` rule and in the service worker's shell),
fetched by `crates/hookecho/src/fonts.rs` right after boot, and hot-added with `set_fonts`. First
paint uses Inter + Phosphor, which cover the app's own text; anything that needs a fallback glyph
gets it a moment later, from a file the browser then has cached for a year.

Updating egui means re-copying these — if the faces drift, the web build's fallbacks drift with
them. `fonts.rs` mirrors epaint's family order and `FontTweak` scales; both live in
`epaint/src/text/fonts.rs`.
