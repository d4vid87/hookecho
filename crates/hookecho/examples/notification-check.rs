//! Browser API contract check, built separately from the production app.
//! The driver replaces Notification with a recording constructor; no alerts leave the browser.
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)] // Compile the production adapter without linking the app's wasm start.
#[path = "../src/notify.rs"]
mod notify;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn run_notification_check() {
    notify::desktop_summary("Summary one", "First batch");
    notify::desktop_summary("Summary two", "Updated batch");
    notify::desktop("Individual urgent alert", "Separate card");
}
