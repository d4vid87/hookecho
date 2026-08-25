//! A floating player for a stream URL the user attached to a marker or a chase partner.
//!
//! Only the transports the app already decodes get played here (MJPEG, and HLS when `ffmpeg` is
//! installed) — see [`crate::cam`]. YouTube and Twitch links are handed to the system browser
//! instead, because playing those needs a whole extra stack.

/// A running player: the stream plus the texture its frames land in.
pub struct VideoPlayer {
    pub title: String,
    pub url: String,
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    stream: crate::cam::Stream,
    tex: Option<egui::TextureHandle>,
}

/// True when the URL is something [`crate::cam`] can decode in-app; everything else belongs in a
/// browser.
pub fn playable_in_app(url: &str) -> bool {
    if cfg!(any(target_os = "android", target_arch = "wasm32")) {
        return false;
    }
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    let host = url.to_ascii_lowercase();
    if host.contains("youtube.com") || host.contains("youtu.be") || host.contains("twitch.tv") {
        return false;
    }
    path.ends_with(".m3u8")
        || path.contains("mjpg")
        || path.contains("mjpeg")
        || path.ends_with(".cgi")
}

impl VideoPlayer {
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    pub fn start(title: String, url: String, spawner: &crate::rt::Spawner) -> Self {
        Self {
            title,
            stream: crate::cam::Stream::start(url.clone(), spawner.handle()),
            url,
            tex: None,
        }
    }

    #[cfg(any(target_os = "android", target_arch = "wasm32"))]
    pub fn start(title: String, url: String, _spawner: &crate::rt::Spawner) -> Self {
        Self {
            title,
            url,
            tex: None,
        }
    }

    /// Draw the window. Returns false once the user closes it (the caller drops the player,
    /// which stops the download).
    pub fn show(&mut self, ctx: &egui::Context, drawer: &mut crate::ui::drawer::Drawer) -> bool {
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        {
            if let Some(img) = self.stream.take_frame() {
                match &mut self.tex {
                    // Replaced in place: a 25 fps stream must not mint a texture per frame.
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => {
                        self.tex =
                            Some(ctx.load_texture("video", img, egui::TextureOptions::LINEAR))
                    }
                }
                ctx.request_repaint();
            }
        }
        let mut open = true;
        let title = format!("📹 {}", self.title);
        let Some(window) = drawer.page_sized(
            ctx,
            &title,
            &mut open,
            false,
            480.0,
            egui::Window::new(title.clone()).id(egui::Id::new(("video-window", &self.url))),
        ) else {
            return open;
        };
        window.vscroll(false).show(ctx, |ui| {
                let w = ui.available_width();
                match &self.tex {
                    Some(tex) => {
                        let size = tex.size_vec2();
                        let h = if size.x > 0.0 {
                            w * size.y / size.x
                        } else {
                            270.0
                        };
                        ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(w, h)));
                    }
                    None => {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(w, 200.0), egui::Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 4.0, egui::Color32::from_gray(24));
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            self.status_text(),
                            egui::FontId::proportional(12.0),
                            egui::Color32::from_gray(190),
                        );
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Open in browser").clicked() {
                        if let Err(e) = crate::platform::open_url(&self.url) {
                            log::warn!("open stream URL failed: {e}");
                        }
                    }
                    ui.weak(&self.url);
                });
            });
        open
    }

    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    fn status_text(&self) -> String {
        match self.stream.status() {
            crate::cam::Status::NeedsFfmpeg => {
                "This stream is HLS — install ffmpeg to watch it here".to_string()
            }
            crate::cam::Status::Offline(e) => format!("Stream offline: {e}"),
            _ => "Connecting\u{2026}".to_string(),
        }
    }

    #[cfg(any(target_os = "android", target_arch = "wasm32"))]
    fn status_text(&self) -> String {
        "In-app video is desktop-only — open in a browser".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::playable_in_app;

    #[test]
    fn only_decodable_urls_play_in_app() {
        if cfg!(any(target_os = "android", target_arch = "wasm32")) {
            return;
        }
        assert!(playable_in_app("https://example.com/live/stream.m3u8"));
        assert!(playable_in_app("http://10.0.0.5/mjpg/video.cgi?x=1"));
        assert!(!playable_in_app("https://www.youtube.com/watch?v=abc"));
        assert!(!playable_in_app("https://twitch.tv/somechaser"));
        assert!(!playable_in_app("https://example.com/page.html"));
    }
}
