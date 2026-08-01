//! Listen to a NOAA Weather Radio relay while you watch the radar.
//!
//! NOAA broadcasts NWR over VHF and publishes no stream of its own, so the sources here are
//! third-party relays (Icecast/Broadcastify-style MP3 URLs) that people run for their own county.
//! That is the honest shape of this feature: you bring the URL for your area, and a relay that is
//! down stays down — all this can do is say so and keep retrying.
//!
//! Why not just hand the URL to `rodio::Decoder`: it wants `Read + Seek`, and a live stream has no
//! seek. So the bytes arrive over a channel, symphonia (already in the tree under rodio's `mp3`
//! feature) demuxes and decodes them, and the decoded packets are appended to an ordinary
//! [`rodio::Sink`] — the same sink machinery the alert sounds already use on desktop and Android.

use std::io::{Read, Result as IoResult, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the player is doing, for the one line of UI that shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Stopped,
    Connecting,
    Playing,
    /// Off the air (or the URL is wrong); the player keeps retrying with a backoff.
    Offline(String),
}

/// A running (or starting, or retrying) stream. Dropping it stops playback.
pub struct Player {
    /// Which configured stream is playing, by name.
    pub name: String,
    status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Player {
    /// Start playing `url`, reconnecting on its own until dropped or [`Player::stop`] is called.
    pub fn start(name: String, url: String, volume: f32, rt: &tokio::runtime::Handle) -> Player {
        let status = Arc::new(Mutex::new(Status::Connecting));
        let stop = Arc::new(AtomicBool::new(false));
        let (s2, st2, rt2) = (status.clone(), stop.clone(), rt.clone());
        std::thread::spawn(move || run(url, volume, s2, st2, rt2));
        Player { name, status, stop }
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(Status::Stopped)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Connect, play, and reconnect with a backoff until told to stop. Relay uptime is what it is;
/// the loop is the whole reliability story.
fn run(
    url: String,
    volume: f32,
    status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
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
        match play_once(&url, volume, &status, &stop, &rt) {
            // A clean end of stream is still the relay going away; reconnect like any other drop.
            Ok(()) if stop.load(Ordering::Relaxed) => break,
            Ok(()) => set(Status::Offline("stream ended".into())),
            Err(e) => {
                log::warn!("NWR stream {url}: {e}");
                set(Status::Offline(e.to_string()));
            }
        }
        // Sleep in slices so a stop request doesn't wait out the whole backoff.
        for _ in 0..backoff * 4 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        backoff = (backoff * 2).min(60);
    }
    set(Status::Stopped);
}

/// How many decoded packets may sit in the sink before the decoder waits. Each is ~26 ms of audio,
/// so this is roughly a second of buffer — enough to ride out a hiccup, short enough that stopping
/// is instant.
const SINK_BUFFER_PACKETS: usize = 40;
/// Bounded so a fast relay (or a stalled decoder) can't grow the download into memory.
const CHUNK_QUEUE: usize = 64;

fn play_once(
    url: &str,
    volume: f32,
    status: &Arc<Mutex<Status>>,
    stop: &Arc<AtomicBool>,
    rt: &tokio::runtime::Handle,
) -> anyhow::Result<()> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(CHUNK_QUEUE);
    let (url_owned, stop_dl) = (url.to_string(), stop.clone());
    // The download runs on the app's existing tokio runtime; the decode blocks on this thread.
    rt.spawn(async move { download(url_owned, tx, stop_dl).await });

    let (_stream, handle) = rodio::OutputStream::try_default()?;
    let sink = rodio::Sink::try_new(&handle)?;
    sink.set_volume(volume.clamp(0.0, 1.0));

    let source = symphonia::core::io::MediaSourceStream::new(
        Box::new(StreamSource {
            rx: Mutex::new(rx),
            buf: Vec::new(),
            pos: 0,
        }),
        Default::default(),
    );
    let mut hint = symphonia::core::probe::Hint::new();
    hint.mime_type("audio/mpeg");
    let probed = symphonia::default::get_probe().format(
        &hint,
        source,
        &Default::default(),
        &Default::default(),
    )?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow::anyhow!("stream has no audio track"))?;
    let track_id = track.id;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &Default::default())?;

    if let Ok(mut g) = status.lock() {
        *g = Status::Playing;
    }
    while !stop.load(Ordering::Relaxed) {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // End of stream (or the relay hung up) — let the caller reconnect.
            Err(symphonia::core::errors::Error::IoError(_)) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A corrupt packet mid-stream is normal on a lossy relay; skip it, keep playing.
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        let spec = *decoded.spec();
        let mut samples =
            symphonia::core::audio::SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        samples.copy_interleaved_ref(decoded);
        sink.append(rodio::buffer::SamplesBuffer::new(
            spec.channels.count() as u16,
            spec.rate,
            samples.samples(),
        ));
        // Let the sink drain rather than queueing the whole broadcast into memory.
        while sink.len() > SINK_BUFFER_PACKETS && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    sink.stop();
    Ok(())
}

/// Pump the HTTP body into the channel until it ends or playback stops.
async fn download(url: String, tx: SyncSender<Vec<u8>>, stop: Arc<AtomicBool>) {
    use futures_util::StreamExt;
    let client = match reqwest::Client::builder()
        .user_agent(wxdata::alerts::USER_AGENT)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("NWR client: {e}");
            return;
        }
    };
    // Icecast wants to be asked before it will send its metadata-free stream to some clients; we
    // don't ask for metadata, so the body is plain MP3 frames.
    let resp = match client
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("NWR connect {url}: {e}");
            return;
        }
    };
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        match chunk {
            // A full queue means the decoder is behind; blocking here is the backpressure.
            Ok(b) => {
                if tx.send(b.to_vec()).is_err() {
                    return; // decoder gone
                }
            }
            Err(e) => {
                log::warn!("NWR read {url}: {e}");
                return;
            }
        }
    }
}

/// A `MediaSource` over the download channel: readable, never seekable, unknown length — which is
/// exactly what symphonia expects from a live stream.
struct StreamSource {
    // `MediaSource` demands `Send + Sync`; a `Receiver` is `Send` but not `Sync`, and only this
    // thread ever touches it, so the mutex is a formality rather than contention.
    rx: Mutex<Receiver<Vec<u8>>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for StreamSource {
    fn read(&mut self, out: &mut [u8]) -> IoResult<usize> {
        while self.pos >= self.buf.len() {
            let next = self
                .rx
                .lock()
                .map_err(|_| std::io::Error::other("stream channel poisoned"))?
                .recv();
            match next {
                Ok(next) => {
                    self.buf = next;
                    self.pos = 0;
                }
                // The downloader finished or died: EOF, which symphonia reports as an IO error and
                // the reconnect loop treats as "relay went away".
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Seek for StreamSource {
    fn seek(&mut self, _: SeekFrom) -> IoResult<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "live stream is not seekable",
        ))
    }
}

impl symphonia::core::io::MediaSource for StreamSource {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::io::MediaSource;

    /// End-to-end against a real Icecast MP3 stream (not an NWR relay — this only proves the
    /// download → symphonia → sink path works on a live, unseekable stream).
    #[test]
    #[ignore = "network + audio device"]
    fn plays_a_live_mp3_stream() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let p = Player::start(
            "SomaFM".into(),
            "https://ice1.somafm.com/groovesalad-128-mp3".into(),
            0.2,
            rt.handle(),
        );
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(250));
            if p.status() == Status::Playing {
                break;
            }
        }
        let s = p.status();
        p.stop();
        assert_eq!(s, Status::Playing, "never reached Playing");
    }

    /// A dead relay must say so and keep retrying, not sit silently on "connecting".
    #[test]
    #[ignore = "network + audio device"]
    fn dead_relay_reports_offline() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let p = Player::start(
            "nowhere".into(),
            "https://relay.invalid/stream.mp3".into(),
            0.0,
            rt.handle(),
        );
        let mut seen = Status::Stopped;
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(250));
            seen = p.status();
            if matches!(seen, Status::Offline(_)) {
                break;
            }
        }
        p.stop();
        assert!(matches!(seen, Status::Offline(_)), "got {seen:?}");
    }

    /// The channel reader has to stitch chunk boundaries back together, or every frame that
    /// straddles two HTTP chunks decodes as garbage.
    #[test]
    fn stream_source_reads_across_chunk_boundaries() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        tx.send(vec![1, 2, 3]).unwrap();
        tx.send(vec![4, 5]).unwrap();
        drop(tx);
        let mut s = StreamSource {
            rx: Mutex::new(rx),
            buf: Vec::new(),
            pos: 0,
        };
        let mut out = [0u8; 4];
        assert_eq!(s.read(&mut out).unwrap(), 3);
        assert_eq!(&out[..3], &[1, 2, 3]);
        assert_eq!(s.read(&mut out).unwrap(), 2);
        assert_eq!(&out[..2], &[4, 5]);
        // Sender gone → EOF, not an error.
        assert_eq!(s.read(&mut out).unwrap(), 0);
        assert!(!s.is_seekable() && s.byte_len().is_none());
    }
}
