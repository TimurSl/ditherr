//! The Studio workspace (spec §8): the whole engine exposed as an effect stack
//! plus palette and dither panels, with comparison views, variations, an info
//! panel and unlimited undo.
//!
//! `mod.rs` holds the state and everything that changes it; [`panels`] draws
//! the parameter controls and [`ui`] lays out the window. Edits are made on
//! [`Edit`] values (layers plus a look), which is also what undo snapshots and
//! project files store, so the UI never has a private copy of the settings.

pub mod ops;
mod panels;
mod ui;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use ditherr_core::{AutoMethod, PaletteSpec, Rgba8Image};
use ditherr_io::{Edit, ExportSettings, PaletteFile, PaletteFormat, Project, Variation};
use eframe::egui::{self, TextureHandle, TextureOptions};

use crate::app::export_to;
use crate::history::History;
use crate::info::Stats;
use crate::loaded::{to_color_image, Loaded};
use crate::view::View;
use crate::worker::{scaled_look, Job, Worker};
use ops::{add_color, extract_colors, palette_colors};

const PREVIEW_KEY: usize = 0;

/// Formats offered by the export menu: file extension and label.
pub const EXPORT_FORMATS: [(&str, &str); 5] = [
    ("png", "PNG"),
    ("gif", "GIF"),
    ("bmp", "BMP"),
    ("tif", "TIFF"),
    ("webp", "WebP"),
];

/// Integer upscales offered by the export menu.
pub const EXPORT_FACTORS: [u32; 6] = [1, 2, 3, 4, 8, 16];

/// What the right-hand parameter panel is editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    /// A pre-process layer, by position in the stack.
    Layer(usize),
    PixelScale,
    Palette,
    Dither,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareMode {
    /// The result, with the original on a toggle (or while Space is held).
    Single,
    /// One image with a draggable divider: original left, result right.
    Split,
    SideBySide,
}

/// The pixel under the pointer.
#[derive(Debug, Clone, Copy)]
struct Hover {
    px: [u32; 2],
    color: [u8; 3],
}

#[derive(Default)]
struct Info {
    stats: Option<Stats>,
    millis: u128,
    out_size: (u32, u32),
}

/// What a UI pass changed, so it can become one undo step.
#[derive(Default)]
pub(crate) struct Changes {
    label: Option<String>,
    key: Option<String>,
}

impl Changes {
    /// A discrete change (a button, a menu choice): always its own step.
    pub(crate) fn step(label: impl Into<String>) -> Self {
        Self {
            label: Some(label.into()),
            key: None,
        }
    }

    /// Notes a widget edit. Drags and typing merge into one step per gesture.
    pub(crate) fn edited(&mut self, response: &egui::Response, label: &str) -> bool {
        if !response.changed() {
            return false;
        }
        self.label = Some(label.to_owned());
        self.key = (response.dragged() || response.has_focus()).then(|| label.to_owned());
        true
    }

    /// Like [`edited`](Self::edited) for widgets that open a popup (colour
    /// pickers), where the drag happens outside the widget's own response:
    /// every change inside one pointer gesture merges.
    pub(crate) fn edited_drag(&mut self, response: &egui::Response, label: &str) -> bool {
        if !response.changed() {
            return false;
        }
        self.label = Some(label.to_owned());
        self.key = Some(label.to_owned());
        true
    }

    /// Notes a discrete change made while building the panel.
    pub(crate) fn note(&mut self, label: impl Into<String>) {
        self.label = Some(label.into());
        self.key = None;
    }

    fn any(&self) -> bool {
        self.label.is_some()
    }
}

pub struct Studio {
    previews: Worker,
    thumbs: Worker,

    /// The working copy; always equal to `history.current()` between frames.
    edit: Edit,
    history: History<Edit>,
    variations: Vec<Variation>,
    var_tex: Vec<Option<TextureHandle>>,
    export: ExportSettings,
    project_path: Option<PathBuf>,
    /// A project asked for this source image; the app opens it.
    pending_source: Option<PathBuf>,

    selected: Selected,
    extract_method: AutoMethod,
    extract_colors: u16,
    /// Clicking the preview adds the original's colour under the pointer.
    eyedropper: bool,

    view: View,
    zoom_accum: f32,
    compare: CompareMode,
    /// Divider position for [`CompareMode::Split`], `0..=1` across the image.
    split: f32,
    show_original: bool,
    hover: Option<Hover>,

    preview_tex: Option<TextureHandle>,
    /// CPU copy of the preview, for the pixel readout.
    preview_img: Option<Rgba8Image>,
    info: Info,
    error: Option<String>,
    notice: Option<String>,
    export_rx: Option<Receiver<Result<PathBuf, String>>>,

    seq: u64,
    /// Results older than this belong to a previous image.
    preview_min_tag: u64,
    thumb_epoch: u64,
    dirty: bool,
    thumbs_dirty: bool,
}

type Repaint = Arc<dyn Fn() + Send + Sync>;

impl Studio {
    pub fn new(repaint: Repaint) -> Self {
        let edit = Edit::default();
        Self {
            previews: Worker::new(repaint.clone()),
            thumbs: Worker::new(repaint),
            history: History::new("Start", edit.clone()),
            edit,
            variations: Vec::new(),
            var_tex: Vec::new(),
            export: ExportSettings::default(),
            project_path: None,
            pending_source: None,
            selected: Selected::Dither,
            extract_method: AutoMethod::KMeans,
            extract_colors: 8,
            eyedropper: false,
            view: View::default(),
            zoom_accum: 0.0,
            compare: CompareMode::Single,
            split: 0.5,
            show_original: false,
            hover: None,
            preview_tex: None,
            preview_img: None,
            info: Info::default(),
            error: None,
            notice: None,
            export_rx: None,
            seq: 0,
            preview_min_tag: 0,
            thumb_epoch: 0,
            dirty: false,
            thumbs_dirty: false,
        }
    }

    // ---- document ------------------------------------------------------

    /// Replaces the edit and starts a fresh history (opening a project, or
    /// handing over a look from Quick Stylize).
    pub fn load_edit(&mut self, edit: Edit, label: &str) {
        self.history.reset(label, edit.clone());
        self.edit = edit;
        self.fix_selection();
        self.dirty = true;
    }

    /// Takes the source image a freshly opened project asked for.
    pub fn take_pending_source(&mut self) -> Option<PathBuf> {
        self.pending_source.take()
    }

    /// Call when a different image is shown: the old preview no longer applies.
    pub fn image_changed(&mut self) {
        self.preview_min_tag = self.seq + 1;
        self.preview_tex = None;
        self.preview_img = None;
        self.hover = None;
        self.info = Info::default();
        self.view.reset();
        self.dirty = true;
        self.thumbs_dirty = true;
    }

    /// Records what the UI changed as an undo step and schedules a render.
    pub(crate) fn commit(&mut self, changes: Changes) {
        if !changes.any() {
            return;
        }
        let label = changes.label.unwrap_or_default();
        if self
            .history
            .record(label, self.edit.clone(), changes.key.as_deref())
        {
            self.dirty = true;
        }
    }

    fn fix_selection(&mut self) {
        if let Selected::Layer(i) = self.selected {
            let n = self.edit.layers.len();
            if n == 0 {
                self.selected = Selected::Dither;
            } else if i >= n {
                self.selected = Selected::Layer(n - 1);
            }
        }
    }

    fn adopt_history_state(&mut self) {
        self.edit = self.history.current().clone();
        self.fix_selection();
        self.dirty = true;
    }

    pub fn undo(&mut self) {
        if self.history.undo().is_some() {
            self.adopt_history_state();
        }
    }

    pub fn redo(&mut self) {
        if self.history.redo().is_some() {
            self.adopt_history_state();
        }
    }

    pub fn jump_to(&mut self, step: usize) {
        if self.history.jump(step).is_some() {
            self.adopt_history_state();
        }
    }

    // ---- layers --------------------------------------------------------

    pub fn add_layer(&mut self, name: &str, step: ditherr_core::Step) {
        let i = ops::add_layer(&mut self.edit, step);
        self.selected = Selected::Layer(i);
        self.commit(Changes::step(format!("Add {name}")));
    }

    pub fn delete_layer(&mut self, idx: usize) {
        let Some(name) = self.edit.layers.get(idx).map(|l| ops::step_name(&l.step)) else {
            return;
        };
        ops::remove_layer(&mut self.edit, idx);
        self.fix_selection();
        self.commit(Changes::step(format!("Delete {name}")));
    }

    pub fn move_layer(&mut self, idx: usize, delta: isize) {
        if let Some(to) = ops::move_layer(&mut self.edit, idx, delta) {
            let name = ops::step_name(&self.edit.layers[to].step);
            if self.selected == Selected::Layer(idx) {
                self.selected = Selected::Layer(to);
            }
            self.commit(Changes::step(format!("Move {name}")));
        }
    }

    pub fn toggle_layer(&mut self, idx: usize) {
        if let Some(on) = ops::toggle_layer(&mut self.edit, idx) {
            let name = ops::step_name(&self.edit.layers[idx].step);
            let verb = if on { "Enable" } else { "Disable" };
            self.commit(Changes::step(format!("{verb} {name}")));
        }
    }

    // ---- variations ----------------------------------------------------

    pub fn save_variation(&mut self) {
        let n = self.variations.len() + 1;
        self.variations.push(Variation {
            name: format!("Variation {n}"),
            edit: self.edit.clone(),
        });
        self.var_tex.push(None);
        self.thumbs_dirty = true;
    }

    pub fn apply_variation(&mut self, idx: usize) {
        let Some(v) = self.variations.get(idx) else {
            return;
        };
        let label = format!("Apply {}", v.name);
        self.edit = v.edit.clone();
        self.fix_selection();
        self.commit(Changes::step(label));
    }

    pub fn delete_variation(&mut self, idx: usize) {
        if idx < self.variations.len() {
            self.variations.remove(idx);
            self.var_tex.remove(idx);
            self.thumbs_dirty = true;
        }
    }

    // ---- palette -------------------------------------------------------

    /// The colours of the current palette: listed ones directly, automatic
    /// ones from the last render.
    fn current_palette(&self) -> Option<Vec<[u8; 3]>> {
        palette_colors(&self.edit.look.palette).or_else(|| {
            self.info
                .stats
                .as_ref()
                .map(|s| s.palette.clone())
                .filter(|p| !p.is_empty())
        })
    }

    fn set_custom_palette(&mut self, colors: Vec<[u8; 3]>, label: &str) {
        self.edit.look.palette = PaletteSpec::Custom(colors);
        self.selected = Selected::Palette;
        self.commit(Changes::step(label));
    }

    /// Turns whatever palette is set into an editable list of colours.
    pub fn make_palette_custom(&mut self) {
        match self.current_palette() {
            Some(colors) => self.set_custom_palette(colors, "Edit palette"),
            None => self.error = Some("This palette is too large to edit as a list".into()),
        }
    }

    /// Adds a colour picked from the image, starting a custom palette if needed.
    pub fn pick_color(&mut self, color: [u8; 3]) {
        let mut colors = match &self.edit.look.palette {
            PaletteSpec::Custom(c) => c.clone(),
            _ => self.current_palette().unwrap_or_default(),
        };
        if add_color(&mut colors, color) {
            self.set_custom_palette(colors, "Pick colour");
        } else {
            self.notice = Some("That colour is already in the palette".into());
        }
    }

    /// Replaces the palette with colours extracted from `img`.
    pub fn extract_palette(&mut self, img: &Rgba8Image) {
        let colors = extract_colors(img, self.extract_method, usize::from(self.extract_colors));
        if colors.is_empty() {
            self.error = Some("The image has no opaque pixels to take colours from".into());
        } else {
            self.set_custom_palette(colors, "Extract palette");
        }
    }

    /// Loads a palette file (.gpl, .hex, .pal, .act); any other file is read as
    /// an image and its colours extracted.
    pub fn import_palette(&mut self, path: &Path) -> Result<usize, String> {
        let colors = if PaletteFormat::from_path(path).is_some() {
            ditherr_io::read_palette(path)
                .map_err(|e| e.to_string())?
                .colors
        } else {
            let img = ditherr_io::read_image(path).map_err(|e| e.to_string())?;
            let colors =
                extract_colors(&img, self.extract_method, usize::from(self.extract_colors));
            if colors.is_empty() {
                return Err(format!("{}: no opaque pixels", path.display()));
            }
            colors
        };
        let n = colors.len();
        self.set_custom_palette(colors, "Import palette");
        Ok(n)
    }

    /// Writes the current palette to a palette file chosen by extension.
    pub fn export_palette(&self, path: &Path) -> Result<usize, String> {
        let colors = self
            .current_palette()
            .ok_or("There is no palette to export yet")?;
        let name = self.edit.look.name.trim();
        let file = PaletteFile {
            name: if name.is_empty() {
                "Ditherr palette".into()
            } else {
                name.into()
            },
            colors,
        };
        ditherr_io::write_palette(path, &file).map_err(|e| e.to_string())?;
        Ok(file.colors.len())
    }

    // ---- files ---------------------------------------------------------

    /// Loads a `.ditherlook` as a new undo step; layers it replaces are gone
    /// from the working copy but stay reachable through history.
    pub fn load_look(&mut self, path: &Path) -> Result<(), String> {
        let look = ditherr_io::read_look(path).map_err(|e| e.to_string())?;
        let label = if look.name.is_empty() {
            "Load look".to_string()
        } else {
            format!("Load {}", look.name)
        };
        self.edit = Edit::from_look(look);
        self.fix_selection();
        self.commit(Changes::step(label));
        Ok(())
    }

    /// Writes the current edit as a `.ditherlook`. Disabled layers are left out,
    /// because a look has no way to hold them.
    pub fn save_look(&self, path: &Path) -> Result<(), String> {
        let mut look = self.edit.look();
        if look.name.is_empty() {
            look.name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        ditherr_io::write_look(path, &look).map_err(|e| e.to_string())
    }

    pub fn open_project(&mut self, path: &Path) -> Result<(), String> {
        let p = ditherr_io::read_project(path).map_err(|e| e.to_string())?;
        self.history.reset("Open project", p.edit.clone());
        self.edit = p.edit;
        self.var_tex = vec![None; p.variations.len()];
        self.variations = p.variations;
        self.export = p.export;
        self.project_path = Some(path.to_owned());
        self.pending_source = p.source;
        self.fix_selection();
        self.dirty = true;
        self.thumbs_dirty = true;
        Ok(())
    }

    pub fn save_project(&mut self, path: &Path, source: Option<&Path>) -> Result<(), String> {
        let project = Project {
            source: source.map(Path::to_path_buf),
            edit: self.edit.clone(),
            variations: self.variations.clone(),
            export: self.export.clone(),
        };
        ditherr_io::write_project(path, &project).map_err(|e| e.to_string())?;
        self.project_path = Some(path.to_owned());
        Ok(())
    }

    /// Renders the full-size original in the background and writes it to `path`.
    pub fn export_to(&mut self, original: &Arc<Rgba8Image>, path: PathBuf) {
        let src = Arc::clone(original);
        let look = self.edit.look();
        let factor = self.export.factor;
        let (tx, rx) = channel();
        self.export_rx = Some(rx);
        self.notice = Some("Exporting…".into());
        std::thread::spawn(move || {
            let _ = tx.send(export_to(&src, &look, factor, &path));
        });
    }

    fn poll_export(&mut self) {
        let Some(rx) = &self.export_rx else { return };
        if let Ok(result) = rx.try_recv() {
            self.export_rx = None;
            match result {
                Ok(path) => self.notice = Some(format!("Saved {}", path.display())),
                Err(e) => {
                    self.notice = None;
                    self.error = Some(e);
                }
            }
        }
    }

    // ---- rendering -----------------------------------------------------

    fn submit_preview(&mut self, img: &Loaded) {
        let look = scaled_look(&self.edit.look(), img.preview_shrink);
        self.seq += 1;
        let batch = self.previews.begin_batch();
        self.previews.submit(
            batch,
            Job {
                key: PREVIEW_KEY,
                tag: self.seq,
                src: Arc::clone(&img.preview_src),
                look,
                measure: true,
            },
        );
    }

    fn submit_thumbs(&mut self, img: &Loaded) {
        self.thumb_epoch += 1;
        let batch = self.thumbs.begin_batch();
        for (idx, v) in self.variations.iter().enumerate() {
            self.thumbs.submit(
                batch,
                Job {
                    key: idx,
                    tag: self.thumb_epoch,
                    src: Arc::clone(&img.thumb_src),
                    look: scaled_look(&v.edit.look(), img.thumb_shrink),
                    measure: false,
                },
            );
        }
    }

    fn collect_results(&mut self, ctx: &egui::Context) {
        for done in self.previews.poll() {
            if done.tag < self.preview_min_tag {
                continue;
            }
            match done.result {
                Ok(out) => {
                    self.info = Info {
                        millis: out.took.as_millis(),
                        out_size: (out.image.width, out.image.height),
                        stats: out.stats,
                    };
                    self.error = None;
                    let color = to_color_image(&out.image);
                    match &mut self.preview_tex {
                        Some(t) => t.set(color, TextureOptions::NEAREST),
                        None => {
                            self.preview_tex = Some(ctx.load_texture(
                                "studio-preview",
                                color,
                                TextureOptions::NEAREST,
                            ))
                        }
                    }
                    self.preview_img = Some(out.image);
                }
                Err(e) => self.error = Some(e),
            }
        }
        for done in self.thumbs.poll() {
            if done.tag != self.thumb_epoch || done.key >= self.var_tex.len() {
                continue;
            }
            if let Ok(out) = done.result {
                self.var_tex[done.key] = Some(ctx.load_texture(
                    format!("variation-{}", done.key),
                    to_color_image(&out.image),
                    TextureOptions::NEAREST,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ditherr_core::{Algorithm, ErrorKernel, Rendered, Step};
    use std::time::{Duration, Instant};

    fn studio() -> Studio {
        Studio::new(Arc::new(|| {}))
    }

    fn photo(w: u32, h: u32) -> Rgba8Image {
        let data = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [
                    (x * 255 / w) as u8,
                    (y * 255 / h) as u8,
                    ((x ^ y) % 256) as u8,
                    255,
                ]
            })
            .collect();
        Rgba8Image::new(w, h, data).unwrap()
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ditherr-studio-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn undo_redo_and_labels_follow_edits() {
        let mut s = studio();
        s.add_layer("Contrast", Step::Contrast(0.3));
        s.edit.look.params.strength = 0.4;
        s.commit(Changes::step("Strength"));
        assert_eq!(s.history.len(), 3);
        assert_eq!(
            s.history.labels().collect::<Vec<_>>(),
            ["Start", "Add Contrast", "Strength"]
        );

        s.undo();
        assert_eq!(s.edit.look.params.strength, 1.0);
        assert_eq!(s.edit.layers.len(), 1);
        s.undo();
        assert!(s.edit.layers.is_empty());
        assert_eq!(s.selected, Selected::Dither, "selection falls back");
        s.redo();
        s.redo();
        assert_eq!(s.edit.look.params.strength, 0.4);
        assert_eq!(s.edit, *s.history.current());

        s.jump_to(0);
        assert_eq!(s.edit, Edit::default());
        assert!(s.dirty);
    }

    #[test]
    fn a_drag_is_one_undo_step() {
        let mut s = studio();
        for v in [0.9, 0.8, 0.7, 0.6] {
            s.edit.look.params.strength = v;
            s.commit(Changes {
                label: Some("Strength".into()),
                key: Some("Strength".into()),
            });
        }
        assert_eq!(s.history.len(), 2);
        s.undo();
        assert_eq!(s.edit.look.params.strength, 1.0);
    }

    #[test]
    fn layer_actions_keep_selection_valid() {
        let mut s = studio();
        s.add_layer("Exposure", Step::Exposure(1.0));
        s.add_layer("Contrast", Step::Contrast(0.2));
        s.add_layer("Flip vertical", Step::FlipVertical);
        assert_eq!(s.selected, Selected::Layer(2));

        s.move_layer(2, -2);
        assert_eq!(
            s.selected,
            Selected::Layer(0),
            "selection follows the layer"
        );
        assert_eq!(s.edit.layers[0].step, Step::FlipVertical);

        s.toggle_layer(0);
        assert!(!s.edit.layers[0].enabled);
        assert_eq!(s.edit.look().preprocess.len(), 2);

        s.delete_layer(0);
        s.delete_layer(0);
        s.delete_layer(0);
        assert_eq!(s.selected, Selected::Dither);
        s.delete_layer(5); // out of range: ignored
        assert_eq!(
            s.history.labels().last(),
            Some("Delete Contrast"),
            "{:?}",
            s.history.labels().collect::<Vec<_>>()
        );
    }

    #[test]
    fn variations_save_apply_and_delete() {
        let mut s = studio();
        s.edit.look.algorithm = Algorithm::Bayer { size: 8 };
        s.commit(Changes::step("Bayer"));
        s.save_variation();
        s.edit.look.algorithm = Algorithm::ErrorDiffusion(ErrorKernel::Atkinson);
        s.commit(Changes::step("Atkinson"));
        s.save_variation();
        assert_eq!(s.variations.len(), 2);
        assert_eq!(s.var_tex.len(), 2);
        assert!(s.thumbs_dirty);

        s.apply_variation(0);
        assert_eq!(s.edit.look.algorithm, Algorithm::Bayer { size: 8 });
        assert_eq!(s.history.labels().last(), Some("Apply Variation 1"));
        s.undo();
        assert_eq!(
            s.edit.look.algorithm,
            Algorithm::ErrorDiffusion(ErrorKernel::Atkinson)
        );

        s.delete_variation(0);
        assert_eq!(s.variations.len(), 1);
        assert_eq!(s.var_tex.len(), 1);
        s.delete_variation(9);
        assert_eq!(s.variations.len(), 1);
    }

    #[test]
    fn palette_editing_actions() {
        let mut s = studio();
        s.edit.look.palette = PaletteSpec::Preset("gameboy".into());
        s.make_palette_custom();
        let PaletteSpec::Custom(c) = &s.edit.look.palette else {
            panic!("not custom")
        };
        assert_eq!(c.len(), 4);

        s.pick_color([1, 2, 3]);
        let PaletteSpec::Custom(c) = &s.edit.look.palette else {
            panic!()
        };
        assert_eq!(c.len(), 5);
        assert_eq!(c.last(), Some(&[1, 2, 3]));

        let steps = s.history.len();
        s.pick_color([1, 2, 3]);
        assert_eq!(s.history.len(), steps, "a duplicate colour is not a step");
        assert!(s.notice.is_some());

        // Picking onto a built-in palette starts from its colours.
        let mut s = studio();
        s.edit.look.palette = PaletteSpec::Gray { levels: 3 };
        s.pick_color([9, 9, 9]);
        let PaletteSpec::Custom(c) = &s.edit.look.palette else {
            panic!()
        };
        assert_eq!(c.len(), 4);
    }

    #[test]
    fn extract_and_palette_files() {
        let dir = scratch("pal");
        let mut s = studio();
        let img = photo(32, 32);
        s.extract_colors = 5;
        s.extract_palette(&img);
        let PaletteSpec::Custom(extracted) = s.edit.look.palette.clone() else {
            panic!()
        };
        assert!(!extracted.is_empty() && extracted.len() <= 5);

        for ext in ["gpl", "hex", "pal", "act"] {
            let path = dir.join(format!("p.{ext}"));
            assert_eq!(s.export_palette(&path).unwrap(), extracted.len());
            let mut other = studio();
            assert_eq!(other.import_palette(&path).unwrap(), extracted.len());
            assert_eq!(
                other.edit.look.palette,
                PaletteSpec::Custom(extracted.clone())
            );
            assert_eq!(other.history.labels().last(), Some("Import palette"));
        }

        // Any image works as a palette source.
        let png = dir.join("src.png");
        let data = (0..64)
            .flat_map(|i| {
                if i % 8 < 4 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                }
            })
            .collect();
        let two_tone = Rgba8Image::new(8, 8, data).unwrap();
        ditherr_io::save(&png, &Rendered::Truecolor(two_tone)).unwrap();
        let mut other = studio();
        other.extract_colors = 2;
        assert_eq!(other.import_palette(&png).unwrap(), 2);

        assert!(other.import_palette(&dir.join("missing.gpl")).is_err());
        // Nothing to export before a render when the palette depends on the image.
        let mut fresh = studio();
        fresh.edit.look.palette = PaletteSpec::Auto {
            method: AutoMethod::KMeans,
            colors: 4,
        };
        assert!(fresh.export_palette(&dir.join("x.gpl")).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn projects_and_looks_round_trip() {
        let dir = scratch("proj");
        let mut s = studio();
        s.add_layer("Exposure", Step::Exposure(0.5));
        s.toggle_layer(0);
        s.edit.look.pixel_scale = 4;
        s.commit(Changes::step("Pixel scale"));
        s.save_variation();
        s.export = ExportSettings {
            format: "gif".into(),
            factor: 3,
        };
        let src = dir.join("img/cat.png");
        let path = dir.join("a.ditherproj");
        s.save_project(&path, Some(&src)).unwrap();

        let mut back = studio();
        back.open_project(&path).unwrap();
        assert_eq!(back.edit, s.edit);
        assert!(!back.edit.layers[0].enabled, "disabled layers survive");
        assert_eq!(back.variations, s.variations);
        assert_eq!(back.export, s.export);
        assert_eq!(back.take_pending_source(), Some(src));
        assert_eq!(back.take_pending_source(), None);
        assert_eq!(back.history.len(), 1, "opening starts a new history");
        assert!(back.open_project(&dir.join("nope.ditherproj")).is_err());

        // A look file holds only what the engine runs: disabled layers are dropped.
        let look_path = dir.join("mine.ditherlook");
        s.save_look(&look_path).unwrap();
        let look = ditherr_io::read_look(&look_path).unwrap();
        assert_eq!(look.name, "mine");
        assert_eq!(look.pixel_scale, 4);
        assert!(look.preprocess.is_empty());

        let mut loader = studio();
        loader.load_look(&look_path).unwrap();
        assert_eq!(loader.edit.look.pixel_scale, 4);
        assert_eq!(loader.history.labels().last(), Some("Load mine"));
        std::fs::remove_dir_all(dir).ok();
    }

    // ---- with a live render thread -----------------------------------------

    fn frame(ctx: &egui::Context, s: &mut Studio, img: &Loaded) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| s.show(ui, Some(img)));
        out.textures_delta.clear();
    }

    fn pump(ctx: &egui::Context, s: &mut Studio, img: &Loaded, done: impl Fn(&Studio) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done(s) && Instant::now() < deadline {
            frame(ctx, s, img);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(done(s), "timed out; error: {:?}", s.error);
    }

    #[test]
    fn renders_measures_and_follows_edits() {
        let ctx = egui::Context::default();
        let mut s = Studio::new(Arc::new(|| {}));
        let img = Loaded::new(&ctx, "t.png".into(), photo(96, 64), None);
        s.image_changed();
        pump(&ctx, &mut s, &img, |s| s.info.stats.is_some());
        let stats = s.info.stats.clone().unwrap();
        assert_eq!(stats.colors, 2, "default look is black and white");
        assert_eq!(stats.palette.len(), 2);
        assert!(stats.png_bytes > 0);
        assert_eq!(s.info.out_size, (96, 64));
        assert!(s.preview_img.is_some());

        // Pixel scale 4 shrinks the working image; Game Boy has four colours.
        s.edit.look.pixel_scale = 4;
        s.edit.look.palette = PaletteSpec::Preset("gameboy".into());
        s.commit(Changes::step("Look"));
        pump(&ctx, &mut s, &img, |s| {
            s.info
                .stats
                .as_ref()
                .is_some_and(|st| st.palette.len() == 4)
        });
        let hist: u32 = s.info.stats.as_ref().unwrap().histogram.iter().sum();
        assert_eq!(hist, 24 * 16, "the histogram counts the 4x-reduced image");

        // A disabled layer does not reach the engine.
        s.add_layer(
            "Crop",
            Step::Crop {
                x: 0,
                y: 0,
                width: 40,
                height: 40,
            },
        );
        pump(&ctx, &mut s, &img, |s| s.info.out_size == (40, 40));
        s.toggle_layer(0);
        pump(&ctx, &mut s, &img, |s| s.info.out_size == (96, 64));

        // Undo returns to the earlier look and renders it again.
        s.jump_to(0);
        pump(&ctx, &mut s, &img, |s| {
            s.info
                .stats
                .as_ref()
                .is_some_and(|st| st.palette.len() == 2)
        });
        assert!(s.error.is_none());
    }

    #[test]
    fn bad_values_report_an_error_and_keep_the_last_preview() {
        let ctx = egui::Context::default();
        let mut s = Studio::new(Arc::new(|| {}));
        let img = Loaded::new(&ctx, "t".into(), photo(32, 32), None);
        s.image_changed();
        pump(&ctx, &mut s, &img, |s| s.preview_tex.is_some());

        // An empty custom palette is invalid.
        s.edit.look.palette = PaletteSpec::Custom(Vec::new());
        s.commit(Changes::step("Break"));
        pump(&ctx, &mut s, &img, |s| s.error.is_some());
        assert!(s.preview_tex.is_some());

        s.undo();
        pump(&ctx, &mut s, &img, |s| s.error.is_none());
    }

    #[test]
    fn variation_thumbnails_render() {
        let ctx = egui::Context::default();
        let mut s = Studio::new(Arc::new(|| {}));
        let img = Loaded::new(&ctx, "t".into(), photo(64, 48), None);
        s.image_changed();
        s.save_variation();
        s.edit.look.palette = PaletteSpec::Preset("cga".into());
        s.commit(Changes::step("CGA"));
        s.save_variation();
        pump(&ctx, &mut s, &img, |s| {
            s.var_tex.iter().all(Option::is_some)
        });
        s.delete_variation(0);
        pump(&ctx, &mut s, &img, |s| {
            s.var_tex.len() == 1 && s.var_tex[0].is_some()
        });
    }

    #[test]
    fn export_writes_the_full_size_original() {
        let mut s = Studio::new(Arc::new(|| {}));
        let big = Arc::new(photo(100, 60));
        s.export.factor = 2;
        let dir = scratch("export");
        let path = dir.join("out.png");
        s.export_to(&big, path.clone());
        let deadline = Instant::now() + Duration::from_secs(30);
        while s.export_rx.is_some() && Instant::now() < deadline {
            s.poll_export();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(s.error.is_none(), "{:?}", s.error);
        assert!(s.notice.as_deref().unwrap_or("").starts_with("Saved"));
        let back = ditherr_io::read_image(&path).unwrap();
        assert_eq!((back.width, back.height), (200, 120));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn every_panel_and_compare_mode_draws() {
        use ditherr_core::{Resampler, ResizeTarget, Rotation};

        let ctx = egui::Context::default();
        let mut s = Studio::new(Arc::new(|| {}));
        let img = Loaded::new(&ctx, "t".into(), photo(80, 60), None);
        s.image_changed();
        pump(&ctx, &mut s, &img, |s| s.preview_tex.is_some());

        // One of every layer, so each parameter panel gets drawn.
        for (name, step) in ops::new_layer_kinds() {
            s.add_layer(name, step);
        }
        s.add_layer(
            "Resize",
            Step::Resize {
                target: ResizeTarget::Pixels {
                    width: 50,
                    height: 40,
                },
                resampler: Resampler::Bilinear,
            },
        );
        s.add_layer("Rotate", Step::Rotate(Rotation::Cw180));
        let mut views = vec![Selected::PixelScale, Selected::Dither, Selected::Palette];
        views.extend((0..s.edit.layers.len()).map(Selected::Layer));

        let palettes = [
            PaletteSpec::Gray { levels: 4 },
            PaletteSpec::Levels { r: 4, g: 4, b: 4 },
            PaletteSpec::Preset("c64".into()),
            PaletteSpec::Custom(vec![[0, 0, 0], [255, 0, 0], [255, 255, 255]]),
            PaletteSpec::Auto {
                method: AutoMethod::Octree,
                colors: 6,
            },
        ];
        let algorithms = [
            Algorithm::ErrorDiffusion(ErrorKernel::Stucki),
            Algorithm::Bayer { size: 4 },
            Algorithm::BlueNoise,
            Algorithm::WhiteNoise,
            Algorithm::Threshold { level: 0.4 },
            Algorithm::Posterize,
        ];

        for selected in views {
            for compare in [
                CompareMode::Single,
                CompareMode::Split,
                CompareMode::SideBySide,
            ] {
                s.selected = selected;
                s.compare = compare;
                s.eyedropper = compare == CompareMode::Split;
                s.show_original = compare == CompareMode::Single && selected == Selected::Palette;
                for _ in 0..2 {
                    frame(&ctx, &mut s, &img);
                }
            }
        }
        for palette in palettes {
            s.edit.look.palette = palette;
            s.selected = Selected::Palette;
            s.commit(Changes::step("palette"));
            for _ in 0..2 {
                frame(&ctx, &mut s, &img);
            }
        }
        for algorithm in algorithms {
            s.edit.look.algorithm = algorithm;
            s.selected = Selected::Dither;
            s.commit(Changes::step("algorithm"));
            for _ in 0..2 {
                frame(&ctx, &mut s, &img);
            }
        }
        // Zoomed in with a pan, in every compare mode.
        s.view.zoom = crate::view::Zoom::Pixels(4);
        s.view.pan = egui::vec2(30.0, -20.0);
        for compare in [
            CompareMode::Single,
            CompareMode::Split,
            CompareMode::SideBySide,
        ] {
            s.compare = compare;
            frame(&ctx, &mut s, &img);
        }
        // Nothing loaded: the empty states draw too.
        let mut empty = Studio::new(Arc::new(|| {}));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| empty.show(ui, None));
        out.textures_delta.clear();
    }
}
