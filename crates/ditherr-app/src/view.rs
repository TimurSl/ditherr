//! Where an image sits inside a viewport: fit-to-window, pixel-accurate zoom
//! (spec §13, "no smoothing") and panning. Pure geometry, no drawing.

use egui::{pos2, vec2, Pos2, Rect, Vec2};

/// Zoom steps in whole device pixels per image pixel.
pub const ZOOM_STEPS: [u32; 6] = [1, 2, 3, 4, 8, 16];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    /// As large as fits. Whole device pixels per image pixel when that is at
    /// least 1, so enlarged dither patterns stay crisp.
    Fit,
    /// This many device pixels per image pixel (100% is 1, 200% is 2, ...).
    Pixels(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub zoom: Zoom,
    /// Offset of the image centre from the viewport centre, in points.
    pub pan: Vec2,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: Zoom::Fit,
            pan: Vec2::ZERO,
        }
    }
}

impl View {
    /// Points per image pixel.
    pub fn scale(&self, viewport: Vec2, image: Vec2, ppp: f32) -> f32 {
        match self.zoom {
            Zoom::Pixels(n) => n as f32 / ppp,
            Zoom::Fit => {
                let fit = (viewport.x / image.x).min(viewport.y / image.y);
                if fit * ppp >= 1.0 {
                    (fit * ppp).floor() / ppp
                } else {
                    fit
                }
            }
        }
    }

    /// The rectangle the image occupies, in the same coordinates as `viewport`.
    /// Its corner is snapped to a device pixel so nothing is resampled.
    pub fn image_rect(&self, viewport: Rect, image: Vec2, ppp: f32) -> Rect {
        let scale = self.scale(viewport.size(), image, ppp);
        let size = image * scale;
        let pan = self.clamped_pan(viewport.size(), size);
        let min = viewport.center() + pan - size / 2.0;
        let snap = |v: f32| (v * ppp).round() / ppp;
        Rect::from_min_size(pos2(snap(min.x), snap(min.y)), size)
    }

    /// Panning only makes sense along an axis where the image overflows, and
    /// never lets the image leave the viewport.
    fn clamped_pan(&self, viewport: Vec2, shown: Vec2) -> Vec2 {
        let limit = |view: f32, img: f32| ((img - view) / 2.0).max(0.0);
        vec2(
            self.pan
                .x
                .clamp(-limit(viewport.x, shown.x), limit(viewport.x, shown.x)),
            self.pan
                .y
                .clamp(-limit(viewport.y, shown.y), limit(viewport.y, shown.y)),
        )
    }

    /// Next larger zoom step; from Fit it goes to the first step above the fitted size.
    pub fn zoom_in(&mut self, viewport: Vec2, image: Vec2, ppp: f32) {
        let current = self.scale(viewport, image, ppp) * ppp;
        let next = ZOOM_STEPS
            .iter()
            .copied()
            .find(|&n| n as f32 > current + 0.001)
            .unwrap_or(ZOOM_STEPS[ZOOM_STEPS.len() - 1]);
        self.zoom = Zoom::Pixels(next);
    }

    /// Next smaller step; below the first one it returns to Fit.
    pub fn zoom_out(&mut self, viewport: Vec2, image: Vec2, ppp: f32) {
        let current = self.scale(viewport, image, ppp) * ppp;
        match ZOOM_STEPS
            .iter()
            .copied()
            .rev()
            .find(|&n| (n as f32) < current - 0.001)
        {
            Some(n) => self.zoom = Zoom::Pixels(n),
            None => self.reset(),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Drags the image by `delta` points.
    pub fn pan_by(&mut self, delta: Vec2) {
        if self.zoom != Zoom::Fit {
            self.pan += delta;
        }
    }

    /// Label such as "Fit" or "200%".
    pub fn label(&self) -> String {
        match self.zoom {
            Zoom::Fit => "Fit".into(),
            Zoom::Pixels(n) => format!("{}%", n * 100),
        }
    }
}

/// The image pixel under `pos` when the image is drawn in `rect`.
pub fn pixel_at(pos: Pos2, rect: Rect, image_size: [u32; 2]) -> Option<[u32; 2]> {
    if !rect.contains(pos) || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    let fx = (pos.x - rect.min.x) / rect.width();
    let fy = (pos.y - rect.min.y) / rect.height();
    let x = ((fx * image_size[0] as f32) as u32).min(image_size[0].saturating_sub(1));
    let y = ((fy * image_size[1] as f32) as u32).min(image_size[1].saturating_sub(1));
    Some([x, y])
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Vec2 = vec2(800.0, 600.0);

    fn viewport() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), VIEW)
    }

    #[test]
    fn fit_uses_whole_pixel_multiples_when_enlarging() {
        let v = View::default();
        // 100x50 into 800x600 fits at 8x.
        assert_eq!(v.scale(VIEW, vec2(100.0, 50.0), 1.0), 8.0);
        // 130x50 fits at 6.15x and floors to 6x.
        assert_eq!(v.scale(VIEW, vec2(130.0, 50.0), 1.0), 6.0);
        // On a 2x display a device pixel is half a point: 6.15 * 2 floors to 12.
        assert_eq!(v.scale(VIEW, vec2(130.0, 50.0), 2.0), 6.0);
        // Large images shrink smoothly (no whole-pixel rule below 1x).
        let s = v.scale(VIEW, vec2(1600.0, 1200.0), 1.0);
        assert!((s - 0.5).abs() < 1e-6);
    }

    #[test]
    fn fixed_zoom_is_device_pixels_per_image_pixel() {
        let v = View {
            zoom: Zoom::Pixels(2),
            pan: Vec2::ZERO,
        };
        assert_eq!(v.scale(VIEW, vec2(10.0, 10.0), 1.0), 2.0);
        assert_eq!(v.scale(VIEW, vec2(10.0, 10.0), 2.0), 1.0);
    }

    #[test]
    fn image_is_centred_and_snapped() {
        let v = View::default();
        let r = v.image_rect(viewport(), vec2(100.0, 50.0), 1.0);
        assert_eq!(r.size(), vec2(800.0, 400.0));
        assert_eq!(r.center(), viewport().center());
        for ppp in [1.0, 1.5, 2.0] {
            let r = v.image_rect(viewport(), vec2(99.0, 51.0), ppp);
            for edge in [r.min.x, r.min.y] {
                let device = edge * ppp;
                assert!((device - device.round()).abs() < 1e-3, "ppp {ppp}: {edge}");
            }
        }
    }

    #[test]
    fn pan_is_ignored_at_fit_and_clamped_when_zoomed() {
        let mut v = View::default();
        v.pan_by(vec2(50.0, 50.0));
        assert_eq!(v.pan, Vec2::ZERO);

        v.zoom = Zoom::Pixels(16);
        // 100x100 at 16x is 1600x1600 in an 800x600 viewport: 400 and 500 of slack per side.
        v.pan_by(vec2(10_000.0, -10_000.0));
        let r = v.image_rect(viewport(), vec2(100.0, 100.0), 1.0);
        assert_eq!(r.min.x, viewport().center().x + 400.0 - 800.0);
        assert_eq!(r.min.y, viewport().center().y - 500.0 - 800.0);

        // A small image cannot be dragged off-centre.
        v.zoom = Zoom::Pixels(1);
        let r = v.image_rect(viewport(), vec2(100.0, 100.0), 1.0);
        assert_eq!(r.center(), viewport().center());
    }

    #[test]
    fn zoom_steps_go_up_and_back_to_fit() {
        let image = vec2(100.0, 100.0); // fits at 6x
        let mut v = View::default();
        v.zoom_in(VIEW, image, 1.0);
        assert_eq!(v.zoom, Zoom::Pixels(8));
        v.zoom_in(VIEW, image, 1.0);
        assert_eq!(v.zoom, Zoom::Pixels(16));
        v.zoom_in(VIEW, image, 1.0);
        assert_eq!(v.zoom, Zoom::Pixels(16), "stops at the top");
        v.zoom_out(VIEW, image, 1.0);
        assert_eq!(v.zoom, Zoom::Pixels(8));
        v.zoom_out(VIEW, image, 1.0);
        assert_eq!(v.zoom, Zoom::Pixels(4));
        for _ in 0..4 {
            v.zoom_out(VIEW, image, 1.0);
        }
        assert_eq!(v.zoom, Zoom::Fit);
    }

    #[test]
    fn labels() {
        assert_eq!(View::default().label(), "Fit");
        let v = View {
            zoom: Zoom::Pixels(4),
            pan: Vec2::ZERO,
        };
        assert_eq!(v.label(), "400%");
    }

    #[test]
    fn pixel_lookup() {
        let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(80.0, 40.0));
        assert_eq!(pixel_at(pos2(100.0, 100.0), rect, [8, 4]), Some([0, 0]));
        assert_eq!(pixel_at(pos2(125.0, 135.0), rect, [8, 4]), Some([2, 3]));
        assert_eq!(pixel_at(pos2(179.9, 139.9), rect, [8, 4]), Some([7, 3]));
        assert_eq!(pixel_at(pos2(99.0, 100.0), rect, [8, 4]), None);
        assert_eq!(pixel_at(pos2(100.0, 141.0), rect, [8, 4]), None);
    }
}
