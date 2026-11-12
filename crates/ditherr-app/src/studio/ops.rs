//! Studio's edit operations as plain functions on data: layer stack changes,
//! palette editing and colour extraction. No UI types, so everything here is
//! testable without a window.

use ditherr_core::extract::extract;
use ditherr_core::palette::PaletteSpec;
use ditherr_core::presets::{self, PresetKind};
use ditherr_core::quantizer::Space;
use ditherr_core::{AutoMethod, Metric, Resampler, ResizeTarget, Rgba8Image, Rotation, Step};
use ditherr_io::Edit;

/// The steps offered by "Add step", with the values a new layer starts at.
pub fn new_layer_kinds() -> Vec<(&'static str, Step)> {
    vec![
        ("Exposure", Step::Exposure(0.0)),
        ("Contrast", Step::Contrast(0.2)),
        ("Gamma", Step::Gamma(1.0)),
        (
            "Levels",
            Step::Levels {
                black: 0.0,
                white: 1.0,
                gamma: 1.0,
            },
        ),
        ("Saturation", Step::Saturation(1.0)),
        (
            "Grayscale",
            Step::Grayscale {
                weights: [0.2126, 0.7152, 0.0722],
            },
        ),
        (
            "Resize",
            Step::Resize {
                target: ResizeTarget::Percent(50.0),
                resampler: Resampler::Lanczos,
            },
        ),
        (
            "Crop",
            Step::Crop {
                x: 0,
                y: 0,
                width: 256,
                height: 256,
            },
        ),
        ("Rotate", Step::Rotate(Rotation::Cw90)),
        ("Flip horizontal", Step::FlipHorizontal),
        ("Flip vertical", Step::FlipVertical),
    ]
}

pub fn step_name(step: &Step) -> &'static str {
    match step {
        Step::Exposure(_) => "Exposure",
        Step::Contrast(_) => "Contrast",
        Step::Gamma(_) => "Gamma",
        Step::Levels { .. } => "Levels",
        Step::Saturation(_) => "Saturation",
        Step::Grayscale { .. } => "Grayscale",
        Step::Resize { .. } => "Resize",
        Step::Crop { .. } => "Crop",
        Step::Rotate(_) => "Rotate",
        Step::FlipHorizontal => "Flip horizontal",
        Step::FlipVertical => "Flip vertical",
    }
}

/// A short readout of a layer's main value for the stack list.
pub fn step_summary(step: &Step) -> String {
    match step {
        Step::Exposure(v) => format!("{v:+.2} EV"),
        Step::Contrast(v) => format!("{v:+.2}"),
        Step::Gamma(v) => format!("{v:.2}"),
        Step::Levels {
            black,
            white,
            gamma,
        } => format!("{black:.2}–{white:.2}, γ {gamma:.2}"),
        Step::Saturation(v) => format!("{:.0}%", v * 100.0),
        Step::Grayscale { .. } => String::new(),
        Step::Resize { target, .. } => match target {
            ResizeTarget::Pixels { width, height } => format!("{width}×{height}"),
            ResizeTarget::Percent(p) => format!("{p:.0}%"),
            ResizeTarget::Width(w) => format!("width {w}"),
            ResizeTarget::Height(h) => format!("height {h}"),
            ResizeTarget::Fit {
                max_width,
                max_height,
            } => format!("fit {max_width}×{max_height}"),
        },
        Step::Crop { width, height, .. } => format!("{width}×{height}"),
        Step::Rotate(r) => match r {
            Rotation::Cw90 => "90°".into(),
            Rotation::Cw180 => "180°".into(),
            Rotation::Cw270 => "270°".into(),
        },
        Step::FlipHorizontal | Step::FlipVertical => String::new(),
    }
}

/// Appends `step` as a new enabled layer and returns its index.
pub fn add_layer(edit: &mut Edit, step: Step) -> usize {
    edit.layers.push(ditherr_core::Layer::new(step));
    edit.layers.len() - 1
}

/// Removes layer `idx`; false if it does not exist.
pub fn remove_layer(edit: &mut Edit, idx: usize) -> bool {
    if idx < edit.layers.len() {
        edit.layers.remove(idx);
        true
    } else {
        false
    }
}

/// Moves layer `idx` by `delta` places (negative is up) and returns its new
/// index, or `None` if it would leave the stack.
pub fn move_layer(edit: &mut Edit, idx: usize, delta: isize) -> Option<usize> {
    let to = idx.checked_add_signed(delta)?;
    if idx >= edit.layers.len() || to >= edit.layers.len() {
        return None;
    }
    let layer = edit.layers.remove(idx);
    edit.layers.insert(to, layer);
    Some(to)
}

/// Flips a layer on or off; returns the new state.
pub fn toggle_layer(edit: &mut Edit, idx: usize) -> Option<bool> {
    let layer = edit.layers.get_mut(idx)?;
    layer.enabled = !layer.enabled;
    Some(layer.enabled)
}

// ---- palettes ------------------------------------------------------------

/// Whether the palette spec lists its colours up front, so the palette editor
/// can show and edit them. Automatic palettes depend on the image.
fn resolves_without_image(spec: &PaletteSpec) -> bool {
    let grid = |r: u16, g: u16, b: u16| u64::from(r) * u64::from(g) * u64::from(b) <= 256;
    match spec {
        PaletteSpec::Gray { .. } | PaletteSpec::Custom(_) => true,
        PaletteSpec::Levels { r, g, b } => grid(*r, *g, *b),
        PaletteSpec::Preset(id) => match presets::find(id).map(|p| p.kind) {
            Some(PresetKind::List(_)) => true,
            Some(PresetKind::Grid { r, g, b }) => grid(r, g, b),
            None => false,
        },
        PaletteSpec::Auto { .. } => false,
    }
}

/// The colours a palette spec stands for, when they do not depend on the image
/// and fit in 256.
pub fn palette_colors(spec: &PaletteSpec) -> Option<Vec<[u8; 3]>> {
    if !resolves_without_image(spec) {
        return None;
    }
    spec.resolve(&[], Metric::default(), Space { linear: true })
        .ok()
        .map(|q| q.colors())
}

/// Moves colour `idx` by `delta` places; returns its new index.
pub fn move_color(colors: &mut Vec<[u8; 3]>, idx: usize, delta: isize) -> Option<usize> {
    let to = idx.checked_add_signed(delta)?;
    if idx >= colors.len() || to >= colors.len() {
        return None;
    }
    let c = colors.remove(idx);
    colors.insert(to, c);
    Some(to)
}

/// Dark to light, by Rec. 709 luma of the sRGB values (ties keep their order).
pub fn sort_by_luma(colors: &mut [[u8; 3]]) {
    colors.sort_by_key(|c| 2126 * u32::from(c[0]) + 7152 * u32::from(c[1]) + 722 * u32::from(c[2]));
}

/// Adds `color` unless the palette already has it or is full (256 colours).
/// Returns whether it was added.
pub fn add_color(colors: &mut Vec<[u8; 3]>, color: [u8; 3]) -> bool {
    if colors.len() >= ditherr_io::MAX_COLORS || colors.contains(&color) {
        return false;
    }
    colors.push(color);
    true
}

/// A colour of the image at `(x, y)`; `None` outside it or on a transparent pixel.
pub fn pixel_color(img: &Rgba8Image, x: u32, y: u32) -> Option<[u8; 3]> {
    if x >= img.width || y >= img.height {
        return None;
    }
    let i = (y as usize * img.width as usize + x as usize) * 4;
    let p = &img.data[i..i + 4];
    (p[3] > 0).then_some([p[0], p[1], p[2]])
}

/// At most this many pixels feed a colour extraction.
const SAMPLE_MAX: usize = 262_144;

/// Extracts `n` colours from the mostly-opaque pixels of `img`. Empty if the
/// image has none.
pub fn extract_colors(img: &Rgba8Image, method: AutoMethod, n: usize) -> Vec<[u8; 3]> {
    let stride = (img.data.len() / 4).div_ceil(SAMPLE_MAX).max(1);
    let pixels: Vec<[u8; 3]> = img
        .data
        .chunks_exact(4)
        .step_by(stride)
        .filter(|p| p[3] > 127)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    if pixels.is_empty() {
        return Vec::new();
    }
    extract(method, &pixels, n.clamp(2, ditherr_io::MAX_COLORS))
}

/// `#rrggbb`.
pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Parses `#rrggbb` or `rrggbb`.
pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let t = text.trim().trim_start_matches('#');
    if t.len() != 6 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(t, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Whether the rendered preview lines up pixel for pixel with the original,
/// which is true unless a crop, rotate or resize step changed its geometry.
/// Pixel scaling only rounds the size up to a multiple of the scale.
pub fn preview_is_aligned(original: (u32, u32), result: (u32, u32), pixel_scale: u32) -> bool {
    let slack = pixel_scale.max(1);
    result.0 >= original.0
        && result.1 >= original.1
        && result.0 - original.0 < slack
        && result.1 - original.1 < slack
}

#[cfg(test)]
mod tests {
    use super::*;
    use ditherr_core::Look;

    fn edit_with(n: usize) -> Edit {
        let mut e = Edit::default();
        for i in 0..n {
            add_layer(&mut e, Step::Exposure(i as f32));
        }
        e
    }

    fn exposures(e: &Edit) -> Vec<f32> {
        e.layers
            .iter()
            .map(|l| match l.step {
                Step::Exposure(v) => v,
                _ => -1.0,
            })
            .collect()
    }

    #[test]
    fn every_new_layer_kind_is_valid_and_named() {
        let kinds = new_layer_kinds();
        assert_eq!(kinds.len(), 11);
        for (name, step) in kinds {
            step.validate().unwrap();
            assert_eq!(step_name(&step), name);
            let _ = step_summary(&step);
        }
    }

    #[test]
    fn layers_move_toggle_and_remove() {
        let mut e = edit_with(4);
        assert_eq!(move_layer(&mut e, 1, -1), Some(0));
        assert_eq!(exposures(&e), [1.0, 0.0, 2.0, 3.0]);
        assert_eq!(move_layer(&mut e, 0, 2), Some(2));
        assert_eq!(exposures(&e), [0.0, 2.0, 1.0, 3.0]);
        // Off either end: refused, nothing changes.
        assert_eq!(move_layer(&mut e, 0, -1), None);
        assert_eq!(move_layer(&mut e, 3, 1), None);
        assert_eq!(move_layer(&mut e, 9, -1), None);
        assert_eq!(exposures(&e), [0.0, 2.0, 1.0, 3.0]);

        assert_eq!(toggle_layer(&mut e, 1), Some(false));
        assert_eq!(toggle_layer(&mut e, 1), Some(true));
        assert_eq!(toggle_layer(&mut e, 7), None);

        assert!(remove_layer(&mut e, 0));
        assert!(!remove_layer(&mut e, 9));
        assert_eq!(exposures(&e), [2.0, 1.0, 3.0]);
    }

    #[test]
    fn layer_order_changes_the_look() {
        let mut e = Edit::from_look(Look::default());
        add_layer(&mut e, Step::Exposure(1.0));
        add_layer(&mut e, Step::FlipVertical);
        let before = e.look().preprocess;
        move_layer(&mut e, 0, 1);
        assert_ne!(e.look().preprocess, before);
        assert_eq!(
            e.look().preprocess,
            vec![Step::FlipVertical, Step::Exposure(1.0)]
        );
    }

    #[test]
    fn palette_colours_of_each_spec() {
        assert_eq!(
            palette_colors(&PaletteSpec::Gray { levels: 2 })
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            palette_colors(&PaletteSpec::Preset("gameboy".into()))
                .unwrap()
                .len(),
            4
        );
        assert_eq!(
            palette_colors(&PaletteSpec::Custom(vec![[1, 2, 3]])),
            Some(vec![[1, 2, 3]])
        );
        assert_eq!(palette_colors(&PaletteSpec::rgb332()).unwrap().len(), 256);
        // Too big to edit, or dependent on the image.
        assert_eq!(palette_colors(&PaletteSpec::rgb565()), None);
        assert_eq!(
            palette_colors(&PaletteSpec::Auto {
                method: AutoMethod::KMeans,
                colors: 4
            }),
            None
        );
        assert_eq!(palette_colors(&PaletteSpec::Preset("nope".into())), None);
    }

    #[test]
    fn every_listed_preset_that_fits_can_be_edited() {
        for p in presets::PRESETS {
            let spec = PaletteSpec::Preset(p.id.into());
            if let Some(colors) = palette_colors(&spec) {
                assert!(colors.len() >= 2 && colors.len() <= 256, "{}", p.id);
                PaletteSpec::Custom(colors).validate().unwrap();
            }
        }
    }

    #[test]
    fn colour_list_edits() {
        let mut c = vec![[10, 10, 10], [200, 0, 0], [0, 200, 0]];
        assert_eq!(move_color(&mut c, 2, -2), Some(0));
        assert_eq!(c[0], [0, 200, 0]);
        assert_eq!(move_color(&mut c, 0, -1), None);
        assert_eq!(move_color(&mut c, 2, 1), None);

        assert!(add_color(&mut c, [1, 2, 3]));
        assert!(!add_color(&mut c, [1, 2, 3]), "no duplicates");

        sort_by_luma(&mut c);
        assert_eq!(c.first(), Some(&[1, 2, 3]));
        assert_eq!(c.last(), Some(&[0, 200, 0]), "green is the lightest");
    }

    #[test]
    fn palette_stops_at_256_colours() {
        let mut c: Vec<[u8; 3]> = (0..=255).map(|i| [i, 0, 0]).collect();
        assert!(!add_color(&mut c, [0, 1, 0]));
        assert_eq!(c.len(), 256);
    }

    #[test]
    fn picking_and_extracting_colours() {
        let mut data = Vec::new();
        for i in 0..64u8 {
            data.extend_from_slice(if i < 32 {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 255, 255]
            });
        }
        let img = Rgba8Image::new(8, 8, data).unwrap();
        assert_eq!(pixel_color(&img, 0, 0), Some([255, 0, 0]));
        assert_eq!(pixel_color(&img, 7, 7), Some([0, 0, 255]));
        assert_eq!(pixel_color(&img, 8, 0), None);

        let mut got = extract_colors(&img, AutoMethod::MedianCut, 2);
        got.sort();
        assert_eq!(got, vec![[0, 0, 255], [255, 0, 0]]);

        let clear = Rgba8Image::new(2, 2, vec![0; 16]).unwrap();
        assert!(extract_colors(&clear, AutoMethod::KMeans, 4).is_empty());
        assert_eq!(pixel_color(&clear, 0, 0), None);
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex([255, 0, 128]), "#ff0080");
        assert_eq!(parse_hex("#ff0080"), Some([255, 0, 128]));
        assert_eq!(parse_hex(" FF0080 "), Some([255, 0, 128]));
        assert_eq!(parse_hex("#ff008"), None);
        assert_eq!(parse_hex("gg0000"), None);
    }

    #[test]
    fn alignment_allows_only_pixel_scale_padding() {
        assert!(preview_is_aligned((120, 80), (120, 80), 1));
        assert!(preview_is_aligned((120, 80), (120, 81), 3));
        // Rounding up to a multiple of 3 can add at most two pixels (79 -> 81).
        assert!(preview_is_aligned((120, 79), (120, 81), 3));
        assert!(!preview_is_aligned((120, 80), (120, 83), 3));
        assert!(!preview_is_aligned((120, 80), (120, 81), 1));
        // A crop, rotate or resize changes the size.
        assert!(!preview_is_aligned((120, 80), (60, 40), 1));
        assert!(!preview_is_aligned((120, 80), (80, 120), 2));
    }
}
