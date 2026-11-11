use std::path::Path;

use ditherr_core::Rgba8Image;

use crate::error::{Error, Result};

/// Decodes PNG, JPEG, WebP, BMP, TIFF or GIF (first frame) from memory.
/// 16-bit sources are reduced to 8 bits.
pub fn decode_image(bytes: &[u8]) -> Result<Rgba8Image> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| Error::Decode(e.to_string()))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    Ok(Rgba8Image::new(w, h, img.into_raw())?)
}

pub fn read_image(path: impl AsRef<Path>) -> Result<Rgba8Image> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })?;
    decode_image(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ditherr_core::{IndexedImage, Rendered};

    #[test]
    fn decodes_jpeg() {
        let img = image::RgbImage::from_fn(16, 16, |x, y| {
            image::Rgb([(x * 16) as u8, (y * 16) as u8, 90])
        });
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
        let out = decode_image(&bytes).unwrap();
        assert_eq!((out.width, out.height), (16, 16));
        assert!(out.data.chunks(4).all(|p| p[3] == 255));
        // JPEG is lossy: just check the gradient survived.
        assert!(out.data[(15 * 16 + 15) * 4] > out.data[0] + 100);
    }

    #[test]
    fn decodes_first_gif_frame() {
        let img = IndexedImage {
            width: 3,
            height: 2,
            indices: vec![0, 1, 2, 2, 1, 0],
            palette: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]],
        };
        let bytes = crate::encode(&Rendered::Indexed(img.clone()), crate::Format::Gif).unwrap();
        assert_eq!(decode_image(&bytes).unwrap(), img.to_rgba8());
    }

    #[test]
    fn reduces_sixteen_bit_png_to_eight() {
        let mut bytes = Vec::new();
        let mut enc = png::Encoder::new(&mut bytes, 2, 1);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Sixteen);
        let mut w = enc.write_header().unwrap();
        // Big-endian 16-bit samples: (0xFF00, 0x8000, 0x0000), (0x0100, 0xFFFF, 0x7F00).
        w.write_image_data(&[
            0xFF, 0x00, 0x80, 0x00, 0, 0, 0x01, 0x00, 0xFF, 0xFF, 0x7F, 0x00,
        ])
        .unwrap();
        w.finish().unwrap();
        let out = decode_image(&bytes).unwrap();
        // 16-bit samples are scaled proportionally (v * 255 / 65535), within rounding.
        let samples = [0xFF00u32, 0x8000, 0x0000, 0x0100, 0xFFFF, 0x7F00];
        for (i, &v) in samples.iter().enumerate() {
            let got = i32::from(out.data[i / 3 * 4 + i % 3]);
            let want = (v * 255 / 65535) as i32;
            assert!(
                (got - want).abs() <= 1,
                "sample {i}: got {got}, want about {want}"
            );
        }
        assert_eq!((out.data[3], out.data[7]), (255, 255));
    }

    #[test]
    fn garbage_is_a_decode_error() {
        assert!(matches!(
            decode_image(b"not an image"),
            Err(Error::Decode(_))
        ));
        assert!(matches!(read_image("/no/such/file.png"), Err(Error::Io(_))));
    }
}
