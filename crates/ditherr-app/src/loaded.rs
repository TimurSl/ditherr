//! An image opened in the app, shared by Quick Stylize and Studio.

use std::path::PathBuf;
use std::sync::Arc;

use ditherr_core::Rgba8Image;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

use crate::worker::{fit_within, PREVIEW_EDGE, THUMB_EDGE};

pub struct Loaded {
    pub name: String,
    /// Where it was read from; `None` for images that did not come from a file.
    pub path: Option<PathBuf>,
    pub original: Arc<Rgba8Image>,
    /// What the live preview renders (the original, shrunk if it is large).
    pub preview_src: Arc<Rgba8Image>,
    pub thumb_src: Arc<Rgba8Image>,
    pub preview_shrink: f32,
    pub thumb_shrink: f32,
    /// The preview source as a texture, for comparing against the result.
    pub original_tex: TextureHandle,
}

impl Loaded {
    pub fn new(ctx: &egui::Context, name: String, img: Rgba8Image, path: Option<PathBuf>) -> Self {
        let original = Arc::new(img);
        let preview_src = Arc::new(fit_within(&original, PREVIEW_EDGE));
        let thumb_src = Arc::new(fit_within(&original, THUMB_EDGE));
        let original_tex = ctx.load_texture(
            "original",
            to_color_image(&preview_src),
            TextureOptions::NEAREST,
        );
        Self {
            name,
            path,
            preview_shrink: preview_src.width as f32 / original.width as f32,
            thumb_shrink: thumb_src.width as f32 / original.width as f32,
            original,
            preview_src,
            thumb_src,
            original_tex,
        }
    }

    /// File name without its extension, or "image".
    pub fn stem(&self) -> String {
        std::path::Path::new(&self.name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image".into())
    }
}

pub fn to_color_image(img: &Rgba8Image) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.data)
}
