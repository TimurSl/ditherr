//! Pixel buffers.

use crate::color::{linear_to_srgb8, srgb8_to_linear};
use crate::error::{Error, Result};

/// 8-bit sRGB RGBA image with straight (non-premultiplied) alpha. This is the
/// engine's input type; decoding is done elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba8Image {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major.
    pub data: Vec<u8>,
}

impl Rgba8Image {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::EmptyImage);
        }
        if data.len() != width as usize * height as usize * 4 {
            return Err(Error::InvalidParam(format!(
                "pixel data has {} bytes, expected {}",
                data.len(),
                width as usize * height as usize * 4
            )));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// Nearest-neighbour integer upscale.
    pub fn upscale(&self, factor: u32) -> Self {
        let pixels: &[[u8; 4]] = as_pixels(&self.data);
        let out = upscale_nearest(pixels, self.width, self.height, factor);
        Self {
            width: self.width * factor,
            height: self.height * factor,
            data: out.into_iter().flatten().collect(),
        }
    }
}

fn as_pixels(data: &[u8]) -> &[[u8; 4]] {
    // Length is always a multiple of 4 for a validated image.
    let (body, rest) = data.as_chunks::<4>();
    debug_assert!(rest.is_empty());
    body
}

/// Linear-light `f32` RGBA working image. Alpha is straight.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageF32 {
    pub width: u32,
    pub height: u32,
    pub data: Vec<[f32; 4]>,
}

impl ImageF32 {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![[0.0; 4]; width as usize * height as usize],
        }
    }

    pub fn from_rgba8(src: &Rgba8Image) -> Self {
        let data = as_pixels(&src.data)
            .iter()
            .map(|p| {
                [
                    srgb8_to_linear(p[0]),
                    srgb8_to_linear(p[1]),
                    srgb8_to_linear(p[2]),
                    f32::from(p[3]) / 255.0,
                ]
            })
            .collect();
        Self {
            width: src.width,
            height: src.height,
            data,
        }
    }

    pub fn to_rgba8(&self) -> Rgba8Image {
        let mut data = Vec::with_capacity(self.data.len() * 4);
        for p in &self.data {
            data.push(linear_to_srgb8(p[0]));
            data.push(linear_to_srgb8(p[1]));
            data.push(linear_to_srgb8(p[2]));
            data.push((p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
        Rgba8Image {
            width: self.width,
            height: self.height,
            data,
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub(crate) fn premultiply(&mut self) {
        for p in &mut self.data {
            p[0] *= p[3];
            p[1] *= p[3];
            p[2] *= p[3];
        }
    }

    pub(crate) fn unpremultiply(&mut self) {
        for p in &mut self.data {
            if p[3] > 1e-6 {
                p[0] /= p[3];
                p[1] /= p[3];
                p[2] /= p[3];
            } else {
                p[0] = 0.0;
                p[1] = 0.0;
                p[2] = 0.0;
            }
        }
    }
}

/// Result of quantization when the palette fits in 8 bits: one palette index
/// per pixel. Keeping the image indexed until export is what makes 1/2/4/8-bit
/// PNG and GIF output exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    pub indices: Vec<u8>,
    pub palette: Vec<[u8; 3]>,
}

impl IndexedImage {
    /// Smallest PNG-style bit depth (1, 2, 4 or 8) that can hold the palette.
    pub fn bit_depth(&self) -> u8 {
        match self.palette.len() {
            0..=2 => 1,
            3..=4 => 2,
            5..=16 => 4,
            _ => 8,
        }
    }

    pub fn upscale(&self, factor: u32) -> Self {
        Self {
            width: self.width * factor,
            height: self.height * factor,
            indices: upscale_nearest(&self.indices, self.width, self.height, factor),
            palette: self.palette.clone(),
        }
    }

    pub fn to_rgba8(&self) -> Rgba8Image {
        let mut data = Vec::with_capacity(self.indices.len() * 4);
        for &i in &self.indices {
            let c = self.palette[usize::from(i)];
            data.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        Rgba8Image {
            width: self.width,
            height: self.height,
            data,
        }
    }
}

/// What a render produces. Palettes of up to 256 colours stay indexed;
/// larger ones (for example RGB565) come out as truecolour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rendered {
    Indexed(IndexedImage),
    Truecolor(Rgba8Image),
}

impl Rendered {
    pub fn width(&self) -> u32 {
        match self {
            Rendered::Indexed(i) => i.width,
            Rendered::Truecolor(i) => i.width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Rendered::Indexed(i) => i.height,
            Rendered::Truecolor(i) => i.height,
        }
    }

    pub fn upscale(&self, factor: u32) -> Self {
        if factor <= 1 {
            return self.clone();
        }
        match self {
            Rendered::Indexed(i) => Rendered::Indexed(i.upscale(factor)),
            Rendered::Truecolor(i) => Rendered::Truecolor(i.upscale(factor)),
        }
    }

    pub fn to_rgba8(&self) -> Rgba8Image {
        match self {
            Rendered::Indexed(i) => i.to_rgba8(),
            Rendered::Truecolor(i) => i.clone(),
        }
    }
}

fn upscale_nearest<T: Copy>(src: &[T], width: u32, height: u32, factor: u32) -> Vec<T> {
    let (w, h, f) = (width as usize, height as usize, factor.max(1) as usize);
    let mut out = Vec::with_capacity(w * h * f * f);
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let start = out.len();
        for &p in row {
            out.extend(std::iter::repeat_n(p, f));
        }
        let scaled_row: Vec<T> = out[start..].to_vec();
        for _ in 1..f {
            out.extend_from_slice(&scaled_row);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upscale_repeats_pixels() {
        let img = IndexedImage {
            width: 2,
            height: 1,
            indices: vec![0, 1],
            palette: vec![[0, 0, 0], [255, 255, 255]],
        };
        let up = img.upscale(2);
        assert_eq!((up.width, up.height), (4, 2));
        assert_eq!(up.indices, vec![0, 0, 1, 1, 0, 0, 1, 1]);
    }

    #[test]
    fn rgba8_roundtrip_through_linear() {
        let data: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 2, 200]).collect();
        let img = Rgba8Image::new(16, 16, data).unwrap();
        assert_eq!(ImageF32::from_rgba8(&img).to_rgba8(), img);
    }

    #[test]
    fn bit_depth_thresholds() {
        let mk = |n| IndexedImage {
            width: 1,
            height: 1,
            indices: vec![0],
            palette: vec![[0; 3]; n],
        };
        assert_eq!(mk(2).bit_depth(), 1);
        assert_eq!(mk(3).bit_depth(), 2);
        assert_eq!(mk(16).bit_depth(), 4);
        assert_eq!(mk(17).bit_depth(), 8);
    }
}
