//! Keyboard shortcuts.
//!
//! A flat binding table maps keys to [`Action`]s; the app applies each action by mutating
//! the active `MapView`, so hotkeys and toolbox buttons share one code path. U8's Hotkeys
//! settings tab swaps `DEFAULTS` for a user-loaded list without touching call sites.

use wxdata::level2::Moment;

/// A thing a key can trigger.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Product(Moment),
    TiltUp,
    TiltDown,
    OpenSiteDialog,
    Reload,
    CycleBasemap,
    ToggleAlertPanel,
    ToggleObs,
    ToggleObsTour,
    InstantReplay,
}

/// A key bound to an action.
pub struct Binding {
    pub key: egui::Key,
    pub action: Action,
}

/// Default bindings: 1–6 select products, PageUp/Down change tilt, F3 site dialog, F5 reload.
pub const DEFAULTS: &[Binding] = &[
    Binding { key: egui::Key::Num1, action: Action::Product(Moment::Reflectivity) },
    Binding { key: egui::Key::Num2, action: Action::Product(Moment::Velocity) },
    Binding { key: egui::Key::Num3, action: Action::Product(Moment::SpectrumWidth) },
    Binding { key: egui::Key::Num4, action: Action::Product(Moment::DifferentialReflectivity) },
    Binding { key: egui::Key::Num5, action: Action::Product(Moment::DifferentialPhase) },
    Binding { key: egui::Key::Num6, action: Action::Product(Moment::CorrelationCoefficient) },
    Binding { key: egui::Key::PageUp, action: Action::TiltUp },
    Binding { key: egui::Key::PageDown, action: Action::TiltDown },
    Binding { key: egui::Key::F3, action: Action::OpenSiteDialog },
    Binding { key: egui::Key::F5, action: Action::Reload },
    Binding { key: egui::Key::Z, action: Action::CycleBasemap },
    Binding { key: egui::Key::A, action: Action::ToggleAlertPanel },
    Binding { key: egui::Key::F8, action: Action::ToggleObs },
    Binding { key: egui::Key::F9, action: Action::ToggleObsTour },
    Binding { key: egui::Key::R, action: Action::InstantReplay },
];

/// Actions triggered this frame. No-op while a text field has focus so typing a site id
/// doesn't fire product shortcuts.
pub fn poll(ctx: &egui::Context) -> Vec<Action> {
    if ctx.memory(|m| m.focused().is_some()) {
        return Vec::new(); // a text field has focus; don't steal keys
    }
    ctx.input_mut(|i| {
        DEFAULTS
            .iter()
            .filter(|b| i.consume_key(egui::Modifiers::NONE, b.key))
            .map(|b| b.action)
            .collect()
    })
}
