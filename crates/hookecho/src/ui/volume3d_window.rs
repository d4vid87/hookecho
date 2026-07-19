//! 3D reflectivity window: an orbitable maximum-intensity raymarch of the volume, drawn by an
//! `egui_wgpu` paint callback. Drag to orbit, scroll to zoom.

use crate::render3d::{orbit_uniform, Volume3dCallback, Volume3dUpload};

/// Show the 3D window. `az`/`el` are orbit degrees, `dist` the camera distance; `pending` is a
/// one-shot volume upload consumed by the first paint. `n`/`nz` are the grid dimensions.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    az: &mut f32,
    el: &mut f32,
    dist: &mut f32,
    pending: &mut Option<Volume3dUpload>,
    n: u32,
    nz: u32,
) {
    let mut keep = *open;
    egui::Window::new("3D Reflectivity")
        .open(&mut keep)
        .default_size([480.0, 420.0])
        .show(ctx, |ui| {
            ui.weak("Drag to orbit · scroll to zoom · max-intensity projection");
            let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
            if resp.dragged() {
                let d = resp.drag_delta();
                *az -= d.x * 0.4;
                *el = (*el + d.y * 0.3).clamp(2.0, 88.0);
            }
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 && resp.hovered() {
                *dist = (*dist - scroll * 0.003).clamp(1.3, 6.0);
            }
            let aspect = rect.width() / rect.height().max(1.0);
            let uniform = orbit_uniform(*az, *el, *dist, aspect, n, nz, 256);
            let cb = Volume3dCallback { upload: pending.take(), uniform };
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, cb));
        });
    *open = keep;
}
