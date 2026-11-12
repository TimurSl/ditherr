//! The Quick Stylize window (spec §7): open an image, pick a look from a
//! gallery of live thumbnails, adjust five controls, export.
//!
//! The UI holds no pixel logic. It builds a [`Look`], hands it to the render
//! [`Worker`], and shows what comes back.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use ditherr_core::{presets, Look, Rgba8Image};
use ditherr_io::Edit;
use eframe::egui::{self, Color32, TextureHandle, TextureOptions};

use crate::loaded::{to_color_image, Loaded};
use crate::quick::{
    self, apply_settings, starter_looks, Entry, PaletteChoice, Settings, COLOR_RANGE,
    CONTRAST_RANGE, PIXEL_SIZE_RANGE, STRENGTH_RANGE,
};
use crate::studio::Studio;
use crate::worker::{scaled_look, Job, Worker, THUMB_EDGE};

const PREVIEW_KEY: usize = 0;
/// Random variations kept in the gallery; the oldest is dropped beyond this.
const MAX_VARIATIONS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Png,
    Gif,
}

impl ExportFormat {
    fn extension(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Gif => "gif",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Workspace {
    Quick,
    Studio,
}

#[derive(Default)]
struct Info {
    colors: usize,
    millis: u128,
    out_size: (u32, u32),
}

pub struct App {
    workspace: Workspace,
    studio: Studio,
    previews: Worker,
    thumbs: Worker,
    image: Option<Loaded>,

    gallery: Vec<Entry>,
    thumb_tex: Vec<Option<TextureHandle>>,
    selected: usize,
    variations_made: u64,

    settings: Settings,
    preview_tex: Option<TextureHandle>,
    show_original: bool,
    info: Info,
    error: Option<String>,

    export_format: ExportFormat,
    export_factor: u32,
    export_rx: Option<Receiver<Result<PathBuf, String>>>,
    notice: Option<String>,

    seq: u64,
    /// Preview results older than this belong to a previous image.
    preview_min_tag: u64,
    thumb_epoch: u64,
    thumb_batch: u64,
    dirty: bool,
}

impl App {
    pub fn new(ctx: &egui::Context, open: Option<&Path>) -> Self {
        let repaint = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        let gallery = starter_looks();
        let settings = Settings::from_look(&gallery[0].look);
        let mut app = Self {
            workspace: Workspace::Quick,
            studio: Studio::new(repaint.clone()),
            previews: Worker::new(repaint.clone()),
            thumbs: Worker::new(repaint),
            image: None,
            thumb_tex: vec![None; gallery.len()],
            gallery,
            selected: 0,
            variations_made: 0,
            settings,
            preview_tex: None,
            show_original: false,
            info: Info::default(),
            error: None,
            export_format: ExportFormat::Png,
            export_factor: 1,
            export_rx: None,
            notice: None,
            seq: 0,
            preview_min_tag: 0,
            thumb_epoch: 0,
            thumb_batch: 0,
            dirty: false,
        };
        if let Some(path) = open {
            if is_project(path) {
                app.open_project(path);
            } else {
                app.open_path(ctx, path);
            }
        }
        app
    }

    // ---- loading -------------------------------------------------------

    fn open_path(&mut self, ctx: &egui::Context, path: &Path) {
        match ditherr_io::read_image(path) {
            Ok(img) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "image".into());
                self.set_image(ctx, name, img);
                if let Some(loaded) = &mut self.image {
                    loaded.path = Some(path.to_path_buf());
                }
            }
            Err(e) => {
                let message = e.to_string();
                self.studio.report_error(message.clone());
                self.error = Some(message);
            }
        }
    }

    fn set_image(&mut self, ctx: &egui::Context, name: String, img: Rgba8Image) {
        self.image = Some(Loaded::new(ctx, name, img, None));
        self.error = None;
        self.notice = None;
        self.preview_tex = None;
        self.preview_min_tag = self.seq + 1;
        for t in &mut self.thumb_tex {
            *t = None;
        }
        self.submit_all_thumbs();
        self.dirty = true;
        self.studio.image_changed();
    }

    /// Opens a project file: Studio takes the edit and asks for its source
    /// image, which [`App::show`] then loads.
    fn open_project(&mut self, path: &Path) {
        match self.studio.open_project(path) {
            Ok(()) => self.workspace = Workspace::Studio,
            Err(e) => self.studio.report_error(e),
        }
    }

    /// Hands the current Quick Stylize look to Studio with every setting exposed.
    fn open_in_studio(&mut self) {
        let look = self.current_look();
        self.studio
            .load_edit(Edit::from_look(look), "From Quick Stylize");
        self.workspace = Workspace::Studio;
    }

    // ---- rendering -----------------------------------------------------

    fn current_look(&self) -> Look {
        apply_settings(&self.gallery[self.selected].look, &self.settings)
    }

    fn submit_preview(&mut self) {
        let Some(img) = &self.image else { return };
        let look = scaled_look(&self.current_look(), img.preview_shrink);
        self.seq += 1;
        let batch = self.previews.begin_batch();
        self.previews.submit(
            batch,
            Job {
                key: PREVIEW_KEY,
                tag: self.seq,
                src: Arc::clone(&img.preview_src),
                look,
                measure: false,
            },
        );
    }

    fn thumb_job(&self, idx: usize) -> Option<Job> {
        let img = self.image.as_ref()?;
        Some(Job {
            key: idx,
            tag: self.thumb_epoch,
            src: Arc::clone(&img.thumb_src),
            look: scaled_look(&self.gallery[idx].look, img.thumb_shrink),
            measure: false,
        })
    }

    fn submit_all_thumbs(&mut self) {
        self.thumb_epoch += 1;
        self.thumb_batch = self.thumbs.begin_batch();
        for idx in 0..self.gallery.len() {
            if let Some(job) = self.thumb_job(idx) {
                self.thumbs.submit(self.thumb_batch, job);
            }
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
                        colors: out.colors,
                        millis: out.took.as_millis(),
                        out_size: (out.image.width, out.image.height),
                    };
                    self.error = None;
                    let img = to_color_image(&out.image);
                    match &mut self.preview_tex {
                        Some(t) => t.set(img, TextureOptions::NEAREST),
                        None => {
                            self.preview_tex =
                                Some(ctx.load_texture("preview", img, TextureOptions::NEAREST))
                        }
                    }
                }
                Err(e) => self.error = Some(e),
            }
        }
        for done in self.thumbs.poll() {
            if done.tag != self.thumb_epoch || done.key >= self.thumb_tex.len() {
                continue;
            }
            if let Ok(out) = done.result {
                self.thumb_tex[done.key] = Some(ctx.load_texture(
                    format!("thumb-{}", done.key),
                    to_color_image(&out.image),
                    TextureOptions::NEAREST,
                ));
            }
        }
    }

    // ---- actions -------------------------------------------------------

    fn select(&mut self, idx: usize) {
        if idx != self.selected {
            self.selected = idx;
            self.settings = Settings::from_look(&self.gallery[idx].look);
            self.dirty = true;
        }
    }

    fn randomize(&mut self) {
        self.variations_made += 1;
        let n = self.variations_made;
        let seed = n.wrapping_mul(0x2545_f491_4f6c_dd1d) ^ self.seq;
        let name = format!("Variation {n}");
        let look = Look {
            name: name.clone(),
            ..quick::randomize(seed)
        };

        // Keep the starters and the newest variations.
        let starters = starter_looks().len();
        if self.gallery.len() - starters >= MAX_VARIATIONS {
            self.gallery.remove(starters);
            self.thumb_tex.remove(starters);
            // Indices shifted, so the thumbnails are rendered again.
            self.submit_all_thumbs_after_push(Entry { name, look });
        } else {
            self.push_variation(Entry { name, look });
        }
        self.selected = self.gallery.len() - 1;
        self.settings = Settings::from_look(&self.gallery[self.selected].look);
        self.dirty = true;
    }

    fn push_variation(&mut self, entry: Entry) {
        self.gallery.push(entry);
        self.thumb_tex.push(None);
        let idx = self.gallery.len() - 1;
        if let Some(job) = self.thumb_job(idx) {
            self.thumbs.submit(self.thumb_batch, job);
        }
    }

    fn submit_all_thumbs_after_push(&mut self, entry: Entry) {
        self.gallery.push(entry);
        self.thumb_tex.push(None);
        self.submit_all_thumbs();
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter(
                "Images",
                &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif"],
            )
            .pick_file();
        if let Some(path) = picked {
            self.open_path(ctx, &path);
        }
    }

    fn export(&mut self) {
        let Some(img) = &self.image else { return };
        let stem = Path::new(&img.name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image".into());
        let ext = self.export_format.extension();
        let Some(path) = rfd::FileDialog::new()
            .add_filter(ext, &[ext])
            .set_file_name(format!("{stem}-ditherr.{ext}"))
            .save_file()
        else {
            return;
        };

        let src = Arc::clone(&img.original);
        let look = self.current_look();
        let factor = self.export_factor;
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

    // ---- UI ------------------------------------------------------------

    /// Draws the whole window into `ui`.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.collect_results(&ctx);
        self.poll_export();
        self.handle_drops(&ctx);

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui, &ctx));
        // A project that was just opened names the image it was made from.
        if let Some(source) = self.studio.take_pending_source() {
            self.open_path(&ctx, &source);
        }

        match self.workspace {
            Workspace::Quick => {
                egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
                egui::Panel::left("looks")
                    .default_size(340.0)
                    .min_size(260.0)
                    .show(ui, |ui| self.side_panel(ui));
                egui::CentralPanel::default().show(ui, |ui| self.preview(ui));

                // Quick Stylize renders only while it is on screen.
                if self.dirty {
                    self.dirty = false;
                    self.submit_preview();
                }
            }
            Workspace::Studio => self.studio.show(ui, self.image.as_ref()),
        }
        if self.export_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        // Desktop only: a dropped file always has a real path. The last one wins.
        if let Some(file) = dropped.last() {
            let path = file.path().to_path_buf();
            if is_project(&path) {
                self.open_project(&path);
            } else {
                self.open_path(ctx, &path);
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.heading("Ditherr");
            ui.separator();
            ui.selectable_value(&mut self.workspace, Workspace::Quick, "Quick Stylize");
            ui.selectable_value(&mut self.workspace, Workspace::Studio, "Studio");
            ui.separator();
            if ui.button("Open…").clicked() {
                self.open_dialog(ctx);
            }
            if let Some(img) = &self.image {
                ui.label(format!(
                    "{} ({}×{})",
                    img.name, img.original.width, img.original.height
                ));
            }
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match self.workspace {
                    Workspace::Quick => self.quick_export_controls(ui),
                    Workspace::Studio => self.studio.toolbar(ui, self.image.as_ref()),
                },
            );
        });
    }

    fn quick_export_controls(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.image.is_some() && self.export_rx.is_none(), |ui| {
            if ui.button("Export").clicked() {
                self.export();
            }
        });
        egui::ComboBox::from_id_salt("scale")
            .selected_text(format!("{}×", self.export_factor))
            .width(50.0)
            .show_ui(ui, |ui| {
                for f in [1, 2, 4] {
                    ui.selectable_value(&mut self.export_factor, f, format!("{f}×"));
                }
            });
        egui::ComboBox::from_id_salt("format")
            .selected_text(self.export_format.extension().to_uppercase())
            .width(60.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.export_format, ExportFormat::Png, "PNG");
                ui.selectable_value(&mut self.export_format, ExportFormat::Gif, "GIF");
            });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);
            ui.strong("Looks");
            let mut clicked = None;
            ui.horizontal_wrapped(|ui| {
                for (idx, entry) in self.gallery.iter().enumerate() {
                    if thumbnail(
                        ui,
                        &entry.name,
                        self.thumb_tex[idx].as_ref(),
                        idx == self.selected,
                    )
                    .clicked()
                    {
                        clicked = Some(idx);
                    }
                }
            });
            if let Some(idx) = clicked {
                self.select(idx);
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Randomize").clicked() {
                    self.randomize();
                }
                if ui
                    .button("Open in Studio")
                    .on_hover_text("Edit this look with every setting exposed")
                    .clicked()
                {
                    self.open_in_studio();
                }
            });

            ui.separator();
            ui.strong("Adjust");
            self.sliders(ui);
        });
    }

    fn sliders(&mut self, ui: &mut egui::Ui) {
        let base = self.gallery[self.selected].look.clone();
        let mut changed = false;

        egui::Grid::new("sliders").num_columns(2).show(ui, |ui| {
            ui.label("Pixel size");
            changed |= ui
                .add(egui::Slider::new(
                    &mut self.settings.pixel_size,
                    PIXEL_SIZE_RANGE.0..=PIXEL_SIZE_RANGE.1,
                ))
                .changed();
            ui.end_row();

            ui.label("Colours");
            let adjustable = self.settings.colors_adjustable(&base);
            ui.add_enabled_ui(adjustable, |ui| {
                changed |= ui
                    .add(egui::Slider::new(
                        &mut self.settings.colors,
                        COLOR_RANGE.0..=COLOR_RANGE.1,
                    ))
                    .on_disabled_hover_text("This palette has a fixed set of colours")
                    .changed();
            });
            ui.end_row();

            ui.label("Dither strength");
            changed |= ui
                .add(egui::Slider::new(
                    &mut self.settings.strength,
                    STRENGTH_RANGE.0..=STRENGTH_RANGE.1,
                ))
                .changed();
            ui.end_row();

            ui.label("Contrast");
            changed |= ui
                .add(egui::Slider::new(
                    &mut self.settings.contrast,
                    CONTRAST_RANGE.0..=CONTRAST_RANGE.1,
                ))
                .changed();
            ui.end_row();

            ui.label("Palette");
            changed |= palette_picker(ui, &mut self.settings.palette);
            ui.end_row();
        });

        if ui.button("Reset").clicked() {
            self.settings = Settings::from_look(&base);
            changed = true;
        }
        self.dirty |= changed;
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let Some(img) = &self.image else {
            ui.centered_and_justified(|ui| {
                ui.label("Drop an image here, or press Open…");
            });
            return;
        };

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.show_original, "Show original");
            ui.label("(hold Space to peek)");
        });
        let peek = self.show_original || ui.input(|i| i.key_down(egui::Key::Space));

        let tex = if peek {
            Some(&img.original_tex)
        } else {
            self.preview_tex.as_ref()
        };
        let avail = ui.available_size();
        let (rect, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, Color32::from_gray(24));

        let Some(tex) = tex else {
            ui.put(rect, egui::Spinner::new());
            return;
        };
        let size = tex.size_vec2();
        let ppp = ui.pixels_per_point();
        let fit = (rect.width() / size.x).min(rect.height() / size.y);
        // Whole device pixels per image pixel when zoomed in, so nothing blurs.
        let scale = if fit * ppp >= 1.0 {
            (fit * ppp).floor() / ppp
        } else {
            fit
        };
        let shown = egui::Rect::from_center_size(rect.center(), size * scale);
        ui.painter().image(
            tex.id(),
            shown,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(e) = &self.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            } else if let Some(n) = &self.notice {
                ui.label(n);
            } else if self.image.is_some() {
                ui.label(format!(
                    "{}×{} · {} colours · {} ms",
                    self.info.out_size.0, self.info.out_size.1, self.info.colors, self.info.millis
                ));
            } else {
                ui.label("No image loaded");
            }
        });
    }
}

fn palette_picker(ui: &mut egui::Ui, choice: &mut PaletteChoice) -> bool {
    let label = match choice {
        PaletteChoice::Look => "Look's own".to_string(),
        PaletteChoice::Gray => "Grays".to_string(),
        PaletteChoice::FromImage => "From image".to_string(),
        PaletteChoice::Preset(id) => presets::find(id).map_or(id.clone(), |p| p.name.to_string()),
    };
    let before = choice.clone();
    egui::ComboBox::from_id_salt("palette")
        .selected_text(label)
        .width(170.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(choice, PaletteChoice::Look, "Look's own");
            ui.selectable_value(choice, PaletteChoice::Gray, "Grays");
            ui.selectable_value(choice, PaletteChoice::FromImage, "From image");
            ui.separator();
            for p in presets::PRESETS {
                ui.selectable_value(choice, PaletteChoice::Preset(p.id.into()), p.name);
            }
        });
    *choice != before
}

/// Whether `path` names a Ditherr project file.
fn is_project(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ditherr_io::PROJECT_EXTENSION))
}

pub(crate) fn thumbnail(
    ui: &mut egui::Ui,
    name: &str,
    tex: Option<&TextureHandle>,
    selected: bool,
) -> egui::Response {
    let edge = THUMB_EDGE as f32 * 0.6;
    let size = egui::vec2(edge * 1.4, edge * 1.1 + 18.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();
    let visuals = ui.visuals();
    if selected {
        painter.rect_filled(rect, 4.0, visuals.selection.bg_fill);
    } else if response.hovered() {
        painter.rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
    }

    let area = egui::Rect::from_min_size(
        rect.min + egui::vec2(4.0, 4.0),
        egui::vec2(size.x - 8.0, edge * 1.1 - 4.0),
    );
    painter.rect_filled(area, 2.0, Color32::from_gray(30));
    if let Some(tex) = tex {
        let t = tex.size_vec2();
        let s = (area.width() / t.x).min(area.height() / t.y);
        let shown = egui::Rect::from_center_size(area.center(), t * s);
        painter.image(
            tex.id(),
            shown,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    painter.text(
        egui::pos2(rect.center().x, rect.max.y - 3.0),
        egui::Align2::CENTER_BOTTOM,
        name,
        egui::FontId::proportional(12.0),
        if selected {
            visuals.strong_text_color()
        } else {
            visuals.text_color()
        },
    );
    response.on_hover_text(name)
}

/// Renders the original image with `look`, upscales so that `factor == 1`
/// matches the original size, and writes it to `path`.
pub(crate) fn export_to(
    src: &Rgba8Image,
    look: &Look,
    factor: u32,
    path: &Path,
) -> Result<PathBuf, String> {
    let rendered = ditherr_core::render(src, look).map_err(|e| e.to_string())?;
    let out = rendered.upscale(look.pixel_scale.max(1) * factor.max(1));
    ditherr_io::save(path, &out).map_err(|e| e.to_string())?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

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

    fn frame(ctx: &egui::Context, app: &mut App) {
        // A real backend uploads the texture deltas; the test just discards them.
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.show(ui));
        out.textures_delta.clear();
    }

    /// Runs frames until `done` holds or a deadline passes.
    fn pump(ctx: &egui::Context, app: &mut App, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done(app) && Instant::now() < deadline {
            frame(ctx, app);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(done(app), "timed out; error: {:?}", app.error);
    }

    #[test]
    fn loads_an_image_and_fills_preview_and_gallery() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, None);
        frame(&ctx, &mut app); // empty state draws

        app.set_image(&ctx, "test.png".into(), photo(120, 80));
        pump(&ctx, &mut app, |a| {
            a.preview_tex.is_some() && a.thumb_tex.iter().all(Option::is_some)
        });
        assert!(app.error.is_none());
        // Game Boy: 4 colours at most, output at the source size.
        assert!(app.info.colors <= 4);
        assert_eq!(app.info.out_size, (120, 81));
    }

    #[test]
    fn selecting_and_adjusting_rerenders() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, None);
        app.set_image(&ctx, "t".into(), photo(96, 64));
        pump(&ctx, &mut app, |a| a.preview_tex.is_some());

        // Newspaper is 2-colour gray; ask for 6 levels.
        app.select(1);
        app.settings.colors = 6;
        app.dirty = true;
        pump(&ctx, &mut app, |a| a.info.colors > 2);
        assert!(app.info.colors <= 6);
    }

    #[test]
    fn randomize_adds_a_selected_variation_and_caps_the_list() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, None);
        app.set_image(&ctx, "t".into(), photo(64, 48));
        let starters = app.gallery.len();
        for _ in 0..(MAX_VARIATIONS + 3) {
            app.randomize();
        }
        assert_eq!(app.gallery.len(), starters + MAX_VARIATIONS);
        assert_eq!(app.selected, app.gallery.len() - 1);
        assert_eq!(app.thumb_tex.len(), app.gallery.len());
        pump(&ctx, &mut app, |a| {
            a.preview_tex.is_some() && a.thumb_tex.iter().all(Option::is_some)
        });
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn a_new_image_discards_results_for_the_old_one() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, None);
        app.set_image(&ctx, "a".into(), photo(200, 100));
        app.set_image(&ctx, "b".into(), photo(50, 30));
        pump(&ctx, &mut app, |a| a.preview_tex.is_some());
        // Game Boy renders at scale 3: 50x30 becomes 17x10 pixels, shown at 51x30.
        pump(&ctx, &mut app, |a| a.info.out_size == (51, 30));
        // Later frames never bring back the first image's 201x102 result.
        for _ in 0..20 {
            frame(&ctx, &mut app);
            std::thread::sleep(Duration::from_millis(10));
            assert_eq!(app.info.out_size, (51, 30));
        }
    }

    #[test]
    fn export_matches_the_original_size_at_one_x() {
        let src = photo(100, 60);
        let looks = starter_looks();
        let dir = std::env::temp_dir().join(format!("ditherr-app-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        for (entry, ext) in [(&looks[0], "png"), (&looks[1], "gif")] {
            let path = dir.join(format!("out.{ext}"));
            export_to(&src, &entry.look, 1, &path).unwrap();
            let back = ditherr_io::read_image(&path).unwrap();
            let s = entry.look.pixel_scale;
            // Rounded up to a whole number of output pixels.
            assert_eq!(back.width, 100u32.div_ceil(s) * s, "{}", entry.name);
            assert_eq!(back.height, 60u32.div_ceil(s) * s, "{}", entry.name);

            let path2 = dir.join(format!("out2.{ext}"));
            export_to(&src, &entry.look, 2, &path2).unwrap();
            let back2 = ditherr_io::read_image(&path2).unwrap();
            assert_eq!(back2.width, back.width * 2);
        }
        std::fs::remove_dir_all(dir).ok();
    }
}
