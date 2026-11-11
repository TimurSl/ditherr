//! Decoding and encoding for Ditherr: images, indexed PNG, GIF, BMP and look files.
//!
//! Palette files and project files arrive with the Studio milestone.

mod error;
mod export;
mod import;
mod look_file;

pub use error::{Error, Result};
pub use export::{encode, encode_indexed_png, save, Format};
pub use import::{decode_image, read_image};
pub use look_file::{look_to_string, parse_look, read_look, write_look, LOOK_EXTENSION};
