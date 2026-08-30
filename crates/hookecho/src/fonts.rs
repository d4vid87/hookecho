//! The app's font set, and the browser build's lazy half of it.
//!
//! Native and Android get egui's default faces the normal way, through eframe's `default_fonts`
//! feature. The web build does not: those four faces were ~776 KB gzipped, a fifth of the whole
//! wasm, on the critical path of a first visit — for glyphs that only ever appear as a fallback
//! behind Inter. On the web they are four files instead (see `web/fonts/README.md`), fetched
//! right after boot and hot-added with `set_fonts`.
//!
//! The end state is byte-identical to native: [`full`] rebuilds exactly what epaint's
//! `FontDefinitions::default()` builds — same faces, same family order, same `FontTweak` scales —
//! and then applies the app's own two faces on top the same way [`base`] does. What differs is
//! only *when* the fallbacks arrive.

#[cfg(target_arch = "wasm32")]
use egui::FontTweak;
use egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

/// Inter, subsetted (SIL OFL, see `data/fonts/Inter-LICENSE.txt`) — ~26 KB gzipped, and the face
/// essentially every glyph the app draws comes from.
const INTER: &[u8] = include_bytes!("../data/fonts/Inter-Regular-subset.ttf");

/// The font set to start with.
///
/// Native: egui's defaults, plus Phosphor's icon glyphs (the mobile chrome draws line icons egui's
/// default face has none of) and Inter in front as the proportional face.
///
/// Web: the same two faces and nothing behind them, because `default_fonts` is off there. Not an
/// empty family in sight — Inter covers Latin, punctuation and symbols, so first paint looks the
/// same; what is missing until [`full`] lands is the fallback for glyphs Inter's subset dropped.
pub fn base() -> FontDefinitions {
    decorate(FontDefinitions::default())
}

/// Add the app's own faces to `fonts`, whatever it already holds.
///
/// Order matters and is the reason this is one function rather than two call sites: Phosphor
/// inserts itself at index 1 of Proportional, Inter goes in front of everything at 0. Run against
/// egui's defaults that yields `[Inter, Ubuntu-Light, phosphor, …]`, which is what native has had
/// all along — so the web build reaching the same list is a matter of feeding this the same input.
fn decorate(mut fonts: FontDefinitions) -> FontDefinitions {
    fonts
        .font_data
        .insert("Inter".to_owned(), Arc::new(FontData::from_static(INTER)));
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "Inter".to_owned());
    // `add_to_fonts` does `insert(1, …)`, which panics on a list shorter than that — on the web,
    // before Inter goes in, the list is empty. Ordering the two this way is not cosmetic.
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // Monospace is empty on the web until the fetched Hack arrives; Inter is a readable stand-in
    // and an empty family is a panic in egui, not a fallback.
    let mono = fonts.families.entry(FontFamily::Monospace).or_default();
    if mono.is_empty() {
        mono.push("Inter".to_owned());
    }
    fonts
}

/// The web build's four fetched faces, in the order [`FILES`] names them.
#[cfg(target_arch = "wasm32")]
pub type Fetched = [Vec<u8>; 4];

/// The files the browser fetches, in the order [`full`] expects them. Names match `web/fonts/`.
#[cfg(target_arch = "wasm32")]
pub const FILES: [&str; 4] = [
    "Hack-Regular",
    "NotoEmoji-Regular",
    "Ubuntu-Light",
    "emoji-icon-font",
];

/// The complete font set, built from faces fetched at runtime.
///
/// This is epaint's `FontDefinitions::default()`, restated: the same four faces under the same
/// names, the same two `FontTweak` scales (0.81 and 0.90 — the emoji faces are drawn smaller than
/// their metrics ask for), the same family order. Restated rather than reused because the whole
/// point is that the web build does not compile those faces in, and `default()` without them is
/// empty. Mirror any change epaint makes here; `web/fonts/README.md` says so too.
#[cfg(target_arch = "wasm32")]
pub fn full(fetched: Fetched) -> FontDefinitions {
    let [hack, noto, ubuntu, emoji] = fetched;
    let mut fonts = FontDefinitions::empty();
    let tweaked = |bytes: Vec<u8>, scale: f32| {
        Arc::new(FontData::from_owned(bytes).tweak(FontTweak {
            scale,
            ..Default::default()
        }))
    };
    fonts
        .font_data
        .insert("Hack".to_owned(), Arc::new(FontData::from_owned(hack)));
    fonts
        .font_data
        .insert("NotoEmoji-Regular".to_owned(), tweaked(noto, 0.81));
    fonts.font_data.insert(
        "Ubuntu-Light".to_owned(),
        Arc::new(FontData::from_owned(ubuntu)),
    );
    fonts
        .font_data
        .insert("emoji-icon-font".to_owned(), tweaked(emoji, 0.90));
    fonts.families.insert(
        FontFamily::Monospace,
        vec![
            "Hack".to_owned(),
            "Ubuntu-Light".to_owned(), // fallback for √ etc
            "NotoEmoji-Regular".to_owned(),
            "emoji-icon-font".to_owned(),
        ],
    );
    fonts.families.insert(
        FontFamily::Proportional,
        vec![
            "Ubuntu-Light".to_owned(),
            "NotoEmoji-Regular".to_owned(),
            "emoji-icon-font".to_owned(),
        ],
    );
    decorate(fonts)
}

/// Fetch the four faces and install them, then repaint. Failure is not fatal and never leaves a
/// broken UI: the boot fonts stay, the app keeps working without its fallback glyphs, and the
/// reason goes to the console.
///
/// The URLs are hashed at build time and published by the page as `globalThis.__fontUrls` — the
/// same shape the rest of the shell uses to hand hashed asset names to Rust.
///
// ponytail: three tries with a flat backoff and no persistence beyond the browser's own HTTP
// cache, which is where a year-long immutable response belongs anyway.
#[cfg(target_arch = "wasm32")]
pub fn spawn_load(ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        for attempt in 0..3u32 {
            match load().await {
                Ok(fetched) => {
                    ctx.set_fonts(full(fetched));
                    ctx.request_repaint();
                    log::info!("fonts: fallback faces installed");
                    return;
                }
                Err(e) => {
                    log::warn!("fonts: fetch failed ({e}), attempt {}", attempt + 1);
                    sleep_ms(2000 * (attempt + 1)).await;
                }
            }
        }
        log::error!("fonts: giving up — running on the boot faces, fallback glyphs unavailable");
    });
}

/// `setTimeout` as a future. The browser build has no runtime timer of its own — see `rt.rs`.
#[cfg(target_arch = "wasm32")]
async fn sleep_ms(ms: u32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

#[cfg(target_arch = "wasm32")]
async fn load() -> Result<Fetched, String> {
    let urls = font_urls().ok_or("page published no __fontUrls")?;
    let mut out: Vec<Vec<u8>> = Vec::with_capacity(FILES.len());
    for url in &urls {
        let resp = reqwest::get(url).await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("{url}: HTTP {}", resp.status()));
        }
        out.push(resp.bytes().await.map_err(|e| e.to_string())?.to_vec());
    }
    let n = out.len();
    out.try_into()
        .map_err(|_| format!("expected {} faces, got {n}", FILES.len()))
}

/// The four URLs the page published, in [`FILES`] order.
#[cfg(target_arch = "wasm32")]
fn font_urls() -> Option<Vec<String>> {
    let obj = js_sys::Reflect::get(&js_sys::global(), &"__fontUrls".into()).ok()?;
    if obj.is_undefined() || obj.is_null() {
        return None;
    }
    // The page publishes site-absolute paths ("/dist/font-…"); reqwest wants a whole URL, and a
    // relative one is a "builder error" it reports long after the useful stack has gone.
    let origin = web_sys::window()?.location().origin().ok()?;
    FILES
        .iter()
        .map(|name| {
            let path = js_sys::Reflect::get(&obj, &(*name).into())
                .ok()
                .and_then(|v| v.as_string())?;
            Some(if path.starts_with("http") {
                path
            } else {
                format!("{origin}{path}")
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    /// The two faces the app supplies itself must be in front on every platform, and no family
    /// may be empty — egui panics on an empty family, and on the web these lists start bare.
    #[test]
    fn base_puts_inter_first_and_leaves_no_family_empty() {
        let fonts = super::base();
        for (family, list) in &fonts.families {
            assert!(!list.is_empty(), "{family:?} is empty");
        }
        let prop = &fonts.families[&egui::FontFamily::Proportional];
        assert_eq!(prop[0], "Inter");
        assert!(prop.contains(&"phosphor".to_owned()));
    }
}
