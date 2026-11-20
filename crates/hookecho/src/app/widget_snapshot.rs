//! Foreground captures for the Android home-screen widgets.

#[cfg(target_os = "android")]
impl super::HookEchoApp {
    pub(super) fn drive_widget_snapshot(&mut self, ctx: &egui::Context) {
        let every = std::time::Duration::from_secs(if self.settings.battery_saver {
            900
        } else {
            300
        });
        if self.screenshot_pending.is_some()
            || self.share_card.is_some()
            || self.views[self.active].volume.is_none()
            || !crate::platform::activity::is_active()
            || self.widget_shot_at.is_some_and(|t| t.elapsed() < every)
            || self.widget_writer.available_permits() == 0
        {
            return;
        }
        // Rate-limit the widget-manager lookup even on phones with no installed widgets.
        self.widget_shot_at = Some(wxdata::clock::Instant::now());
        if crate::platform::radar_widget_installed() {
            if let Some(path) = crate::paths::widget_snapshot() {
                self.screenshot_pending = Some(super::ShotDest::Widget(path));
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
        } else if crate::platform::alert_widget_installed() {
            self.queue_widget_write(None);
        }
    }

    pub(super) fn queue_widget_write(
        &self,
        snapshot: Option<(std::path::PathBuf, std::sync::Arc<egui::ColorImage>)>,
    ) {
        let Ok(permit) = self.widget_writer.clone().try_acquire_owned() else {
            return;
        };
        let here = self.chase_pos.or_else(|| {
            self.settings
                .markers
                .iter()
                .find(|m| m.home)
                .map(|m| (m.lon, m.lat))
        });
        let line = here.and_then(|(lon, lat)| {
            super::widget_storm_line(self.active_storm_cells(), lon, lat, self.metric())
        });
        self.spawner.spawn_blocking(move || {
            let _permit = permit;
            if let Some((path, image)) = &snapshot {
                if let Err(error) = write_png(path, image) {
                    log::warn!("widget snapshot failed: {error}");
                    return;
                }
            }
            if let Some(path) = crate::paths::widget_caption() {
                let result = match line {
                    Some(line) => crate::settings::atomic_write(&path, line.as_bytes()),
                    None => std::fs::remove_file(&path),
                };
                if let Err(error) = result {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        log::warn!("widget caption failed: {error}");
                    }
                }
            }
            if snapshot.is_some() {
                crate::platform::refresh_radar_widget();
            }
        });
    }
}

fn write_png(path: &std::path::Path, image: &egui::ColorImage) -> anyhow::Result<()> {
    use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgba};
    // RemoteViews sends the decoded bitmap over Binder. 150k RGBA pixels fit in 600 KB.
    const MAX_PIXELS: u32 = 150_000;
    let width = u32::try_from(image.width())?;
    let height = u32::try_from(image.height())?;
    anyhow::ensure!(width > 0 && height > 0, "empty widget image");
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    let pixels = u64::from(width) * u64::from(height);
    if pixels > u64::from(MAX_PIXELS) {
        let scale = (f64::from(MAX_PIXELS) / pixels as f64).sqrt();
        let w = ((f64::from(width) * scale) as u32).clamp(1, MAX_PIXELS);
        let h = ((f64::from(height) * scale) as u32).clamp(1, MAX_PIXELS / w);
        let source = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, image.as_raw())
            .ok_or_else(|| anyhow::anyhow!("invalid widget image"))?;
        let small = image::imageops::resize(&source, w, h, image::imageops::FilterType::Triangle);
        encoder.write_image(&small, w, h, ExtendedColorType::Rgba8)?;
    } else {
        encoder.write_image(image.as_raw(), width, height, ExtendedColorType::Rgba8)?;
    }
    crate::settings::atomic_write(path, &png)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_png;

    #[test]
    fn widget_png_is_bounded_and_failed_capture_keeps_last_picture() {
        let path = std::env::temp_dir().join(format!("hookecho-widget-{}.png", std::process::id()));
        let image = |size| egui::ColorImage::filled(size, egui::Color32::RED);
        write_png(&path, &image([2, 3])).unwrap();
        assert_eq!(image::image_dimensions(&path).unwrap(), (2, 3));
        write_png(&path, &image([1000, 500])).unwrap();
        let (w, h) = image::image_dimensions(&path).unwrap();
        assert!(w * h <= 150_000 && w.abs_diff(h * 2) <= 1);
        let picture = std::fs::read(&path).unwrap();
        assert!(write_png(&path, &image([0, 1])).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), picture);
        std::fs::remove_file(path).unwrap();
    }
}
