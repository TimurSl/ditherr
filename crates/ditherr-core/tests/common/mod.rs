//! Shared helpers for engine integration tests.
#![allow(dead_code)]

use ditherr_core::{Rendered, Rgba8Image};

/// A deterministic test picture built with integer maths only: gradients, a
/// colour-banded lower half, a disc, and a transparent corner.
pub fn test_image(w: u32, h: u32) -> Rgba8Image {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    let (cx, cy, r2) = ((w / 2) as i64, (h / 2) as i64, ((h / 3) * (h / 3)) as i64);
    for y in 0..h {
        for x in 0..w {
            let mut px = [
                (x * 255 / (w - 1)) as u8,
                (y * 255 / (h - 1)) as u8,
                (((x * 3 + y * 5) % 64) * 4) as u8,
                255,
            ];
            let (dx, dy) = (i64::from(x) - cx, i64::from(y) - cy);
            if dx * dx + dy * dy < r2 {
                px = [230, 200, 40, 255];
            }
            if x < w / 8 && y < h / 8 {
                px[3] = 0;
            }
            data.extend_from_slice(&px);
        }
    }
    Rgba8Image::new(w, h, data).unwrap()
}

pub fn flat_gray(w: u32, h: u32, v: u8) -> Rgba8Image {
    let mut data = Vec::new();
    for _ in 0..w * h {
        data.extend_from_slice(&[v, v, v, 255]);
    }
    Rgba8Image::new(w, h, data).unwrap()
}

pub fn flip_h(img: &Rgba8Image) -> Rgba8Image {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut data = Vec::with_capacity(img.data.len());
    for y in 0..h {
        for x in (0..w).rev() {
            let i = (y * w + x) * 4;
            data.extend_from_slice(&img.data[i..i + 4]);
        }
    }
    Rgba8Image::new(img.width, img.height, data).unwrap()
}

pub fn transpose(img: &Rgba8Image) -> Rgba8Image {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut data = Vec::with_capacity(img.data.len());
    for x in 0..w {
        for y in 0..h {
            let i = (y * w + x) * 4;
            data.extend_from_slice(&img.data[i..i + 4]);
        }
    }
    Rgba8Image::new(img.height, img.width, data).unwrap()
}

pub fn rgba(r: &Rendered) -> Rgba8Image {
    r.to_rgba8()
}

/// 64-bit FNV-1a.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Hash of a rendered image including its dimensions, palette and indices.
pub fn hash_rendered(r: &Rendered) -> u64 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&r.width().to_le_bytes());
    bytes.extend_from_slice(&r.height().to_le_bytes());
    match r {
        Rendered::Indexed(i) => {
            bytes.push(0);
            for c in &i.palette {
                bytes.extend_from_slice(c);
            }
            bytes.extend_from_slice(&i.indices);
        }
        Rendered::Truecolor(t) => {
            bytes.push(1);
            bytes.extend_from_slice(&t.data);
        }
    }
    fnv1a(&bytes)
}
