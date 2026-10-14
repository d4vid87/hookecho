//! Pane-local display choices. These never modify a decoded sweep or native probe.

use crate::colormap::{ColorTable, Palettes};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use wxdata::level2::Moment;

fn default_opacity() -> f32 { 1.0 }

fn default_blend_width() -> f32 {
    0.55
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RadarAppearance {
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default = "default_blend_width")]
    pub blend_width: f32,
    #[serde(default)]
    pub precision: bool,
    #[serde(default)]
    pub multipass: bool,
    #[serde(default)]
    pub fade_old_sectors: bool,
    #[serde(default)]
    pub upper_bounds: [Option<f32>; Moment::ALL.len()],
    /// Embedded tables keep saved panes portable when imported palettes move.
    #[serde(default, with = "portable_palettes")]
    pub palettes: BTreeMap<String, ColorTable>,
}

/// A portable display preset; station, product and playhead remain pane-local.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AppearancePreset {
    pub appearance: RadarAppearance,
    pub smooth: bool,
    pub globe: bool,
    pub thresholds: [Option<f32>; Moment::ALL.len()],
    pub threshold_enabled: [bool; Moment::ALL.len()],
}

impl AppearancePreset {
    pub fn capture(view: &crate::view::MapView) -> Self {
        Self { appearance: view.appearance.clone(), smooth: view.smooth,
            globe: view.camera.globe, thresholds: view.thresholds,
            threshold_enabled: view.threshold_enabled }
    }

    pub fn apply(&self, view: &mut crate::view::MapView) {
        view.appearance = self.appearance.clone();
        view.appearance.sanitize();
        view.smooth = self.smooth;
        view.camera.set_globe(self.globe);
        for (i, moment) in Moment::ALL.iter().enumerate() {
            let (lo, hi) = moment.value_range();
            view.thresholds[i] = self.thresholds[i].filter(|v| v.is_finite()).map(|v| v.clamp(lo, hi));
            view.threshold_enabled[i] = self.threshold_enabled[i] && view.thresholds[i].is_some();
        }
    }
}

// Reuse the app's existing portable palette format instead of a second JSON
// schema for color stops. This also keeps serde code off the web critical path.
mod portable_palettes {
    use super::*;
    use serde::{Deserialize, Serialize};
    pub fn serialize<S: serde::Serializer>(
        tables: &BTreeMap<String, ColorTable>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let text: BTreeMap<_, _> = tables
            .iter()
            .map(|(key, table)| (key, crate::colormap::to_pal_string(table)))
            .collect();
        text.serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<String, ColorTable>, D::Error> {
        let text = BTreeMap::<String, String>::deserialize(deserializer)?;
        Ok(text
            .into_iter()
            .filter_map(|(key, text)| {
                crate::colormap::parse_pal(&text)
                    .ok()
                    .map(|table| (key, table))
            })
            .collect())
    }
}

impl Default for RadarAppearance {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            blend_width: default_blend_width(),
            precision: false,
            multipass: false,
            fade_old_sectors: false,
            upper_bounds: Default::default(),
            palettes: Default::default(),
        }
    }
}

impl RadarAppearance {
    pub fn sanitize(&mut self) {
        self.opacity = if self.opacity.is_finite() { self.opacity.clamp(0.0, 1.0) } else { 1.0 };
        self.blend_width = if self.blend_width.is_finite() {
            self.blend_width.clamp(0.05, 1.0)
        } else {
            default_blend_width()
        };
        for (i, moment) in Moment::ALL.iter().enumerate() {
            let (lo, hi) = moment.value_range();
            self.upper_bounds[i] = self.upper_bounds[i]
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(lo, hi));
        }
        self.palettes.retain(|key, table| {
            Moment::ALL.iter().any(|m| m.short_name() == key)
                && !table.stops.is_empty()
                && table.stops.iter().all(|s| s.value.is_finite())
                && table
                    .stops
                    .windows(2)
                    .all(|stops| stops[0].value <= stops[1].value)
        });
    }

    pub fn table(
        &self,
        palettes: &Palettes,
        moment: Moment,
        theme: crate::settings::Theme,
    ) -> ColorTable {
        self.palettes
            .get(moment.short_name())
            .cloned()
            .unwrap_or_else(|| crate::colormap::effective_table(palettes, moment, theme))
    }

    pub fn fingerprint(&self, moment: Moment) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.opacity.to_bits().hash(&mut h);
        self.blend_width.to_bits().hash(&mut h);
        self.precision.hash(&mut h);
        self.multipass.hash(&mut h);
        self.fade_old_sectors.hash(&mut h);
        self.upper_bounds[moment.index()]
            .map(f32::to_bits)
            .hash(&mut h);
        if let Some(table) = self.palettes.get(moment.short_name()) {
            table.rf.hash(&mut h);
            for stop in &table.stops {
                stop.value.to_bits().hash(&mut h);
                stop.rgba.hash(&mut h);
                stop.end.hash(&mut h);
                stop.solid.hash(&mut h);
            }
        }
        h.finish()
    }

    pub fn apply_upload(&self, upload: &mut crate::render::RadarUpload, moment: Moment) {
        scale_opacity(&mut upload.lut, self.opacity);
        upload.uniform[17] = self.blend_width;
        upload.uniform[19] = f32::from(self.multipass);
        if let Some(ceiling) = self.upper_bounds[moment.index()] {
            mask_above(&mut upload.lut, moment.value_range(), ceiling);
        }
    }
}

fn scale_opacity(lut: &mut [u8], opacity: f32) {
    let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 };
    for rgba in lut.as_chunks_mut::<4>().0 {
        rgba[3] = (f32::from(rgba[3]) * opacity).round() as u8;
    }
}

/// Apply the physical upper bound to every precipitation palette row, preserving
/// no-data/range-fold classifications and all native measurement storage.
fn mask_above(lut: &mut [u8], (lo, hi): (f32, f32), ceiling: f32) {
    let width = lut.len() / (3 * 4);
    for row in lut.chunks_exact_mut(width * 4) {
        for raw in 2..width {
            let value = lo + (raw as f32 - 2.0) / (width - 3) as f32 * (hi - lo);
            if value > ceiling {
                row[raw * 4 + 3] = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_opacity_preserves_colors_and_transparent_data() {
        let mut lut = [10, 20, 30, 255, 40, 50, 60, 0, 70, 80, 90, 100];
        scale_opacity(&mut lut, 0.5);
        assert_eq!(lut, [10, 20, 30, 128, 40, 50, 60, 0, 70, 80, 90, 50]);
        let mut appearance: RadarAppearance = serde_json::from_str("{}").unwrap();
        assert_eq!(appearance.opacity, 1.0);
        let old_key = appearance.fingerprint(Moment::Reflectivity);
        appearance.opacity = 0.5;
        assert_ne!(old_key, appearance.fingerprint(Moment::Reflectivity));
        appearance.opacity = f32::NAN;
        appearance.sanitize();
        assert_eq!(appearance.opacity, 1.0);
    }

    #[test]
    fn preset_roundtrip_copies_display_without_changing_station_or_product() {
        use crate::render::mercator::Camera;
        let mut source = crate::view::MapView::new(Some("KTLX".into()), Camera::at_lonlat(-97.0, 35.0, 5.0));
        source.appearance.precision = true;
        source.appearance.blend_width = 0.73;
        source.appearance.palettes.insert("REF".into(), crate::colormap::default_table(Moment::Reflectivity).clone());
        source.smooth = true;
        source.camera.globe = true;
        source.thresholds[0] = Some(20.0);
        source.threshold_enabled[0] = true;
        let preset = AppearancePreset::capture(&source);
        let preset: AppearancePreset = serde_json::from_str(&serde_json::to_string(&preset).unwrap()).unwrap();
        let mut destination = crate::view::MapView::new(Some("KFWS".into()), Camera::at_lonlat(-98.0, 32.0, 6.0));
        destination.moment = Moment::Velocity;
        preset.apply(&mut destination);
        assert_eq!(AppearancePreset::capture(&destination), preset);
        assert_eq!(destination.site.as_deref(), Some("KFWS"));
        assert_eq!(destination.moment, Moment::Velocity);
        assert_eq!(destination.camera.center, Camera::at_lonlat(-98.0, 32.0, 6.0).center);
    }

    #[test]
    fn bound_clips_every_category_without_hiding_fold_or_changing_colors() {
        let mut lut = vec![255; 256 * 4 * 3];
        mask_above(&mut lut, (0.0, 253.0), 100.0);
        for row in lut.as_chunks::<1024>().0 {
            assert_eq!(row[7], 255); // range-fold
            assert_eq!(row[102 * 4 + 3], 255); // inclusive upper bound
            assert_eq!(row[103 * 4 + 3], 0);
            assert_eq!(row[255 * 4], 255); // color bytes untouched
        }
    }

    #[test]
    fn old_layout_defaults_and_portable_palette_roundtrip() {
        let mut a: RadarAppearance = serde_json::from_str("{}").unwrap();
        assert_eq!(a, RadarAppearance::default());
        a.palettes.insert(
            "REF".into(),
            crate::colormap::default_table(Moment::Reflectivity).clone(),
        );
        let encoded = serde_json::to_string(&a).unwrap();
        assert_eq!(a, serde_json::from_str(&encoded).unwrap());
    }

    #[test]
    fn portable_palettes_preserve_alpha_breaks_and_solid_bands() {
        let mut a = RadarAppearance::default();
        let table = crate::colormap::parse_pal("Product: REF\nUnits: dBZ\nStep: 5\nRF: 100 50 25\nColor4: -10 1 2 3 100 4 5 6 150\nSolidColor4: 20 10 20 30 75\nColor: 50 100 200 250\n").unwrap();
        a.palettes.insert("REF".into(), table);
        let json = serde_json::to_string(&a).unwrap();
        let restored: RadarAppearance = serde_json::from_str(&json).unwrap();
        assert_eq!(a, restored);
        assert!(json.contains("Color4:"));
    }

    #[test]
    fn sanitizes_nonfinite_imports_and_fingerprint_changes_only_relevant_moment() {
        let mut a = RadarAppearance::default();
        let before = a.fingerprint(Moment::Reflectivity);
        a.upper_bounds[Moment::Velocity.index()] = Some(20.0);
        assert_eq!(before, a.fingerprint(Moment::Reflectivity));
        a.blend_width = f32::NAN;
        a.upper_bounds[Moment::Reflectivity.index()] = Some(f32::INFINITY);
        a.sanitize();
        assert_eq!(a.blend_width, 0.55);
        assert!(a.upper_bounds[Moment::Reflectivity.index()].is_none());
    }
}
