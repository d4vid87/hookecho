//! 3D reflectivity window: an orbitable maximum-intensity raymarch of the volume, drawn by an
//! `egui_wgpu` paint callback. Drag to orbit, scroll to zoom, and cut the volume down to the part
//! you care about with a reflectivity floor and three axis slabs.

use crate::render3d::{orbit_uniform, threshold_index, View3d, Volume3dCallback, Volume3dUpload};

/// Everything the window keeps between frames: the orbit camera, the dBZ floor, and the slab.
#[derive(Clone, Copy, Debug)]
pub struct Volume3dState {
    pub az: f32,
    pub el: f32,
    pub dist: f32,
    /// Reflectivity floor in dBZ. Anything weaker is treated as empty.
    pub threshold_dbz: f32,
    /// Slab bounds as fractions of the box, `[x0, x1, y0, y1, z0, z1]`.
    pub clip: [f32; 6],
}

impl Default for Volume3dState {
    fn default() -> Self {
        Self {
            az: 30.0,
            el: 25.0,
            dist: 3.0,
            // The volume's own floor: nothing hidden until the user asks.
            threshold_dbz: f32::NEG_INFINITY,
            clip: [0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
        }
    }
}

/// One `min..max` pair of sliders for an axis of the slab.
fn axis_slice(ui: &mut egui::Ui, label: &str, lo: &mut f32, hi: &mut f32) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::Slider::new(lo, 0.0..=1.0).show_value(false));
        ui.add(egui::Slider::new(hi, 0.0..=1.0).show_value(false));
    });
    // Keep the pair ordered so an inverted drag empties the view instead of inverting the slab.
    if *lo > *hi {
        std::mem::swap(lo, hi);
    }
}

/// Show the 3D window. `pending` is a one-shot volume upload consumed by the first paint;
/// `n`/`nz` are the grid dimensions and `range` the volume's `(value_min, value_max)` dBZ span.
pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    st: &mut Volume3dState,
    pending: &mut Option<Volume3dUpload>,
    n: u32,
    nz: u32,
    range: (f32, f32),
) {
    let mut keep = *open;
    crate::ui::phone_surface(ctx, egui::Window::new("3D Reflectivity"))
        .open(&mut keep)
        .default_size([480.0, 480.0])
        .show(ctx, |ui| {
            ui.weak("Drag to orbit · scroll to zoom · max-intensity projection");
            ui.horizontal(|ui| {
                let mut on = st.threshold_dbz.is_finite();
                if ui
                    .checkbox(&mut on, "Only above")
                    .on_hover_text("Hide everything weaker, so cores stand alone")
                    .changed()
                {
                    st.threshold_dbz = if on { 45.0 } else { f32::NEG_INFINITY };
                }
                if on {
                    let mut dbz = st.threshold_dbz;
                    if ui
                        .add(egui::Slider::new(&mut dbz, range.0..=range.1).suffix(" dBZ"))
                        .changed()
                    {
                        st.threshold_dbz = dbz;
                    }
                    if ui
                        .button("Hail core")
                        .on_hover_text("45 dBZ — the usual floor for a hail core")
                        .clicked()
                    {
                        st.threshold_dbz = 45.0;
                    }
                }
            });
            egui::CollapsingHeader::new("Slice")
                .default_open(false)
                .show(ui, |ui| {
                    let [x0, x1, y0, y1, z0, z1] = &mut st.clip;
                    axis_slice(ui, "E–W", x0, x1);
                    axis_slice(ui, "N–S", y0, y1);
                    axis_slice(ui, "Up ", z0, z1);
                    if ui.button("Whole volume").clicked() {
                        st.clip = [0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
                    }
                });
            let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
            if resp.dragged() {
                let d = resp.drag_delta();
                st.az -= d.x * 0.4;
                st.el = (st.el + d.y * 0.3).clamp(2.0, 88.0);
            }
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 && resp.hovered() {
                st.dist = (st.dist - scroll * 0.003).clamp(1.3, 6.0);
            }
            let aspect = rect.width() / rect.height().max(1.0);
            // The threshold is a uniform, not a re-upload: dragging the slider never rebuilds the
            // texture.
            let view = View3d {
                threshold_idx: if st.threshold_dbz.is_finite() {
                    threshold_index(st.threshold_dbz, range)
                } else {
                    2.0
                },
                clip: st.clip,
            };
            let uniform = orbit_uniform(st.az, st.el, st.dist, aspect, n, nz, 256, view);
            // On the window's first frame egui may hand us a zero-area rect (auto-size pass);
            // the paint callback would be culled and the one-shot upload lost, leaving the view
            // black until reopened. Hold the upload until the rect is real.
            let upload = (rect.height() >= 1.0 && rect.width() >= 1.0)
                .then(|| pending.take())
                .flatten();
            let cb = Volume3dCallback { upload, uniform };
            ui.painter()
                .add(egui_wgpu::Callback::new_paint_callback(rect, cb));
        });
    *open = keep;
}

#[cfg(test)]
mod tests {
    use crate::render3d::threshold_index;

    #[test]
    fn dbz_maps_into_the_volume_index_range() {
        let range = (-30.0, 80.0);
        assert_eq!(threshold_index(-30.0, range), 2.0);
        assert_eq!(threshold_index(80.0, range), 255.0);
        // 45 dBZ sits where the hail-core preset should: well up the ramp, not at either end.
        let idx = threshold_index(45.0, range);
        assert!((170.0..=180.0).contains(&idx), "{idx}");
        // Out-of-range asks clamp rather than wrap.
        assert_eq!(threshold_index(-99.0, range), 2.0);
        assert_eq!(threshold_index(999.0, range), 255.0);
    }
}
