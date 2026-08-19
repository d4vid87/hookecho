//! Frame profiling, compiled out unless the `profiling` cargo feature is on.
//!
//! Build with `cargo run --features profiling` (or `android/build.sh` with `FEATURES=profiling`),
//! then attach `puffin_viewer --url 127.0.0.1:8585`. On Android, `adb forward tcp:8585 tcp:8585`
//! first. A TCP server instead of an in-app flamegraph window keeps the profiler off the device's
//! own frame budget — the thing being measured — and avoids pinning an extra egui-versioned dep.

/// Wraps a block of work in a puffin scope. No-op (and argument-free) without the feature.
#[macro_export]
macro_rules! prof_scope {
    ($name:expr) => {
        #[cfg(feature = "profiling")]
        puffin::profile_scope!($name);
    };
}

/// Marks the end of a frame and, on the first call, starts the puffin TCP server.
pub fn new_frame() {
    #[cfg(feature = "profiling")]
    {
        use std::sync::OnceLock;
        static SERVER: OnceLock<Option<puffin_http::Server>> = OnceLock::new();
        SERVER.get_or_init(|| {
            puffin::set_scopes_on(true);
            match puffin_http::Server::new("0.0.0.0:8585") {
                Ok(s) => {
                    log::info!("puffin server listening on 0.0.0.0:8585");
                    install_pacing_sink();
                    Some(s)
                }
                Err(e) => {
                    log::error!("puffin server failed to start: {e}");
                    None
                }
            }
        });
        puffin::GlobalProfiler::lock().new_frame();
    }
}

/// Per-frame pacing, printed periodically. `cost` is the CPU time puffin measured inside the
/// frame; `gap` is the wall time between the starts of consecutive frames — the number that goes
/// bad when a vsync is missed, and the one a flamegraph cannot show you.
#[cfg(feature = "profiling")]
#[derive(Default)]
struct Pacing {
    cost_us: Vec<i64>,
    gap_us: Vec<i64>,
    last_start_ns: Option<i64>,
}

#[cfg(feature = "profiling")]
impl Pacing {
    fn push(&mut self, frame: &puffin::FrameData) {
        let (start, _) = frame.range_ns();
        if let Some(prev) = self.last_start_ns.replace(start) {
            self.gap_us.push((start - prev) / 1_000);
        }
        self.cost_us.push(frame.duration_ns() / 1_000);
    }

    /// `p50/p90/p99/max`, in milliseconds. Sorts a copy: this runs once every few hundred frames,
    /// off the frame path, on a few hundred `i64`s.
    fn quantiles(v: &[i64]) -> String {
        if v.is_empty() {
            return "n/a".to_string();
        }
        let mut v = v.to_vec();
        v.sort_unstable();
        let at = |q: f64| v[(((v.len() - 1) as f64) * q).round() as usize] as f64 / 1000.0;
        format!(
            "p50 {:.2} p90 {:.2} p99 {:.2} max {:.2}",
            at(0.5),
            at(0.9),
            at(0.99),
            at(1.0)
        )
    }

    /// Frames whose gap ran past `budget`. A handful is normal (the app repaints on demand, so an
    /// idle stretch is a long gap and not a dropped frame); a steady fraction while panning is
    /// the jank itself.
    fn over(v: &[i64], budget_us: i64) -> usize {
        v.iter().filter(|g| **g > budget_us).count()
    }

    fn report(&self) {
        log::info!(
            "perf: {} frames | cpu ms {} | gap ms {} | gaps >20ms {}/{}",
            self.cost_us.len(),
            Self::quantiles(&self.cost_us),
            Self::quantiles(&self.gap_us),
            Self::over(&self.gap_us, 20_000),
            self.gap_us.len(),
        );
    }
}

/// How many frames a pacing report covers. `HOOKECHO_PERF_EVERY=0` turns the reports off and
/// leaves the TCP server alone.
#[cfg(feature = "profiling")]
fn report_every() -> usize {
    std::env::var("HOOKECHO_PERF_EVERY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300)
}

/// Attach the pacing sink. Called once, from the same `OnceLock` that starts the server.
#[cfg(feature = "profiling")]
fn install_pacing_sink() {
    let every = report_every();
    if every == 0 {
        return;
    }
    puffin::GlobalProfiler::lock().add_sink(Box::new(move |frame| {
        if let Ok(mut p) = PACING.lock() {
            p.push(&frame);
            if p.cost_us.len() >= every {
                p.report();
                *p = Pacing::default();
            }
        }
    }));
}

/// ponytail: a mutex around the accumulator because puffin's sink is `Fn`, not `FnMut`. It is
/// taken once per frame and held for a push, which is nothing next to the frame it is measuring.
#[cfg(feature = "profiling")]
static PACING: std::sync::Mutex<Pacing> = std::sync::Mutex::new(Pacing {
    cost_us: Vec::new(),
    gap_us: Vec::new(),
    last_start_ns: None,
});

/// Runs under `cargo test -p hookecho --features profiling` — the rest of the module is compiled
/// out without it, and so is this.
#[cfg(all(test, feature = "profiling"))]
mod tests {
    use super::Pacing;

    #[test]
    fn quantiles_and_the_over_budget_count_read_the_right_frames() {
        // 1..=100 ms in microseconds.
        let v: Vec<i64> = (1..=100).map(|ms| ms * 1_000).collect();
        assert_eq!(Pacing::quantiles(&v), "p50 51.00 p90 90.00 p99 99.00 max 100.00");
        // 80 of them are over 20 ms, and the budget is exclusive: 20 ms itself is not late.
        assert_eq!(Pacing::over(&v, 20_000), 80);
        assert_eq!(Pacing::quantiles(&[]), "n/a");
    }

    #[test]
    fn the_gap_is_between_frame_starts_and_the_first_frame_has_none() {
        let mut p = Pacing::default();
        p.last_start_ns = Some(1_000_000);
        // A frame starting 16.7 ms later.
        p.gap_us.push((17_700_000 - 1_000_000) / 1_000);
        assert_eq!(p.gap_us, vec![16_700]);
    }
}
