//! Live camera video for the station cards.
//!
//! Two transports, chosen from the URL, because that is what public cameras actually serve:
//!
//! * **MJPEG** — a `multipart/x-mixed-replace` body that never ends, each part a whole JPEG. It
//!   decodes with what the app already carries (`reqwest` streaming plus the `image` crate), so
//!   this path has no dependency cost and no external program.
//! * **HLS** (`.m3u8`) — segmented H.264, which needs a real video decoder. Rather than link one
//!   in (a large native dependency, awkward on Windows, with licensing to explain), we shell out
//!   to an `ffmpeg` binary if the user has one and say so plainly if they do not. Everything else
//!   on the card keeps working either way.
//!
//! Frames land in a mutex behind a sequence number; the UI takes the newest one each frame and
//! uploads it as a texture. Nothing here touches egui state, so a stalled camera cannot stall the
//! map.

use egui::ColorImage;
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the player is doing, for the one line of status on the card.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Connecting,
    Playing,
    /// The camera is unreachable or hung up; the player keeps retrying with a backoff.
    Offline(String),
    /// HLS was asked for and no `ffmpeg` is installed. Retrying will not help, so the player stops
    /// and the card explains what to install.
    NeedsFfmpeg,
    Stopped,
}

/// Which decoder a URL calls for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Mjpeg,
    Hls,
}

/// Pick a transport from the URL. Anything that is not an HLS playlist is tried as MJPEG, which
/// is also the right guess for the `.cgi` and `/mjpg/video` paths cameras like to use.
pub fn transport_for(url: &str) -> Transport {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    if path.to_ascii_lowercase().ends_with(".m3u8") {
        Transport::Hls
    } else {
        Transport::Mjpeg
    }
}

/// HLS frames are letterboxed into this fixed size, so the raw pipe has a frame length we can
/// count on without parsing ffmpeg's output.
const HLS_W: usize = 640;
const HLS_H: usize = 360;

/// A single JPEG this large is a camera misbehaving (or a body that never contained a frame);
/// dropping the buffer is better than growing it forever.
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Frames waiting to be decoded. Small on purpose: a slow decoder should drop frames, not queue
/// seconds of stale video.
const FRAME_QUEUE: usize = 2;

/// A running camera stream. Dropping it stops the download, the decoder and any ffmpeg child.
pub struct Stream {
    pub url: String,
    pub transport: Transport,
    status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    frame: Arc<Mutex<Option<ColorImage>>>,
    seq: Arc<AtomicU64>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Stream {
    /// Start playing `url`, reconnecting on its own until dropped.
    pub fn start(url: String, rt: &tokio::runtime::Handle) -> Stream {
        let transport = transport_for(&url);
        let status = Arc::new(Mutex::new(Status::Connecting));
        let stop = Arc::new(AtomicBool::new(false));
        let frame = Arc::new(Mutex::new(None));
        let seq = Arc::new(AtomicU64::new(0));
        let (u, s, st, f, q, rt2) = (
            url.clone(),
            status.clone(),
            stop.clone(),
            frame.clone(),
            seq.clone(),
            rt.clone(),
        );
        std::thread::spawn(move || run(u, transport, s, st, f, q, rt2));
        Stream {
            url,
            transport,
            status,
            stop,
            frame,
            seq,
        }
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(Status::Stopped)
    }

    /// How many frames have arrived. The UI compares this against what it last uploaded, so it
    /// only touches the GPU when there is something new.
    pub fn seq(&self) -> u64 {
        self.seq.load(Ordering::Relaxed)
    }

    /// Take the newest frame, leaving the slot empty. `None` when nothing new has arrived.
    pub fn take_frame(&self) -> Option<ColorImage> {
        self.frame.lock().ok()?.take()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Connect, play, reconnect. The retry loop is the whole reliability story for a public camera.
#[allow(clippy::too_many_arguments)]
fn run(
    url: String,
    transport: Transport,
    status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    frame: Arc<Mutex<Option<ColorImage>>>,
    seq: Arc<AtomicU64>,
    rt: tokio::runtime::Handle,
) {
    let set = |s: Status| {
        if let Ok(mut g) = status.lock() {
            *g = s;
        }
    };
    let mut backoff = 2u64;
    while !stop.load(Ordering::Relaxed) {
        set(Status::Connecting);
        let played = match transport {
            Transport::Mjpeg => play_mjpeg(&url, &status, &stop, &frame, &seq, &rt),
            Transport::Hls => play_hls(&url, &status, &stop, &frame, &seq),
        };
        match played {
            Ok(()) if stop.load(Ordering::Relaxed) => break,
            Ok(()) => set(Status::Offline("stream ended".into())),
            Err(e) if is_missing_ffmpeg(&e) => {
                set(Status::NeedsFfmpeg);
                return;
            }
            Err(e) => {
                log::warn!("camera {url}: {e}");
                set(Status::Offline(e.to_string()));
            }
        }
        for _ in 0..backoff * 4 {
            if stop.load(Ordering::Relaxed) {
                set(Status::Stopped);
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        backoff = (backoff * 2).min(60);
    }
    set(Status::Stopped);
}

fn is_missing_ffmpeg(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
}

/// Publish a decoded frame, replacing whatever the UI has not picked up yet.
fn publish(frame: &Arc<Mutex<Option<ColorImage>>>, seq: &Arc<AtomicU64>, img: ColorImage) {
    if let Ok(mut g) = frame.lock() {
        *g = Some(img);
    }
    seq.fetch_add(1, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------------------------
// MJPEG
// ---------------------------------------------------------------------------------------------

/// Stream an MJPEG body: the download runs on the app's tokio runtime and hands whole JPEGs over
/// a bounded channel; this thread decodes them.
fn play_mjpeg(
    url: &str,
    status: &Arc<Mutex<Status>>,
    stop: &Arc<AtomicBool>,
    frame: &Arc<Mutex<Option<ColorImage>>>,
    seq: &Arc<AtomicU64>,
    rt: &tokio::runtime::Handle,
) -> anyhow::Result<()> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(FRAME_QUEUE);
    let (u, s) = (url.to_string(), stop.clone());
    let dl = rt.spawn(async move { download_mjpeg(u, tx, s).await });

    let mut first = true;
    while !stop.load(Ordering::Relaxed) {
        let jpeg = match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(j) => j,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            // The downloader finished or failed; surface whichever it was.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg) {
            Ok(img) => {
                if first {
                    if let Ok(mut g) = status.lock() {
                        *g = Status::Playing;
                    }
                    first = false;
                }
                publish(frame, seq, to_color_image(&img.to_rgba8()));
            }
            // A torn frame is routine on a camera behind a bad link; skip it and keep going.
            Err(e) => log::debug!("mjpeg frame from {url}: {e}"),
        }
    }
    // Wait for the downloader's own verdict so a 404 is reported as a 404, not "stream ended".
    match rt.block_on(dl) {
        Ok(r) => r,
        Err(e) => Err(anyhow::anyhow!("download task: {e}")),
    }
}

fn to_color_image(rgba: &image::RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    )
}

/// Read the multipart body and emit each complete JPEG. Rather than trust the boundary marker —
/// cameras get it wrong, and some serve a bare stream of JPEGs with no multipart framing at all —
/// this scans for the JPEG start and end markers, which is what every one of them really sends.
async fn download_mjpeg(
    url: String,
    tx: SyncSender<Vec<u8>>,
    stop: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    use futures_util::StreamExt;

    // A fresh client: an MJPEG body never ends, so it must not inherit a request timeout.
    let resp = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", wxdata::alerts::USER_AGENT)
        .send()
        .await?
        .error_for_status()?;
    let mut body = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();

    while let Some(chunk) = body.next().await {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        buf.extend_from_slice(&chunk?);
        while let Some((start, end)) = find_jpeg(&buf) {
            let jpeg = buf[start..end].to_vec();
            buf.drain(..end);
            // A full queue means the decoder is behind: drop the frame rather than stall the
            // socket and fall further behind.
            if tx.try_send(jpeg).is_err() && stop.load(Ordering::Relaxed) {
                return Ok(());
            }
        }
        if buf.len() > MAX_FRAME_BYTES {
            buf.clear();
        }
    }
    Ok(())
}

/// Byte range of the first complete JPEG in `buf` (`SOI` through `EOI`, end exclusive).
fn find_jpeg(buf: &[u8]) -> Option<(usize, usize)> {
    let start = buf.windows(2).position(|w| w == [0xFF, 0xD8])?;
    let end = buf[start + 2..]
        .windows(2)
        .position(|w| w == [0xFF, 0xD9])
        .map(|p| start + 2 + p + 2)?;
    Some((start, end))
}

// ---------------------------------------------------------------------------------------------
// HLS, via an external ffmpeg
// ---------------------------------------------------------------------------------------------

/// The ffmpeg invocation, split out so the "how do I test this by hand" answer is one line.
fn ffmpeg_args(url: &str) -> Vec<String> {
    [
        "-loglevel", "error",
        "-nostdin",
        // Live playlists: start near the live edge rather than replaying the whole window.
        "-fflags", "nobuffer",
        "-i", url,
        "-an",
        "-vf",
        &format!(
            "scale={HLS_W}:{HLS_H}:force_original_aspect_ratio=decrease,\
             pad={HLS_W}:{HLS_H}:(ow-iw)/2:(oh-ih)/2"
        ),
        "-f", "rawvideo",
        "-pix_fmt", "rgba",
        "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Pipe an HLS stream through ffmpeg as raw RGBA. Every frame is exactly `HLS_W * HLS_H * 4`
/// bytes, so the reader needs no parsing at all.
fn play_hls(
    url: &str,
    status: &Arc<Mutex<Status>>,
    stop: &Arc<AtomicBool>,
    frame: &Arc<Mutex<Option<ColorImage>>>,
    seq: &Arc<AtomicU64>,
) -> anyhow::Result<()> {
    let mut ffmpeg = std::process::Command::new("ffmpeg");
    crate::platform::no_window(&mut ffmpeg);
    let mut child = ffmpeg
        .args(ffmpeg_args(url))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn()?;
    let mut out = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("ffmpeg gave no output pipe"))?;
    // Taking stdout hands the child itself to a guard, so every exit path below kills it.
    let _guard = ChildGuard(child);

    let mut buf = vec![0u8; HLS_W * HLS_H * 4];
    let mut first = true;
    while !stop.load(Ordering::Relaxed) {
        match out.read_exact(&mut buf) {
            Ok(()) => {}
            // ffmpeg exited (bad playlist, network drop) — let the caller reconnect.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        if first {
            if let Ok(mut g) = status.lock() {
                *g = Status::Playing;
            }
            first = false;
        }
        publish(
            frame,
            seq,
            ColorImage::from_rgba_unmultiplied([HLS_W, HLS_H], &buf),
        );
    }
    Ok(())
}

/// Kills the ffmpeg child whenever the player leaves — cleanly, on error, or because the card was
/// closed. Without this, a closed card leaves ffmpeg pulling video forever.
struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_comes_from_the_url() {
        assert_eq!(transport_for("https://x/live/stream.m3u8"), Transport::Hls);
        // A query string must not hide the extension.
        assert_eq!(
            transport_for("https://x/playlist.m3u8?token=abc"),
            Transport::Hls
        );
        assert_eq!(
            transport_for("http://cam.local/mjpg/video.cgi"),
            Transport::Mjpeg
        );
        assert_eq!(transport_for("http://cam.local/"), Transport::Mjpeg);
    }

    #[test]
    fn finds_whole_jpegs_and_leaves_partial_ones() {
        // Two frames back to back, with multipart noise between them.
        let mut buf = Vec::new();
        buf.extend_from_slice(b"--boundary\r\nContent-Type: image/jpeg\r\n\r\n");
        buf.extend_from_slice(&[0xFF, 0xD8, 1, 2, 3, 0xFF, 0xD9]);
        buf.extend_from_slice(b"\r\n--boundary\r\n\r\n");
        buf.extend_from_slice(&[0xFF, 0xD8, 4, 5]);

        let (s, e) = find_jpeg(&buf).unwrap();
        assert_eq!(&buf[s..e], &[0xFF, 0xD8, 1, 2, 3, 0xFF, 0xD9]);
        // After consuming it, the trailing partial frame is not yet a frame.
        let rest = buf.split_off(e);
        assert_eq!(find_jpeg(&rest), None);
        // A body with no start marker yields nothing rather than panicking.
        assert_eq!(find_jpeg(b"HTTP/1.0 404 Not Found"), None);
    }

    /// Play a real stream for a few seconds and assert frames actually arrive. Needs the network
    /// (and, for HLS, an installed ffmpeg), so it is run by hand:
    /// `HOOKECHO_CAM_URL=<url> cargo test -p hookecho --lib cam -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_stream_delivers_frames() {
        let url = std::env::var("HOOKECHO_CAM_URL")
            .expect("set HOOKECHO_CAM_URL to an MJPEG or .m3u8 URL");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let stream = Stream::start(url, rt.handle());
        let mut last = None;
        for _ in 0..60 {
            std::thread::sleep(Duration::from_millis(500));
            if let Some(img) = stream.take_frame() {
                println!("{:?} frame {:?} seq {}", stream.transport, img.size, stream.seq());
                last = Some(img);
                if stream.seq() >= 3 {
                    break;
                }
            }
        }
        let img = last.unwrap_or_else(|| panic!("no frames; status {:?}", stream.status()));
        assert!(img.size[0] > 0 && img.size[1] > 0);
        assert_eq!(stream.status(), Status::Playing);
    }

    #[test]
    fn ffmpeg_command_is_a_raw_rgba_pipe() {
        let args = ffmpeg_args("https://x/live.m3u8");
        assert!(args.contains(&"rawvideo".to_string()));
        assert!(args.contains(&"rgba".to_string()));
        assert!(args.last().unwrap() == "-", "must write to stdout");
        // The scale filter must produce exactly the frame size the reader counts on.
        let vf = args.iter().find(|a| a.starts_with("scale=")).unwrap();
        assert!(vf.contains(&format!("pad={HLS_W}:{HLS_H}")));
    }
}
