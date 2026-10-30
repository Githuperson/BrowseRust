//! Decoding theme images into RGBA frames with hard size limits.

use image::{AnimationDecoder, ImageDecoder, ImageFormat, Limits};
use std::io::Cursor;

pub struct Frame {
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}

pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// One frame for stills; several for animated GIFs.
    pub frames: Vec<Frame>,
}

const MAX_SOURCE_DIM: u32 = 8192;
const MAX_FRAMES: usize = 64;

/// Detect an image by its magic bytes. SVG (and anything else) is refused: an SVG can
/// carry script and external references.
pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    match bytes {
        [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => Some(ImageFormat::Png),
        [0xFF, 0xD8, 0xFF, ..] => Some(ImageFormat::Jpeg),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some(ImageFormat::Gif),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(ImageFormat::WebP),
        _ => None,
    }
}

fn fit(w: u32, h: u32, max_dim: u32) -> (u32, u32) {
    if w <= max_dim && h <= max_dim {
        return (w.max(1), h.max(1));
    }
    let s = max_dim as f32 / w.max(h) as f32;
    (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1))
}

/// Decode `bytes`, scaling so neither side exceeds `max_dim`. Animated GIFs keep up to
/// 64 frames. Fails (never panics) on corrupt data or images whose declared size is
/// above 8192x8192.
pub fn decode_image(bytes: &[u8], max_dim: u32) -> Result<Decoded, String> {
    let fmt = sniff(bytes).ok_or("not a PNG, JPEG, GIF or WebP image")?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_DIM);
    limits.max_image_height = Some(MAX_SOURCE_DIM);
    limits.max_alloc = Some(256 * 1024 * 1024);

    let resize = |img: image::RgbaImage| -> image::RgbaImage {
        let (w, h) = fit(img.width(), img.height(), max_dim);
        if (w, h) == (img.width(), img.height()) { img } else { image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle) }
    };

    if fmt == ImageFormat::Gif {
        let mut dec = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        dec.set_limits(limits).map_err(|e| e.to_string())?;
        let (sw, sh) = dec.dimensions();
        let (w, h) = fit(sw, sh, max_dim);
        let mut frames = Vec::new();
        for f in dec.into_frames().take(MAX_FRAMES) {
            let f = f.map_err(|e| e.to_string())?;
            let (n, d) = f.delay().numer_denom_ms();
            let delay_ms = if d == 0 { 100 } else { (n / d).clamp(20, 10_000) };
            frames.push(Frame { rgba: resize(f.into_buffer()).into_raw(), delay_ms });
        }
        if frames.is_empty() {
            return Err("GIF has no frames".into());
        }
        return Ok(Decoded { width: w, height: h, frames });
    }

    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), fmt);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    let img = resize(img);
    Ok(Decoded { width: img.width(), height: img.height(), frames: vec![Frame { rgba: img.into_raw(), delay_ms: 0 }] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
        out
    }

    fn gif(frames: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
            for i in 0..frames {
                let img = image::RgbaImage::from_pixel(8, 4, image::Rgba([(i * 40) as u8, 0, 0, 255]));
                let f = image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(80, 1));
                enc.encode_frame(f).unwrap();
            }
        }
        out
    }

    #[test]
    fn decodes_and_downscales_stills() {
        let d = decode_image(&png(400, 200), 100).unwrap();
        assert_eq!((d.width, d.height), (100, 50));
        assert_eq!(d.frames.len(), 1);
        assert_eq!(d.frames[0].rgba.len(), 100 * 50 * 4);
        assert_eq!(&d.frames[0].rgba[..4], &[10, 20, 30, 255]);
        let small = decode_image(&png(10, 10), 100).unwrap();
        assert_eq!((small.width, small.height), (10, 10), "never upscaled");
    }

    #[test]
    fn keeps_animation_frames_and_delays() {
        let d = decode_image(&gif(5), 64).unwrap();
        assert_eq!(d.frames.len(), 5);
        assert_eq!((d.width, d.height), (8, 4));
        assert!(d.frames.iter().all(|f| f.delay_ms == 80 && f.rgba.len() == 8 * 4 * 4));
    }

    #[test]
    fn refuses_svg_html_and_garbage() {
        for bad in [b"<svg xmlns='http://www.w3.org/2000/svg'><script>1</script></svg>".as_slice(), b"<html>", b"", b"\x89PNG", b"GIF89"] {
            assert!(decode_image(bad, 100).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn truncated_images_fail_cleanly() {
        let p = png(50, 50);
        assert!(decode_image(&p[..p.len() / 2], 100).is_err());
    }

    #[test]
    fn sniffing_is_by_content_not_name() {
        assert_eq!(sniff(&png(1, 1)), Some(ImageFormat::Png));
        assert_eq!(sniff(b"GIF89a...."), Some(ImageFormat::Gif));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageFormat::WebP));
        assert_eq!(sniff(b"<svg"), None);
    }
}
