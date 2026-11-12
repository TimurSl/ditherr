//! Ditherr engine: pure, deterministic pixel processing.
//!
//! This crate performs no I/O and touches no GPU. Every frontend (CLI, GUI)
//! goes through it so that identical settings yield identical bytes. To keep
//! that true across platforms, transcendental maths uses `libm` (enforced by
//! `clippy.toml`) and nothing depends on thread scheduling.
//!
//! Pipeline: source → [`preprocess`] stack (linear-light `f32`) → pixel-scale
//! downsample → quantize + [`dither`] → [`IndexedImage`] → integer upscale.
//! [`render`] runs all of it from a [`Look`].

// Colour maths loops over the three channels by index; that reads better than iterator
// gymnastics when several arrays are indexed in step.
#![allow(clippy::needless_range_loop)]

pub mod buffer;
pub mod color;
pub mod dither;
pub mod error;
pub mod extract;
pub mod look;
pub mod palette;
pub mod preprocess;
pub mod presets;
pub mod quantizer;
pub mod resize;
pub mod stack;

pub use buffer::{ImageF32, IndexedImage, Rendered, Rgba8Image};
pub use color::Metric;
pub use dither::{Algorithm, ErrorClamp, ErrorKernel, Params, ScanDirection};
pub use error::{Error, Result};
pub use extract::AutoMethod;
pub use look::{render, Look, LOOK_VERSION};
pub use palette::PaletteSpec;
pub use preprocess::{Rotation, Step};
pub use resize::{Resampler, ResizeTarget};
pub use stack::Layer;
