//! System-tray icon via StatusNotifier (KDE/freedesktop). Linux-only; other platforms get a
//! no-op stub so the call sites stay clean. The tray runs its own D-Bus service thread; menu
//! clicks (and left-click activation) are delivered to the app through a channel it polls.

/// Command from the tray menu to the app.
pub enum TrayCmd {
    /// Restore/raise the window.
    Show,
    /// Quit the application for real (bypasses close-to-tray).
    Quit,
}

#[cfg(target_os = "linux")]
mod imp {
    use super::TrayCmd;
    use std::sync::mpsc::{Receiver, Sender};

    struct HookEchoTray {
        tx: Sender<TrayCmd>,
        icon: ksni::Icon,
    }

    impl ksni::Tray for HookEchoTray {
        fn id(&self) -> String {
            "hookecho".into()
        }
        fn title(&self) -> String {
            "Hook Echo-WX".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            vec![self.icon.clone()]
        }
        // Left-click restores the window.
        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.tx.send(TrayCmd::Show);
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::StandardItem;
            vec![
                StandardItem {
                    label: "Show Hook Echo-WX".into(),
                    activate: Box::new(|t: &mut HookEchoTray| {
                        let _ = t.tx.send(TrayCmd::Show);
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Quit".into(),
                    activate: Box::new(|t: &mut HookEchoTray| {
                        let _ = t.tx.send(TrayCmd::Quit);
                    }),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    /// ARGB32 (network byte order) tray icon from our procedural logo.
    fn logo_icon() -> ksni::Icon {
        let size = 64usize;
        let mut data = crate::icon::rgba(size); // RGBA
        for px in data.chunks_exact_mut(4) {
            px.rotate_right(1); // RGBA -> ARGB
        }
        ksni::Icon { width: size as i32, height: size as i32, data }
    }

    /// Spawn the tray service. Returns the command receiver, or `None` if no StatusNotifier host
    /// is available (the app falls back to minimize-to-taskbar). The service `Handle` is leaked so
    /// the tray lives for the process lifetime.
    pub fn spawn() -> Option<Receiver<TrayCmd>> {
        use ksni::blocking::TrayMethods;
        let (tx, rx) = std::sync::mpsc::channel();
        let tray = HookEchoTray { tx, icon: logo_icon() };
        match tray.spawn() {
            Ok(handle) => {
                std::mem::forget(handle);
                Some(rx)
            }
            Err(e) => {
                log::warn!("tray icon unavailable ({e}); using taskbar fallback");
                None
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::TrayCmd;
    use std::sync::mpsc::Receiver;

    /// No native tray on this platform yet (Windows would use `tray-icon`).
    pub fn spawn() -> Option<Receiver<TrayCmd>> {
        None
    }
}

pub use imp::spawn;
