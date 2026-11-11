//! Palette descriptions that live inside a [`Look`](crate::Look).

use serde::{Deserialize, Serialize};

use crate::color::Metric;
use crate::error::{Error, Result};
use crate::extract::{extract, AutoMethod};
use crate::presets::{self, PresetKind};
use crate::quantizer::{Quantizer, Space};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PaletteSpec {
    /// `levels` evenly spaced grays, `2..=256`. Two levels is 1-bit black and white.
    Gray { levels: u16 },
    /// A full grid with this many levels per channel, e.g. `{8, 8, 4}` for RGB 3-3-2
    /// or `{32, 64, 32}` for RGB565.
    Levels { r: u16, g: u16, b: u16 },
    /// A built-in preset by id; see [`presets::PRESETS`].
    Preset(String),
    /// An explicit list of sRGB colours (up to 256).
    Custom(Vec<[u8; 3]>),
    /// A palette computed from the (pre-processed) image.
    Auto { method: AutoMethod, colors: u16 },
}

impl PaletteSpec {
    pub fn black_white() -> Self {
        PaletteSpec::Gray { levels: 2 }
    }

    pub fn rgb332() -> Self {
        PaletteSpec::Levels { r: 8, g: 8, b: 4 }
    }

    pub fn rgb565() -> Self {
        PaletteSpec::Levels {
            r: 32,
            g: 64,
            b: 32,
        }
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            PaletteSpec::Gray { levels } if !(2..=256).contains(levels) => Err(
                Error::InvalidParam("gray levels must be between 2 and 256".into()),
            ),
            PaletteSpec::Levels { r, g, b }
                if [r, g, b].iter().any(|&&n| !(2..=256).contains(&n)) =>
            {
                Err(Error::InvalidParam(
                    "levels per channel must be between 2 and 256".into(),
                ))
            }
            PaletteSpec::Preset(id) if presets::find(id).is_none() => {
                Err(Error::UnknownPreset(id.clone()))
            }
            PaletteSpec::Custom(c) if c.is_empty() || c.len() > 256 => Err(Error::InvalidParam(
                "a custom palette needs 1 to 256 colours".into(),
            )),
            PaletteSpec::Auto { colors, .. } if !(2..=256).contains(colors) => Err(
                Error::InvalidParam("automatic palettes need 2 to 256 colours".into()),
            ),
            _ => Ok(()),
        }
    }

    /// Builds the quantizer. `sample` is sRGB 8-bit pixels of the image being
    /// processed; only automatic palettes read it.
    pub fn resolve(&self, sample: &[[u8; 3]], metric: Metric, space: Space) -> Result<Quantizer> {
        self.validate()?;
        Ok(match self {
            PaletteSpec::Gray { levels } => {
                let n = u32::from(*levels);
                let colors = (0..n)
                    .map(|k| {
                        let v = ((k * 255 * 2 + (n - 1)) / (2 * (n - 1))) as u8;
                        [v, v, v]
                    })
                    .collect();
                Quantizer::palette(colors, metric, space)
            }
            PaletteSpec::Levels { r, g, b } => Quantizer::grid(*r, *g, *b, space),
            PaletteSpec::Preset(id) => {
                let preset = presets::find(id).ok_or_else(|| Error::UnknownPreset(id.clone()))?;
                match preset.kind {
                    PresetKind::List(list) => Quantizer::palette(
                        list.iter().map(|&c| presets::unpack(c)).collect(),
                        metric,
                        space,
                    ),
                    PresetKind::Grid { r, g, b } => Quantizer::grid(r, g, b, space),
                }
            }
            PaletteSpec::Custom(colors) => Quantizer::palette(colors.clone(), metric, space),
            PaletteSpec::Auto { method, colors } => {
                if sample.is_empty() {
                    return Err(Error::EmptyImage);
                }
                Quantizer::palette(
                    extract(*method, sample, usize::from(*colors)),
                    metric,
                    space,
                )
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACE: Space = Space { linear: true };

    #[test]
    fn gray_levels_span_black_to_white() {
        let q = PaletteSpec::Gray { levels: 4 }
            .resolve(&[], Metric::Oklab, SPACE)
            .unwrap();
        assert_eq!(q.colors(), vec![[0; 3], [85; 3], [170; 3], [255; 3]]);
    }

    #[test]
    fn every_preset_resolves() {
        for p in presets::PRESETS {
            let q = PaletteSpec::Preset(p.id.into())
                .resolve(&[], Metric::Oklab, SPACE)
                .unwrap();
            assert!(q.len() >= 2, "{}", p.id);
        }
    }

    #[test]
    fn rgb_presets_have_expected_sizes() {
        let n = |s: PaletteSpec| s.resolve(&[], Metric::Rgb, SPACE).unwrap().len();
        assert_eq!(n(PaletteSpec::rgb332()), 256);
        assert_eq!(n(PaletteSpec::rgb565()), 65536);
    }

    #[test]
    fn validation() {
        assert!(PaletteSpec::Gray { levels: 1 }.validate().is_err());
        assert!(PaletteSpec::Preset("nope".into()).validate().is_err());
        assert!(PaletteSpec::Custom(vec![]).validate().is_err());
        assert!(PaletteSpec::Auto {
            method: AutoMethod::Octree,
            colors: 1
        }
        .validate()
        .is_err());
        assert!(PaletteSpec::black_white().validate().is_ok());
    }

    #[test]
    fn auto_needs_pixels() {
        let spec = PaletteSpec::Auto {
            method: AutoMethod::MedianCut,
            colors: 4,
        };
        assert_eq!(
            spec.resolve(&[], Metric::Oklab, SPACE).unwrap_err(),
            Error::EmptyImage
        );
        let q = spec
            .resolve(
                &[[0, 0, 0], [255, 255, 255], [255, 0, 0]],
                Metric::Oklab,
                SPACE,
            )
            .unwrap();
        assert_eq!(q.len(), 3);
    }
}
