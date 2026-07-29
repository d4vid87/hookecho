//! The live-station layer: markers on the map, and the floating cards they open.
//!
//! Desktop only, by design — the cards carry camera video, which means an ffmpeg child process for
//! HLS and a decoder thread per open card. That is not a phone's job, and Android cannot spawn a
//! downloaded binary at all.
//!
//! The layer keeps the state the map needs (which stations are in view, which cameras are near
//! them, the current electric-field table) and owns the open cards, including their video streams:
//! closing a card drops its [`crate::cam::Stream`], which stops the download and kills any ffmpeg.

use crate::ui::station_card::{Card, EFieldKind};
use egui::ColorImage;
use std::sync::mpsc::{Receiver, Sender};
use wxdata::dotcams::DotCam;
use wxdata::efield::Ppef;
use wxdata::stations::StationOb;

/// The still-image fetch channel: decoded images in, keyed by their URL.
type StillChannel = (Sender<(String, ColorImage)>, Receiver<(String, ColorImage)>);

/// A camera this close to a station is looking at that station's weather. Beyond it the pairing
/// is a guess, and the card would rather show no camera than the wrong sky.
const CAM_RADIUS_KM: f64 = 25.0;

/// Layer state.
#[derive(Default)]
pub struct Layer {
    /// Stations in (or near) the current view.
    pub obs: Vec<StationOb>,
    /// Highway cameras in the same box, for pairing with a card.
    pub cams: Vec<DotCam>,
    /// The current PPEF table, when the ionospheric source is in use.
    pub ppef: Option<Ppef>,
    /// The latest field-mill reading, when the user has configured a mill URL.
    pub mill_kv_per_m: Option<f32>,
    /// Open cards, oldest first.
    pub cards: Vec<Card>,
    /// Still images for cameras that post frames instead of streaming.
    stills: Option<StillChannel>,
}

impl Layer {
    /// Fold a fresh poll into the layer and every open card.
    pub fn ingest(&mut self, obs: Vec<StationOb>) {
        // Resolve the field values first: they read the whole layer, and the cards below hold it
        // mutably.
        let fields: Vec<Option<f32>> = self
            .cards
            .iter()
            .map(|c| self.efield_for(c.ob.lon))
            .collect();
        for (card, e) in self.cards.iter_mut().zip(fields) {
            if let Some(fresh) = obs.iter().find(|o| o.key() == card.key) {
                card.push(fresh.clone(), e);
            }
        }
        self.obs = obs;
    }

    /// The electric-field value a card at `lon` should chart, in the unit [`Self::efield_kind`]
    /// names. A configured field mill wins: it is the local, physical measurement.
    pub fn efield_for(&self, lon: f64) -> Option<f32> {
        if let Some(kv) = self.mill_kv_per_m {
            return Some(kv);
        }
        self.ppef.as_ref()?.latest_at(lon).map(|(_, v)| v)
    }

    pub fn efield_kind(&self) -> EFieldKind {
        if self.mill_kv_per_m.is_some() {
            EFieldKind::Mill
        } else {
            EFieldKind::Ppef
        }
    }

    /// The station nearest a clicked point, within `radius_px` on screen. `to_screen` projects a
    /// lon/lat the way the pane does.
    pub fn hit(
        &self,
        pos: egui::Pos2,
        radius2: f32,
        to_screen: impl Fn(f64, f64) -> egui::Pos2,
    ) -> Option<StationOb> {
        self.obs
            .iter()
            .map(|o| (o, to_screen(o.lon, o.lat).distance_sq(pos)))
            .filter(|(_, d2)| *d2 <= radius2)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(o, _)| o.clone())
    }

    /// Open a card for a station, pairing it with the nearest streaming camera and starting the
    /// video. Opening the same station twice just focuses the card that is already up.
    pub fn open_card(
        &mut self,
        ob: StationOb,
        rt: &tokio::runtime::Handle,
        http: &reqwest::Client,
        ctx: &egui::Context,
    ) {
        let key = ob.key();
        if self.cards.iter().any(|c| c.key == key) {
            ctx.move_to_top(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new(("station-card", &key)),
            ));
            return;
        }
        let e = self.efield_for(ob.lon);
        let mut card = Card::new(ob.clone(), self.efield_kind());
        card.push(ob.clone(), e);

        self.cards.push(card);
        self.pair_cameras(rt, http, ctx);
    }

    /// Give every camera-less card its nearest camera. Called when a card opens and again each
    /// time the catalog lands — the district files are megabytes and often arrive after the first
    /// click, and a card opened in that window would otherwise stay blank until reopened.
    pub fn pair_cameras(
        &mut self,
        rt: &tokio::runtime::Handle,
        http: &reqwest::Client,
        ctx: &egui::Context,
    ) {
        if self.cams.is_empty() {
            return;
        }
        let mut stills = Vec::new();
        for card in self.cards.iter_mut() {
            #[cfg(not(target_os = "android"))]
            if card.cam.is_some() {
                continue;
            }
            if card.still_key.is_some() {
                continue;
            }
            let Some(cam) = nearest_cam(&self.cams, card.ob.lat, card.ob.lon) else {
                continue;
            };
            card.cam_owner = cam.agency.clone();
            card.cam_name = if cam.place.is_empty() {
                cam.name.clone()
            } else {
                format!("{} · {}", cam.name, cam.place)
            };
            #[cfg(not(target_os = "android"))]
            if let Some(url) = &cam.stream_url {
                card.cam = Some(crate::cam::Stream::start(url.clone(), rt));
            }
            // No stream (and on a phone, never a stream): fall back to the newest still.
            let streaming = cfg!(not(target_os = "android")) && cam.stream_url.is_some();
            if !streaming {
                if let Some(url) = cam.image_url.clone() {
                    card.still_key = Some(url.clone());
                    stills.push(url);
                }
            }
        }
        for url in stills {
            self.fetch_still(url, http, rt, ctx);
        }
    }

    /// Pull a still image for a camera that doesn't stream.
    fn fetch_still(
        &mut self,
        url: String,
        http: &reqwest::Client,
        rt: &tokio::runtime::Handle,
        ctx: &egui::Context,
    ) {
        let (tx, _) = self.stills.get_or_insert_with(std::sync::mpsc::channel);
        let (tx, http, ctx) = (tx.clone(), http.clone(), ctx.clone());
        rt.spawn(async move {
            match fetch_image(&http, &url).await {
                Ok(img) => {
                    let _ = tx.send((url, img));
                    ctx.request_repaint();
                }
                Err(e) => log::warn!("camera still {url}: {e}"),
            }
        });
    }

    /// Re-pull the still for every card showing one. A still-only camera posts a new frame every
    /// minute or so; without this the picture on the card is whatever it was when it opened.
    pub fn refresh_stills(
        &mut self,
        rt: &tokio::runtime::Handle,
        http: &reqwest::Client,
        ctx: &egui::Context,
    ) {
        let urls: Vec<String> = self
            .cards
            .iter()
            .filter_map(|c| c.still_key.clone())
            .collect();
        for url in urls {
            self.fetch_still(url, http, rt, ctx);
        }
    }

    /// Draw every open card, upload any new video frames, and drop the ones the user closed.
    /// Returns true while at least one card is showing live video, so the caller can keep the
    /// frame loop awake.
    pub fn show_cards(&mut self, ctx: &egui::Context, tz: Option<wxdata::tz::Tz>) -> bool {
        // Land any still images that arrived since the last frame.
        if let Some((_, rx)) = &self.stills {
            let landed: Vec<_> = rx.try_iter().collect();
            for (url, img) in landed {
                let tex = ctx.load_texture(
                    format!("station-still:{url}"),
                    img,
                    egui::TextureOptions::LINEAR,
                );
                for card in self.cards.iter_mut() {
                    if card.still_key.as_deref() == Some(url.as_str()) {
                        card.tex = Some(tex.clone());
                    }
                }
            }
        }

        // Only a video card animates, and only desktop has video.
        #[allow(unused_mut)]
        let mut animating = false;
        for (i, card) in self.cards.iter_mut().enumerate() {
            #[cfg(not(target_os = "android"))]
            if let Some(stream) = &card.cam {
                let seq = stream.seq();
                if seq != card.last_seq {
                    if let Some(img) = stream.take_frame() {
                        card.last_seq = seq;
                        // One texture per card, replaced in place: a 640x360 stream at 25 fps must
                        // not mint a new texture name every frame.
                        match &mut card.tex {
                            Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                            None => {
                                card.tex = Some(ctx.load_texture(
                                    format!("station-cam:{}", card.key),
                                    img,
                                    egui::TextureOptions::LINEAR,
                                ))
                            }
                        }
                    }
                }
                animating = true;
            }
            crate::ui::station_card::show(ctx, card, tz, i);
        }
        // Dropping a closed card stops its download and kills its ffmpeg (see `cam::Stream`).
        self.cards.retain(|c| c.open);
        animating
    }
}

/// Nearest camera to a point within [`CAM_RADIUS_KM`], preferring one that streams video.
fn nearest_cam(cams: &[DotCam], lat: f64, lon: f64) -> Option<&DotCam> {
    let near = |c: &DotCam| {
        let d = km_between(lat, lon, c.lat, c.lon);
        (d <= CAM_RADIUS_KM).then_some(d)
    };
    let pick = |streaming: bool| {
        cams.iter()
            .filter(|c| c.stream_url.is_some() == streaming)
            .filter_map(|c| near(c).map(|d| (c, d)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(c, _)| c)
    };
    pick(true).or_else(|| pick(false))
}

/// Great-circle distance in km.
fn km_between(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let (dp, dl) = ((lat2 - lat1).to_radians(), (lon2 - lon1).to_radians());
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    6371.0 * 2.0 * a.sqrt().asin()
}

async fn fetch_image(http: &reqwest::Client, url: &str) -> anyhow::Result<ColorImage> {
    let bytes = http
        .get(url)
        .header("User-Agent", crate::tiles::USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let img = image::load_from_memory(&bytes)?.to_rgba8();
    let (w, h) = img.dimensions();
    Ok(ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        img.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wxdata::stations::Network;

    fn cam(name: &str, lat: f64, lon: f64, stream: bool) -> DotCam {
        DotCam {
            id: name.into(),
            agency: "Caltrans D3".into(),
            name: name.into(),
            place: "Sacramento".into(),
            lat,
            lon,
            stream_url: stream.then(|| "https://x/live.m3u8".to_string()),
            image_url: Some("https://x/still.jpg".into()),
        }
    }

    fn station(lat: f64, lon: f64) -> StationOb {
        StationOb {
            id: "KSAC".into(),
            name: "Sacramento Executive".into(),
            network: Network::Metar,
            lat,
            lon,
            time: None,
            temp_c: Some(30.0),
            dewp_c: Some(12.0),
            rh_pct: None,
            wdir_deg: None,
            wspd_kt: None,
            gust_kt: None,
            pressure_mb: None,
            precip_rate_mmh: None,
            elev_m: None,
        }
    }

    #[test]
    fn a_streaming_camera_wins_over_a_closer_still_one() {
        let cams = vec![
            cam("still, 1 km", 38.5, -121.5, false),
            cam("stream, 8 km", 38.57, -121.5, true),
            cam("stream, 200 km", 40.3, -121.5, true),
        ];
        let picked = nearest_cam(&cams, 38.49, -121.5).unwrap();
        assert_eq!(picked.name, "stream, 8 km");
        // Nothing within range at all: no camera rather than a wrong one.
        assert!(nearest_cam(&cams, 34.0, -118.0).is_none());
    }

    #[test]
    fn a_still_only_area_still_gets_a_camera() {
        let cams = vec![cam("still", 38.5, -121.5, false)];
        assert_eq!(nearest_cam(&cams, 38.49, -121.5).unwrap().name, "still");
    }

    #[test]
    fn ingest_feeds_open_cards_and_leaves_others_alone() {
        let mut layer = Layer::default();
        layer.cards.push(Card::new(station(38.5, -121.5), EFieldKind::Ppef));
        let mut fresh = station(38.5, -121.5);
        fresh.temp_c = Some(31.0);
        fresh.time = Some(chrono::Utc::now());
        layer.ingest(vec![fresh]);
        assert_eq!(layer.cards[0].history.len(), 1);
        assert_eq!(layer.cards[0].ob.temp_c, Some(31.0));
        // A poll that no longer includes the station leaves its card untouched, not blank.
        layer.ingest(Vec::new());
        assert_eq!(layer.cards[0].ob.temp_c, Some(31.0));
    }

    #[test]
    fn a_field_mill_outranks_the_ionospheric_model() {
        let mut layer = Layer::default();
        assert_eq!(layer.efield_kind(), EFieldKind::Ppef);
        assert_eq!(layer.efield_for(-121.5), None);
        layer.mill_kv_per_m = Some(4.2);
        assert_eq!(layer.efield_kind(), EFieldKind::Mill);
        assert_eq!(layer.efield_for(-121.5), Some(4.2));
    }

    #[test]
    fn distance_is_in_kilometres() {
        // A degree of latitude is ~111 km.
        let d = km_between(38.0, -121.0, 39.0, -121.0);
        assert!((110.0..112.0).contains(&d), "got {d}");
    }
}
