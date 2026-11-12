//! Studio's window layout: toolbar, effect stack, variations, history, info
//! panel and the comparison canvas.

use std::time::Duration;

use eframe::egui::{
    self, pos2, vec2, Align, Color32, CursorIcon, Key, KeyboardShortcut, Layout, Modifiers, Rect,
    Sense, Stroke, Ui,
};

use super::panels::{
    algorithm_summary, dither_panel, layer_panel, palette_summary, pixel_scale_panel, swatch_strip,
};
use super::{ops, Changes, CompareMode, Hover, Selected, Studio, EXPORT_FACTORS, EXPORT_FORMATS};
use crate::app::thumbnail;
use crate::info::{estimate_full_size, format_bytes};
use crate::loaded::Loaded;
use crate::view::pixel_at;
use crate::worker::scaled_look;

/// The small glyphs of the stack and palette rows. egui's bundled fonts lack
/// arrows and crosses, so they are drawn as shapes.
#[derive(Clone, Copy)]
pub(super) enum Icon {
    Up,
    Down,
    Cross,
}

/// A small square button showing `icon`; greyed out and inert when not `enabled`.
pub(super) fn icon_button(ui: &mut Ui, icon: Icon, enabled: bool, tip: &str) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), sense);
    let visuals = ui.style().interact(&response);
    if enabled && response.hovered() {
        ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
    }
    let color = if enabled {
        visuals.fg_stroke.color
    } else {
        ui.visuals().weak_text_color().gamma_multiply(0.5)
    };
    let stroke = Stroke::new(1.6, color);
    let c = rect.center();
    let r = 3.5;
    let painter = ui.painter();
    match icon {
        Icon::Up => painter.add(egui::Shape::line(
            vec![
                pos2(c.x - r, c.y + r * 0.6),
                pos2(c.x, c.y - r * 0.6),
                pos2(c.x + r, c.y + r * 0.6),
            ],
            stroke,
        )),
        Icon::Down => painter.add(egui::Shape::line(
            vec![
                pos2(c.x - r, c.y - r * 0.6),
                pos2(c.x, c.y + r * 0.6),
                pos2(c.x + r, c.y - r * 0.6),
            ],
            stroke,
        )),
        Icon::Cross => {
            painter.line_segment([pos2(c.x - r, c.y - r), pos2(c.x + r, c.y + r)], stroke);
            painter.line_segment([pos2(c.x - r, c.y + r), pos2(c.x + r, c.y - r)], stroke)
        }
    };
    response.on_hover_text(tip)
}

/// Gap between the two images of the side-by-side view.
const GAP: f32 = 6.0;

#[derive(Clone, Copy)]
enum ZoomCmd {
    In,
    Out,
    Fit,
}

impl Studio {
    /// Draws the whole workspace below the app's top bar.
    pub fn show(&mut self, ui: &mut Ui, loaded: Option<&Loaded>) {
        let ctx = ui.ctx().clone();
        self.collect_results(&ctx);
        self.poll_export();
        self.shortcuts(&ctx, loaded);

        egui::Panel::bottom("studio-status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("studio-left")
            .default_size(290.0)
            .min_size(240.0)
            .show(ui, |ui| self.left_panel(ui));
        egui::Panel::right("studio-right")
            .default_size(330.0)
            .min_size(270.0)
            .show(ui, |ui| self.right_panel(ui, loaded));
        egui::CentralPanel::default().show(ui, |ui| self.canvas(ui, loaded));

        // A gesture ends when the pointer is released; the next edit starts a new step.
        if !ctx.input(|i| i.pointer.any_down()) {
            self.history.close();
        }
        if let Some(img) = loaded {
            if self.dirty {
                self.dirty = false;
                self.submit_preview(img);
            }
            if self.thumbs_dirty {
                self.thumbs_dirty = false;
                self.submit_thumbs(img);
            }
        }
        if self.export_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Records the error and shows it in the status bar.
    pub fn report_error(&mut self, message: String) {
        self.error = Some(message);
    }

    fn shortcuts(&mut self, ctx: &egui::Context, loaded: Option<&Loaded>) {
        // Typing in a text field keeps its own undo.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let redo = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        let redo_y = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        // Shift+Ctrl+Z must be tried before Ctrl+Z, which it also satisfies.
        let (do_redo, do_undo, do_save) = ctx.input_mut(|i| {
            let r = i.consume_shortcut(&redo) | i.consume_shortcut(&redo_y);
            let u = !r && i.consume_shortcut(&undo);
            (r, u, i.consume_shortcut(&save))
        });
        if do_redo {
            self.redo();
        }
        if do_undo {
            self.undo();
        }
        if do_save {
            self.save_project_action(loaded, false);
        }
    }

    // ---- toolbar ---------------------------------------------------------

    /// The Studio part of the app's top bar. Laid out right to left, so the
    /// first control added is the rightmost.
    pub fn toolbar(&mut self, ui: &mut Ui, loaded: Option<&Loaded>) {
        ui.add_enabled_ui(loaded.is_some() && self.export_rx.is_none(), |ui| {
            if ui.button("Export").clicked() {
                if let Some(img) = loaded {
                    self.export_dialog(img);
                }
            }
        });
        egui::ComboBox::from_id_salt("studio-factor")
            .selected_text(format!("{}×", self.export.factor))
            .width(50.0)
            .show_ui(ui, |ui| {
                for f in EXPORT_FACTORS {
                    ui.selectable_value(&mut self.export.factor, f, format!("{f}×"));
                }
            });
        let label = EXPORT_FORMATS
            .iter()
            .find(|(ext, _)| *ext == self.export.format)
            .map_or("PNG", |(_, l)| *l);
        egui::ComboBox::from_id_salt("studio-format")
            .selected_text(label)
            .width(64.0)
            .show_ui(ui, |ui| {
                for (ext, name) in EXPORT_FORMATS {
                    ui.selectable_value(&mut self.export.format, ext.to_string(), name);
                }
            });
        ui.separator();

        ui.menu_button("Look", |ui| {
            if ui.button("Load look…").clicked() {
                ui.close();
                let picked = rfd::FileDialog::new()
                    .add_filter("Ditherr look", &[ditherr_io::LOOK_EXTENSION])
                    .pick_file();
                if let Some(path) = picked {
                    if let Err(e) = self.load_look(&path) {
                        self.error = Some(e);
                    }
                }
            }
            if ui.button("Save look…").clicked() {
                ui.close();
                let picked = rfd::FileDialog::new()
                    .add_filter("Ditherr look", &[ditherr_io::LOOK_EXTENSION])
                    .set_file_name(format!("look.{}", ditherr_io::LOOK_EXTENSION))
                    .save_file();
                if let Some(path) = picked {
                    match self.save_look(&path) {
                        Ok(()) => self.notice = Some(format!("Saved {}", path.display())),
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        });
        ui.menu_button("Project", |ui| {
            if ui.button("Open project…").clicked() {
                ui.close();
                let picked = rfd::FileDialog::new()
                    .add_filter("Ditherr project", &[ditherr_io::PROJECT_EXTENSION])
                    .pick_file();
                if let Some(path) = picked {
                    if let Err(e) = self.open_project(&path) {
                        self.error = Some(e);
                    }
                }
            }
            if ui.button("Save project").clicked() {
                ui.close();
                self.save_project_action(loaded, false);
            }
            if ui.button("Save project as…").clicked() {
                ui.close();
                self.save_project_action(loaded, true);
            }
        });
        ui.separator();

        if ui
            .add_enabled(self.history.can_redo(), egui::Button::new("Redo"))
            .on_hover_text("Ctrl+Shift+Z")
            .clicked()
        {
            self.redo();
        }
        if ui
            .add_enabled(self.history.can_undo(), egui::Button::new("Undo"))
            .on_hover_text("Ctrl+Z")
            .clicked()
        {
            self.undo();
        }
    }

    fn export_dialog(&mut self, img: &Loaded) {
        let ext = self.export.format.clone();
        let picked = rfd::FileDialog::new()
            .add_filter(&ext, &[ext.as_str()])
            .set_file_name(format!("{}-ditherr.{ext}", img.stem()))
            .save_file();
        if let Some(path) = picked {
            self.export_to(&img.original, path);
        }
    }

    /// Saves to the project's file, or asks for one (always, with `ask`).
    fn save_project_action(&mut self, loaded: Option<&Loaded>, ask: bool) {
        let path = match (&self.project_path, ask) {
            (Some(p), false) => p.clone(),
            _ => {
                let stem = loaded.map_or_else(|| "project".to_string(), Loaded::stem);
                let picked = rfd::FileDialog::new()
                    .add_filter("Ditherr project", &[ditherr_io::PROJECT_EXTENSION])
                    .set_file_name(format!("{stem}.{}", ditherr_io::PROJECT_EXTENSION))
                    .save_file();
                match picked {
                    Some(p) => p,
                    None => return,
                }
            }
        };
        let source = loaded.and_then(|l| l.path.clone());
        match self.save_project(&path, source.as_deref()) {
            Ok(()) if source.is_some() => {
                self.notice = Some(format!("Saved project {}", path.display()));
            }
            Ok(()) => {
                self.notice = Some(format!(
                    "Saved project {} without a source image: none was opened from a file",
                    path.display()
                ));
            }
            Err(e) => self.error = Some(e),
        }
    }

    // ---- left panel --------------------------------------------------------

    fn left_panel(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);
            self.stack_section(ui);
            ui.separator();
            self.variations_section(ui);
            ui.separator();
            self.history_section(ui);
        });
    }

    fn stack_section(&mut self, ui: &mut Ui) {
        ui.strong("Effect stack");
        let n = self.edit.layers.len();
        let mut toggle = None;
        let mut select = None;
        let mut mv = None;
        let mut del = None;
        for (i, layer) in self.edit.layers.iter().enumerate() {
            ui.horizontal(|ui| {
                let mut on = layer.enabled;
                if ui
                    .checkbox(&mut on, "")
                    .on_hover_text("Switch this step on or off")
                    .changed()
                {
                    toggle = Some(i);
                }
                let name = ops::step_name(&layer.step);
                let summary = ops::step_summary(&layer.step);
                let text = if summary.is_empty() {
                    name.to_string()
                } else {
                    format!("{name}  {summary}")
                };
                let selected = self.selected == Selected::Layer(i);
                let label = if layer.enabled {
                    egui::RichText::new(text)
                } else {
                    egui::RichText::new(text).weak().strikethrough()
                };
                if ui.selectable_label(selected, label).clicked() {
                    select = Some(Selected::Layer(i));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, Icon::Cross, true, "Delete step").clicked() {
                        del = Some(i);
                    }
                    if icon_button(ui, Icon::Down, i + 1 < n, "Move down").clicked() {
                        mv = Some((i, 1));
                    }
                    if icon_button(ui, Icon::Up, i > 0, "Move up").clicked() {
                        mv = Some((i, -1));
                    }
                });
            });
        }

        let mut add = None;
        ui.menu_button("+ Add step", |ui| {
            for (name, step) in ops::new_layer_kinds() {
                if ui.button(name).clicked() {
                    add = Some((name, step));
                    ui.close();
                }
            }
        });

        if let Some(s) = select {
            self.selected = s;
        }
        if let Some(i) = toggle {
            self.toggle_layer(i);
        }
        if let Some((i, d)) = mv {
            self.move_layer(i, d);
        }
        if let Some(i) = del {
            self.delete_layer(i);
        }
        if let Some((name, step)) = add {
            self.add_layer(name, step);
        }

        ui.add_space(6.0);
        ui.weak("Steps run top to bottom, then these stages:");
        let look = &self.edit.look;
        let stages = [
            (
                Selected::PixelScale,
                format!("Pixel scale  ×{}", look.pixel_scale),
            ),
            (
                Selected::Palette,
                format!("Palette  {}", palette_summary(&look.palette)),
            ),
            (
                Selected::Dither,
                format!("Dither  {}", algorithm_summary(&look.algorithm)),
            ),
        ];
        for (which, text) in stages {
            if ui.selectable_label(self.selected == which, text).clicked() {
                self.selected = which;
            }
        }
    }

    fn variations_section(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.strong("Variations");
            if ui
                .button("Save current")
                .on_hover_text("Keep this look to compare with others")
                .clicked()
            {
                self.save_variation();
            }
        });
        if self.variations.is_empty() {
            ui.weak("Saved looks appear here.");
            return;
        }
        let mut apply = None;
        let mut delete = None;
        ui.horizontal_wrapped(|ui| {
            for (i, v) in self.variations.iter().enumerate() {
                let current = v.edit == self.edit;
                let r = thumbnail(ui, &v.name, self.var_tex[i].as_ref(), current);
                if r.clicked() {
                    apply = Some(i);
                }
                r.context_menu(|ui| {
                    if ui.button("Delete").clicked() {
                        delete = Some(i);
                        ui.close();
                    }
                });
            }
        });
        ui.weak("Click to apply, right-click to delete.");
        if let Some(i) = apply {
            self.apply_variation(i);
        }
        if let Some(i) = delete {
            self.delete_variation(i);
        }
    }

    fn history_section(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.strong("History");
            if ui
                .add_enabled(self.history.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(self.history.can_redo(), egui::Button::new("Redo"))
                .clicked()
            {
                self.redo();
            }
        });
        let labels: Vec<String> = self.history.labels().map(str::to_owned).collect();
        let at = self.history.position();
        let mut jump = None;
        egui::ScrollArea::vertical()
            .id_salt("history-list")
            .max_height(200.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (i, label) in labels.iter().enumerate() {
                    // Steps after the current one are redo steps: shown dimmed.
                    let text = if i > at {
                        egui::RichText::new(format!("{i}. {label}")).weak()
                    } else {
                        egui::RichText::new(format!("{i}. {label}"))
                    };
                    if ui.selectable_label(i == at, text).clicked() {
                        jump = Some(i);
                    }
                }
            });
        if let Some(i) = jump {
            self.jump_to(i);
        }
    }

    // ---- right panel ---------------------------------------------------------

    fn right_panel(&mut self, ui: &mut Ui, loaded: Option<&Loaded>) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);
            let mut ch = Changes::default();
            match self.selected {
                Selected::Layer(i) => {
                    if let Some(layer) = self.edit.layers.get_mut(i) {
                        ui.heading(ops::step_name(&layer.step));
                        layer_panel(ui, layer, &mut ch);
                    }
                }
                Selected::PixelScale => {
                    ui.heading("Pixel scale");
                    pixel_scale_panel(ui, &mut self.edit.look, &mut ch);
                }
                Selected::Dither => {
                    ui.heading("Dither");
                    dither_panel(ui, &mut self.edit.look, &mut ch);
                }
                Selected::Palette => {
                    ui.heading("Palette");
                    self.palette_panel(ui, loaded);
                }
            }
            self.commit(ch);
            ui.add_space(8.0);
            ui.separator();
            self.info_section(ui, loaded);
        });
    }

    fn info_section(&self, ui: &mut Ui, loaded: Option<&Loaded>) {
        ui.strong("Info");
        let Some(stats) = &self.info.stats else {
            ui.weak("Nothing rendered yet.");
            return;
        };
        ui.label(format!(
            "{}×{} · {} colours used · {} ms",
            self.info.out_size.0, self.info.out_size.1, stats.colors, self.info.millis
        ));

        ui.add_space(4.0);
        ui.label("Tones going into the dither");
        histogram(ui, &stats.histogram);
        ui.weak("black to white");

        ui.add_space(4.0);
        ui.label("Colour use");
        usage_bar(ui, &stats.top_colors, stats.pixels);
        for &(color, count) in &stats.top_colors {
            ui.horizontal(|ui| {
                swatch_strip(ui, &[color]);
                ui.monospace(ops::hex(color));
                let share = count as f64 * 100.0 / stats.pixels.max(1) as f64;
                ui.label(format!("{share:.1}%"));
            });
        }
        if stats.colors > stats.top_colors.len() {
            ui.weak(format!(
                "… and {} more",
                stats.colors - stats.top_colors.len()
            ));
        }

        if let (Some(img), true) = (loaded, stats.png_bytes > 0) {
            let preview_px = f64::from(img.preview_src.width) * f64::from(img.preview_src.height);
            let full_px = f64::from(img.original.width) * f64::from(img.original.height);
            let factor = f64::from(self.export.factor);
            let scale = full_px / preview_px * factor * factor;
            ui.add_space(4.0);
            ui.label(format!(
                "Estimated PNG size at export: ~{}",
                format_bytes(estimate_full_size(stats.png_bytes, scale))
            ));
        }
    }

    // ---- canvas --------------------------------------------------------------

    fn canvas(&mut self, ui: &mut Ui, loaded: Option<&Loaded>) {
        let Some(img) = loaded else {
            ui.centered_and_justified(|ui| {
                ui.label("Open an image, or hand a look over from Quick Stylize.");
            });
            return;
        };

        let mut zoom_cmd = None;
        // Wraps instead of overflowing, which would widen the canvas past its panel.
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.compare, CompareMode::Single, "Result");
            ui.selectable_value(&mut self.compare, CompareMode::Split, "Split");
            ui.selectable_value(&mut self.compare, CompareMode::SideBySide, "Side by side");
            if self.compare == CompareMode::Single {
                ui.checkbox(&mut self.show_original, "Original");
                ui.weak("(hold Space)");
            }
            ui.separator();
            if ui
                .small_button("−")
                .on_hover_text("Zoom out (Ctrl+scroll)")
                .clicked()
            {
                zoom_cmd = Some(ZoomCmd::Out);
            }
            ui.label(format!("Zoom {}", self.view.label()));
            if ui
                .small_button("+")
                .on_hover_text("Zoom in (Ctrl+scroll)")
                .clicked()
            {
                zoom_cmd = Some(ZoomCmd::In);
            }
            let zoomed = self.view.zoom != crate::view::Zoom::Fit;
            if ui
                .add_enabled(zoomed, egui::Button::new("Fit").small())
                .on_hover_text("Fit the image to the window (or double-click it)")
                .clicked()
            {
                zoom_cmd = Some(ZoomCmd::Fit);
            }
            if self.eyedropper {
                ui.separator();
                ui.colored_label(Color32::LIGHT_YELLOW, "Click the image to pick a colour");
            }
        });

        // Never larger than the panel, whatever the toolbar above did to the layout.
        let avail = ui.available_size().min(ui.max_rect().size());
        let (resp, painter) = ui.allocate_painter(avail, Sense::click_and_drag());
        let rect = resp.rect;
        painter.rect_filled(rect, 0.0, Color32::from_gray(24));
        self.hover = None;

        let Some(tex) = self.preview_tex.clone() else {
            ui.put(rect, egui::Spinner::new());
            return;
        };
        let ppp = ui.pixels_per_point();
        let result_size = tex.size_vec2();
        let side = self.compare == CompareMode::SideBySide;
        let half_w = (rect.width() - GAP) / 2.0;
        let left_view = Rect::from_min_size(
            rect.min,
            vec2(if side { half_w } else { rect.width() }, rect.height()),
        );
        let right_view = Rect::from_min_size(
            pos2(rect.min.x + half_w + GAP, rect.min.y),
            vec2(half_w, rect.height()),
        );

        // Zoom and pan.
        let view_size = left_view.size();
        match zoom_cmd {
            Some(ZoomCmd::In) => self.view.zoom_in(view_size, result_size, ppp),
            Some(ZoomCmd::Out) => self.view.zoom_out(view_size, result_size, ppp),
            Some(ZoomCmd::Fit) => self.view.reset(),
            None => {}
        }
        if resp.hovered() {
            // egui spreads one wheel notch over several frames, so deltas are summed
            // and one zoom step is taken per ~25% (about one notch). The sum is
            // dropped when the gesture pauses, so a step never carries over.
            let pinch = ui.input(|i| i.zoom_delta());
            if pinch == 1.0 {
                self.zoom_accum = 0.0;
            } else {
                self.zoom_accum += pinch - 1.0;
                if self.zoom_accum > 0.25 {
                    self.zoom_accum = 0.0;
                    self.view.zoom_in(view_size, result_size, ppp);
                } else if self.zoom_accum < -0.25 {
                    self.zoom_accum = 0.0;
                    self.view.zoom_out(view_size, result_size, ppp);
                }
            }
        }
        if resp.double_clicked() {
            self.view.reset();
        }
        if resp.dragged() {
            self.view.pan_by(resp.drag_delta());
        }

        // Where each image sits. The original is laid out at the result's scale;
        // when a crop, rotate or resize changed the geometry the two no longer
        // line up, so it is stretched over the result instead.
        let orig_px = img.original_tex.size();
        let orig_size = vec2(orig_px[0] as f32, orig_px[1] as f32);
        let pixel_scale = scaled_look(&self.edit.look, img.preview_shrink).pixel_scale;
        let aligned = ops::preview_is_aligned(
            (orig_px[0] as u32, orig_px[1] as u32),
            (result_size.x as u32, result_size.y as u32),
            pixel_scale,
        );
        let result_rect = self.view.image_rect(left_view, result_size, ppp);
        let result_rect_right = self.view.image_rect(right_view, result_size, ppp);
        let original_in = |r: Rect| {
            if aligned {
                Rect::from_min_size(r.min, orig_size * (r.width() / result_size.x))
            } else {
                r
            }
        };
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let draw = |p: &egui::Painter, t: &egui::TextureHandle, r: Rect| {
            p.image(t.id(), r, uv, Color32::WHITE);
        };

        let peek = self.show_original || ui.input(|i| i.key_down(Key::Space));
        let mut readout_rect = result_rect;
        match self.compare {
            CompareMode::Single => {
                if peek {
                    draw(&painter, &img.original_tex, original_in(result_rect));
                } else {
                    draw(&painter, &tex, result_rect);
                }
            }
            CompareMode::Split => {
                let x = result_rect.min.x + self.split * result_rect.width();
                let left = Rect::from_min_max(rect.min, pos2(x, rect.max.y));
                let right = Rect::from_min_max(pos2(x, rect.min.y), rect.max);
                draw(
                    &painter.with_clip_rect(left),
                    &img.original_tex,
                    original_in(result_rect),
                );
                draw(&painter.with_clip_rect(right), &tex, result_rect);
                tag(
                    &painter,
                    result_rect.min.max(rect.min) + vec2(6.0, 6.0),
                    "Original",
                );
                self.split_handle(ui, &painter, rect, result_rect, x);
            }
            CompareMode::SideBySide => {
                draw(
                    &painter.with_clip_rect(left_view),
                    &img.original_tex,
                    original_in(result_rect),
                );
                draw(&painter.with_clip_rect(right_view), &tex, result_rect_right);
                tag(&painter, left_view.min + vec2(6.0, 6.0), "Original");
                tag(&painter, right_view.min + vec2(6.0, 6.0), "Result");
                if let Some(pos) = resp.hover_pos() {
                    if right_view.contains(pos) {
                        readout_rect = result_rect_right;
                    }
                }
            }
        }
        if peek && self.compare == CompareMode::Single {
            tag(&painter, rect.min + vec2(6.0, 6.0), "Original");
        }

        // Pixel readout and the colour picker.
        let hovered_px = resp
            .hover_pos()
            .and_then(|pos| pixel_at(pos, readout_rect, [result_px(&tex).0, result_px(&tex).1]));
        if let (Some(px), Some(preview)) = (hovered_px, &self.preview_img) {
            let i = (px[1] as usize * preview.width as usize + px[0] as usize) * 4;
            if let Some(p) = preview.data.get(i..i + 3) {
                self.hover = Some(Hover {
                    px,
                    color: [p[0], p[1], p[2]],
                });
            }
        }
        if self.eyedropper && resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        }
        if self.eyedropper && resp.clicked() {
            match hovered_px {
                Some(px) if aligned => match ops::pixel_color(&img.preview_src, px[0], px[1]) {
                    Some(c) => self.pick_color(c),
                    None => self.notice = Some("That pixel is transparent".into()),
                },
                Some(_) => {
                    self.notice = Some(
                        "Picking needs the result to line up with the original: switch off crop, rotate and resize steps"
                            .into(),
                    );
                }
                None => {}
            }
        }
    }

    /// The draggable divider of the split view.
    fn split_handle(
        &mut self,
        ui: &mut Ui,
        painter: &egui::Painter,
        rect: Rect,
        image: Rect,
        x: f32,
    ) {
        let top = image.min.y.max(rect.min.y);
        let bottom = image.max.y.min(rect.max.y);
        let handle = Rect::from_min_max(pos2(x - 8.0, top), pos2(x + 8.0, bottom)).intersect(rect);
        let r = ui.interact(
            handle,
            ui.make_persistent_id("studio-split-handle"),
            Sense::drag(),
        );
        if r.hovered() || r.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if r.dragged() && image.width() > 0.0 {
            self.split = (self.split + r.drag_delta().x / image.width()).clamp(0.0, 1.0);
        }
        painter.line_segment(
            [pos2(x, top), pos2(x, bottom)],
            Stroke::new(2.0, Color32::WHITE),
        );
        let mid = pos2(x, (top + bottom) / 2.0);
        painter.circle_filled(mid, 8.0, Color32::WHITE);
        painter.circle_stroke(mid, 8.0, Stroke::new(1.0, Color32::from_gray(60)));
    }

    // ---- status bar -------------------------------------------------------------

    fn status_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if let Some(e) = &self.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            } else if let Some(n) = &self.notice {
                ui.label(n);
            } else {
                ui.label(format!(
                    "{} · {} steps in history",
                    self.view.label(),
                    self.history.len() - 1
                ));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(h) = &self.hover {
                    ui.monospace(format!("{}, {}  {}", h.px[0], h.px[1], ops::hex(h.color)));
                    swatch_strip(ui, &[h.color]);
                }
            });
        });
    }
}

fn result_px(tex: &egui::TextureHandle) -> (u32, u32) {
    let s = tex.size();
    (s[0] as u32, s[1] as u32)
}

/// A small caption with a dark backing, drawn at `pos`.
fn tag(painter: &egui::Painter, pos: egui::Pos2, text: &str) {
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(12.0),
        Color32::WHITE,
    );
    let rect = Rect::from_min_size(pos, galley.size()).expand(3.0);
    painter.rect_filled(rect, 3.0, Color32::from_black_alpha(160));
    painter.galley(pos, galley, Color32::WHITE);
}

fn histogram(ui: &mut Ui, bins: &[u32]) {
    let height = 56.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_gray(20));
    let max = bins.iter().copied().max().unwrap_or(1).max(1) as f32;
    let bar = rect.width() / bins.len().max(1) as f32;
    let floor = rect.max.y - 2.0;
    for (i, &n) in bins.iter().enumerate() {
        let h = n as f32 / max * (height - 6.0);
        let x = rect.min.x + i as f32 * bar;
        let r = Rect::from_min_max(pos2(x, floor - h), pos2(x + (bar - 0.5).max(1.0), floor));
        painter.rect_filled(r, 0.0, Color32::from_gray(200));
    }
}

/// One bar split into the colours of the result in proportion to their use.
fn usage_bar(ui: &mut Ui, colors: &[([u8; 3], u32)], total: u64) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_gray(20));
    let mut x = rect.min.x;
    for &([r, g, b], n) in colors {
        let w = n as f32 / total.max(1) as f32 * rect.width();
        let seg = Rect::from_min_max(
            pos2(x, rect.min.y),
            pos2((x + w).min(rect.max.x), rect.max.y),
        );
        painter.rect_filled(seg, 0.0, Color32::from_rgb(r, g, b));
        x += w;
    }
}
