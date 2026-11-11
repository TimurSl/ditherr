//! The [`Look`]: one serialisable description of everything that turns a source
//! image into dithered output. It is the universal preset, the CLI input and
//! the undo/redo unit.

use serde::{Deserialize, Serialize};

use crate::buffer::{ImageF32, Rendered, Rgba8Image};
use crate::color::{linear_to_rgb8, rgb8_to_linear, Metric};
use crate::dither::{self, Algorithm, ErrorKernel, Params, WorkImage};
use crate::error::{Error, Result};
use crate::palette::PaletteSpec;
use crate::preprocess::{self, Step};
use crate::quantizer::{Quantizer, Space};
use crate::resize::box_downsample;

/// Version written into new look files. Files with a higher version are rejected.
pub const LOOK_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    pub version: u32,
    pub name: String,
    pub preprocess: Vec<Step>,
    /// Dither at `1/pixel_scale` resolution (box-filtered); upscale at export
    /// with nearest-neighbour for a chunky look.
    pub pixel_scale: u32,
    pub palette: PaletteSpec,
    pub metric: Metric,
    pub algorithm: Algorithm,
    pub params: Params,
    /// What transparent pixels are composited onto.
    pub background: [u8; 3],
}

impl Default for Look {
    fn default() -> Self {
        Self {
            version: LOOK_VERSION,
            name: String::new(),
            preprocess: Vec::new(),
            pixel_scale: 1,
            palette: PaletteSpec::black_white(),
            metric: Metric::default(),
            algorithm: Algorithm::ErrorDiffusion(ErrorKernel::FloydSteinberg),
            params: Params::default(),
            background: [255, 255, 255],
        }
    }
}

impl Look {
    pub fn validate(&self) -> Result<()> {
        if self.version > LOOK_VERSION {
            return Err(Error::UnsupportedVersion(self.version));
        }
        if self.pixel_scale == 0 {
            return Err(Error::InvalidParam("pixel scale must be at least 1".into()));
        }
        for s in &self.preprocess {
            s.validate()?;
        }
        self.palette.validate()?;
        self.algorithm.validate()?;
        self.params.validate()
    }

    pub fn space(&self) -> Space {
        Space {
            linear: self.params.linear_light,
        }
    }
}

/// Runs the pre-process stack, then the pixel-scale downsample.
pub fn preprocess(src: &Rgba8Image, look: &Look) -> Result<ImageF32> {
    look.validate()?;
    let img = ImageF32::from_rgba8(src);
    let img = preprocess::apply(&img, &look.preprocess)?;
    Ok(box_downsample(&img, look.pixel_scale))
}

/// Composites onto the background, clamps to gamut, converts to working space and applies bias.
pub fn working_image(img: &ImageF32, look: &Look) -> WorkImage {
    let bg = rgb8_to_linear(look.background);
    let space = look.space();
    let bias = look.params.bias;
    let data = img
        .data
        .iter()
        .map(|p| {
            let a = p[3].clamp(0.0, 1.0);
            let lin = [
                (p[0] * a + bg[0] * (1.0 - a)).clamp(0.0, 1.0),
                (p[1] * a + bg[1] * (1.0 - a)).clamp(0.0, 1.0),
                (p[2] * a + bg[2] * (1.0 - a)).clamp(0.0, 1.0),
            ];
            let w = space.from_linear(lin);
            [w[0] + bias, w[1] + bias, w[2] + bias]
        })
        .collect();
    WorkImage {
        width: img.width as usize,
        height: img.height as usize,
        data,
    }
}

/// At most this many pixels feed automatic palette extraction.
const PALETTE_SAMPLE_MAX: usize = 262_144;

/// sRGB 8-bit sample of the opaque, composited image for palette extraction.
/// Uses the same composite as [`working_image`] but without bias.
pub fn palette_sample(img: &ImageF32, look: &Look) -> Vec<[u8; 3]> {
    let bg = rgb8_to_linear(look.background);
    let stride = img.data.len().div_ceil(PALETTE_SAMPLE_MAX).max(1);
    img.data
        .iter()
        .step_by(stride)
        .map(|p| {
            let a = p[3].clamp(0.0, 1.0);
            linear_to_rgb8([
                p[0] * a + bg[0] * (1.0 - a),
                p[1] * a + bg[1] * (1.0 - a),
                p[2] * a + bg[2] * (1.0 - a),
            ])
        })
        .collect()
}

/// Resolves the palette and dithers a pre-processed image.
pub fn dither_image(img: &ImageF32, look: &Look) -> Result<Rendered> {
    let sample = palette_sample(img, look);
    let quant: Quantizer = look.palette.resolve(&sample, look.metric, look.space())?;
    let work = working_image(img, look);
    let codes = dither::run(&work, &quant, &look.algorithm, &look.params);
    Ok(quant.finish(&codes, img.width, img.height))
}

/// The whole pipeline: pre-process, pixel-scale, quantize and dither.
///
/// The result is at pixel-scale resolution; use [`Rendered::upscale`] to
/// restore the original size.
pub fn render(src: &Rgba8Image, look: &Look) -> Result<Rendered> {
    let img = preprocess(src, look)?;
    dither_image(&img, look)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_look_is_valid() {
        assert!(Look::default().validate().is_ok());
    }

    #[test]
    fn rejects_newer_versions_and_zero_scale() {
        let l = Look {
            version: LOOK_VERSION + 1,
            ..Look::default()
        };
        assert_eq!(
            l.validate(),
            Err(Error::UnsupportedVersion(LOOK_VERSION + 1))
        );
        let l = Look {
            pixel_scale: 0,
            ..Look::default()
        };
        assert!(l.validate().is_err());
    }

    #[test]
    fn pixel_scale_shrinks_output() {
        let src = Rgba8Image::new(10, 7, vec![128; 10 * 7 * 4]).unwrap();
        let look = Look {
            pixel_scale: 3,
            ..Look::default()
        };
        let out = render(&src, &look).unwrap();
        assert_eq!((out.width(), out.height()), (4, 3));
        let up = out.upscale(3);
        assert_eq!((up.width(), up.height()), (12, 9));
    }

    #[test]
    fn transparent_pixels_take_the_background() {
        let src = Rgba8Image::new(1, 1, vec![0, 0, 0, 0]).unwrap();
        let look = Look {
            background: [255, 255, 255],
            ..Look::default()
        };
        let Rendered::Indexed(img) = render(&src, &look).unwrap() else {
            panic!()
        };
        assert_eq!(img.palette[usize::from(img.indices[0])], [255, 255, 255]);
        let look = Look {
            background: [0, 0, 0],
            ..Look::default()
        };
        let Rendered::Indexed(img) = render(&src, &look).unwrap() else {
            panic!()
        };
        assert_eq!(img.palette[usize::from(img.indices[0])], [0, 0, 0]);
    }
}
