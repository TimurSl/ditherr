//! Studio's parameter panels (spec §2 to §5): every setting of a layer, the
//! pixel scale, the palette and the dither, drawn with plain egui controls.
//!
//! Panels edit data in place and report what they changed through
//! [`Changes`], which the caller turns into one undo step.

use std::mem::discriminant;
use std::ops::RangeInclusive;

use ditherr_core::{
    presets, Algorithm, AutoMethod, ErrorClamp, ErrorKernel, Layer, Look, Metric, PaletteSpec,
    Params, Resampler, ResizeTarget, Rotation, ScanDirection, Step,
};
use eframe::egui::{self, Ui};

use super::ops::{self, hex};
use super::ui::{icon_button, Icon};
use super::{Changes, Studio};
use crate::loaded::Loaded;

const LABEL_WIDTH: f32 = 96.0;

/// A labelled row of a two-column grid.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.label(label);
    add(ui);
    ui.end_row();
}

fn float_slider(
    ui: &mut Ui,
    ch: &mut Changes,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
) {
    row(ui, label, |ui| {
        let r = ui.add(egui::Slider::new(value, range).fixed_decimals(2));
        ch.edited(&r, label);
    });
}

/// A slider row that can be greyed out when the setting has no effect.
fn slider_row(
    ui: &mut Ui,
    ch: &mut Changes,
    enabled: bool,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
) {
    ui.add_enabled(enabled, egui::Label::new(label));
    let r = ui.add_enabled(enabled, egui::Slider::new(value, range).fixed_decimals(2));
    ch.edited(&r, label);
    ui.end_row();
}

fn log_slider(
    ui: &mut Ui,
    ch: &mut Changes,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
) {
    row(ui, label, |ui| {
        let r = ui.add(
            egui::Slider::new(value, range)
                .logarithmic(true)
                .fixed_decimals(2),
        );
        ch.edited(&r, label);
    });
}

fn drag_u32(ui: &mut Ui, ch: &mut Changes, label: &str, value: &mut u32, min: u32) -> bool {
    let r = ui.add(egui::DragValue::new(value).range(min..=65_535).speed(1.0));
    ch.edited(&r, label)
}

/// A drop-down that sets `current` to one of `options`. Returns whether it changed.
fn combo<T: PartialEq + Clone>(
    ui: &mut Ui,
    id: &str,
    current: &mut T,
    options: &[(T, &str)],
) -> bool {
    let text = options
        .iter()
        .find(|(v, _)| v == current)
        .map_or("", |(_, n)| *n);
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(text)
        .width(150.0)
        .show_ui(ui, |ui| {
            for (value, name) in options {
                if ui.selectable_label(value == current, *name).clicked() && value != current {
                    *current = value.clone();
                    changed = true;
                }
            }
        });
    changed
}

fn kernel_name(k: ErrorKernel) -> &'static str {
    match k {
        ErrorKernel::FloydSteinberg => "Floyd–Steinberg",
        ErrorKernel::JarvisJudiceNinke => "Jarvis–Judice–Ninke",
        ErrorKernel::Stucki => "Stucki",
        ErrorKernel::Burkes => "Burkes",
        ErrorKernel::Sierra => "Sierra (3-row)",
        ErrorKernel::SierraTwoRow => "Sierra (2-row)",
        ErrorKernel::SierraLite => "Sierra Lite",
        ErrorKernel::Atkinson => "Atkinson",
        ErrorKernel::StevensonArce => "Stevenson–Arce",
    }
}

/// One-line description of a palette for the stack list.
pub(super) fn palette_summary(spec: &PaletteSpec) -> String {
    match spec {
        PaletteSpec::Gray { levels } => format!("{levels} grays"),
        PaletteSpec::Levels { r, g, b } => format!("RGB {r}·{g}·{b}"),
        PaletteSpec::Preset(id) => {
            presets::find(id).map_or_else(|| id.clone(), |p| p.name.to_string())
        }
        PaletteSpec::Custom(c) => format!("{} custom colours", c.len()),
        PaletteSpec::Auto { colors, .. } => format!("{colors} from image"),
    }
}

/// One-line description of an algorithm for the stack list.
pub(super) fn algorithm_summary(algorithm: &Algorithm) -> String {
    match algorithm {
        Algorithm::ErrorDiffusion(k) => kernel_name(*k).to_string(),
        Algorithm::Bayer { size } => format!("Bayer {size}×{size}"),
        Algorithm::BlueNoise => "Blue noise".into(),
        Algorithm::WhiteNoise => "White noise".into(),
        Algorithm::Threshold { level } => format!("Threshold {level:.2}"),
        Algorithm::Posterize => "Posterize".into(),
    }
}

/// The algorithm families in the drop-down, with the values a switch starts at.
fn algorithm_options() -> Vec<(Algorithm, &'static str)> {
    vec![
        (
            Algorithm::ErrorDiffusion(ErrorKernel::FloydSteinberg),
            "Error diffusion",
        ),
        (Algorithm::Bayer { size: 8 }, "Ordered (Bayer)"),
        (Algorithm::BlueNoise, "Blue noise"),
        (Algorithm::WhiteNoise, "White noise"),
        (Algorithm::Threshold { level: 0.5 }, "Threshold"),
        (Algorithm::Posterize, "Posterize (no dither)"),
    ]
}

fn metric_options() -> Vec<(Metric, &'static str)> {
    vec![
        (Metric::Oklab, "Oklab"),
        (Metric::Lab2000, "CIELAB ΔE 2000"),
        (Metric::Lab76, "CIELAB ΔE 76"),
        (Metric::WeightedRgb, "Weighted RGB"),
        (Metric::Rgb, "RGB"),
    ]
}

fn method_options() -> Vec<(AutoMethod, &'static str)> {
    vec![
        (AutoMethod::KMeans, "k-means"),
        (AutoMethod::MedianCut, "Median cut"),
        (AutoMethod::Octree, "Octree"),
    ]
}

fn resampler_options() -> Vec<(Resampler, &'static str)> {
    vec![
        (Resampler::Lanczos, "Lanczos"),
        (Resampler::Bicubic, "Bicubic"),
        (Resampler::Bilinear, "Bilinear"),
        (Resampler::Nearest, "Nearest"),
    ]
}

// ---- layers --------------------------------------------------------------

fn target_options() -> Vec<(ResizeTarget, &'static str)> {
    vec![
        (ResizeTarget::Percent(50.0), "Percent"),
        (
            ResizeTarget::Pixels {
                width: 320,
                height: 240,
            },
            "Exact size",
        ),
        (ResizeTarget::Width(320), "Width"),
        (ResizeTarget::Height(240), "Height"),
        (
            ResizeTarget::Fit {
                max_width: 320,
                max_height: 240,
            },
            "Fit inside",
        ),
    ]
}

pub(super) fn layer_panel(ui: &mut Ui, layer: &mut Layer, ch: &mut Changes) {
    let name = ops::step_name(&layer.step);
    ui.add_enabled_ui(layer.enabled, |ui| {
        egui::Grid::new("layer-params")
            .num_columns(2)
            .min_col_width(LABEL_WIDTH)
            .show(ui, |ui| match &mut layer.step {
                Step::Exposure(v) => float_slider(ui, ch, "Stops", v, -4.0..=4.0),
                Step::Contrast(v) => float_slider(ui, ch, "Amount", v, -0.95..=2.0),
                Step::Gamma(v) => log_slider(ui, ch, "Gamma", v, 0.2..=4.0),
                Step::Saturation(v) => float_slider(ui, ch, "Saturation", v, 0.0..=3.0),
                Step::Levels {
                    black,
                    white,
                    gamma,
                } => {
                    // Keep a gap so white always stays above black.
                    let (b, w) = (*black, *white);
                    float_slider(ui, ch, "Black point", black, 0.0..=(w - 0.01));
                    float_slider(ui, ch, "White point", white, (b + 0.01)..=1.0);
                    log_slider(ui, ch, "Midtone gamma", gamma, 0.2..=4.0);
                }
                Step::Grayscale { weights } => {
                    for (w, label) in weights.iter_mut().zip(["Red", "Green", "Blue"]) {
                        float_slider(ui, ch, label, w, 0.0..=1.0);
                    }
                    row(ui, "Presets", |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Rec. 709").clicked() {
                                *weights = [0.2126, 0.7152, 0.0722];
                                ch.note("Grayscale weights");
                            }
                            if ui.button("Average").clicked() {
                                *weights = [1.0 / 3.0; 3];
                                ch.note("Grayscale weights");
                            }
                        });
                    });
                }
                Step::Resize { target, resampler } => {
                    row(ui, "Size by", |ui| {
                        if combo_by_variant(ui, "resize-kind", target, &target_options()) {
                            ch.note("Resize mode");
                        }
                    });
                    match target {
                        ResizeTarget::Percent(p) => {
                            float_slider(ui, ch, "Percent", p, 1.0..=400.0);
                        }
                        ResizeTarget::Pixels { width, height } => {
                            row(ui, "Width", |ui| {
                                drag_u32(ui, ch, "Resize width", width, 1);
                            });
                            row(ui, "Height", |ui| {
                                drag_u32(ui, ch, "Resize height", height, 1);
                            });
                        }
                        ResizeTarget::Width(w) => row(ui, "Width", |ui| {
                            drag_u32(ui, ch, "Resize width", w, 1);
                        }),
                        ResizeTarget::Height(h) => row(ui, "Height", |ui| {
                            drag_u32(ui, ch, "Resize height", h, 1);
                        }),
                        ResizeTarget::Fit {
                            max_width,
                            max_height,
                        } => {
                            row(ui, "Max width", |ui| {
                                drag_u32(ui, ch, "Resize width", max_width, 1);
                            });
                            row(ui, "Max height", |ui| {
                                drag_u32(ui, ch, "Resize height", max_height, 1);
                            });
                        }
                    }
                    row(ui, "Resampling", |ui| {
                        if combo(ui, "resampler", resampler, &resampler_options()) {
                            ch.note("Resampling");
                        }
                    });
                }
                Step::Crop {
                    x,
                    y,
                    width,
                    height,
                } => {
                    row(ui, "Left", |ui| {
                        drag_u32(ui, ch, "Crop", x, 0);
                    });
                    row(ui, "Top", |ui| {
                        drag_u32(ui, ch, "Crop", y, 0);
                    });
                    row(ui, "Width", |ui| {
                        drag_u32(ui, ch, "Crop", width, 1);
                    });
                    row(ui, "Height", |ui| {
                        drag_u32(ui, ch, "Crop", height, 1);
                    });
                }
                Step::Rotate(r) => row(ui, "Turn clockwise", |ui| {
                    let opts = [
                        (Rotation::Cw90, "90°"),
                        (Rotation::Cw180, "180°"),
                        (Rotation::Cw270, "270°"),
                    ];
                    if combo(ui, "rotation", r, &opts) {
                        ch.note("Rotate");
                    }
                }),
                Step::FlipHorizontal | Step::FlipVertical => {
                    ui.label("This step has no settings.");
                    ui.end_row();
                }
            });
    });
    if !layer.enabled {
        ui.weak(format!("{name} is switched off; its settings are kept."));
    }
}

// ---- pixel scale ----------------------------------------------------------

pub(super) fn pixel_scale_panel(ui: &mut Ui, look: &mut Look, ch: &mut Changes) {
    ui.label("The image is reduced to 1/N of its size before dithering, then enlarged with nearest-neighbour so each dithered pixel becomes an N×N block.");
    ui.add_space(4.0);
    egui::Grid::new("pixel-scale")
        .num_columns(2)
        .min_col_width(LABEL_WIDTH)
        .show(ui, |ui| {
            row(ui, "Pixel size", |ui| {
                let r = ui.add(egui::Slider::new(&mut look.pixel_scale, 1..=64));
                ch.edited(&r, "Pixel size");
            });
        });
}

// ---- dither ---------------------------------------------------------------

pub(super) fn dither_panel(ui: &mut Ui, look: &mut Look, ch: &mut Changes) {
    let diffusion = matches!(look.algorithm, Algorithm::ErrorDiffusion(_));
    egui::Grid::new("dither-algorithm")
        .num_columns(2)
        .min_col_width(LABEL_WIDTH)
        .show(ui, |ui| {
            row(ui, "Algorithm", |ui| {
                let options = algorithm_options();
                let mut chosen = look.algorithm.clone();
                if combo_by_variant(ui, "algorithm", &mut chosen, &options) {
                    look.algorithm = chosen;
                    ch.note("Algorithm");
                }
            });
            match &mut look.algorithm {
                Algorithm::ErrorDiffusion(kernel) => row(ui, "Kernel", |ui| {
                    let options: Vec<_> = ErrorKernel::ALL
                        .iter()
                        .map(|&k| (k, kernel_name(k)))
                        .collect();
                    if combo(ui, "kernel", kernel, &options) {
                        ch.note("Kernel");
                    }
                }),
                Algorithm::Bayer { size } => row(ui, "Matrix size", |ui| {
                    let options = [(2u8, "2×2"), (4, "4×4"), (8, "8×8"), (16, "16×16")];
                    if combo(ui, "bayer", size, &options) {
                        ch.note("Matrix size");
                    }
                }),
                Algorithm::Threshold { level } => {
                    float_slider(ui, ch, "Level", level, 0.0..=1.0);
                }
                _ => {}
            }
        });

    ui.add_space(6.0);
    ui.strong("Parameters");
    let params = &mut look.params;
    let algorithm = look.algorithm.clone();
    let dithers = !matches!(
        algorithm,
        Algorithm::Posterize | Algorithm::Threshold { .. }
    );
    egui::Grid::new("dither-params")
        .num_columns(2)
        .min_col_width(LABEL_WIDTH)
        .show(ui, |ui| {
            slider_row(ui, ch, dithers, "Strength", &mut params.strength, 0.0..=2.0);
            slider_row(ui, ch, true, "Threshold bias", &mut params.bias, -0.5..=0.5);
            slider_row(ui, ch, dithers, "Jitter", &mut params.jitter, 0.0..=1.0);
            scan_params(ui, ch, params, diffusion);
            row(ui, "Linear light", |ui| {
                let r = ui
                    .checkbox(&mut params.linear_light, "")
                    .on_hover_text("Dither in linear light. Off gives visibly darker midtones.");
                ch.edited(&r, "Linear light");
            });
            row(ui, "Seed", |ui| {
                ui.horizontal(|ui| {
                    let r = ui.add(egui::DragValue::new(&mut params.seed).speed(1.0));
                    ch.edited(&r, "Seed");
                    if ui.button("Random").clicked() {
                        params.seed = fresh_seed();
                        ch.note("Random seed");
                    }
                });
            });
        });
}

/// Scan order, direction and error clamping: error diffusion only.
fn scan_params(ui: &mut Ui, ch: &mut Changes, params: &mut Params, diffusion: bool) {
    ui.add_enabled_ui(diffusion, |ui| {
        ui.label("Serpentine");
    });
    ui.add_enabled_ui(diffusion, |ui| {
        let r = ui.checkbox(&mut params.serpentine, "");
        ch.edited(&r, "Serpentine");
    });
    ui.end_row();

    ui.add_enabled_ui(diffusion, |ui| {
        ui.label("Scan direction");
    });
    ui.add_enabled_ui(diffusion, |ui| {
        let options = [
            (ScanDirection::LeftToRight, "Left to right"),
            (ScanDirection::RightToLeft, "Right to left"),
            (ScanDirection::TopToBottom, "Top to bottom"),
            (ScanDirection::BottomToTop, "Bottom to top"),
        ];
        if combo(ui, "scan", &mut params.scan, &options) {
            ch.note("Scan direction");
        }
    });
    ui.end_row();

    ui.add_enabled_ui(diffusion, |ui| {
        ui.label("Error clamp");
    });
    ui.add_enabled_ui(diffusion, |ui| {
        ui.horizontal(|ui| {
            let mut kind = match params.error_clamp {
                ErrorClamp::Auto => 0u8,
                ErrorClamp::Off => 1,
                ErrorClamp::Limit(_) => 2,
            };
            let options = [(0u8, "Automatic"), (1, "Off"), (2, "Limit")];
            if combo(ui, "clamp", &mut kind, &options) {
                params.error_clamp = match kind {
                    0 => ErrorClamp::Auto,
                    1 => ErrorClamp::Off,
                    _ => ErrorClamp::Limit(0.5),
                };
                ch.note("Error clamp");
            }
            if let ErrorClamp::Limit(limit) = &mut params.error_clamp {
                let r = ui.add(egui::Slider::new(limit, 0.0..=1.0).fixed_decimals(2));
                ch.edited(&r, "Error clamp");
            }
        });
    });
    ui.end_row();
}

/// A new seed from the clock; seeds only need to differ, not be secure.
fn fresh_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64 >> 16)
}

/// Like [`combo`], but matches options by enum variant only, so a value with
/// different contents (a resize target of another size) still shows its name.
fn combo_by_variant<T: Clone>(
    ui: &mut Ui,
    id: &str,
    current: &mut T,
    options: &[(T, &str)],
) -> bool {
    let text = options
        .iter()
        .find(|(v, _)| discriminant(v) == discriminant(current))
        .map_or("", |(_, n)| *n);
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(text)
        .width(150.0)
        .show_ui(ui, |ui| {
            for (value, name) in options {
                let selected = discriminant(value) == discriminant(current);
                if ui.selectable_label(selected, *name).clicked() && !selected {
                    *current = value.clone();
                    changed = true;
                }
            }
        });
    changed
}

// ---- palette ----------------------------------------------------------------

/// The palette kinds in the drop-down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Gray,
    Levels,
    Preset,
    Custom,
    Auto,
}

fn kind_of(spec: &PaletteSpec) -> Kind {
    match spec {
        PaletteSpec::Gray { .. } => Kind::Gray,
        PaletteSpec::Levels { .. } => Kind::Levels,
        PaletteSpec::Preset(_) => Kind::Preset,
        PaletteSpec::Custom(_) => Kind::Custom,
        PaletteSpec::Auto { .. } => Kind::Auto,
    }
}

/// What the palette editor's row buttons asked for.
enum ColorAction {
    Move(usize, isize),
    Delete(usize),
}

impl Studio {
    pub(super) fn palette_panel(&mut self, ui: &mut Ui, loaded: Option<&Loaded>) {
        let mut ch = Changes::default();

        egui::Grid::new("palette-kind")
            .num_columns(2)
            .min_col_width(LABEL_WIDTH)
            .show(ui, |ui| {
                row(ui, "Palette", |ui| {
                    let mut kind = kind_of(&self.edit.look.palette);
                    let options = [
                        (Kind::Gray, "Grays"),
                        (Kind::Levels, "RGB levels"),
                        (Kind::Preset, "Preset"),
                        (Kind::Custom, "Custom colours"),
                        (Kind::Auto, "From image"),
                    ];
                    if combo(ui, "palette-kind-combo", &mut kind, &options) {
                        self.edit.look.palette = self.palette_of_kind(kind);
                        ch.note("Palette type");
                    }
                });
                match &mut self.edit.look.palette {
                    PaletteSpec::Gray { levels } => row(ui, "Levels", |ui| {
                        let r = ui.add(egui::Slider::new(levels, 2..=256).logarithmic(true));
                        ch.edited(&r, "Gray levels");
                    }),
                    PaletteSpec::Levels { r, g, b } => {
                        for (v, label) in [
                            (&mut *r, "Red levels"),
                            (&mut *g, "Green levels"),
                            (&mut *b, "Blue levels"),
                        ] {
                            row(ui, label, |ui| {
                                let resp = ui.add(egui::Slider::new(v, 2..=64));
                                ch.edited(&resp, label);
                            });
                        }
                        let n = u32::from(*r) * u32::from(*g) * u32::from(*b);
                        row(ui, "Colours", |ui| {
                            ui.label(n.to_string());
                        });
                    }
                    PaletteSpec::Preset(id) => row(ui, "Preset", |ui| {
                        let mut chosen = id.clone();
                        let options: Vec<(String, &str)> = presets::PRESETS
                            .iter()
                            .map(|p| (p.id.to_string(), p.name))
                            .collect();
                        if combo(ui, "preset", &mut chosen, &options) {
                            *id = chosen;
                            ch.note("Preset palette");
                        }
                    }),
                    PaletteSpec::Auto { method, colors } => {
                        row(ui, "Method", |ui| {
                            if combo(ui, "auto-method", method, &method_options()) {
                                ch.note("Palette method");
                            }
                        });
                        row(ui, "Colours", |ui| {
                            let r = ui.add(egui::Slider::new(colors, 2..=256).logarithmic(true));
                            ch.edited(&r, "Palette colours");
                        });
                    }
                    PaletteSpec::Custom(_) => {}
                }
                row(ui, "Colour match", |ui| {
                    if combo(ui, "metric", &mut self.edit.look.metric, &metric_options()) {
                        ch.note("Colour match");
                    }
                });
                row(ui, "Background", |ui| {
                    let r = ui
                        .color_edit_button_srgb(&mut self.edit.look.background)
                        .on_hover_text("Transparent pixels are composited onto this colour");
                    ch.edited_drag(&r, "Background");
                });
            });

        ui.add_space(6.0);
        self.palette_swatches(ui, &mut ch);

        ui.add_space(6.0);
        ui.strong("Palette tools");
        self.palette_tools(ui, loaded);

        self.commit(ch);
    }

    /// A fresh palette of the chosen kind, keeping the current colours where it can.
    fn palette_of_kind(&self, kind: Kind) -> PaletteSpec {
        match kind {
            Kind::Gray => PaletteSpec::Gray { levels: 4 },
            Kind::Levels => PaletteSpec::Levels { r: 4, g: 4, b: 4 },
            Kind::Preset => PaletteSpec::Preset("gameboy".into()),
            Kind::Custom => PaletteSpec::Custom(
                self.current_palette()
                    .unwrap_or_else(|| vec![[0, 0, 0], [255, 255, 255]]),
            ),
            Kind::Auto => PaletteSpec::Auto {
                method: self.extract_method,
                colors: 8,
            },
        }
    }

    /// The colour list: editable for a custom palette, read-only otherwise.
    fn palette_swatches(&mut self, ui: &mut Ui, ch: &mut Changes) {
        if let PaletteSpec::Custom(colors) = &mut self.edit.look.palette {
            let mut action = None;
            let n = colors.len();
            ui.horizontal(|ui| {
                ui.strong(format!("Colours ({n})"));
                if ui.button("Add").clicked() && n < ditherr_io::MAX_COLORS {
                    let last = colors.last().copied().unwrap_or([128, 128, 128]);
                    colors.push(last);
                    ch.note("Add colour");
                }
                if ui.button("Sort by lightness").clicked() {
                    ops::sort_by_luma(colors);
                    ch.note("Sort colours");
                }
                if ui.button("Reverse").clicked() {
                    colors.reverse();
                    ch.note("Reverse colours");
                }
            });
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .id_salt("custom-colors")
                .show(ui, |ui| {
                    for (i, color) in colors.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            let r = ui.color_edit_button_srgb(color);
                            ch.edited_drag(&r, "Edit colour");
                            ui.monospace(hex(*color));
                            if icon_button(ui, Icon::Up, i > 0, "Move up").clicked() {
                                action = Some(ColorAction::Move(i, -1));
                            }
                            if icon_button(ui, Icon::Down, i + 1 < n, "Move down").clicked() {
                                action = Some(ColorAction::Move(i, 1));
                            }
                            if icon_button(ui, Icon::Cross, n > 1, "Delete colour").clicked() {
                                action = Some(ColorAction::Delete(i));
                            }
                        });
                    }
                });
            match action {
                Some(ColorAction::Move(i, d)) => {
                    if ops::move_color(colors, i, d).is_some() {
                        ch.note("Move colour");
                    }
                }
                Some(ColorAction::Delete(i)) => {
                    colors.remove(i);
                    ch.note("Delete colour");
                }
                None => {}
            }
        } else {
            let colors = self.current_palette();
            match colors {
                Some(colors) => {
                    ui.strong(format!("Colours ({})", colors.len()));
                    swatch_strip(ui, &colors);
                    if ui
                        .button("Edit as custom colours")
                        .on_hover_text("Turn this palette into a list you can change")
                        .clicked()
                    {
                        self.make_palette_custom();
                    }
                }
                None => {
                    ui.weak("The colours appear after the first render.");
                }
            }
        }
    }

    fn palette_tools(&mut self, ui: &mut Ui, loaded: Option<&Loaded>) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Import…")
                .on_hover_text("A .gpl, .hex, .pal or .act file, or any image")
                .clicked()
            {
                let picked = rfd::FileDialog::new()
                    .add_filter(
                        "Palette or image",
                        &[
                            "gpl", "hex", "pal", "act", "png", "jpg", "jpeg", "webp", "bmp", "tif",
                            "tiff", "gif",
                        ],
                    )
                    .pick_file();
                if let Some(path) = picked {
                    match self.import_palette(&path) {
                        Ok(n) => self.notice = Some(format!("Imported {n} colours")),
                        Err(e) => self.error = Some(e),
                    }
                }
            }
            if ui
                .button("Export…")
                .on_hover_text("Save the palette as .gpl, .hex, .pal or .act")
                .clicked()
            {
                let picked = rfd::FileDialog::new()
                    .add_filter("GIMP palette", &["gpl"])
                    .add_filter("Hex list", &["hex"])
                    .add_filter("JASC palette", &["pal"])
                    .add_filter("Adobe colour table", &["act"])
                    .set_file_name("palette.gpl")
                    .save_file();
                if let Some(path) = picked {
                    match self.export_palette(&path) {
                        Ok(n) => {
                            self.notice = Some(format!("Saved {n} colours to {}", path.display()))
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        });

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            combo(
                ui,
                "extract-method",
                &mut self.extract_method,
                &method_options(),
            );
            ui.add(
                egui::DragValue::new(&mut self.extract_colors)
                    .range(2..=256)
                    .suffix(" colours"),
            );
            let enabled = loaded.is_some();
            if ui
                .add_enabled(enabled, egui::Button::new("Extract from image"))
                .clicked()
            {
                if let Some(img) = loaded {
                    let src = std::sync::Arc::clone(&img.preview_src);
                    self.extract_palette(&src);
                }
            }
        });

        ui.add_space(4.0);
        let r = ui
            .add_enabled(
                loaded.is_some(),
                egui::Button::new("Pick colours from the image").selected(self.eyedropper),
            )
            .on_hover_text("Click the image to add the original's colour under the pointer");
        if r.clicked() {
            self.eyedropper = !self.eyedropper;
        }
        if self.eyedropper {
            ui.weak("Click the image to add a colour.");
        }
    }
}

/// A row of colour squares.
pub(super) fn swatch_strip(ui: &mut Ui, colors: &[[u8; 3]]) {
    let size = 14.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
        for &c in colors {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 2.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
            resp.on_hover_text(hex(c));
        }
    });
}
