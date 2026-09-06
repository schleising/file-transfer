//! Shared `AppIcon.png` for the sidebar brand and the macOS About panel.

use std::io::Cursor;
use std::sync::OnceLock;

use image::RgbaImage;

const PNG: &[u8] = include_bytes!("../assets/AppIcon.png");

/// Data-URL for the sidebar mark (downscaled so WKWebView is not holding a 1024² PNG).
pub fn brand_src() -> &'static str {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| {
        let mut png = Vec::new();
        icon_rgba(64)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("encode brand icon");
        format!("data:image/png;base64,{}", encode_base64(&png))
    })
}

pub fn about_icon() -> Option<dioxus::desktop::muda::Icon> {
    let img = icon_rgba(256);
    let (w, h) = img.dimensions();
    dioxus::desktop::muda::Icon::from_rgba(img.into_raw(), w, h).ok()
}

fn icon_rgba(px: u32) -> RgbaImage {
    let img = image::load_from_memory(PNG).expect("AppIcon.png");
    let img = img
        .resize_exact(px, px, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    apply_macos_icon_mask(img)
}

/// Clip to the macOS / iOS app-icon squircle (superellipse, n = 5).
///
/// The source PNG is a full-bleed square; Dock and Launchpad apply this mask.
/// The sidebar and About panel do not, so we bake it into the pixels.
fn apply_macos_icon_mask(mut img: RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let size = w.min(h);
    for y in 0..size {
        for x in 0..size {
            let cover = squircle_coverage(x, y, size);
            let p = img.get_pixel_mut(x, y);
            if cover <= 0.001 {
                *p = image::Rgba([0, 0, 0, 0]);
                continue;
            }
            if cover < 0.999 {
                p.0[3] = (f32::from(p.0[3]) * cover + 0.5) as u8;
            }
        }
    }
    img
}

fn squircle_coverage(px: u32, py: u32, size: u32) -> f32 {
    const N: f32 = 5.0;
    const OFFSETS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
    let s = size as f32;
    let mut hit = 0.0;
    for dy in OFFSETS {
        for dx in OFFSETS {
            let x = (px as f32 + dx).mul_add(2.0 / s, -1.0);
            let y = (py as f32 + dy).mul_add(2.0 / s, -1.0);
            if x.abs().powf(N) + y.abs().powf(N) <= 1.0 {
                hit += 1.0;
            }
        }
    }
    hit / 16.0
}

fn encode_base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let a = u32::from(chunk[0]);
        let b = u32::from(chunk.get(1).copied().unwrap_or(0));
        let c = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (a << 16) | (b << 8) | c;
        out.push(char::from(TABLE[((n >> 18) & 63) as usize]));
        out.push(char::from(TABLE[((n >> 12) & 63) as usize]));
        if chunk.len() > 1 {
            out.push(char::from(TABLE[((n >> 6) & 63) as usize]));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(TABLE[(n & 63) as usize]));
        } else {
            out.push('=');
        }
    }
    out
}
