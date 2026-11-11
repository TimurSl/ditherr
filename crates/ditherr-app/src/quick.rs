//! Quick Stylize logic (spec §7): starter looks, the five controls and randomize.
//!
//! Everything here is plain data in, [`Look`] out, so it is testable without a
//! window. The controls never touch pixels; they edit a copy of the selected
//! look and the engine renders it.

use ditherr_core::{Algorithm, AutoMethod, ErrorKernel, Look, Metric, PaletteSpec, Step};

pub const PIXEL_SIZE_RANGE: (u32, u32) = (1, 16);
pub const COLOR_RANGE: (u16, u16) = (2, 32);
pub const CONTRAST_RANGE: (f32, f32) = (-0.8, 0.8);
pub const STRENGTH_RANGE: (f32, f32) = (0.0, 1.5);

/// A look offered in the gallery.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub look: Look,
}

/// Where the palette slider's choice comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteChoice {
    /// Keep the selected look's own palette.
    Look,
    /// Evenly spaced grays; the colour count picks how many.
    Gray,
    /// Colours extracted from the image (k-means); the colour count picks how many.
    FromImage,
    /// A built-in preset by id.
    Preset(String),
}

/// The five Quick Stylize controls.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Size of one output pixel in source pixels.
    pub pixel_size: u32,
    pub colors: u16,
    pub strength: f32,
    /// Added on top of any contrast the look already applies.
    pub contrast: f32,
    pub palette: PaletteChoice,
}

impl Settings {
    /// Slider positions that reproduce `look` exactly.
    pub fn from_look(look: &Look) -> Self {
        Self {
            pixel_size: look
                .pixel_scale
                .clamp(PIXEL_SIZE_RANGE.0, PIXEL_SIZE_RANGE.1),
            colors: color_count(&look.palette).unwrap_or(4),
            strength: look
                .params
                .strength
                .clamp(STRENGTH_RANGE.0, STRENGTH_RANGE.1),
            contrast: 0.0,
            palette: PaletteChoice::Look,
        }
    }

    /// Whether the colour-count slider has any effect for `base`.
    pub fn colors_adjustable(&self, base: &Look) -> bool {
        match self.palette {
            PaletteChoice::Gray | PaletteChoice::FromImage => true,
            PaletteChoice::Preset(_) => false,
            PaletteChoice::Look => color_count(&base.palette).is_some(),
        }
    }
}

/// Number of colours of palettes whose size is a free parameter.
fn color_count(p: &PaletteSpec) -> Option<u16> {
    match p {
        PaletteSpec::Gray { levels } => Some(*levels),
        PaletteSpec::Auto { colors, .. } => Some(*colors),
        _ => None,
    }
    .map(|n| n.clamp(COLOR_RANGE.0, COLOR_RANGE.1))
}

/// The look to render: `base` with the five controls applied.
pub fn apply_settings(base: &Look, s: &Settings) -> Look {
    let mut look = base.clone();
    look.pixel_scale = s.pixel_size.max(1);
    look.params.strength = s.strength;
    if s.contrast != 0.0 {
        look.preprocess.push(Step::Contrast(s.contrast));
    }
    look.palette = match &s.palette {
        PaletteChoice::Gray => PaletteSpec::Gray { levels: s.colors },
        PaletteChoice::FromImage => PaletteSpec::Auto {
            method: AutoMethod::KMeans,
            colors: s.colors,
        },
        PaletteChoice::Preset(id) => PaletteSpec::Preset(id.clone()),
        PaletteChoice::Look => match &base.palette {
            PaletteSpec::Gray { .. } => PaletteSpec::Gray { levels: s.colors },
            PaletteSpec::Auto { method, .. } => PaletteSpec::Auto {
                method: *method,
                colors: s.colors,
            },
            other => other.clone(),
        },
    };
    look
}

fn entry(name: &str, look: Look) -> Entry {
    Entry {
        name: name.into(),
        look: Look {
            name: name.into(),
            ..look
        },
    }
}

fn gray_steps(contrast: f32) -> Vec<Step> {
    vec![
        Step::Grayscale {
            weights: [0.2126, 0.7152, 0.0722],
        },
        Step::Contrast(contrast),
    ]
}

/// The built-in gallery (spec §7 "Starter looks").
pub fn starter_looks() -> Vec<Entry> {
    vec![
        entry(
            "Game Boy",
            Look {
                preprocess: gray_steps(0.25),
                pixel_scale: 3,
                palette: PaletteSpec::Preset("gameboy".into()),
                ..Look::default()
            },
        ),
        // The engine has ordered and diffusion dithers but no clustered-dot
        // screen yet; a coarse Bayer matrix is the closest print-like look.
        entry(
            "Newspaper",
            Look {
                preprocess: gray_steps(0.3),
                pixel_scale: 3,
                palette: PaletteSpec::Gray { levels: 2 },
                algorithm: Algorithm::Bayer { size: 8 },
                background: [244, 240, 228],
                ..Look::default()
            },
        ),
        entry(
            "Macintosh 1-bit",
            Look {
                preprocess: gray_steps(0.1),
                pixel_scale: 2,
                palette: PaletteSpec::Preset("mac-1bit".into()),
                algorithm: Algorithm::ErrorDiffusion(ErrorKernel::Atkinson),
                ..Look::default()
            },
        ),
        entry(
            "CGA",
            Look {
                pixel_scale: 3,
                palette: PaletteSpec::Preset("cga".into()),
                algorithm: Algorithm::Bayer { size: 4 },
                metric: Metric::default(),
                ..Look::default()
            },
        ),
        entry(
            "E-ink",
            Look {
                preprocess: gray_steps(0.15),
                palette: PaletteSpec::Custom(vec![
                    [0x1b, 0x1c, 0x1a],
                    [0x6b, 0x6d, 0x66],
                    [0xb3, 0xb5, 0xab],
                    [0xe6, 0xe7, 0xde],
                ]),
                algorithm: Algorithm::ErrorDiffusion(ErrorKernel::Burkes),
                ..Look::default()
            },
        ),
        entry(
            "Risograph",
            Look {
                preprocess: vec![Step::Saturation(1.2), Step::Contrast(0.15)],
                pixel_scale: 2,
                palette: PaletteSpec::Custom(vec![
                    [0xf4, 0xef, 0xe6],
                    [0xff, 0x6c, 0xb5],
                    [0x00, 0x78, 0xbf],
                    [0x3b, 0x2a, 0x6e],
                ]),
                algorithm: Algorithm::ErrorDiffusion(ErrorKernel::SierraLite),
                params: ditherr_core::Params {
                    jitter: 0.08,
                    ..ditherr_core::Params::default()
                },
                ..Look::default()
            },
        ),
    ]
}

/// Small deterministic generator (splitmix64) so randomize needs no dependency
/// and the same seed always gives the same variation.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.next() >> 40) as f32 / (1u64 << 24) as f32 * (hi - lo)
    }
}

/// A random but sensible look: any algorithm, a palette, a pixel size.
pub fn randomize(seed: u64) -> Look {
    let mut rng = Rng(seed);

    let algorithm = match rng.below(5) {
        0 => Algorithm::ErrorDiffusion(ErrorKernel::ALL[rng.below(ErrorKernel::ALL.len())]),
        1 => Algorithm::Bayer {
            size: [2, 4, 8, 16][rng.below(4)],
        },
        2 => Algorithm::BlueNoise,
        3 => Algorithm::ErrorDiffusion(ErrorKernel::Atkinson),
        _ => Algorithm::WhiteNoise,
    };

    let presets = ditherr_core::presets::PRESETS;
    let palette = match rng.below(4) {
        0 => PaletteSpec::Preset(presets[rng.below(presets.len())].id.into()),
        1 => PaletteSpec::Gray {
            levels: 2 + rng.below(5) as u16,
        },
        2 => PaletteSpec::Auto {
            method: AutoMethod::KMeans,
            colors: 3 + rng.below(10) as u16,
        },
        _ => PaletteSpec::Levels {
            r: 2 + rng.below(3) as u16,
            g: 2 + rng.below(3) as u16,
            b: 2 + rng.below(3) as u16,
        },
    };

    let mut look = Look {
        pixel_scale: 1 + rng.below(6) as u32,
        palette,
        algorithm,
        ..Look::default()
    };
    look.params.strength = rng.range(0.6, 1.0);
    look.params.jitter = if rng.below(3) == 0 {
        rng.range(0.0, 0.15)
    } else {
        0.0
    };
    look.params.seed = rng.next() >> 16;
    if rng.below(2) == 0 {
        look.preprocess.push(Step::Contrast(rng.range(0.0, 0.4)));
    }
    look
}

#[cfg(test)]
mod tests {
    use super::*;
    use ditherr_core::{render, Rgba8Image};

    fn gradient(w: u32, h: u32) -> Rgba8Image {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 120, 255]);
            }
        }
        Rgba8Image::new(w, h, data).unwrap()
    }

    #[test]
    fn starters_are_valid_and_render() {
        let src = gradient(48, 32);
        let starters = starter_looks();
        assert_eq!(starters.len(), 6);
        for e in starters {
            e.look
                .validate()
                .unwrap_or_else(|err| panic!("{}: {err}", e.name));
            render(&src, &e.look).unwrap_or_else(|err| panic!("{}: {err}", e.name));
        }
    }

    #[test]
    fn untouched_controls_reproduce_the_look() {
        for e in starter_looks() {
            let s = Settings::from_look(&e.look);
            assert_eq!(apply_settings(&e.look, &s), e.look, "{}", e.name);
        }
    }

    #[test]
    fn controls_edit_the_look() {
        let base = &starter_looks()[0].look;
        let mut s = Settings::from_look(base);
        s.pixel_size = 5;
        s.strength = 0.5;
        s.contrast = 0.4;
        s.palette = PaletteChoice::FromImage;
        s.colors = 7;
        let look = apply_settings(base, &s);
        assert_eq!(look.pixel_scale, 5);
        assert_eq!(look.params.strength, 0.5);
        assert_eq!(look.preprocess.last(), Some(&Step::Contrast(0.4)));
        assert_eq!(
            look.palette,
            PaletteSpec::Auto {
                method: AutoMethod::KMeans,
                colors: 7
            }
        );
        look.validate().unwrap();
        // The base is never modified.
        assert_eq!(base, &starter_looks()[0].look);
    }

    #[test]
    fn colour_slider_follows_the_palette() {
        let starters = starter_looks();
        let newspaper = &starters[1].look; // gray, free count
        let gameboy = &starters[0].look; // fixed preset

        let mut s = Settings::from_look(newspaper);
        assert!(s.colors_adjustable(newspaper));
        s.colors = 5;
        assert_eq!(
            apply_settings(newspaper, &s).palette,
            PaletteSpec::Gray { levels: 5 }
        );

        let s = Settings::from_look(gameboy);
        assert!(!s.colors_adjustable(gameboy));
        assert_eq!(apply_settings(gameboy, &s).palette, gameboy.palette);

        let s = Settings {
            palette: PaletteChoice::Preset("pico8".into()),
            ..s
        };
        assert!(!s.colors_adjustable(gameboy));
        assert_eq!(
            apply_settings(gameboy, &s).palette,
            PaletteSpec::Preset("pico8".into())
        );
    }

    #[test]
    fn randomize_is_deterministic_and_valid() {
        let src = gradient(40, 40);
        for seed in 0..200 {
            let look = randomize(seed);
            assert_eq!(look, randomize(seed));
            look.validate()
                .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            // Settings round-trip keeps it valid too.
            let s = Settings::from_look(&look);
            apply_settings(&look, &s).validate().unwrap();
            if seed < 40 {
                render(&src, &look).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            }
        }
        assert_ne!(randomize(1), randomize(2));
    }
}
