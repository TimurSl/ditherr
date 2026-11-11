use std::path::Path;

use ditherr_core::{Look, LOOK_VERSION};

use crate::error::{Error, Result};

pub const LOOK_EXTENSION: &str = "ditherlook";

/// Serialises a look as RON with a short header comment.
pub fn look_to_string(look: &Look) -> Result<String> {
    let body = ron::ser::to_string_pretty(look, ron::ser::PrettyConfig::default())
        .map_err(|e| Error::Look(e.to_string()))?;
    Ok(format!(
        "// Ditherr look, format version {LOOK_VERSION}\n{body}\n"
    ))
}

/// Parses and validates a look. Files from a newer format version are rejected.
pub fn parse_look(text: &str) -> Result<Look> {
    let look: Look = ron::from_str(text).map_err(|e| Error::Look(e.to_string()))?;
    look.validate()?;
    Ok(look)
}

pub fn read_look(path: impl AsRef<Path>) -> Result<Look> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })?;
    parse_look(&text).map_err(|e| match e {
        Error::Look(m) => Error::Look(format!("{}: {m}", path.display())),
        other => other,
    })
}

pub fn write_look(path: impl AsRef<Path>, look: &Look) -> Result<()> {
    std::fs::write(path, look_to_string(look)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ditherr_core::{
        Algorithm, AutoMethod, ErrorClamp, ErrorKernel, Metric, PaletteSpec, Params, Resampler,
        ResizeTarget, Rotation, ScanDirection, Step,
    };

    fn busy_look() -> Look {
        Look {
            name: "busy".into(),
            preprocess: vec![
                Step::Exposure(0.5),
                Step::Levels {
                    black: 0.1,
                    white: 0.9,
                    gamma: 1.1,
                },
                Step::Grayscale {
                    weights: [0.3, 0.6, 0.1],
                },
                Step::Resize {
                    target: ResizeTarget::Fit {
                        max_width: 64,
                        max_height: 64,
                    },
                    resampler: Resampler::Bicubic,
                },
                Step::Crop {
                    x: 1,
                    y: 2,
                    width: 3,
                    height: 4,
                },
                Step::Rotate(Rotation::Cw90),
                Step::FlipVertical,
            ],
            pixel_scale: 3,
            palette: PaletteSpec::Auto {
                method: AutoMethod::KMeans,
                colors: 12,
            },
            metric: Metric::Lab2000,
            algorithm: Algorithm::ErrorDiffusion(ErrorKernel::Atkinson),
            params: Params {
                strength: 0.8,
                scan: ScanDirection::BottomToTop,
                error_clamp: ErrorClamp::Limit(0.3),
                seed: 99,
                ..Params::default()
            },
            background: [10, 20, 30],
            ..Look::default()
        }
    }

    #[test]
    fn roundtrips_every_kind_of_field() {
        let mut looks = vec![busy_look(), Look::default()];
        for palette in [
            PaletteSpec::Gray { levels: 4 },
            PaletteSpec::rgb565(),
            PaletteSpec::Preset("gameboy".into()),
            PaletteSpec::Custom(vec![[1, 2, 3], [250, 251, 252]]),
        ] {
            for algorithm in [
                Algorithm::Threshold { level: 0.4 },
                Algorithm::Posterize,
                Algorithm::Bayer { size: 8 },
                Algorithm::BlueNoise,
                Algorithm::WhiteNoise,
            ] {
                looks.push(Look {
                    palette: palette.clone(),
                    algorithm,
                    ..Look::default()
                });
            }
        }
        for look in looks {
            let text = look_to_string(&look).unwrap();
            assert_eq!(parse_look(&text).unwrap(), look, "{text}");
        }
    }

    #[test]
    fn missing_fields_take_defaults() {
        let look = parse_look("(palette: Preset(\"c64\"))").unwrap();
        assert_eq!(look.palette, PaletteSpec::Preset("c64".into()));
        assert_eq!(look.algorithm, Look::default().algorithm);
        assert_eq!(look.pixel_scale, 1);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(matches!(parse_look("(version: 99)"), Err(Error::Core(_))));
        assert!(matches!(
            parse_look("(pixel_scale: 0)"),
            Err(Error::Core(_))
        ));
        assert!(matches!(
            parse_look("(palette: Preset(\"nope\"))"),
            Err(Error::Core(_))
        ));
        assert!(matches!(parse_look("this is not ron"), Err(Error::Look(_))));
    }
}
