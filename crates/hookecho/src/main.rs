//! Hook Echo-WX — advanced NEXRAD weather radar viewer.
//!
//! The desktop binary: dispatches the `--headless-*` verifiers, then launches the windowed app.
//! The shared app + render pipelines live in the `hookecho` library crate (see `lib.rs`), so the
//! Android `cdylib` reuses all of it through its own `android_main` entry.
//!
//! Windows release builds link the GUI subsystem so launching from Explorer doesn't pop a console
//! window; `AttachConsole` at the top of `main` re-attaches to a parent terminal when there is one,
//! so the `--headless-*` verifiers and `--version` still print. Debug builds keep the console.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

#[cfg(not(target_os = "android"))]
use hookecho::{audio, headless, icon, settings, tiles, tray};
#[cfg(not(target_os = "android"))]
use wxdata::level2::Moment;

/// The bin target is desktop-only (Android enters through `android_main` in the lib, and never
/// runs this executable), but `cargo build` for the Android target still compiles it — so it
/// needs to at least link there.
#[cfg(target_os = "android")]
fn main() {}

#[cfg(not(target_os = "android"))]
fn main() -> eframe::Result<()> {
    // GUI-subsystem exes start with no stdout; borrow the parent terminal's if we were launched
    // from one. Fails harmlessly from Explorer.
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();

    // `hookecho --version` — matches what the About window and the update check compare against.
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("hookecho {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Terminal report: `hookecho --status [--json|--line] [LAT,LON]` — conditions and nearby
    // alerts for your markers, for a status bar, a cron job, or Home Assistant.
    if let Some(pos) = args.iter().position(|a| a == "--status") {
        use hookecho::status;
        let mode = if args.iter().any(|a| a == "--json") {
            status::Mode::Json
        } else if args.iter().any(|a| a == "--line") {
            status::Mode::Line
        } else {
            status::Mode::Human
        };
        // The point is the first non-flag argument after `--status`, so `--status --json 35,-97`
        // and `--status 35,-97 --json` both work.
        let point = args[pos + 1..]
            .iter()
            .find(|a| !a.starts_with("--"))
            .map(String::as_str);
        let spots = status::spots(&settings::Settings::load(), point);
        if let Err(e) = status::run(&spots, mode) {
            eprintln!("status failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // HTTP mode: `hookecho --serve [PORT] [--bind ADDR]` — the same report over the network, plus
    // a radar snapshot. Loopback by default; `--bind 0.0.0.0` is a deliberate act.
    if let Some(pos) = args.iter().position(|a| a == "--serve") {
        let port = args
            .get(pos + 1)
            .filter(|a| !a.starts_with("--"))
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(8080);
        let bind = flag_value(&args, "--bind").unwrap_or("127.0.0.1");
        // `--web-root DIR` also serves the browser build sitting in that directory.
        let web_root = flag_value(&args, "--web-root").map(std::path::PathBuf::from);
        let spots = hookecho::status::spots(&settings::Settings::load(), None);
        if let Err(e) = hookecho::serve::run(spots, bind, port, web_root) {
            eprintln!("serve failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Logo export: `hookecho --headless-icon <out.png> [SIZE]` (PNG inspection, not a desktop
    // capture). The logo is drawn procedurally, so any size is exact rather than resampled —
    // which is what the hicolor theme wants, one file per size.
    if let Some(pos) = args.iter().position(|a| a == "--headless-icon") {
        let out = args.get(pos + 1).map(String::as_str).unwrap_or("icon.png");
        let size = args
            .get(pos + 2)
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(256)
            .clamp(16, 1024);
        let px = icon::rgba(size as usize);
        if let Err(e) = image::save_buffer(out, &px, size, size, image::ColorType::Rgba8) {
            eprintln!("icon export failed: {e}");
            std::process::exit(1);
        }
        println!("wrote {out}");
        return Ok(());
    }

    // Windows icon export: `hookecho --headless-ico <out.ico>`. Run by hand when the logo changes;
    // the result is checked in as packaging/windows/icon.ico and embedded by build.rs.
    if let Some(pos) = args.iter().position(|a| a == "--headless-ico") {
        let out = args.get(pos + 1).map(String::as_str).unwrap_or("icon.ico");
        if let Err(e) = write_ico(out) {
            eprintln!("ico export failed: {e}");
            std::process::exit(1);
        }
        println!("wrote {out}");
        return Ok(());
    }

    // Tray self-check: `hookecho --tray-test` spawns the tray and reports availability.
    if args.iter().any(|a| a == "--tray-test") {
        match tray::spawn() {
            Some(_rx) => {
                println!("tray: StatusNotifier host present, tray icon spawned");
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            None => println!("tray: no StatusNotifier host (would fall back to taskbar)"),
        }
        return Ok(());
    }

    // Audio cue check: `hookecho --chime` plays the default chime once and exits.
    if args.iter().any(|a| a == "--chime") {
        audio::play(&settings::AlertSound::default(), 0.2);
        std::thread::sleep(std::time::Duration::from_millis(900));
        return Ok(());
    }

    // Overlay verify: `hookecho --headless-overlay <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-overlay") {
        let out = args
            .get(pos + 1)
            .map(String::as_str)
            .unwrap_or("overlay.png");
        if let Err(e) = headless::run_overlay(out) {
            eprintln!("headless overlay render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // MRMS mosaic verify: `hookecho --headless-mrms <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-mrms") {
        let out = args.get(pos + 1).map(String::as_str).unwrap_or("mrms.png");
        if let Err(e) = headless::run_mrms(out) {
            eprintln!("headless mrms render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // National field-layer verify: `hookecho --headless-field <slug> <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-field") {
        let slug = args
            .get(pos + 1)
            .map(String::as_str)
            .unwrap_or("rotation30");
        let out = args
            .get(pos + 2)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("field.png");
        if let Err(e) = headless::run_field(slug, out) {
            eprintln!("headless field render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Global model verify: `hookecho --headless-global <gfs|ecmwf> <slug> <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-global") {
        let model = args.get(pos + 1).map(String::as_str).unwrap_or("gfs");
        let slug = args.get(pos + 2).map(String::as_str).unwrap_or("mslp");
        let out = args
            .get(pos + 3)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("global.png");
        if let Err(e) = headless::run_global(model, slug, out) {
            eprintln!("headless global render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Model difference: `hookecho --headless-diff <field-slug> [out.png]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-diff") {
        let slug = args.get(pos + 1).map(String::as_str).unwrap_or("mslp");
        let out = args
            .get(pos + 2)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("diff.png");
        if let Err(e) = headless::run_diff(slug, out) {
            eprintln!("headless difference render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Tropical verify: `hookecho --headless-tropical`.
    if args.iter().any(|a| a == "--headless-tropical") {
        if let Err(e) = headless::run_tropical() {
            eprintln!("headless tropical failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // SPC outlook verify (all eight days): `hookecho --headless-outlooks`.
    if args.iter().any(|a| a == "--headless-outlooks") {
        if let Err(e) = headless::run_outlooks() {
            eprintln!("headless outlooks failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Surface-obs verify: `hookecho --headless-metar [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-metar") {
        let site = args
            .get(pos + 1)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        if let Err(e) = headless::run_metar(site) {
            eprintln!("headless metar failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // River-gauge verify: `hookecho --headless-gauges [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-gauges") {
        let site = args
            .get(pos + 1)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        if let Err(e) = headless::run_gauges(site) {
            eprintln!("headless gauges failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Chase-pack verify: `hookecho --headless-chasepack [lat lon radius_km zmax] [--basemap slug]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-chasepack") {
        let num = |i: usize, d: f64| args.get(pos + i).and_then(|a| a.parse().ok()).unwrap_or(d);
        let (lat, lon, radius, zmax) = (
            num(1, 35.3),
            num(2, -97.5),
            num(3, 100.0),
            (num(4, 12.0) as u8).min(18),
        );
        let slug = args
            .iter()
            .position(|a| a == "--basemap")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
            .unwrap_or("carto-dark");
        if let Err(e) = headless::run_chasepack(lat, lon, radius, zmax, slug) {
            eprintln!("headless chasepack failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // HRRR contour verify: `hookecho --headless-contours <mslp|t2m|td2m|cape|srh>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-contours") {
        let kind = args.get(pos + 1).map(String::as_str).unwrap_or("mslp");
        if let Err(e) = headless::run_contours(kind) {
            eprintln!("headless contours failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Gridded L3 verify: `hookecho --headless-l3grid <dvl|eet> [SITE] <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-l3grid") {
        let kind = args.get(pos + 1).map(String::as_str).unwrap_or("dvl");
        let site = args
            .get(pos + 2)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        let out = args
            .get(pos + 3)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("l3grid.png");
        if let Err(e) = headless::run_l3grid(kind, site, out) {
            eprintln!("headless l3grid render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Environment-suite verify: `hookecho --headless-env <sbcape|mlcape|srh1|srh3> <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-env") {
        let slug = args.get(pos + 1).map(String::as_str).unwrap_or("sbcape");
        let out = args
            .get(pos + 2)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("env.png");
        if let Err(e) = headless::run_env(slug, out) {
            eprintln!("headless env render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // HRRR layer verify: `hookecho --headless-hrrr [refc|uh|smoke] [fcsthour] <out.png>`.
    // The layer name is optional and defaults to future radar, so the old argument order
    // (`--headless-hrrr 3 out.png`) still works.
    if let Some(pos) = args.iter().position(|a| a == "--headless-hrrr") {
        use hookecho::render::FieldLayer as FL;
        let mut rest = args[pos + 1..]
            .iter()
            .take_while(|a| !a.starts_with("--"))
            .map(String::as_str);
        let mut next = rest.next();
        let layer = match next {
            Some("uh") | Some("rotation") => {
                next = rest.next();
                FL::UpdraftHelicity
            }
            Some("smoke") => {
                next = rest.next();
                FL::Smoke
            }
            Some("refc") | Some("radar") => {
                next = rest.next();
                FL::Hrrr
            }
            _ => FL::Hrrr,
        };
        let fh: u8 = next.and_then(|s| s.parse().ok()).unwrap_or(1);
        let out = if next.is_some_and(|s| s.parse::<u8>().is_ok()) {
            rest.next()
        } else {
            next
        }
        .unwrap_or("hrrr.png");
        if let Err(e) = headless::run_hrrr_layer(layer, fh, out) {
            eprintln!("headless hrrr render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // GLM lightning verify: `hookecho --headless-glm`.
    if args.iter().any(|a| a == "--headless-glm") {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let res = rt.block_on(async {
            let client = reqwest::Client::new();
            let mut feed = wxdata::glm::GlmFeed::new(15);
            let n = feed.poll(&client).await?;
            Ok::<_, anyhow::Error>((n, feed))
        });
        match res {
            Ok((added, feed)) => {
                let f = feed.flashes();
                println!("GLM: {added} flashes ingested, {} in the window", f.len());
                if let (Some(first), Some(last)) = (f.front(), f.back()) {
                    let age = (chrono::Utc::now() - last.time).num_seconds();
                    println!(
                        "  span {} .. {}  (newest {age}s old)",
                        first.time.format("%H:%M:%SZ"),
                        last.time.format("%H:%M:%SZ")
                    );
                    let lats: Vec<f64> = f.iter().map(|x| x.lat).collect();
                    let lons: Vec<f64> = f.iter().map(|x| x.lon).collect();
                    println!(
                        "  lat [{:.1}, {:.1}]  lon [{:.1}, {:.1}]",
                        lats.iter().cloned().fold(f64::MAX, f64::min),
                        lats.iter().cloned().fold(f64::MIN, f64::max),
                        lons.iter().cloned().fold(f64::MAX, f64::min),
                        lons.iter().cloned().fold(f64::MIN, f64::max),
                    );
                    let e: Vec<f64> = f.iter().map(|x| x.energy).filter(|v| *v > 0.0).collect();
                    if !e.is_empty() {
                        println!(
                            "  energy [{:.3e}, {:.3e}] J",
                            e.iter().cloned().fold(f64::MAX, f64::min),
                            e.iter().cloned().fold(f64::MIN, f64::max)
                        );
                    }
                }
                // The gridded half: the same flashes as the flash-density field layer sees.
                match wxdata::glm::flash_density(
                    f,
                    0.05,
                    chrono::Duration::minutes(15),
                    chrono::Utc::now(),
                ) {
                    Some(g) => {
                        let mut cells: Vec<f32> =
                            g.values.iter().copied().filter(|v| !v.is_nan()).collect();
                        cells.sort_by(|a, b| b.total_cmp(a));
                        println!(
                            "  density {}x{} cells, {} lit, max {}",
                            g.nx,
                            g.ny,
                            cells.len(),
                            cells.first().copied().unwrap_or(0.0)
                        );
                        let top: Vec<String> =
                            cells.iter().take(5).map(|v| format!("{v:.0}")).collect();
                        println!("  busiest cells: {}", top.join(", "));
                    }
                    None => println!("  density: no flashes in the window"),
                }
            }
            Err(e) => {
                eprintln!("headless glm failed: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    // Surface analysis verify: `hookecho --headless-fronts`.
    if args.iter().any(|a| a == "--headless-fronts") {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let res = rt.block_on(async {
            let client = reqwest::Client::new();
            wxdata::fronts::fetch(&client).await
        });
        match res {
            Ok(a) => {
                println!(
                    "surface analysis: {} fronts, {} centers, valid {}",
                    a.fronts.len(),
                    a.centers.len(),
                    a.valid
                        .map(|v| v.format("%Y-%m-%d %H:%MZ").to_string())
                        .unwrap_or_else(|| "?".into())
                );
                for kind in [
                    wxdata::fronts::FrontKind::Cold,
                    wxdata::fronts::FrontKind::Warm,
                    wxdata::fronts::FrontKind::Stationary,
                    wxdata::fronts::FrontKind::Occluded,
                    wxdata::fronts::FrontKind::Trough,
                ] {
                    let n = a.fronts.iter().filter(|f| f.kind == kind).count();
                    let pts: usize = a
                        .fronts
                        .iter()
                        .filter(|f| f.kind == kind)
                        .map(|f| f.pts.len())
                        .sum();
                    println!("  {:<18} {n:>3} segments, {pts:>4} points", kind.label());
                }
                let (h, l) = (
                    a.centers.iter().filter(|c| c.high).count(),
                    a.centers.iter().filter(|c| !c.high).count(),
                );
                println!("  {h} highs, {l} lows");
            }
            Err(e) => {
                eprintln!("headless fronts failed: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    // CAPPI verify: `hookecho --headless-cappi [SITE] [alt_km] <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-cappi") {
        let site = args
            .get(pos + 1)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        let alt: f32 = args
            .get(pos + 2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(3.0);
        let out = args
            .get(pos + 3)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("cappi.png");
        if let Err(e) = headless::run_cappi(site, alt, out) {
            eprintln!("headless cappi failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // 3D raymarch verify: `hookecho --headless-3d [SITE] <out.png> [--threshold DBZ]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-3d") {
        let site = args
            .get(pos + 1)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        let out = args
            .get(pos + 2)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("volume3d.png");
        let threshold = args
            .iter()
            .position(|a| a == "--threshold")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<f32>().ok());
        if let Err(e) = headless::run_3d(site, out, threshold) {
            eprintln!("headless 3d render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Cross-section verify: `hookecho --headless-xsection SITE lon1,lat1 lon2,lat2 <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-xsection") {
        let site = args.get(pos + 1).map(String::as_str).unwrap_or("KTLX");
        let parse = |s: &str| -> Option<(f64, f64)> {
            let (a, b) = s.split_once(',')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        };
        let a = args
            .get(pos + 2)
            .and_then(|s| parse(s))
            .unwrap_or((-98.5, 35.3));
        let b = args
            .get(pos + 3)
            .and_then(|s| parse(s))
            .unwrap_or((-96.0, 35.3));
        let out = args
            .get(pos + 4)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("xsection.png");
        if let Err(e) = headless::run_xsection(site, a, b, out) {
            eprintln!("headless xsection failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Storm-reports verify: `hookecho --headless-reports [sts ets]` (RFC3339-ish UTC window).
    if let Some(pos) = args.iter().position(|a| a == "--headless-reports") {
        let sts = args
            .get(pos + 1)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str);
        let ets = args
            .get(pos + 2)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str);
        let window = sts.zip(ets);
        if let Err(e) = headless::run_reports(window) {
            eprintln!("headless reports failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // AFD verify: `hookecho --headless-afd [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-afd") {
        let site = args
            .get(pos + 1)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        if let Err(e) = headless::run_afd(site) {
            eprintln!("headless afd failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Aviation verify: `hookecho --headless-aviation`.
    if args.iter().any(|a| a == "--headless-aviation") {
        if let Err(e) = headless::run_aviation() {
            eprintln!("headless aviation failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Sounding-indices verify: `hookecho --headless-indices [lon] [lat]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-indices") {
        let lon: f64 = args
            .get(pos + 1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(-97.5);
        let lat: f64 = args
            .get(pos + 2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(35.3);
        if let Err(e) = headless::run_indices(lon, lat) {
            eprintln!("headless indices failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // TDS verify: `hookecho --headless-tds <SITE>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-tds") {
        let site = args.get(pos + 1).map(String::as_str).unwrap_or("KTLX");
        if let Err(e) = headless::run_tds(site) {
            eprintln!("headless tds failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Dual-pol signature verify: `hookecho --headless-dualpol [SITE] [H0_KM]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-dualpol") {
        let site = args.get(pos + 1).map(String::as_str).unwrap_or("KTLX");
        let h0 = args.get(pos + 2).and_then(|s| s.parse().ok()).unwrap_or(4.0);
        if let Err(e) = headless::run_dualpol(site, h0) {
            eprintln!("headless dualpol failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Rotation verify: `hookecho --headless-rotation [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-rotation") {
        let site = args.get(pos + 1).map(String::as_str).unwrap_or("KTLX");
        if let Err(e) = headless::run_rotation(site) {
            eprintln!("headless rotation failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Tornado climatology verify: `hookecho --headless-climatology [lon lat]` (default Moore, OK).
    if let Some(pos) = args.iter().position(|a| a == "--headless-climatology") {
        let lon = args
            .get(pos + 1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(-97.49);
        let lat = args
            .get(pos + 2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(35.34);
        if let Err(e) = headless::run_climatology(lon, lat) {
            eprintln!("headless climatology failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // ProbSevere verify: `hookecho --headless-probsevere`.
    if args.iter().any(|a| a == "--headless-probsevere") {
        if let Err(e) = headless::run_probsevere() {
            eprintln!("headless probsevere failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Spotter Network verify: `hookecho --headless-spotters [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-spotters") {
        let site = args.get(pos + 1).map(String::as_str).unwrap_or("KTLX");
        if let Err(e) = headless::run_spotters(site) {
            eprintln!("headless spotters failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // MRMS lightning verify: `hookecho --headless-lightning <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-lightning") {
        let out = args
            .get(pos + 1)
            .map(String::as_str)
            .unwrap_or("lightning.png");
        if let Err(e) = headless::run_lightning(out) {
            eprintln!("headless lightning render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Multi-pane render verify: `hookecho --headless-multipane [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-multipane") {
        let site = args
            .get(pos + 1)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        if let Err(e) = headless::run_multipane(site, "pane0.png", "pane1.png") {
            eprintln!("headless multipane failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Placefile verify: `hookecho --headless-placefile <file.txt> [out.png]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-placefile") {
        let path = args
            .get(pos + 1)
            .map(String::as_str)
            .unwrap_or("placefile.txt");
        let out = args
            .get(pos + 2)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("placefile.png");
        if let Err(e) = headless::run_placefile(path, out) {
            eprintln!("headless placefile failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Level 3 storm-cell verify: `hookecho --headless-cells [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-cells") {
        let site = args.get(pos + 1).map(String::as_str).unwrap_or("KTLX");
        if let Err(e) = headless::run_cells(site) {
            eprintln!("headless cells failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Multi-radar mosaic verify: `hookecho --headless-mosaic [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-mosaic") {
        let site = args
            .get(pos + 1)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        if let Err(e) = headless::run_mosaic(site) {
            eprintln!("headless mosaic failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Warning verification: `hookecho --headless-verify WFO START END`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-verify") {
        let (wfo, start, end) = (args.get(pos + 1), args.get(pos + 2), args.get(pos + 3));
        let (Some(wfo), Some(start), Some(end)) = (wfo, start, end) else {
            eprintln!("usage: --headless-verify WFO START END   (e.g. OUN 2013-05-20 2013-05-21)");
            std::process::exit(2);
        };
        if let Err(e) = headless::run_verify(wfo, start, end) {
            eprintln!("headless verify failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Sensor/obs verify: `hookecho --headless-obs [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-obs") {
        let site = args
            .get(pos + 1)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        if let Err(e) = headless::run_obs(site) {
            eprintln!("headless obs failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // VAD wind-profile verify: `hookecho --headless-vwp [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-vwp") {
        let site = args
            .get(pos + 1)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        if let Err(e) = headless::run_vwp(site) {
            eprintln!("headless vwp failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // NWS alert-metadata verify: `hookecho --headless-alerts`.
    if args.iter().any(|a| a == "--headless-alerts") {
        if let Err(e) = headless::run_alerts() {
            eprintln!("headless alerts failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Archived-warning verify: `hookecho --headless-archwarn <RFC3339>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-archwarn") {
        let ts = args
            .get(pos + 1)
            .map(String::as_str)
            .unwrap_or("2013-05-20T20:00:00Z");
        if let Err(e) = headless::run_archwarn(ts) {
            eprintln!("headless archwarn failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Live-stream verify: `hookecho --headless-live <out.png> [SITE] [--moment M]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-live") {
        let out = args.get(pos + 1).map(String::as_str).unwrap_or("live.png");
        let site = args
            .get(pos + 2)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        let moment = flag_value(&args, "--moment")
            .and_then(Moment::from_code)
            .unwrap_or(Moment::Reflectivity);
        if let Err(e) = headless::run_live(out, site, moment) {
            eprintln!("headless live render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Headless verify mode: `hookecho --headless <out.png> [SITE] [--moment REF] [--tilt N]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless") {
        let out = args
            .get(pos + 1)
            .map(String::as_str)
            .unwrap_or("hookecho.png");
        // First positional after out that isn't a flag is the site.
        let site = args
            .get(pos + 2)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        let moment = flag_value(&args, "--moment")
            .and_then(Moment::from_code)
            .unwrap_or(Moment::Reflectivity);
        let tilt = flag_value(&args, "--tilt")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        let smooth = args.iter().any(|a| a == "--smooth");
        let dealias = args.iter().any(|a| a == "--dealias");
        let pal = flag_value(&args, "--pal");
        let date = flag_value(&args, "--date")
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
        let time = flag_value(&args, "--time");
        use tiles::BasemapStyle;
        let basemap = match flag_value(&args, "--basemap") {
            Some("sat") => BasemapStyle::Satellite,
            Some(s) => BasemapStyle::from_slug(s),
            None => BasemapStyle::None,
        };
        // `--srv DIR/SPD` (degrees / knots) applies storm-relative velocity.
        let storm_uv = flag_value(&args, "--srv").and_then(|v| {
            let (d, s) = v.split_once('/')?;
            let dir: f32 = d.parse().ok()?;
            let spd_kt: f32 = s.parse().ok()?;
            let spd = spd_kt / 1.943_844;
            let r = dir.to_radians();
            Some((spd * r.sin(), spd * r.cos()))
        });
        if let Err(e) = headless::run(
            out, site, moment, tilt, smooth, pal, storm_uv, date, time, basemap, dealias,
        ) {
            eprintln!("headless render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Desktop-widget mode: `hookecho --snapshot out.png [SITE] [--size N] [--zoom Z]
    // [--every SECS]`. The same off-screen render `--headless` and the server's `/snapshot.png`
    // use, written where conky, a desktop wallpaper script or `feh --reload` can pick it up.
    if let Some(pos) = args.iter().position(|a| a == "--snapshot") {
        let out = args
            .get(pos + 1)
            .map(String::as_str)
            .unwrap_or("hookecho.png");
        let site = args
            .get(pos + 2)
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .unwrap_or("KTLX");
        let moment = flag_value(&args, "--moment")
            .and_then(Moment::from_code)
            .unwrap_or(Moment::Reflectivity);
        let basemap = match flag_value(&args, "--basemap") {
            Some("sat") => tiles::BasemapStyle::Satellite,
            Some(s) => tiles::BasemapStyle::from_slug(s),
            None => tiles::BasemapStyle::Dark,
        };
        headless::set_output(
            flag_value(&args, "--size").and_then(|v| v.parse().ok()),
            flag_value(&args, "--zoom").and_then(|v| v.parse().ok()),
        );
        let every = flag_value(&args, "--every").and_then(|v| v.parse::<u64>().ok());
        loop {
            // Render to a sibling temp file and rename over the target: a widget polling the file
            // on its own clock must never catch a half-written PNG, and rename is atomic. The
            // `.png` stays on the end because the encoder picks its format from the extension.
            let tmp = format!("{out}.tmp.png");
            match headless::run(
                &tmp, site, moment, 0, true, None, None, None, None, basemap, false,
            )
            .and_then(|_| std::fs::rename(&tmp, out).map_err(Into::into))
            {
                Ok(()) => println!("wrote {out}"),
                Err(e) => {
                    eprintln!("snapshot failed: {e}");
                    let _ = std::fs::remove_file(&tmp);
                    // A one-shot render reports the failure; a loop keeps going, because the
                    // usual cause is a feed being briefly unreachable.
                    if every.is_none() {
                        std::process::exit(1);
                    }
                }
            }
            let Some(secs) = every else { return Ok(()) };
            std::thread::sleep(std::time::Duration::from_secs(secs.max(10)));
        }
    }

    // A `hookecho://goto/…` link handed over by the desktop (the .desktop MimeType on Linux, the
    // URL Protocol registry key on Windows) arrives as an argument. The app already knows how to
    // read one out of the environment — Android's notification tap uses the same path — so this
    // is a handover, not a second parser.
    if let Some(link) = args.iter().find(|a| a.starts_with("hookecho://")) {
        // An already-running instance is what the user means by "open this link": a second
        // window with its own caches, streams and GPU context is not. Hand it over and leave.
        if hand_link_to_running_instance(link) {
            log::info!("handed {link} to the running instance");
            return Ok(());
        }
        log::info!("opening link {link}");
        std::env::set_var("HOOKECHO_GOTO", link);
    }
    single_instance::listen();

    hookecho::run_desktop()
}

/// Write `link` into the drop box the running instance polls, if there is one.
///
/// Returns whether it was handed over; false means this process is the instance.
#[cfg(not(target_os = "android"))]
fn hand_link_to_running_instance(link: &str) -> bool {
    if !single_instance::another_is_running() {
        return false;
    }
    let Some(path) = hookecho::paths::goto_file() else {
        return false;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&path, link).is_ok()
}

/// Whether another Hook Echo owns this machine's session, over a bound loopback port.
///
/// A bound socket is its own liveness proof — no stale pid file to reason about — and the
/// greeting keeps an unrelated listener on the port from swallowing links.
///
/// ponytail: a fixed port and a plain greeting, no payload over the wire (the link goes through
/// the same `goto.txt` drop box Android already uses). If the port is taken by something else,
/// the link opens a second instance, which is exactly today's behavior.
#[cfg(not(target_os = "android"))]
mod single_instance {
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream, SocketAddrV4};
    use std::time::Duration;

    /// Registered-user range, and nothing else known claims it.
    const PORT: u16 = 47_913;
    const GREETING: &[u8] = b"hookecho\n";

    pub fn another_is_running() -> bool {
        probe(PORT)
    }

    fn probe(port: u16) -> bool {
        let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        let Ok(mut sock) = TcpStream::connect_timeout(&addr.into(), Duration::from_millis(300))
        else {
            return false;
        };
        let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
        let mut buf = [0u8; GREETING.len()];
        sock.read_exact(&mut buf).is_ok() && buf == GREETING
    }

    /// Answer future launches. Silently does nothing if the port is unavailable — the app runs
    /// fine without this, it just stops being the one instance.
    pub fn listen() {
        listen_on(PORT);
    }

    /// Returns the port actually bound, so a test can use an ephemeral one.
    fn listen_on(port: u16) -> Option<u16> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).ok()?;
        let bound = listener.local_addr().ok()?.port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let _ = stream.write_all(GREETING);
            }
        });
        Some(bound)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_greeting_is_what_identifies_us() {
            // Nothing listening: not us, and not a hang either.
            assert!(!probe(1));

            let port = listen_on(0).expect("bind an ephemeral port");
            assert!(probe(port), "a listening instance answers");
        }
    }
}

/// Write a multi-resolution Windows `.ico` of the app logo (16/32/48/256 px, the sizes Explorer,
/// the taskbar and the installer ask for).
#[cfg(not(target_os = "android"))]
fn write_ico(out: &str) -> anyhow::Result<()> {
    use image::codecs::ico::{IcoEncoder, IcoFrame};

    let frames: Vec<IcoFrame> = [16u32, 32, 48, 256]
        .iter()
        .map(|&n| {
            IcoFrame::as_png(
                &icon::rgba(n as usize),
                n,
                n,
                image::ExtendedColorType::Rgba8,
            )
        })
        .collect::<Result<_, _>>()?;
    IcoEncoder::new(std::io::BufWriter::new(std::fs::File::create(out)?)).encode_images(&frames)?;
    Ok(())
}

/// The token following `flag` on the command line, if present.
#[cfg(not(target_os = "android"))]
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}
