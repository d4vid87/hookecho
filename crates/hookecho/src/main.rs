//! Hook Echo-WX — advanced NEXRAD weather radar viewer.

mod app;
mod audio;
mod basemap_style;
mod colormap;
mod digest;
mod events;
mod geo;
mod gps;
mod headless;
mod hotkeys;
mod icon;
mod loopexport;
mod overlay_build;
mod render;
mod render3d;
mod settings;
mod theme;
mod tiles;
mod timeline;
mod tray;
mod ui;
mod vector_tiles;
mod view;

use app::HookEchoApp;
use wxdata::level2::Moment;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();

    // Logo export: `hookecho --headless-icon <out.png>` (PNG inspection, not a desktop capture).
    if let Some(pos) = args.iter().position(|a| a == "--headless-icon") {
        let out = args.get(pos + 1).map(String::as_str).unwrap_or("icon.png");
        let px = icon::rgba(256);
        if let Err(e) = image::save_buffer(out, &px, 256, 256, image::ColorType::Rgba8) {
            eprintln!("icon export failed: {e}");
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

    // Audio cue check: `hookecho --chime` plays the warning chime once and exits.
    if args.iter().any(|a| a == "--chime") {
        audio::warning_chime();
        std::thread::sleep(std::time::Duration::from_millis(900));
        return Ok(());
    }

    // Overlay verify: `hookecho --headless-overlay <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-overlay") {
        let out = args.get(pos + 1).map(String::as_str).unwrap_or("overlay.png");
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
        let slug = args.get(pos + 1).map(String::as_str).unwrap_or("rotation30");
        let out = args.get(pos + 2).filter(|a| !a.starts_with("--")).map(String::as_str).unwrap_or("field.png");
        if let Err(e) = headless::run_field(slug, out) {
            eprintln!("headless field render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // HRRR future-radar verify: `hookecho --headless-hrrr [fcsthour] <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-hrrr") {
        let fh: u8 = args.get(pos + 1).and_then(|s| s.parse().ok()).unwrap_or(1);
        let out = args.get(pos + 2).filter(|a| !a.starts_with("--")).map(String::as_str).unwrap_or("hrrr.png");
        if let Err(e) = headless::run_hrrr(fh, out) {
            eprintln!("headless hrrr render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // 3D raymarch verify: `hookecho --headless-3d [SITE] <out.png>`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-3d") {
        let site = args.get(pos + 1).filter(|a| !a.starts_with("--")).map(String::as_str).unwrap_or("KTLX");
        let out = args.get(pos + 2).filter(|a| !a.starts_with("--")).map(String::as_str).unwrap_or("volume3d.png");
        if let Err(e) = headless::run_3d(site, out) {
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
        let a = args.get(pos + 2).and_then(|s| parse(s)).unwrap_or((-98.5, 35.3));
        let b = args.get(pos + 3).and_then(|s| parse(s)).unwrap_or((-96.0, 35.3));
        let out = args.get(pos + 4).filter(|a| !a.starts_with("--")).map(String::as_str).unwrap_or("xsection.png");
        if let Err(e) = headless::run_xsection(site, a, b, out) {
            eprintln!("headless xsection failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // SPC storm-reports verify: `hookecho --headless-reports`.
    if args.iter().any(|a| a == "--headless-reports") {
        if let Err(e) = headless::run_reports() {
            eprintln!("headless reports failed: {e}");
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

    // Tornado climatology verify: `hookecho --headless-climatology [lon lat]` (default Moore, OK).
    if let Some(pos) = args.iter().position(|a| a == "--headless-climatology") {
        let lon = args.get(pos + 1).and_then(|s| s.parse().ok()).unwrap_or(-97.49);
        let lat = args.get(pos + 2).and_then(|s| s.parse().ok()).unwrap_or(35.34);
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
        let out = args.get(pos + 1).map(String::as_str).unwrap_or("lightning.png");
        if let Err(e) = headless::run_lightning(out) {
            eprintln!("headless lightning render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Multi-pane render verify: `hookecho --headless-multipane [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-multipane") {
        let site = args.get(pos + 1).filter(|a| !a.starts_with("--")).map(String::as_str).unwrap_or("KTLX");
        if let Err(e) = headless::run_multipane(site, "pane0.png", "pane1.png") {
            eprintln!("headless multipane failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Placefile verify: `hookecho --headless-placefile <file.txt> [out.png]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-placefile") {
        let path = args.get(pos + 1).map(String::as_str).unwrap_or("placefile.txt");
        let out = args.get(pos + 2).filter(|a| !a.starts_with("--")).map(String::as_str).unwrap_or("placefile.png");
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

    // Sensor/obs verify: `hookecho --headless-obs [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-obs") {
        let site = args.get(pos + 1).filter(|a| !a.starts_with("--")).map(String::as_str).unwrap_or("KTLX");
        if let Err(e) = headless::run_obs(site) {
            eprintln!("headless obs failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // VAD wind-profile verify: `hookecho --headless-vwp [SITE]`.
    if let Some(pos) = args.iter().position(|a| a == "--headless-vwp") {
        let site = args.get(pos + 1).filter(|a| !a.starts_with("--")).map(String::as_str).unwrap_or("KTLX");
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
        let out = args.get(pos + 1).map(String::as_str).unwrap_or("hookecho.png");
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
        if let Err(e) = headless::run(out, site, moment, tilt, smooth, pal, storm_uv, date, time, basemap, dealias) {
            eprintln!("headless render failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("Hook Echo-WX")
            .with_icon(icon::icon_data()),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Hook Echo-WX",
        native_options,
        Box::new(|cc| Ok(Box::new(HookEchoApp::new(cc)))),
    )
}

/// The token following `flag` on the command line, if present.
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}
