//! Animated-GIF export of a radar loop. The app captures each timeline frame via the screenshot
//! path (GUI-only), collects them here, and encodes a looping GIF. The encoder is the testable
//! part; the capture state machine lives in [`crate::app`].

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, RgbaImage};
use std::path::Path;

/// Output container for a loop export.
#[derive(Clone, Copy, PartialEq)]
pub enum LoopFormat {
    Gif,
    Mp4,
}

/// Encode `frames` into an MP4 (H.264) at `path` via the `ffmpeg` CLI. Frames are written as
/// PNGs to a temp dir and muxed at `fps`. Errors if ffmpeg is missing or fails.
pub fn encode_mp4(frames: &[RgbaImage], fps: u32, path: &Path) -> anyhow::Result<()> {
    if frames.is_empty() {
        anyhow::bail!("no frames captured");
    }
    // Stage PNGs in a unique temp dir.
    let dir = std::env::temp_dir().join(format!("hookecho_mp4_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    for (i, img) in frames.iter().enumerate() {
        img.save(dir.join(format!("f{i:04}.png")))?;
    }
    // Even dimensions are required by yuv420p; pad if odd.
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-framerate", &fps.to_string(), "-i"])
        .arg(dir.join("f%04d.png"))
        .args([
            "-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-movflags", "+faststart",
        ])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let _ = std::fs::remove_dir_all(&dir);
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => anyhow::bail!("ffmpeg exited {s}"),
        Err(e) => anyhow::bail!("ffmpeg not available ({e}); GIF export works without it"),
    }
}

/// Encode `frames` into a looping GIF at `path`, each shown for `delay_ms` milliseconds.
pub fn encode_gif(frames: &[RgbaImage], delay_ms: u16, path: &Path) -> anyhow::Result<()> {
    if frames.is_empty() {
        anyhow::bail!("no frames captured");
    }
    let file = std::fs::File::create(path)?;
    let mut enc = GifEncoder::new(std::io::BufWriter::new(file));
    enc.set_repeat(Repeat::Infinite)?;
    let delay = Delay::from_numer_denom_ms(delay_ms as u32, 1);
    for img in frames {
        enc.encode_frame(Frame::from_parts(img.clone(), 0, 0, delay))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn encodes_mp4_when_ffmpeg_present() {
        // Skip cleanly if ffmpeg isn't installed (CI without it still passes).
        if std::process::Command::new("ffmpeg").arg("-version").stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
        {
            let mut a = RgbaImage::new(16, 16);
            for p in a.pixels_mut() {
                *p = image::Rgba([200, 40, 40, 255]);
            }
            let path = std::env::temp_dir().join("hookecho_mp4_test.mp4");
            encode_mp4(&[a.clone(), a], 5, &path).expect("mp4 encode");
            let bytes = std::fs::read(&path).expect("mp4 written");
            // MP4 files carry an 'ftyp' box near the start.
            assert!(bytes.windows(4).take(64).any(|w| w == b"ftyp"), "looks like an MP4");
        }
    }
}
