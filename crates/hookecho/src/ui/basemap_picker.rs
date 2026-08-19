//! The basemap chooser: a thumbnail grid, grouped by category.
//!
//! Replaces a fifty-one-entry combo box on desktop and an eight-chip row plus a disclosure on the
//! phone. The complaint that started this was "basemaps very lacking" — there were forty of them,
//! but a flat alphabetical-ish list of names is not a way to choose a *look*.
//!
//! Raster styles show one real tile (z6 over the middle of CONUS, fetched through the normal
//! per-style disk cache). Vector styles show a painted swatch of their own palette rather than a
//! rasterised MVT tile: the swatch is honest about the colors, which is what the choice is
//! actually between, and it needs no fetch at all.
//!
//! ponytail: painted swatches, not rendered MVT; one fixed thumbnail tile per style.

use crate::settings::Settings;
use crate::tiles::{BasemapStyle, Category, TileManager};

/// Card size. Wide enough for a 256-px tile to read at a glance without the grid needing a
/// scrollbar on a phone.
const CARD_W: f32 = 104.0;
const CARD_H: f32 = 72.0;

/// Paint the swatch a vector style gets instead of a fetched tile: its background with a band of
/// its own road and water colors, so the cards differ the way the maps do.
fn swatch(painter: &egui::Painter, rect: egui::Rect, style: BasemapStyle) {
    let Some(pal) = style.vector_palette() else {
        painter.rect_filled(rect, 4.0, egui::Color32::from_gray(40));
        return;
    };
    let c = |v: [u8; 4]| egui::Color32::from_rgb(v[0], v[1], v[2]);
    let vs = crate::basemap_style::style(pal);
    let bg = vs.background.map(c).unwrap_or(egui::Color32::from_gray(60));
    painter.rect_filled(rect, 4.0, bg);
    // A road, a casing under it, and a river: the three things the palettes differ most in.
    if let Some((road, w)) = crate::basemap_style::stroke(pal, "transportation", "motorway") {
        if let Some((case, cw)) = crate::basemap_style::casing(pal, "transportation", "motorway") {
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 6.0, rect.bottom() - 22.0),
                    egui::pos2(rect.right() - 6.0, rect.top() + 20.0),
                ],
                egui::Stroke::new(cw * 2.2, c(case)),
            );
        }
        painter.line_segment(
            [
                egui::pos2(rect.left() + 6.0, rect.bottom() - 22.0),
                egui::pos2(rect.right() - 6.0, rect.top() + 20.0),
            ],
            egui::Stroke::new(w * 2.2, c(road)),
        );
    }
    if let Some(water) = crate::basemap_style::fill(pal, "water", "") {
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.bottom() - 14.0),
                egui::vec2(rect.width(), 14.0),
            ),
            0.0,
            c(water),
        );
    }
}

/// The `Auto` card: half the Dark swatch, half the Light one, because that is what it does.
fn auto_swatch(painter: &egui::Painter, rect: egui::Rect) {
    let (l, r) = rect.split_left_right_at_fraction(0.5);
    swatch(painter, l, BasemapStyle::Dark);
    swatch(painter, r, BasemapStyle::Light);
}

/// One card. Returns its click response.
fn card(
    ui: &mut egui::Ui,
    tiles: &mut TileManager,
    style: BasemapStyle,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(CARD_W, CARD_H + 16.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let img_rect = egui::Rect::from_min_size(rect.min, egui::vec2(CARD_W, CARD_H));
    let painter = ui.painter_at(rect);
    // Only ask for a thumbnail once the card is on screen, so opening the picker does not fire
    // fifty fetches for cards nobody scrolled to.
    let thumb = if style.is_raster() {
        tiles.thumb(style, ui.ctx())
    } else {
        None
    };
    match thumb {
        Some(tex) => {
            egui::Image::new(&tex)
                .corner_radius(4.0)
                .paint_at(ui, img_rect);
        }
        None if style == BasemapStyle::Auto => auto_swatch(&painter, img_rect),
        None => swatch(&painter, img_rect, style),
    }
    if !enabled {
        painter.rect_filled(
            img_rect,
            4.0,
            egui::Color32::from_black_alpha(150),
        );
        painter.text(
            img_rect.center(),
            egui::Align2::CENTER_CENTER,
            "🔒",
            egui::FontId::proportional(18.0),
            egui::Color32::from_gray(210),
        );
    }
    let border = if selected {
        egui::Stroke::new(2.0, ui.visuals().selection.bg_fill)
    } else if response.hovered() {
        egui::Stroke::new(1.0, ui.visuals().widgets.hovered.bg_stroke.color)
    } else {
        egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color)
    };
    painter.rect_stroke(img_rect, 4.0, border, egui::StrokeKind::Inside);
    painter.text(
        egui::pos2(rect.center().x, img_rect.bottom() + 2.0),
        egui::Align2::CENTER_TOP,
        style.short_label(),
        egui::FontId::proportional(11.0),
        ui.visuals().text_color(),
    );
    response
}

/// The whole grid. Returns the style the user picked, if any.
///
/// Takes `Settings` rather than three loose flags because availability depends on two API keys
/// and the custom template, and passing those separately is how they get out of step.
pub fn grid(
    ui: &mut egui::Ui,
    tiles: &mut TileManager,
    current: BasemapStyle,
    settings: &Settings,
) -> Option<BasemapStyle> {
    let mb = !settings.mapbox_key.is_empty();
    let mt = !settings.maptiler_key.is_empty();
    let cx = crate::tiles::valid_xyz_template(&settings.custom_tile_url);
    let mut picked = None;
    for cat in Category::ALL {
        let styles: Vec<BasemapStyle> = BasemapStyle::ALL
            .into_iter()
            .filter(|s| s.category() == cat)
            // A style whose key is missing is still shown, locked — that is how anyone finds out
            // the key would unlock it. A custom slot with no template is not, because there is
            // nothing to say about it that the settings row does not already say.
            .filter(|s| *s != BasemapStyle::CustomXyz || cx)
            .collect();
        if styles.is_empty() {
            continue;
        }
        ui.label(egui::RichText::new(cat.label()).strong());
        ui.horizontal_wrapped(|ui| {
            for s in styles {
                let enabled = s.available(mb, mt, cx);
                let r = card(ui, tiles, s, s == current, enabled);
                if r.clicked() {
                    picked = Some(s);
                }
                if !enabled {
                    r.on_hover_text(match s.provider_kind() {
                        crate::tiles::Provider::Mapbox => "Needs a Mapbox access token (Settings)",
                        crate::tiles::Provider::MapTiler => "Needs a MapTiler API key (Settings)",
                        crate::tiles::Provider::Builtin => "Not available in this build",
                    });
                }
            }
        });
        ui.add_space(6.0);
    }
    picked
}
