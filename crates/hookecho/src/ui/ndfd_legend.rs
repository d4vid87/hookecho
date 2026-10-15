//! NOAA's NDFD WMS omits GetLegendGraphic. Its public map viewer exposes the matching
//! product scale through `wxmap_legendImage.php`; fetch that small image once per product.

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use chrono::{DateTime, Utc};
use egui::{Color32, ColorImage, Rect, TextureHandle, TextureOptions};

use crate::tiles::BasemapStyle;
use wxdata::clock::Instant;

const LEGEND_W: usize = 600;
const LEGEND_H: usize = 30;

#[derive(Default)]
struct Entry {
    tex: Option<TextureHandle>,
    rx: Option<Receiver<Result<ColorImage, String>>>,
    requested: Option<Instant>,
}

pub struct NdfdLegends {
    entries: [Entry; 12],
}

impl Default for NdfdLegends {
    fn default() -> Self { Self { entries: std::array::from_fn(|_| Entry::default()) } }
}

fn slot(style: BasemapStyle) -> Option<(usize, &'static str, &'static str)> {
    match style {
        BasemapStyle::NdfdRainChance => Some((0, "pop12", "Rain chance · %")),
        BasemapStyle::NdfdMaxTemp => Some((1, "maxt", "Daytime high · °F")),
        BasemapStyle::NdfdMinTemp => Some((2, "mint", "Overnight low · °F")),
        BasemapStyle::NdfdAlaskaRainChance => Some((3, "pop12", "Alaska rain chance · %")),
        BasemapStyle::NdfdAlaskaMaxTemp => Some((4, "maxt", "Alaska daytime high · °F")),
        BasemapStyle::NdfdAlaskaMinTemp => Some((5, "mint", "Alaska overnight low · °F")),
        BasemapStyle::NdfdHawaiiRainChance => Some((6, "pop12", "Hawaii rain chance · %")),
        BasemapStyle::NdfdHawaiiMaxTemp => Some((7, "maxt", "Hawaii daytime high · °F")),
        BasemapStyle::NdfdHawaiiMinTemp => Some((8, "mint", "Hawaii overnight low · °F")),
        BasemapStyle::NdfdWindGust => Some((9, "windgust", "Wind gust · mph")),
        BasemapStyle::NdfdAlaskaWindGust => Some((10, "windgust", "Alaska wind gust · mph")),
        BasemapStyle::NdfdHawaiiWindGust => Some((11, "windgust", "Hawaii wind gust · mph")),

        _ => None,
    }
}

fn url(region: &str, element: &str, valid: DateTime<Utc>) -> String {
    let bounds = match region {
        "alaska" => "left=-19000000&bottom=6400000&right=-14000000&top=11800000",
        "hawaii" => "left=-18000000&bottom=2000000&right=-17000000&top=3000000",
        _ => "left=-13000000&bottom=3000000&right=-7500000&top=6200000",
    };
    format!("https://digital.weather.gov/scripts/wxmap_legendImage.php?dataset=ndfd&element={element}\
&region={region}&vt={}&opacity=0.8&{bounds}&width={LEGEND_W}&ms=english", valid.format("%Y-%m-%dT%H:%M"))
}

impl NdfdLegends {
    pub fn update(
        &mut self, ctx: &egui::Context, spawner: &crate::rt::Spawner,
        http: &reqwest::Client, style: BasemapStyle, valid: DateTime<Utc>,
    ) {
        let Some((i, element, _)) = slot(style) else { return };
        let entry = &mut self.entries[i];
        if let Some(rx) = &entry.rx {
            match rx.try_recv() {
                Ok(Ok(image)) => {
                    entry.tex = Some(ctx.load_texture(format!("ndfd-legend-{i}"), image, TextureOptions::LINEAR));
                    entry.rx = None;
                    ctx.request_repaint();
                }
                Ok(Err(error)) => {
                    log::warn!("NDFD {element} legend unavailable: {error}");
                    entry.rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => entry.rx = None,
                Err(mpsc::TryRecvError::Empty) if entry.requested.is_some_and(|t| t.elapsed() > Duration::from_secs(30)) => entry.rx = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if entry.tex.is_some() || entry.rx.is_some()
            || entry.requested.is_some_and(|t| t.elapsed() < Duration::from_secs(120)) { return; }

        let (tx, rx) = mpsc::channel();
        entry.rx = Some(rx);
        entry.requested = Some(Instant::now());
        let http = http.clone();
        let ctx = ctx.clone();
        let source = url(style.ndfd_region().unwrap(), element, valid);
        spawner.spawn(async move {
            let result = async {
                let response = http.get(wxdata::net::fetch_url(&source))
                    .timeout(Duration::from_secs(15)).send().await
                    .map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?;
                use futures_util::StreamExt;
                let mut stream = response.bytes_stream();
                let mut bytes = Vec::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|e| e.to_string())?;
                    if bytes.len() + chunk.len() > 64 * 1024 { return Err("legend exceeds 64 KiB".into()); }
                    bytes.extend_from_slice(&chunk);
                }
                let image = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgba8();
                if image.width() as usize != LEGEND_W || image.height() as usize != LEGEND_H {
                    return Err("unexpected legend dimensions".into());
                }
                Ok(ColorImage::from_rgba_unmultiplied([LEGEND_W, LEGEND_H], image.as_raw()))
            }.await;
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    /// Returns the height occupied so other map legends can stack beneath it.
    pub fn draw(&self, painter: &egui::Painter, pane: Rect, style: BasemapStyle, y: f32) -> f32 {
        let Some((i, _, label)) = slot(style) else { return 0.0 };
        let width = (pane.width() - 24.0).min(410.0);
        if width < 120.0 { return 0.0; }
        let height = 58.0;
        let panel = Rect::from_min_size(pane.left_top() + egui::vec2(12.0, y), egui::vec2(width, height));
        painter.rect_filled(panel, 7.0, Color32::from_rgb(16, 40, 61));
        painter.rect_stroke(panel, 7.0, egui::Stroke::new(1.0, Color32::from_rgb(66, 98, 122)), egui::StrokeKind::Inside);
        painter.text(panel.left_top() + egui::vec2(8.0, 6.0), egui::Align2::LEFT_TOP,
            format!("NDFD {label} · NOAA/NWS"), egui::FontId::proportional(11.0), Color32::WHITE);
        if let Some(tex) = &self.entries[i].tex {
            painter.image(tex.id(), Rect::from_min_size(panel.left_top() + egui::vec2(8.0, 22.0),
                egui::vec2(width - 16.0, 27.0)), Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            painter.text(panel.left_top() + egui::vec2(8.0, 28.0), egui::Align2::LEFT_TOP,
                if self.entries[i].rx.is_some() { "Loading NOAA scale…" } else { "NOAA scale unavailable" },
                egui::FontId::proportional(11.0), Color32::LIGHT_GRAY);
        }
        height + 8.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legend_urls_use_selected_valid_time_and_product() {
        let valid = DateTime::parse_from_rfc3339("2026-10-16T00:00:00Z").unwrap().with_timezone(&Utc);
        assert!(url("conus", "maxt", valid).contains("element=maxt&region=conus&vt=2026-10-16T00:00"));
        assert!(url("alaska", "maxt", valid).contains("region=alaska"));
        assert!(url("hawaii", "pop12", valid).contains("region=hawaii"));
        assert_eq!(slot(BasemapStyle::NdfdRainChance).unwrap().1, "pop12");
        assert_eq!(slot(BasemapStyle::NdfdMinTemp).unwrap().1, "mint");
        assert_eq!(slot(BasemapStyle::NdfdAlaskaMaxTemp).unwrap().0, 4);
        assert_eq!(slot(BasemapStyle::NdfdHawaiiRainChance).unwrap().0, 6);
        for style in [BasemapStyle::NdfdWindGust, BasemapStyle::NdfdAlaskaWindGust,
            BasemapStyle::NdfdHawaiiWindGust] {
            assert_eq!(slot(style).unwrap().1, "windgust");
            assert!(url(style.ndfd_region().unwrap(), "windgust", valid).contains("element=windgust"));
        }
        assert!(slot(BasemapStyle::Dark).is_none());
    }
}
