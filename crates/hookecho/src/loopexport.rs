//! Bounded radar-loop capture and cancellable GIF/video encoding. The capture state machine
//! lives in [`crate::app`]; native encoders run in the blocking pool and browser GIFs in a worker.

use image::codecs::gif::{GifEncoder, Repeat};
use image::RgbaImage;
use image::{Delay, Frame};
use std::path::Path;
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};

const WEB_CAPTURE_BYTES: usize = 64 * 1024 * 1024;
const NATIVE_CAPTURE_BYTES: usize = 128 * 1024 * 1024;

/// Keep all selected scans, reducing resolution only when their raw pixels exceed the budget.
pub fn capture_dimensions(width: u32, height: u32, count: usize, browser: bool) -> anyhow::Result<(u32, u32)> {
    anyhow::ensure!(width > 0 && height > 0 && count > 0, "no frames captured");
    let budget = if browser { WEB_CAPTURE_BYTES } else { NATIVE_CAPTURE_BYTES };
    let pixels = (width as f64) * (height as f64) * 4.0 * count as f64;
    let mut scale = (budget as f64 / pixels).sqrt().min(1.0);
    if browser { scale = scale.min(960.0 / width.max(height) as f64); }
    let dimensions = ((width as f64 * scale).floor().max(1.0) as u32,
        (height as f64 * scale).floor().max(1.0) as u32);
    anyhow::ensure!(dimensions.0.max(dimensions.1) >= width.max(height).min(320),
        "This timeline is too long to export within the memory limit; select fewer scans");
    validate_capture_bytes(dimensions.0, dimensions.1, count, browser)?;
    Ok(dimensions)
}

pub fn validate_capture_bytes(width: u32, height: u32, count: usize, browser: bool) -> anyhow::Result<()> {
    let bytes = (width as usize).checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4)).and_then(|bytes| bytes.checked_mul(count));
    let budget = if browser { WEB_CAPTURE_BYTES } else { NATIVE_CAPTURE_BYTES };
    anyhow::ensure!(bytes.is_some_and(|bytes| bytes <= budget),
        "Loop exceeds the export memory limit; select fewer scans");
    Ok(())
}

#[derive(Clone, Default)]
pub struct ExportControl {
    cancelled: Arc<AtomicBool>,
    completed: Arc<AtomicUsize>,
}

impl ExportControl {
    pub fn cancel(&self) { self.cancelled.store(true, Ordering::Relaxed); }
    pub fn is_cancelled(&self) -> bool { self.cancelled.load(Ordering::Relaxed) }
    pub fn completed(&self) -> usize { self.completed.load(Ordering::Relaxed) }
    pub fn progress(&self, count: usize) { self.completed.store(count, Ordering::Relaxed); }
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.is_cancelled(), "Loop export cancelled");
        Ok(())
    }
}

/// Output container for a loop export.
#[derive(Clone, Copy, PartialEq)]
pub enum LoopFormat {
    Gif,
    Mp4,
}

pub struct Manifest<'a> {
    pub site: Option<&'a str>,
    pub product: &'a str,
    pub units: &'a str,
    pub tilt: usize,
    pub fps: f32,
    pub center: [f64; 2],
    pub zoom: f64,
}

fn manifest_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_else(|| std::ffi::OsStr::new("hookecho-loop")).to_os_string();
    name.push(".json");
    path.with_file_name(name)
}

pub fn write_manifest(path: &Path, info: &Manifest<'_>, sources: &[String]) -> anyhow::Result<()> {
    let frames: Vec<_> = sources
        .iter()
        .map(|source| {
            let object = wxdata::level2::Identifier::new(source.clone());
            serde_json::json!({
                "source_object": source,
                "valid_time": object.date_time().map(|time| time.to_rfc3339()),
            })
        })
        .collect();
    std::fs::write(
        manifest_path(path),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "hookecho.loop/v1",
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "output": path.file_name().and_then(|name| name.to_str()),
            "site": info.site,
            "product": info.product,
            "units": info.units,
            "tilt_index": info.tilt,
            "playback_fps": info.fps,
            "view": { "center_lon_lat": info.center, "zoom": info.zoom },
            "frames": frames,
        }))?,
    )?;
    Ok(())
}

/// Encode `frames` into an MP4 (H.264) at `path` via the `ffmpeg` CLI. Frames are written as
/// PNGs to a temp dir and muxed at `fps`. Errors if ffmpeg is missing or fails.
pub fn encode_mp4(frames: &[RgbaImage], fps: u32, path: &Path) -> anyhow::Result<()> {
    encode_mp4_timed(frames, &vec![1_000 / fps.max(1); frames.len()], path)
}

pub fn encode_mp4_timed(
    frames: &[RgbaImage],
    delays_ms: &[u32],
    path: &Path,
) -> anyhow::Result<()> {
    encode_mp4_controlled(frames, delays_ms, path, &ExportControl::default())
}

fn encode_mp4_controlled(frames: &[RgbaImage], delays_ms: &[u32], path: &Path, control: &ExportControl) -> anyhow::Result<()> {
    if frames.is_empty() {
        anyhow::bail!("no frames captured");
    }
    anyhow::ensure!(
        frames.len() == delays_ms.len(),
        "frame timing count mismatch"
    );
    let temporary = ExportDirectory::new(&std::env::temp_dir())?;
    let dir = &temporary.0;
    for (i, img) in frames.iter().enumerate() {
        control.check()?;
        img.save(dir.join(format!("f{i:04}.png")))?;
        control.progress(i + 1);
    }
    let mut concat = String::new();
    for (i, delay) in delays_ms.iter().enumerate() {
        concat.push_str(&format!(
            "file 'f{i:04}.png'\nduration {:.3}\n",
            *delay as f64 / 1_000.0
        ));
    }
    concat.push_str(&format!("file 'f{:04}.png'\n", frames.len() - 1));
    std::fs::write(dir.join("frames.txt"), concat)?;
    // Even dimensions are required by yuv420p; pad if odd.
    let mut ffmpeg = std::process::Command::new("ffmpeg");
    crate::platform::no_window(&mut ffmpeg);
    // current_dir changes relative output paths, so resolve the destination before spawning.
    let output = if path.is_absolute() { path.to_owned() } else { std::env::current_dir()?.join(path) };
    control.check()?;
    let mut child = ffmpeg
        .current_dir(dir)
        .args(["-y", "-f", "concat", "-safe", "0", "-i", "frames.txt"])
        .args([
            "-vf",
            "pad=ceil(iw/2)*2:ceil(ih/2)*2",
            "-fps_mode",
            "vfr",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
        ])
        .arg(output)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn().map_err(|e| anyhow::anyhow!("ffmpeg not available ({e}); GIF export works without it"))?;
    loop {
        if control.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Loop export cancelled");
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => anyhow::bail!("ffmpeg exited {status}"),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        }
    }
}

struct ExportDirectory(std::path::PathBuf);

impl ExportDirectory {
    fn new(parent: &Path) -> std::io::Result<Self> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let path = parent.join(format!(".hookecho-export-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for ExportDirectory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

/// Stage a complete clip and provenance before replacing the selected destination.
#[cfg(not(target_arch = "wasm32"))]
pub fn encode_export(frames: &[RgbaImage], format: LoopFormat, path: &Path, info: &Manifest<'_>, sources: &[String], control: &ExportControl) -> anyhow::Result<()> {
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let temporary = ExportDirectory::new(parent)?;
    let name = path.file_name().ok_or_else(|| anyhow::anyhow!("Invalid export filename"))?;
    let staged = temporary.0.join(name);
    let delays = vec![(1000.0 / info.fps.clamp(0.1, 15.0)) as u32; frames.len()];
    match format {
        LoopFormat::Gif => {
            validate_gif_frames(frames, &delays)?;
            let file = std::fs::File::create(&staged)?;
            encode_gif_to(frames, &delays, std::io::BufWriter::new(file), control)?;
        }
        LoopFormat::Mp4 => encode_mp4_controlled(frames, &delays, &staged, control)?,
    }
    control.check()?;
    write_manifest(&staged, info, sources)?;
    control.check()?;
    publish_export(&staged, path, &manifest_path(&staged), &manifest_path(path))?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn publish_export(staged: &Path, path: &Path, staged_manifest: &Path, manifest: &Path) -> anyhow::Result<()> {
    let destinations = [path, manifest];
    let mut existed = [false; 2];
    for (index, destination) in destinations.iter().enumerate() {
        match std::fs::symlink_metadata(destination) {
            Ok(metadata) => {
                anyhow::ensure!(metadata.is_file(), "Export destination is not a regular file: {}", destination.display());
                existed[index] = true;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let backup = ExportDirectory::new(parent)?;
    let backups = [backup.0.join("clip"), backup.0.join("manifest")];
    let mut installed = [false; 2];
    let result = (|| -> std::io::Result<()> {
        for index in 0..2 {
            if existed[index] { std::fs::rename(destinations[index], &backups[index])?; }
        }
        for (index, source) in [staged, staged_manifest].iter().enumerate() {
            std::fs::rename(source, destinations[index])?;
            installed[index] = true;
        }
        Ok(())
    })();
    if let Err(error) = result {
        for index in 0..2 {
            let restore = if backups[index].exists() {
                std::fs::rename(&backups[index], destinations[index])
            } else if installed[index] {
                std::fs::remove_file(destinations[index])
            } else { Ok(()) };
            if let Err(restore_error) = restore {
                let recovery = backup.0.clone();
                // Keep prior files available when the filesystem also refuses rollback.
                std::mem::forget(backup);
                anyhow::bail!("Export publication failed ({error}); restore failed ({restore_error}); previous files retained in {}", recovery.display());
            }
        }
        return Err(error.into());
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = window, js_name = __hookechoExportVideo)]
    fn export_browser_video(
        frames: &js_sys::Array,
        width: u32,
        height: u32,
        fps: f32,
        progress: &js_sys::Function,
    ) -> js_sys::Promise;
    #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = window, js_name = __hookechoExportGif)]
    fn export_browser_gif(
        frames: &js_sys::Array,
        width: u32,
        height: u32,
        delay_ms: u32,
        progress: &js_sys::Function,
    ) -> js_sys::Promise;
}

#[cfg(target_arch = "wasm32")]
pub struct BrowserExport {
    pub promise: js_sys::Promise,
    pub cancel: js_sys::Function,
    _progress: wasm_bindgen::closure::Closure<dyn FnMut(u32)>,
}

/// Consume the Rust images while copying into transferable JS buffers, keeping the handoff bounded.
#[cfg(target_arch = "wasm32")]
pub fn begin_browser_export(frames: Vec<RgbaImage>, fps: f32, format: LoopFormat, control: ExportControl) -> Result<BrowserExport, String> {
    use wasm_bindgen::JsCast;
    let first = frames.first().ok_or("no frames captured")?;
    let (width, height) = first.dimensions();
    validate_capture_bytes(width, height, frames.len(), true).map_err(|error| error.to_string())?;
    let js_frames = js_sys::Array::new();
    for frame in frames {
        if frame.dimensions() != (width, height) {
            return Err("the viewport changed size during export".to_string());
        }
        js_frames.push(&js_sys::Uint8Array::from(frame.as_raw().as_slice()));
    }
    let progress = wasm_bindgen::closure::Closure::wrap(Box::new(move |done: u32| {
        control.progress(done as usize);
    }) as Box<dyn FnMut(u32)>);
    let promise = match format {
        LoopFormat::Gif => export_browser_gif(&js_frames, width, height, (1000.0 / fps.clamp(0.1, 15.0)) as u32, progress.as_ref().unchecked_ref()),
        LoopFormat::Mp4 => export_browser_video(&js_frames, width, height, fps, progress.as_ref().unchecked_ref()),
    };
    let cancel = js_sys::Reflect::get(&promise, &"cancel".into()).map_err(|_| "Export cancellation unavailable")?
        .dyn_into::<js_sys::Function>().map_err(|_| "Export cancellation unavailable")?;
    Ok(BrowserExport { promise, cancel, _progress: progress })
}

/// Called only in the dedicated GIF worker. Convert one transferred image at a time.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn encode_gif_worker(frames: js_sys::Array, width: u32, height: u32, delay_ms: u32, progress: js_sys::Function) -> Result<Vec<u8>, wasm_bindgen::JsValue> {
    let encode = || -> anyhow::Result<Vec<u8>> {
        anyhow::ensure!(frames.length() > 0, "no frames captured");
        validate_capture_bytes(width, height, frames.length() as usize, true)?;
        anyhow::ensure!(width > 0 && height > 0 && width.max(height) <= 960, "Invalid GIF dimensions");
        let mut output = std::io::Cursor::new(Vec::new());
        {
            let mut encoder = GifEncoder::new_with_speed(&mut output, 30);
            encoder.set_repeat(Repeat::Infinite)?;
            for index in 0..frames.length() {
                let bytes = js_sys::Uint8Array::new(&frames.get(index));
                anyhow::ensure!(bytes.length() as usize == width as usize * height as usize * 4, "GIF frame size changed during export");
                let image = RgbaImage::from_raw(width, height, bytes.to_vec()).ok_or_else(|| anyhow::anyhow!("Invalid GIF frame"))?;
                frames.set(index, wasm_bindgen::JsValue::NULL);
                encoder.encode_frame(Frame::from_parts(image, 0, 0, Delay::from_numer_denom_ms(delay_ms, 1)))?;
                let _ = progress.call1(&wasm_bindgen::JsValue::UNDEFINED, &((index + 1) as f64).into());
            }
        }
        Ok(output.into_inner())
    };
    encode().map_err(|error| error.to_string().into())
}

/// Encode `frames` into a looping GIF at `path`, each shown for `delay_ms` milliseconds.
#[cfg(not(target_arch = "wasm32"))]
pub fn encode_gif(frames: &[RgbaImage], delay_ms: u16, path: &Path) -> anyhow::Result<()> {
    encode_gif_timed(frames, &vec![delay_ms as u32; frames.len()], path)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn encode_gif_timed(
    frames: &[RgbaImage],
    delays_ms: &[u32],
    path: &Path,
) -> anyhow::Result<()> {
    validate_gif_frames(frames, delays_ms)?;
    let file = std::fs::File::create(path)?;
    encode_gif_to(frames, delays_ms, std::io::BufWriter::new(file), &ExportControl::default())
}

#[cfg(not(target_arch = "wasm32"))]
fn encode_gif_to<W: std::io::Write>(
    frames: &[RgbaImage],
    delays_ms: &[u32],
    mut writer: W,
    control: &ExportControl,
) -> anyhow::Result<()> {
    validate_gif_frames(frames, delays_ms)?;
    let mut enc = GifEncoder::new_with_speed(&mut writer, 1);
    enc.set_repeat(Repeat::Infinite)?;
    for (index, (img, delay_ms)) in frames.iter().zip(delays_ms).enumerate() {
        control.check()?;
        let delay = Delay::from_numer_denom_ms(*delay_ms, 1);
        enc.encode_frame(Frame::from_parts(img.clone(), 0, 0, delay))?;
        control.progress(index + 1);
    }
    drop(enc);
    writer.flush()?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_gif_frames(frames: &[RgbaImage], delays_ms: &[u32]) -> anyhow::Result<()> {
    if frames.is_empty() {
        anyhow::bail!("no frames captured");
    }
    anyhow::ensure!(
        frames.len() == delays_ms.len(),
        "frame timing count mismatch"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_budget_preserves_small_native_exports_and_bounds_large_ones() {
        assert_eq!(capture_dimensions(1920, 1080, 8, false).unwrap(), (1920, 1080));
        let (width, height) = capture_dimensions(3840, 2160, 60, false).unwrap();
        assert!(width < 3840 && height < 2160);
        validate_capture_bytes(width, height, 60, false).unwrap();
        assert!(capture_dimensions(1920, 1080, 100_000, true).is_err());
        assert!(validate_capture_bytes(u32::MAX, u32::MAX, usize::MAX, false).is_err());
    }

    #[test]
    fn browser_budget_bounds_portrait_frames_and_exact_byte_limit() {
        let dimensions = capture_dimensions(1080, 3840, 4, true).unwrap();
        assert_eq!(dimensions.1, 960);
        assert!(dimensions.0 < 960);
        assert!(validate_capture_bytes(1024, 1024, 16, true).is_ok());
        assert!(validate_capture_bytes(1024, 1024, 17, true).is_err());
        assert!(capture_dimensions(0, 1080, 4, true).is_err());
    }

    #[test]
    fn gif_encoding_reports_completed_frames() {
        let control = ExportControl::default();
        let mut bytes = Vec::new();
        encode_gif_to(&[RgbaImage::new(2, 2), RgbaImage::new(2, 2)], &[100, 100], &mut bytes, &control).unwrap();
        assert_eq!(control.completed(), 2);
        assert_eq!(&bytes[..3], b"GIF");
    }

    #[test]
    fn cancelled_export_preserves_destination_and_removes_staging() {
        let temporary = ExportDirectory::new(&std::env::temp_dir()).unwrap();
        let path = temporary.0.join("loop.gif");
        std::fs::write(&path, b"previous export").unwrap();
        let control = ExportControl::default();
        control.cancel();
        let result = encode_export(&[RgbaImage::new(2, 2)], LoopFormat::Gif, &path,
            &Manifest { site: None, product: "REF", units: "dBZ", tilt: 0, fps: 4.0, center: [0.0, 0.0], zoom: 8.0 }, &[], &control);
        assert_eq!(result.unwrap_err().to_string(), "Loop export cancelled");
        assert_eq!(std::fs::read(path).unwrap(), b"previous export");
        assert_eq!(std::fs::read_dir(&temporary.0).unwrap().count(), 1);
    }

    #[test]
    fn export_staging_directories_are_distinct_and_cleaned() {
        let first = ExportDirectory::new(&std::env::temp_dir()).unwrap();
        let second = ExportDirectory::new(&std::env::temp_dir()).unwrap();
        assert_ne!(first.0, second.0);
        let first_path = first.0.clone();
        drop(first);
        assert!(!first_path.exists());
        assert!(second.0.exists());
    }

    #[test]
    fn failed_manifest_publication_restores_the_previous_clip_and_manifest() {
        let temporary = ExportDirectory::new(&std::env::temp_dir()).unwrap();
        let path = temporary.0.join("loop.gif");
        let manifest = temporary.0.join("loop.gif.json");
        let staged = temporary.0.join("new.gif");
        std::fs::write(&path, b"previous clip").unwrap();
        std::fs::write(&manifest, b"previous manifest").unwrap();
        std::fs::write(&staged, b"new clip").unwrap();
        assert!(publish_export(&staged, &path, &temporary.0.join("missing.json"), &manifest).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"previous clip");
        assert_eq!(std::fs::read(&manifest).unwrap(), b"previous manifest");
        assert_eq!(std::fs::read_dir(&temporary.0).unwrap().count(), 2);
    }

    #[test]
    fn a_directory_at_the_manifest_destination_cannot_replace_the_previous_clip() {
        let temporary = ExportDirectory::new(&std::env::temp_dir()).unwrap();
        let path = temporary.0.join("loop.gif");
        let manifest = temporary.0.join("loop.gif.json");
        let staged = temporary.0.join("new.gif");
        std::fs::write(&path, b"previous clip").unwrap();
        std::fs::write(&staged, b"new clip").unwrap();
        std::fs::create_dir(&manifest).unwrap();
        assert!(publish_export(&staged, &path, &temporary.0.join("new.json"), &manifest).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"previous clip");
        assert!(manifest.is_dir());
    }

    #[test]
    fn encodes_a_multiframe_gif() {
        let mut a = RgbaImage::new(8, 8);
        let mut b = RgbaImage::new(8, 8);
        for p in a.pixels_mut() {
            *p = image::Rgba([255, 0, 0, 255]);
        }
        for p in b.pixels_mut() {
            *p = image::Rgba([0, 0, 255, 255]);
        }
        let dir = std::env::temp_dir().join("hookecho_gif_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loop.gif");
        encode_gif(&[a, b], 100, &path).expect("encode");
        let meta = std::fs::metadata(&path).expect("file written");
        assert!(meta.len() > 0, "gif is non-empty");
        // GIF magic header.
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..3], b"GIF");
    }

    #[test]
    fn empty_frames_error() {
        let path = std::env::temp_dir().join("hookecho_gif_empty.gif");
        assert!(encode_gif(&[], 100, &path).is_err());
        assert!(encode_mp4(&[], 5, &path).is_err());
    }

    #[test]
    fn rejects_mismatched_frame_timing() {
        let path = std::env::temp_dir().join("hookecho_gif_bad_timing.gif");
        assert!(encode_gif_timed(&[RgbaImage::new(1, 1)], &[], &path).is_err());
    }

    #[test]
    fn writes_a_provenance_sidecar() {
        let dir = std::env::temp_dir().join("hookecho_loop_manifest_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loop.gif");
        write_manifest(
            &path,
            &Manifest {
                site: Some("KTLX"),
                product: "REF",
                units: "dBZ",
                tilt: 0,
                fps: 4.0,
                center: [-97.2, 35.3],
                zoom: 8.0,
            },
            &["KTLX20240526_011000_V06".into()],
        )
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("loop.gif.json")).unwrap()).unwrap();
        assert_eq!(value["schema"], "hookecho.loop/v1");
        assert_eq!(
            value["frames"][0]["valid_time"],
            "2024-05-26T01:10:00+00:00"
        );
    }

    #[test]
    fn encodes_mp4_when_ffmpeg_present() {
        // Skip cleanly if ffmpeg isn't installed (CI without it still passes).
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            let mut a = RgbaImage::new(16, 16);
            for p in a.pixels_mut() {
                *p = image::Rgba([200, 40, 40, 255]);
            }
            let path = std::env::temp_dir().join("hookecho_mp4_test.mp4");
            encode_mp4(&[a.clone(), a], 5, &path).expect("mp4 encode");
            let bytes = std::fs::read(&path).expect("mp4 written");
            // MP4 files carry an 'ftyp' box near the start.
            assert!(
                bytes.windows(4).take(64).any(|w| w == b"ftyp"),
                "looks like an MP4"
            );
        }
    }
}
