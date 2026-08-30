//! The measuring stick the performance work is judged by.
//!
//! `wxdata::stats` does the counting; this turns those counters into something readable: an egui
//! window when `HOOKECHO_PERF=1` is set, and a line in the log once a minute, which is the only
//! readout `--headless` and `--serve` runs have. Rates are per minute over the last window rather
//! than process-lifetime averages, because the question every wave asks is "what is it doing
//! *now*, idle" — a lifetime average of a session that spent ten minutes scrubbing hides it.
//!
//! Native only. The browser build shares these code paths but carries no counters (see
//! `wxdata::stats`), so a native number stands in for the wasm one.
//!
// ponytail: env var, not a menu item — turning the readout on is a developer action, and a new
// window in the UI is a user-visible change this work is not allowed to make.

use std::time::{Duration, Instant};

/// How often the rates are recomputed and the log line printed.
const WINDOW: Duration = Duration::from_secs(60);

pub struct PerfReadout {
    /// `HOOKECHO_PERF=1`: draw the window as well as logging.
    show: bool,
    /// Start of the current window, and the counter values it began with.
    since: Instant,
    base: Vec<(&'static str, u64)>,
    /// Per-minute rates from the last completed window, ready to draw.
    rates: Vec<(&'static str, f64)>,
    /// The idle repaint interval the app last asked for, in ms — the number the heartbeat work
    /// moves, and the one that explains the frame rate above it.
    pub idle_ms: u64,
    /// Process start, for the startup line — the real one when `mark_start` was called, else
    /// this struct's construction, which is as early as a library-only build can see.
    start: Instant,
    first_frame: Option<Duration>,
}

/// Process start, for the startup line.
///
/// Set from `main` before anything else runs. Without it the startup number would begin at app
/// construction and miss the window, the adapter and the pipeline compile — which is most of what
/// a launch costs and the whole reason to measure it.
static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Stamp the start of the process. Idempotent; the first call wins.
pub fn mark_start() {
    let _ = START.set(Instant::now());
}

fn process_start() -> Instant {
    *START.get_or_init(Instant::now)
}

impl Default for PerfReadout {
    fn default() -> Self {
        Self::new()
    }
}

impl PerfReadout {
    pub fn new() -> Self {
        Self {
            show: std::env::var("HOOKECHO_PERF").is_ok_and(|v| v != "0"),
            since: Instant::now(),
            base: wxdata::stats::snapshot(),
            rates: Vec::new(),
            idle_ms: 0,
            start: process_start(),
            first_frame: None,
        }
    }

    /// Call once per frame, early. Rolls the window when it is up and draws the window if asked.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if self.first_frame.is_none() {
            let dt = self.start.elapsed();
            self.first_frame = Some(dt);
            log::info!("perf: first frame after {} ms", dt.as_millis());
        }
        if self.since.elapsed() >= WINDOW {
            self.roll();
        }
        if self.show {
            self.window(ctx);
        }
    }

    /// Close the window: turn the deltas into per-minute rates and log them.
    fn roll(&mut self) {
        let now = wxdata::stats::snapshot();
        let per_min = 60.0 / self.since.elapsed().as_secs_f64().max(0.001);
        self.rates = now
            .iter()
            .zip(&self.base)
            .map(|((label, n), (_, was))| (*label, n.saturating_sub(*was) as f64 * per_min))
            .collect();
        let line = self
            .rates
            .iter()
            .map(|(l, r)| format!("{l}={r:.0}/min"))
            .collect::<Vec<_>>()
            .join(" ");
        log::info!("perf: idle={}ms {line}", self.idle_ms);
        self.since = Instant::now();
        self.base = now;
    }

    fn window(&self, ctx: &egui::Context) {
        egui::Window::new("Perf")
            .default_pos(egui::pos2(12.0, 60.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.monospace(format!("idle repaint  {} ms", self.idle_ms));
                if let Some(dt) = self.first_frame {
                    ui.monospace(format!("first frame   {} ms", dt.as_millis()));
                }
                ui.separator();
                if self.rates.is_empty() {
                    ui.monospace(format!(
                        "measuring… {}s",
                        WINDOW.saturating_sub(self.since.elapsed()).as_secs()
                    ));
                    return;
                }
                for (label, rate) in &self.rates {
                    ui.monospace(format!("{label:<21}{rate:>9.0}/min"));
                }
            });
    }
}
