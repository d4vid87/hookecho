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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::Arc;

    struct HookEchoTray {
        tx: Sender<TrayCmd>,
        icon: ksni::Icon,
    }

    impl ksni::Tray for HookEchoTray {
        fn id(&self) -> String {
            "hookecho".into()
        }
        fn title(&self) -> String {
            "HookEcho".into()
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
                    label: "Show HookEcho".into(),
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

    /// ARGB32 (network byte order) tray icon from the app logo.
    fn logo_icon() -> ksni::Icon {
        let size = 64usize;
        let mut data = crate::icon::rgba(size); // RGBA
        for px in data.as_chunks_mut::<4>().0 {
            px.rotate_right(1); // RGBA -> ARGB
        }
        ksni::Icon {
            width: size as i32,
            height: size as i32,
            data,
        }
    }

    /// Spawn the tray service. Returns the command receiver and a flag that becomes true once a
    /// StatusNotifier host has accepted the item (it stays false when there is none, and the app
    /// falls back to minimize-to-taskbar).
    ///
    /// The registration itself is a blocking D-Bus round trip, so it happens on its own thread:
    /// on the main thread it sat in front of the first frame, and a wedged host stalled launch
    /// outright. The service `Handle` is leaked so the tray lives for the process lifetime.
    pub fn spawn() -> (Receiver<TrayCmd>, Arc<AtomicBool>) {
        use ksni::blocking::TrayMethods;
        let (tx, rx) = std::sync::mpsc::channel();
        let present = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&present);
        std::thread::spawn(move || {
            let tray = HookEchoTray {
                tx,
                icon: logo_icon(),
            };
            match tray.spawn() {
                Ok(handle) => {
                    flag.store(true, Ordering::Relaxed);
                    std::mem::forget(handle);
                }
                Err(e) => log::warn!("tray icon unavailable ({e}); using taskbar fallback"),
            }
        });
        (rx, present)
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::TrayCmd;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::Receiver;
    use std::sync::Arc;

    /// No native tray on this platform yet (Windows would use `tray-icon`). The receiver is a
    /// live-but-empty channel so the call site needs no `cfg`.
    pub fn spawn() -> (Receiver<TrayCmd>, Arc<AtomicBool>) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::mem::forget(tx); // keep the channel open rather than hand back a disconnected one
        (rx, Arc::new(AtomicBool::new(false)))
    }
}

pub use imp::spawn;
