//! Writes the example looks in `looks/`. Regenerate with:
//!
//! ```sh
//! cargo run -p ditherr-io --example gen_looks
//! ```

use ditherr_core::{Algorithm, ErrorKernel, Look, Metric, PaletteSpec, Params, Step};
use ditherr_io::write_look;

fn main() {
    let gray = Step::Grayscale {
        weights: [0.2126, 0.7152, 0.0722],
    };
    let looks = [
        (
            "gameboy",
            Look {
                name: "Game Boy".into(),
                preprocess: vec![gray.clone(), Step::Contrast(0.25)],
                pixel_scale: 3,
                palette: PaletteSpec::Preset("gameboy".into()),
                algorithm: Algorithm::ErrorDiffusion(ErrorKernel::FloydSteinberg),
                ..Look::default()
            },
        ),
        (
            "1bit-atkinson",
            Look {
                name: "1-bit Atkinson".into(),
                preprocess: vec![gray, Step::Contrast(0.2)],
                palette: PaletteSpec::Preset("mac-1bit".into()),
                algorithm: Algorithm::ErrorDiffusion(ErrorKernel::Atkinson),
                ..Look::default()
            },
        ),
        (
            "pico8-bayer",
            Look {
                name: "PICO-8 Bayer".into(),
                pixel_scale: 2,
                palette: PaletteSpec::Preset("pico8".into()),
                metric: Metric::Oklab,
                algorithm: Algorithm::Bayer { size: 8 },
                ..Look::default()
            },
        ),
        (
            "gray4-blue-noise",
            Look {
                name: "4-level gray, blue noise".into(),
                palette: PaletteSpec::Gray { levels: 4 },
                algorithm: Algorithm::BlueNoise,
                ..Look::default()
            },
        ),
        (
            "rgb332-sierra",
            Look {
                name: "RGB 3-3-2, Sierra Lite".into(),
                palette: PaletteSpec::rgb332(),
                algorithm: Algorithm::ErrorDiffusion(ErrorKernel::SierraLite),
                params: Params {
                    strength: 0.9,
                    ..Params::default()
                },
                ..Look::default()
            },
        ),
    ];
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../looks");
    for (file, look) in looks {
        let path = format!("{dir}/{file}.ditherlook");
        write_look(&path, &look).expect("write look");
        println!("wrote {path}");
    }
}
