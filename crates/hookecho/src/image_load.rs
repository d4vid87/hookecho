//! Bounded decoding for user images and remote photographs/sprite sheets.
use std::io::{Cursor, Read};

pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_PIXELS: u64 = 8 * 1024 * 1024;
const MAX_SIDE: u32 = 8192;

pub(crate) fn read(path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= MAX_BYTES, "Image exceeds the 8 MiB file limit");
    Ok(bytes)
}

pub(crate) async fn fetch(http: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
    use futures_util::StreamExt;
    let response = http.get(url)
        .timeout(wxdata::net::FEED_TIMEOUT)
        .header("User-Agent", crate::tiles::USER_AGENT)
        .send().await?.error_for_status()?;
    anyhow::ensure!(response.content_length().is_none_or(|n| n <= MAX_BYTES as u64), "Image exceeds the 8 MiB download limit");
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        anyhow::ensure!(bytes.len().saturating_add(chunk.len()) <= MAX_BYTES, "Image exceeds the 8 MiB download limit");
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) fn decode(bytes: &[u8]) -> anyhow::Result<image::RgbaImage> {
    decode_bounded(bytes, MAX_SIDE, MAX_PIXELS, 64 * 1024 * 1024)
}

/// Providers use 256/512-pixel tiles; allow 1024 for custom sources without large GPU uploads.
pub(crate) fn decode_tile(bytes: &[u8]) -> anyhow::Result<image::RgbaImage> {
    decode_bounded(bytes, 1024, 1024 * 1024, 16 * 1024 * 1024)
}

#[inline(never)] // Share the decoder machinery between image and tile limits.
fn decode_bounded(bytes: &[u8], max_side: u32, max_pixels: u64, max_alloc: u64) -> anyhow::Result<image::RgbaImage> {
    use image::ImageDecoder;
    anyhow::ensure!(bytes.len() <= MAX_BYTES, "Image exceeds the 8 MiB file limit");
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_side);
    limits.max_image_height = Some(max_side);
    limits.max_alloc = Some(max_alloc);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder()?;
    let (w, h) = decoder.dimensions();
    anyhow::ensure!(w > 0 && h > 0 && u64::from(w) * u64::from(h) <= max_pixels, "Image exceeds the {max_pixels} pixel limit");
    decoder.set_limits(limits)?;
    Ok(image::DynamicImage::from_decoder(decoder)?.into_rgba8())
}

/// Sprite coordinates must remain intact; photographs can fit a smaller GPU texture.
pub(crate) fn color(mut image: image::RgbaImage, max_side: usize, sprite: bool) -> anyhow::Result<egui::ColorImage> {
    let side = max_side.min(if sprite { MAX_SIDE as usize } else { 2048 }) as u32;
    anyhow::ensure!(side > 0, "Image texture limit is unavailable");
    if image.width() > side || image.height() > side {
        anyhow::ensure!(!sprite, "Icon sheet exceeds the texture dimension limit");
        image = image::DynamicImage::ImageRgba8(image).thumbnail(side, side).into_rgba8();
    }
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize], image.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    #[test]
    fn oversized_image_header_is_rejected_before_pixel_allocation() {
        let png: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x20, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x57, 0xc1, 0x95, 0x85, 0x00, 0x00, 0x00, 0x08, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01, 0x48, 0x06, 0x89, 0xd2, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82];
        assert!(decode(png).unwrap_err().to_string().contains("pixel limit"));
    }

    #[test]
    fn disk_cache_reads_are_bounded_too() {
        let file = std::env::temp_dir().join(format!("hookecho-image-limit-{}", std::process::id()));
        let handle = std::fs::File::create(&file).unwrap();
        handle.set_len(MAX_BYTES as u64 + 1).unwrap();
        assert!(read(&file).is_err());
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn ordinary_images_decode_and_photos_fit_without_distorting_sprites() {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(&[120; 16 * 8 * 4], 16, 8, image::ExtendedColorType::Rgba8).unwrap();
        let image = decode(&png).unwrap();
        assert_eq!(color(image.clone(), 8, false).unwrap().size, [8, 4]);
        assert!(color(image, 8, true).is_err());
        assert!(decode(&vec![0; MAX_BYTES + 1]).is_err());
    }

    #[test]
    fn raster_tiles_accept_provider_sizes_and_reject_oversized_headers() {
        for side in [256, 512, 1024] {
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&vec![120; side as usize * side as usize * 4], side, side, image::ExtendedColorType::Rgba8).unwrap();
            assert_eq!(decode_tile(&png).unwrap().dimensions(), (side, side));
        }
        for format in [image::ImageFormat::Jpeg, image::ImageFormat::WebP] {
            let mut encoded = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(512, 512, image::Rgb([40, 90, 130])))
                .write_to(&mut encoded, format).unwrap();
            assert_eq!(decode_tile(encoded.get_ref()).unwrap().dimensions(), (512, 512));
        }
        for (width, height) in [(8193, 1), (1, 8193), (1025, 1025)] {
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&vec![0; width as usize * height as usize * 4], width, height, image::ExtendedColorType::Rgba8).unwrap();
            assert!(png.len() < MAX_BYTES);
            assert!(decode_tile(&png).is_err(), "{width}x{height} must not reach the GPU");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn downloads_reject_oversized_headers_and_unannounced_bodies() {
        use std::io::Write;
        for chunked in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/image", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_write_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                let _ = socket.read(&mut [0; 4096]);
                if chunked {
                    let _ = write!(socket, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", MAX_BYTES + 1);
                    let _ = socket.write_all(&vec![0; MAX_BYTES + 1]);
                    let _ = socket.write_all(b"\r\n0\r\n\r\n");
                } else {
                    write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_BYTES + 1).unwrap();
                }
            });
            assert!(fetch(&reqwest::Client::new(), &url).await.unwrap_err().to_string().contains("8 MiB"));
            server.join().unwrap();
        }
    }
}
