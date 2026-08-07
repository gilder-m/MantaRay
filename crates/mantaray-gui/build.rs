//! Embeds the application icon into the Windows executable.
//!
//! The icon is generated here rather than shipped as an asset - the same
//! miniature spectrum the runtime window icon draws: a teal peak over its
//! amber region on the dark plot, in the default palette. On other platforms
//! this build script does nothing; the runtime icon covers the window there.

use std::io::Write;

/// The default palette's colours, kept in step with `theme.rs` by eye - the
/// build script cannot depend on the crate it builds.
const BACKGROUND: [u8; 3] = [11, 16, 32];
const FOREGROUND: [u8; 3] = [64, 224, 208];
const REGION: [u8; 3] = [255, 179, 71];

/// The trace: a Gaussian peak on a sloping continuum, as a fraction of height.
fn height_at(x: usize, size: usize) -> f64 {
    let t = x as f64 / (size - 1) as f64;
    let continuum = 0.28 - 0.12 * t;
    let peak = 0.62 * (-((t - 0.42) / 0.10).powi(2)).exp();
    continuum + peak
}

/// One icon image as BGRA rows, bottom-up, as BMP-in-ICO wants them.
fn image(size: usize) -> Vec<u8> {
    let mut rows = Vec::with_capacity(size * size * 4);
    let region = (size * 10 / 32)..=(size * 17 / 32);
    for row in 0..size {
        // Bottom-up: row 0 is the bottom of the picture.
        let from_bottom = row;
        for x in 0..size {
            let level = (height_at(x, size) * size as f64) as usize;
            let thick = (size / 16).max(1);
            let colour = if from_bottom >= level && from_bottom < level + thick {
                FOREGROUND
            } else if from_bottom < level && region.contains(&x) {
                REGION
            } else if from_bottom < level {
                [BACKGROUND[0] + 18, BACKGROUND[1] + 22, BACKGROUND[2] + 26]
            } else {
                BACKGROUND
            };
            rows.extend_from_slice(&[colour[2], colour[1], colour[0], 255]);
        }
    }
    rows
}

/// A one-image `.ico` file: ICONDIR, one ICONDIRENTRY, and a BMP payload
/// (BITMAPINFOHEADER with doubled height, BGRA rows, then an empty AND mask).
fn ico(size: usize) -> Vec<u8> {
    let pixels = image(size);
    let mask_stride = size.div_ceil(32) * 4;
    let mask = vec![0u8; mask_stride * size];
    let bmp_size = 40 + pixels.len() + mask.len();

    let mut out = Vec::with_capacity(22 + bmp_size);
    // ICONDIR: reserved, type 1 (icon), one image.
    out.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    // ICONDIRENTRY: width, height, colours, reserved, planes, bpp, size, offset.
    out.push(if size >= 256 { 0 } else { size as u8 });
    out.push(if size >= 256 { 0 } else { size as u8 });
    out.extend_from_slice(&[0, 0, 1, 0, 32, 0]);
    out.extend_from_slice(&(bmp_size as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes());
    // BITMAPINFOHEADER, height doubled for the AND mask.
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]); // compression, sizes, resolutions, colours
    out.extend_from_slice(&pixels);
    out.extend_from_slice(&mask);
    out
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let path = std::path::Path::new(&out_dir).join("mantaray.ico");
    let mut file = std::fs::File::create(&path).expect("write the icon");
    file.write_all(&ico(32)).expect("icon bytes");
    drop(file);
    winresource::WindowsResource::new()
        .set_icon(path.to_str().expect("utf-8 path"))
        .compile()
        .expect("embed the icon");
}
