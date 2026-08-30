//! System-tray icon via StatusNotifier (KDE/freedesktop). Linux-only; other platforms get a
//! no-op stub so the call sites stay clean. The tray runs its own D-Bus service thread; menu
//! clicks (and left-click activation) are delivered to the app through a channel it polls.

/// Command from the tray menu to the app.
pub enum TrayCmd {
    /// Restore/raise the window.
    Show,
    /// Quit the application for real (bypasses close-to-tray).
    Quit,
    /// Toggle audio alerts (mirrors the in-app mute action).
    Mute,
    /// Jump the active pane to this radar site.
    Site(String),
}

/// What the tray shows about the app. Pushed only when it changes — the menu is a D-Bus object,
/// not something to repaint per frame.
#[derive(Clone, Default, PartialEq)]
pub struct TrayState {
    /// Active alerts in the current view, for the summary line.
    pub alerts: usize,
    pub muted: bool,
    /// Starred sites, offered as jump items.
    pub starred: Vec<String>,
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{TrayCmd, TrayState};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::Arc;

    struct HookEchoTray {
        tx: Sender<TrayCmd>,
        icon: ksni::Icon,
        state: TrayState,
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
            use ksni::menu::{CheckmarkItem, StandardItem};
            let mut items: Vec<ksni::MenuItem<Self>> = vec![
                StandardItem {
                    label: match self.state.alerts {
                        0 => "No active alerts".into(),
                        1 => "1 active alert".into(),
                        n => format!("{n} active alerts"),
                    },
                    // A status line, not a button.
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                CheckmarkItem {
                    label: "Mute audio alerts".into(),
                    checked: self.state.muted,
                    activate: Box::new(|t: &mut HookEchoTray| {
                        let _ = t.tx.send(TrayCmd::Mute);
                    }),
                    ..Default::default()
                }
                .into(),
            ];
            // Starred sites: the same list the toolbox presets dropdown offers.
            for site in &self.state.starred {
                let id = site.clone();
                items.push(
                    StandardItem {
                        label: id.clone(),
                        activate: Box::new(move |t: &mut HookEchoTray| {
                            let _ = t.tx.send(TrayCmd::Site(id.clone()));
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            items.extend([
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
            ]);
            items
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
        let (state_tx, state_rx) = std::sync::mpsc::channel::<TrayState>();
        let _ = STATE_TX.set(state_tx);
        let present = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&present);
        std::thread::spawn(move || {
            let tray = HookEchoTray {
                tx,
                icon: logo_icon(),
                state: TrayState::default(),
            };
            let handle = match tray.spawn() {
                Ok(handle) => {
                    flag.store(true, Ordering::Relaxed);
                    handle
                }
                Err(e) => {
                    log::warn!("tray icon unavailable ({e}); using taskbar fallback");
                    return;
                }
            };
            // The handle used to be leaked, which is why the menu could never change. Kept here
            // instead: this thread outlives the process anyway, and it owns the updates.
            while let Ok(state) = state_rx.recv() {
                handle.update(|t: &mut HookEchoTray| t.state = state);
            }
        });
        (rx, present)
    }

    /// Where [`set_state`] posts; set once by [`spawn`].
    static STATE_TX: std::sync::OnceLock<Sender<TrayState>> = std::sync::OnceLock::new();

    /// Push new tray state. Cheap and non-blocking — the D-Bus update happens on the tray thread.
    pub fn set_state(state: TrayState) {
        if let Some(tx) = STATE_TX.get() {
            let _ = tx.send(state);
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::{TrayCmd, TrayState};
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

    /// No tray to tell.
    pub fn set_state(_state: TrayState) {}
}

pub use imp::{set_state, spawn};
