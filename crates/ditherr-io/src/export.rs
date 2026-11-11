//! Writing rendered images. Indexed results stay indexed: PNG is written at the
//! smallest true bit depth (1, 2, 4 or 8), GIF and BMP use the palette as is.

use std::io::Write;
use std::path::Path;

use ditherr_core::{IndexedImage, Rendered, Rgba8Image};
use image::codecs::bmp::BmpEncoder;
use image::ExtendedColorType;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Gif,
    Bmp,
    Tiff,
    /// Lossless.
    WebP,
}

impl Format {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "png" => Some(Format::Png),
            "gif" => Some(Format::Gif),
            "bmp" => Some(Format::Bmp),
            "tif" | "tiff" => Some(Format::Tiff),
            "webp" => Some(Format::WebP),
            _ => None,
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(Self::from_extension)
    }
}

fn enc_err(e: impl std::fmt::Display) -> Error {
    Error::Encode(e.to_string())
}

fn flat_palette(img: &IndexedImage) -> Vec<u8> {
    img.palette.iter().flatten().copied().collect()
}

/// Packs palette indices into PNG scanlines at `depth` bits, MSB first, rows byte-aligned.
fn pack_indices(img: &IndexedImage, depth: u8) -> Vec<u8> {
    let w = img.width as usize;
    if depth == 8 {
        return img.indices.clone();
    }
    let per_byte = 8 / usize::from(depth);
    let row_bytes = w.div_ceil(per_byte);
    let mut out = vec![0u8; row_bytes * img.height as usize];
    for (y, row) in img.indices.chunks(w).enumerate() {
        for (x, &i) in row.iter().enumerate() {
            let shift = 8 - usize::from(depth) * (x % per_byte + 1);
            out[y * row_bytes + x / per_byte] |= i << shift;
        }
    }
    out
}

/// Writes an indexed PNG at the smallest bit depth that fits the palette.
pub fn encode_indexed_png<W: Write>(w: W, img: &IndexedImage) -> Result<()> {
    let depth = img.bit_depth();
    let mut enc = png::Encoder::new(w, img.width, img.height);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(match depth {
        1 => png::BitDepth::One,
        2 => png::BitDepth::Two,
        4 => png::BitDepth::Four,
        _ => png::BitDepth::Eight,
    });
    enc.set_palette(flat_palette(img));
    let mut writer = enc.write_header().map_err(enc_err)?;
    writer
        .write_image_data(&pack_indices(img, depth))
        .map_err(enc_err)?;
    writer.finish().map_err(enc_err)
}

fn encode_truecolor_png<W: Write>(w: W, img: &Rgba8Image) -> Result<()> {
    let mut enc = png::Encoder::new(w, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(enc_err)?;
    writer.write_image_data(&img.data).map_err(enc_err)?;
    writer.finish().map_err(enc_err)
}

fn encode_gif<W: Write>(w: W, img: &IndexedImage) -> Result<()> {
    let (gw, gh) = match (u16::try_from(img.width), u16::try_from(img.height)) {
        (Ok(w), Ok(h)) => (w, h),
        _ => return Err(Error::Encode("GIF dimensions are limited to 65535".into())),
    };
    let mut enc = gif::Encoder::new(w, gw, gh, &flat_palette(img)).map_err(enc_err)?;
    let frame = gif::Frame::from_indexed_pixels(gw, gh, img.indices.clone(), None);
    enc.write_frame(&frame).map_err(enc_err)
}

fn rgb_bytes(img: &Rgba8Image) -> Vec<u8> {
    img.data
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

/// Encodes `img` in `format`.
///
/// Indexed images keep their palette in PNG, GIF and BMP. GIF cannot hold
/// truecolour output (palettes above 256 colours); that is an error rather
/// than a silent re-quantization. TIFF and WebP are written as 8-bit RGB.
pub fn encode(img: &Rendered, format: Format) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    match (format, img) {
        (Format::Png, Rendered::Indexed(i)) => encode_indexed_png(&mut out, i)?,
        (Format::Png, Rendered::Truecolor(t)) => encode_truecolor_png(&mut out, t)?,
        (Format::Gif, Rendered::Indexed(i)) => encode_gif(&mut out, i)?,
        (Format::Gif, Rendered::Truecolor(_)) => {
            return Err(Error::Encode(
                "GIF holds at most 256 colours; this palette is larger".into(),
            ))
        }
        (Format::Bmp, Rendered::Indexed(i)) => {
            BmpEncoder::new(&mut out)
                .encode_with_palette(
                    &i.indices,
                    i.width,
                    i.height,
                    ExtendedColorType::L8,
                    Some(&i.palette),
                )
                .map_err(enc_err)?;
        }
        (Format::Bmp | Format::Tiff | Format::WebP, _) => {
            let rgba = img.to_rgba8();
            let fmt = match format {
                Format::Bmp => image::ImageFormat::Bmp,
                Format::Tiff => image::ImageFormat::Tiff,
                _ => image::ImageFormat::WebP,
            };
            let buf = image::RgbImage::from_raw(rgba.width, rgba.height, rgb_bytes(&rgba))
                .ok_or_else(|| Error::Encode("buffer size mismatch".into()))?;
            buf.write_to(&mut std::io::Cursor::new(&mut out), fmt)
                .map_err(enc_err)?;
        }
    }
    Ok(out)
}

/// Saves `img`, choosing the format from the file extension.
pub fn save(path: impl AsRef<Path>, img: &Rendered) -> Result<()> {
    let path = path.as_ref();
    let format = Format::from_path(path).ok_or_else(|| {
        Error::Encode(format!(
            "{}: unknown output format; use .png, .gif, .bmp, .tif or .webp",
            path.display()
        ))
    })?;
    let bytes = encode(img, format)?;
    std::fs::write(path, bytes).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed(width: u32, height: u32, colors: usize) -> IndexedImage {
        let palette: Vec<[u8; 3]> = (0..colors)
            .map(|i| {
                [
                    (i * 37 % 256) as u8,
                    (i * 91 % 256) as u8,
                    (i * 5 % 256) as u8,
                ]
            })
            .collect();
        let indices = (0..width * height)
            .map(|i| ((i * 7 + i / 3) as usize % colors) as u8)
            .collect();
        IndexedImage {
            width,
            height,
            indices,
            palette,
        }
    }

    /// Reads a PNG back without any expansion, so bit depth and indices are visible.
    fn read_png_raw(bytes: &[u8]) -> (png::Info<'static>, Vec<u8>) {
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::IDENTITY);
        let mut reader = dec.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut buf).unwrap();
        buf.truncate(frame.buffer_size());
        (reader.info().clone(), buf)
    }

    fn unpack(raw: &[u8], width: usize, height: usize, depth: usize) -> Vec<u8> {
        let per = 8 / depth;
        let row_bytes = width.div_ceil(per);
        let mask = ((1u16 << depth) - 1) as u8;
        let mut out = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let byte = raw[y * row_bytes + x / per];
                let shift = 8 - depth * (x % per + 1);
                out.push((byte >> shift) & mask);
            }
        }
        out
    }

    #[test]
    fn indexed_png_uses_true_bit_depth_and_roundtrips() {
        for (colors, depth) in [(2, 1), (3, 2), (4, 2), (5, 4), (16, 4), (17, 8), (256, 8)] {
            // Odd widths exercise row padding.
            let img = indexed(13, 9, colors);
            let mut bytes = Vec::new();
            encode_indexed_png(&mut bytes, &img).unwrap();
            let (info, raw) = read_png_raw(&bytes);
            assert_eq!(info.color_type, png::ColorType::Indexed, "{colors} colours");
            assert_eq!(info.bit_depth as u8, depth, "{colors} colours");
            let palette = info.palette.as_ref().unwrap();
            assert_eq!(
                &palette[..colors * 3],
                &img.palette.iter().flatten().copied().collect::<Vec<u8>>()[..]
            );
            let back = unpack(&raw, 13, 9, usize::from(depth));
            assert_eq!(back, img.indices, "{colors} colours");
        }
    }

    #[test]
    fn exported_png_decodes_to_the_previewed_pixels() {
        let img = indexed(20, 11, 6);
        let bytes = encode(&Rendered::Indexed(img.clone()), Format::Png).unwrap();
        let decoded = crate::decode_image(&bytes).unwrap();
        assert_eq!(decoded, img.to_rgba8());
    }

    #[test]
    fn truecolor_png_roundtrips() {
        let data: Vec<u8> = (0..6 * 5)
            .flat_map(|i| [i as u8 * 8, 255 - i as u8, 7, 255])
            .collect();
        let img = Rgba8Image::new(6, 5, data).unwrap();
        let bytes = encode(&Rendered::Truecolor(img.clone()), Format::Png).unwrap();
        assert_eq!(crate::decode_image(&bytes).unwrap(), img);
    }

    #[test]
    fn gif_roundtrips_indices_and_palette() {
        for colors in [2, 4, 5, 16, 100, 256] {
            let img = indexed(17, 10, colors);
            let bytes = encode(&Rendered::Indexed(img.clone()), Format::Gif).unwrap();
            let mut opts = gif::DecodeOptions::new();
            opts.set_color_output(gif::ColorOutput::Indexed);
            let mut dec = opts.read_info(std::io::Cursor::new(&bytes)).unwrap();
            let pal = dec.global_palette().unwrap().to_vec();
            let frame = dec.read_next_frame().unwrap().unwrap();
            assert_eq!(frame.buffer.as_ref(), &img.indices[..], "{colors} colours");
            assert_eq!(
                &pal[..colors * 3],
                &img.palette.iter().flatten().copied().collect::<Vec<u8>>()[..]
            );
        }
    }

    #[test]
    fn gif_rejects_truecolor() {
        let img = Rgba8Image::new(1, 1, vec![0, 0, 0, 255]).unwrap();
        assert!(encode(&Rendered::Truecolor(img), Format::Gif).is_err());
    }

    #[test]
    fn bmp_roundtrips_pixels() {
        let img = indexed(15, 7, 8);
        let bytes = encode(&Rendered::Indexed(img.clone()), Format::Bmp).unwrap();
        assert_eq!(crate::decode_image(&bytes).unwrap(), img.to_rgba8());
    }

    #[test]
    fn tiff_and_webp_lossless_roundtrip() {
        let img = indexed(16, 9, 12);
        for f in [Format::Tiff, Format::WebP] {
            let bytes = encode(&Rendered::Indexed(img.clone()), f).unwrap();
            assert_eq!(
                crate::decode_image(&bytes).unwrap(),
                img.to_rgba8(),
                "{f:?}"
            );
        }
    }

    #[test]
    fn extension_detection() {
        assert_eq!(Format::from_path(Path::new("a/b.PNG")), Some(Format::Png));
        assert_eq!(Format::from_path(Path::new("x.tiff")), Some(Format::Tiff));
        assert_eq!(Format::from_path(Path::new("x.xyz")), None);
    }
}
